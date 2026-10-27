//! Bundled binary assets.
//!
//! [`asset_bytes!`](crate::asset_bytes) and
//! [`asset_blocks!`](crate::asset_blocks) take the same caller-relative path
//! as `include_bytes!`, and native builds embed the file. In the browser,
//! `tools/build-web.sh` publishes every bundled asset under `assets/` at its
//! repository-relative path: whole for `asset_bytes!`, fetched once on first
//! use; in fixed-size blocks for `asset_blocks!`, fetched as reads need them.
//! Fetched bytes are kept for the life of the page. Build-generated browser
//! assets are published under `generated/` and loaded with `try_load`.

use std::borrow::Cow;
use std::ops::Range;

/// `&'static [u8]` for a bundled file, by path relative to the calling
/// source file.
#[macro_export]
macro_rules! asset_bytes {
    ($path:expr) => {{
        #[cfg(not(target_arch = "wasm32"))]
        let bytes: &'static [u8] = include_bytes!($path);
        #[cfg(target_arch = "wasm32")]
        let bytes: &'static [u8] = $crate::assets::load(file!(), $path);
        bytes
    }};
}

/// [`Blocks`](crate::assets::Blocks) for a bundled file that is read in
/// small pieces, by path relative to the calling source file.
#[macro_export]
macro_rules! asset_blocks {
    ($path:expr) => {{
        #[cfg(not(target_arch = "wasm32"))]
        let blocks = $crate::assets::Blocks::embedded(include_bytes!($path));
        #[cfg(target_arch = "wasm32")]
        let blocks = $crate::assets::Blocks::published(file!(), $path);
        blocks
    }};
}

/// Random access to a bundled file without loading all of it in the
/// browser. Keep one per file, as the browser's fetched blocks live in it.
pub struct Blocks {
    #[cfg(not(target_arch = "wasm32"))]
    bytes: &'static [u8],
    #[cfg(target_arch = "wasm32")]
    published: web::PublishedBlocks,
}

impl Blocks {
    #[doc(hidden)]
    #[cfg(not(target_arch = "wasm32"))]
    pub const fn embedded(bytes: &'static [u8]) -> Self {
        Self { bytes }
    }

    #[doc(hidden)]
    #[cfg(target_arch = "wasm32")]
    pub fn published(caller_file: &str, relative: &str) -> Self {
        Self {
            published: web::PublishedBlocks::new(&resolve(caller_file, relative)),
        }
    }

