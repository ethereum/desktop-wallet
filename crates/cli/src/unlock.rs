use std::{
    fs,
    io::{BufRead, IsTerminal},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context;
use clap::Args;
use edw_core::{
    database::{
        Database, encrypted::EncryptedDatabase, file::FileDatabase, scoped::ScopedDatabaseExt,
    },
    network::{SupportedNetwork, db::NetworkDb},
};
use zeroize::Zeroizing;

use crate::{GlobalArgs, session};

/// Where a script may pass the decryption password.
///
/// Named for what it is so nobody mistakes it for an account password or an RPC credential.
/// An environment variable is readable by anything running as the user, so this exists for
/// automation and the terminal session is what interactive use should rely on.
const PASSWORD_ENV: &str = "EDW_DECRYPTION_PASSWORD";

#[derive(Args, Debug)]
pub(crate) struct UnlockArgs {
    /// Network to unlock. Unlocks this network and locks every other.
    #[arg(long, default_value = "mainnet")]
    pub(crate) network: SupportedNetwork,
}

pub(crate) fn network_dir(data_dir: &Path, network: SupportedNetwork) -> PathBuf {
    data_dir.join(network.slug())
}

/// Whether an encrypted store already exists, checked before anything can create one.
pub(crate) fn is_initialized(dir: &Path) -> bool {
    fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some())
}

pub(crate) fn locked_error() -> anyhow::Error {
    anyhow::anyhow!("wallet is locked; run `edw unlock --network <mainnet|sepolia|local>`")
}

pub(crate) async fn open_existing_store(
    data_dir: &Path,
    network: SupportedNetwork,
    password: &[u8],
) -> Result<Arc<dyn Database>, anyhow::Error> {
    let dir = network_dir(data_dir, network);
    if !is_initialized(&dir) {
        anyhow::bail!(
            "no wallet instance for {network} at {}; run `edw unlock --network {network}`",
            dir.display()
        );
    }
    let backend: Arc<dyn Database> = Arc::new(
        FileDatabase::open(&dir)
            .with_context(|| format!("error opening the store at {}", dir.display()))?,
    );
    Ok(Arc::new(
        EncryptedDatabase::unlock(backend, password)
            .await
            .context("error unlocking the store")?,
    ))
}

async fn network_store(
    data_dir: &Path,
    network: SupportedNetwork,
) -> Result<session::Session, anyhow::Error> {
    let data_dir = session::canonical_data_dir(data_dir);
    let dir = network_dir(&data_dir, network);
    let initialized = is_initialized(&dir);
    let password = password(initialized, &dir, network, &data_dir)?;

    let backend: Arc<dyn Database> = Arc::new(
        FileDatabase::open(&dir)
            .with_context(|| format!("error opening the store at {}", dir.display()))?,
    );

    let store: Arc<dyn Database> = if initialized {
        Arc::new(
            EncryptedDatabase::unlock(backend, password.as_bytes())
                .await
                .context("error unlocking the store")?,
        )
    } else {
        Arc::new(
            EncryptedDatabase::create(backend, password.as_bytes())
                .await
                .context("error creating the store")?,
        )
    };

    let prefs = store.scoped(b"preferences");
    if prefs.get_network().await?.is_none() {
        prefs
            .put_network(&network.preferences())
            .await
            .context("error seeding network preferences")?;
    }

    Ok(session::Session {
        data_dir,
        network,
        password,
    })
}

/// The decryption password.
fn password(
    initialized: bool,
    dir: &Path,
    network: SupportedNetwork,
    data_dir: &Path,
) -> Result<Zeroizing<String>, anyhow::Error> {
    if let Some(value) = std::env::var_os(PASSWORD_ENV) {
        let value = value
            .into_string()
            .map_err(|_| anyhow::anyhow!("{PASSWORD_ENV} is not valid UTF-8"))?;
        return Ok(Zeroizing::new(value));
    }

    if let Some(session) = session::load()
        && session.network == network
        && session.data_dir == data_dir
    {
        return Ok(session.password);
    }

    if initialized {
        Ok(prompt("Decryption password: ")?)
    } else {
        Ok(setup(dir, network)?)
    }
}

/// Walks a first run through choosing a decryption password.
fn setup(dir: &Path, network: SupportedNetwork) -> Result<Zeroizing<String>, anyhow::Error> {
    println!(
        "No wallet instance exists for {network} at {}.",
        dir.display()
    );
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

pub(crate) async fn run_unlock(
    global: &GlobalArgs,
    args: &UnlockArgs,
) -> Result<(), anyhow::Error> {
    let data_dir = session::canonical_data_dir(&global.data_dir);
    let network = args.network;
    let dir = network_dir(&data_dir, network);
    let existed = is_initialized(&dir);
    let previous = session::load();
    let was_same = previous
        .as_ref()
        .is_some_and(|s| s.network == network && s.data_dir == data_dir);
    let previous_network = previous.as_ref().map(|s| s.network);

    let sess = network_store(&data_dir, network).await?;

    if !existed {
        println!("Encrypted store created at {}.", dir.display());
    }

    match session::store(&sess) {
        Ok(()) => {
            if was_same {
                println!("Already unlocked for {network}.");
            } else if let Some(previous) = previous_network.filter(|n| *n != network) {
                println!(
                    "Unlocked {network}; {previous} is now locked. Run `edw lock` to lock the wallet."
                );
            } else {
                println!("Unlocked {network}. Run `edw lock` to lock the wallet.");
            }
        }
        Err(error) => {
            println!("Password accepted, but a session could not be saved: {error:#}");
            println!("The next command will require `edw unlock` again.");
        }
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
