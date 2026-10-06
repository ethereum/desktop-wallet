use anyhow::Context;
use clap::Args;
use edw_core::{
    instance::{DataDir, Instance},
    network::NetworkId,
};
use zeroize::Zeroizing;

use crate::{
    GlobalArgs,
    input::Input,
    session::{Session, SessionFile},
};

/// Where a script may pass the decryption password.
///
/// Named for what it is so nobody mistakes it for an account password or an RPC credential.
/// An environment variable is readable by anything running as the user, so this exists for
/// automation and the terminal session is what interactive use should rely on.
const PASSWORD_ENV: &str = "EDW_DECRYPTION_PASSWORD";

const MISSING_PASSWORD: &str =
    "no decryption password; set EDW_DECRYPTION_PASSWORD or pipe it on stdin";

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
        let sessions = SessionFile::runtime();
        let previous = sessions.load().await;
        let was_same = previous
            .as_ref()
            .is_some_and(|s| s.network == network && s.data_dir == data_dir.path());
        let previous_network = previous.as_ref().map(|s| s.network);

        let password = password(&data_dir, network, global.input()).await?;
        Instance::open_or_create(&data_dir, network, password.as_bytes()).await?;

        if !existed {
            println!(
                "Encrypted store created at {}.",
                data_dir.instance_dir(network).display()
            );
            println!("Add your RPC endpoint with `edw network endpoint add <name> --url <url>`.");
            println!("Create a profile with `edw profile new` or `edw profile import`.");
        }

        let session = Session {
            data_dir: data_dir.path().to_path_buf(),
            network,
            password,
        };
        match sessions.store(&session).await {
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

/// Unlocks an existing instance at a terminal, for a command that found no session.
///
/// Picks the only instance under `data_dir`, or asks which one when there are several, and
/// keeps the session as `edw unlock` would. Creates nothing.
pub async fn prompt_unlock(data_dir: &DataDir, input: Input) -> Result<Instance, anyhow::Error> {
    let network = match data_dir.instances().as_slice() {
        [] => anyhow::bail!(
            "no wallet instance at {}; run `edw unlock`",
            data_dir.path().display()
        ),
        [only] => *only,
        several => {
            let labels: Vec<String> = several.iter().map(ToString::to_string).collect();
            let choice = input
                .choose("Select a network to unlock:", &labels)?
                .context(
                    "several wallet instances exist; run `edw unlock --network <name or id>`",
                )?;
            several[choice]
        }
    };

    eprintln!("Wallet is locked; unlocking {network}.");
    let password = input
        .secret("Decryption password: ")?
        .context(MISSING_PASSWORD)?;
    let instance = Instance::open(data_dir, network, password.as_bytes()).await?;
    let session = Session {
        data_dir: data_dir.path().to_path_buf(),
        network,
        password,
    };
    if let Err(error) = SessionFile::runtime().store(&session).await {
        eprintln!("Password accepted, but a session could not be saved: {error:#}");
    }
    Ok(instance)
}

/// The decryption password, from the environment, the session, or `input`.
async fn password(
    data_dir: &DataDir,
    network: NetworkId,
    input: Input,
) -> Result<Zeroizing<String>, anyhow::Error> {
    if let Some(value) = std::env::var_os(PASSWORD_ENV) {
        let value = value
            .into_string()
            .map_err(|_| anyhow::anyhow!("{PASSWORD_ENV} is not valid UTF-8"))?;
        return Ok(Zeroizing::new(value));
    }

    if let Some(session) = SessionFile::runtime().load().await
        && session.network == network
        && session.data_dir == data_dir.path()
    {
        return Ok(session.password);
    }

    if data_dir.has_instance(network) {
        input
            .secret("Decryption password: ")?
            .context(MISSING_PASSWORD)
    } else {
        setup(data_dir, network, input)
    }
}

/// Walks a first run through choosing a decryption password.
fn setup(
    data_dir: &DataDir,
    network: NetworkId,
    input: Input,
) -> Result<Zeroizing<String>, anyhow::Error> {
    if input == Input::Disabled {
        anyhow::bail!("{MISSING_PASSWORD}; nothing was created");
    }
    println!(
        "No wallet instance exists for {network} at {}.",
        data_dir.instance_dir(network).display()
    );
    println!("A decryption password must be set up before anything can be stored.");
    println!("There is no recovery path if it is lost: the data is encrypted under it alone.");

    let password = input
        .secret("New decryption password: ")?
        .context(MISSING_PASSWORD)?;
    if password.is_empty() {
        anyhow::bail!("the decryption password cannot be empty; nothing was created");
    }

    // Only a typed password can hold a typo worth catching; piped stdin would just be
    // repeating itself.
    if input == Input::Terminal {
        let confirmation = input
            .secret("Confirm decryption password: ")?
            .context(MISSING_PASSWORD)?;
        if password.as_str() != confirmation.as_str() {
            anyhow::bail!("the passwords do not match; nothing was created");
        }
    }

    Ok(password)
}
