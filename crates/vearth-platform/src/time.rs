//! Monotonic and wall-clock time. `std::time::Instant::now` panics on
//! `wasm32-unknown-unknown`; these types are `std`'s own on native targets.

pub use std::time::Duration;
pub use web_time::{Instant, SystemTime, SystemTimeError, UNIX_EPOCH};
