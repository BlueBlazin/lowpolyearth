//! Native Low Poly Earth viewer built with winit and wgpu.

#![recursion_limit = "256"]

pub mod admission;
pub mod app;
pub mod atlas;
mod benchmark_drone;
pub mod camera;
pub mod capture;
mod cloud_shadows;
mod egui_input;
pub mod gfx;
pub mod host;
pub mod image_contract;
pub mod input;
mod launch_wordmark;
mod local_time;
pub mod location;
pub mod location_title;
mod lod_selection;
mod map;
#[cfg(target_os = "macos")]
pub mod native_menu;
pub mod platform_paths;
pub mod player;
pub mod prepared_render;
pub mod privacy;
pub mod replay;
pub mod replay_measurement;
pub mod residency;
pub mod rollout;
pub mod sky;
mod sky_environment;
pub mod solar;
pub mod streaming;
pub mod telemetry;
pub mod transition;
pub mod weather;
mod world_aa;

pub mod chunk_cache;
#[cfg(not(target_arch = "wasm32"))]
mod indexed_chunk_store;
#[cfg(target_arch = "wasm32")]
#[path = "indexed_chunk_store_web.rs"]
mod indexed_chunk_store;
#[cfg(not(target_arch = "wasm32"))]
pub mod screenshot;
pub mod settings;
pub mod sources;
#[cfg(not(target_arch = "wasm32"))]
pub mod staging;
pub mod ui;
mod vehicle_effects;
pub mod vehicles;
