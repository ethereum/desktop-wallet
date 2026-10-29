use std::fmt;

use clap::Args;
use edw_core::{
    instance::{DataDir, Instance},
    network::NetworkId,
};
use serde::Serialize;
use zeroize::Zeroizing;

use crate::{
    GlobalArgs, failure,
    input::Input,
    report::Report,
    session::{self, Session, SessionFile},
};

/// Where a script may pass the decryption password. Any process running as the user can read
/// it, so interactive use should rely on the session instead.
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

#[derive(Serialize)]
struct UnlockReport {
    network: String,
    created: Option<String>,
    already_unlocked: bool,
    locked: Option<String>,
    session_held: bool,
    #[serde(skip)]
    session_error: Option<String>,
}

#[derive(Serialize)]
struct LockReport {
    was_unlocked: bool,
}

impl Report for UnlockReport {
    const KIND: &'static str = "edw/unlock";
    const VERSION: u32 = 2;
}

impl fmt::Display for UnlockReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(dir) = &self.created {
            writeln!(f, "Encrypted store created at {dir}.")?;
            writeln!(
                f,
                "Mainnet and Sepolia start on an active RPC named publicnode (500ms between calls), the public Pimlico bundler, and the Saga indexer. Add another RPC with `edw network rpc-config add <name> --url <url>`."
            )?;
            writeln!(
                f,
                "Create a profile with `edw profile new` or `edw profile import`."
            )?;
        }

        if let Some(error) = &self.session_error {
            writeln!(
                f,
                "Password accepted, but a session could not be saved: {error}"
            )?;
            return write!(
                f,
                "Each command will ask for the password again at a terminal, or read {PASSWORD_ENV}."
            );
        }

        if self.already_unlocked {
            write!(f, "Already unlocked for {}.", self.network)
        } else if let Some(locked) = &self.locked {
            write!(
                f,
                "Unlocked {}; {locked} is now locked. Run `edw lock` to lock the wallet.",
                self.network
            )
        } else {
            write!(
                f,
                "Unlocked {}. Run `edw lock` to lock the wallet.",
                self.network
            )
        }
    }
}

impl Report for LockReport {
    const KIND: &'static str = "edw/lock";
    const VERSION: u32 = 1;
}

impl fmt::Display for LockReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.was_unlocked {
            write!(f, "Locked.")
        } else {
            write!(f, "Not unlocked; nothing to do.")
        }
    }
}

impl UnlockArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let data_dir = global.data_dir();
        let network = self.network;
        let existed = data_dir.has_instance(network);
        let sessions = SessionFile::runtime();
        let previous = sessions.load_if_present().await;
        let previous_network = previous.as_ref().map(|s| s.network);

        let password = password(&data_dir, network, global.input()).await?;
        let was_same = previous.is_some_and(|s| {
            s.network == network && s.data_dir == data_dir.path() && s.password == password
        });
        Instance::open_or_create(&data_dir, network, password.as_bytes()).await?;

        let session = Session {
            data_dir: data_dir.path().to_path_buf(),
            network,
            password,
        };
        let session_error = match sessions.store(&session).await {
            Ok(()) => None,
            Err(error) => {
                if let Some(previous) = previous_network
                    && let Err(clear_error) = sessions.clear().await
                {
                    return Err(clear_error.context(format!(
                        "password accepted, but no session could be held, and the session for {previous} could not be removed, so it is still unlocked"
                    )));
                }
                if global.non_interactive && session::can_hold() {
                    return Err(error.context("password accepted, but no session could be held"));
                }
                Some(format!("{error:#}"))
            }
        };
        let held = session_error.is_none();
        UnlockReport {
            network: network.to_string(),
            created: (!existed).then(|| data_dir.instance_dir(network).display().to_string()),
            already_unlocked: held && was_same,
            locked: previous_network
                .filter(|previous| held && *previous != network)
                .map(|previous| previous.to_string()),
            session_held: held,
            session_error,
        }
        .emit(global.mode())
    }
}

pub async fn run_lock(global: &GlobalArgs) -> Result<(), anyhow::Error> {
    LockReport {
        was_unlocked: SessionFile::runtime().clear().await?,
    }
    .emit(global.mode())
}

