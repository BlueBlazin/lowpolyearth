//! Opt-in reactor-native HTTP transport boundary.
//!
//! These object-safe transports deliberately stop at raw bytes. Generator-
//! facing source traits stay synchronous, so a priority coordinator can await
//! and cache dependencies before handing the unchanged deterministic generator
//! a memory-backed view—without per-sample futures.
//!
//! AsyncHttpFetcher and AsyncHttpRange run reqwest on one dedicated, bounded
//! Tokio runtime. One priority dispatcher atomically grants both the global
//! ceiling and each clone-shared provider budget before polling network I/O.
//! Reqwest provides nonblocking sockets and pooled connections. The
//! from_fetcher constructors keep a fixed-size blocking worker pool for tests
//! and incremental migration only.

use crate::pmtiles::RangeFetcher;
use crate::{DataError, TileFetcher, metrics};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::task::{Context, Poll, Waker};
#[cfg(not(target_arch = "wasm32"))]
use vearth_platform::rt::runtime::{Builder, Runtime};
use vearth_platform::rt::task::JoinHandle;
use vearth_platform::time::Duration;

pub type FetchFuture<'a> = Pin<Box<dyn Future<Output = Result<Vec<u8>, DataError>> + Send + 'a>>;

/// Cloneable, live request priority consumed by HTTP admission.
///
/// A fixed handle is convenient for ordinary provider requests. Shared cache
/// flights use [`Self::shared`] plus leases or linked child handles so the
/// queued operation continuously inherits the maximum priority of its active
/// waiters. Non-finite and signed zero values normalize to zero, matching the
/// streaming queue.
#[derive(Clone)]
pub struct RequestPriority {
    inner: Arc<Mutex<RequestPriorityState>>,
}

struct RequestPriorityState {
    base: Option<f64>,
    next_id: u64,
    contributions: HashMap<u64, PriorityContribution>,
}

#[derive(Clone)]
enum PriorityContribution {
    Value(f64),
    Linked(RequestPriority),
}

/// An active contribution to a shared request priority. Dropping it removes
/// that waiter from the inherited maximum before admission selects the next
/// queued request.
pub struct RequestPriorityLease {
    priority: RequestPriority,
    id: Option<u64>,
}

fn normalized_priority(priority: f64) -> f64 {
    if !priority.is_finite() || priority == 0.0 {
        0.0
    } else {
        priority
    }
}

fn max_priority(left: f64, right: f64) -> f64 {
    if right.total_cmp(&left).is_gt() {
        right
    } else {
        left
    }
}

impl RequestPriority {
    pub fn new(priority: f64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RequestPriorityState {
                base: Some(normalized_priority(priority)),
                next_id: 0,
                contributions: HashMap::new(),
            })),
        }
    }

    /// Empty shared priority. Each active cache waiter should hold a lease for
    /// exactly as long as it awaits the shared result.
    pub fn shared() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RequestPriorityState {
                base: None,
                next_id: 0,
                contributions: HashMap::new(),
            })),
        }
    }

    pub fn lease(&self, priority: f64) -> RequestPriorityLease {
        self.insert(PriorityContribution::Value(normalized_priority(priority)))
    }

    /// Link another live priority into this one. This is used when a PMTiles
    /// leaf flight is shared by multiple tile flights: changes to any tile's
    /// active waiter set remain visible to the already-queued range request.
    pub fn link(&self, child: &RequestPriority) -> RequestPriorityLease {
        assert!(
            !Arc::ptr_eq(&self.inner, &child.inner),
            "request priority cannot link to itself"
        );
        self.insert(PriorityContribution::Linked(child.clone()))
    }

    fn insert(&self, contribution: PriorityContribution) -> RequestPriorityLease {
        let id = {
            let mut state = self.inner.lock().unwrap();
            let id = state.next_id;
            state.next_id = state.next_id.wrapping_add(1);
            state.contributions.insert(id, contribution);
            id
        };
        RequestPriorityLease {
            priority: self.clone(),
            id: Some(id),
        }
    }

    pub fn current(&self) -> f64 {
        // Clone before following linked handles so nested priorities never
        // hold two mutexes simultaneously and cannot deadlock.
        let (base, contributions) = {
            let state = self.inner.lock().unwrap();
            (
                state.base,
                state.contributions.values().cloned().collect::<Vec<_>>(),
            )
        };
        let mut current = base;
        for contribution in contributions {
            let value = match contribution {
                PriorityContribution::Value(value) => value,
                PriorityContribution::Linked(priority) => priority.current(),
            };
            current = Some(current.map_or(value, |old| max_priority(old, value)));
        }
        current.unwrap_or(0.0)
    }
}

impl Default for RequestPriority {
    fn default() -> Self {
        Self::new(0.0)
    }
}

impl Drop for RequestPriorityLease {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            self.priority
                .inner
                .lock()
                .unwrap()
                .contributions
                .remove(&id);
        }
    }
}

/// Object-safe async equivalent of TileFetcher.
pub trait AsyncTileFetcher: Send + Sync {
    fn fetch<'a>(&'a self, url: &'a str) -> FetchFuture<'a>;

    /// Priority-aware admission. Existing fake/custom implementations remain
    /// source-compatible and inherit the legacy behavior by default.
    fn fetch_with_priority<'a>(
        &'a self,
        url: &'a str,
        priority: RequestPriority,
    ) -> FetchFuture<'a> {
        let _ = priority;
        self.fetch(url)
    }
}

/// Object-safe async equivalent of RangeFetcher.
pub trait AsyncRangeFetcher: Send + Sync {
    fn fetch_range(&self, offset: u64, len: u64) -> FetchFuture<'_>;

    /// Priority-aware range admission. Existing fake/custom implementations
    /// need only implement [`Self::fetch_range`].
    fn fetch_range_with_priority(
        &self,
        offset: u64,
        len: u64,
        priority: RequestPriority,
    ) -> FetchFuture<'_> {
        let _ = priority;
        self.fetch_range(offset, len)
    }
}

const TILE_TIMEOUT: Duration = Duration::from_secs(20);
const RANGE_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(not(target_arch = "wasm32"))]
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
#[cfg(not(target_arch = "wasm32"))]
const TCP_KEEPALIVE: Duration = Duration::from_secs(30);
// Keep the whole Terrarium quota warm: a fresh TLS connection to a distant
// object store costs several round trips before the first byte.
#[cfg(not(target_arch = "wasm32"))]
const IDLE_CONNECTIONS_PER_HOST: usize = 96;
// Room for the Terrarium (96) and Protomaps (64) quotas together.
const MAX_GLOBAL_REQUESTS: usize = 192;

/// Stall recovery for idempotent static objects. An attempt that receives no
/// bytes (headers or body) for `stall` is abandoned and reissued; the last of
/// `attempts` waits out the client's whole-request timeout instead. Object
/// stores occasionally leave single requests hanging for minutes while their
/// siblings complete in about a second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StallRetry {
    pub stall: Duration,
    pub attempts: u32,
}

/// Per-provider quota consumed by the joint global priority dispatcher.
struct ProviderAdmission {
    capacity: usize,
    available: AtomicUsize,
}

impl ProviderAdmission {
    fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "HTTP admission capacity must be non-zero");
        Self {
            capacity,
            available: AtomicUsize::new(capacity),
        }
    }

    fn try_reserve(&self) -> bool {
        let mut available = self.available.load(AtomicOrdering::Acquire);
        while available > 0 {
            match self.available.compare_exchange_weak(
                available,
                available - 1,
                AtomicOrdering::AcqRel,
                AtomicOrdering::Acquire,
            ) {
                Ok(_) => return true,
                Err(current) => available = current,
            }
        }
        false
    }

    fn release(&self) {
        let previous = self.available.fetch_add(1, AtomicOrdering::Release);
        debug_assert!(previous < self.capacity);
    }
}

enum AdmissionWaiterState {
    Pending(Option<Waker>),
    Granted,
    Consumed,
    Canceled,
}

struct AdmissionWaiter {
    sequence: u64,
    priority: RequestPriority,
    provider: Arc<ProviderAdmission>,
    state: Mutex<AdmissionWaiterState>,
}

struct PriorityAdmissionState {
    capacity: usize,
    available: usize,
    next_sequence: u64,
    waiters: Vec<Arc<AdmissionWaiter>>,
}

/// One process-wide queue that grants the global and provider budgets
/// atomically. Every dispatch re-reads live priorities, so a shared cache
/// flight can inherit a later, higher-priority waiter before it starts I/O.
struct PriorityAdmission {
    state: Mutex<PriorityAdmissionState>,
}

