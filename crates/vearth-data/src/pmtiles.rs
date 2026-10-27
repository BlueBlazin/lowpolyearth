//! PMTiles v3 reader over HTTP range requests.
//!
//! PMTiles is a single-file tile archive designed for range reads: a fixed
//! 127-byte header, a root directory, optional leaf directories, and the
//! tile data — so a 100+ GB planet archive on a public bucket serves
//! individual tiles in 1–3 small GETs. Directories are cached in memory;
//! callers cache tiles.
//!
//! Spec: <https://github.com/protomaps/PMTiles/blob/main/spec/v3>

use crate::DataError;
#[cfg(feature = "async-http")]
use crate::async_http::{AsyncRangeFetcher, RequestPriority};
use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, Mutex, OnceLock};
#[cfg(feature = "async-http")]
use std::task::{Poll, Waker};
use vearth_platform::time::Instant;

/// Byte-range transport. Implementations must return exactly the requested
/// range (or error) — a server that ignores Range headers would otherwise
/// stream the whole archive.
pub trait RangeFetcher: Send + Sync {
    fn fetch_range(&self, offset: u64, len: u64) -> Result<Vec<u8>, DataError>;
}

/// Local-file ranges: offline use and tests.
pub struct FileRange(pub std::fs::File);

impl RangeFetcher for FileRange {
    fn fetch_range(&self, offset: u64, len: u64) -> Result<Vec<u8>, DataError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileExt;
            let mut buf = vec![0u8; len as usize];
            self.0
                .read_exact_at(&mut buf, offset)
                .map_err(|e| DataError::Http(format!("file range: {e}")))?;
            Ok(buf)
        }
        #[cfg(not(unix))]
        {
            let _ = (offset, len);
            Err(DataError::NotAvailable)
        }
    }
}

/// HTTP Range transport: ureq natively, synchronous XHR in browser workers.
/// Hosted builds are read from their block objects instead.
pub struct HttpRange {
    #[cfg(not(target_arch = "wasm32"))]
    agent: ureq::Agent,
    #[cfg(target_arch = "wasm32")]
    timeout: vearth_platform::time::Duration,
    pub url: String,
}

impl HttpRange {
    pub fn new(url: impl Into<String>) -> Self {
        Self::with_timeout(url, vearth_platform::time::Duration::from_secs(30))
    }

    pub fn with_timeout(url: impl Into<String>, timeout: vearth_platform::time::Duration) -> Self {
        #[cfg(target_arch = "wasm32")]
        {
            HttpRange {
                timeout,
                url: url.into(),
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let config = ureq::Agent::config_builder()
                .timeout_global(Some(timeout))
                .user_agent(concat!("vearth/", env!("CARGO_PKG_VERSION")))
                // Range reads arrive in bursts from many workers; keep the
                // connections alive instead of re-handshaking TLS per request.
                .max_idle_connections(64)
                .max_idle_connections_per_host(32)
                .build();
            HttpRange {
                agent: config.into(),
                url: url.into(),
            }
        }
    }

    /// Reads `offset..=end`, accepting only a partial-content response and
    /// at most one byte more than `len`, so an ignored Range cannot stream a
    /// whole archive into memory.
    #[cfg(not(target_arch = "wasm32"))]
    fn get_range(&self, offset: u64, end: u64, len: u64) -> Result<Vec<u8>, DataError> {
        if crate::vectors::is_block_archive(&self.url) {
            return self.get_blocks(offset, end);
        }
        let mut resp = self
            .agent
            .get(&self.url)
            .header("Range", &format!("bytes={offset}-{end}"))
            .call()
            .map_err(|e| DataError::Http(e.to_string()))?;
        if resp.status() != 206 {
            return Err(DataError::Http(format!(
                "range request got {} (server ignores Range?)",
                resp.status()
            )));
        }
        let mut buf = Vec::with_capacity(len as usize);
        resp.body_mut()
            .as_reader()
            .take(len.saturating_add(1))
            .read_to_end(&mut buf)
            .map_err(|e| DataError::Http(e.to_string()))?;
        Ok(buf)
    }

    /// Assembles bytes `offset..=end` of a hosted build from its block
    /// objects. Blocking reads keep no block cache: the app reads through the
    /// async transport, which does.
    #[cfg(not(target_arch = "wasm32"))]
    fn get_blocks(&self, offset: u64, end: u64) -> Result<Vec<u8>, DataError> {
        let size = crate::vectors::HOSTED_BUILD_BLOCK_BYTES;
        let mut body = Vec::with_capacity((end - offset + 1) as usize);
        for (index, span) in crate::vectors::block_spans(offset, end) {
            let url = crate::vectors::block_url(&self.url, index);
            let mut resp = self
                .agent
                .get(&url)
                .call()
                .map_err(|e| DataError::Http(format!("{url}: {e}")))?;
            if resp.status() != 200 {
                return Err(DataError::Http(format!("{url}: got {}", resp.status())));
            }
            let mut block = Vec::with_capacity(size as usize);
            resp.body_mut()
                .as_reader()
                .take(size + 1)
                .read_to_end(&mut block)
                .map_err(|e| DataError::Http(format!("{url}: {e}")))?;
            let bytes = block.get(span).ok_or_else(|| {
                DataError::Http(format!(
                    "range {offset}-{end} extends past the end of {}",
                    self.url
                ))
            })?;
            body.extend_from_slice(bytes);
        }
        Ok(body)
    }

    #[cfg(target_arch = "wasm32")]
    fn get_range(&self, offset: u64, end: u64, _len: u64) -> Result<Vec<u8>, DataError> {
        let response =
            crate::web_http::request("GET", &self.url, Some((offset, end)), self.timeout)?;
        if response.status != 206 {
            return Err(DataError::Http(format!(
                "range request got {} (server ignores Range?)",
                response.status
            )));
        }
        Ok(response.body)
    }
}

impl RangeFetcher for HttpRange {
    fn fetch_range(&self, offset: u64, len: u64) -> Result<Vec<u8>, DataError> {
        if len == 0 {
            return Err(DataError::Http("range length must be non-zero".into()));
        }
        let end = offset
            .checked_add(len - 1)
            .ok_or_else(|| DataError::Http("range end overflow".into()))?;
        let wanted = usize::try_from(len)
            .map_err(|_| DataError::Http("range length does not fit memory".into()))?;
        let series = crate::metrics::http_acquisition("range", &self.url);
        let request = series.enqueue().start();
        let buf = match self.get_range(offset, end, len) {
            Ok(buf) => buf,
            Err(error) => {
                request.error();
                crate::metrics::record_err();
                return Err(error);
            }
        };
        if buf.len() != wanted {
            request.error();
            crate::metrics::record_err();
            return Err(DataError::Http(format!(
                "range returned {} bytes, wanted {len}",
                buf.len()
            )));
        }
        request.success(buf.len());
        crate::metrics::record(buf.len());
        Ok(buf)
    }
}

type LeafKey = (u64, u32);
type LeafSlot = OnceLock<Result<Arc<Vec<Entry>>, DataError>>;

/// Cancellation-safe shared result used by async cache fills. The operation
/// that owns a slot is spawned independently from its first waiter, so
/// dropping that waiter cannot strand everybody else behind an unfinished
/// entry.
#[cfg(feature = "async-http")]
pub(crate) struct AsyncSlot<T: Clone> {
    state: Mutex<AsyncSlotState<T>>,
}

#[cfg(feature = "async-http")]
enum AsyncSlotState<T> {
    Pending(Vec<Waker>),
    Ready(T),
}

#[cfg(feature = "async-http")]
impl<T: Clone> AsyncSlot<T> {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(AsyncSlotState::Pending(Vec::new())),
        }
    }

    pub(crate) fn ready(&self) -> Option<T> {
        match &*self.state.lock().unwrap() {
            AsyncSlotState::Ready(value) => Some(value.clone()),
            AsyncSlotState::Pending(_) => None,
        }
    }

    pub(crate) fn complete(&self, value: T) {
        let waiters = {
            let mut state = self.state.lock().unwrap();
            match std::mem::replace(&mut *state, AsyncSlotState::Ready(value)) {
                AsyncSlotState::Pending(waiters) => waiters,
                AsyncSlotState::Ready(_) => return,
            }
        };
        for waiter in waiters {
            waiter.wake();
        }
    }

    pub(crate) async fn wait(&self) -> T {
        std::future::poll_fn(|cx| {
            let mut state = self.state.lock().unwrap();
            match &mut *state {
                AsyncSlotState::Ready(value) => Poll::Ready(value.clone()),
                AsyncSlotState::Pending(waiters) => {
                    if !waiters.iter().any(|waiter| waiter.will_wake(cx.waker())) {
                        waiters.push(cx.waker().clone());
                    }
                    Poll::Pending
                }
            }
        })
        .await
    }
}

