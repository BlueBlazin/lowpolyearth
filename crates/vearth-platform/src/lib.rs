//! Platform services shared by native and browser builds.
//!
//! Native targets re-export `std` and Tokio unchanged. The browser build runs
//! every Rust thread in a Web Worker over one shared WebAssembly memory; the
//! page's main thread only creates workers and forwards DOM input. Because no
//! Rust code runs on the main thread, blocking primitives (`Mutex`, `Condvar`,
//! channel receives, `thread::sleep`) remain valid everywhere. JavaScript
//! objects never cross threads: fetches and timers run on one reactor worker.

#![cfg_attr(target_arch = "wasm32", feature(thread_local))]

#[cfg(any(target_arch = "wasm32", test))]
pub mod alloc;
pub mod assets;
pub mod env;
#[cfg(any(not(target_arch = "wasm32"), feature = "rt"))]
pub mod fs;
pub mod thread;
pub mod time;

#[cfg(feature = "rt")]
pub mod rt;

#[cfg(target_arch = "wasm32")]
pub mod web;
