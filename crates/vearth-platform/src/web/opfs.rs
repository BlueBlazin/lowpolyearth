//! Origin-private file system (OPFS) storage.
//!
//! Paths are `/`-separated and relative to the origin's OPFS root. Every
//! operation runs on the reactor worker, because the File System API is
//! promise-based; the `blocking` functions park the calling worker until the
//! reactor finishes and must not be called on the reactor itself.

use std::io;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

/// One file in a directory listing.
#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub size: u64,
    /// Milliseconds since the Unix epoch of the last write.
    pub modified_ms: f64,
}

fn js_error(error: JsValue) -> io::Error {
    let name = js_sys::Reflect::get(&error, &"name".into())
        .ok()
        .and_then(|name| name.as_string())
        .unwrap_or_default();
    let kind = match name.as_str() {
        "NotFoundError" => io::ErrorKind::NotFound,
        "QuotaExceededError" => io::ErrorKind::StorageFull,
        "TypeMismatchError" | "TypeError" => io::ErrorKind::InvalidInput,
        _ => io::ErrorKind::Other,
    };
    let message = error
        .dyn_ref::<js_sys::Error>()
        .map(|error| String::from(error.message()))
        .unwrap_or_else(|| format!("{error:?}"));
    io::Error::new(kind, message)
}

async fn root() -> io::Result<web_sys::FileSystemDirectoryHandle> {
    let navigator = js_sys::global()
        .unchecked_into::<web_sys::WorkerGlobalScope>()
        .navigator();
    Ok(JsFuture::from(navigator.storage().get_directory())
        .await
        .map_err(js_error)?
        .unchecked_into())
}

fn split(path: &str) -> (Vec<&str>, &str) {
    let mut parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let name = parts.pop().unwrap_or_default();
    (parts, name)
}

async fn directory(
    components: &[&str],
    create: bool,
) -> io::Result<web_sys::FileSystemDirectoryHandle> {
    let mut directory = root().await?;
    let options = web_sys::FileSystemGetDirectoryOptions::new();
    options.set_create(create);
    for component in components {
        directory =
            JsFuture::from(directory.get_directory_handle_with_options(component, &options))
                .await
                .map_err(js_error)?
                .unchecked_into();
    }
    Ok(directory)
}

