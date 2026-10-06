use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::network::NetworkId;

/// The directory that holds one wallet instance per network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDir(PathBuf);

impl DataDir {
    /// Resolves `path` to an absolute path with symlinks resolved, also when its tail does not
    /// exist yet, so two spellings of one directory compare equal.
    #[must_use]
    pub fn new(path: &Path) -> Self {
        let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        let mut suffix = PathBuf::new();
        let mut cursor = absolute.as_path();
        loop {
            if let Ok(canonical) = fs::canonicalize(cursor) {
                return Self(if suffix.as_os_str().is_empty() {
                    canonical
                } else {
                    canonical.join(suffix)
                });
            }
            match (cursor.file_name(), cursor.parent()) {
                (Some(name), Some(parent)) if parent != cursor => {
                    suffix = Path::new(name).join(suffix);
                    cursor = parent;
                }
                _ => return Self(absolute),
            }
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }

    #[must_use]
    pub fn instance_dir(&self, network_id: NetworkId) -> PathBuf {
        self.0.join(network_id.to_string())
    }

    /// Whether an instance exists for `network_id`, checked before anything can create one.
    #[must_use]
    pub fn has_instance(&self, network_id: NetworkId) -> bool {
        fs::read_dir(self.instance_dir(network_id))
            .is_ok_and(|mut entries| entries.next().is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_leaf_resolves_under_its_canonical_parent() {
        let parent = std::env::temp_dir();
        let missing = parent.join("edw-missing-leaf");
        assert_eq!(
            DataDir::new(&missing).path(),
            fs::canonicalize(&parent).unwrap().join("edw-missing-leaf")
        );
    }

    #[test]
    fn a_preset_instance_lives_under_its_name_and_any_other_under_its_number() {
        let data_dir = DataDir::new(Path::new("/edw"));
        assert_eq!(
            data_dir.instance_dir(NetworkId(1)),
            Path::new("/edw/mainnet")
        );
        assert_eq!(
            data_dir.instance_dir(NetworkId(1337)),
            Path::new("/edw/1337")
        );
    }
}
