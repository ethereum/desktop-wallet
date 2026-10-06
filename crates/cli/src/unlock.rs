use std::io::{BufRead, IsTerminal};

use anyhow::Context;
use clap::Args;
use edw_core::{
    instance::{DataDir, Instance},
    network::NetworkId,
};
use zeroize::Zeroizing;

use crate::{GlobalArgs, session::Session};

/// Where a script may pass the decryption password.
///
/// Named for what it is so nobody mistakes it for an account password or an RPC credential.
/// An environment variable is readable by anything running as the user, so this exists for
/// automation and the terminal session is what interactive use should rely on.
const PASSWORD_ENV: &str = "EDW_DECRYPTION_PASSWORD";

#[derive(Args, Debug)]
pub struct UnlockArgs {
    /// Network to unlock: `mainnet`, `sepolia`, `local`, or any numeric network id. Unlocks
    /// this network and locks every other.
    #[arg(long, default_value = "mainnet")]
    network: NetworkId,
}

impl UnlockArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let data_dir = global.data_dir();
        let network = self.network;
        let existed = data_dir.has_instance(network);
        let previous = Session::load();
        let was_same = previous
            .as_ref()
            .is_some_and(|s| s.network == network && s.data_dir == data_dir.path());
        let previous_network = previous.as_ref().map(|s| s.network);

        let password = password(&data_dir, network)?;
        Instance::open_or_create(&data_dir, network, password.as_bytes()).await?;

        if !existed {
            println!(
                "Encrypted store created at {}.",
                data_dir.instance_dir(network).display()
            );
            println!("Create a profile with `edw profile generate` or `edw profile import`.");
        }

        let session = Session {
            data_dir: data_dir.path().to_path_buf(),
            network,
            password,
        };
        match session.store() {
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

/// The decryption password, from the environment, the session, or the terminal.
fn password(data_dir: &DataDir, network: NetworkId) -> Result<Zeroizing<String>, anyhow::Error> {
    if let Some(value) = std::env::var_os(PASSWORD_ENV) {
        let value = value
            .into_string()
            .map_err(|_| anyhow::anyhow!("{PASSWORD_ENV} is not valid UTF-8"))?;
        return Ok(Zeroizing::new(value));
    }

    if let Some(session) = Session::load()
        && session.network == network
        && session.data_dir == data_dir.path()
    {
        return Ok(session.password);
    }

    if data_dir.has_instance(network) {
        Ok(prompt("Decryption password: ")?)
    } else {
        Ok(setup(data_dir, network)?)
    }
}

/// Walks a first run through choosing a decryption password.
fn setup(data_dir: &DataDir, network: NetworkId) -> Result<Zeroizing<String>, anyhow::Error> {
    println!(
        "No wallet instance exists for {network} at {}.",
        data_dir.instance_dir(network).display()
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