#[derive(Default)]
struct LeafCache {
    entries: HashMap<LeafKey, (Arc<LeafSlot>, u64)>,
    clock: u64,
}

impl LeafCache {
    /// Keep completed LRU entries bounded without ever evicting an in-flight
    /// slot. The cache may exceed the soft cap briefly when every entry is
    /// being initialized; preserving single-flight is more important than a
    /// momentary map entry.
    fn prune_completed(&mut self) {
        while self.entries.len() > LEAF_CACHE_MAX {
            let victim = self
                .entries
                .iter()
                .filter(|(_, (slot, _))| slot.get().is_some())
                .min_by_key(|(_, (_, used_at))| *used_at)
                .map(|(key, _)| *key);
            let Some(victim) = victim else { break };
            self.entries.remove(&victim);
        }
    }
}

#[cfg(feature = "async-http")]
type AsyncLeafSlot = AsyncSlot<Result<Arc<Vec<Entry>>, DataError>>;

#[cfg(feature = "async-http")]
#[derive(Default)]
struct AsyncLeafCache {
    /// Slot, LRU clock, and live maximum priority of active tile waiters.
    entries: HashMap<LeafKey, (Arc<AsyncLeafSlot>, u64, RequestPriority)>,
    clock: u64,
}

#[cfg(feature = "async-http")]
impl AsyncLeafCache {
    fn prune_completed(&mut self) {
        while self.entries.len() > LEAF_CACHE_MAX {
            let victim = self
                .entries
                .iter()
                .filter(|(_, (slot, _, _))| slot.ready().is_some())
                .min_by_key(|(_, (_, used_at, _))| *used_at)
                .map(|(key, _)| *key);
            let Some(victim) = victim else { break };
            self.entries.remove(&victim);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Gzip,
    Unsupported(u8),
}

impl Compression {
    fn from_byte(b: u8) -> Self {
        match b {
            0 | 1 => Compression::None,
            2 => Compression::Gzip,
            other => Compression::Unsupported(other),
        }
    }

    fn decompress(self, data: Vec<u8>) -> Result<Vec<u8>, DataError> {
        match self {
            Compression::None => Ok(data),
            Compression::Gzip => {
                let mut out = Vec::with_capacity(data.len() * 4);
                flate2::read::GzDecoder::new(&data[..])
                    .read_to_end(&mut out)
                    .map_err(|e| DataError::Decode(format!("gzip: {e}")))?;
                Ok(out)
            }
            Compression::Unsupported(b) => Err(DataError::Decode(format!(
                "pmtiles: unsupported compression {b}"
            ))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Header {
    pub spec_version: u8,
    pub root_offset: u64,
    pub root_len: u64,
    pub metadata_offset: u64,
    pub metadata_len: u64,
    pub leaf_offset: u64,
    pub tile_data_offset: u64,
    pub internal_compression: Compression,
    pub tile_compression: Compression,
    pub tile_type: u8,
    pub min_zoom: u8,
    pub max_zoom: u8,
}

pub const TILE_TYPE_MVT: u8 = 1;

fn parse_header(buf: &[u8]) -> Result<Header, DataError> {
    if buf.len() < 127 || &buf[0..7] != b"PMTiles" {
        return Err(DataError::Decode("pmtiles: bad magic".into()));
    }
    if buf[7] != 3 {
        return Err(DataError::Decode(format!("pmtiles: version {}", buf[7])));
    }
    let u64at = |o: usize| u64::from_le_bytes(buf[o..o + 8].try_into().unwrap());
    Ok(Header {
        spec_version: buf[7],
        root_offset: u64at(8),
        root_len: u64at(16),
        metadata_offset: u64at(24),
        metadata_len: u64at(32),
        leaf_offset: u64at(40),
        tile_data_offset: u64at(56),
        internal_compression: Compression::from_byte(buf[97]),
        tile_compression: Compression::from_byte(buf[98]),
        tile_type: buf[99],
        min_zoom: buf[100],
        max_zoom: buf[101],
    })
}

/// Directory entry: `run_length == 0` marks a leaf-directory pointer.
#[derive(Debug, Clone, Copy)]
struct Entry {
    tile_id: u64,
    offset: u64,
    length: u32,
    run_length: u32,
}

fn parse_directory(buf: &[u8]) -> Result<Vec<Entry>, DataError> {
    let mut pos = 0usize;
    let mut varint = || -> Result<u64, DataError> {
        let mut v = 0u64;
        let mut shift = 0u32;
        loop {
            let b = *buf
                .get(pos)
                .ok_or_else(|| DataError::Decode("pmtiles: truncated directory".into()))?;
            pos += 1;
            v |= ((b & 0x7f) as u64) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
            shift += 7;
        }
    };
    let n = varint()? as usize;
    if n > 10_000_000 {
        return Err(DataError::Decode("pmtiles: absurd directory size".into()));
    }
    let mut entries = vec![
        Entry {
            tile_id: 0,
            offset: 0,
            length: 0,
            run_length: 0
        };
        n
    ];
    let mut id = 0u64;
    for e in entries.iter_mut() {
        id += varint()?;
        e.tile_id = id;
    }
    for e in entries.iter_mut() {
        e.run_length = varint()? as u32;
    }
    for e in entries.iter_mut() {
        e.length = varint()? as u32;
    }
    for i in 0..n {
        let v = varint()?;
        entries[i].offset = if v == 0 && i > 0 {
            entries[i - 1].offset + entries[i - 1].length as u64
        } else {
            v - 1
        };
    }
    Ok(entries)
}

/// z/x/y → PMTiles tile id: cumulative count of tiles above `z`, plus the
/// Hilbert-curve index of (x, y) at order z. Signed intermediates are part
/// of the reference algorithm (rotation can go transiently negative).
pub fn tile_id(z: u8, x: u32, y: u32) -> u64 {
    let acc: u64 = (0..z).map(|i| 1u64 << (2 * i)).sum();
    let (mut x, mut y) = (x as i64, y as i64);
    let mut d = 0i64;
    let mut s = if z == 0 { 0 } else { 1i64 << (z - 1) };
    while s > 0 {
        let rx = i64::from(x & s > 0);
        let ry = i64::from(y & s > 0);
        d += s * s * ((3 * rx) ^ ry);
        // Rotate quadrant.
        if ry == 0 {
            if rx == 1 {
                x = s - 1 - x;
                y = s - 1 - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        s /= 2;
    }
    acc + d as u64
}

/// The immutable bytes that open one archive: its fixed header and its still
/// compressed root directory and metadata. An archive URL that names a single
/// immutable build can reuse them across launches instead of fetching them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PmTilesStartup {
    header: Vec<u8>,
    root: Vec<u8>,
    metadata: Vec<u8>,
}

impl PmTilesStartup {
    const MAGIC: &'static [u8; 8] = b"VPMSTRT1";

    /// Versioned, checksummed encoding for local storage.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            Self::MAGIC.len() + 16 + self.header.len() + self.root.len() + self.metadata.len(),
        );
        out.extend_from_slice(Self::MAGIC);
        for part in [&self.header, &self.root, &self.metadata] {
            out.extend_from_slice(&(part.len() as u32).to_le_bytes());
        }
        out.extend_from_slice(&self.checksum().to_le_bytes());
        for part in [&self.header, &self.root, &self.metadata] {
            out.extend_from_slice(part);
        }
        out
    }

    /// Accept only a complete, uncorrupted record whose header describes the
    /// root directory and metadata it carries.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let rest = bytes.strip_prefix(Self::MAGIC)?;
        let word = |index: usize| -> Option<u32> {
            Some(u32::from_le_bytes(
                rest.get(index * 4..index * 4 + 4)?.try_into().ok()?,
            ))
        };
        let lengths = [word(0)?, word(1)?, word(2)?].map(|len| len as usize);
        let checksum = word(3)?;
        let mut payload = rest.get(16..)?;
        if payload.len() != lengths.iter().sum::<usize>() {
            return None;
        }
        let mut take = |len: usize| {
            let (part, tail) = payload.split_at(len);
            payload = tail;
            part.to_vec()
        };
        let startup = Self {
            header: take(lengths[0]),
            root: take(lengths[1]),
            metadata: take(lengths[2]),
        };
        let header = parse_header(&startup.header).ok()?;
        (startup.checksum() == checksum
            && header.root_len == startup.root.len() as u64
            && header.metadata_len == startup.metadata.len() as u64)
            .then_some(startup)
    }

    fn checksum(&self) -> u32 {
        let mut crc = flate2::Crc::new();
        for part in [&self.header, &self.root, &self.metadata] {
            crc.update(part);
        }
        crc.sum()
    }

    #[cfg(feature = "async-http")]
    fn decode_root_and_metadata(self) -> Result<(Vec<Entry>, String), DataError> {
        let compression = parse_header(&self.header)?.internal_compression;
        let root = parse_directory(&compression.decompress(self.root)?)?;
        let metadata = String::from_utf8(compression.decompress(self.metadata)?)
            .map_err(|_| DataError::Decode("pmtiles: metadata utf8".into()))?;
        Ok((root, metadata))
    }
}

enum PmTilesBackend {
    Blocking(Box<dyn RangeFetcher>),
    #[cfg(feature = "async-http")]
    Async(Arc<dyn AsyncRangeFetcher>),
}

pub struct PmTiles {
    backend: PmTilesBackend,
    pub header: Header,
    root: Vec<Entry>,
    /// Decoded leaf directories, keyed by their offset within the leaf
    /// section. A planet archive has ~thousands; each is ~100–300 KB decoded
    /// and only a handful are hot for any camera position.
    leaves: Mutex<LeafCache>,
    #[cfg(feature = "async-http")]
    async_leaves: Arc<Mutex<AsyncLeafCache>>,
    leaf_metrics: crate::metrics::Acquisition,
}

const LEAF_CACHE_MAX: usize = 64;

impl PmTiles {
    pub fn open(fetcher: Box<dyn RangeFetcher>) -> Result<Self, DataError> {
        let head = fetcher.fetch_range(0, 127)?;
        let header = parse_header(&head)?;
        let root_raw = fetcher.fetch_range(header.root_offset, header.root_len)?;
        let root = parse_directory(&header.internal_compression.decompress(root_raw)?)?;
        Ok(PmTiles {
            backend: PmTilesBackend::Blocking(fetcher),
            header,
            root,
            leaves: Mutex::new(LeafCache::default()),
            #[cfg(feature = "async-http")]
            async_leaves: Arc::new(Mutex::new(AsyncLeafCache::default())),
            leaf_metrics: crate::metrics::acquisition("pmtiles.leaf"),
        })
    }

    /// Open an archive without blocking a Tokio executor thread. Header and
    /// root-directory decompression/parsing run on the blocking pool; all
    /// byte ranges remain reactor-native.
    #[cfg(feature = "async-http")]
    pub async fn open_async(fetcher: Arc<dyn AsyncRangeFetcher>) -> Result<Self, DataError> {
        let head = fetcher.fetch_range(0, 127).await?;
        let header = vearth_platform::rt::task::spawn_blocking(move || parse_header(&head))
            .await
            .map_err(|error| DataError::Decode(format!("pmtiles header task: {error}")))??;
        let root_raw = fetcher
            .fetch_range(header.root_offset, header.root_len)
            .await?;
        let compression = header.internal_compression;
        let root = vearth_platform::rt::task::spawn_blocking(move || {
            compression
                .decompress(root_raw)
                .and_then(|raw| parse_directory(&raw))
        })
        .await
        .map_err(|error| DataError::Decode(format!("pmtiles root task: {error}")))??;
        Ok(PmTiles {
            backend: PmTilesBackend::Async(fetcher),
            header,
            root,
            leaves: Mutex::new(LeafCache::default()),
            async_leaves: Arc::new(Mutex::new(AsyncLeafCache::default())),
            leaf_metrics: crate::metrics::acquisition("pmtiles.leaf"),
        })
    }

    /// Open the archive and validate/decode its metadata with root and metadata
    /// range requests in parallel after the header has supplied their offsets.
    #[cfg(feature = "async-http")]
    pub async fn open_async_with_metadata(
        fetcher: Arc<dyn AsyncRangeFetcher>,
    ) -> Result<(Self, String), DataError> {
        Self::open_async_with_startup(fetcher)
            .await
            .map(|(archive, metadata, _)| (archive, metadata))
    }

    /// Like [`Self::open_async_with_metadata`], also returning the immutable
    /// startup bytes so a later launch can reopen the same archive offline.
    #[cfg(feature = "async-http")]
    pub async fn open_async_with_startup(
        fetcher: Arc<dyn AsyncRangeFetcher>,
    ) -> Result<(Self, String, PmTilesStartup), DataError> {
        use std::task::Poll;

        let head = fetcher.fetch_range(0, 127).await?;
        let header = parse_header(&head)?;
        let mut root_request = fetcher.fetch_range(header.root_offset, header.root_len);
        let mut metadata_request = fetcher.fetch_range(header.metadata_offset, header.metadata_len);
        let (mut root_raw, mut metadata_raw) = (None, None);
        let (root_raw, metadata_raw) = std::future::poll_fn(|cx| {
            if root_raw.is_none() {
                match root_request.as_mut().poll(cx) {
                    Poll::Ready(Ok(bytes)) => root_raw = Some(bytes),
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                    Poll::Pending => {}
                }
            }
            if metadata_raw.is_none() {
                match metadata_request.as_mut().poll(cx) {
                    Poll::Ready(Ok(bytes)) => metadata_raw = Some(bytes),
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                    Poll::Pending => {}
                }
            }
            if root_raw.is_some() && metadata_raw.is_some() {
                Poll::Ready(Ok((root_raw.take().unwrap(), metadata_raw.take().unwrap())))
            } else {
                Poll::Pending
            }
        })
        .await?;
        drop((root_request, metadata_request));
        let startup = PmTilesStartup {
            header: head,
            root: root_raw,
            metadata: metadata_raw,
        };
        let parts = startup.clone();
        let (root, metadata) =
            vearth_platform::rt::task::spawn_blocking(move || parts.decode_root_and_metadata())
                .await
                .map_err(|error| DataError::Decode(format!("pmtiles startup task: {error}")))??;
        Ok((
            Self::async_archive(fetcher, header, root),
            metadata,
            startup,
        ))
    }

    /// Open from startup bytes recorded for this exact archive. Only tile and
    /// leaf-directory reads use `fetcher`; opening performs no network I/O.
    #[cfg(feature = "async-http")]
    pub fn open_async_from_startup(
        fetcher: Arc<dyn AsyncRangeFetcher>,
        startup: &PmTilesStartup,
    ) -> Result<(Self, String), DataError> {
        let header = parse_header(&startup.header)?;
        let (root, metadata) = startup.clone().decode_root_and_metadata()?;
        Ok((Self::async_archive(fetcher, header, root), metadata))
    }

    #[cfg(feature = "async-http")]
    fn async_archive(
        fetcher: Arc<dyn AsyncRangeFetcher>,
        header: Header,
        root: Vec<Entry>,
    ) -> Self {
        Self {
            backend: PmTilesBackend::Async(fetcher),
            header,
            root,
            leaves: Mutex::new(LeafCache::default()),
            async_leaves: Arc::new(Mutex::new(AsyncLeafCache::default())),
            leaf_metrics: crate::metrics::acquisition("pmtiles.leaf"),
        }
    }

    /// True when this archive must be warmed through the async API. Useful to
    /// enforce that synchronous generator paths remain cache-only.
    pub fn is_async(&self) -> bool {
        #[cfg(feature = "async-http")]
        if matches!(self.backend, PmTilesBackend::Async(_)) {
            return true;
        }
        false
    }

    fn blocking_fetcher(&self) -> Result<&dyn RangeFetcher, DataError> {
        match &self.backend {
            PmTilesBackend::Blocking(fetcher) => Ok(fetcher.as_ref()),
            #[cfg(feature = "async-http")]
            PmTilesBackend::Async(_) => Err(DataError::NotAvailable),
        }
    }

    #[cfg(feature = "async-http")]
    fn async_fetcher(&self) -> Result<Arc<dyn AsyncRangeFetcher>, DataError> {
        match &self.backend {
            PmTilesBackend::Async(fetcher) => Ok(fetcher.clone()),
            PmTilesBackend::Blocking(_) => Err(DataError::NotAvailable),
        }
    }

    /// Archive metadata JSON (attribution, layer schema).
    pub fn metadata_json(&self) -> Result<String, DataError> {
        let raw = self
            .blocking_fetcher()?
            .fetch_range(self.header.metadata_offset, self.header.metadata_len)?;
        let bytes = self.header.internal_compression.decompress(raw)?;
        String::from_utf8(bytes).map_err(|_| DataError::Decode("pmtiles: metadata utf8".into()))
    }

    fn leaf(&self, offset: u64, len: u32) -> Result<Arc<Vec<Entry>>, DataError> {
        let key = (offset, len);
        let slot = {
            let mut cache = self.leaves.lock().unwrap();
            cache.clock = cache.clock.wrapping_add(1);
            let used_at = cache.clock;
            if let Some((slot, last_used)) = cache.entries.get_mut(&key) {
                *last_used = used_at;
                slot.clone()
            } else {
                let slot = Arc::new(LeafSlot::new());
                cache.entries.insert(key, (slot.clone(), used_at));
                cache.prune_completed();
                slot
            }
        };

        if let Some(hit) = slot.get() {
            let bytes = hit
                .as_ref()
                .map_or(0, |entries| entries.len() * std::mem::size_of::<Entry>());
            self.leaf_metrics.cache_hit(bytes);
            return hit.clone();
        }

        self.leaf_metrics.cache_miss();
        let waited_at = Instant::now();
        let mut initialized_here = false;
        let result = slot
            .get_or_init(|| {
                initialized_here = true;
                let request = self.leaf_metrics.enqueue().start();
                let raw = match self.blocking_fetcher().and_then(|fetcher| {
                    fetcher.fetch_range(self.header.leaf_offset + offset, len as u64)
                }) {
                    Ok(raw) => raw,
                    Err(error) => {
                        request.error();
                        return Err(error);
                    }
                };
                let raw_len = raw.len();
                let entries = match self
                    .header
                    .internal_compression
                    .decompress(raw)
                    .and_then(|raw| parse_directory(&raw))
                {
                    Ok(entries) => entries,
                    Err(error) => {
                        request.error();
                        return Err(error);
                    }
                };
                request.success(raw_len);
                Ok(Arc::new(entries))
            })
            .clone();

        if !initialized_here {
            self.leaf_metrics.shared_wait(waited_at.elapsed());
        } else {
            let mut cache = self.leaves.lock().unwrap();
            if result.is_err()
                && cache
                    .entries
                    .get(&key)
                    .is_some_and(|(cached, _)| Arc::ptr_eq(cached, &slot))
            {
                // A failed initialization is shared with current waiters but
                // never poisons a later attempt.
                cache.entries.remove(&key);
            }
            cache.prune_completed();
        }
        result
    }

    #[cfg(feature = "async-http")]
    async fn leaf_async(
        &self,
        offset: u64,
        len: u32,
        priority: RequestPriority,
    ) -> Result<Arc<Vec<Entry>>, DataError> {
        let fetcher = self.async_fetcher()?;
        let key = (offset, len);
        let (slot, flight_priority, leader) = {
            let mut cache = self.async_leaves.lock().unwrap();
            cache.clock = cache.clock.wrapping_add(1);
            let used_at = cache.clock;
            if let Some((slot, last_used, flight_priority)) = cache.entries.get_mut(&key) {
                *last_used = used_at;
                (slot.clone(), flight_priority.clone(), false)
            } else {
                let slot = Arc::new(AsyncLeafSlot::new());
                let flight_priority = RequestPriority::shared();
                cache
                    .entries
                    .insert(key, (slot.clone(), used_at, flight_priority.clone()));
                cache.prune_completed();
                (slot, flight_priority, true)
            }
        };
        let _priority_link = flight_priority.link(&priority);

        if let Some(hit) = slot.ready() {
            let bytes = hit
                .as_ref()
                .map_or(0, |entries| entries.len() * std::mem::size_of::<Entry>());
            self.leaf_metrics.cache_hit(bytes);
            return hit;
        }

        self.leaf_metrics.cache_miss();
        let waited_at = Instant::now();
        if leader {
            let slot_for_task = slot.clone();
            let cache = self.async_leaves.clone();
            let metrics = self.leaf_metrics.clone();
            let leaf_offset = self.header.leaf_offset;
            let compression = self.header.internal_compression;
            vearth_platform::rt::spawn(async move {
                let request = metrics.enqueue().start();
                let result = match fetcher
                    .fetch_range_with_priority(leaf_offset + offset, len as u64, flight_priority)
                    .await
                {
                    Ok(raw) => {
                        let raw_len = raw.len();
                        match vearth_platform::rt::task::spawn_blocking(move || {
                            compression
                                .decompress(raw)
                                .and_then(|raw| parse_directory(&raw))
                                .map(Arc::new)
                        })
                        .await
                        {
                            Ok(Ok(entries)) => {
                                request.success(raw_len);
                                Ok(entries)
                            }
                            Ok(Err(error)) => {
                                request.error();
                                Err(error)
                            }
                            Err(error) => {
                                request.error();
                                Err(DataError::Decode(format!("pmtiles leaf task: {error}")))
                            }
                        }
                    }
                    Err(error) => {
                        request.error();
                        Err(error)
                    }
                };

                if result.is_err() {
                    let mut cache = cache.lock().unwrap();
                    if cache
                        .entries
                        .get(&key)
                        .is_some_and(|(cached, _, _)| Arc::ptr_eq(cached, &slot_for_task))
                    {
                        cache.entries.remove(&key);
                    }
                }
                slot_for_task.complete(result);
                cache.lock().unwrap().prune_completed();
            });
        }

        let result = slot.wait().await;
        if !leader {
            self.leaf_metrics.shared_wait(waited_at.elapsed());
        }
        result
    }

    /// Async metadata counterpart for archives opened with `PmTiles::open_async`.
    #[cfg(feature = "async-http")]
    pub async fn metadata_json_async(&self) -> Result<String, DataError> {
        let fetcher = self.async_fetcher()?;
        let raw = fetcher
            .fetch_range(self.header.metadata_offset, self.header.metadata_len)
            .await?;
        let compression = self.header.internal_compression;
        vearth_platform::rt::task::spawn_blocking(move || {
            let bytes = compression.decompress(raw)?;
            String::from_utf8(bytes).map_err(|_| DataError::Decode("pmtiles: metadata utf8".into()))
        })
        .await
        .map_err(|error| DataError::Decode(format!("pmtiles metadata task: {error}")))?
    }

    /// Fetch and decompress one tile through an async range backend. Leaf
    /// directories use a bounded, failure-evicting keyed single-flight.
    #[cfg(feature = "async-http")]
    pub async fn tile_async(&self, z: u8, x: u32, y: u32) -> Result<Option<Vec<u8>>, DataError> {
        self.tile_async_with_priority_handle(z, x, y, RequestPriority::default())
            .await
    }

    /// Priority-carrying variant used by async provider preparation. Shared
    /// leaf flights retain the maximum priority observed across waiters.
    #[cfg(feature = "async-http")]
    pub async fn tile_async_with_priority(
        &self,
        z: u8,
        x: u32,
        y: u32,
        priority: f64,
    ) -> Result<Option<Vec<u8>>, DataError> {
        self.tile_async_with_priority_handle(z, x, y, RequestPriority::new(priority))
            .await
    }

    /// Live-handle variant used by shared Protomaps flights. Late waiters can
    /// raise (or, when canceled, lower) queued HTTP admission priority.
    #[cfg(feature = "async-http")]
    pub async fn tile_async_with_priority_handle(
        &self,
        z: u8,
        x: u32,
        y: u32,
        priority: RequestPriority,
    ) -> Result<Option<Vec<u8>>, DataError> {
        let fetcher = self.async_fetcher()?;
        if z > self.header.max_zoom {
            return Ok(None);
        }
        let tid = tile_id(z, x, y);
        let mut entries: &[Entry] = &self.root;
        let mut leaf_hold;
        for _ in 0..8 {
            let idx = match entries.partition_point(|entry| entry.tile_id <= tid) {
                0 => return Ok(None),
                position => position - 1,
            };
            let entry = entries[idx];
            if entry.run_length == 0 {
                leaf_hold = self
                    .leaf_async(entry.offset, entry.length, priority.clone())
                    .await?;
                entries = &leaf_hold;
                continue;
            }
            if tid - entry.tile_id < entry.run_length as u64 {
                let raw = fetcher
                    .fetch_range_with_priority(
                        self.header.tile_data_offset + entry.offset,
                        entry.length as u64,
                        priority.clone(),
                    )
                    .await?;
                let compression = self.header.tile_compression;
                return vearth_platform::rt::task::spawn_blocking(move || {
                    compression.decompress(raw)
                })
                .await
                .map_err(|error| DataError::Decode(format!("pmtiles tile decode task: {error}")))?
                .map(Some);
            }
            return Ok(None);
        }
        Err(DataError::Decode(
            "pmtiles: directory nesting too deep".into(),
        ))
    }

    /// Fetch and decompress one tile; None if absent from the archive.
    pub fn tile(&self, z: u8, x: u32, y: u32) -> Result<Option<Vec<u8>>, DataError> {
        let fetcher = self.blocking_fetcher()?;
        if z > self.header.max_zoom {
            return Ok(None);
        }
        let tid = tile_id(z, x, y);
        let mut entries: &[Entry] = &self.root;
        let mut leaf_hold;
        // ≤4 levels of directories per spec; guard against cycles anyway.
        for _ in 0..8 {
            // Last entry with tile_id <= tid.
            let idx = match entries.partition_point(|e| e.tile_id <= tid) {
                0 => return Ok(None),
                p => p - 1,
            };
            let e = entries[idx];
            if e.run_length == 0 {
                leaf_hold = self.leaf(e.offset, e.length)?;
                entries = &leaf_hold;
                continue;
            }
            if tid - e.tile_id < e.run_length as u64 {
                let raw = fetcher
                    .fetch_range(self.header.tile_data_offset + e.offset, e.length as u64)?;
                return Ok(Some(self.header.tile_compression.decompress(raw)?));
            }
            return Ok(None);
        }
        Err(DataError::Decode(
            "pmtiles: directory nesting too deep".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "async-http")]
    use crate::async_http::{AsyncRangeFetcher, FetchFuture};
    #[cfg(feature = "async-http")]
    use std::future::Future as _;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use vearth_platform::time::Duration;

    #[test]
    fn tile_id_matches_spec_examples() {
        // Reference values from the PMTiles spec / js implementation.
        assert_eq!(tile_id(0, 0, 0), 0);
        assert_eq!(tile_id(1, 0, 0), 1);
        assert_eq!(tile_id(1, 0, 1), 2);
        assert_eq!(tile_id(1, 1, 1), 3);
        assert_eq!(tile_id(1, 1, 0), 4);
        assert_eq!(tile_id(2, 0, 0), 5);
        // The Manhattan probe value cross-checked against the python impl.
        assert_eq!(tile_id(15, 9649, 12314), 552516028);
    }

    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(data).unwrap();
        enc.finish().unwrap()
    }

    fn varint(out: &mut Vec<u8>, mut v: u64) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                break;
            }
            out.push(b | 0x80);
        }
    }

    /// Build a minimal one-tile archive in memory and serve it via FileRange.
    fn build_archive(dir_gzip: bool, tile_payload: &[u8], z: u8, x: u32, y: u32) -> Vec<u8> {
        let tid = tile_id(z, x, y);
        let tile_stored = gzip(tile_payload);

        let mut dir = Vec::new();
        varint(&mut dir, 1); // one entry
        varint(&mut dir, tid);
        varint(&mut dir, 1); // run_length
        varint(&mut dir, tile_stored.len() as u64);
        varint(&mut dir, 1); // offset+1 (0 within tile data)
        let dir_stored = if dir_gzip { gzip(&dir) } else { dir };

        let meta = br#"{"name":"test"}"#;
        let meta_stored = if dir_gzip { gzip(meta) } else { meta.to_vec() };

        let root_offset = 127u64;
        let meta_offset = root_offset + dir_stored.len() as u64;
        let tile_offset = meta_offset + meta_stored.len() as u64;

        let mut buf = Vec::new();
        buf.extend_from_slice(b"PMTiles");
        buf.push(3);
        let mut u64le = |v: u64| buf.extend_from_slice(&v.to_le_bytes());
        u64le(root_offset);
        u64le(dir_stored.len() as u64);
        u64le(meta_offset);
        u64le(meta_stored.len() as u64);
        u64le(0); // leaf offset
        u64le(0); // leaf len
        u64le(tile_offset);
        u64le(tile_stored.len() as u64);
        u64le(1); // addressed tiles
        u64le(1); // tile entries
        u64le(1); // tile contents
        buf.push(1); // clustered
        buf.push(if dir_gzip { 2 } else { 1 }); // internal compression
        buf.push(2); // tile compression: gzip
        buf.push(TILE_TYPE_MVT);
        buf.push(0); // min zoom
        buf.push(15); // max zoom
        while buf.len() < 127 {
            buf.push(0);
        }
        assert_eq!(buf.len(), 127);
        buf.extend_from_slice(&dir_stored);
        buf.extend_from_slice(&meta_stored);
        buf.extend_from_slice(&tile_stored);
        buf
    }

    /// One-tile archive whose root points through a leaf directory, plus the
    /// exact leaf range for contention instrumentation.
    fn build_leaf_archive(tile_payload: &[u8], z: u8, x: u32, y: u32) -> (Vec<u8>, u64, u64) {
        let tid = tile_id(z, x, y);

        let mut leaf = Vec::new();
        varint(&mut leaf, 1);
        varint(&mut leaf, tid);
        varint(&mut leaf, 1); // one tile
        varint(&mut leaf, tile_payload.len() as u64);
        varint(&mut leaf, 1); // tile data offset 0 + 1

        let mut root = Vec::new();
        varint(&mut root, 1);
        varint(&mut root, tid);
        varint(&mut root, 0); // leaf pointer
        varint(&mut root, leaf.len() as u64);
        varint(&mut root, 1); // leaf offset 0 + 1

        let root_offset = 127u64;
        let leaf_offset = root_offset + root.len() as u64;
        let tile_offset = leaf_offset + leaf.len() as u64;
        let mut buf = Vec::new();
        buf.extend_from_slice(b"PMTiles");
        buf.push(3);
        let mut u64le = |v: u64| buf.extend_from_slice(&v.to_le_bytes());
        u64le(root_offset);
        u64le(root.len() as u64);
        u64le(leaf_offset); // zero-length metadata may share the boundary
        u64le(0);
        u64le(leaf_offset);
        u64le(leaf.len() as u64);
        u64le(tile_offset);
        u64le(tile_payload.len() as u64);
        u64le(1);
        u64le(1);
        u64le(1);
        buf.push(1); // clustered
        buf.push(1); // internal compression: none
        buf.push(1); // tile compression: none
        buf.push(TILE_TYPE_MVT);
        buf.push(z);
        buf.push(z);
        while buf.len() < 127 {
            buf.push(0);
        }
        buf.extend_from_slice(&root);
        buf.extend_from_slice(&leaf);
        buf.extend_from_slice(tile_payload);
        (buf, leaf_offset, leaf.len() as u64)
    }

    struct SlowCountingRange {
        bytes: Arc<Vec<u8>>,
        leaf_offset: u64,
        leaf_len: u64,
        leaf_fetches: Arc<AtomicUsize>,
    }

    struct FlakyLeafRange {
        bytes: Arc<Vec<u8>>,
        leaf_offset: u64,
        leaf_len: u64,
        leaf_attempts: Arc<AtomicUsize>,
    }

    impl RangeFetcher for FlakyLeafRange {
        fn fetch_range(&self, offset: u64, len: u64) -> Result<Vec<u8>, DataError> {
            if offset == self.leaf_offset
                && len == self.leaf_len
                && self.leaf_attempts.fetch_add(1, Ordering::SeqCst) == 0
            {
                return Err(DataError::Http("transient leaf failure".into()));
            }
            let start = usize::try_from(offset).unwrap();
            let end = start + usize::try_from(len).unwrap();
            self.bytes
                .get(start..end)
                .map(|bytes| bytes.to_vec())
                .ok_or_else(|| DataError::Http("test range outside archive".into()))
        }
    }

    impl RangeFetcher for SlowCountingRange {
        fn fetch_range(&self, offset: u64, len: u64) -> Result<Vec<u8>, DataError> {
            if offset == self.leaf_offset && len == self.leaf_len {
                self.leaf_fetches.fetch_add(1, Ordering::SeqCst);
                // Hold the leader long enough for every synchronized caller
                // to reach the same not-yet-initialized slot.
                vearth_platform::thread::sleep(Duration::from_millis(75));
            }
            let start = usize::try_from(offset).unwrap();
            let end = start + usize::try_from(len).unwrap();
            self.bytes
                .get(start..end)
                .map(|bytes| bytes.to_vec())
                .ok_or_else(|| DataError::Http("test range outside archive".into()))
        }
    }

    fn open_bytes(bytes: &[u8]) -> PmTiles {
        let dir = std::env::temp_dir().join(format!("vearth-pmtiles-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("a{}.pmtiles", bytes.len()));
        std::fs::write(&path, bytes).unwrap();
        PmTiles::open(Box::new(FileRange(std::fs::File::open(&path).unwrap()))).unwrap()
    }

    #[test]
    fn reads_back_tile_and_metadata() {
        let payload = b"hello mvt".to_vec();
        let pm = open_bytes(&build_archive(true, &payload, 15, 9649, 12314));
        assert_eq!(pm.header.max_zoom, 15);
        assert_eq!(pm.tile(15, 9649, 12314).unwrap().unwrap(), payload);
        assert_eq!(pm.tile(15, 9650, 12314).unwrap(), None);
        assert_eq!(pm.tile(3, 1, 1).unwrap(), None);
        assert!(pm.metadata_json().unwrap().contains("test"));
    }

    #[test]
    fn uncompressed_directories_work() {
        let payload = vec![7u8; 100];
        let pm = open_bytes(&build_archive(false, &payload, 4, 5, 6));
        assert_eq!(pm.tile(4, 5, 6).unwrap().unwrap(), payload);
    }

    #[test]
    fn concurrent_leaf_misses_share_one_range_read() {
        const WORKERS: usize = 12;
        let payload = b"shared leaf payload".to_vec();
        let (archive, leaf_offset, leaf_len) = build_leaf_archive(&payload, 8, 73, 101);
        let leaf_fetches = Arc::new(AtomicUsize::new(0));
        let pm = Arc::new(
            PmTiles::open(Box::new(SlowCountingRange {
                bytes: Arc::new(archive),
                leaf_offset,
                leaf_len,
                leaf_fetches: leaf_fetches.clone(),
            }))
            .unwrap(),
        );
        let barrier = Arc::new(Barrier::new(WORKERS));

        vearth_platform::thread::scope(|scope| {
            let mut threads = Vec::new();
            for _ in 0..WORKERS {
                let pm = pm.clone();
                let barrier = barrier.clone();
                let payload = payload.clone();
                threads.push(scope.spawn(move || {
                    barrier.wait();
                    assert_eq!(pm.tile(8, 73, 101).unwrap().unwrap(), payload);
                }));
            }
            for thread in threads {
                thread.join().unwrap();
            }
        });

        assert_eq!(leaf_fetches.load(Ordering::SeqCst), 1);
        // The completed slot remains a normal cache hit.
        assert_eq!(pm.tile(8, 73, 101).unwrap().unwrap(), payload);
        assert_eq!(leaf_fetches.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn failed_leaf_single_flight_does_not_poison_retry() {
        let payload = b"retry leaf payload".to_vec();
        let (archive, leaf_offset, leaf_len) = build_leaf_archive(&payload, 8, 73, 101);
        let leaf_attempts = Arc::new(AtomicUsize::new(0));
        let pm = PmTiles::open(Box::new(FlakyLeafRange {
            bytes: Arc::new(archive),
            leaf_offset,
            leaf_len,
            leaf_attempts: leaf_attempts.clone(),
        }))
        .unwrap();

        assert!(pm.tile(8, 73, 101).is_err());
        assert_eq!(pm.tile(8, 73, 101).unwrap().unwrap(), payload);
        assert_eq!(leaf_attempts.load(Ordering::SeqCst), 2);
    }

    #[cfg(feature = "async-http")]
    struct AsyncTestRange {
        bytes: Arc<Vec<u8>>,
        leaf_range: Option<(u64, u64)>,
        leaf_attempts: Arc<AtomicUsize>,
        leaf_delay: Duration,
        fail_first_leaf: bool,
        leaf_started: Option<Arc<tokio::sync::Notify>>,
        leaf_release: Option<Arc<tokio::sync::Notify>>,
        observed_priority: Option<Arc<Mutex<Option<RequestPriority>>>>,
    }

    #[cfg(feature = "async-http")]
    impl AsyncTestRange {
        fn memory(bytes: Vec<u8>) -> Self {
            Self {
                bytes: Arc::new(bytes),
                leaf_range: None,
                leaf_attempts: Arc::new(AtomicUsize::new(0)),
                leaf_delay: Duration::ZERO,
                fail_first_leaf: false,
                leaf_started: None,
                leaf_release: None,
                observed_priority: None,
            }
        }

        fn leaf_archive(
            bytes: Vec<u8>,
            leaf_offset: u64,
            leaf_len: u64,
            attempts: Arc<AtomicUsize>,
        ) -> Self {
            Self {
                bytes: Arc::new(bytes),
                leaf_range: Some((leaf_offset, leaf_len)),
                leaf_attempts: attempts,
                leaf_delay: Duration::ZERO,
                fail_first_leaf: false,
                leaf_started: None,
                leaf_release: None,
                observed_priority: None,
            }
        }
    }

    #[cfg(feature = "async-http")]
    impl AsyncRangeFetcher for AsyncTestRange {
        fn fetch_range(&self, offset: u64, len: u64) -> FetchFuture<'_> {
            let bytes = self.bytes.clone();
            let is_leaf = self.leaf_range == Some((offset, len));
            let attempts = self.leaf_attempts.clone();
            let delay = self.leaf_delay;
            let fail_first = self.fail_first_leaf;
            let started = self.leaf_started.clone();
            let release = self.leaf_release.clone();
            Box::pin(async move {
                if is_leaf {
                    let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                    if let Some(started) = started {
                        started.notify_one();
                    }
                    if let Some(release) = release {
                        release.notified().await;
                    }
                    if !delay.is_zero() {
                        vearth_platform::rt::time::sleep(delay).await;
                    }
                    if fail_first && attempt == 0 {
                        return Err(DataError::Http("transient async leaf failure".into()));
                    }
                }
                let start = usize::try_from(offset).unwrap();
                let end = start + usize::try_from(len).unwrap();
                bytes
                    .get(start..end)
                    .map(|range| range.to_vec())
                    .ok_or_else(|| DataError::Http("test range outside archive".into()))
            })
        }

        fn fetch_range_with_priority(
            &self,
            offset: u64,
            len: u64,
            priority: RequestPriority,
        ) -> FetchFuture<'_> {
            if self.leaf_range == Some((offset, len))
                && let Some(observed) = &self.observed_priority
            {
                *observed.lock().unwrap() = Some(priority);
            }
            self.fetch_range(offset, len)
        }
    }

    #[cfg(feature = "async-http")]
    fn async_runtime() -> vearth_platform::rt::runtime::Runtime {
        vearth_platform::rt::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn async_reader_matches_blocking_reader_and_sync_calls_fail_fast() {
        async_runtime().block_on(async {
            let payload = b"async parity payload".to_vec();
            let archive = build_archive(true, &payload, 8, 73, 101);
            let blocking = open_bytes(&archive);
            let asynchronous = PmTiles::open_async(Arc::new(AsyncTestRange::memory(archive)))
                .await
                .unwrap();

            assert!(asynchronous.is_async());
            assert_eq!(
                asynchronous.tile_async(8, 73, 101).await.unwrap(),
                blocking.tile(8, 73, 101).unwrap()
            );
            assert_eq!(asynchronous.tile_async(8, 74, 101).await.unwrap(), None);
            assert_eq!(
                asynchronous.metadata_json_async().await.unwrap(),
                blocking.metadata_json().unwrap()
            );
            assert_eq!(asynchronous.tile(8, 73, 101), Err(DataError::NotAvailable));
            assert_eq!(asynchronous.metadata_json(), Err(DataError::NotAvailable));
        });
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn startup_reads_root_and_metadata_concurrently_without_duplicate_ranges() {
        struct ConcurrentStartupRange {
            bytes: Arc<Vec<u8>>,
            barrier: Arc<tokio::sync::Barrier>,
            requests: Arc<Mutex<Vec<(u64, u64)>>>,
        }
        impl AsyncRangeFetcher for ConcurrentStartupRange {
            fn fetch_range(&self, offset: u64, len: u64) -> FetchFuture<'_> {
                Box::pin(async move {
                    self.requests.lock().unwrap().push((offset, len));
                    if offset != 0 {
                        self.barrier.wait().await;
                    }
                    Ok(self.bytes[offset as usize..(offset + len) as usize].to_vec())
                })
            }
        }
        async_runtime().block_on(async {
            let archive = build_archive(true, b"startup tile", 8, 73, 101);
            let reference = open_bytes(&archive);
            let requests = Arc::new(Mutex::new(Vec::new()));
            let range = ConcurrentStartupRange {
                bytes: Arc::new(archive),
                barrier: Arc::new(tokio::sync::Barrier::new(2)),
                requests: requests.clone(),
            };
            let (reader, metadata) = vearth_platform::rt::time::timeout(
                Duration::from_secs(1),
                PmTiles::open_async_with_metadata(Arc::new(range)),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(metadata, reference.metadata_json().unwrap());
            let entries = |reader: &PmTiles| {
                reader
                    .root
                    .iter()
                    .map(|e| (e.tile_id, e.offset, e.length, e.run_length))
                    .collect::<Vec<_>>()
            };
            assert_eq!(entries(&reader), entries(&reference));
            assert_eq!(reader.header.root_offset, reference.header.root_offset);
            let requests = requests.lock().unwrap();
            assert_eq!(requests.len(), 3);
            assert_eq!(requests[0], (0, 127));
            assert!(requests.contains(&(reader.header.root_offset, reader.header.root_len)));
            assert!(
                requests.contains(&(reader.header.metadata_offset, reader.header.metadata_len))
            );
        });
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn recorded_startup_reopens_the_same_archive_without_startup_reads() {
        struct CountingRange {
            bytes: Vec<u8>,
            requests: Arc<Mutex<Vec<(u64, u64)>>>,
        }
        impl AsyncRangeFetcher for CountingRange {
            fn fetch_range(&self, offset: u64, len: u64) -> FetchFuture<'_> {
                Box::pin(async move {
                    self.requests.lock().unwrap().push((offset, len));
                    Ok(self.bytes[offset as usize..(offset + len) as usize].to_vec())
                })
            }
        }
        async_runtime().block_on(async {
            let archive = build_archive(true, b"recorded startup tile", 8, 73, 101);
            let range = |requests: &Arc<Mutex<Vec<(u64, u64)>>>| {
                Arc::new(CountingRange {
                    bytes: archive.clone(),
                    requests: requests.clone(),
                })
            };
            let network = Arc::new(Mutex::new(Vec::new()));
            let (online, online_metadata, startup) =
                PmTiles::open_async_with_startup(range(&network))
                    .await
                    .unwrap();
            let recorded = PmTilesStartup::from_bytes(&startup.to_bytes()).unwrap();
            assert_eq!(recorded, startup);

            let offline = Arc::new(Mutex::new(Vec::new()));
            let (reopened, reopened_metadata) =
                PmTiles::open_async_from_startup(range(&offline), &recorded).unwrap();
            assert!(offline.lock().unwrap().is_empty(), "opening must not read");
            assert_eq!(reopened_metadata, online_metadata);
            let entries = |reader: &PmTiles| {
                reader
                    .root
                    .iter()
                    .map(|e| (e.tile_id, e.offset, e.length, e.run_length))
                    .collect::<Vec<_>>()
            };
            assert_eq!(entries(&reopened), entries(&online));
            assert_eq!(
                reopened.tile_async(8, 73, 101).await.unwrap(),
                online.tile_async(8, 73, 101).await.unwrap()
            );
            assert_eq!(
                offline.lock().unwrap().len(),
                1,
                "only the tile range is read"
            );
        });
    }

    #[test]
    fn startup_records_reject_corruption_truncation_and_mismatched_headers() {
        let archive = build_archive(true, b"tile", 8, 73, 101);
        let header = parse_header(&archive[..127]).unwrap();
        let slice =
            |offset: u64, len: u64| archive[offset as usize..(offset + len) as usize].to_vec();
        let startup = PmTilesStartup {
            header: archive[..127].to_vec(),
            root: slice(header.root_offset, header.root_len),
            metadata: slice(header.metadata_offset, header.metadata_len),
        };
        let bytes = startup.to_bytes();
        assert_eq!(PmTilesStartup::from_bytes(&bytes), Some(startup.clone()));
        for index in [0, 9, 30, bytes.len() - 1] {
            let mut corrupt = bytes.clone();
            corrupt[index] ^= 0x40;
            assert_eq!(PmTilesStartup::from_bytes(&corrupt), None, "byte {index}");
        }
        assert_eq!(PmTilesStartup::from_bytes(&bytes[..bytes.len() - 1]), None);
        let mut extended = bytes.clone();
        extended.push(0);
        assert_eq!(PmTilesStartup::from_bytes(&extended), None);
        let mismatched = PmTilesStartup {
            root: startup.metadata.clone(),
            ..startup
        };
        assert_eq!(PmTilesStartup::from_bytes(&mismatched.to_bytes()), None);
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn concurrent_async_leaf_misses_share_one_range_read() {
        async_runtime().block_on(async {
            const WORKERS: usize = 12;
            let payload = b"async shared leaf payload".to_vec();
            let (archive, leaf_offset, leaf_len) = build_leaf_archive(&payload, 8, 73, 101);
            let attempts = Arc::new(AtomicUsize::new(0));
            let mut fetcher =
                AsyncTestRange::leaf_archive(archive, leaf_offset, leaf_len, attempts.clone());
            fetcher.leaf_delay = Duration::from_millis(75);
            let pm = Arc::new(PmTiles::open_async(Arc::new(fetcher)).await.unwrap());

            let mut workers = Vec::new();
            for _ in 0..WORKERS {
                let pm = pm.clone();
                let payload = payload.clone();
                workers.push(vearth_platform::rt::spawn(async move {
                    assert_eq!(pm.tile_async(8, 73, 101).await.unwrap().unwrap(), payload);
                }));
            }
            for worker in workers {
                worker.await.unwrap();
            }
            assert_eq!(attempts.load(Ordering::SeqCst), 1);
            assert_eq!(pm.tile_async(8, 73, 101).await.unwrap().unwrap(), payload);
            assert_eq!(attempts.load(Ordering::SeqCst), 1);
        });
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn canceling_first_async_leaf_waiter_does_not_cancel_shared_fill() {
        async_runtime().block_on(async {
            let payload = b"async cancellation payload".to_vec();
            let (archive, leaf_offset, leaf_len) = build_leaf_archive(&payload, 8, 73, 101);
            let attempts = Arc::new(AtomicUsize::new(0));
            let started = Arc::new(tokio::sync::Notify::new());
            let release = Arc::new(tokio::sync::Notify::new());
            let mut fetcher =
                AsyncTestRange::leaf_archive(archive, leaf_offset, leaf_len, attempts.clone());
            fetcher.leaf_started = Some(started.clone());
            fetcher.leaf_release = Some(release.clone());
            let pm = Arc::new(PmTiles::open_async(Arc::new(fetcher)).await.unwrap());

            let first = {
                let pm = pm.clone();
                vearth_platform::rt::spawn(async move { pm.tile_async(8, 73, 101).await })
            };
            started.notified().await;
            first.abort();
            assert!(first.await.unwrap_err().is_cancelled());
            let second = {
                let pm = pm.clone();
                vearth_platform::rt::spawn(async move { pm.tile_async(8, 73, 101).await })
            };
            release.notify_one();
            assert_eq!(second.await.unwrap().unwrap().unwrap(), payload);
            assert_eq!(attempts.load(Ordering::SeqCst), 1);
        });
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn failed_async_leaf_single_flight_does_not_poison_retry() {
        async_runtime().block_on(async {
            let payload = b"async retry leaf payload".to_vec();
            let (archive, leaf_offset, leaf_len) = build_leaf_archive(&payload, 8, 73, 101);
            let attempts = Arc::new(AtomicUsize::new(0));
            let mut fetcher =
                AsyncTestRange::leaf_archive(archive, leaf_offset, leaf_len, attempts.clone());
            fetcher.fail_first_leaf = true;
            let pm = PmTiles::open_async(Arc::new(fetcher)).await.unwrap();

            assert!(pm.tile_async(8, 73, 101).await.is_err());
            assert_eq!(pm.tile_async(8, 73, 101).await.unwrap().unwrap(), payload);
            assert_eq!(attempts.load(Ordering::SeqCst), 2);
        });
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn async_leaf_flight_exposes_max_active_waiter_priority() {
        async_runtime().block_on(async {
            let payload = b"priority leaf payload".to_vec();
            let (archive, leaf_offset, leaf_len) = build_leaf_archive(&payload, 8, 73, 101);
            let attempts = Arc::new(AtomicUsize::new(0));
            let started = Arc::new(tokio::sync::Notify::new());
            let release = Arc::new(tokio::sync::Notify::new());
            let observed = Arc::new(Mutex::new(None));
            let mut fetcher =
                AsyncTestRange::leaf_archive(archive, leaf_offset, leaf_len, attempts.clone());
            fetcher.leaf_started = Some(started.clone());
            fetcher.leaf_release = Some(release.clone());
            fetcher.observed_priority = Some(observed.clone());
            let pm = Arc::new(PmTiles::open_async(Arc::new(fetcher)).await.unwrap());

            let low = {
                let pm = pm.clone();
                vearth_platform::rt::spawn(async move {
                    pm.tile_async_with_priority(8, 73, 101, 1.0).await
                })
            };
            started.notified().await;
            let mut high = Box::pin(pm.tile_async_with_priority(8, 73, 101, 10.0));
            std::future::poll_fn(|cx| match high.as_mut().poll(cx) {
                Poll::Pending => Poll::Ready(()),
                Poll::Ready(_) => panic!("shared leaf unexpectedly completed while gated"),
            })
            .await;
            let priority = observed.lock().unwrap().as_ref().unwrap().clone();
            assert_eq!(priority.current(), 10.0);
            drop(high);
            assert_eq!(priority.current(), 1.0);
            release.notify_one();
            assert_eq!(low.await.unwrap().unwrap().unwrap(), payload);
            assert_eq!(attempts.load(Ordering::SeqCst), 1);
        });
    }
}
