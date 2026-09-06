//! Encryption at rest for any [`Database`].
//!
//! [`EncryptedDatabase`] is a decorator: it wraps an arbitrary backend and encrypts every
//! record written through it. Encryption is therefore a property of the storage seam rather
//! than of each repository that happens to remember to ask for it, and this module is the
//! only place in the workspace that performs cryptography on stored data.
//!
//! The scheme, versioned by [`STORE_VERSION`]:
//!
//! - The root of the record hierarchy is a random data key, generated once when the store is
//!   created and never derived from anything the user types. It is held in the header,
//!   wrapped by one or more [`KeySource`] slots, each of which can recover it on its own.
//!   Today the only slot kind stretches a password with Argon2id (64 MiB, 3 passes); a
//!   hardware token is a second kind, added without disturbing the first.
//! - Every record gets its own key and its own blinded storage key, both derived from the
//!   data key with HKDF-SHA256 over the record's full logical key. Because a
//!   [`super::scoped::ScopedDatabase`] prefix is part of that logical key, each vault and
//!   executor lands in a keyspace that is cryptographically isolated rather than merely
//!   prefixed: a record lifted from one scope will not decrypt in another.
//! - Values are sealed with XChaCha20-Poly1305 under a random 192-bit nonce, with the
//!   version and logical key bound in as associated data.
//! - Storage keys are blinded, so a backend never sees a logical key name. Names only:
//!   record count and ciphertext length still leak, and blinding forecloses prefix
//!   iteration. Both costs are recorded under "Known costs" in `spec/01-architecture.md`.
//!
//! Separating the data key from the password is what lets a credential change without
//! touching records: rotating a password rewraps one slot, where deriving record keys from
//! the password directly would have meant re-encrypting the entire store.
//!
//! [`KeySource`], [`DataKey`] and [`StoredSlot`] are `pub(crate)`: a slot implementation
//! handles the data key in the clear, so it belongs beside the other secret-handling code.

use std::sync::Arc;

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, AeadCore, OsRng, Payload, rand_core::RngCore},
};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::{ZeroizeOnDrop, Zeroizing};

use super::{Database, DatabaseError};

/// Format version of the store: the header, and every blob sealed under it. Bumping it is a
/// migration, so a new slot kind travels as a new [`StoredSlot::kind`] instead.
const STORE_VERSION: u8 = 2;

/// Plaintext key the header lives under. It must stay unblinded: it is read before any key
/// material exists.
const HEADER_KEY: &[u8] = b"edw:keystore:v1";

const HEADER_MAGIC: [u8; 8] = *b"EDWSTORE";

const RECORD_KEY_INFO: &[u8] = b"edw:record-key:v1";

const STORAGE_KEY_INFO: &[u8] = b"edw:storage-key:v1";

const RECORD_AAD_DOMAIN: &[u8] = b"edw:record:";

const SLOT_AAD_DOMAIN: &[u8] = b"edw:slot:";

/// Slot kind tag for [`PasswordKeySource`]. Persisted, and bound into the slot's AEAD tag,
/// so renaming it orphans every existing store.
const PASSWORD_SLOT_KIND: &str = "argon2id-password";

const SALT_LEN: usize = 16;

const KEY_LEN: usize = 32;

const NONCE_LEN: usize = 24;

/// Argon2id cost parameters. 64 MiB and 3 passes, as specced in `spec/01-architecture.md`.
const ARGON2_M_COST: u32 = 64 * 1024;

const ARGON2_T_COST: u32 = 3;

const ARGON2_P_COST: u32 = 1;

/// A credential that can wrap and recover the store's [`DataKey`].
///
/// One implementation per unlock method. [`PasswordKeySource`] is the only one today; a
/// hardware token is the next, and is why the trait is async.
#[async_trait::async_trait]
pub(crate) trait KeySource: Send + Sync {
    /// Stable tag identifying slots this source can read. Persisted in the header.
    fn kind(&self) -> &'static str;

    /// Wraps `data_key` into a fresh slot, choosing new parameters for it.
    async fn wrap(&self, data_key: &DataKey) -> Result<StoredSlot, EncryptedDatabaseError>;

