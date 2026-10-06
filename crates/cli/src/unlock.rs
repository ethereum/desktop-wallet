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
    mnemonic,
    network::{SupportedNetwork, db::NetworkDb},
    profile::simple::{db::SimpleProfileDb, set_profile_name},
};
use zeroize::Zeroizing;

use crate::{
    GlobalArgs,
    session::{self, Session},
};

/// Where a script may pass the decryption password.
///
/// Named for what it is so nobody mistakes it for an account password or an RPC credential.
/// An environment variable is readable by anything running as the user, so this exists for
/// automation and the terminal session is what interactive use should rely on.
const PASSWORD_ENV: &str = "EDW_DECRYPTION_PASSWORD";

#[derive(Args, Debug)]
pub struct UnlockArgs {
    /// Network to unlock. Defaults to mainnet. Unlocks this network and locks every other.
    #[arg(long, default_value = "mainnet")]
    network: SupportedNetwork,
}

impl UnlockArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let data_dir = session::canonical_data_dir(&global.data_dir);
        let network = self.network;
        let dir = network_dir(&data_dir, network);
        let existed = is_initialized(&dir);
        let previous = Session::load();
        let was_same = previous
            .as_ref()
            .is_some_and(|s| s.network == network && s.data_dir == data_dir);
        let previous_network = previous.as_ref().map(|s| s.network);

        let (sess, new_phrase) = network_store(&data_dir, network).await?;

        if !existed {
            println!("Encrypted store created at {}.", dir.display());
        }

        if let Some(phrase) = new_phrase {
            println!("Write this recovery phrase down now. It is shown only this once.");
            println!();
            println!("{}", phrase.as_str());
            println!();

            if std::io::stdin().is_terminal() {
                let store =
                    open_existing_store(&sess.data_dir, sess.network, sess.password.as_bytes())
                        .await?;
                let profiles = store.clone().scoped(b"profiles").list_profiles().await?;
                let name = crate::profile::prompt_profile_name(None, &profiles, 0, Some((0, 0)))?;
                if name.is_some() {
                    set_profile_name(store, 0, 0, name).await?;
                }
            }

            println!("Mnemonic 0 and profile 0 were created.");
        }

        match sess.store() {
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
}

// TODO: maybe replace or relocate: path layout belongs to a data-dir type, not the unlock command.
pub fn network_dir(data_dir: &Path, network: SupportedNetwork) -> PathBuf {
    data_dir.join(network.slug())
}

// TODO: maybe replace or relocate: FileDatabase should answer whether its directory holds a store.
/// Whether an encrypted store already exists, checked before anything can create one.
fn is_initialized(dir: &Path) -> bool {
    fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some())
}

// TODO: maybe replace or relocate: an error constructor; use a thiserror variant.
pub fn locked_error() -> anyhow::Error {
    anyhow::anyhow!("wallet is locked; run `edw unlock`")
}

// TODO: maybe replace or relocate: repeats the unlock branch of network_store.
pub async fn open_existing_store(
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

// TODO: maybe replace or relocate: mixes store open/create, preference seeding, and Session construction.
async fn network_store(
    data_dir: &Path,
    network: SupportedNetwork,
) -> Result<(Session, Option<Zeroizing<String>>), anyhow::Error> {
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

    let prefs = store.clone().scoped(b"preferences");
    if prefs.get_network_configs().await?.is_empty() {
        let config = network.default_config();
        prefs
            .put_network_configs(std::slice::from_ref(&config))
            .await
            .context("error seeding networkConfigs")?;
        prefs
            .put_active(&config.name)
            .await
            .context("error seeding the active networkConfig")?;
    }

    let new_phrase = if initialized {
        None
    } else {
        let record = mnemonic::seed_new_instance(store, false)
            .await
            .context("error seeding the default mnemonic")?;
        Some(Zeroizing::new(record.phrase.clone()))
    };

    Ok((
        Session {
            data_dir,
            network,
            password,
        },
        new_phrase,
    ))
}

// TODO: maybe replace or relocate: password-source resolution; four loose args suggest a missing type.
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

    if let Some(session) = Session::load()
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

// TODO: maybe replace or relocate: one of several stdin prompt helpers (see profile/mod.rs).
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
