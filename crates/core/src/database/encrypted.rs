//! Encryption at rest for any [`Database`].
//!
//! [`EncryptedDatabase`] wraps a backend and encrypts every record. A random data key is
//! wrapped by credential slots in the header (`argon2id-password` today). Record keys and
//! blinded storage keys are derived from it with HKDF-SHA256; values are sealed with
//! XChaCha20-Poly1305. Changing a password rewraps one slot and leaves records untouched.
//!
//! The scheme and its known costs are in `spec/01-architecture.md`.

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

/// Header and ciphertext format version. New slot kinds use [`StoredSlot::kind`], not a bump.
const STORE_VERSION: u8 = 2;

/// Unblinded key for the header, which must be readable before any key material exists.
const HEADER_KEY: &[u8] = b"edw:keystore:v1";

const HEADER_MAGIC: [u8; 8] = *b"EDWSTORE";

const RECORD_KEY_INFO: &[u8] = b"edw:record-key:v1";

const STORAGE_KEY_INFO: &[u8] = b"edw:storage-key:v1";

const RECORD_AAD_DOMAIN: &[u8] = b"edw:record:";

const SLOT_AAD_DOMAIN: &[u8] = b"edw:slot:";

/// Persisted kind tag for [`PasswordKeySource`]. Renaming it orphans existing stores.
const PASSWORD_SLOT_KIND: &str = "argon2id-password";

const SALT_LEN: usize = 16;

const KEY_LEN: usize = 32;

const NONCE_LEN: usize = 24;

/// Argon2id costs from `spec/01-architecture.md`: 64 MiB, 3 passes, one lane.
const ARGON2_M_COST: u32 = 64 * 1024;

const ARGON2_T_COST: u32 = 3;

const ARGON2_P_COST: u32 = 1;

/// A credential that can wrap and recover the store's [`DataKey`].
///
/// [`PasswordKeySource`] is the only implementation today. The trait is async so a hardware
/// token can be a second kind without a redesign.
#[async_trait::async_trait]
pub(crate) trait KeySource: Send + Sync {
    fn kind(&self) -> &'static str;

    async fn wrap(&self, data_key: &DataKey) -> Result<StoredSlot, EncryptedDatabaseError>;

    async fn unwrap(&self, slot: &StoredSlot) -> Result<DataKey, EncryptedDatabaseError>;
}

/// Root key for record derivation.
///
/// Deliberately not `Debug`, `Clone`, or `Serialize`.
#[derive(ZeroizeOnDrop)]
pub(crate) struct DataKey([u8; KEY_LEN]);

/// One way of recovering the [`DataKey`], as persisted in the header.
///
/// `params` are opaque except to the named `kind`, so unknown slots can be skipped at unlock.
#[derive(Serialize, Deserialize)]
pub(crate) struct StoredSlot {
    kind: String,
    params: Vec<u8>,
    wrapped: Vec<u8>,
}

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

    pub(crate) fn expose(&self) -> &[u8; KEY_LEN] {
        &self.0
    }
}

impl PasswordKeySource {
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

        // Header params are untrusted. Argon2's own bounds allow multi-GiB / unbounded
        // hashing, so only the costs this version writes are accepted.
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
    /// Initializes a fresh encrypted store over `db`.
    ///
    /// The caller retains the password and is responsible for wiping it.
    pub async fn create(
        db: Arc<dyn Database>,
        password: &[u8],
    ) -> Result<Self, EncryptedDatabaseError> {
        Self::create_with(db, &PasswordKeySource::new(password)?).await
    }

    /// Unlocks an existing encrypted store over `db`.
    pub async fn unlock(
        db: Arc<dyn Database>,
        password: &[u8],
    ) -> Result<Self, EncryptedDatabaseError> {
        Self::unlock_with(db, &PasswordKeySource::new(password)?).await
    }

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

