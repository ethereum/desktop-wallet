use std::{
    fs,
    io::{BufRead, IsTerminal},
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

pub(crate) fn network_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("network")
}

/// Whether an encrypted store already exists, checked before anything can create one.
pub(crate) fn is_initialized(dir: &Path) -> bool {
    fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some())
}

pub(crate) async fn network_store(data_dir: &Path) -> Result<Arc<dyn Database>, anyhow::Error> {
    let dir = network_dir(data_dir);
    let initialized = is_initialized(&dir);
    let (password, prompted) = password(initialized, &dir)?;

    let backend: Arc<dyn Database> = Arc::new(
        FileDatabase::open(&dir)
            .with_context(|| format!("error opening the store at {}", dir.display()))?,
    );

    let store = if initialized {
        EncryptedDatabase::unlock(backend, password.as_bytes())
            .await
            .context("error unlocking the store")?
    } else {
        EncryptedDatabase::create(backend, password.as_bytes())
            .await
            .context("error creating the store")?
    };

    if prompted {
        let _ = session::store(&password);
    }

    Ok(Arc::new(store))
}

/// The decryption password, and whether it came from a prompt rather than an existing source.
fn password(initialized: bool, dir: &Path) -> Result<(Zeroizing<String>, bool), anyhow::Error> {
    if let Some(value) = std::env::var_os(PASSWORD_ENV) {
        let value = value
            .into_string()
            .map_err(|_| anyhow::anyhow!("{PASSWORD_ENV} is not valid UTF-8"))?;
        return Ok((Zeroizing::new(value), false));
    }

    if let Some(password) = session::load() {
        return Ok((password, false));
    }

    if initialized {
        Ok((prompt("Decryption password: ")?, true))
    } else {
        Ok((setup(dir)?, true))
    }
}

/// Walks a first run through choosing a decryption password.
fn setup(dir: &Path) -> Result<Zeroizing<String>, anyhow::Error> {
    println!("No encrypted store exists at {}.", dir.display());
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
fn prompt(label: &str) -> Result<Zeroizing<String>, anyhow::Error> {
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
    let dir = network_dir(&global.data_dir);
    let existed = is_initialized(&dir);
    let was_unlocked = session::load().is_some();

    drop(network_store(&global.data_dir).await?);

    if !existed {
        println!("Encrypted store created at {}.", dir.display());
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