impl PriorityAdmission {
    fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "HTTP admission capacity must be non-zero");
        Self {
            state: Mutex::new(PriorityAdmissionState {
                capacity,
                available: capacity,
                next_sequence: 0,
                waiters: Vec::new(),
            }),
        }
    }

    fn acquire(
        self: &Arc<Self>,
        provider: Arc<ProviderAdmission>,
        priority: RequestPriority,
    ) -> AdmissionFuture {
        let waiter;
        let wakes = {
            let mut state = self.state.lock().unwrap();
            let sequence = state.next_sequence;
            state.next_sequence = state.next_sequence.wrapping_add(1);
            waiter = Arc::new(AdmissionWaiter {
                sequence,
                priority,
                provider,
                state: Mutex::new(AdmissionWaiterState::Pending(None)),
            });
            state.waiters.push(waiter.clone());
            Self::dispatch_locked(&mut state)
        };
        for wake in wakes {
            wake.wake();
        }
        AdmissionFuture {
            admission: self.clone(),
            waiter,
            completed: false,
        }
    }

    fn dispatch_locked(state: &mut PriorityAdmissionState) -> Vec<Waker> {
        let mut wakes = Vec::new();
        while state.available > 0 {
            let mut best: Option<(usize, f64, u64)> = None;
            for (index, waiter) in state.waiters.iter().enumerate() {
                if waiter.provider.available.load(AtomicOrdering::Acquire) == 0
                    || !matches!(
                        *waiter.state.lock().unwrap(),
                        AdmissionWaiterState::Pending(_)
                    )
                {
                    continue;
                }
                let priority = waiter.priority.current();
                let replace = best.is_none_or(|(_, old_priority, old_sequence)| {
                    priority.total_cmp(&old_priority).is_gt()
                        || (priority.total_cmp(&old_priority).is_eq()
                            && waiter.sequence < old_sequence)
                });
                if replace {
                    best = Some((index, priority, waiter.sequence));
                }
            }
            let Some((index, _, _)) = best else { break };
            let waiter = state.waiters[index].clone();
            let mut waiter_state = waiter.state.lock().unwrap();
            if !matches!(*waiter_state, AdmissionWaiterState::Pending(_)) {
                continue;
            }
            if !waiter.provider.try_reserve() {
                continue;
            }
            let wake = match std::mem::replace(&mut *waiter_state, AdmissionWaiterState::Granted) {
                AdmissionWaiterState::Pending(wake) => wake,
                _ => unreachable!(),
            };
            drop(waiter_state);
            state.waiters.swap_remove(index);
            state.available -= 1;
            if let Some(wake) = wake {
                wakes.push(wake);
            }
        }
        wakes
    }

    fn cancel_pending(&self, sequence: u64) {
        let wakes = {
            let mut state = self.state.lock().unwrap();
            if let Some(index) = state
                .waiters
                .iter()
                .position(|waiter| waiter.sequence == sequence)
            {
                state.waiters.swap_remove(index);
            }
            Self::dispatch_locked(&mut state)
        };
        for wake in wakes {
            wake.wake();
        }
    }

    fn release(&self, provider: &ProviderAdmission) {
        provider.release();
        let wakes = {
            let mut state = self.state.lock().unwrap();
            debug_assert!(state.available < state.capacity);
            state.available += 1;
            Self::dispatch_locked(&mut state)
        };
        for wake in wakes {
            wake.wake();
        }
    }

    #[cfg(test)]
    fn queued_len(&self) -> usize {
        self.state.lock().unwrap().waiters.len()
    }
}

struct AdmissionFuture {
    admission: Arc<PriorityAdmission>,
    waiter: Arc<AdmissionWaiter>,
    completed: bool,
}

impl Future for AdmissionFuture {
    type Output = PriorityPermit;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut state = this.waiter.state.lock().unwrap();
        match &mut *state {
            AdmissionWaiterState::Pending(waker) => {
                if waker.as_ref().is_none_or(|old| !old.will_wake(cx.waker())) {
                    *waker = Some(cx.waker().clone());
                }
                Poll::Pending
            }
            AdmissionWaiterState::Granted => {
                *state = AdmissionWaiterState::Consumed;
                this.completed = true;
                Poll::Ready(PriorityPermit {
                    admission: this.admission.clone(),
                    provider: this.waiter.provider.clone(),
                })
            }
            AdmissionWaiterState::Consumed | AdmissionWaiterState::Canceled => {
                panic!("HTTP admission future polled after completion")
            }
        }
    }
}

impl Drop for AdmissionFuture {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        enum Cancellation {
            Pending,
            Granted,
            None,
        }
        let cancellation = {
            let mut state = self.waiter.state.lock().unwrap();
            match *state {
                AdmissionWaiterState::Pending(_) => {
                    *state = AdmissionWaiterState::Canceled;
                    Cancellation::Pending
                }
                AdmissionWaiterState::Granted => {
                    *state = AdmissionWaiterState::Canceled;
                    Cancellation::Granted
                }
                AdmissionWaiterState::Consumed | AdmissionWaiterState::Canceled => {
                    Cancellation::None
                }
            }
        };
        match cancellation {
            Cancellation::Pending => self.admission.cancel_pending(self.waiter.sequence),
            Cancellation::Granted => self.admission.release(&self.waiter.provider),
            Cancellation::None => {}
        }
    }
}

struct PriorityPermit {
    admission: Arc<PriorityAdmission>,
    provider: Arc<ProviderAdmission>,
}

impl Drop for PriorityPermit {
    fn drop(&mut self) {
        self.admission.release(&self.provider);
    }
}

/// A cancellation-safe, live-priority concurrency gate for provider work
/// that must happen before HTTP admission (for example disk reads and
/// decoding). Keeping this gate outside the transport bounds an eager tile
/// plan's pre-admission work while preserving priority-before-admission
/// semantics.
pub struct PriorityGate {
    admission: Arc<PriorityAdmission>,
    budget: Arc<ProviderAdmission>,
}

impl PriorityGate {
    pub fn new(max_active: usize) -> Self {
        assert!(max_active > 0, "priority gate capacity must be non-zero");
        Self {
            admission: Arc::new(PriorityAdmission::new(max_active)),
            budget: Arc::new(ProviderAdmission::new(max_active)),
        }
    }

    pub async fn acquire(&self, priority: RequestPriority) -> PriorityGatePermit {
        PriorityGatePermit {
            _permit: self.admission.acquire(self.budget.clone(), priority).await,
        }
    }
}

/// Releases a [`PriorityGate`] slot when dropped.
pub struct PriorityGatePermit {
    _permit: PriorityPermit,
}

struct HttpReactor {
    #[cfg(not(target_arch = "wasm32"))]
    runtime: Runtime,
    admission: Arc<PriorityAdmission>,
}

impl HttpReactor {
    fn new() -> Self {
        // Browser fetches run on the platform reactor worker instead.
        #[cfg(not(target_arch = "wasm32"))]
        let runtime = Builder::new_multi_thread()
            .worker_threads(
                vearth_platform::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(4)
                    .clamp(2, 4),
            )
            .thread_name("vearth-http-reactor")
            .enable_all()
            .build()
            .expect("vearth async HTTP runtime");
        // Socket concurrency is independent of CPU worker count. Let source
        // quotas overlap while retaining a hard process-wide request ceiling;
        // decoding and generation have their own bounded worker pools.
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            runtime,
            admission: Arc::new(PriorityAdmission::new(MAX_GLOBAL_REQUESTS)),
        }
    }
}

fn reactor() -> &'static HttpReactor {
    static REACTOR: OnceLock<HttpReactor> = OnceLock::new();
    REACTOR.get_or_init(HttpReactor::new)
}

#[cfg(target_arch = "wasm32")]
#[path = "async_http_web.rs"]
mod web;

#[cfg(not(target_arch = "wasm32"))]
type Client = reqwest::Client;
#[cfg(target_arch = "wasm32")]
type Client = web::Client;

#[cfg(target_arch = "wasm32")]
fn http_client(timeout: Duration) -> Client {
    web::client(timeout)
}

