//! `std::thread` on native targets. In the browser, spawned threads are Web
//! Workers created by the page's main thread (see `web`); the rest of
//! `std::thread` (sleep, park, current) works unchanged inside workers.

#[cfg(not(target_arch = "wasm32"))]
pub use std::thread::*;

#[cfg(target_arch = "wasm32")]
pub use crate::web::thread::*;
