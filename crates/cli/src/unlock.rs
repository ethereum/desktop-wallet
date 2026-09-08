use std::{
    fs,
    io::{BufRead, IsTerminal, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context;
use edw_core::database::{Database, encrypted::EncryptedDatabase, file::FileDatabase};
use zeroize::Zeroizing;

use crate::{GlobalArgs, session};

/// Where a script may pass the decryption password.
///
/// Named for what it is so nobody mistakes it for an account password or an RPC credential.
/// An environment variable is readable by anything running as the user, so this exists for
/// automation and the terminal session is what interactive use should rely on.
const PASSWORD_ENV: &str = "EDW_DECRYPTION_PASSWORD";
const UNLOCK_FILE: &str = ".unlock";

pub(crate) fn network_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("network")
}

fn unlock_path(data_dir: &Path) -> PathBuf {
    data_dir.join(UNLOCK_FILE)
}

/// Ensures this data dir has a wallet password and returns it.
///
/// First run writes `{data_dir}/.unlock` and never creates a network store. Later calls
/// verify against that file before any `EncryptedDatabase` is opened.
///
/// # Errors
/// Wrong password, I/O, or an empty password on first run.
pub(crate) async fn ensure_unlocked(data_dir: &Path) -> Result<Zeroizing<String>, anyhow::Error> {
    let (provided, persist_session) = match env_password()? {
        Some(password) => (Some(password), false),
        None => match session::load() {
            Some(password) => (Some(password), false),
            None => (None, true),
        },
    };

    let password = ensure_unlocked_inner(data_dir, provided).await?;
    if persist_session {
        let _ = session::store(&password);
    }
    Ok(password)
}

async fn ensure_unlocked_inner(
    data_dir: &Path,
    provided: Option<Zeroizing<String>>,
) -> Result<Zeroizing<String>, anyhow::Error> {
    let path = unlock_path(data_dir);
    if path.exists() {
        let blob = fs::read(&path).with_context(|| format!("error reading {}", path.display()))?;
        let password = match provided {
            Some(password) => password,
            None => prompt("Decryption password: ")?,
        };
        EncryptedDatabase::check_password_verifier(&blob, password.as_bytes())
            .context("incorrect password")?;
        return Ok(password);
    }

    if let Some(store_dir) = find_existing_store(data_dir).await? {
        let password = match provided {
            Some(password) => password,
            None => prompt("Decryption password: ")?,
        };
        let backend = Arc::new(
            FileDatabase::open(&store_dir)
                .with_context(|| format!("error opening {}", store_dir.display()))?,
        );
        EncryptedDatabase::unlock(backend, password.as_bytes())
            .await
            .context("incorrect password")?;
        write_unlock_file(data_dir, &password)?;
        return Ok(password);
    }

    let password = match provided {
        Some(password) => {
            if password.is_empty() {
                anyhow::bail!("the decryption password cannot be empty; nothing was created");
            }
            password
        }
        None => setup(data_dir)?,
    };
    write_unlock_file(data_dir, &password)?;
    Ok(password)
}

fn write_unlock_file(data_dir: &Path, password: &str) -> Result<(), anyhow::Error> {
    fs::create_dir_all(data_dir)
        .with_context(|| format!("error creating {}", data_dir.display()))?;
    let blob = EncryptedDatabase::create_password_verifier(password.as_bytes())?;
    let path = unlock_path(data_dir);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("error creating {}", path.display()))?;
    file.write_all(&blob)
        .with_context(|| format!("error writing {}", path.display()))?;
    Ok(())
}

async fn find_existing_store(data_dir: &Path) -> Result<Option<PathBuf>, anyhow::Error> {
    let network = network_dir(data_dir);
    if header_at(&network).await? {
        return Ok(Some(network));
    }

    let entries = match fs::read_dir(data_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("error listing the data directory"),
    };

    for entry in entries {
        let entry = entry.context("error listing the data directory")?;
        if !entry
            .file_type()
            .context("error listing the data directory")?
            .is_dir()
        {
            continue;
        }
        if entry.path() == network {
            continue;
        }
        let db = entry.path().join("db");
        if header_at(&db).await? {
            return Ok(Some(db));
        }
    }

    Ok(None)
}

