//! Encryption at rest for any [`Database`].
//!
//! [`EncryptedDatabase`] is a decorator: it wraps an arbitrary backend and encrypts every
//! record written through it. Encryption is therefore a property of the storage seam rather
//! than of each repository that happens to remember to ask for it, and this module is the
//! only place in the workspace that performs cryptography on stored data.
//!
//! The scheme, versioned by [`RECORD_VERSION`]:
//!
//! - The root of trust is the user's password. Argon2id (64 MiB, 3 passes) stretches it into
//!   a master key. No OS keychain or secure enclave is involved.
//! - Every record gets its own key and its own blinded storage key, both derived from the
//!   master key with HKDF-SHA256 over the record's full logical key. Because a
//!   [`super::scoped::ScopedDatabase`] prefix is part of that logical key, each vault and
//!   executor lands in a keyspace that is cryptographically isolated rather than merely
//!   prefixed: a record lifted from one scope will not decrypt in another.
//! - Values are sealed with XChaCha20-Poly1305 under a random 192-bit nonce, with the
//!   version and logical key bound in as associated data.
//! - Storage keys are blinded, so a backend never sees a logical key name. This hides the
//!   names only. Record count and ciphertext length are not hidden: a backend storing one
//!   file per record discloses how many records exist, and a length of
//!   `1 + 24 + plaintext + 16` distinguishes a 32-byte signing key from a 20-byte address.
//!   Closing that would need fixed-size padding and a layout that does not leak cardinality.
//!   The cost of blinding is that prefix iteration over the backend is not possible; the
//!   [`Database`] trait exposes no such operation.

use std::sync::Arc;

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, AeadCore, OsRng, Payload, rand_core::RngCore},
};
use edw_core::database::{Database, DatabaseError};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::{ZeroizeOnDrop, Zeroizing};

/// Format version of both the header and every record blob.
const RECORD_VERSION: u8 = 1;

/// Plaintext key the header lives under. It must stay unblinded: it is read before any key
/// material exists.
const HEADER_KEY: &[u8] = b"edw:keystore:v1";
const HEADER_MAGIC: [u8; 8] = *b"EDWSTORE";

const RECORD_KEY_INFO: &[u8] = b"edw:record-key:v1";
const STORAGE_KEY_INFO: &[u8] = b"edw:storage-key:v1";
const VERIFIER_KEY: &[u8] = b"edw:verifier:v1";
const VERIFIER_PLAINTEXT: &[u8] = b"edw:unlocked:v1";

const SALT_LEN: usize = 16;
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;

/// Argon2id cost parameters. 64 MiB and 3 passes, as specced in `spec/01-architecture.md`.
const ARGON2_M_COST: u32 = 64 * 1024;
const ARGON2_T_COST: u32 = 3;
const ARGON2_P_COST: u32 = 1;

/// A password-derived master key. Deliberately not `Debug`, `Clone`, or `Serialize`: it must
/// not be copyable into a log line or a stored record.
#[derive(ZeroizeOnDrop)]
struct MasterKey([u8; KEY_LEN]);

#[derive(Serialize, Deserialize)]
struct KeystoreHeader {
    magic: [u8; 8],
    version: u8,
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
    salt: Vec<u8>,
    verifier: Vec<u8>,
}

/// Encrypts every record written to an inner [`Database`].
///
/// Construct with [`EncryptedDatabase::create`] for a fresh store, or
/// [`EncryptedDatabase::unlock`] for an existing one.
pub struct EncryptedDatabase {
    db: Arc<dyn Database>,
    master: MasterKey,
    salt: Vec<u8>,
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
    #[error("unsupported format version {0}, expected {RECORD_VERSION}")]
    UnsupportedVersion(u8),
    #[error("header declares key-derivation parameters this build does not accept")]
    UnsupportedParameters,
    #[error("password must not be empty")]
    EmptyPassword,
    #[error("key derivation failed")]
    KeyDerivation,
    #[error("header serialization error: {0}")]
    Serialization(#[from] postcard::Error),
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
        if password.is_empty() {
            return Err(EncryptedDatabaseError::EmptyPassword);
        }
        if db.get(HEADER_KEY).await?.is_some() {
            return Err(EncryptedDatabaseError::AlreadyInitialized);
        }

        let mut salt = vec![0u8; SALT_LEN];
        OsRng.fill_bytes(&mut salt);
        let master =
            derive_master_key(password, &salt, ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST)?;

        let store = Self {
            db,
            master,
            salt: salt.clone(),
        };
        let verifier = store.seal(VERIFIER_KEY, VERIFIER_PLAINTEXT)?;

        let header = KeystoreHeader {
            magic: HEADER_MAGIC,
            version: RECORD_VERSION,
            m_cost: ARGON2_M_COST,
            t_cost: ARGON2_T_COST,
            p_cost: ARGON2_P_COST,
            salt,
            verifier,
        };
        store
            .db
            .put(HEADER_KEY, &postcard::to_stdvec(&header)?)
            .await?;
        Ok(store)
    }

