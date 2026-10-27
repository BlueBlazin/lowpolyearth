//! Whole-file storage: `std::fs` natively, the origin-private file system in
//! the browser.
//!
//! Only whole-file operations are offered. In the browser, paths are
//! interpreted relative to the OPFS root, directories are created implicitly
//! by writes, and every call blocks the calling worker until the reactor
//! completes it, so these must not be called on the reactor worker.

#[cfg(not(target_arch = "wasm32"))]
pub use std::fs::{create_dir_all, read, remove_dir_all, remove_file, rename, write};

#[cfg(target_arch = "wasm32")]
pub use web::{create_dir_all, list_files, read, remove_dir_all, remove_file, rename, write};

/// Names of the regular files directly inside `path`.
#[cfg(not(target_arch = "wasm32"))]
pub fn list_files(path: impl AsRef<std::path::Path>) -> std::io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && let Ok(name) = entry.file_name().into_string()
        {
            names.push(name);
        }
    }
    Ok(names)
}

#[cfg(target_arch = "wasm32")]
mod web {
    use crate::web::opfs::blocking;
    use std::io;
    use std::path::Path;

    fn key(path: &Path) -> io::Result<String> {
        let path = path
            .to_str()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "non-UTF-8 path"))?;
        Ok(path.trim_start_matches('/').to_owned())
    }

    pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
        blocking::read(&key(path.as_ref())?)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "file not found"))
    }

    /// Replaces the file atomically.
    pub fn write(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> io::Result<()> {
        blocking::write(&key(path.as_ref())?, contents.as_ref().to_vec())
    }

    /// Directories exist implicitly once a file is written below them.
    pub fn create_dir_all(_path: impl AsRef<Path>) -> io::Result<()> {
        Ok(())
    }

    /// Copies then removes; OPFS has no cross-directory rename for files
    /// opened through handles. `write` is already atomic, so callers that
    /// write a temporary file and rename it keep their guarantee.
    pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
        let bytes = read(from.as_ref())?;
        write(to.as_ref(), bytes)?;
        remove_file(from)
    }

    pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
        blocking::remove(&key(path.as_ref())?)
    }

    pub fn remove_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
        blocking::remove(&key(path.as_ref())?)
    }

    pub fn list_files(path: impl AsRef<Path>) -> io::Result<Vec<String>> {
        Ok(blocking::list_files(&key(path.as_ref())?)?
            .into_iter()
            .map(|entry| entry.name)
            .collect())
    }
}