    /// Recovers the data key from a slot this source recognizes.
    async fn unwrap(&self, slot: &StoredSlot) -> Result<DataKey, EncryptedDatabaseError>;
}

/// The key every record's key is derived from.
///
/// Deliberately not `Debug`, `Clone`, or `Serialize`: it must not be copyable into a log
/// line or a stored record.
#[derive(ZeroizeOnDrop)]
pub(crate) struct DataKey([u8; KEY_LEN]);

/// One way of recovering the store's [`DataKey`], as persisted in the header.
///
/// `params` is opaque to every kind but the one named by `kind`, so a build that does not
/// recognize a slot can still parse the header and unlock through a slot it does.
#[derive(Serialize, Deserialize)]
pub(crate) struct StoredSlot {
    kind: String,
    params: Vec<u8>,
    wrapped: Vec<u8>,
}

/// Parameters a [`PasswordKeySource`] slot carries. Serialized into [`StoredSlot::params`].
#[derive(Serialize, Deserialize)]
struct Argon2idParams {
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
    salt: Vec<u8>,
}

/// Recovers the data key by stretching a password with Argon2id.
pub(crate) struct PasswordKeySource {
    password: Zeroizing<Vec<u8>>,
}

#[derive(Serialize, Deserialize)]
struct KeystoreHeader {
    magic: [u8; 8],
    version: u8,
    slots: Vec<StoredSlot>,
}

/// Encrypts every record written to an inner [`Database`].
///
/// Construct with [`EncryptedDatabase::create`] for a fresh store, or
/// [`EncryptedDatabase::unlock`] for an existing one.
pub struct EncryptedDatabase {
    db: Arc<dyn Database>,
    data_key: DataKey,
}

#[derive(Debug, thiserror::Error)]
pub enum EncryptedDatabaseError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("incorrect password")]
    InvalidPassword,
    #[error("store is already initialized")]
    AlreadyInitialized,
    #[error("store is not initialized")]
    NotInitialized,
    #[error("record is corrupt or was written for a different key")]
    Corrupt,
    #[error("unsupported format version {0}, expected {STORE_VERSION}")]
    UnsupportedVersion(u8),
    #[error("header declares key-derivation parameters this build does not accept")]
    UnsupportedParameters,
    #[error("password must not be empty")]
    EmptyPassword,
    #[error("key derivation failed")]
    KeyDerivation,
    #[error("no slot of kind {0} in this store")]
    NoMatchingSlot(&'static str),
    #[error("header serialization error: {0}")]
    Serialization(#[from] postcard::Error),
}

impl DataKey {
    fn random() -> Self {
        let mut key = [0u8; KEY_LEN];
        OsRng.fill_bytes(&mut key);
        Self(key)
    }

    fn from_slice(bytes: &[u8]) -> Result<Self, EncryptedDatabaseError> {
        let key: [u8; KEY_LEN] = bytes
            .try_into()
            .map_err(|_| EncryptedDatabaseError::Corrupt)?;
        Ok(Self(key))
    }

    /// The raw bytes, for a [`KeySource`] that has to wrap them.
    pub(crate) fn expose(&self) -> &[u8; KEY_LEN] {
        &self.0
    }
}

impl PasswordKeySource {
    /// # Errors
    /// Returns [`EncryptedDatabaseError::EmptyPassword`] if `password` is empty.
    pub(crate) fn new(password: &[u8]) -> Result<Self, EncryptedDatabaseError> {
        if password.is_empty() {
            return Err(EncryptedDatabaseError::EmptyPassword);
        }
        Ok(Self {
            password: Zeroizing::new(password.to_vec()),
        })
    }
}

#[async_trait::async_trait]
impl KeySource for PasswordKeySource {
    fn kind(&self) -> &'static str {
        PASSWORD_SLOT_KIND
    }

    async fn wrap(&self, data_key: &DataKey) -> Result<StoredSlot, EncryptedDatabaseError> {
        let mut salt = vec![0u8; SALT_LEN];
        OsRng.fill_bytes(&mut salt);

        let wrapping_key = derive_wrapping_key(
            &self.password,
            &salt,
            ARGON2_M_COST,
            ARGON2_T_COST,
            ARGON2_P_COST,
        )?;
        let wrapped = seal_with(
            &wrapping_key,
            &slot_associated_data(PASSWORD_SLOT_KIND),
            data_key.expose(),
        )?;

        let params = Argon2idParams {
            m_cost: ARGON2_M_COST,
            t_cost: ARGON2_T_COST,
            p_cost: ARGON2_P_COST,
            salt,
        };
        Ok(StoredSlot {
            kind: PASSWORD_SLOT_KIND.to_string(),
            params: postcard::to_stdvec(&params)?,
            wrapped,
        })
    }

    async fn unwrap(&self, slot: &StoredSlot) -> Result<DataKey, EncryptedDatabaseError> {
        let params: Argon2idParams = postcard::from_bytes(&slot.params)?;

        // The header is untrusted input: it is plaintext, so anything able to write the store
        // can choose these. Argon2's own bounds are far too loose to lean on (`MAX_M_COST` is
        // `u32::MAX` KiB), so an unchecked slot turns a corrupt or hostile file into an
        // out-of-memory abort or an unbounded hang. Only the parameters this version writes
        // are accepted; a future cost change travels with a new slot kind.
        if params.m_cost != ARGON2_M_COST
            || params.t_cost != ARGON2_T_COST
            || params.p_cost != ARGON2_P_COST
            || params.salt.len() != SALT_LEN
        {
            return Err(EncryptedDatabaseError::UnsupportedParameters);
        }

        let wrapping_key = derive_wrapping_key(
            &self.password,
            &params.salt,
            params.m_cost,
            params.t_cost,
            params.p_cost,
        )?;
        let plaintext = open_with(
            &wrapping_key,
            &slot_associated_data(PASSWORD_SLOT_KIND),
            &slot.wrapped,
        )
        .map_err(|_| EncryptedDatabaseError::InvalidPassword)?;

        DataKey::from_slice(&plaintext)
    }
}

