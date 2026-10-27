//! Browser runtime: Web Worker threads, console logging and panic reporting.
//!
//! Thread lifecycle, as implemented by `web/thread.js` beside the page:
//!
//! 1. The main thread compiles the module once and starts the root worker,
//!    which instantiates it, creating the shared memory.
//! 2. Every thread (the root included) receives a `MessagePort` to the main
//!    thread through [`attach_spawner`]. Spawning posts a boxed start record to
//!    that port; the main thread creates a worker, instantiates the same
//!    module over the same memory and calls [`run_thread`] with the record.
//!
//! Workers are never created from other workers, so a thread that blocks
//! right after spawning (for example on `join`) cannot stall the new
//! thread's startup.

mod console;
#[cfg(feature = "rt")]
pub mod opfs;
#[cfg(feature = "rt")]
pub(crate) mod rt;
pub mod thread;

pub use console::{install_console_logger, set_log_filter};
pub use thread::{attach_spawner, run_thread};