/// Reads a whole file; `Ok(None)` if it does not exist.
pub async fn read(path: &str) -> io::Result<Option<Vec<u8>>> {
    let (components, name) = split(path);
    let directory = match directory(&components, false).await {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let handle: web_sys::FileSystemFileHandle =
        match JsFuture::from(directory.get_file_handle(name)).await {
            Ok(handle) => handle.unchecked_into(),
            Err(error) => {
                let error = js_error(error);
                return match error.kind() {
                    io::ErrorKind::NotFound => Ok(None),
                    _ => Err(error),
                };
            }
        };
    let file: web_sys::File = JsFuture::from(handle.get_file())
        .await
        .map_err(js_error)?
        .unchecked_into();
    let buffer = JsFuture::from(file.array_buffer())
        .await
        .map_err(js_error)?;
    Ok(Some(js_sys::Uint8Array::new(&buffer).to_vec()))
}

/// Replaces a file atomically: readers see the old or the new contents.
pub async fn write(path: &str, bytes: &[u8]) -> io::Result<()> {
    let (components, name) = split(path);
    let directory = directory(&components, true).await?;
    let options = web_sys::FileSystemGetFileOptions::new();
    options.set_create(true);
    let handle: web_sys::FileSystemFileHandle =
        JsFuture::from(directory.get_file_handle_with_options(name, &options))
            .await
            .map_err(js_error)?
            .unchecked_into();
    let stream: web_sys::FileSystemWritableFileStream = JsFuture::from(handle.create_writable())
        .await
        .map_err(js_error)?
        .unchecked_into();
    // File streams reject views of the shared WebAssembly memory.
    let unshared = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
    unshared.copy_from(bytes);
    let result = async {
        JsFuture::from(stream.write_with_js_u8_array(&unshared).map_err(js_error)?)
            .await
            .map_err(js_error)?;
        JsFuture::from(stream.close()).await.map_err(js_error)?;
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = JsFuture::from(stream.abort()).await;
    }
    result
}

/// Removes a file or a whole directory tree; missing entries are not errors.
pub async fn remove(path: &str) -> io::Result<()> {
    let (components, name) = split(path);
    let directory = match directory(&components, false).await {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let options = web_sys::FileSystemRemoveOptions::new();
    options.set_recursive(true);
    match JsFuture::from(directory.remove_entry_with_options(name, &options)).await {
        Ok(_) => Ok(()),
        Err(error) => match js_error(error) {
            error if error.kind() == io::ErrorKind::NotFound => Ok(()),
            error => Err(error),
        },
    }
}

/// Lists the files directly inside a directory (subdirectories excluded).
pub async fn list_files(path: &str) -> io::Result<Vec<Entry>> {
    let components: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let directory = match directory(&components, false).await {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut entries = Vec::new();
    let iterator = directory.values();
    loop {
        let next = JsFuture::from(iterator.next().map_err(js_error)?)
            .await
            .map_err(js_error)?;
        let done = js_sys::Reflect::get(&next, &"done".into())
            .map(|done| done.is_truthy())
            .unwrap_or(true);
        if done {
            break;
        }
        let handle = js_sys::Reflect::get(&next, &"value".into()).map_err(js_error)?;
        let Ok(handle) = handle.dyn_into::<web_sys::FileSystemFileHandle>() else {
            continue;
        };
        let file: web_sys::File = JsFuture::from(handle.get_file())
            .await
            .map_err(js_error)?
            .unchecked_into();
        entries.push(Entry {
            name: handle.name(),
            size: file.size() as u64,
            modified_ms: file.last_modified(),
        });
    }
    Ok(entries)
}

/// Lists the names of subdirectories inside a directory.
pub async fn list_directories(path: &str) -> io::Result<Vec<String>> {
    let components: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let directory = match directory(&components, false).await {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut names = Vec::new();
    let iterator = directory.values();
    loop {
        let next = JsFuture::from(iterator.next().map_err(js_error)?)
            .await
            .map_err(js_error)?;
        if js_sys::Reflect::get(&next, &"done".into())
            .map(|done| done.is_truthy())
            .unwrap_or(true)
        {
            break;
        }
        let handle = js_sys::Reflect::get(&next, &"value".into()).map_err(js_error)?;
        if let Ok(handle) = handle.dyn_into::<web_sys::FileSystemDirectoryHandle>() {
            names.push(handle.name());
        }
    }
    Ok(names)
}

/// Blocking forms for worker threads other than the reactor.
pub mod blocking {
    use super::Entry;
    use crate::rt::{block_on, spawn_local_on_reactor};
    use std::io;

    fn run<T: Send + 'static, F>(operation: impl FnOnce() -> F + Send + 'static) -> io::Result<T>
    where
        F: std::future::Future<Output = io::Result<T>> + 'static,
    {
        block_on(spawn_local_on_reactor(operation))
            .map_err(|error| io::Error::other(error.to_string()))?
    }

    pub fn read(path: &str) -> io::Result<Option<Vec<u8>>> {
        let path = path.to_owned();
        run(move || async move { super::read(&path).await })
    }

    pub fn write(path: &str, bytes: Vec<u8>) -> io::Result<()> {
        let path = path.to_owned();
        run(move || async move { super::write(&path, &bytes).await })
    }

    pub fn remove(path: &str) -> io::Result<()> {
        let path = path.to_owned();
        run(move || async move { super::remove(&path).await })
    }

    pub fn list_files(path: &str) -> io::Result<Vec<Entry>> {
        let path = path.to_owned();
        run(move || async move { super::list_files(&path).await })
    }

    pub fn list_directories(path: &str) -> io::Result<Vec<String>> {
        let path = path.to_owned();
        run(move || async move { super::list_directories(&path).await })
    }
}
