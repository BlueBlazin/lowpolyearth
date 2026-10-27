//! Cube-sphere addressing, coordinate frames, and surface buffer contracts.
//!
//! This crate has no I/O, GPU, or platform dependencies. Geometry transforms
//! and payload encodings are deterministic for identical inputs.

pub mod cell;
pub mod cell_key;
pub mod chunk;
pub mod contracts;
pub mod cubesphere;
pub mod ellipsoid;
pub mod frame;
pub mod lod;
pub mod mesh;

pub use cell::{CellId, ChunkAddr};
pub use chunk::CHUNK_DIM;
pub use cubesphere::Face;
pub use frame::ChunkFrame;
