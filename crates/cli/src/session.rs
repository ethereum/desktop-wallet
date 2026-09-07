use std::{
    fs,
    io::{IsTerminal, Write},
    os::{
        fd::AsFd,
        unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use zeroize::Zeroizing;

const TTL: Duration = Duration::from_mins(15);

fn terminal_id() -> Option<String> {
    fn identify<T: AsFd + IsTerminal>(stream: &T) -> Option<String> {
        if !stream.is_terminal() {
            return None;
        }
        let descriptor = stream.as_fd().try_clone_to_owned().ok()?;
        let meta = fs::File::from(descriptor).metadata().ok()?;
        Some(format!("{:x}-{:x}", meta.rdev(), meta.ctime()))
    }

    // stdin is the terminal in ordinary use; stdout and stderr cover a piped-in password.
    identify(&std::io::stdin())
        .or_else(|| identify(&std::io::stdout()))
        .or_else(|| identify(&std::io::stderr()))
}

fn directory() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(|dir| PathBuf::from(dir).join("edw"))
}

fn path() -> Option<PathBuf> {
    Some(directory()?.join(format!("session-{}", terminal_id()?)))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn available() -> bool {
    path().is_some()
}

pub(crate) fn load() -> Option<Zeroizing<String>> {
    let path = path()?;
    let raw = Zeroizing::new(fs::read_to_string(&path).ok()?);
    let (deadline, password) = raw.split_once('\n')?;

    if now() >= deadline.parse::<u64>().ok()? {
        let _ = fs::remove_file(&path);
        return None;
    }

    let password = Zeroizing::new(password.to_string());
    let _ = store(&password);
    Some(password)
}

pub(crate) fn store(password: &str) -> Result<(), anyhow::Error> {
    let directory = directory().context("XDG_RUNTIME_DIR is not set, so no session can be held")?;
    fs::create_dir_all(&directory)
        .with_context(|| format!("error creating {}", directory.display()))?;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;

    let path = path().context("no controlling terminal, so no session can be held")?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("error writing {}", path.display()))?;

    write!(file, "{}\n{password}", now() + TTL.as_secs())?;
    Ok(())
}

pub(crate) fn clear() -> Result<bool, anyhow::Error> {
    let Some(path) = path() else {
        return Ok(false);
    };
    match fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("error removing {}", path.display())),
    }
}
