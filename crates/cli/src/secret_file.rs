use std::{
    fs::{self, OpenOptions},
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

use anyhow::Context;
use zeroize::Zeroizing;

/// Reads `path`, refusing anything but a regular file owned by the current user with mode 0400
/// or 0600.
///
/// The contents are returned as is, trailing newline included.
pub fn read(path: &Path, label: &str, max_bytes: u64) -> Result<Zeroizing<String>, anyhow::Error> {
    let linked = fs::symlink_metadata(path)
        .with_context(|| format!("error reading the {label} file {}", path.display()))?;
    if linked.file_type().is_symlink() {
        anyhow::bail!("the {label} file cannot be a symbolic link");
    }
    if !linked.is_file() {
        anyhow::bail!("the {label} file must be a regular file");
    }

    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .with_context(|| format!("error opening the {label} file {}", path.display()))?;
    let opened = file.metadata()?;
    if (opened.dev(), opened.ino()) != (linked.dev(), linked.ino()) {
        anyhow::bail!("the {label} file changed while it was being opened");
    }
    // SAFETY: geteuid has no preconditions and cannot fail.
    if opened.uid() != unsafe { libc::geteuid() } {
        anyhow::bail!("the {label} file must be owned by the current user");
    }
    let permissions = opened.mode() & 0o777;
    if permissions != 0o400 && permissions != 0o600 {
        anyhow::bail!("the {label} file permissions must be 0400 or 0600; run chmod 600 on it");
    }

    let mut contents = Zeroizing::new(String::new());
    file.by_ref()
        .take(max_bytes + 1)
        .read_to_string(&mut contents)
        .with_context(|| format!("error reading the {label} file {}", path.display()))?;
    if contents.len() as u64 > max_bytes {
        anyhow::bail!("the {label} file is too large (maximum {max_bytes} bytes)");
    }
    Ok(contents)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::{
        os::unix::fs::{PermissionsExt, symlink},
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "edw-secret-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self, contents: &str, mode: u32) -> PathBuf {
            let path = self.0.join("secret");
            fs::write(&path, contents).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn error(path: &Path) -> String {
        format!("{:#}", read(path, "mnemonic", 64).unwrap_err())
    }

    #[test]
    fn reads_an_owner_only_file() {
        let scratch = Scratch::new();
        let path = scratch.file("one two\nthree\n", 0o600);

        assert_eq!(
            read(&path, "mnemonic", 64).unwrap().as_str(),
            "one two\nthree\n"
        );
    }

    #[test]
    fn refuses_a_file_others_can_read() {
        let scratch = Scratch::new();
        let path = scratch.file("words", 0o644);

        assert!(error(&path).contains("must be 0400 or 0600"));
    }

    #[test]
    fn refuses_a_symbolic_link() {
        let scratch = Scratch::new();
        let target = scratch.file("words", 0o600);
        let link = scratch.0.join("link");
        symlink(&target, &link).unwrap();

        assert!(error(&link).contains("symbolic link"));
    }

    #[test]
    fn refuses_a_file_over_the_limit() {
        let scratch = Scratch::new();
        let path = scratch.file(&"a".repeat(65), 0o600);

        assert!(error(&path).contains("too large"));
    }
}
