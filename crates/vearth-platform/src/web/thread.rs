//! `std::thread`-compatible spawning for Web Workers.

use std::cell::RefCell;
use std::io;
use std::marker::PhantomData;
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

pub use std::thread::{Result, Thread, ThreadId, current, park, park_timeout, sleep, yield_now};

/// Stack reserved for each worker's shadow stack in shared memory, matching
/// Rust's default for spawned native threads.
const DEFAULT_STACK_SIZE: usize = 2 << 20;

struct Start {
    name: String,
    main: Box<dyn FnOnce() + Send>,
    persistent: bool,
}

thread_local! {
    static SPAWNER: RefCell<Option<web_sys::MessagePort>> = const { RefCell::new(None) };
    // `std::thread::current().name()` is unset for threads std did not spawn.
    static NAME: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Names the current worker for logs and panic reports.
pub fn set_current_name(name: &str) {
    NAME.with_borrow_mut(|current| *current = Some(name.to_owned()));
}

/// The name given when this worker was spawned or named.
pub fn current_name() -> Option<String> {
    NAME.with_borrow(Clone::clone)
}

/// Connects the current worker to the main thread's worker factory. Called by
/// the thread bootstrap script before any Rust code spawns.
#[wasm_bindgen(js_name = __vearth_attach_spawner)]
pub fn attach_spawner(port: web_sys::MessagePort) {
    SPAWNER.with_borrow_mut(|spawner| *spawner = Some(port));
}

/// Runs a thread's start record. Returns whether the worker must stay alive
/// after its closure returns because it now drives JavaScript event-loop work.
#[wasm_bindgen(js_name = __vearth_run_thread)]
pub fn run_thread(start: u32) -> bool {
    // SAFETY: `start` came from `Box::into_raw` in `spawn_worker` and the main
    // thread delivers each record to exactly one new worker.
    let start = unsafe { Box::from_raw(start as usize as *mut Start) };
    let persistent = start.persistent;
    set_current_name(&start.name);
    (start.main)();
    persistent
}

/// Starts a worker running `main`. A persistent worker keeps its event loop
/// after `main` returns (the async reactor); others close when it returns.
pub(crate) fn spawn_worker(
    name: &str,
    stack_size: usize,
    persistent: bool,
    main: Box<dyn FnOnce() + Send>,
) -> io::Result<()> {
    let start = Box::into_raw(Box::new(Start {
        name: name.to_owned(),
        main,
        persistent,
    }));
    let message = js_sys::Object::new();
    let set = |key: &str, value: JsValue| js_sys::Reflect::set(&message, &key.into(), &value);
    let posted = set("name", name.into())
        .and_then(|_| set("start", (start as usize as u32).into()))
        .and_then(|_| set("stackSize", (stack_size as u32).into()))
        .map_err(|error| format!("{error:?}"))
        .and_then(|_| {
            SPAWNER.with_borrow(|spawner| match spawner {
                Some(port) => port
                    .post_message(&message)
                    .map_err(|error| format!("{error:?}")),
                None => Err("thread has no worker spawner".into()),
            })
        });
    posted.map_err(|error| {
        // SAFETY: the record was not delivered, so this thread still owns it.
        drop(unsafe { Box::from_raw(start) });
        io::Error::other(format!("could not spawn worker {name:?}: {error}"))
    })
}

struct Packet<T> {
    result: Mutex<Option<Result<T>>>,
    done: Condvar,
}

/// Owned permission to join a spawned worker thread.
pub struct JoinHandle<T> {
    packet: Arc<Packet<T>>,
}

impl<T> JoinHandle<T> {
    /// Waits for the thread to finish, returning its panic payload if it panicked.
    pub fn join(self) -> Result<T> {
        let mut result = self.packet.result.lock().unwrap();
        loop {
            if let Some(result) = result.take() {
                return result;
            }
            result = self.packet.done.wait(result).unwrap();
        }
    }

    pub fn is_finished(&self) -> bool {
        self.packet.result.lock().unwrap().is_some()
    }
}

/// Thread factory mirroring `std::thread::Builder`.
#[derive(Debug, Default)]
pub struct Builder {
    name: Option<String>,
    stack_size: Option<usize>,
}

impl Builder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn name(mut self, name: String) -> Self {
        self.name = Some(name);
        self
    }

    pub fn stack_size(mut self, size: usize) -> Self {
        self.stack_size = Some(size);
        self
    }

    pub fn spawn<F, T>(self, f: F) -> io::Result<JoinHandle<T>>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let packet = Arc::new(Packet {
            result: Mutex::new(None),
            done: Condvar::new(),
        });
        let their_packet = packet.clone();
        let main = Box::new(move || {
            let result = catch_unwind(AssertUnwindSafe(f));
            *their_packet.result.lock().unwrap() = Some(result);
            their_packet.done.notify_all();
        });
        spawn_worker(
            self.name.as_deref().unwrap_or("vearth-thread"),
            self.stack_size.unwrap_or(DEFAULT_STACK_SIZE),
            false,
            main,
        )?;
        Ok(JoinHandle { packet })
    }
}

pub fn spawn<F, T>(f: F) -> JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    Builder::new().spawn(f).expect("failed to spawn thread")
}

/// Logical processors reported by the browser.
pub fn available_parallelism() -> io::Result<NonZeroUsize> {
    let global = js_sys::global().unchecked_into::<web_sys::WorkerGlobalScope>();
    let count = global.navigator().hardware_concurrency() as usize;
    NonZeroUsize::new(count).ok_or_else(|| io::Error::other("hardwareConcurrency unavailable"))
}

/// Scoped threads run inline in the browser. Only blocking-provider fallback
/// paths use them; sequential execution keeps their results identical and
/// cannot exhaust worker capacity while the caller waits.
pub fn scope<'env, F, T>(f: F) -> T
where
    F: for<'scope> FnOnce(&'scope Scope<'scope, 'env>) -> T,
{
    let scope = Scope {
        unjoined_panic: AtomicBool::new(false),
        scope: PhantomData,
        env: PhantomData,
    };
    let result = f(&scope);
    if scope.unjoined_panic.load(Ordering::Relaxed) {
        panic!("a scoped thread panicked");
    }
    result
}

pub struct Scope<'scope, 'env: 'scope> {
    unjoined_panic: AtomicBool,
    scope: PhantomData<&'scope mut &'scope ()>,
    env: PhantomData<&'env mut &'env ()>,
}

impl<'scope> Scope<'scope, '_> {
    pub fn spawn<F, T>(&'scope self, f: F) -> ScopedJoinHandle<'scope, T>
    where
        F: FnOnce() -> T + Send + 'scope,
        T: Send + 'scope,
    {
        ScopedJoinHandle {
            result: Some(catch_unwind(AssertUnwindSafe(f))),
            unjoined_panic: &self.unjoined_panic,
        }
    }
}

pub struct ScopedJoinHandle<'scope, T> {
    result: Option<Result<T>>,
    unjoined_panic: &'scope AtomicBool,
}

impl<T> ScopedJoinHandle<'_, T> {
    pub fn join(mut self) -> Result<T> {
        self.result.take().expect("scoped thread result")
    }

    pub fn is_finished(&self) -> bool {
        true
    }
}

impl<T> Drop for ScopedJoinHandle<'_, T> {
    fn drop(&mut self) {
        if matches!(self.result, Some(Err(_))) {
            self.unjoined_panic.store(true, Ordering::Relaxed);
        }
    }
}
