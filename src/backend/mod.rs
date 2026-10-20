use std::io;
use std::path::{Path, PathBuf};

/// Access to the EC attributes exposed by the msi-ec driver.
///
/// The production implementation talks to sysfs; tests use an in-memory
/// fake so all parsing/discovery logic can be covered without hardware.
pub trait EcBackend {
    fn exists(&self, path: &Path) -> bool;
    fn read(&self, path: &Path) -> io::Result<String>;
    /// Sorted entry paths inside a directory (empty vec when missing).
    fn read_dir(&self, path: &Path) -> Vec<PathBuf>;
}

pub struct SysfsBackend;

impl EcBackend for SysfsBackend {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn read(&self, path: &Path) -> io::Result<String> {
        let raw = std::fs::read_to_string(path)?;
        Ok(raw.trim_end().to_string())
    }

    fn read_dir(&self, path: &Path) -> Vec<PathBuf> {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(path)
            .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
            .unwrap_or_default();
        entries.sort_by_key(|p| p.file_name().unwrap_or_default().to_owned());
        entries
    }
}

#[cfg(test)]
pub mod fake;