    /// Tries every slot of `source`'s kind, so a second credential of the same kind is not
    /// shadowed.
    pub(crate) async fn unlock_with(
        db: Arc<dyn Database>,
        source: &dyn KeySource,
    ) -> Result<Self, EncryptedDatabaseError> {
        let header = read_header(&db).await?;

        let mut last_error = EncryptedDatabaseError::NoMatchingSlot(source.kind());
        for slot in header.slots.iter().filter(|s| s.kind == source.kind()) {
            match source.unwrap(slot).await {
                Ok(data_key) => return Ok(Self { db, data_key }),
                Err(error) => last_error = error,
            }
        }
        Err(last_error)
    }

    /// Unlocks `db`, or creates a store if it has no header.
    ///
    /// A missing header is treated as empty: leftover records become unreadable. Prefer
    /// [`Self::create`] or [`Self::unlock`] when the intent is known.
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

    /// Rewraps the password slot under `new_password`. Other slot kinds stay enrolled.
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

    /// Kind tag of each header slot, in index order.
    pub async fn slot_kinds(&self) -> Result<Vec<String>, EncryptedDatabaseError> {
        let header = read_header(&self.db).await?;
        Ok(header.slots.into_iter().map(|slot| slot.kind).collect())
    }

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

    fn expand(
        &self,
        domain: &[u8],
        key: &[u8],
        out: &mut [u8],
    ) -> Result<(), EncryptedDatabaseError> {
        // No HKDF salt: the data key is already uniformly random.
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

fn associated_data(key: &[u8]) -> Vec<u8> {
    associated_data_in(RECORD_AAD_DOMAIN, key)
}

fn slot_associated_data(kind: &str) -> Vec<u8> {
    associated_data_in(SLOT_AAD_DOMAIN, kind.as_bytes())
}

/// Domain-separates record AAD from slot AAD. Without the prefix, an unscoped record named
/// `argon2id-password` would share AAD with the password slot.
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

    // Hash into the zeroizing buffer so a derived-then-moved key is not left on the stack.
    let mut wrapping_key = Zeroizing::new([0u8; KEY_LEN]);
    argon2
        .hash_password_into(password, salt, wrapping_key.as_mut())
        .map_err(|_| EncryptedDatabaseError::KeyDerivation)?;
    Ok(wrapping_key)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashSet,
        sync::Arc,
        time::{Duration, Instant},
    };

    use uuid::Uuid;

    use super::*;
    use crate::database::{memory::MemoryDatabase, scoped::ScopedDatabaseExt};

    const PASSWORD: &[u8] = b"correct horse battery staple";
    const NEXT_PASSWORD: &[u8] = b"a different passphrase entirely";

    fn memory() -> Arc<MemoryDatabase> {
        Arc::new(MemoryDatabase::new())
    }

    async fn create(backend: &Arc<MemoryDatabase>) -> EncryptedDatabase {
        EncryptedDatabase::create(backend.clone(), PASSWORD)
            .await
            .unwrap()
    }

    async fn write_header(backend: &Arc<MemoryDatabase>, header: &KeystoreHeader) {
        let inner: Arc<dyn Database> = backend.clone();
        inner
            .put(HEADER_KEY, &postcard::to_stdvec(header).unwrap())
            .await
            .unwrap();
    }

    async fn header(backend: &Arc<MemoryDatabase>) -> KeystoreHeader {
        let inner: Arc<dyn Database> = backend.clone();
        read_header(&inner).await.unwrap()
    }

    async fn push_slot(backend: &Arc<MemoryDatabase>, slot: StoredSlot) {
        let mut header = header(backend).await;
        header.slots.push(slot);
        write_header(backend, &header).await;
    }

    async fn prepend_slot(backend: &Arc<MemoryDatabase>, slot: StoredSlot) {
        let mut header = header(backend).await;
        header.slots.insert(0, slot);
        write_header(backend, &header).await;
    }

    fn future_slot() -> StoredSlot {
        StoredSlot {
            kind: "some-future-token".to_string(),
            params: vec![0xde, 0xad],
            wrapped: vec![0x00; 64],
        }
    }

