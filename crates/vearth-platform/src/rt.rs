//! The Tokio subset used by Low Poly Earth.
//!
//! Native targets re-export Tokio. The browser implementation keeps Tokio's
//! signatures and semantics that the application relies on: `spawn` runs a
//! `Send` future on the reactor worker, `spawn_blocking` runs a closure on a
//! bounded pool of blocking workers, `JoinHandle::abort` drops the task at its
//! next poll, dropping a `JoinHandle` detaches it, and panics surface as
//! [`task::JoinError`]. Every runtime shares the one reactor and pool.

#[cfg(not(target_arch = "wasm32"))]
pub use tokio::{runtime, spawn, task, time};

#[cfg(target_arch = "wasm32")]
pub use crate::web::rt::{runtime, spawn, spawn_local_on_reactor, task, time};

/// Creates a runtime for synchronous callers that need to drive futures
/// (startup probes, tests). Native builds use a current-thread Tokio runtime
/// with timers; browser futures always run on the shared reactor.
pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
    runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime")
        .block_on(future)
}