#[cfg(not(target_arch = "wasm32"))]
fn http_client(timeout: Duration) -> Client {
    // reqwest's no-provider feature avoids pulling a second crypto backend.
    // The blocking ureq stack already uses rustls+ring, so explicitly install
    // that provider before reqwest constructs its TLS configuration. Losing a
    // race means another valid process-wide provider was installed first.
    static TLS_PROVIDER: OnceLock<()> = OnceLock::new();
    TLS_PROVIDER.get_or_init(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
    reqwest::Client::builder()
        .timeout(timeout)
        .user_agent(concat!("vearth/", env!("CARGO_PKG_VERSION")))
        .pool_idle_timeout(POOL_IDLE_TIMEOUT)
        .pool_max_idle_per_host(IDLE_CONNECTIONS_PER_HOST)
        .tcp_keepalive(TCP_KEEPALIVE)
        .build()
        .expect("vearth reqwest client")
}

struct ReactorFuture {
    task: JoinHandle<Result<Vec<u8>, DataError>>,
}

impl Future for ReactorFuture {
    type Output = Result<Vec<u8>, DataError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.task).poll(cx) {
            Poll::Ready(Ok(result)) => Poll::Ready(result),
            Poll::Ready(Err(error)) => Poll::Ready(Err(DataError::Http(format!(
                "async HTTP task failed: {error}"
            )))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for ReactorFuture {
    fn drop(&mut self) {
        // Tokio abort drops the queued or in-flight telemetry guard inside the
        // task, releases its joint admission permit, and cancels reqwest I/O.
        self.task.abort();
    }
}

fn default_transport_admission() -> Arc<ProviderAdmission> {
    // This is deliberately no tighter than any possible process ceiling, so
    // the legacy constructors retain their old effective concurrency and do
    // not initialize the reactor merely by being constructed.
    Arc::new(ProviderAdmission::new(MAX_GLOBAL_REQUESTS))
}

fn bounded_transport_admission(max_in_flight: usize) -> Arc<ProviderAdmission> {
    assert!(
        max_in_flight > 0,
        "async HTTP provider limit must be greater than zero"
    );
    Arc::new(ProviderAdmission::new(max_in_flight))
}

fn provider_series(kind: &str, label: impl Into<String>) -> metrics::Acquisition {
    let label = label.into();
    assert!(
        !label.trim().is_empty(),
        "async HTTP provider label must not be empty"
    );
    metrics::acquisition(format!("async.{kind}.reactor:{label}"))
}

/// A transport operation's future. Browser fetch futures hold JavaScript
/// objects and are therefore built on, and never leave, the reactor worker.
#[cfg(not(target_arch = "wasm32"))]
trait Operation: Future<Output = Result<Vec<u8>, DataError>> + Send + 'static {}
#[cfg(not(target_arch = "wasm32"))]
impl<F: Future<Output = Result<Vec<u8>, DataError>> + Send + 'static> Operation for F {}
#[cfg(target_arch = "wasm32")]
trait Operation: Future<Output = Result<Vec<u8>, DataError>> + 'static {}
#[cfg(target_arch = "wasm32")]
impl<F: Future<Output = Result<Vec<u8>, DataError>> + 'static> Operation for F {}

fn spawn_reactor<M, F>(
    series: metrics::Acquisition,
    transport_admission: Arc<ProviderAdmission>,
    priority: RequestPriority,
    operation: M,
) -> ReactorFuture
where
    M: FnOnce() -> F + Send + 'static,
    F: Operation,
{
    spawn_reactor_with_admission(
        series,
        reactor().admission.clone(),
        transport_admission,
        priority,
        operation,
    )
}

fn spawn_reactor_with_admission<M, F>(
    series: metrics::Acquisition,
    global_admission: Arc<PriorityAdmission>,
    transport_admission: Arc<ProviderAdmission>,
    priority: RequestPriority,
    operation: M,
) -> ReactorFuture
where
    M: FnOnce() -> F + Send + 'static,
    F: Operation,
{
    // Construct the guard before spawning: even cancellation before the
    // reactor's first poll is counted as a queued cancellation.
    let queued = series.enqueue();
    let task = move || async move {
        let _permit = global_admission
            .acquire(transport_admission, priority)
            .await;
        let in_flight = queued.start();
        let result = operation().await;
        match &result {
            Ok(bytes) => in_flight.success(bytes.len()),
            Err(DataError::NotAvailable) => in_flight.not_found(),
            Err(_) => in_flight.error(),
        }
        result
    };
    #[cfg(not(target_arch = "wasm32"))]
    let task = reactor().runtime.spawn(task());
    #[cfg(target_arch = "wasm32")]
    let task = vearth_platform::rt::spawn_local_on_reactor(task);
    ReactorFuture { task }
}

/// Archive reads share block downloads (see [`blocks`]). Only a read that
/// starts a download takes the global and provider permits, and only for that
/// download: readers waiting on a block already in flight must not hold
/// transport slots, or a few blocks would occupy the whole budget.
struct BlockAdmission {
    global: Arc<PriorityAdmission>,
    provider: Arc<ProviderAdmission>,
    priority: RequestPriority,
}

impl BlockAdmission {
    async fn acquire(&self) -> PriorityPermit {
        self.global
            .acquire(self.provider.clone(), self.priority.clone())
            .await
    }
}

/// [`spawn_reactor`] for an operation that takes its own permits.
fn spawn_reactor_self_admitted<M, F>(series: metrics::Acquisition, operation: M) -> ReactorFuture
where
    M: FnOnce() -> F + Send + 'static,
    F: Operation,
{
    let queued = series.enqueue();
    let task = move || async move {
        let in_flight = queued.start();
        let result = operation().await;
        match &result {
            Ok(bytes) => in_flight.success(bytes.len()),
            Err(DataError::NotAvailable) => in_flight.not_found(),
            Err(_) => in_flight.error(),
        }
        result
    };
    #[cfg(not(target_arch = "wasm32"))]
    let task = reactor().runtime.spawn(task());
    #[cfg(target_arch = "wasm32")]
    let task = vearth_platform::rt::spawn_local_on_reactor(task);
    ReactorFuture { task }
}

#[path = "async_http_blocks.rs"]
mod blocks;

/// [`fetch_http_range`] for a hosted build, through shared blocks.
async fn fetch_archive_range(
    client: Client,
    url: Arc<str>,
    offset: u64,
    end: u64,
    wanted: usize,
    stall_retry: Option<StallRetry>,
    admission: BlockAdmission,
) -> Result<Vec<u8>, DataError> {
    let attempts = stall_retry.map_or(1, |retry| retry.attempts.max(1));
    let mut attempt = 1;
    let result = loop {
        let stall = stall_retry
            .filter(|_| attempt < attempts)
            .map(|retry| retry.stall);
        #[cfg(target_arch = "wasm32")]
        let outcome =
            web::fetch_archive_attempt(&client, &url, offset, end, wanted, stall, &admission).await;
        #[cfg(not(target_arch = "wasm32"))]
        let outcome = blocks::read(&client, &url, offset, end, wanted, stall, &admission).await;
        match outcome {
            Ok(bytes) => break Ok(bytes),
            Err(failure) if failure.retryable && attempt < attempts => {
                log::debug!(
                    "retrying archive range {offset}-{end} after attempt {attempt}: {}",
                    failure.error
                );
                attempt += 1;
            }
            Err(failure) => break Err(failure.error),
        }
    };
    match result {
        Ok(bytes) => {
            metrics::record(bytes.len());
            Ok(bytes)
        }
        Err(error) => {
            metrics::record_err();
            Err(error)
        }
    }
}

struct AttemptError {
    error: DataError,
    retryable: bool,
}

impl AttemptError {
    fn retryable(error: impl ToString) -> Self {
        Self {
            error: DataError::Http(error.to_string()),
            retryable: true,
        }
    }

    fn fatal(error: impl ToString) -> Self {
        Self {
            error: DataError::Http(error.to_string()),
            retryable: false,
        }
    }
}

/// Await `future`, or give up after `stall` when one is set.
async fn within<T>(stall: Option<Duration>, future: impl Future<Output = T>) -> Option<T> {
    match stall {
        Some(stall) => vearth_platform::rt::time::timeout(stall, future).await.ok(),
        None => Some(future.await),
    }
}

#[cfg(target_arch = "wasm32")]
use web::{fetch_block, fetch_http_attempt, fetch_http_range_attempt};

/// One attempt at downloading block `index` of the hosted build at `url`.
#[cfg(not(target_arch = "wasm32"))]
async fn fetch_block(
    client: &Client,
    url: &str,
    index: u64,
    stall: Option<Duration>,
) -> Result<blocks::Block, AttemptError> {
    let size = crate::vectors::HOSTED_BUILD_BLOCK_BYTES as usize;
    let stalled =
        || AttemptError::retryable(format!("no data for {:?}", stall.unwrap_or_default()));
    let request = client.get(crate::vectors::block_url(url, index)).send();
    let mut response = within(stall, request)
        .await
        .ok_or_else(stalled)?
        .map_err(AttemptError::retryable)?;
    let status = response.status();
    if status != reqwest::StatusCode::OK {
        return Err(AttemptError {
            error: DataError::Http(format!("archive block {index} got {status}")),
            retryable: status.is_server_error(),
        });
    }
    let expected = response.content_length();
    let mut bytes = Vec::with_capacity(size);
    while let Some(chunk) = within(stall, response.chunk())
        .await
        .ok_or_else(stalled)?
        .map_err(AttemptError::retryable)?
    {
        if bytes.len() + chunk.len() > size {
            return Err(AttemptError::fatal(format!(
                "archive block {index} is larger than {size} bytes"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    // Every block but the archive's last is full; reads past the end of a
    // short block fail in `blocks::read`. A body that ends early is a dropped
    // connection, and must not be cached as a short block.
    if bytes.is_empty() || expected.is_some_and(|expected| expected != bytes.len() as u64) {
        return Err(AttemptError::retryable(format!(
            "archive block {index} returned {} bytes",
            bytes.len()
        )));
    }
    Ok(bytes.into())
}

#[cfg(not(target_arch = "wasm32"))]
async fn fetch_http_attempt(
    client: &Client,
    url: &str,
    stall: Option<Duration>,
) -> Result<Vec<u8>, AttemptError> {
    let stalled =
        || AttemptError::retryable(format!("no data for {:?}", stall.unwrap_or_default()));
    let mut response = within(stall, client.get(url).send())
        .await
        .ok_or_else(stalled)?
        .map_err(AttemptError::retryable)?;
    let status = response.status();
    // Match the blocking ureq transport: 404 means the object does not
    // exist, other 4xx/5xx are errors, as is a redirect response that
    // reqwest could not follow (normally because it omitted Location).
    // Followable redirects have already been resolved by reqwest.
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(AttemptError {
            error: DataError::NotAvailable,
            retryable: false,
        });
    }
    let terminal_redirect = matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308);
    if status.is_client_error() || status.is_server_error() || terminal_redirect {
        return Err(AttemptError {
            error: DataError::Http(format!("request got {status}")),
            retryable: status.is_server_error(),
        });
    }
    if stall.is_none() {
        return response
            .bytes()
            .await
            .map(|bytes| bytes.to_vec())
            .map_err(AttemptError::retryable);
    }
    let mut body = Vec::new();
    while let Some(chunk) = within(stall, response.chunk())
        .await
        .ok_or_else(stalled)?
        .map_err(AttemptError::retryable)?
    {
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn fetch_http_body(
    client: Client,
    url: String,
    stall_retry: Option<StallRetry>,
) -> Result<Vec<u8>, DataError> {
    let host = metrics::http_acquisition("tile", &url);
    let request = host.enqueue().start();
    let attempts = stall_retry.map_or(1, |retry| retry.attempts.max(1));
    let mut attempt = 1;
    let result = loop {
        let stall = stall_retry
            .filter(|_| attempt < attempts)
            .map(|retry| retry.stall);
        match fetch_http_attempt(&client, &url, stall).await {
            Ok(bytes) => break Ok(bytes),
            Err(failure) if failure.retryable && attempt < attempts => {
                log::debug!("retrying {url} after attempt {attempt}: {}", failure.error);
                attempt += 1;
            }
            Err(failure) => break Err(failure.error),
        }
    };
    match result {
        Ok(bytes) => {
            request.success(bytes.len());
            metrics::record(bytes.len());
            Ok(bytes)
        }
        Err(DataError::NotAvailable) => {
            request.not_found();
            Err(DataError::NotAvailable)
        }
        Err(error) => {
            request.error();
            metrics::record_err();
            Err(error)
        }
    }
}

fn checked_range(offset: u64, len: u64) -> Result<(u64, usize), DataError> {
    if len == 0 {
        return Err(DataError::Http("range length must be non-zero".into()));
    }
    let end = offset
        .checked_add(len - 1)
        .ok_or_else(|| DataError::Http("range end overflow".into()))?;
    let wanted = usize::try_from(len)
        .map_err(|_| DataError::Http("range length does not fit memory".into()))?;
    Ok((end, wanted))
}

fn validate_content_range(value: &str, offset: u64, end: u64) -> Result<(), DataError> {
    let (unit, value) = value
        .trim()
        .split_once(' ')
        .ok_or_else(|| DataError::Http(format!("invalid Content-Range {value:?}")))?;
    if !unit.eq_ignore_ascii_case("bytes") {
        return Err(DataError::Http(format!(
            "invalid Content-Range unit {unit:?}"
        )));
    }
    let (range, total) = value
        .split_once('/')
        .ok_or_else(|| DataError::Http(format!("invalid Content-Range {value:?}")))?;
    let (got_start, got_end) = range
        .split_once('-')
        .ok_or_else(|| DataError::Http(format!("invalid Content-Range {value:?}")))?;
    let got_start = got_start
        .parse::<u64>()
        .map_err(|_| DataError::Http(format!("invalid Content-Range {value:?}")))?;
    let got_end = got_end
        .parse::<u64>()
        .map_err(|_| DataError::Http(format!("invalid Content-Range {value:?}")))?;
    if (got_start, got_end) != (offset, end) {
        return Err(DataError::Http(format!(
            "range response identified bytes {got_start}-{got_end}, wanted {offset}-{end}"
        )));
    }
    if total != "*" {
        let total = total
            .parse::<u64>()
            .map_err(|_| DataError::Http(format!("invalid Content-Range {value:?}")))?;
        if total <= got_end {
            return Err(DataError::Http(format!(
                "invalid Content-Range total {total} for byte {got_end}"
            )));
        }
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
async fn fetch_http_range_attempt(
    client: &Client,
    url: &str,
    offset: u64,
    end: u64,
    wanted: usize,
    stall: Option<Duration>,
) -> Result<Vec<u8>, AttemptError> {
    let stalled =
        || AttemptError::retryable(format!("no data for {:?}", stall.unwrap_or_default()));
    let fatal = |message: String| AttemptError {
        error: DataError::Http(message),
        retryable: false,
    };
    let request = client
        .get(url)
        .header(reqwest::header::RANGE, format!("bytes={offset}-{end}"))
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .send();
    let mut response = within(stall, request)
        .await
        .ok_or_else(stalled)?
        .map_err(AttemptError::retryable)?;
    let status = response.status();
    if status != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(AttemptError {
            error: DataError::Http(format!(
                "range request got {status} (server ignores Range?)"
            )),
            retryable: status.is_server_error(),
        });
    }
    let content_range = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .ok_or_else(|| fatal("range response omitted Content-Range".into()))?
        .to_str()
        .map_err(|_| fatal("range response has invalid Content-Range".into()))?;
    validate_content_range(content_range, offset, end).map_err(|error| AttemptError {
        error,
        retryable: false,
    })?;

    let mut bytes = Vec::with_capacity(wanted);
    while let Some(chunk) = within(stall, response.chunk())
        .await
        .ok_or_else(stalled)?
        .map_err(AttemptError::retryable)?
    {
        let remaining = wanted.saturating_sub(bytes.len());
        if chunk.len() > remaining {
            return Err(fatal(format!("range returned more than {wanted} bytes")));
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.len() != wanted {
        // A body that ends early is a dropped connection, not a bad server.
        return Err(AttemptError::retryable(format!(
            "range returned {} bytes, wanted {wanted}",
            bytes.len()
        )));
    }
    Ok(bytes)
}

async fn fetch_http_range(
    client: Client,
    url: Arc<str>,
    offset: u64,
    end: u64,
    wanted: usize,
    stall_retry: Option<StallRetry>,
) -> Result<Vec<u8>, DataError> {
    let host = metrics::http_acquisition("range", &url);
    let request = host.enqueue().start();
    let attempts = stall_retry.map_or(1, |retry| retry.attempts.max(1));
    let mut attempt = 1;
    let result = loop {
        let stall = stall_retry
            .filter(|_| attempt < attempts)
            .map(|retry| retry.stall);
        match fetch_http_range_attempt(&client, &url, offset, end, wanted, stall).await {
            Ok(bytes) => break Ok(bytes),
            Err(failure) if failure.retryable && attempt < attempts => {
                log::debug!(
                    "retrying range {offset}-{end} of {url} after attempt {attempt}: {}",
                    failure.error
                );
                attempt += 1;
            }
            Err(failure) => break Err(failure.error),
        }
    };
    match result {
        Ok(bytes) => {
            request.success(bytes.len());
            metrics::record(bytes.len());
            Ok(bytes)
        }
        Err(error) => {
            request.error();
            metrics::record_err();
            Err(error)
        }
    }
}

#[derive(Clone)]
enum TileBackend {
    Reactor {
        client: Client,
        admission: Arc<ProviderAdmission>,
        stall_retry: Option<StallRetry>,
    },
    Blocking(Arc<dyn TileFetcher>),
}

/// Reactor-native whole-body HTTP transport with a 20-second default timeout.
///
/// The returned future can be polled by any executor (or manually): the actual
/// socket work always runs on vearth's dedicated Tokio runtime.
#[derive(Clone)]
pub struct AsyncHttpFetcher {
    backend: TileBackend,
    series: metrics::Acquisition,
}

impl AsyncHttpFetcher {
    pub fn new() -> Self {
        Self::with_timeout(TILE_TIMEOUT)
    }

    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            backend: TileBackend::Reactor {
                client: http_client(timeout),
                // Preserve the original constructors' behavior: their local
                // bound equals the process-wide ceiling. Provider wiring can
                // opt into a deliberately smaller budget below.
                admission: default_transport_admission(),
                stall_retry: None,
            },
            series: metrics::acquisition("async.tile.reactor"),
        }
    }

    /// Reactor-native transport with a stable provider label and an explicit
    /// clone-shared quota granted jointly with the process-wide ceiling.
    ///
    /// Construct one value per provider and clone it for that provider's
    /// dependency requests. `label` should be stable and low-cardinality (for
    /// example `"mapterhorn"`), because it also names the metric series.
    pub fn for_provider(label: impl Into<String>, max_in_flight: usize) -> Self {
        Self::for_provider_with_timeout(label, max_in_flight, TILE_TIMEOUT)
    }

    /// [`Self::for_provider`] with a custom whole-request timeout.
    pub fn for_provider_with_timeout(
        label: impl Into<String>,
        max_in_flight: usize,
        timeout: Duration,
    ) -> Self {
        Self {
            backend: TileBackend::Reactor {
                client: http_client(timeout),
                admission: bounded_transport_admission(max_in_flight),
                stall_retry: None,
            },
            series: provider_series("tile", label),
        }
    }

    /// Reissue stalled requests. Only for idempotent static objects: a
    /// server that computes each response (a rendering map service) would repeat
    /// that work for every retry.
    pub fn with_stall_retry(mut self, retry: StallRetry) -> Self {
        if let TileBackend::Reactor { stall_retry, .. } = &mut self.backend {
            *stall_retry = Some(retry);
        }
        self
    }

    /// Adapt a blocking byte transport on a separate fixed-size I/O pool.
    ///
    /// This migration/test path is not reactor-native. Production async HTTP
    /// should use Self::new or Self::with_timeout.
    pub fn from_fetcher(inner: Arc<dyn TileFetcher>) -> Self {
        Self {
            backend: TileBackend::Blocking(inner),
            series: metrics::acquisition("async.tile.blocking-pool"),
        }
    }
}

impl Default for AsyncHttpFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl AsyncTileFetcher for AsyncHttpFetcher {
    fn fetch<'a>(&'a self, url: &'a str) -> FetchFuture<'a> {
        self.fetch_with_priority(url, RequestPriority::default())
    }

    fn fetch_with_priority<'a>(
        &'a self,
        url: &'a str,
        priority: RequestPriority,
    ) -> FetchFuture<'a> {
        let url = url.to_owned();
        match &self.backend {
            TileBackend::Reactor {
                client,
                admission,
                stall_retry,
            } => Box::pin(spawn_reactor(
                self.series.clone(),
                admission.clone(),
                priority,
                {
                    let client = client.clone();
                    let stall_retry = *stall_retry;
                    move || fetch_http_body(client, url, stall_retry)
                },
            )),
            TileBackend::Blocking(inner) => {
                let inner = inner.clone();
                Box::pin(spawn_blocking_fetch(self.series.clone(), move || {
                    inner.fetch(&url)
                }))
            }
        }
    }
}

#[derive(Clone)]
enum RangeBackend {
    Reactor {
        client: Client,
        url: Arc<str>,
        admission: Arc<ProviderAdmission>,
        stall_retry: Option<StallRetry>,
    },
    Blocking(Arc<dyn RangeFetcher>),
}

/// Reactor-native PMTiles range transport with a 30-second default timeout.
///
/// It requires HTTP 206, an exact matching Content-Range, and exactly the
/// requested byte count. A server returning the wrong range or a short or
/// oversized body is rejected.
#[derive(Clone)]
pub struct AsyncHttpRange {
    backend: RangeBackend,
    series: metrics::Acquisition,
}

impl AsyncHttpRange {
    pub fn new(url: impl Into<String>) -> Self {
        Self::with_timeout(url, RANGE_TIMEOUT)
    }

    pub fn with_timeout(url: impl Into<String>, timeout: Duration) -> Self {
        Self {
            backend: RangeBackend::Reactor {
                client: http_client(timeout),
                url: Arc::from(url.into()),
                admission: default_transport_admission(),
                stall_retry: None,
            },
            series: metrics::acquisition("async.range.reactor"),
        }
    }

    /// Reactor-native range transport with a stable provider label and an
    /// explicit clone-shared quota granted jointly with the process ceiling.
    pub fn for_provider(
        url: impl Into<String>,
        label: impl Into<String>,
        max_in_flight: usize,
    ) -> Self {
        Self::for_provider_with_timeout(url, label, max_in_flight, RANGE_TIMEOUT)
    }

    /// [`Self::for_provider`] with a custom whole-request timeout.
    pub fn for_provider_with_timeout(
        url: impl Into<String>,
        label: impl Into<String>,
        max_in_flight: usize,
        timeout: Duration,
    ) -> Self {
        Self {
            backend: RangeBackend::Reactor {
                client: http_client(timeout),
                url: Arc::from(url.into()),
                admission: bounded_transport_admission(max_in_flight),
                stall_retry: None,
            },
            series: provider_series("range", label),
        }
    }

    /// Reissue stalled range reads. Byte ranges of a static archive are
    /// idempotent; see [`StallRetry`].
    pub fn with_stall_retry(mut self, retry: StallRetry) -> Self {
        if let RangeBackend::Reactor { stall_retry, .. } = &mut self.backend {
            *stall_retry = Some(retry);
        }
        self
    }

    /// Adapt a blocking range transport on a separate fixed-size I/O pool.
    pub fn from_fetcher(inner: Arc<dyn RangeFetcher>) -> Self {
        Self {
            backend: RangeBackend::Blocking(inner),
            series: metrics::acquisition("async.range.blocking-pool"),
        }
    }
}

impl AsyncRangeFetcher for AsyncHttpRange {
    fn fetch_range(&self, offset: u64, len: u64) -> FetchFuture<'_> {
        self.fetch_range_with_priority(offset, len, RequestPriority::default())
    }

    fn fetch_range_with_priority(
        &self,
        offset: u64,
        len: u64,
        priority: RequestPriority,
    ) -> FetchFuture<'_> {
        let (end, wanted) = match checked_range(offset, len) {
            Ok(range) => range,
            Err(error) => return Box::pin(std::future::ready(Err(error))),
        };
        match &self.backend {
            RangeBackend::Reactor {
                client,
                url,
                admission,
                stall_retry,
            } if crate::vectors::is_block_archive(url) => {
                let admission = BlockAdmission {
                    global: reactor().admission.clone(),
                    provider: admission.clone(),
                    priority,
                };
                let client = client.clone();
                let url = url.clone();
                let stall_retry = *stall_retry;
                Box::pin(spawn_reactor_self_admitted(
                    self.series.clone(),
                    move || {
                        fetch_archive_range(
                            client,
                            url,
                            offset,
                            end,
                            wanted,
                            stall_retry,
                            admission,
                        )
                    },
                ))
            }
            RangeBackend::Reactor {
                client,
                url,
                admission,
                stall_retry,
            } => Box::pin(spawn_reactor(
                self.series.clone(),
                admission.clone(),
                priority,
                {
                    let client = client.clone();
                    let url = url.clone();
                    let stall_retry = *stall_retry;
                    move || fetch_http_range(client, url, offset, end, wanted, stall_retry)
                },
            )),
            RangeBackend::Blocking(inner) => {
                let inner = inner.clone();
                Box::pin(spawn_blocking_fetch(self.series.clone(), move || {
                    inner.fetch_range(offset, len)
                }))
            }
        }
    }
}

// Blocking adapters below are deliberately isolated from the reactor. They
// exist only for custom fetchers and staged migration.

type BlockingJob = Box<dyn FnOnce() + Send + 'static>;

#[derive(Default)]
struct BlockingQueue {
    jobs: Mutex<VecDeque<BlockingJob>>,
    ready: Condvar,
}

struct BlockingPool {
    queue: Arc<BlockingQueue>,
}

impl BlockingPool {
    fn new() -> Self {
        let queue = Arc::new(BlockingQueue::default());
        let workers = vearth_platform::thread::available_parallelism()
            .map(|n| n.get() * 2)
            .unwrap_or(8)
            .clamp(8, 32);
        for i in 0..workers {
            let queue = queue.clone();
            vearth_platform::thread::Builder::new()
                .name(format!("vearth-blocking-io-{i}"))
                .spawn(move || {
                    loop {
                        let job = {
                            let mut jobs = queue.jobs.lock().unwrap();
                            while jobs.is_empty() {
                                jobs = queue.ready.wait(jobs).unwrap();
                            }
                            jobs.pop_front().unwrap()
                        };
                        job();
                    }
                })
                .expect("blocking I/O adapter worker");
        }
        Self { queue }
    }

    fn submit(&self, job: BlockingJob) {
        self.queue.jobs.lock().unwrap().push_back(job);
        self.queue.ready.notify_one();
    }
}

fn blocking_pool() -> &'static BlockingPool {
    static POOL: OnceLock<BlockingPool> = OnceLock::new();
    POOL.get_or_init(BlockingPool::new)
}

struct BlockingTaskState {
    result: Option<Result<Vec<u8>, DataError>>,
    waker: Option<Waker>,
    abandoned: bool,
}

struct BlockingTaskFuture {
    state: Arc<Mutex<BlockingTaskState>>,
}

impl Future for BlockingTaskFuture {
    type Output = Result<Vec<u8>, DataError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.state.lock().unwrap();
        if let Some(result) = state.result.take() {
            Poll::Ready(result)
        } else {
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

impl Drop for BlockingTaskFuture {
    fn drop(&mut self) {
        self.state.lock().unwrap().abandoned = true;
    }
}

fn spawn_blocking_fetch(
    series: metrics::Acquisition,
    fetch: impl FnOnce() -> Result<Vec<u8>, DataError> + Send + 'static,
) -> BlockingTaskFuture {
    let state = Arc::new(Mutex::new(BlockingTaskState {
        result: None,
        waker: None,
        abandoned: false,
    }));
    let completed = state.clone();
    let queued = series.enqueue();
    blocking_pool().submit(Box::new(move || {
        if completed.lock().unwrap().abandoned {
            return;
        }
        let in_flight = queued.start();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(fetch))
            .unwrap_or_else(|_| Err(DataError::Http("blocking I/O adapter panicked".into())));
        let success_bytes = result.as_ref().ok().map(Vec::len);
        let not_found = matches!(&result, Err(DataError::NotAvailable));
        let (abandoned, waker) = {
            let mut state = completed.lock().unwrap();
            if state.abandoned {
                (true, None)
            } else {
                state.result = Some(result);
                (false, state.waker.take())
            }
        };
        if abandoned {
            // Running blocking I/O cannot be interrupted safely, but its
            // result is discarded and the adapter outcome is cancellation.
            drop(in_flight);
            return;
        }
        match success_bytes {
            Some(bytes) => in_flight.success(bytes),
            None if not_found => in_flight.not_found(),
            None => in_flight.error(),
        }
        if let Some(waker) = waker {
            waker.wake();
        }
    }));
    BlockingTaskFuture { state }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pmtiles::HttpRange;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::task::{Wake, Waker};

    struct ThreadWake(vearth_platform::thread::Thread);

    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        let waker = Waker::from(Arc::new(ThreadWake(vearth_platform::thread::current())));
        let mut context = Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);
        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(output) => return output,
                Poll::Pending => vearth_platform::thread::park_timeout(Duration::from_secs(2)),
            }
        }
    }

    fn poll_once<F: Future + Unpin>(future: &mut F) -> Poll<F::Output> {
        let waker = Waker::from(Arc::new(ThreadWake(vearth_platform::thread::current())));
        let mut context = Context::from_waker(&waker);
        Pin::new(future).poll(&mut context)
    }

    #[test]
    fn admission_orders_by_live_priority_then_fifo() {
        let admission = Arc::new(PriorityAdmission::new(1));
        let provider = Arc::new(ProviderAdmission::new(1));
        let holder = block_on(admission.acquire(provider.clone(), RequestPriority::new(0.0)));
        let mut low = admission.acquire(provider.clone(), RequestPriority::new(1.0));
        let mut high = admission.acquire(provider.clone(), RequestPriority::new(10.0));
        assert!(poll_once(&mut low).is_pending());
        assert!(poll_once(&mut high).is_pending());
        drop(holder);

        let high_permit = match poll_once(&mut high) {
            Poll::Ready(permit) => permit,
            Poll::Pending => panic!("higher priority request was not admitted first"),
        };
        assert!(poll_once(&mut low).is_pending());
        drop(high_permit);
        let low_permit = match poll_once(&mut low) {
            Poll::Ready(permit) => permit,
            Poll::Pending => panic!("lower priority request never inherited released permit"),
        };
        drop(low_permit);
        assert_eq!(admission.queued_len(), 0);

        // Equal priorities remain FIFO by stable admission sequence.
        let holder = block_on(admission.acquire(provider.clone(), RequestPriority::new(0.0)));
        let mut first = admission.acquire(provider.clone(), RequestPriority::new(4.0));
        let mut second = admission.acquire(provider.clone(), RequestPriority::new(4.0));
        assert!(poll_once(&mut first).is_pending());
        assert!(poll_once(&mut second).is_pending());
        drop(holder);
        let first_permit = match poll_once(&mut first) {
            Poll::Ready(permit) => permit,
            Poll::Pending => panic!("equal-priority FIFO order was not preserved"),
        };
        assert!(poll_once(&mut second).is_pending());
        drop(first_permit);
        drop(block_on(second));
    }

    #[test]
    fn admission_reloads_shared_max_active_waiter_priority() {
        let admission = Arc::new(PriorityAdmission::new(1));
        let provider = Arc::new(ProviderAdmission::new(1));
        let holder = block_on(admission.acquire(provider.clone(), RequestPriority::default()));

        let shared = RequestPriority::shared();
        let _low_waiter = shared.lease(1.0);
        let mut inherited = admission.acquire(provider.clone(), shared.clone());
        let mut medium = admission.acquire(provider.clone(), RequestPriority::new(5.0));
        assert!(poll_once(&mut inherited).is_pending());
        assert!(poll_once(&mut medium).is_pending());
        let high_waiter = shared.lease(10.0);
        assert_eq!(shared.current(), 10.0);
        drop(holder);
        let inherited_permit = match poll_once(&mut inherited) {
            Poll::Ready(permit) => permit,
            Poll::Pending => panic!("late shared waiter did not raise queued admission"),
        };
        assert!(poll_once(&mut medium).is_pending());
        drop(inherited_permit);
        drop(block_on(medium));
        drop(high_waiter);

        // A canceled high waiter is removed from the active maximum.
        let holder = block_on(admission.acquire(provider.clone(), RequestPriority::default()));
        let shared = RequestPriority::shared();
        let _low_waiter = shared.lease(1.0);
        let high_waiter = shared.lease(10.0);
        let mut inherited = admission.acquire(provider.clone(), shared.clone());
        let mut medium = admission.acquire(provider.clone(), RequestPriority::new(5.0));
        assert!(poll_once(&mut inherited).is_pending());
        assert!(poll_once(&mut medium).is_pending());
        drop(high_waiter);
        assert_eq!(shared.current(), 1.0);
        drop(holder);
        let medium_permit = match poll_once(&mut medium) {
            Poll::Ready(permit) => permit,
            Poll::Pending => panic!("canceled waiter continued inflating shared priority"),
        };
        assert!(poll_once(&mut inherited).is_pending());
        drop(medium_permit);
        drop(block_on(inherited));
    }

    #[test]
    fn admission_cancellation_removes_queued_and_granted_waiters() {
        let admission = Arc::new(PriorityAdmission::new(1));
        let provider = Arc::new(ProviderAdmission::new(1));
        let holder = block_on(admission.acquire(provider.clone(), RequestPriority::default()));
        let mut high = admission.acquire(provider.clone(), RequestPriority::new(10.0));
        let mut low = admission.acquire(provider.clone(), RequestPriority::new(1.0));
        assert!(poll_once(&mut high).is_pending());
        assert!(poll_once(&mut low).is_pending());
        drop(high);
        assert_eq!(admission.queued_len(), 1);
        drop(holder);
        drop(block_on(low));
        assert_eq!(admission.queued_len(), 0);

        // Cancellation after dispatch but before the future consumes its
        // permit must immediately grant the next queued request.
        let holder = block_on(admission.acquire(provider.clone(), RequestPriority::default()));
        let mut high = admission.acquire(provider.clone(), RequestPriority::new(10.0));
        let mut low = admission.acquire(provider.clone(), RequestPriority::new(1.0));
        assert!(poll_once(&mut high).is_pending());
        assert!(poll_once(&mut low).is_pending());
        drop(holder); // high is now Granted but deliberately unpolled
        drop(high);
        drop(block_on(low));
        assert_eq!(admission.queued_len(), 0);
    }

    #[test]
    fn global_admission_overlaps_provider_quotas_and_reclaims_canceled_grants() {
        let reactor = HttpReactor::new();
        let admission = reactor.admission.clone();
        assert_eq!(
            admission.state.lock().unwrap().capacity,
            MAX_GLOBAL_REQUESTS
        );
        let terrain_limit = crate::terrarium::ASYNC_TILE_LIMIT;
        let terrain = Arc::new(ProviderAdmission::new(terrain_limit));
        let vectors = Arc::new(ProviderAdmission::new(16));
        let other = Arc::new(ProviderAdmission::new(MAX_GLOBAL_REQUESTS));
        let mut terrain_permits: Vec<_> = (0..terrain_limit)
            .map(|_| block_on(admission.acquire(terrain.clone(), RequestPriority::default())))
            .collect();
        let mut terrain_tail = admission.acquire(terrain.clone(), RequestPriority::new(30.0));
        assert!(poll_once(&mut terrain_tail).is_pending());

        // A saturated source must leave the remaining global capacity
        // available to other providers' independent quotas.
        let mut vector_permits: Vec<_> = (0..16)
            .map(|_| block_on(admission.acquire(vectors.clone(), RequestPriority::default())))
            .collect();
        let mut vector_tail = admission.acquire(vectors.clone(), RequestPriority::new(20.0));
        assert!(poll_once(&mut vector_tail).is_pending());
        let other_permits: Vec<_> = (0..MAX_GLOBAL_REQUESTS - (terrain_limit + 16))
            .map(|_| block_on(admission.acquire(other.clone(), RequestPriority::default())))
            .collect();
        let mut other_tail = admission.acquire(other.clone(), RequestPriority::new(10.0));
        assert!(poll_once(&mut other_tail).is_pending());
        assert_eq!(admission.state.lock().unwrap().available, 0);
        assert_eq!(admission.queued_len(), 3);

        drop(terrain_tail);
        drop(terrain_permits.pop());
        assert_eq!(admission.queued_len(), 1);
        assert_eq!(admission.state.lock().unwrap().available, 0);
        // The other provider now owns an unpolled grant. Canceling it must
        // release both budgets without admitting the still-full vector quota.
        drop(other_tail);
        assert_eq!(admission.state.lock().unwrap().available, 1);
        assert!(poll_once(&mut vector_tail).is_pending());
        drop(vector_permits.pop());
        let vector_permit = match poll_once(&mut vector_tail) {
            Poll::Ready(permit) => permit,
            Poll::Pending => panic!("released vector quota did not admit its queued request"),
        };
        drop(vector_permit);
        drop(vector_permits);
        drop(terrain_permits);
        drop(other_permits);
        assert_eq!(admission.queued_len(), 0);
        assert_eq!(
            admission.state.lock().unwrap().available,
            MAX_GLOBAL_REQUESTS
        );
        for provider in [terrain, vectors, other] {
            assert_eq!(
                provider.available.load(AtomicOrdering::Acquire),
                provider.capacity
            );
        }
    }

    struct LegacyAsyncFake;

    impl AsyncTileFetcher for LegacyAsyncFake {
        fn fetch<'a>(&'a self, _url: &'a str) -> FetchFuture<'a> {
            Box::pin(std::future::ready(Ok(b"legacy-tile".to_vec())))
        }
    }

    impl AsyncRangeFetcher for LegacyAsyncFake {
        fn fetch_range(&self, _offset: u64, _len: u64) -> FetchFuture<'_> {
            Box::pin(std::future::ready(Ok(b"legacy-range".to_vec())))
        }
    }

    #[test]
    fn priority_trait_defaults_preserve_legacy_fake_implementations() {
        let fake = LegacyAsyncFake;
        assert_eq!(
            block_on(fake.fetch_with_priority("unused", RequestPriority::new(8.0))).unwrap(),
            b"legacy-tile"
        );
        assert_eq!(
            block_on(fake.fetch_range_with_priority(0, 4, RequestPriority::new(8.0))).unwrap(),
            b"legacy-range"
        );
    }

    fn read_request(stream: &mut TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        let mut buf = [0u8; 1024];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            request.extend_from_slice(&buf[..n]);
        }
        String::from_utf8_lossy(&request).into_owned()
    }

    fn serve_once(response: Vec<u8>, delay: Duration) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        vearth_platform::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = tx.send(read_request(&mut stream));
            vearth_platform::thread::sleep(delay);
            let _ = stream.write_all(&response);
        });
        (format!("http://{address}/data"), rx)
    }

    /// Serve two connections concurrently, holding the first response until
    /// `release` fires. The request-index channel makes transport admission
    /// observable without relying on timing inside the reactor.
    fn serve_two_with_first_gated(
        response: Vec<u8>,
    ) -> (String, mpsc::Receiver<usize>, mpsc::Sender<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (seen_tx, seen_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        vearth_platform::thread::spawn(move || {
            let mut first_release = Some(release_rx);
            for index in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let response = response.clone();
                let seen = seen_tx.clone();
                let release = if index == 0 {
                    first_release.take()
                } else {
                    None
                };
                vearth_platform::thread::spawn(move || {
                    let _ = read_request(&mut stream);
                    let _ = seen.send(index);
                    if let Some(release) = release {
                        let _ = release.recv();
                    }
                    let _ = stream.write_all(&response);
                });
            }
        });
        (format!("http://{address}/data"), seen_rx, release_tx)
    }

    /// Serve one connection per entry in order; `None` accepts the request
    /// and then never answers. Returns the number of requests received.
    fn serve_sequence(
        responses: Vec<Option<Vec<u8>>>,
    ) -> (String, Arc<std::sync::atomic::AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let seen = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = seen.clone();
        vearth_platform::thread::spawn(move || {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let _ = read_request(&mut stream);
                counter.fetch_add(1, AtomicOrdering::SeqCst);
                match response {
                    Some(response) => {
                        let _ = stream.write_all(&response);
                    }
                    None => {
                        vearth_platform::thread::spawn(move || {
                            vearth_platform::thread::sleep(Duration::from_secs(5));
                            drop(stream);
                        });
                    }
                }
            }
        });
        (format!("http://{address}/data"), seen)
    }

    #[test]
    fn stall_retry_reissues_a_hung_request_but_not_a_client_error() {
        let retry = StallRetry {
            stall: Duration::from_millis(150),
            attempts: 3,
        };
        let fetcher =
            AsyncHttpFetcher::for_provider_with_timeout("stall-test", 4, Duration::from_secs(4))
                .with_stall_retry(retry);

        let (url, seen) = serve_sequence(vec![None, Some(response("200 OK", b"tile"))]);
        let started = vearth_platform::time::Instant::now();
        assert_eq!(block_on(fetcher.fetch(&url)).unwrap(), b"tile");
        assert_eq!(seen.load(AtomicOrdering::SeqCst), 2);
        assert!(started.elapsed() < Duration::from_secs(2));

        let (url, seen) = serve_sequence(vec![
            Some(response("404 Not Found", b"")),
            Some(response("200 OK", b"tile")),
        ]);
        assert!(matches!(
            block_on(fetcher.fetch(&url)),
            Err(DataError::NotAvailable)
        ));
        assert_eq!(seen.load(AtomicOrdering::SeqCst), 1);

        // The final attempt has no stall deadline: a slow but eventual answer
        // is accepted instead of failing the tile.
        let (url, _) = serve_once(response("200 OK", b"slow"), Duration::from_millis(400));
        let single = AsyncHttpFetcher::for_provider_with_timeout(
            "stall-test-final",
            4,
            Duration::from_secs(4),
        )
        .with_stall_retry(StallRetry {
            stall: Duration::from_millis(150),
            attempts: 1,
        });
        assert_eq!(block_on(single.fetch(&url)).unwrap(), b"slow");
    }

    #[test]
    fn range_stall_retry_reissues_a_hung_read_but_not_a_bad_range() {
        let retry = StallRetry {
            stall: Duration::from_millis(150),
            attempts: 3,
        };
        let partial = |content_range| range_response("206 Partial Content", content_range, b"abcd");
        let fetcher = |url: String| {
            AsyncHttpRange::for_provider_with_timeout(
                url,
                "range-stall-test",
                4,
                Duration::from_secs(4),
            )
            .with_stall_retry(retry)
        };

        let (url, seen) = serve_sequence(vec![None, Some(partial("bytes 2-5/10"))]);
        assert_eq!(block_on(fetcher(url).fetch_range(2, 4)).unwrap(), b"abcd");
        assert_eq!(seen.load(AtomicOrdering::SeqCst), 2);

        let (url, seen) = serve_sequence(vec![
            Some(partial("bytes 0-3/10")),
            Some(partial("bytes 2-5/10")),
        ]);
        assert!(matches!(
            block_on(fetcher(url).fetch_range(2, 4)),
            Err(DataError::Http(_))
        ));
        assert_eq!(seen.load(AtomicOrdering::SeqCst), 1);
    }

    fn response_with_headers(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
        let mut response =
            format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n", body.len()).into_bytes();
        for (name, value) in headers {
            response.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
        }
        response.extend_from_slice(b"Connection: close\r\n\r\n");
        response.extend_from_slice(body);
        response
    }

    fn response(status: &str, body: &[u8]) -> Vec<u8> {
        response_with_headers(status, &[], body)
    }

    fn range_response(status: &str, content_range: &str, body: &[u8]) -> Vec<u8> {
        response_with_headers(status, &[("Content-Range", content_range)], body)
    }

    fn assert_http_result_parity(
        blocking: Result<Vec<u8>, DataError>,
        asynchronous: Result<Vec<u8>, DataError>,
    ) {
        match (blocking, asynchronous) {
            (Ok(blocking), Ok(asynchronous)) => assert_eq!(asynchronous, blocking),
            (Err(DataError::Http(_)), Err(DataError::Http(_)))
            | (Err(DataError::NotAvailable), Err(DataError::NotAvailable)) => {}
            (blocking, asynchronous) => {
                panic!("blocking/async result mismatch: {blocking:?} vs {asynchronous:?}")
            }
        }
    }

    fn wait_for_counter(series: &metrics::Acquisition, before: u64) {
        let deadline = vearth_platform::time::Instant::now() + Duration::from_secs(1);
        while series.snapshot().canceled == before {
            assert!(vearth_platform::time::Instant::now() < deadline);
            vearth_platform::thread::yield_now();
        }
    }

    #[test]
    fn tile_transport_reads_body_and_rejects_error_status() {
        let (url, _) = serve_once(response("200 OK", b"tile"), Duration::ZERO);
        let fetcher = AsyncHttpFetcher::with_timeout(Duration::from_secs(1));
        assert_eq!(block_on(fetcher.fetch(&url)).unwrap(), b"tile");

        // Blocking ureq rejects a redirect response without Location; the
        // async transport must expose the same failure shape.
        let (url, _) = serve_once(response("302 Found", b"redirectless"), Duration::ZERO);
        assert!(matches!(
            block_on(fetcher.fetch(&url)),
            Err(DataError::Http(_))
        ));

        let (url, _) = serve_once(
            response("503 Service Unavailable", b"later"),
            Duration::ZERO,
        );
        assert!(matches!(
            block_on(fetcher.fetch(&url)),
            Err(DataError::Http(_))
        ));
    }

    #[test]
    fn tile_transport_matches_blocking_bytes_and_error_shape() {
        for wire in [
            response("200 OK", b"tile bytes"),
            response("302 Found", b"redirectless"),
            response("503 Service Unavailable", b"later"),
        ] {
            let (url, _) = serve_once(wire.clone(), Duration::ZERO);
            let blocking = crate::HttpFetcher::with_timeout(Duration::from_secs(1)).fetch(&url);
            let (url, _) = serve_once(wire, Duration::ZERO);
            let asynchronous =
                block_on(AsyncHttpFetcher::with_timeout(Duration::from_secs(1)).fetch(&url));
            assert_http_result_parity(blocking, asynchronous);
        }
    }

    #[test]
    fn hosted_reads_assemble_ranges_from_one_cached_download_per_block() {
        // The server accepts one connection, so later reads must be served
        // from the cached block. It is the build's last, short block.
        let (url, request) = serve_once(response("200 OK", b"0123456789"), Duration::ZERO);
        let client = http_client(Duration::from_secs(1));
        let admission = BlockAdmission {
            global: Arc::new(PriorityAdmission::new(4)),
            provider: Arc::new(ProviderAdmission::new(4)),
            priority: RequestPriority::default(),
        };
        let read = |offset, end, wanted| {
            reactor().runtime.block_on(blocks::read(
                &client, &url, offset, end, wanted, None, &admission,
            ))
        };
        assert_eq!(read(2, 5, 4).map_err(|e| e.error).unwrap(), b"2345");
        assert_eq!(read(7, 9, 3).map_err(|e| e.error).unwrap(), b"789");
        assert!(read(8, 12, 5).is_err_and(|e| !e.retryable));
        let request = request.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(request.starts_with("GET /data/0.bin "), "{request}");
    }

    #[test]
    fn range_transport_requires_206_exact_length_and_header() {
        let (url, request) = serve_once(
            range_response("206 Partial Content", "bytes 2-5/100", b"cdef"),
            Duration::ZERO,
        );
        let fetcher = AsyncHttpRange::with_timeout(url, Duration::from_secs(1));
        assert_eq!(block_on(fetcher.fetch_range(2, 4)).unwrap(), b"cdef");
        let request = request.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            request
                .to_ascii_lowercase()
                .contains("range: bytes=2-5\r\n")
        );

        let (url, _) = serve_once(response("200 OK", b"cdef"), Duration::ZERO);
        let fetcher = AsyncHttpRange::with_timeout(url, Duration::from_secs(1));
        assert!(matches!(
            block_on(fetcher.fetch_range(2, 4)),
            Err(DataError::Http(_))
        ));

        for body in [&b"bad"[..], &b"extra"[..]] {
            let (url, _) = serve_once(
                range_response("206 Partial Content", "bytes 2-5/100", body),
                Duration::ZERO,
            );
            let fetcher = AsyncHttpRange::with_timeout(url, Duration::from_secs(1));
            assert!(matches!(
                block_on(fetcher.fetch_range(2, 4)),
                Err(DataError::Http(_))
            ));
        }
    }

    #[test]
    fn range_transport_matches_blocking_bytes_and_error_shape() {
        for wire in [
            range_response("206 Partial Content", "bytes 2-5/100", b"cdef"),
            response("200 OK", b"cdef"),
            range_response("206 Partial Content", "bytes 2-5/100", b"bad"),
            range_response("206 Partial Content", "bytes 2-5/100", b"extra"),
        ] {
            let (url, _) = serve_once(wire.clone(), Duration::ZERO);
            let blocking = HttpRange::with_timeout(url, Duration::from_secs(1)).fetch_range(2, 4);
            let (url, _) = serve_once(wire, Duration::ZERO);
            let asynchronous = block_on(
                AsyncHttpRange::with_timeout(url, Duration::from_secs(1)).fetch_range(2, 4),
            );
            assert_http_result_parity(blocking, asynchronous);
        }
    }

    #[test]
    fn range_transport_rejects_missing_or_mismatched_content_range() {
        for wire in [
            response("206 Partial Content", b"cdef"),
            range_response("206 Partial Content", "bytes 3-6/100", b"cdef"),
            range_response("206 Partial Content", "bytes 2-5/5", b"cdef"),
            range_response("206 Partial Content", "items 2-5/100", b"cdef"),
        ] {
            let (url, _) = serve_once(wire, Duration::ZERO);
            let fetcher = AsyncHttpRange::with_timeout(url, Duration::from_secs(1));
            assert!(matches!(
                block_on(fetcher.fetch_range(2, 4)),
                Err(DataError::Http(_))
            ));
        }
    }

    #[test]
    fn range_transport_rejects_invalid_ranges_without_network() {
        let fetcher = AsyncHttpRange::new("http://unused.test/archive.pmtiles");
        assert!(matches!(
            block_on(fetcher.fetch_range(0, 0)),
            Err(DataError::Http(_))
        ));
        assert!(matches!(
            block_on(fetcher.fetch_range(u64::MAX, 2)),
            Err(DataError::Http(_))
        ));
    }

    #[test]
    fn configured_timeout_bounds_the_whole_request() {
        let (url, _) = serve_once(response("200 OK", b"late"), Duration::from_millis(200));
        let fetcher = AsyncHttpFetcher::with_timeout(Duration::from_millis(30));
        assert!(matches!(
            block_on(fetcher.fetch(&url)),
            Err(DataError::Http(_))
        ));
    }

    #[test]
    fn dropping_queued_reactor_future_records_cancellation() {
        let series = metrics::acquisition(format!(
            "async-reactor-cancel-test-{}-{:?}",
            std::process::id(),
            vearth_platform::thread::current().id()
        ));
        let before = series.snapshot().canceled;
        let global = Arc::new(PriorityAdmission::new(1));
        let provider = Arc::new(ProviderAdmission::new(1));
        let holder = block_on(global.acquire(provider.clone(), RequestPriority::default()));
        let future = spawn_reactor_with_admission(
            series.clone(),
            global,
            provider,
            RequestPriority::default(),
            || std::future::ready(Ok(b"unused".to_vec())),
        );
        drop(future);
        wait_for_counter(&series, before);
        assert_eq!(series.snapshot().queued, 0);
        drop(holder);
    }

    #[test]
    fn canceling_in_flight_tile_releases_provider_permit() {
        let (url, seen, release) = serve_two_with_first_gated(response("200 OK", b"second"));
        let label = format!(
            "tile-permit-release-{}-{:?}",
            std::process::id(),
            vearth_platform::thread::current().id()
        );
        let fetcher = AsyncHttpFetcher::for_provider_with_timeout(label, 1, Duration::from_secs(2));
        let before = fetcher.series.snapshot().canceled;

        let first = fetcher.fetch(&url);
        assert_eq!(seen.recv_timeout(Duration::from_secs(1)).unwrap(), 0);
        let second_fetcher = fetcher.clone();
        let second = second_fetcher.fetch(&url);
        assert!(matches!(
            seen.recv_timeout(Duration::from_millis(75)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));

        drop(first);
        assert_eq!(seen.recv_timeout(Duration::from_secs(1)).unwrap(), 1);
        release.send(()).unwrap();
        assert_eq!(block_on(second).unwrap(), b"second");
        wait_for_counter(&fetcher.series, before);
    }

    #[test]
    fn canceling_in_flight_range_releases_provider_permit() {
        let (url, seen, release) = serve_two_with_first_gated(range_response(
            "206 Partial Content",
            "bytes 0-3/100",
            b"next",
        ));
        let label = format!(
            "range-permit-release-{}-{:?}",
            std::process::id(),
            vearth_platform::thread::current().id()
        );
        let fetcher =
            AsyncHttpRange::for_provider_with_timeout(url, label, 1, Duration::from_secs(2));
        let before = fetcher.series.snapshot().canceled;

        let first = fetcher.fetch_range(0, 4);
        assert_eq!(seen.recv_timeout(Duration::from_secs(1)).unwrap(), 0);
        let second_fetcher = fetcher.clone();
        let second = second_fetcher.fetch_range(0, 4);
        assert!(matches!(
            seen.recv_timeout(Duration::from_millis(75)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));

        drop(first);
        assert_eq!(seen.recv_timeout(Duration::from_secs(1)).unwrap(), 1);
        release.send(()).unwrap();
        assert_eq!(block_on(second).unwrap(), b"next");
        wait_for_counter(&fetcher.series, before);
    }

    struct GatedFetcher {
        started: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
        returning: mpsc::Sender<()>,
    }

    impl TileFetcher for GatedFetcher {
        fn fetch(&self, _url: &str) -> Result<Vec<u8>, DataError> {
            self.started.send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
            self.returning.send(()).unwrap();
            Ok(b"unused".to_vec())
        }
    }

    #[test]
    fn dropping_blocking_adapter_future_discards_result() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (returning_tx, returning_rx) = mpsc::channel();
        let fetcher = AsyncHttpFetcher::from_fetcher(Arc::new(GatedFetcher {
            started: started_tx,
            release: Mutex::new(release_rx),
            returning: returning_tx,
        }));
        let before = fetcher.series.snapshot().canceled;

        let future = fetcher.fetch("http://unused.test/");
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        drop(future);
        release_tx.send(()).unwrap();
        returning_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        wait_for_counter(&fetcher.series, before);
    }
}