    async fn record_blobs(backend: &Arc<MemoryDatabase>) -> Vec<(Vec<u8>, Vec<u8>)> {
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

    fn keys_of(backend: &Arc<MemoryDatabase>) -> HashSet<Vec<u8>> {
        backend.keys().unwrap().into_iter().collect()
    }

    fn sole_new_key(before: &HashSet<Vec<u8>>, after: &HashSet<Vec<u8>>) -> Vec<u8> {
        let mut added = after.difference(before);
        let key = added.next().expect("a write should add one key").clone();
        assert!(added.next().is_none(), "a write added more than one key");
        key
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
    }

    #[tokio::test]
    async fn backend_does_not_store_plaintext_or_logical_keys() {
        let backend = memory();
        let store: Arc<dyn Database> = Arc::new(create(&backend).await);
        let vault_id = Uuid::new_v4();

        store.put(b"vaults", b"index").await.unwrap();
        store
            .clone()
            .scoped(vault_id.as_bytes())
            .put(b"pk", b"secret")
            .await
            .unwrap();

        for key in backend.keys().unwrap() {
            assert_ne!(key.as_slice(), b"pk");
            assert_ne!(key.as_slice(), b"vaults");
            assert!(!contains(&key, vault_id.as_bytes()));
            let value = backend.get(&key).await.unwrap().unwrap();
            assert_ne!(&*value, b"secret");
            assert_ne!(&*value, b"index");
        }
    }

    #[tokio::test]
    async fn ciphertext_does_not_decrypt_in_another_scope() {
        let backend = memory();
        let store: Arc<dyn Database> = Arc::new(create(&backend).await);
        let first = store.clone().scoped(Uuid::new_v4().as_bytes());
        let second = store.scoped(Uuid::new_v4().as_bytes());

        let before = keys_of(&backend);
        first.put(b"pk", b"first secret").await.unwrap();
        let first_slot = sole_new_key(&before, &keys_of(&backend));

        let after_first = keys_of(&backend);
        second.put(b"pk", b"second secret").await.unwrap();
        let second_slot = sole_new_key(&after_first, &keys_of(&backend));

        let lifted = backend.get(&first_slot).await.unwrap().unwrap().to_vec();
        backend.put(&second_slot, &lifted).await.unwrap();

        assert!(second.get(b"pk").await.is_err());
    }

    #[tokio::test]
    async fn repeated_writes_produce_distinct_ciphertexts() {
        let backend = memory();
        let store = create(&backend).await;

        let before = keys_of(&backend);
        store.put(b"pk", b"secret").await.unwrap();
        let slot = sole_new_key(&before, &keys_of(&backend));

        let first = backend.get(&slot).await.unwrap().unwrap().to_vec();
        store.put(b"pk", b"secret").await.unwrap();
        let second = backend.get(&slot).await.unwrap().unwrap().to_vec();

        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn tampered_record_fails_to_open() {
        let backend = memory();
        let store = create(&backend).await;

        let before = keys_of(&backend);
        store.put(b"pk", b"secret").await.unwrap();
        let slot = sole_new_key(&before, &keys_of(&backend));

        let mut blob = backend.get(&slot).await.unwrap().unwrap().to_vec();
        let last = blob.len() - 1;
        blob[last] ^= 0x01;
        backend.put(&slot, &blob).await.unwrap();

        assert!(store.get(b"pk").await.is_err());
    }

    #[tokio::test]
    async fn change_password_does_not_rewrite_records() {
        let backend = memory();
        let db = create(&backend).await;
        db.put(b"pk", b"secret").await.unwrap();
        let before = record_blobs(&backend).await;

        db.change_password(NEXT_PASSWORD).await.unwrap();

        assert_eq!(record_blobs(&backend).await, before);
    }

    #[tokio::test]
    async fn change_password_unlocks_with_the_new_password() {
        let backend = memory();
        let db = create(&backend).await;
        db.put(b"pk", b"secret").await.unwrap();
        db.change_password(NEXT_PASSWORD).await.unwrap();
        drop(db);

        let reopened = EncryptedDatabase::unlock(backend, NEXT_PASSWORD)
            .await
            .unwrap();
        assert_eq!(&*reopened.get(b"pk").await.unwrap().unwrap(), b"secret");
    }

    #[tokio::test]
    async fn change_password_rejects_the_old_password() {
        let backend = memory();
        let db = create(&backend).await;
        db.change_password(NEXT_PASSWORD).await.unwrap();
        drop(db);

        let err = EncryptedDatabase::unlock(backend, PASSWORD)
            .await
            .err()
            .unwrap();
        assert!(matches!(err, EncryptedDatabaseError::InvalidPassword));
    }

    #[tokio::test]
    async fn either_password_slot_unlocks_the_store() {
        let backend = memory();
        let db = create(&backend).await;
        db.put(b"pk", b"secret").await.unwrap();

        let extra = PasswordKeySource::new(NEXT_PASSWORD)
            .unwrap()
            .wrap(&db.data_key)
            .await
            .unwrap();
        push_slot(&backend, extra).await;
        drop(db);

        for password in [PASSWORD, NEXT_PASSWORD] {
            let opened = EncryptedDatabase::unlock(backend.clone(), password)
                .await
                .unwrap();
            assert_eq!(&*opened.get(b"pk").await.unwrap().unwrap(), b"secret");
        }
    }

    #[tokio::test]
    async fn change_password_keeps_other_slot_kinds() {
        let backend = memory();
        let db = create(&backend).await;
        push_slot(&backend, future_slot()).await;

        db.change_password(NEXT_PASSWORD).await.unwrap();

        assert_eq!(
            db.slot_kinds().await.unwrap(),
            vec!["some-future-token", PASSWORD_SLOT_KIND],
        );
    }

    #[tokio::test]
    async fn unknown_slot_kind_does_not_block_unlock() {
        let backend = memory();
        let db = create(&backend).await;
        db.put(b"pk", b"secret").await.unwrap();
        drop(db);

        prepend_slot(&backend, future_slot()).await;

        let opened = EncryptedDatabase::unlock(backend, PASSWORD).await.unwrap();
        assert_eq!(&*opened.get(b"pk").await.unwrap().unwrap(), b"secret");
    }

    #[tokio::test]
    async fn store_with_no_readable_slot_reports_no_matching_slot() {
        let backend = memory();
        let db = create(&backend).await;
        drop(db);

        let mut header = header(&backend).await;
        header.slots[0].kind = "some-future-token".to_string();
        write_header(&backend, &header).await;

        let err = EncryptedDatabase::unlock(backend, PASSWORD)
            .await
            .err()
            .unwrap();
        assert!(matches!(
            err,
            EncryptedDatabaseError::NoMatchingSlot(PASSWORD_SLOT_KIND)
        ));
    }

    #[tokio::test]
    async fn unwrap_rejects_parameters_this_build_did_not_write() {
        let source = PasswordKeySource::new(PASSWORD).unwrap();
        let cases = [
            (ARGON2_M_COST + 1, ARGON2_T_COST, ARGON2_P_COST, SALT_LEN),
            (ARGON2_M_COST, ARGON2_T_COST + 1, ARGON2_P_COST, SALT_LEN),
            (ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST + 1, SALT_LEN),
            (ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST, SALT_LEN - 1),
        ];

        for (m_cost, t_cost, p_cost, salt_len) in cases {
            let slot = StoredSlot {
                kind: PASSWORD_SLOT_KIND.to_string(),
                params: postcard::to_stdvec(&Argon2idParams {
                    m_cost,
                    t_cost,
                    p_cost,
                    salt: vec![0; salt_len],
                })
                .unwrap(),
                wrapped: vec![0; 64],
            };

            let started = Instant::now();
            let err = source.unwrap(&slot).await.err().unwrap();
            assert!(
                matches!(err, EncryptedDatabaseError::UnsupportedParameters),
                "{err:?}"
            );
            assert!(
                started.elapsed() < Duration::from_millis(500),
                "parameters were hashed before they were rejected"
            );
        }
    }
}