/// Unlocks an existing instance at a terminal, for a command that found no session.
///
/// Picks the only instance under `data_dir`, or asks which one when there are several, and
/// keeps the session as `edw unlock` would. Creates nothing.
pub async fn prompt_unlock(data_dir: &DataDir, input: Input) -> Result<Instance, anyhow::Error> {
    let network = choose_network(data_dir, input)?;
    eprintln!("Wallet is locked; unlocking {network}.");
    let password = input
        .secret("Decryption password: ")?
        .filter(|password| !password.is_empty())
        .ok_or_else(|| failure::locked("no decryption password was entered"))?;
    let instance = Instance::open(data_dir, network, password.as_bytes()).await?;
    let session = Session {
        data_dir: data_dir.path().to_path_buf(),
        network,
        password,
    };
    if session::can_hold()
        && let Err(error) = SessionFile::runtime().store(&session).await
    {
        eprintln!("Password accepted, but a session could not be saved: {error:#}");
    }
    Ok(instance)
}

/// The only instance under `data_dir`, or the one picked when there are several.
pub fn choose_network(data_dir: &DataDir, input: Input) -> Result<NetworkId, anyhow::Error> {
    match data_dir.instances().as_slice() {
        [] => anyhow::bail!(
            "no wallet instance at {}; run `edw unlock`",
            data_dir.path().display()
        ),
        [only] => Ok(*only),
        several => {
            let labels: Vec<String> = several.iter().map(ToString::to_string).collect();
            let choice = input
                .choose("Select a network to unlock:", &labels)?
                .ok_or_else(|| {
                    failure::usage(if session::can_hold() {
                        "several wallet instances exist; run `edw unlock --network <name or id>`"
                    } else {
                        "several wallet instances exist, and with no session here none can be chosen without a terminal; use a data directory that holds one"
                    })
                })?;
            Ok(several[choice])
        }
    }
}

/// The decryption password from [`PASSWORD_ENV`], when it is set.
pub fn env_password() -> Result<Option<Zeroizing<String>>, anyhow::Error> {
    std::env::var_os(PASSWORD_ENV)
        .map(|value| {
            value
                .into_string()
                .map(Zeroizing::new)
                .map_err(|_| failure::usage(format!("{PASSWORD_ENV} is not valid UTF-8")))
        })
        .transpose()
}

/// The decryption password, from the environment, the session, or `input`.
async fn password(
    data_dir: &DataDir,
    network: NetworkId,
    input: Input,
) -> Result<Zeroizing<String>, anyhow::Error> {
    if let Some(password) = env_password()? {
        return Ok(password);
    }

    // Only a stale session is skipped; other open errors are left to `open_or_create`, which
    // repairs a missing network record.
    let sessions = SessionFile::runtime();
    if let Some(session) = sessions.load_if_present().await
        && session.network == network
        && session.data_dir == data_dir.path()
        && !matches!(sessions.open(&session).await, Ok(None))
    {
        return Ok(session.password);
    }

    if data_dir.has_instance(network) {
        input
            .secret("Decryption password: ")?
            .filter(|password| !password.is_empty())
            .ok_or_else(|| failure::locked(MISSING_PASSWORD))
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
        return Err(failure::locked(format!(
            "{MISSING_PASSWORD}; nothing was created"
        )));
    }
    eprintln!(
        "No wallet instance exists for {network} at {}.",
        data_dir.instance_dir(network).display()
    );
    eprintln!("A decryption password must be set up before anything can be stored.");
    eprintln!("There is no recovery path if it is lost: the data is encrypted under it alone.");

    let password = input
        .secret("New decryption password: ")?
        .ok_or_else(|| failure::locked(MISSING_PASSWORD))?;
    if password.is_empty() && input == Input::Piped {
        return Err(failure::locked(format!(
            "{MISSING_PASSWORD}; nothing was created"
        )));
    }
    if password.is_empty() {
        return Err(failure::usage(
            "the decryption password cannot be empty; nothing was created",
        ));
    }

    // Only a typed password can hold a typo worth catching; piped stdin would just be
    // repeating itself.
    if input == Input::Terminal {
        let confirmation = input
            .secret("Confirm decryption password: ")?
            .ok_or_else(|| failure::locked(MISSING_PASSWORD))?;
        if password.as_str() != confirmation.as_str() {
            return Err(failure::usage(
                "the passwords do not match; nothing was created",
            ));
        }
    }

    Ok(password)
}