    pub fn len(&self) -> usize {
        #[cfg(not(target_arch = "wasm32"))]
        return self.bytes.len();
        #[cfg(target_arch = "wasm32")]
        return self.published.len();
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The bytes in `range`, borrowed unless they span published blocks.
    /// Blocks the calling worker in the browser while blocks download; a
    /// failed download is retried by the next read.
    pub fn read(&self, range: Range<usize>) -> Result<Cow<'static, [u8]>, String> {
        if range.start > range.end || range.end > self.len() {
            return Err(format!(
                "read {range:?} is outside the {}-byte asset",
                self.len()
            ));
        }
        #[cfg(not(target_arch = "wasm32"))]
        return Ok(Cow::Borrowed(&self.bytes[range]));
        #[cfg(target_arch = "wasm32")]
        return self.published.read(range);
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::{configure, download_uncached, is_published, load, try_load};

/// Normalizes `relative` against the directory of `caller_file`, giving the
/// repository-relative path the site publishes.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn resolve(caller_file: &str, relative: &str) -> String {
    let mut parts: Vec<&str> = caller_file.split('/').collect();
    parts.pop();
    for part in relative.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

#[cfg(target_arch = "wasm32")]
mod web {
    use std::borrow::Cow;
    use std::collections::HashMap;
    use std::io::Read;
    use std::ops::Range;
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::Duration;
    use wasm_bindgen::JsCast;

    /// How the site publishes one asset.
    #[derive(Clone, Debug)]
    struct Published {
        /// Uncompressed length, checked after download.
        size: usize,
        /// Content hash, used as the cache-busting query.
        version: String,
        /// Published gzip-compressed with a `.gz` suffix (per block for
        /// block assets).
        gzip: bool,
        /// Number of consecutive `.NNN` parts the published file is split
        /// into (static hosts limit file sizes), or 0 if it is whole.
        parts: usize,
        /// For block assets, the uncompressed block length; each block is
        /// published as `.bNNNNN`. 0 for whole assets.
        block: usize,
    }

    struct Catalog {
        base_url: String,
        entries: HashMap<String, Published>,
    }

    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    /// One slot per path; holding a slot's lock while downloading keeps
    /// concurrent users of an asset to one download.
    type Slot = Arc<Mutex<Option<&'static [u8]>>>;
    static LOADED: OnceLock<Mutex<HashMap<String, Slot>>> = OnceLock::new();

    const ATTEMPTS: usize = 4;
    const TIMEOUT: Duration = Duration::from_secs(120);

    /// Sets the asset base URL and the published catalog
    /// (`asset-manifest.json`). Called once by the root worker at startup.
    pub fn configure(base_url: String, manifest_json: &str) -> Result<(), String> {
        let entries = manifest::parse(manifest_json)?.into_iter().collect();
        CATALOG
            .set(Catalog { base_url, entries })
            .map_err(|_| "assets already configured".to_owned())
    }

    fn catalog() -> Result<&'static Catalog, String> {
        CATALOG
            .get()
            .ok_or_else(|| "assets are not configured".into())
    }

    /// The bytes of a published asset, downloading it on first use. Blocks
    /// the calling worker; a missing or corrupt asset is unrecoverable, like
    /// a missing embedded file, so this panics after retries.
    pub fn load(caller_file: &str, relative: &str) -> &'static [u8] {
        let path = super::resolve(caller_file, relative);
        match try_load(&path) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => panic!("asset {path}: not in the published asset manifest"),
            Err(error) => panic!("asset {path}: {error}"),
        }
    }

    /// The bytes of the published asset at `path`, downloading it on first
    /// use, or `None` when the site publishes no such asset. Blocks the
    /// calling worker. A failed download is retried on the next call.
    pub fn try_load(path: &str) -> Result<Option<&'static [u8]>, String> {
        let catalog = catalog()?;
        let Some(published) = catalog.entries.get(path) else {
            return Ok(None);
        };
        if published.block != 0 {
            return Err(format!("{path} is published in blocks"));
        }
        let slot = LOADED
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .entry(path.to_owned())
            .or_default()
            .clone();
        let mut slot = slot.lock().unwrap();
        if slot.is_none() {
            let bytes = download(
                path,
                published,
                &urls(catalog, path, published),
                published.size,
            )?;
            *slot = Some(Box::leak(bytes.into_boxed_slice()));
        }
        Ok(*slot)
    }

    /// Whether the site publishes a whole asset at `path`.
    pub fn is_published(path: &str) -> bool {
        catalog().is_ok_and(|catalog| {
            catalog
                .entries
                .get(path)
                .is_some_and(|published| published.block == 0)
        })
    }

    /// Downloads the published asset at `path` without keeping it, for
    /// callers that decode it once into their own structures; `None` when
    /// the site publishes no such asset. Blocks the calling worker.
    pub fn download_uncached(path: &str) -> Result<Option<Vec<u8>>, String> {
        let catalog = catalog()?;
        let Some(published) = catalog.entries.get(path) else {
            return Ok(None);
        };
        if published.block != 0 {
            return Err(format!("{path} is published in blocks"));
        }
        download(
            path,
            published,
            &urls(catalog, path, published),
            published.size,
        )
        .map(Some)
    }

    /// The published file, or its consecutive parts.
    fn urls(catalog: &Catalog, path: &str, published: &Published) -> Vec<String> {
        let url = format!("{}{}", catalog.base_url, path);
        if published.parts == 0 {
            vec![url]
        } else {
            (0..published.parts)
                .map(|part| format!("{url}.{part:03}"))
                .collect()
        }
    }

    pub struct PublishedBlocks {
        path: String,
        published: &'static Published,
        slots: Box<[OnceLock<&'static [u8]>]>,
    }

    impl PublishedBlocks {
        /// Panics when the site does not publish `path` in blocks, like a
        /// missing embedded file.
        pub fn new(path: &str) -> Self {
            let published = catalog()
                .and_then(|catalog| {
                    catalog
                        .entries
                        .get(path)
                        .filter(|published| published.block != 0)
                        .ok_or_else(|| "not published in blocks".into())
                })
                .unwrap_or_else(|error| panic!("asset {path}: {error}"));
            Self {
                path: path.to_owned(),
                published,
                slots: (0..published.size.div_ceil(published.block))
                    .map(|_| OnceLock::new())
                    .collect(),
            }
        }

        pub fn len(&self) -> usize {
            self.published.size
        }

        pub fn read(&self, range: Range<usize>) -> Result<Cow<'static, [u8]>, String> {
            let size = self.published.block;
            if range.is_empty() {
                return Ok(Cow::Borrowed(&[]));
            }
            let first = range.start / size;
            let last = (range.end - 1) / size;
            if first == last {
                let block = self.block(first)?;
                return Ok(Cow::Borrowed(
                    &block[range.start - first * size..range.end - first * size],
                ));
            }
            let mut bytes = Vec::with_capacity(range.len());
            for index in first..=last {
                let block = self.block(index)?;
                let start = range.start.max(index * size) - index * size;
                let end = range.end.min((index + 1) * size) - index * size;
                bytes.extend_from_slice(&block[start..end]);
            }
            Ok(Cow::Owned(bytes))
        }

        /// Concurrent first reads of one block may both download it; only
        /// the stored copy is kept.
        fn block(&self, index: usize) -> Result<&'static [u8], String> {
            if let Some(block) = self.slots[index].get() {
                return Ok(block);
            }
            let size = self.published.block;
            let expected = size.min(self.published.size - index * size);
            let url = format!("{}{}.b{index:05}", catalog()?.base_url, self.path);
            let bytes = download(&self.path, self.published, &[url], expected)?;
            Ok(self.slots[index].get_or_init(|| Box::leak(bytes.into_boxed_slice())))
        }
    }

    /// Fetches and concatenates `urls` (with the publisher's `.gz` suffix and
    /// version query), then decompresses and checks the length.
    fn download(
        path: &str,
        published: &Published,
        urls: &[String],
        expected: usize,
    ) -> Result<Vec<u8>, String> {
        let suffix = if published.gzip { ".gz" } else { "" };
        let started = crate::time::Instant::now();
        let mut last_error = String::new();
        for attempt in 1..=ATTEMPTS {
            let body = urls.iter().try_fold(Vec::new(), |mut body, url| {
                body.extend(fetch(&format!("{url}{suffix}?v={}", published.version))?);
                Ok::<_, String>(body)
            });
            match body {
                Ok(body) => {
                    let bytes = if published.gzip {
                        let mut bytes = Vec::with_capacity(expected);
                        flate2::read::GzDecoder::new(&body[..])
                            .read_to_end(&mut bytes)
                            .map_err(|error| format!("gzip: {error}"))?;
                        bytes
                    } else {
                        body
                    };
                    if bytes.len() != expected {
                        last_error = format!("got {} bytes, expected {expected}", bytes.len());
                        continue;
                    }
                    if published.block == 0 {
                        log::info!(
                            "asset {path}: {:.1} MiB in {:.0} ms",
                            bytes.len() as f64 / 1048576.0,
                            started.elapsed().as_secs_f64() * 1e3
                        );
                    }
                    return Ok(bytes);
                }
                Err(error) => {
                    last_error = error;
                    log::warn!("asset {path}: attempt {attempt} failed: {last_error}");
                    std::thread::sleep(Duration::from_millis(500 * attempt as u64));
                }
            }
        }
        Err(last_error)
    }

    /// Synchronous GET; permitted in workers, where all Rust code runs.
    fn fetch(url: &str) -> Result<Vec<u8>, String> {
        let error = |error: wasm_bindgen::JsValue| format!("{error:?}");
        let xhr = web_sys::XmlHttpRequest::new().map_err(error)?;
        xhr.open_with_async("GET", url, false).map_err(error)?;
        xhr.set_timeout(TIMEOUT.as_millis() as u32);
        xhr.set_response_type(web_sys::XmlHttpRequestResponseType::Arraybuffer);
        xhr.send().map_err(error)?;
        match xhr.status().map_err(error)? {
            200 => {}
            0 => return Err("network error or timeout".into()),
            status => return Err(format!("HTTP {status}")),
        }
        let buffer = xhr.response().map_err(error)?;
        Ok(js_sys::Uint8Array::new(buffer.unchecked_ref::<js_sys::ArrayBuffer>()).to_vec())
    }

    /// The manifest is a flat JSON object of `{"path": {"size": n,
    /// "version": "hash", "gzip": bool, "parts": n, "block": n}}`, parsed by
    /// the browser's JSON implementation.
    mod manifest {
        use super::Published;

        pub fn parse(text: &str) -> Result<Vec<(String, Published)>, String> {
            let value: js_sys::Object = js_sys::JSON::parse(text)
                .map_err(|error| format!("manifest: {error:?}"))?
                .into();
            let mut entries = Vec::new();
            for key in js_sys::Object::keys(&value).iter() {
                let path = key.as_string().ok_or("manifest key")?;
                let entry = js_sys::Reflect::get(&value, &key).map_err(|_| "manifest entry")?;
                let field = |name: &str| js_sys::Reflect::get(&entry, &name.into()).ok();
                let count =
                    |name: &str| field(name).and_then(|v| v.as_f64()).unwrap_or(0.0) as usize;
                let published = Published {
                    size: field("size")
                        .and_then(|v| v.as_f64())
                        .ok_or("manifest size")? as usize,
                    version: field("version")
                        .and_then(|v| v.as_string())
                        .ok_or("manifest version")?,
                    gzip: field("gzip").is_some_and(|v| v.is_truthy()),
                    parts: count("parts"),
                    block: count("block"),
                };
                entries.push((path, published));
            }
            Ok(entries)
        }
    }
}