impl EncryptedDatabase {
    /// Initializes a fresh encrypted store over `db`, writing its header.
    ///
    /// The caller owns the password buffer's lifetime, including wiping it; this does not
    /// take ownership and cannot zeroize it.
    ///
    /// # Errors
    /// Returns [`EncryptedDatabaseError::AlreadyInitialized`] if `db` already holds a header,
    /// so an existing store is never silently re-keyed and its records orphaned, or
    /// [`EncryptedDatabaseError::EmptyPassword`] if `password` is empty.
    pub async fn create(
        db: Arc<dyn Database>,
        password: &[u8],
    ) -> Result<Self, EncryptedDatabaseError> {
        Self::create_with(db, &PasswordKeySource::new(password)?).await
    }

    /// Unlocks an existing encrypted store over `db`.
    ///
    /// # Errors
    /// Returns [`EncryptedDatabaseError::InvalidPassword`] when no password slot's wrapped
    /// data key survives its tag, so a wrong password fails here rather than on the first
    /// read of a real record, or [`EncryptedDatabaseError::UnsupportedParameters`] if a
    /// slot's key-derivation parameters are not the ones this version writes, checked before
    /// any hashing is done.
    pub async fn unlock(
        db: Arc<dyn Database>,
        password: &[u8],
    ) -> Result<Self, EncryptedDatabaseError> {
        Self::unlock_with(db, &PasswordKeySource::new(password)?).await
    }

    /// Initializes a fresh store whose data key is wrapped by `source`.
    ///
    /// # Errors
    /// See [`EncryptedDatabase::create`].
    pub(crate) async fn create_with(
        db: Arc<dyn Database>,
        source: &dyn KeySource,
    ) -> Result<Self, EncryptedDatabaseError> {
        if db.get(HEADER_KEY).await?.is_some() {
            return Err(EncryptedDatabaseError::AlreadyInitialized);
        }

        let data_key = DataKey::random();
        let header = KeystoreHeader {
            magic: HEADER_MAGIC,
            version: STORE_VERSION,
            slots: vec![source.wrap(&data_key).await?],
        };
        db.put(HEADER_KEY, &postcard::to_stdvec(&header)?).await?;

        Ok(Self { db, data_key })
    }

