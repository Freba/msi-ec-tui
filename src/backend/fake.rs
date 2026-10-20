use super::EcBackend;
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

/// In-memory sysfs tree used by unit tests.
#[derive(Default)]
pub struct FakeBackend {
    pub files: BTreeMap<PathBuf, String>,
}

impl FakeBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, path: impl AsRef<Path>, value: impl Into<String>) -> &mut Self {
        self.files.insert(path.as_ref().to_path_buf(), value.into());
        self
    }
}

impl EcBackend for FakeBackend {
    fn exists(&self, path: &Path) -> bool {
        self.files.contains_key(path) || self.files.keys().any(|f| f.starts_with(path) && f != path)
    }

    fn read(&self, path: &Path) -> io::Result<String> {
        self.files.get(path).cloned().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("no such file: {path:?}"))
        })
    }

    fn read_dir(&self, path: &Path) -> Vec<PathBuf> {
        // Direct children, including intermediate "directories" implied by
        // deeper keys (e.g. key a/b/c makes a child b under a).
        let prefix = format!("{}/", path.display());
        let mut set = std::collections::BTreeSet::new();
        for key in self.files.keys() {
            if let Some(rest) = key.to_string_lossy().strip_prefix(&prefix) {
                if let Some(first) = rest.split('/').next() {
                    if !first.is_empty() {
                        set.insert(path.join(first));
                    }
                }
            }
        }
        set.into_iter().collect()
    }
}
