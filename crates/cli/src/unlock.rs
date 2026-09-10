use std::{
    fs,
    io::{BufRead, IsTerminal},
    path::{Path, PathBuf},
};

use anyhow::Context;
use zeroize::Zeroizing;

use crate::{GlobalArgs, session, store};

/// Where a script may pass the decryption password.
///
/// Named for what it is so nobody mistakes it for an account password or an RPC credential.
/// An environment variable is readable by anything running as the user, so this exists for
/// automation and the terminal session is what interactive use should rely on.
const PASSWORD_ENV: &str = "EDW_DECRYPTION_PASSWORD";

pub(crate) const UNLOCK_DIR: &str = "unlock";

pub(crate) fn unlock_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(UNLOCK_DIR)
}

/// Whether an encrypted store already exists, checked before anything can create one.
pub(crate) fn is_initialized(dir: &Path) -> bool {
    fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some())
}

/// Ensures this data dir has a wallet credential and returns the password.
///
/// First run creates `{data_dir}/unlock/` as an [`edw_core::database::encrypted::EncryptedDatabase`].
/// Later calls unlock that store. Profile and network stores are not created here.
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
    let dir = unlock_dir(data_dir);
    let initialized = is_initialized(&dir);
    let password = match provided {
        Some(password) => {
            if !initialized && password.is_empty() {
                anyhow::bail!("the decryption password cannot be empty; nothing was created");
            }
            password
        }
        None => {
            if initialized {
                prompt("Decryption password: ")?
            } else {
                setup(data_dir)?
            }
        }
    };

    let opened = store::open_store(&dir, password.as_bytes()).await;
    if initialized {
        drop(opened.context("incorrect password")?);
    } else {
        drop(opened?);
    }
    Ok(password)
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
    let dir = unlock_dir(&global.data_dir);
    let existed = is_initialized(&dir);
    let was_unlocked = session::load().is_some();

    drop(ensure_unlocked(&global.data_dir).await?);

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
    async fn first_unlock_writes_only_the_unlock_store() {
        let dir = temp_data_dir();
        let password = Zeroizing::new(String::from("test-password"));
        ensure_unlocked_inner(dir.path(), Some(password))
            .await
            .expect("first run");

        assert!(is_initialized(&unlock_dir(dir.path())));
        assert!(!dir.path().join("network").exists());
        let entries: Vec<_> = fs::read_dir(dir.path())
            .expect("list")
            .map(|e| e.expect("entry").file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from(UNLOCK_DIR)]);
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

        crate::store::open_store(&dir.path().join("alice").join("db"), password.as_bytes())
            .await
            .expect("create with wallet password");

        let listed: Vec<_> = fs::read_dir(dir.path())
            .expect("list")
            .map(|e| e.expect("entry").file_name())
            .collect();
        assert!(listed.iter().any(|n| n == UNLOCK_DIR));
        assert!(listed.iter().any(|n| n == "alice"));
        assert!(!listed.iter().any(|n| n == "network"));
    }
}