    /// Unlocks an existing store through any slot `source` recognizes.
    ///
    /// Every slot of the source's kind is tried, so enrolling a second credential of one
    /// kind does not shadow the first.
    ///
    /// # Errors
    /// See [`EncryptedDatabase::unlock`]. Returns
    /// [`EncryptedDatabaseError::NoMatchingSlot`] if the header holds no slot of the
    /// source's kind.
    pub(crate) async fn unlock_with(
        db: Arc<dyn Database>,
        source: &dyn KeySource,
    ) -> Result<Self, EncryptedDatabaseError> {
        let header = read_header(&db).await?;

        let mut recovered = None;
        let mut last_error = None;
        for slot in header.slots.iter().filter(|s| s.kind == source.kind()) {
            match source.unwrap(slot).await {
                Ok(data_key) => {
                    recovered = Some(data_key);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }

        let Some(data_key) = recovered else {
            return Err(
                last_error.unwrap_or_else(|| EncryptedDatabaseError::NoMatchingSlot(source.kind()))
            );
        };
        Ok(Self { db, data_key })
    }

    /// Unlocks an existing store, or initializes one if `db` holds no header.
    ///
    /// # Warning
    /// The two cases are told apart only by whether a header is present, and the store carries
    /// no integrity protection over its collection of records. If the header is lost or
    /// deleted, this initializes a fresh store over the top: the existing records survive on
    /// disk but become permanently unreadable, and the result reports itself as empty rather
    /// than as damaged. Prefer [`EncryptedDatabase::create`] and
    /// [`EncryptedDatabase::unlock`] at call sites that know which one they mean. See EDW-023.
    ///
    /// # Errors
    /// See [`EncryptedDatabase::create`] and [`EncryptedDatabase::unlock`].
    pub async fn open_or_create(
        db: Arc<dyn Database>,
        password: &[u8],
    ) -> Result<Self, EncryptedDatabaseError> {
        if db.get(HEADER_KEY).await?.is_some() {
            Self::unlock(db, password).await
        } else {
            Self::create(db, password).await
        }
    }

    /// Re-protects the store under `new_password`, replacing every password slot and
    /// leaving slots of other kinds enrolled.
    ///
    /// One header write, so no window exists in which both the old and the new password open
    /// the store, and no record is rewritten.
    ///
    /// # Errors
    /// Returns [`EncryptedDatabaseError::EmptyPassword`] if `new_password` is empty, or an
    /// error if the header cannot be read or written.
    pub async fn change_password(&self, new_password: &[u8]) -> Result<(), EncryptedDatabaseError> {
        let source = PasswordKeySource::new(new_password)?;
        let slot = source.wrap(&self.data_key).await?;

        let mut header = read_header(&self.db).await?;
        header
            .slots
            .retain(|existing| existing.kind != PASSWORD_SLOT_KIND);
        header.slots.push(slot);
        self.db
            .put(HEADER_KEY, &postcard::to_stdvec(&header)?)
            .await?;
        Ok(())
    }

    /// The kind tag of every slot in the header, in index order.
    ///
    /// # Errors
    /// Returns an error if the header cannot be read.
    pub async fn slot_kinds(&self) -> Result<Vec<String>, EncryptedDatabaseError> {
        let header = read_header(&self.db).await?;
        Ok(header.slots.into_iter().map(|slot| slot.kind).collect())
    }

    /// Derives the blinded key this logical key is stored under in the backend.
    fn storage_key(&self, key: &[u8]) -> Result<Vec<u8>, EncryptedDatabaseError> {
        let mut out = vec![0u8; KEY_LEN];
        self.expand(STORAGE_KEY_INFO, key, &mut out)?;
        Ok(out)
    }

    fn record_key(&self, key: &[u8]) -> Result<Zeroizing<[u8; KEY_LEN]>, EncryptedDatabaseError> {
        let mut out = Zeroizing::new([0u8; KEY_LEN]);
        self.expand(RECORD_KEY_INFO, key, out.as_mut())?;
        Ok(out)
    }

    /// No HKDF salt: the data key is already uniformly random, which is the case
    /// extract-then-expand does not need one for.
    fn expand(
        &self,
        domain: &[u8],
        key: &[u8],
        out: &mut [u8],
    ) -> Result<(), EncryptedDatabaseError> {
        let hkdf = Hkdf::<Sha256>::new(None, self.data_key.expose());
        hkdf.expand_multi_info(&[domain, key], out)
            .map_err(|_| EncryptedDatabaseError::KeyDerivation)
    }

    fn seal(&self, key: &[u8], value: &[u8]) -> Result<Vec<u8>, EncryptedDatabaseError> {
        seal_with(&*self.record_key(key)?, &associated_data(key), value)
    }

    fn open(&self, key: &[u8], blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, EncryptedDatabaseError> {
        open_with(&*self.record_key(key)?, &associated_data(key), blob)
    }
}

impl From<EncryptedDatabaseError> for DatabaseError {
    fn from(err: EncryptedDatabaseError) -> Self {
        DatabaseError::Other(Box::new(err))
    }
}

#[async_trait::async_trait]
impl Database for EncryptedDatabase {
    async fn get(&self, key: &[u8]) -> Result<Option<Zeroizing<Vec<u8>>>, DatabaseError> {
        let storage_key = self.storage_key(key)?;
        let Some(blob) = self.db.get(&storage_key).await? else {
            return Ok(None);
        };
        Ok(Some(self.open(key, &blob)?))
    }

    async fn put(&self, key: &[u8], value: &[u8]) -> Result<(), DatabaseError> {
        let storage_key = self.storage_key(key)?;
        let blob = self.seal(key, value)?;
        self.db.put(&storage_key, &blob).await
    }

    async fn delete(&self, key: &[u8]) -> Result<(), DatabaseError> {
        let storage_key = self.storage_key(key)?;
        self.db.delete(&storage_key).await
    }
}

async fn read_header(db: &Arc<dyn Database>) -> Result<KeystoreHeader, EncryptedDatabaseError> {
    let Some(bytes) = db.get(HEADER_KEY).await? else {
        return Err(EncryptedDatabaseError::NotInitialized);
    };
    let header: KeystoreHeader = postcard::from_bytes(&bytes)?;

    if header.magic != HEADER_MAGIC {
        return Err(EncryptedDatabaseError::Corrupt);
    }
    if header.version != STORE_VERSION {
        return Err(EncryptedDatabaseError::UnsupportedVersion(header.version));
    }
    Ok(header)
}

/// Binds the format version and the record's logical key into the AEAD tag, so a ciphertext
/// cannot be replayed under a different key even if key derivation were weakened.
fn associated_data(key: &[u8]) -> Vec<u8> {
    associated_data_in(RECORD_AAD_DOMAIN, key)
}

/// Binds the format version and slot kind into a wrapped data key's tag, so a slot cannot be
/// reinterpreted as one of another kind.
fn slot_associated_data(kind: &str) -> Vec<u8> {
    associated_data_in(SLOT_AAD_DOMAIN, kind.as_bytes())
}

/// The domain prefix is what keeps the two spaces disjoint: without it an unscoped record
/// named `argon2id-password` would share associated data with the password slot.
fn associated_data_in(domain: &[u8], context: &[u8]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(1 + domain.len() + context.len());
    aad.push(STORE_VERSION);
    aad.extend_from_slice(domain);
    aad.extend_from_slice(context);
    aad
}

fn seal_with(
    key: &[u8; KEY_LEN],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, EncryptedDatabaseError> {
    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| EncryptedDatabaseError::KeyDerivation)?;

    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| EncryptedDatabaseError::Corrupt)?;

    let mut blob = Vec::with_capacity(1 + NONCE_LEN + ciphertext.len());
    blob.push(STORE_VERSION);
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&ciphertext);
    Ok(blob)
}

fn open_with(
    key: &[u8; KEY_LEN],
    aad: &[u8],
    blob: &[u8],
) -> Result<Zeroizing<Vec<u8>>, EncryptedDatabaseError> {
    let Some((&version, rest)) = blob.split_first() else {
        return Err(EncryptedDatabaseError::Corrupt);
    };
    if version != STORE_VERSION {
        return Err(EncryptedDatabaseError::UnsupportedVersion(version));
    }
    if rest.len() < NONCE_LEN {
        return Err(EncryptedDatabaseError::Corrupt);
    }
    let (nonce, ciphertext) = rest.split_at(NONCE_LEN);
    let nonce: [u8; NONCE_LEN] = nonce
        .try_into()
        .map_err(|_| EncryptedDatabaseError::Corrupt)?;

    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| EncryptedDatabaseError::KeyDerivation)?;
    let plaintext = cipher
        .decrypt(
            &XNonce::from(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| EncryptedDatabaseError::Corrupt)?;
    Ok(Zeroizing::new(plaintext))
}

fn derive_wrapping_key(
    password: &[u8],
    salt: &[u8],
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
) -> Result<Zeroizing<[u8; KEY_LEN]>, EncryptedDatabaseError> {
    let params = Params::new(m_cost, t_cost, p_cost, Some(KEY_LEN))
        .map_err(|_| EncryptedDatabaseError::KeyDerivation)?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    //? Hash directly into the zeroizing buffer. Deriving into a local and moving it would
    //? leave an un-zeroized copy of the key on the stack.
    let mut wrapping_key = Zeroizing::new([0u8; KEY_LEN]);
    argon2
        .hash_password_into(password, salt, wrapping_key.as_mut())
        .map_err(|_| EncryptedDatabaseError::KeyDerivation)?;
    Ok(wrapping_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::memory::MemoryDatabase;

    const PASSWORD: &[u8] = b"correct horse battery staple";
    const NEXT_PASSWORD: &[u8] = b"a different passphrase entirely";

    /// Tests that a value written to an [`EncryptedDatabase`] is not stored in
    /// plaintext in its underlying storage.
    #[tokio::test]
    async fn test_encrypted_db_encrypts() {
        let password: &[u8] = b"password";
        let key: &[u8] = b"key";
        let value: &[u8] = b"value";

        let memory_db = Arc::new(MemoryDatabase::new());
        let db = EncryptedDatabase::create(memory_db.clone(), password)
            .await
            .unwrap();

        db.put(key, value).await.unwrap();

        //? Asserts that the value can be retrieved from the encrypted db.
        assert_eq!(*db.get(key).await.unwrap().unwrap(), value);

        //? Asserts that the plaintext value does not exist in the underlying memory db.
        let memory_keys = memory_db.keys().unwrap();
        for memory_key in memory_keys {
            assert_ne!(memory_key, key);
            assert_ne!(*memory_db.get(&memory_key).await.unwrap().unwrap(), value);
        }
    }

    /// Every record the backend holds, header excluded, so a test can assert that rotating a
    /// credential left the records themselves untouched.
    async fn records(backend: &Arc<MemoryDatabase>) -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut records = Vec::new();
        for key in backend.keys().unwrap() {
            if key == HEADER_KEY {
                continue;
            }
            let value = backend.get(&key).await.unwrap().unwrap();
            records.push((key, value.to_vec()));
        }
        records.sort();
        records
    }

    /// The property the data key exists for: a credential change rewraps one slot and leaves
    /// every record byte-for-byte as it was.
    #[tokio::test]
    async fn rotating_a_password_does_not_touch_records() {
        let backend = Arc::new(MemoryDatabase::new());
        let db = EncryptedDatabase::create(backend.clone(), PASSWORD)
            .await
            .unwrap();
        db.put(b"pk", b"secret").await.unwrap();
        let before = records(&backend).await;

        db.change_password(NEXT_PASSWORD).await.unwrap();
        assert_eq!(db.slot_kinds().await.unwrap(), vec![PASSWORD_SLOT_KIND]);
        drop(db);

        assert_eq!(records(&backend).await, before, "records were rewritten");

        let reopened = EncryptedDatabase::unlock(backend.clone(), NEXT_PASSWORD)
            .await
            .unwrap();
        assert_eq!(*reopened.get(b"pk").await.unwrap().unwrap(), b"secret"[..]);

        let err = EncryptedDatabase::unlock(backend, PASSWORD)
            .await
            .err()
            .unwrap();
        assert!(
            matches!(err, EncryptedDatabaseError::InvalidPassword),
            "the revoked password still opens the store: {err:?}"
        );
    }

    /// A store carrying two slots of one kind opens under either, which is what lets a
    /// second credential be enrolled before the first is revoked.
    #[tokio::test]
    async fn every_slot_recovers_the_same_data_key() {
        let backend = Arc::new(MemoryDatabase::new());
        let db = EncryptedDatabase::create(backend.clone(), PASSWORD)
            .await
            .unwrap();
        db.put(b"pk", b"secret").await.unwrap();

        let extra = PasswordKeySource::new(NEXT_PASSWORD)
            .unwrap()
            .wrap(&db.data_key)
            .await
            .unwrap();
        let inner: Arc<dyn Database> = backend.clone();
        let mut header = read_header(&inner).await.unwrap();
        header.slots.push(extra);
        inner
            .put(HEADER_KEY, &postcard::to_stdvec(&header).unwrap())
            .await
            .unwrap();
        drop(db);

        for password in [PASSWORD, NEXT_PASSWORD] {
            let opened = EncryptedDatabase::unlock(backend.clone(), password)
                .await
                .unwrap();
            assert_eq!(*opened.get(b"pk").await.unwrap().unwrap(), b"secret"[..]);
        }
    }

    /// Rotating a password must not remove a credential of another kind, which is the
    /// failure mode of clearing the slot list wholesale.
    #[tokio::test]
    async fn changing_the_password_leaves_other_slot_kinds_alone() {
        let backend = Arc::new(MemoryDatabase::new());
        let db = EncryptedDatabase::create(backend.clone(), PASSWORD)
            .await
            .unwrap();

        let inner: Arc<dyn Database> = backend.clone();
        let mut header = read_header(&inner).await.unwrap();
        header.slots.push(StoredSlot {
            kind: "some-future-token".to_string(),
            params: vec![0xde, 0xad],
            wrapped: vec![0x00; 64],
        });
        inner
            .put(HEADER_KEY, &postcard::to_stdvec(&header).unwrap())
            .await
            .unwrap();

        db.change_password(NEXT_PASSWORD).await.unwrap();

        assert_eq!(
            db.slot_kinds().await.unwrap(),
            vec!["some-future-token", PASSWORD_SLOT_KIND],
            "the hardware slot was dropped by a password change",
        );
    }

    /// A build that has never heard of a slot kind still unlocks through one it knows.
    #[tokio::test]
    async fn a_slot_of_an_unknown_kind_does_not_prevent_unlocking() {
        let backend = Arc::new(MemoryDatabase::new());
        let db = EncryptedDatabase::create(backend.clone(), PASSWORD)
            .await
            .unwrap();
        db.put(b"pk", b"secret").await.unwrap();
        drop(db);

        let inner: Arc<dyn Database> = backend.clone();
        let mut header = read_header(&inner).await.unwrap();
        header.slots.insert(
            0,
            StoredSlot {
                kind: "some-future-token".to_string(),
                params: vec![0xde, 0xad, 0xbe, 0xef],
                wrapped: vec![0x00; 64],
            },
        );
        inner
            .put(HEADER_KEY, &postcard::to_stdvec(&header).unwrap())
            .await
            .unwrap();

        let opened = EncryptedDatabase::unlock(backend, PASSWORD).await.unwrap();
        assert_eq!(*opened.get(b"pk").await.unwrap().unwrap(), b"secret"[..]);
    }

    /// A store holding no slot this build can read is reported as such, rather than as a bad
    /// password.
    #[tokio::test]
    async fn a_store_with_no_readable_slot_says_so() {
        let backend = Arc::new(MemoryDatabase::new());
        let db = EncryptedDatabase::create(backend.clone(), PASSWORD)
            .await
            .unwrap();
        drop(db);

        let inner: Arc<dyn Database> = backend.clone();
        let mut header = read_header(&inner).await.unwrap();
        header.slots[0].kind = "some-future-token".to_string();
        inner
            .put(HEADER_KEY, &postcard::to_stdvec(&header).unwrap())
            .await
            .unwrap();

        let err = EncryptedDatabase::unlock(backend, PASSWORD)
            .await
            .err()
            .unwrap();
        assert!(
            matches!(
                err,
                EncryptedDatabaseError::NoMatchingSlot(PASSWORD_SLOT_KIND)
            ),
            "expected NoMatchingSlot, got {err:?}"
        );
    }
}