async fn header_at(dir: &Path) -> Result<bool, anyhow::Error> {
    if !dir.exists() {
        return Ok(false);
    }
    let backend =
        FileDatabase::open(dir).with_context(|| format!("error opening {}", dir.display()))?;
    Ok(EncryptedDatabase::has_header(&backend).await?)
}

/// Opens the network store, creating it under the wallet password if it does not exist.
pub(crate) async fn network_store(data_dir: &Path) -> Result<Arc<dyn Database>, anyhow::Error> {
    encrypted_store(data_dir, &network_dir(data_dir)).await
}

/// Opens the network store only if it already has an encrypted header.
pub(crate) async fn try_network_store(
    data_dir: &Path,
) -> Result<Option<Arc<dyn Database>>, anyhow::Error> {
    let dir = network_dir(data_dir);
    if !header_at(&dir).await? {
        return Ok(None);
    }
    Ok(Some(encrypted_store(data_dir, &dir).await?))
}

pub(crate) async fn profile_store(
    name: &str,
    data_dir: &Path,
) -> Result<Arc<dyn Database>, anyhow::Error> {
    encrypted_store(data_dir, &data_dir.join(name).join("db")).await
}

async fn encrypted_store(data_dir: &Path, dir: &Path) -> Result<Arc<dyn Database>, anyhow::Error> {
    let password = ensure_unlocked(data_dir).await?;
    let backend: Arc<dyn Database> = Arc::new(
        FileDatabase::open(dir)
            .with_context(|| format!("error opening the store at {}", dir.display()))?,
    );
    let store = EncryptedDatabase::open_or_create(backend, password.as_bytes())
        .await
        .context("error opening the store")?;
    Ok(Arc::new(store))
}

fn env_password() -> Result<Option<Zeroizing<String>>, anyhow::Error> {
    let Some(value) = std::env::var_os(PASSWORD_ENV) else {
        return Ok(None);
    };
    let value = value
        .into_string()
        .map_err(|_| anyhow::anyhow!("{PASSWORD_ENV} is not valid UTF-8"))?;
    Ok(Some(Zeroizing::new(value)))
}

/// Walks a first run through choosing a decryption password.
fn setup(data_dir: &Path) -> Result<Zeroizing<String>, anyhow::Error> {
    println!("No wallet password is set at {}.", data_dir.display());
    println!("A decryption password must be set up before anything can be stored.");
    println!("There is no recovery path if it is lost: the data is encrypted under it alone.");

    let password = prompt("New decryption password: ")?;
    if password.is_empty() {
        anyhow::bail!("the decryption password cannot be empty; nothing was created");
    }

    // Only a typed password can hold a typo worth catching; a redirected stdin would just be
    // repeating itself.
    if std::io::stdin().is_terminal() {
        let confirmation = prompt("Confirm decryption password: ")?;
        if password.as_str() != confirmation.as_str() {
            anyhow::bail!("the passwords do not match; nothing was created");
        }
    }

    Ok(password)
}

/// Reads a password without echoing it when attached to a terminal.
///
/// With stdin redirected there is no terminal to suppress echo on, so the password is read as
/// a plain line. That is what makes the command scriptable and testable; it is not a weaker
/// path, because a redirected stdin was never being echoed to begin with.
pub(crate) fn prompt(label: &str) -> Result<Zeroizing<String>, anyhow::Error> {
    if std::io::stdin().is_terminal() {
        return Ok(Zeroizing::new(
            rpassword::prompt_password(label).context("error reading the decryption password")?,
        ));
    }

    let mut password = Zeroizing::new(String::new());
    std::io::stdin()
        .lock()
        .read_line(&mut password)
        .context("error reading the decryption password from stdin")?;
    Ok(Zeroizing::new(
        password.trim_end_matches(['\r', '\n']).to_string(),
    ))
}

