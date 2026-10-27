//! Reads of a hosted build, block object by block object.
//!
//! Tiles of one area sit together in an archive, so one block covers the
//! neighbours a view is about to need. Block URLs are shared by every reader,
//! so HTTP and CDN caches serve repeats. Blocks are kept in a bounded
//! in-memory LRU; concurrent readers of a block share one download.

use super::{AttemptError, BlockAdmission, Client, fetch_block};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use tokio::sync::OnceCell;
use vearth_platform::time::Duration;

const CACHE_BYTES: usize = 96 * 1024 * 1024;

pub(super) type Block = Arc<[u8]>;

struct Slot {
    cell: Arc<OnceCell<Block>>,
    used: u64,
}

#[derive(Default)]
struct Cache {
    slots: HashMap<(String, u64), Slot>,
    clock: u64,
    bytes: usize,
}

impl Cache {
    fn cell(&mut self, url: &str, index: u64) -> Arc<OnceCell<Block>> {
        self.clock += 1;
        let slot = self
            .slots
            .entry((url.to_owned(), index))
            .or_insert_with(|| Slot {
                cell: Arc::new(OnceCell::new()),
                used: 0,
            });
        slot.used = self.clock;
        slot.cell.clone()
    }

    /// Accounts a block that just finished downloading, then evicts the
    /// least recently used downloaded blocks beyond the budget.
    fn filled(&mut self, len: usize) {
        self.bytes += len;
        while self.bytes > CACHE_BYTES {
            let Some(oldest) = self
                .slots
                .iter()
                .filter(|(_, slot)| slot.cell.initialized())
                .min_by_key(|(_, slot)| slot.used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(slot) = self.slots.remove(&oldest) {
                self.bytes -= slot.cell.get().map_or(0, |block| block.len());
            }
        }
    }
}

static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(Mutex::default);

fn cache() -> std::sync::MutexGuard<'static, Cache> {
    CACHE.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) async fn read(
    client: &Client,
    url: &str,
    offset: u64,
    end: u64,
    wanted: usize,
    stall: Option<Duration>,
    admission: &BlockAdmission,
) -> Result<Vec<u8>, AttemptError> {
    let mut bytes = Vec::with_capacity(wanted);
    for (index, span) in crate::vectors::block_spans(offset, end) {
        let block = block(client, url, index, stall, admission).await?;
        let part = block.get(span).ok_or_else(|| {
            AttemptError::fatal(format!(
                "range {offset}-{end} extends past the end of {url}"
            ))
        })?;
        bytes.extend_from_slice(part);
    }
    if bytes.len() != wanted {
        return Err(AttemptError::fatal(format!(
            "range {offset}-{end} assembled {} bytes, wanted {wanted}",
            bytes.len()
        )));
    }
    Ok(bytes)
}

async fn block(
    client: &Client,
    url: &str,
    index: u64,
    stall: Option<Duration>,
    admission: &BlockAdmission,
) -> Result<Block, AttemptError> {
    let cell = cache().cell(url, index);
    let mut fetched = false;
    let block = cell
        .get_or_try_init(|| async {
            fetched = true;
            let _permit = admission.acquire().await;
            fetch_block(client, url, index, stall).await
        })
        .await?
        .clone();
    if fetched {
        cache().filled(block.len());
    }
    Ok(block)
}