    /// Unlocks an existing encrypted store over `db`.
    ///
    /// # Errors
    /// Returns [`EncryptedDatabaseError::InvalidPassword`] when the header's verifier fails
    /// its authentication tag, so a wrong password is rejected up front rather than on the
    /// first read of a real record. Returns
    /// [`EncryptedDatabaseError::UnsupportedParameters`] if the header's key-derivation
    /// parameters are not the ones this version writes, which is how a corrupt or tampered
    /// header is rejected before any expensive work is done.
    pub async fn unlock(
        db: Arc<dyn Database>,
        password: &[u8],
    ) -> Result<Self, EncryptedDatabaseError> {
        if password.is_empty() {
            return Err(EncryptedDatabaseError::EmptyPassword);
        }

        let Some(bytes) = db.get(HEADER_KEY).await? else {
            return Err(EncryptedDatabaseError::NotInitialized);
        };
        let header: KeystoreHeader = postcard::from_bytes(&bytes)?;

        if header.magic != HEADER_MAGIC {
            return Err(EncryptedDatabaseError::Corrupt);
        }
        if header.version != RECORD_VERSION {
            return Err(EncryptedDatabaseError::UnsupportedVersion(header.version));
        }
        // The header is untrusted input: it is plaintext, so anything able to write the store
        // can choose these. Argon2's own bounds are far too loose to lean on (`MAX_M_COST` is
        // `u32::MAX` KiB), so an unchecked header turns a corrupt or hostile file into an
        // out-of-memory abort or an unbounded hang. Only the parameters this version writes
        // are accepted; a future cost change travels with a version bump.
        if header.m_cost != ARGON2_M_COST
            || header.t_cost != ARGON2_T_COST
            || header.p_cost != ARGON2_P_COST
            || header.salt.len() != SALT_LEN
        {
            return Err(EncryptedDatabaseError::UnsupportedParameters);
        }

        let master = derive_master_key(
            password,
            &header.salt,
            header.m_cost,
            header.t_cost,
            header.p_cost,
        )?;
        let store = Self {
            db,
            master,
            salt: header.salt,
        };

        store
            .open(VERIFIER_KEY, &header.verifier)
            .map_err(|_| EncryptedDatabaseError::InvalidPassword)?;
        Ok(store)
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

    fn expand(
        &self,
        domain: &[u8],
        key: &[u8],
        out: &mut [u8],
    ) -> Result<(), EncryptedDatabaseError> {
        let hkdf = Hkdf::<Sha256>::new(Some(&self.salt), &self.master.0);
        hkdf.expand_multi_info(&[domain, key], out)
            .map_err(|_| EncryptedDatabaseError::KeyDerivation)
    }

    fn seal(&self, key: &[u8], value: &[u8]) -> Result<Vec<u8>, EncryptedDatabaseError> {
        let record_key = self.record_key(key)?;
        let cipher = XChaCha20Poly1305::new_from_slice(record_key.as_ref())
            .map_err(|_| EncryptedDatabaseError::KeyDerivation)?;

        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let aad = associated_data(key);
        let ciphertext = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: value,
                    aad: &aad,
                },
            )
            .map_err(|_| EncryptedDatabaseError::Corrupt)?;

        let mut blob = Vec::with_capacity(1 + NONCE_LEN + ciphertext.len());
        blob.push(RECORD_VERSION);
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ciphertext);
        Ok(blob)
    }

    fn open(&self, key: &[u8], blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, EncryptedDatabaseError> {
        let Some((&version, rest)) = blob.split_first() else {
            return Err(EncryptedDatabaseError::Corrupt);
        };
        if version != RECORD_VERSION {
            return Err(EncryptedDatabaseError::UnsupportedVersion(version));
        }
        if rest.len() < NONCE_LEN {
            return Err(EncryptedDatabaseError::Corrupt);
        }
        let (nonce, ciphertext) = rest.split_at(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = nonce
            .try_into()
            .map_err(|_| EncryptedDatabaseError::Corrupt)?;

        let record_key = self.record_key(key)?;
        let cipher = XChaCha20Poly1305::new_from_slice(record_key.as_ref())
            .map_err(|_| EncryptedDatabaseError::KeyDerivation)?;

        let aad = associated_data(key);
        let plaintext = cipher
            .decrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| EncryptedDatabaseError::Corrupt)?;
        Ok(Zeroizing::new(plaintext))
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

/// Binds the format version and the record's logical key into the AEAD tag, so a ciphertext
/// cannot be replayed under a different key even if key derivation were weakened.
fn associated_data(key: &[u8]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(1 + key.len());
    aad.push(RECORD_VERSION);
    aad.extend_from_slice(key);
    aad
}

fn derive_master_key(
    password: &[u8],
    salt: &[u8],
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
) -> Result<MasterKey, EncryptedDatabaseError> {
    let params = Params::new(m_cost, t_cost, p_cost, Some(KEY_LEN))
        .map_err(|_| EncryptedDatabaseError::KeyDerivation)?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    //? Hash directly into the MasterKey. Deriving into a local and moving it would leave an
    //? un-zeroized copy of the key on the stack.
    let mut master = MasterKey([0u8; KEY_LEN]);
    argon2
        .hash_password_into(password, salt, &mut master.0)
        .map_err(|_| EncryptedDatabaseError::KeyDerivation)?;
    Ok(master)
}

impl From<EncryptedDatabaseError> for DatabaseError {
    fn from(err: EncryptedDatabaseError) -> Self {
        DatabaseError::Other(Box::new(err))
    }
}