pub(crate) async fn run_unlock(global: &GlobalArgs) -> Result<(), anyhow::Error> {
    let path = unlock_path(&global.data_dir);
    let existed = path.exists();
    let was_unlocked = session::load().is_some();

    ensure_unlocked(&global.data_dir).await?;

    if !existed {
        println!("Wallet password set for {}.", global.data_dir.display());
    }

    if was_unlocked {
        println!("Already unlocked for this terminal.");
    } else if session::available() {
        println!("Unlocked for this terminal. Run `edw lock` to end the session.");
    } else {
        println!(
            "Password accepted, but this terminal cannot hold a session, so the next command \
             will ask again. A session needs a controlling terminal and XDG_RUNTIME_DIR."
        );
    }

    Ok(())
}

pub(crate) fn run_lock() -> Result<(), anyhow::Error> {
    if session::clear()? {
        println!("Locked.");
    } else {
        println!("Not unlocked; nothing to do.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_data_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    #[tokio::test]
    async fn first_unlock_writes_only_the_verifier_file() {
        let dir = temp_data_dir();
        let password = Zeroizing::new(String::from("test-password"));
        ensure_unlocked_inner(dir.path(), Some(password))
            .await
            .expect("first run");

        assert!(unlock_path(dir.path()).exists());
        assert!(!network_dir(dir.path()).exists());
        let entries: Vec<_> = fs::read_dir(dir.path())
            .expect("list")
            .map(|e| e.expect("entry").file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from(UNLOCK_FILE)]);
    }

    #[tokio::test]
    async fn second_unlock_accepts_the_same_password_and_rejects_another() {
        let dir = temp_data_dir();
        let password = Zeroizing::new(String::from("test-password"));
        ensure_unlocked_inner(dir.path(), Some(password.clone()))
            .await
            .expect("first run");

        ensure_unlocked_inner(dir.path(), Some(password))
            .await
            .expect("same password");

        let err = ensure_unlocked_inner(dir.path(), Some(Zeroizing::new(String::from("nope"))))
            .await
            .expect_err("wrong password");
        assert!(err.to_string().contains("incorrect password"));
    }

    #[tokio::test]
    async fn a_new_store_uses_the_wallet_password() {
        let dir = temp_data_dir();
        let password = Zeroizing::new(String::from("test-password"));
        ensure_unlocked_inner(dir.path(), Some(password.clone()))
            .await
            .expect("first run");

        // encrypted_store would call ensure_unlocked (env/session). Drive the store
        // with the already-verified password instead.
        let backend: Arc<dyn Database> =
            Arc::new(FileDatabase::open(dir.path().join("alice").join("db")).expect("open"));
        EncryptedDatabase::open_or_create(backend, password.as_bytes())
            .await
            .expect("create with wallet password");

        assert!(unlock_path(dir.path()).exists());
        let listed: Vec<_> = fs::read_dir(dir.path())
            .expect("list")
            .map(|e| e.expect("entry").file_name())
            .collect();
        assert!(listed.iter().any(|n| n == UNLOCK_FILE));
        assert!(!listed.iter().any(|n| n == "network"));
    }

    #[tokio::test]
    async fn missing_unlock_file_adopts_an_existing_store_password() {
        let dir = temp_data_dir();
        let password = Zeroizing::new(String::from("legacy-password"));
        let backend = Arc::new(FileDatabase::open(network_dir(dir.path())).expect("open"));
        EncryptedDatabase::create(backend, password.as_bytes())
            .await
            .expect("legacy store");
        assert!(!unlock_path(dir.path()).exists());

        ensure_unlocked_inner(dir.path(), Some(password))
            .await
            .expect("migrate");
        assert!(unlock_path(dir.path()).exists());

        let err = ensure_unlocked_inner(dir.path(), Some(Zeroizing::new(String::from("other"))))
            .await
            .expect_err("wrong after migrate");
        assert!(err.to_string().contains("incorrect password"));
    }
}
