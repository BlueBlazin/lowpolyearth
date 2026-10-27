//! wgpu renderer: pipeline setup, per-chunk GPU residency, frame encoding.
//!
//! Chunk model matrices are f64 in ECEF. Each frame rebases them against the
//! camera position in f64 before conversion to f32, preserving precision in
//! nearby vertex math.
//! Depth is reversed-Z with an infinite far plane.
//!
//! `Renderer` is presentation-agnostic (also used by the headless screenshot
//! mode); `Gfx` adds the window surface.

use crate::prepared_render::{
    MAX_RENDER_INDEX_RANGES, PackedRenderCell, PreparedRenderCell, RenderIndexRange,
    TerrainIndexFormat, TerrainIndices, TerrainVertexFormat, TerrainVertices,
};
use crate::rollout::{FinalPipelinePolicy, GpuCapabilities, TerrainSubmission};
use crate::settings::ShadowPreference;
use crate::sky::SkyVisibility;
use crate::solar::{Lighting, greenwich_sidereal_time};
use crate::streaming::ResidentId;
use crate::telemetry::MaterialIndexCounts;
use crate::weather::WeatherRenderState;
use glam::{DMat3, DMat4, DVec3, DVec4, Mat4};
use hw_skymodel::rgb::{SkyParams, SkyState};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc;
use vearth_core::cell::{CellId, ChunkAddr};
use vearth_core::frame::ChunkFrame;
use vearth_core::mesh::{PackedSurfaceVertex, SurfaceVertex};
use vearth_data::climate::ClimateAtlas;
use vearth_gen::flora::{
    CROP_CLOSE_END_M, CROP_COARSE_END_M, CROP_DISTANT_END_M, CROP_MIDDLE_END_M, FloraCell,
    flora_row_gpu_data,
};
#[cfg(test)]
use vearth_gen::flora::{crop_row_cell_phases, crop_row_spacing_m};
use vearth_gen::tier0::{NEAR_TREE_VOXEL_M, TreeInstance, near_tree_color_scale};
use vearth_gen::vegetation::TreeProfileId;
use vearth_platform::time::Instant;
use wgpu::util::DeviceExt;

mod shadow_receiver {
    use super::{SUN_SHADOW_RESOLUTION, SunShadowCascade};
    use glam::{DMat4, DVec2, DVec3};

    const GUARD_FRACTION: f64 = 0.04;

    #[derive(Clone, Copy, Debug, Default)]
    struct Plane {
        inward: DVec3,
        offset: f64,
    }

    #[derive(Clone, Copy, Debug)]
    pub(super) struct ReceiverSupport {
        origin: DVec3,
        planes: [Plane; 5],
        len: usize,
    }

    fn receiver_points(
        camera: DVec3,
        view_rot: DMat4,
        fov_y: f64,
        aspect: f64,
        far_m: f64,
    ) -> Option<[DVec3; 5]> {
        if !camera.is_finite()
            || !view_rot.is_finite()
            || !fov_y.is_finite()
            || !(0.0..std::f64::consts::PI).contains(&fov_y)
            || !aspect.is_finite()
            || aspect <= 0.0
            || !far_m.is_finite()
            || far_m <= 0.0
        {
            return None;
        }
        let camera_to_world = view_rot.inverse();
        let h = far_m * (fov_y * 0.5).tan();
        // Cascade selection uses radial distance. A cone truncated at camera
        // depth far_m contains that radial domain, including corner pixels near
        // the near/far blend. The inner radial exclusion is deliberately ignored.
        let points = [
            DVec3::ZERO,
            DVec3::new(-h * aspect, -h, -far_m),
            DVec3::new(h * aspect, -h, -far_m),
            DVec3::new(-h * aspect, h, -far_m),
            DVec3::new(h * aspect, h, -far_m),
        ]
        .map(|point| camera + camera_to_world.transform_vector3(point));
        points
            .iter()
            .all(|point| point.is_finite())
            .then_some(points)
    }

    fn filter_margin(cascade: &SunShadowCascade, points: &[DVec3; 5]) -> f64 {
        let magnitude = points
            .iter()
            .map(|point| (*point - cascade.center_ecef).abs().element_sum())
            .fold(cascade.radius_m, f64::max);
        // Two texels cover the square 2x2 comparison-filter footprint in every
        // hull-plane direction. Reserve f32 model/projection rounding separately.
        4.0 * cascade.radius_m / f64::from(SUN_SHADOW_RESOLUTION)
            + magnitude * f64::from(f32::EPSILON) * 16.0
            + 1.0e-6
    }

    impl ReceiverSupport {
        pub(super) fn build(
            cascade: &SunShadowCascade,
            camera: DVec3,
            view_rot: DMat4,
            fov_y: f64,
            aspect: f64,
        ) -> Option<Self> {
            Self::with_guard(cascade, camera, view_rot, fov_y, aspect, GUARD_FRACTION)
        }

        fn with_guard(
            cascade: &SunShadowCascade,
            camera: DVec3,
            view_rot: DMat4,
            fov_y: f64,
            aspect: f64,
            guard_fraction: f64,
        ) -> Option<Self> {
            let points = receiver_points(camera, view_rot, fov_y, aspect, cascade.split_far_m)?;
            if !cascade.center_ecef.is_finite()
                || !cascade.right.is_finite()
                || !cascade.up.is_finite()
                || !cascade.radius_m.is_finite()
                || cascade.radius_m <= 0.0
            {
                return None;
            }
            let mut projected = points.map(|point| {
                let relative = point - cascade.center_ecef;
                DVec2::new(cascade.right.dot(relative), cascade.up.dot(relative))
            });
            projected.sort_by(|a, b| a.x.total_cmp(&b.x).then_with(|| a.y.total_cmp(&b.y)));
            let mut hull = [DVec2::ZERO; 10];
            let mut len = 0;
            for point in projected {
                while len >= 2
                    && (hull[len - 1] - hull[len - 2]).perp_dot(point - hull[len - 1]) <= 0.0
                {
                    len -= 1;
                }
                hull[len] = point;
                len += 1;
            }
            let lower = len;
            for &point in projected[..4].iter().rev() {
                while len > lower
                    && (hull[len - 1] - hull[len - 2]).perp_dot(point - hull[len - 1]) <= 0.0
                {
                    len -= 1;
                }
                hull[len] = point;
                len += 1;
            }
            len = len.checked_sub(1)?;
            if !(3..=5).contains(&len) {
                return None;
            }
            let margin = filter_margin(cascade, &points) + cascade.radius_m * guard_fraction;
            let mut support = Self {
                origin: cascade.center_ecef,
                planes: [Plane::default(); 5],
                len,
            };
            for i in 0..len {
                let edge = hull[(i + 1) % len] - hull[i];
                let length = edge.length();
                if !length.is_finite() || length <= 1.0e-9 {
                    return None;
                }
                let inward = DVec2::new(-edge.y, edge.x) / length;
                support.planes[i] = Plane {
                    inward: cascade.right * inward.x + cascade.up * inward.y,
                    offset: -inward.dot(hull[i]) + margin,
                };
            }
            Some(support)
        }

        pub(super) fn intersects(
            &self,
            center: DVec3,
            mut bound_support: impl FnMut(DVec3) -> f64,
        ) -> bool {
            self.planes[..self.len].iter().all(|plane| {
                plane.inward.dot(center - self.origin) + plane.offset + bound_support(plane.inward)
                    >= 0.0
            })
        }

        pub(super) fn covers(
            &self,
            rendered: &SunShadowCascade,
            camera: DVec3,
            view_rot: DMat4,
            fov_y: f64,
            aspect: f64,
        ) -> bool {
            let Some(points) =
                receiver_points(camera, view_rot, fov_y, aspect, rendered.split_far_m)
            else {
                return false;
            };
            let margin = filter_margin(rendered, &points);
            self.planes[..self.len].iter().all(|plane| {
                points
                    .iter()
                    .all(|point| plane.inward.dot(*point - self.origin) + plane.offset >= margin)
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn cascade() -> SunShadowCascade {
            SunShadowCascade {
                light_view_proj: DMat4::IDENTITY,
                center_ecef: DVec3::ZERO,
                right: DVec3::X,
                up: DVec3::Y,
                forward: DVec3::Z,
                radius_m: 20.0,
                depth_extent_m: 1000.0,
                split_far_m: 10.0,
            }
        }

        #[test]
        fn receiver_extrusion_retains_upstream_offscreen_casters_and_rejects_side_geometry() {
            let c = SunShadowCascade {
                right: DVec3::Z,
                up: DVec3::Y,
                forward: -DVec3::X,
                ..cascade()
            };
            let support =
                ReceiverSupport::build(&c, DVec3::ZERO, DMat4::IDENTITY, 1.0, 1.0).unwrap();
            // The sun is along +X. These casters are outside the camera cone but
            // share light XY with the visible receiver at (0, 0, -5).
            assert!(support.intersects(DVec3::new(500.0, 0.0, -5.0), |_| 0.0));
            assert!(support.intersects(DVec3::new(900.0, 0.0, -5.0), |_| 0.0));
            assert!(!support.intersects(DVec3::new(500.0, 0.0, -20.0), |_| 0.0));
            assert!(support.intersects(DVec3::new(500.0, 0.0, -20.0), |_| 10.0));
        }

        #[test]
        fn receiver_filter_margin_retains_neighboring_pcf_texels() {
            let c = cascade();
            let support =
                ReceiverSupport::with_guard(&c, DVec3::ZERO, DMat4::IDENTITY, 1.0, 1.0, 0.0)
                    .unwrap();
            let points =
                receiver_points(DVec3::ZERO, DMat4::IDENTITY, 1.0, 1.0, c.split_far_m).unwrap();
            let edge = c.split_far_m * 0.5_f64.tan();
            let margin = filter_margin(&c, &points);
            assert!(support.intersects(DVec3::new(edge + margin * 0.99, 0.0, -100.0), |_| 0.0));
            assert!(!support.intersects(DVec3::new(edge + margin + 0.001, 0.0, -100.0), |_| 0.0));
        }

        #[test]
        fn escaped_receiver_support_overrides_ineligible_cadence_slot() {
            let c = cascade();
            let support =
                ReceiverSupport::build(&c, DVec3::ZERO, DMat4::IDENTITY, 1.0, 1.0).unwrap();
            let rendered = [Some(c); 2];
            let supports = [Some(support); 2];
            let scheduled =
                super::super::select_sun_shadow_updates(&rendered, &[c; 2], 0, true, 1, false);
            assert_eq!(scheduled, 0);
            let escaped = super::super::sun_shadow_receiver_refresh_mask(
                &rendered,
                &supports,
                DVec3::X * 2.0,
                DMat4::IDENTITY,
                1.0,
                1.0,
            );
            assert_eq!(scheduled | escaped, 3);
            let covered = super::super::sun_shadow_receiver_refresh_mask(
                &rendered,
                &supports,
                DVec3::ZERO,
                DMat4::IDENTITY,
                1.0,
                1.0,
            );
            assert_eq!(scheduled | covered, 0);
            assert_eq!(
                super::super::sun_shadow_receiver_refresh_mask(
                    &rendered,
                    &[None; 2],
                    DVec3::X * 2.0,
                    DMat4::IDENTITY,
                    1.0,
                    1.0,
                ),
                0
            );
        }

        #[test]
        fn receiver_support_reuses_subsets_and_forces_refresh_after_turn_or_translation() {
            let c = cascade();
            let support =
                ReceiverSupport::build(&c, DVec3::ZERO, DMat4::IDENTITY, 1.0, 1.0).unwrap();
            assert!(support.covers(&c, DVec3::ZERO, DMat4::IDENTITY, 1.0, 1.0));
            assert!(support.covers(&c, DVec3::ZERO, DMat4::IDENTITY, 0.6, 1.0));
            assert!(support.covers(&c, DVec3::X * 0.2, DMat4::IDENTITY, 1.0, 1.0));
            assert!(!support.covers(&c, DVec3::X * 2.0, DMat4::IDENTITY, 1.0, 1.0));
            assert!(!support.covers(
                &c,
                DVec3::ZERO,
                DMat4::from_rotation_y(std::f64::consts::FRAC_PI_2),
                1.0,
                1.0
            ));
        }
    }
}

use shadow_receiver::ReceiverSupport;

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Ordinary terrain/player draws need only their model, normal frame and fade.
const TERRAIN_U_SIZE: u64 = 128;
/// Rare mixed chunks add one alternate calibration-landmark model.
const LANDMARK_U_SIZE: u64 = 192;
/// Night-light cells add four spatial fade rows to the ordinary transform.
const LIGHT_U_SIZE: u64 = 192;
/// Tree instances retain their exact model and normal frame, while omitting
/// terrain/light-only fields from `ChunkU`.
const TREE_U_SIZE: u64 = 128;
const FLORA_INSTANCE_SIZE: u64 = 192;
const FLORA_GROUND_OFFSET_BITS: usize = 13;
const FLORA_GROUND_OFFSET_MASK: u32 = (1 << FLORA_GROUND_OFFSET_BITS) - 1;
const MAX_DRAWS: usize = 16 * 1024;
// Contiguous surviving ranges coalesce. Alternating visible/hidden ranges
// maximize commands, at ceil(range_count / 2); an unpartitioned parent uses one.
const MAX_COMMANDS_PER_PARENT: usize = MAX_RENDER_INDEX_RANGES.div_ceil(2);
const MAX_WORLD_COMMANDS: usize = MAX_DRAWS * MAX_COMMANDS_PER_PARENT;
const MAX_LIGHT_DRAWS: usize = MAX_DRAWS;
const MAX_TREE_DRAWS: usize = 8192;
const SUN_SHADOW_CASCADES: usize = 2;
const SUN_SHADOW_RESOLUTION: u32 = 1536;
const SUN_SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth16Unorm;
/// Keep the light-space snapping origin regional. A rotating directional-light
/// basis applied to absolute ECEF coordinates would otherwise turn the Earth's
/// radius into metres of snap-phase motion per rendered frame at 1x time.
/// A cached map cannot represent motion below a fraction of one shadow texel.
/// Refresh after a quarter-texel projected change so the existing 2x2 PCF
/// footprint advances in bounded sub-filter steps rather than frame cadence.
const SUN_SHADOW_PROJECTION_UPDATE_TEXELS: f64 = 0.25;
/// Depth16Unorm is linear for the orthographic shadow projection. Half of one
/// stored depth level is the corresponding representable-change threshold.
const SUN_SHADOW_DEPTH_UPDATE_UNITS: f64 = 0.5;
const SUN_SHADOW_DEPTH_LEVELS: f64 = u16::MAX as f64;
const SUN_SHADOW_TEXEL_BYTES: u64 = 2;
const SUN_SHADOW_WORKING_SET_BYTES: u64 = SUN_SHADOW_RESOLUTION as u64
    * SUN_SHADOW_RESOLUTION as u64
    * SUN_SHADOW_CASCADES as u64
    * SUN_SHADOW_TEXEL_BYTES;
const MAX_SHADOW_CHUNKS: usize = MAX_DRAWS;
const MAX_SHADOW_MODELS: usize = MAX_SHADOW_CHUNKS + 1;
const MAX_SHADOW_COMMANDS: usize =
    MAX_SHADOW_CHUNKS * SUN_SHADOW_CASCADES * MAX_COMMANDS_PER_PARENT;
/// Flora submission limits. The upload buffer is exactly this byte budget, so
/// record or instance-count growth cannot silently exceed it.
pub const MAX_FLORA_DRAWS: usize = 12;
pub const MAX_FLORA_GLYPHS: u32 = 100_000;
pub const MAX_FLORA_TRIANGLES: u64 = 1_500_000;
pub const MAX_FLORA_UPLOAD_BYTES: usize = 128 * 1024;
const MAX_FLORA_INSTANCES: usize = MAX_FLORA_UPLOAD_BYTES / FLORA_INSTANCE_SIZE as usize;
const FLORA_VERTICES_PER_BOX: u32 = 24;
const FLORA_INDICES_PER_BOX: u32 = 36;
// Overview canopy patches must align with every descriptor ground plane.
const CROP_DISTANT_GRID_EDGE: u32 = vearth_gen::flora::FLORA_GROUND_EDGE as u32 - 1;
/// The densest close flora draw emits 512 procedural candidates per cell.
/// Both static index-buffer regions use the same capacity, allowing LODs to
/// move between single and compound glyphs without changing buffer layout.
const MAX_FLORA_CANDIDATES_PER_INSTANCE: u32 = 512;
const FLORA_SINGLE_INDEX_COUNT: u32 = MAX_FLORA_CANDIDATES_PER_INSTANCE * FLORA_INDICES_PER_BOX;
const FLORA_COMPOUND_INDEX_OFFSET: u32 = FLORA_SINGLE_INDEX_COUNT;
const FLORA_COMPOUND_INDEX_COUNT: u32 = FLORA_SINGLE_INDEX_COUNT * 2;
/// For tiny stands, preserving strict front-to-back order costs less than
/// constructing instance batches and avoids a possible small-scene regression.
const TREE_INSTANCE_BATCH_THRESHOLD: usize = 16;
/// Coarse front-to-back bands retain most early-Z benefit while allowing all
/// instances of one species in a band to share a draw call.
const TREE_DISTANCE_BIN_M: f64 = 40.0;
const ARENA_VERTEX_PAGE_BYTES: u64 = 16 * 1024 * 1024;
const ARENA_INDEX_PAGE_BYTES: u64 = 8 * 1024 * 1024;
const LIGHT_ARENA_PAGE_BYTES: u64 = 4 * 1024 * 1024;
/// Streaming admits at most 4 MiB per frame. One quarter of that total keeps
/// the belt's reusable chunks large enough for normal mesh writes without
/// retaining a whole frame's peak allocation in every chunk.
const STAGING_BELT_CHUNK_BYTES: u64 = 1024 * 1024;
/// Fine tree meshes fade in as the farthest tree in a cell moves through this
/// band. A cell-wide blend is required because its fallback trees are baked
/// into one terrain mesh and cannot be hidden independently.
const NEAR_TREE_FADE_INNER_M: f64 = 280.0;
const NEAR_TREE_FADE_OUTER_M: f64 = 360.0;
const AURORA_SEGMENTS: u32 = 1024;
const AURORA_CURTAINS: u32 = 12;
const GPU_QUERY_COUNT: u32 = 12;
const GPU_QUERY_BYTES: u64 = GPU_QUERY_COUNT as u64 * wgpu::QUERY_SIZE as u64;
const GPU_TIMING_RING_SIZE: usize = 3;
const ATMO_LUT_WIDTH: u32 = 256;
const ATMO_LUT_HEIGHT: u32 = 64;
/// Tiled 64³ RGBA8 shape volume, with filtered mips for distant cloud columns.
const CLOUD_SHAPE_SIZE: u32 = 64;
const CLOUD_SHAPE_MIP_LEVELS: u32 = CLOUD_SHAPE_SIZE.ilog2() + 1;
/// `queue.write_texture` rows are padded to the portable copy alignment even
/// though lower quality volume sizes may have shorter useful rows.
const CLOUD_SHAPE_BYTES_PER_ROW: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
const CLOUD_COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const CLOUD_META_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const CLOUD_HISTORY_BUFFERS: u64 = 2;
const CLOUD_INTERLEAVE_SIZE: u32 = 4;
/// The application clock is deliberately allowed to run at 600x. A normal
/// frame can therefore advance weather by ten simulated seconds, and a frame
/// at the application's 100 ms delta cap advances it by sixty. Treat those as
/// continuous evolution; explicit time jumps such as the UI's +/-1 hour
/// controls still invalidate history.
const CLOUD_HISTORY_MAX_WEATHER_STEP_SECONDS: f32 = 90.0;
const PRECIPITATION_INSTANCES: u32 = 6_144;
const CLOUD_COLOR_BYTES_PER_PIXEL: u64 = 8;
const CLOUD_META_BYTES_PER_PIXEL: u64 = 8;
const COLOR_GRADE_LUT_SIZE: u32 = 32;
/// Static screenshots warm each pixel through eight history updates. At one
/// 4×4 update phase per presented frame, eight complete cycles require 128
/// frames.
pub const CLOUD_HISTORY_WARMUP_FRAMES: usize = 128;

const TIMING_IDLE: u8 = 0;
const TIMING_PENDING: u8 = 1;
const TIMING_READY: u8 = 2;
const TIMING_FAILED: u8 = 3;

const SHADOW_BEGIN_QUERY: u32 = 0;
const SHADOW_END_QUERY: u32 = 1;
const WORLD_BEGIN_QUERY: u32 = 2;
const WORLD_END_QUERY: u32 = 3;
const ATMOSPHERE_BEGIN_QUERY: u32 = 4;
const ATMOSPHERE_END_QUERY: u32 = 5;
const CLOUD_BEGIN_QUERY: u32 = 6;
const CLOUD_END_QUERY: u32 = 7;
const PRECIPITATION_BEGIN_QUERY: u32 = 8;
const PRECIPITATION_END_QUERY: u32 = 9;
const TONEMAP_BEGIN_QUERY: u32 = 10;
const TONEMAP_END_QUERY: u32 = 11;

/// Compact clear-sky lookup prepared once at startup. This is intentionally
/// smooth and bounded: solar discs, halos and twilight colours are not encoded
/// as screen-space shapes.
fn build_atmosphere_lut() -> Vec<u8> {
    let mut pixels = vec![0u8; (ATMO_LUT_WIDTH * ATMO_LUT_HEIGHT * 4) as usize];
    for y in 0..ATMO_LUT_HEIGHT {
        let optical = (y as f32 + 0.5) / ATMO_LUT_HEIGHT as f32;
        for x in 0..ATMO_LUT_WIDTH {
            let sun = (x as f32 + 0.5) / ATMO_LUT_WIDTH as f32;
            let transmittance = (-optical * (1.4 + 2.4 * (1.0 - sun))).exp();
            let scatter = 1.0 - (-optical * 3.2).exp();
            let warm = scatter * (1.0 - sun).powf(1.6);
            let multiple = scatter * (0.18 + 0.34 * optical) * (0.35 + 0.65 * sun);
            let base = ((y * ATMO_LUT_WIDTH + x) * 4) as usize;
            for (channel, value) in [transmittance, scatter, warm, multiple]
                .into_iter()
                .enumerate()
            {
                pixels[base + channel] = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
    }
    pixels
}

fn cloud_noise_hash(mut value: u32) -> f32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^= value >> 16;
    (value & 0x00ff_ffff) as f32 * (1.0 / 16_777_216.0)
}

fn cloud_lattice_hash(x: i32, y: i32, z: i32, salt: u32) -> f32 {
    let value = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ (z as u32).wrapping_mul(0xcb1a_b31f)
        ^ salt.wrapping_mul(0x9e37_79b9);
    cloud_noise_hash(value)
}

fn cloud_wrap_cell(value: i32, cells: i32) -> i32 {
    value.rem_euclid(cells)
}

fn cloud_value_noise_tiled(point: [f32; 3], cells: i32) -> f32 {
    let scaled = [
        point[0] * cells as f32,
        point[1] * cells as f32,
        point[2] * cells as f32,
    ];
    let base = [
        scaled[0].floor() as i32,
        scaled[1].floor() as i32,
        scaled[2].floor() as i32,
    ];
    let local = [
        scaled[0] - base[0] as f32,
        scaled[1] - base[1] as f32,
        scaled[2] - base[2] as f32,
    ];
    let blend = local.map(|value| value * value * (3.0 - 2.0 * value));
    let sample = |dx: i32, dy: i32, dz: i32| {
        cloud_lattice_hash(
            cloud_wrap_cell(base[0] + dx, cells),
            cloud_wrap_cell(base[1] + dy, cells),
            cloud_wrap_cell(base[2] + dz, cells),
            0,
        )
    };
    let lerp = |a: f32, b: f32, amount: f32| a + (b - a) * amount;
    let z0 = lerp(
        lerp(sample(0, 0, 0), sample(1, 0, 0), blend[0]),
        lerp(sample(0, 1, 0), sample(1, 1, 0), blend[0]),
        blend[1],
    );
    let z1 = lerp(
        lerp(sample(0, 0, 1), sample(1, 0, 1), blend[0]),
        lerp(sample(0, 1, 1), sample(1, 1, 1), blend[0]),
        blend[1],
    );
    lerp(z0, z1, blend[2])
}

#[cfg(test)]
fn cloud_worley_f1_tiled(point: [f32; 3], cells: i32) -> f32 {
    let scaled = [
        point[0] * cells as f32,
        point[1] * cells as f32,
        point[2] * cells as f32,
    ];
    let base = [
        scaled[0].floor() as i32,
        scaled[1].floor() as i32,
        scaled[2].floor() as i32,
    ];
    let local = [
        scaled[0] - base[0] as f32,
        scaled[1] - base[1] as f32,
        scaled[2] - base[2] as f32,
    ];
    let mut nearest_squared = f32::INFINITY;
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let cell = [
                    cloud_wrap_cell(base[0] + dx, cells),
                    cloud_wrap_cell(base[1] + dy, cells),
                    cloud_wrap_cell(base[2] + dz, cells),
                ];
                let feature = [
                    dx as f32 + cloud_lattice_hash(cell[0], cell[1], cell[2], 1),
                    dy as f32 + cloud_lattice_hash(cell[0], cell[1], cell[2], 2),
                    dz as f32 + cloud_lattice_hash(cell[0], cell[1], cell[2], 3),
                ];
                let delta = [
                    feature[0] - local[0],
                    feature[1] - local[1],
                    feature[2] - local[2],
                ];
                nearest_squared = nearest_squared
                    .min(delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]);
            }
        }
    }
    nearest_squared.sqrt().clamp(0.0, 1.0)
}

struct CloudWorleyLattice {
    cells: usize,
    features: Vec<[f32; 3]>,
}

impl CloudWorleyLattice {
    fn new(cells: usize) -> Self {
        // The halo keeps periodic addressing outside the per-voxel search.
        // Feature offsets stay cell-local to preserve f32 rounding exactly.
        let side = cells + 2;
        let mut features = Vec::with_capacity(side.pow(3));
        for z in 0..side {
            for y in 0..side {
                for x in 0..side {
                    let [x, y, z] =
                        [x, y, z].map(|axis| cloud_wrap_cell(axis as i32 - 1, cells as i32));
                    features.push([1, 2, 3].map(|salt| cloud_lattice_hash(x, y, z, salt)));
                }
            }
        }
        Self { cells, features }
    }

    fn sample(&self, point: [f32; 3]) -> f32 {
        let scaled = point.map(|axis| axis * self.cells as f32);
        let base = scaled.map(|axis| axis.floor() as usize);
        let local = std::array::from_fn::<_, 3, _>(|axis| scaled[axis] - base[axis] as f32);
        let side = self.cells + 2;
        let mut nearest_squared = f32::INFINITY;
        for z in 0..3 {
            for y in 0..3 {
                let start = ((base[2] + z) * side + base[1] + y) * side + base[0];
                for (x, feature) in self.features[start..start + 3].iter().enumerate() {
                    let delta = [
                        (x as i32 - 1) as f32 + feature[0] - local[0],
                        (y as i32 - 1) as f32 + feature[1] - local[1],
                        (z as i32 - 1) as f32 + feature[2] - local[2],
                    ];
                    nearest_squared = nearest_squared
                        .min(delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]);
                }
            }
        }
        nearest_squared.sqrt().clamp(0.0, 1.0)
    }
}

/// Deterministic, licence-independent approximation of the common
/// Perlin/Worley cloud volume layout. At the body tiling, R is the broad
/// hybrid mass and G the inverse-Worley billow field that sets cell crowns.
/// B is a smooth value-noise field the shaders read from a coarse mip at a
/// much broader tiling to cluster cells. A is an inverse-Worley fbm sampled
/// at a much finer edge-detail tiling, so its octaves use lattices unrelated
/// to the body's. Every octave wraps, so trilinear
/// sampling cannot expose a seam as wind advects the volume.
fn build_cloud_shape_volume() -> Vec<u8> {
    let lattices = [4, 8, 16].map(CloudWorleyLattice::new);
    let detail_lattices = [5, 10, 20].map(CloudWorleyLattice::new);
    let slice_stride = (CLOUD_SHAPE_BYTES_PER_ROW * CLOUD_SHAPE_SIZE) as usize;
    let mut pixels = vec![0u8; slice_stride * CLOUD_SHAPE_SIZE as usize];
    for z in 0..CLOUD_SHAPE_SIZE {
        for y in 0..CLOUD_SHAPE_SIZE {
            for x in 0..CLOUD_SHAPE_SIZE {
                let point = [
                    (x as f32 + 0.5) / CLOUD_SHAPE_SIZE as f32,
                    (y as f32 + 0.5) / CLOUD_SHAPE_SIZE as f32,
                    (z as f32 + 0.5) / CLOUD_SHAPE_SIZE as f32,
                ];
                let value_fbm = cloud_value_noise_tiled(point, 4) * 0.625
                    + cloud_value_noise_tiled(point, 8) * 0.25
                    + cloud_value_noise_tiled(point, 16) * 0.125;
                let worley4 = 1.0 - lattices[0].sample(point);
                let worley8 = 1.0 - lattices[1].sample(point);
                let worley16 = 1.0 - lattices[2].sample(point);
                let broad_worley = worley4 * 0.625 + worley8 * 0.25 + worley16 * 0.125;
                let detail = detail_lattices
                    .iter()
                    .zip([0.625, 0.25, 0.125])
                    .map(|(lattice, weight)| (1.0 - lattice.sample(point)) * weight)
                    .sum::<f32>();
                let cluster = cloud_value_noise_tiled(point, 4) * 0.65
                    + cloud_value_noise_tiled(point, 8) * 0.35;
                let channels = [
                    smoothstep_f32(0.18, 0.86, value_fbm * 0.68 + broad_worley * 0.32),
                    broad_worley.clamp(0.0, 1.0),
                    cluster,
                    detail.clamp(0.0, 1.0),
                ];
                let offset = z as usize * slice_stride
                    + y as usize * CLOUD_SHAPE_BYTES_PER_ROW as usize
                    + x as usize * 4;
                for (channel, value) in channels.into_iter().enumerate() {
                    pixels[offset + channel] = (value * 255.0).round() as u8;
                }
            }
        }
    }
    pixels
}

fn cloud_shape_mip_chain(base: Vec<u8>) -> Vec<Vec<u8>> {
    assert_eq!(
        base.len(),
        (CLOUD_SHAPE_BYTES_PER_ROW * CLOUD_SHAPE_SIZE.pow(2)) as usize
    );
    let row = CLOUD_SHAPE_BYTES_PER_ROW as usize;
    let mut levels = vec![base];
    for level in 1..CLOUD_SHAPE_MIP_LEVELS {
        let side = (CLOUD_SHAPE_SIZE >> level) as usize;
        let previous_side = side * 2;
        let previous = levels.last().unwrap();
        let mut pixels = vec![0; row * side * side];
        for z in 0..side {
            for y in 0..side {
                for x in 0..side {
                    for channel in 0..4 {
                        let mut sum = 0_u16;
                        for dz in 0..2 {
                            for dy in 0..2 {
                                for dx in 0..2 {
                                    let index = ((z * 2 + dz) * previous_side + y * 2 + dy) * row
                                        + (x * 2 + dx) * 4
                                        + channel;
                                    sum += u16::from(previous[index]);
                                }
                            }
                        }
                        pixels[(z * side + y) * row + x * 4 + channel] = ((sum + 4) / 8) as u8;
                    }
                }
            }
        }
        levels.push(pixels);
    }
    levels
}

/// A deliberately restrained creative grade. It cools deep atmospheric
/// shadows and gives bright daylight a small warm-density response without
/// encoding exposure, tone mapping or the separate saturation treatment.
fn build_color_grade_lut() -> Vec<u8> {
    let side = COLOR_GRADE_LUT_SIZE;
    let mut pixels = vec![0u8; (side * side * side * 4) as usize];
    for blue in 0..side {
        for green in 0..side {
            for red in 0..side {
                let color = [
                    red as f32 / (side - 1) as f32,
                    green as f32 / (side - 1) as f32,
                    blue as f32 / (side - 1) as f32,
                ];
                let luminance = color[0] * 0.2126 + color[1] * 0.7152 + color[2] * 0.0722;
                let shadows = 1.0 - smoothstep_f32(0.08, 0.62, luminance);
                let highlights = smoothstep_f32(0.30, 0.94, luminance);
                let graded = [
                    color[0] * (1.0 + 0.026 * highlights - 0.006 * shadows) + 0.003 * highlights,
                    color[1] * (1.0 + 0.004 * shadows + 0.003 * highlights),
                    color[2] * (1.0 + 0.020 * shadows - 0.027 * highlights) + 0.002 * shadows,
                ];
                let offset = (((blue * side + green) * side + red) * 4) as usize;
                for channel in 0..3 {
                    pixels[offset + channel] =
                        (graded[channel].clamp(0.0, 1.0) * 255.0).round() as u8;
                }
                pixels[offset + 3] = 255;
            }
        }
    }
    pixels
}

/// Hosek-Wilkie's fitted all-sky state: 27 distribution coefficients followed
/// by the RGB radiance scales. Four-by-four packing matches WGSL uniform rules.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SkyModelUniform {
    values: [[f32; 4]; 8],
}

struct SkyLighting {
    uniform: SkyModelUniform,
    daylight_ambient: [f32; 3],
    horizon_radiance: [[f32; 4]; 3],
}

fn sky_model_lighting(
    solar_sine: f32,
    aerosol_optical_depth: f32,
    humidity: f32,
    snow_cover: f32,
) -> SkyLighting {
    let finite = |value: f32, fallback| if value.is_finite() { value } else { fallback };
    let elevation = finite(solar_sine, 0.5).clamp(0.0, 1.0).asin();
    let humidity_growth = 1.0 + 0.35 * smoothstep_f32(0.72, 0.98, finite(humidity, 0.5));
    // AOD is the observed 550-nm vertical optical depth. Converting it to the
    // model's Linke-style turbidity keeps clean maritime air near T=2 while
    // allowing polluted continental air to approach T=6-8.
    let turbidity = (1.0 + finite(aerosol_optical_depth, 0.08).max(0.0) * 9.0 * humidity_growth)
        .clamp(1.0, 10.0);
    let snow = finite(snow_cover, 0.0).clamp(0.0, 1.0);
    let albedo = [0.12 + 0.63 * snow, 0.13 + 0.65 * snow, 0.15 + 0.67 * snow];
    let state = SkyState::new(&SkyParams {
        elevation,
        turbidity,
        albedo,
    })
    .expect("clamped Hosek-Wilkie sky parameters are valid");
    let (parameters, radiances) = state.raw();
    let mut packed = [0.0; 32];
    packed[..27].copy_from_slice(&parameters);
    packed[27..30].copy_from_slice(&radiances);
    SkyLighting {
        uniform: SkyModelUniform {
            values: bytemuck::cast(packed),
        },
        daylight_ambient: calibrated_sky_ambient(sky_diffuse_average(&state, elevation)),
        horizon_radiance: sky_horizon_radiance(&state, elevation),
    }
}

// E / pi for an unobstructed upward Lambertian receiver. Eight Gauss-Legendre
// nodes in cos(theta) and twelve azimuths integrate the cosine-weighted dome;
// directions and weights are immutable, so each frame only evaluates 96 rays.
fn sky_diffuse_average(state: &SkyState, elevation: f32) -> [f32; 3] {
    use hw_skymodel::rgb::Channel;
    static SAMPLES: std::sync::OnceLock<[[f32; 4]; 96]> = std::sync::OnceLock::new();
    let samples = SAMPLES.get_or_init(|| {
        let nodes = [
            (-0.960_289_84_f32, 0.101_228_535_f32),
            (-0.796_666_5, 0.222_381_04),
            (-0.525_532_4, 0.313_706_64),
            (-0.183_434_64, 0.362_683_77),
            (0.183_434_64, 0.362_683_77),
            (0.525_532_4, 0.313_706_64),
            (0.796_666_5, 0.222_381_04),
            (0.960_289_84, 0.101_228_535),
        ];
        std::array::from_fn(|index| {
            let (node, weight) = nodes[index / 12];
            let cosine = (node + 1.0) * 0.5;
            let azimuth = (index % 12) as f32 + 0.5;
            let horizontal =
                (1.0 - cosine * cosine).sqrt() * (azimuth * std::f32::consts::TAU / 12.0).cos();
            [cosine.acos(), cosine, horizontal, weight * cosine / 12.0]
        })
    });
    let (sun_height, sun_horizontal) = elevation.sin_cos();
    let mut average = [0.0; 3];
    for &[theta, cosine, horizontal, weight] in samples {
        let gamma = (cosine * sun_height + horizontal * sun_horizontal)
            .clamp(-1.0, 1.0)
            .acos();
        for (value, channel) in average.iter_mut().zip([Channel::R, Channel::G, Channel::B]) {
            *value += state.radiance(theta, gamma, channel).max(0.0) * weight;
        }
    }
    average
}

fn sky_horizon_radiance(state: &SkyState, elevation: f32) -> [[f32; 4]; 3] {
    use hw_skymodel::rgb::Channel;
    let view_elevation = 3.0_f32.to_radians();
    [1.0_f32, 0.0, -1.0].map(|azimuth_cosine| {
        let gamma = (view_elevation.sin() * elevation.sin()
            + view_elevation.cos() * elevation.cos() * azimuth_cosine)
            .clamp(-1.0, 1.0)
            .acos();
        let theta = std::f32::consts::FRAC_PI_2 - view_elevation;
        let rgb = [Channel::R, Channel::G, Channel::B]
            .map(|channel| state.radiance(theta, gamma, channel).max(0.0) * 0.035);
        [rgb[0], rgb[1], rgb[2], 0.0]
    })
}

// Keep this CPU counterpart aligned with sky_palette.wgsl. Only the three
// horizon samples need grading; diffuse ambient and terrain shader cost stay
// independent of this presentation adjustment.
pub(crate) fn sunset_sky_grade_cpu(
    radiance: [f32; 3],
    view_sine: f32,
    solar_sine: f32,
    cosine_gamma: f32,
    observer_height: f32,
    aerosol_optical_depth: f32,
    humidity: f32,
) -> [f32; 3] {
    let twilight = smoothstep_f32(
        (-6.0_f32).to_radians().sin(),
        (-1.0_f32).to_radians().sin(),
        solar_sine,
    ) * (1.0
        - smoothstep_f32(
            2.0_f32.to_radians().sin(),
            10.0_f32.to_radians().sin(),
            solar_sine,
        ));
    let low_sky = 1.0
        - smoothstep_f32(
            2.0_f32.to_radians().sin(),
            20.0_f32.to_radians().sin(),
            view_sine.max(0.0),
        );
    let sunward = smoothstep_f32(0.0, 0.95, cosine_gamma);
    let surface = 1.0 - smoothstep_f32(8000.0, 24000.0, observer_height.max(0.0));
    let weather = 0.50
        + 0.30 * smoothstep_f32(0.025, 0.20, aerosol_optical_depth)
        + 0.20 * smoothstep_f32(0.45, 0.90, humidity);
    let amount = twilight * low_sky * sunward * surface * weather;
    std::array::from_fn(|channel| {
        radiance[channel] * (1.0 + ([1.10, 0.72, 0.36][channel] - 1.0) * amount)
    })
}

fn calibrated_sky_ambient(average: [f32; 3]) -> [f32; 3] {
    // Match fitted_clear_sky's linear radiance scale, then compress diffuse
    // energy into the renderer's daylight range. A shared gain and luminance
    // blend retain sky hue without allowing blue fill to repaint materials.
    let scene = average.map(|value| value * 0.035);
    let luminance = scene[0] * 0.2126 + scene[1] * 0.7152 + scene[2] * 0.0722;
    let gain = 0.32 / (0.09 + luminance);
    let ambient = scene.map(|channel| (luminance * 0.35 + channel * 0.65) * gain);
    let peak = ambient.into_iter().fold(0.40_f32, f32::max);
    // One common gain bounds extreme skies without changing their hue.
    ambient.map(|channel| channel * (0.40 / peak))
}

fn smoothstep_f32(edge0: f32, edge1: f32, value: f32) -> f32 {
    let x = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    view_proj: [f32; 16],
    /// xyz = Earth-to-Sun direction; w = 1 for the shared shader ABI.
    sun_dir: [f32; 4],
    moon_dir: [f32; 4],
    ambient: [f32; 4],
    inv_view_proj: [f32; 16],
    /// xyz = planet center camera-relative, w = local ground radius.
    planet_center: [f32; 4],
    /// xy = render size in pixels, z = tan(vertical FOV / 2).
    viewport: [f32; 4],
    /// x/y = cos/sin GMST, z = UTC modulo 4096 s, w = local upward emission.
    sky: [f32; 4],
    /// Eight north-clockwise distant-city light-dome samples.
    horizon_glow_0: [f32; 4],
    horizon_glow_1: [f32; 4],
    weather0: [f32; 4],
    weather1: [f32; 4],
    wind: [f32; 4],
    surface: [f32; 4],
    weather2: [f32; 4],
    weather3: [f32; 4],
    weather4: [f32; 4],
    weather5: [f32; 4],
    weather6: [f32; 4],
    wind_low: [f32; 4],
    wind_mid: [f32; 4],
    wind_upper: [f32; 4],
    /// Previous projection in the previous frame's camera-relative space.
    prev_view_proj: [f32; 16],
    /// xyz translates current camera-relative positions into the preceding
    /// camera-relative frame; w is the elapsed simulation time in seconds.
    camera_to_prev: [f32; 4],
    /// x = history valid, y = jitter phase, z = previous lightning flash,
    /// w = simulation seconds since the latest UTC hour mark.
    temporal: [f32; 4],
    /// xyz = camera ECEF modulo 8192 m, preserving physical texture phase;
    /// w = visible cloud volumes enabled, also used by the reflection cache.
    material_origin: [f32; 4],
    /// Linear horizon radiance toward, beside, and away from the local Sun.
    aerial_sunward: [f32; 4],
    aerial_side: [f32; 4],
    aerial_away: [f32; 4],
    /// xyz = wrapped ECEF cloud advection in metres; w = cloud history
    /// sample cap for the current rate of appearance change.
    cloud_drift: [f32; 4],
}

/// Independently switchable renderer presentation choices. Neither changes
/// chunk generation, surface geometry, collision, weather evolution or scene depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PostProcessSettings {
    pub color_grading: bool,
    pub filmic_saturation: bool,
}

impl Default for PostProcessSettings {
    fn default() -> Self {
        Self {
            color_grading: true,
            filmic_saturation: true,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PostProcessUniform {
    flags: [u32; 4],
}

impl PostProcessUniform {
    fn new(settings: PostProcessSettings) -> Self {
        Self {
            flags: [
                u32::from(settings.color_grading),
                u32::from(settings.filmic_saturation),
                0,
                0,
            ],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TerrainU {
    model: [f32; 16],
    bx: [f32; 4],
    by: [f32; 4],
    bz: [f32; 4],
    /// x = signed LOD dither alpha; y = near-tree blend. Ordinary world
    /// records use z for the signed Y cofactor magnitude and w for model pitch.
    fade: [f32; 4],
}

impl TerrainU {
    fn with_world_normal_parameters(mut self, parameters: [f32; 4]) -> Self {
        self.bx[3] = parameters[0];
        self.bz[3] = parameters[2];
        self.fade[2] = parameters[1];
        self.fade[3] = parameters[3];
        self
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LandmarkU {
    model: [f32; 16],
    bx: [f32; 4],
    by: [f32; 4],
    bz: [f32; 4],
    fade: [f32; 4],
    /// Calibration-tagged voxel units -> camera-relative ECEF.
    landmark_model: [f32; 16],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LightU {
    model: [f32; 16],
    bx: [f32; 4],
    by: [f32; 4],
    bz: [f32; 4],
    fade: [f32; 4],
    /// Effective additive weights for the cell's 4x4 night-light buckets.
    light_fade: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TreeU {
    model: [f32; 16],
    /// xyz = basis normal; w = signed geometric-LOD dither alpha.
    bx: [f32; 4],
    by: [f32; 4],
    bz: [f32; 4],
    /// x = signed terrain-LOD dither alpha; y = near-tree blend; z =
    /// per-instance linear colour scale; w = object-mesh voxel pitch.
    fade: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ShadowModelU {
    model: [f32; 16],
    landmark_model: [f32; 16],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ShadowPassU {
    light_view_proj: [f32; 16],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SunShadowU {
    light_view_proj_0: [f32; 16],
    light_view_proj_1: [f32; 16],
    splits: [f32; 4],
    tuning: [f32; 4],
}

#[derive(Clone, Copy, Debug)]
struct SunShadowCascade {
    light_view_proj: DMat4,
    center_ecef: DVec3,
    right: DVec3,
    up: DVec3,
    forward: DVec3,
    radius_m: f64,
    depth_extent_m: f64,
    split_far_m: f64,
}

#[derive(Clone, Copy, Debug, Default)]
struct SunShadowProjectionChange {
    xy_texels: f64,
    depth_units: f64,
}

fn sun_shadow_policy_enabled(
    configured: bool,
    shadows: ShadowPreference,
    solar_sine: f64,
    camera_altitude_m: f64,
    cloud_optical_depth: f32,
) -> bool {
    configured
        && shadows != ShadowPreference::Off
        && solar_sine.is_finite()
        && solar_sine > 0.015
        && camera_altitude_m.is_finite()
        && camera_altitude_m < 20_000.0
        && cloud_optical_depth.is_finite()
        && cloud_optical_depth < 8.0
}

fn sun_shadow_matrix(
    center_camera_relative: DVec3,
    right: DVec3,
    up: DVec3,
    forward: DVec3,
    radius_m: f64,
    depth_extent_m: f64,
) -> DMat4 {
    let inverse_radius = radius_m.recip();
    let inverse_depth = (depth_extent_m * 2.0).recip();
    DMat4::from_cols(
        DVec4::new(
            right.x * inverse_radius,
            up.x * inverse_radius,
            forward.x * inverse_depth,
            0.0,
        ),
        DVec4::new(
            right.y * inverse_radius,
            up.y * inverse_radius,
            forward.y * inverse_depth,
            0.0,
        ),
        DVec4::new(
            right.z * inverse_radius,
            up.z * inverse_radius,
            forward.z * inverse_depth,
            0.0,
        ),
        DVec4::new(
            -right.dot(center_camera_relative) * inverse_radius,
            -up.dot(center_camera_relative) * inverse_radius,
            0.5 - forward.dot(center_camera_relative) * inverse_depth,
            1.0,
        ),
    )
}

fn snap_sun_shadow_center(
    raw_center_ecef: DVec3,
    snap_anchor_ecef: DVec3,
    right: DVec3,
    up: DVec3,
    texel_m: f64,
) -> DVec3 {
    let relative = raw_center_ecef - snap_anchor_ecef;
    let right_offset = (right.dot(relative) / texel_m).round() * texel_m;
    let up_offset = (up.dot(relative) / texel_m).round() * texel_m;
    raw_center_ecef
        + right * (right_offset - right.dot(relative))
        + up * (up_offset - up.dot(relative))
}

fn build_sun_shadow_cascades(
    cam_pos: DVec3,
    view_rot: DMat4,
    fov_y: f64,
    aspect: f64,
    sun_dir: DVec3,
    camera_altitude_m: f64,
    snap_anchors_ecef: [DVec3; SUN_SHADOW_CASCADES],
) -> Option<[SunShadowCascade; SUN_SHADOW_CASCADES]> {
    if !sun_dir.is_finite()
        || sun_dir.length_squared() < 0.99
        || !fov_y.is_finite()
        || !aspect.is_finite()
        || aspect <= 0.0
    {
        return None;
    }
    let sun_dir = sun_dir.normalize();
    let forward = -sun_dir;
    let helper = if forward.dot(DVec3::Y).abs() < 0.92 {
        DVec3::Y
    } else {
        DVec3::X
    };
    let right = helper.cross(forward).normalize();
    let up = forward.cross(right).normalize();
    let altitude = camera_altitude_m.max(0.0);
    // Concentrate the first map on visible object-scale contact shadows;
    // the second map retains kilometre-scale terrain coverage.
    let near_end = (600.0 + altitude * 0.2).clamp(600.0, 1_800.0);
    let far_end = (6_000.0 + altitude).clamp(6_000.0, 12_000.0);
    let slices = [(0.05, near_end), (near_end * 0.82, far_end)];
    let tan_half_fov = (fov_y * 0.5).tan();
    let camera_to_world = view_rot.inverse();

    let build = |(slice_near, slice_far): (f64, f64), snap_anchor_ecef: DVec3| {
        let mut corners = [DVec3::ZERO; 8];
        let mut index = 0;
        for distance in [slice_near, slice_far] {
            let half_height = distance * tan_half_fov;
            let half_width = half_height * aspect;
            for y in [-half_height, half_height] {
                for x in [-half_width, half_width] {
                    corners[index] = camera_to_world.transform_vector3(DVec3::new(x, y, -distance));
                    index += 1;
                }
            }
        }
        let center_camera_relative = corners.iter().copied().sum::<DVec3>() / 8.0;
        let raw_radius = corners
            .iter()
            .map(|corner| corner.distance(center_camera_relative))
            .fold(0.0_f64, f64::max);
        // Fixed eight-metre radius buckets avoid a slowly breathing projection
        // when FOV math or high-rate camera input changes by sub-pixel amounts.
        let radius_m = (raw_radius / 8.0).ceil().max(1.0) * 8.0;
        let texel_m = radius_m * 2.0 / f64::from(SUN_SHADOW_RESOLUTION);
        let center_ecef = snap_sun_shadow_center(
            cam_pos + center_camera_relative,
            snap_anchor_ecef,
            right,
            up,
            texel_m,
        );
        let center_camera_relative = center_ecef - cam_pos;
        // Include a long upstream/downstream interval for low-Sun casters.
        // XY projection remains the tight receiver bound, so this does not
        // admit unrelated terrain beside the view slice.
        let depth_extent_m = radius_m + (slice_far * 1.5).max(6_000.0);
        SunShadowCascade {
            light_view_proj: sun_shadow_matrix(
                center_camera_relative,
                right,
                up,
                forward,
                radius_m,
                depth_extent_m,
            ),
            center_ecef,
            right,
            up,
            forward,
            radius_m,
            depth_extent_m,
            split_far_m: slice_far,
        }
    };
    Some(std::array::from_fn(|index| {
        build(slices[index], snap_anchors_ecef[index])
    }))
}

fn sun_shadow_projection_coordinates(cascade: &SunShadowCascade, point_ecef: DVec3) -> DVec3 {
    let relative = point_ecef - cascade.center_ecef;
    DVec3::new(
        (cascade.right.dot(relative) / (2.0 * cascade.radius_m) + 0.5)
            * f64::from(SUN_SHADOW_RESOLUTION),
        (cascade.up.dot(relative) / (2.0 * cascade.radius_m) + 0.5)
            * f64::from(SUN_SHADOW_RESOLUTION),
        (cascade.forward.dot(relative) / (2.0 * cascade.depth_extent_m) + 0.5)
            * SUN_SHADOW_DEPTH_LEVELS,
    )
}

/// Exact maximum affine projection change over the desired cascade box. The
/// extrema of each old-vs-new coordinate component occur at one of its eight
/// corners, so this captures translation, rotation and scale without a
/// time-scale-specific or camera-speed-specific heuristic.
fn sun_shadow_projection_change(
    rendered: &SunShadowCascade,
    desired: &SunShadowCascade,
) -> SunShadowProjectionChange {
    let mut change = SunShadowProjectionChange::default();
    for x in [-1.0, 1.0] {
        for y in [-1.0, 1.0] {
            for z in [-1.0, 1.0] {
                let point = desired.center_ecef
                    + desired.right * (x * desired.radius_m)
                    + desired.up * (y * desired.radius_m)
                    + desired.forward * (z * desired.depth_extent_m);
                let before = sun_shadow_projection_coordinates(rendered, point);
                let after = sun_shadow_projection_coordinates(desired, point);
                let delta = (after - before).abs();
                change.xy_texels = change.xy_texels.max(delta.x.max(delta.y));
                change.depth_units = change.depth_units.max(delta.z);
            }
        }
    }
    change
}

fn sun_shadow_projection_requires_refresh(
    rendered: &SunShadowCascade,
    desired: &SunShadowCascade,
) -> bool {
    let change = sun_shadow_projection_change(rendered, desired);
    change.xy_texels >= SUN_SHADOW_PROJECTION_UPDATE_TEXELS
        || change.depth_units >= SUN_SHADOW_DEPTH_UPDATE_UNITS
}

#[cfg(test)]
fn sun_shadow_cascade_mask(
    cascades: &[SunShadowCascade; SUN_SHADOW_CASCADES],
    center_ecef: DVec3,
    radius_m: f64,
) -> u8 {
    sun_shadow_cascade_mask_for_updates(cascades, center_ecef, radius_m, 0b11)
}

fn sun_shadow_cascade_mask_for_updates(
    cascades: &[SunShadowCascade; SUN_SHADOW_CASCADES],
    center_ecef: DVec3,
    radius_m: f64,
    eligible_mask: u8,
) -> u8 {
    let mut mask = 0u8;
    for (index, cascade) in cascades.iter().enumerate() {
        if eligible_mask & (1 << index) == 0 {
            continue;
        }
        let relative = center_ecef - cascade.center_ecef;
        let light_depth = cascade.forward.dot(relative);
        let inside = cascade.right.dot(relative).abs() <= cascade.radius_m + radius_m
            && cascade.up.dot(relative).abs() <= cascade.radius_m + radius_m
            // Upstream casters may project into the receiver slice. Geometry
            // downstream of its farthest receiver cannot and is discarded.
            && light_depth >= -cascade.depth_extent_m - radius_m
            && light_depth <= cascade.radius_m + radius_m;
        if inside {
            mask |= 1 << index;
        }
    }
    mask
}

fn sun_shadow_requires_full_refresh(
    rendered: &[Option<SunShadowCascade>; SUN_SHADOW_CASCADES],
    desired: &[SunShadowCascade; SUN_SHADOW_CASCADES],
) -> bool {
    rendered.iter().zip(desired).any(|(rendered, desired)| {
        let Some(rendered) = rendered else {
            return true;
        };
        rendered.center_ecef.distance(desired.center_ecef) > desired.radius_m * 0.15
            || rendered.forward.dot(desired.forward) < 0.999_96
            || (rendered.radius_m - desired.radius_m).abs() > desired.radius_m * 0.10
    })
}

fn select_sun_shadow_updates(
    rendered: &[Option<SunShadowCascade>; SUN_SHADOW_CASCADES],
    desired: &[SunShadowCascade; SUN_SHADOW_CASCADES],
    caster_dirty_mask: u8,
    active_last_frame: bool,
    update_frame: u64,
    dynamic_near_caster: bool,
) -> u8 {
    if !active_last_frame || sun_shadow_requires_full_refresh(rendered, desired) {
        0b11
    } else {
        let mut dirty = 0u8;
        for index in 0..SUN_SHADOW_CASCADES {
            let projection_changed = rendered[index].is_none_or(|rendered| {
                sun_shadow_projection_requires_refresh(&rendered, &desired[index])
            });
            if projection_changed || caster_dirty_mask & (1 << index) != 0 {
                dirty |= 1 << index;
            }
        }
        if dynamic_near_caster {
            dirty |= 0b01;
        }
        let eligible = match update_frame % 8 {
            0 | 2 | 4 | 6 => 0b01,
            7 => 0b10,
            _ => 0,
        };
        dirty & eligible
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct SunShadowCasterKey {
    resident: ResidentId,
    upload_generation: u64,
    chunk_index: usize,
}

#[derive(Default)]
struct SunShadowCasterPopulation {
    // Probe epochs make exact membership/count comparison order-independent
    // without allocating a fresh set, including repeated draw-list entries.
    keys: HashMap<SunShadowCasterKey, u64>,
    landmark_delta: Option<[u64; 16]>,
    had_player: bool,
}

#[derive(Default)]
struct SunShadowCasterProbe {
    matched: usize,
    added: bool,
    landmark: bool,
}
impl SunShadowCasterProbe {
    fn observe(
        &mut self,
        cached: &mut SunShadowCasterPopulation,
        key: SunShadowCasterKey,
        landmark: bool,
        epoch: u64,
    ) {
        self.landmark |= landmark;
        if let Some(seen) = cached.keys.get_mut(&key) {
            if *seen != epoch {
                *seen = epoch;
                self.matched += 1;
            }
        } else {
            self.added = true;
        }
    }

    fn matches(&self, cached: &SunShadowCasterPopulation, delta: Option<DMat4>) -> bool {
        !self.added
            && self.matched == cached.keys.len()
            && cached.landmark_delta == shadow_landmark_delta(self.landmark, delta)
    }
}

fn shadow_landmark_delta(present: bool, delta: Option<DMat4>) -> Option<[u64; 16]> {
    present
        .then_some(delta)
        .flatten()
        .map(|matrix| matrix.to_cols_array().map(f64::to_bits))
}

#[derive(Clone, Copy)]
struct SunShadowCandidate {
    key: SunShadowCasterKey,
    cell: CellId,
    cascade_mask: u8,
    distance_sq: f64,
}

fn sun_shadow_capacity(requested: usize) -> (usize, usize) {
    let admitted = requested.min(MAX_SHADOW_CHUNKS);
    (admitted, requested - admitted)
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FloraInstance {
    /// Camera-relative affine transform stored as three rows. The omitted
    /// fourth row is always `[0, 0, 0, 1]`.
    model_x: [f32; 4],
    model_y: [f32; 4],
    model_z: [f32; 4],
    identity: [u32; 4],
    state: [u32; 4],
    params: [f32; 4],
    coverage: [u32; 8],
    /// Centimetre base plus 25 signed 13-bit centimetre offsets. Managed
    /// fields are slope-gated; ±40.95 m within one cell covers valid field
    /// relief while keeping all 25 samples and exact centimetre precision.
    ground_base_cm: i32,
    ground_offsets_cm: [u32; 11],
    /// Bit-cast f32 crop-row spacing, phase and perturbation phase.
    crop_rows: [u32; 3],
    /// Explicit storage-array stride padding.
    _padding: u32,
}

#[derive(Clone, Copy)]
struct FloraFrameGpu {
    model: DMat4,
    bound_center: DVec3,
    bound_radius: f64,
    local_y_min: f64,
    local_y_max: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
enum FloraLod {
    Close,
    Middle,
    Coarse,
    Distant,
}

impl FloraLod {
    const COUNT: usize = 4;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum FloraTopology {
    CropFlower,
    Orchard,
    Vineyard,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct FloraLodAssignment {
    lod: FloraLod,
    /// Signed complementary dither alpha: positive fades in; negative fades
    /// out with the exact opposite predicate.
    fade: f32,
}

fn flora_lod_assignments(distance_m: f64) -> ([FloraLodAssignment; 2], usize) {
    const NONE: FloraLodAssignment = FloraLodAssignment {
        lod: FloraLod::Coarse,
        fade: 0.0,
    };
    if distance_m < 50.0 {
        (
            [
                FloraLodAssignment {
                    lod: FloraLod::Close,
                    fade: 1.0,
                },
                NONE,
            ],
            1,
        )
    } else if distance_m < f64::from(CROP_CLOSE_END_M) {
        let t = ((distance_m - 50.0) / f64::from(CROP_CLOSE_END_M - 50.0)) as f32;
        (
            [
                FloraLodAssignment {
                    lod: FloraLod::Close,
                    fade: -(1.0 - t),
                },
                FloraLodAssignment {
                    lod: FloraLod::Middle,
                    fade: t,
                },
            ],
            2,
        )
    } else if distance_m < 180.0 {
        (
            [
                FloraLodAssignment {
                    lod: FloraLod::Middle,
                    fade: 1.0,
                },
                NONE,
            ],
            1,
        )
    } else if distance_m < f64::from(CROP_MIDDLE_END_M) {
        let t = ((distance_m - 180.0) / f64::from(CROP_MIDDLE_END_M - 180.0)) as f32;
        (
            [
                FloraLodAssignment {
                    lod: FloraLod::Middle,
                    fade: -(1.0 - t),
                },
                FloraLodAssignment {
                    lod: FloraLod::Coarse,
                    fade: t,
                },
            ],
            2,
        )
    } else if distance_m < 550.0 {
        (
            [
                FloraLodAssignment {
                    lod: FloraLod::Coarse,
                    fade: 1.0,
                },
                NONE,
            ],
            1,
        )
    } else if distance_m < f64::from(CROP_COARSE_END_M) {
        let t = ((distance_m - 550.0) / f64::from(CROP_COARSE_END_M - 550.0)) as f32;
        (
            [
                FloraLodAssignment {
                    lod: FloraLod::Coarse,
                    fade: -(1.0 - t),
                },
                FloraLodAssignment {
                    lod: FloraLod::Distant,
                    fade: t,
                },
            ],
            2,
        )
    } else if distance_m < 5_000.0 {
        (
            [
                FloraLodAssignment {
                    lod: FloraLod::Distant,
                    fade: 1.0,
                },
                NONE,
            ],
            1,
        )
    } else if distance_m < f64::from(CROP_DISTANT_END_M) {
        let t = ((distance_m - 5_000.0) / f64::from(CROP_DISTANT_END_M - 5_000.0)) as f32;
        (
            [
                FloraLodAssignment {
                    lod: FloraLod::Distant,
                    fade: 1.0 - t,
                },
                NONE,
            ],
            1,
        )
    } else {
        ([NONE, NONE], 0)
    }
}

fn flora_topology(archetype: vearth_gen::flora::FloraArchetype) -> FloraTopology {
    use vearth_gen::flora::FloraArchetype;
    match archetype {
        FloraArchetype::Orchard => FloraTopology::Orchard,
        FloraArchetype::Vineyard => FloraTopology::Vineyard,
        _ => FloraTopology::CropFlower,
    }
}

fn flora_candidate_target(topology: FloraTopology, lod: FloraLod) -> u32 {
    match (topology, lod) {
        (FloraTopology::CropFlower, FloraLod::Close) => 512,
        (FloraTopology::CropFlower, FloraLod::Middle) => 256,
        (FloraTopology::CropFlower, FloraLod::Coarse) => 64,
        (FloraTopology::CropFlower, FloraLod::Distant) => CROP_DISTANT_GRID_EDGE.pow(2),
        (FloraTopology::Orchard, FloraLod::Close) => 128,
        (FloraTopology::Orchard, FloraLod::Middle) => 64,
        (FloraTopology::Orchard, FloraLod::Coarse) => 32,
        (FloraTopology::Orchard, FloraLod::Distant) => 12,
        (FloraTopology::Vineyard, FloraLod::Close) => 256,
        (FloraTopology::Vineyard, FloraLod::Middle) => 128,
        (FloraTopology::Vineyard, FloraLod::Coarse) => 48,
        (FloraTopology::Vineyard, FloraLod::Distant) => 16,
    }
}

fn flora_active_candidates(archetype: vearth_gen::flora::FloraArchetype, lod: FloraLod) -> u32 {
    use vearth_gen::flora::FloraArchetype::{FlowerCrop, MeadowWildflowers, OrnamentalBed};
    if lod == FloraLod::Distant
        && matches!(archetype, FlowerCrop | OrnamentalBed | MeadowWildflowers)
    {
        // Flower fields retain their authored 12-cluster distribution while
        // sharing the padded distant draw with ground-aligned annual crops.
        12
    } else {
        flora_candidate_target(flora_topology(archetype), lod)
    }
}

fn flora_boxes_per_glyph(topology: FloraTopology, lod: FloraLod) -> u32 {
    match (topology, lod) {
        // Middle annual/flower glyphs are already cluster proxies. One larger
        // opaque box preserves their conceptual density for half the triangle
        // cost, leaving budget for the multi-kilometre overview ring.
        (FloraTopology::CropFlower, FloraLod::Middle)
        | (_, FloraLod::Coarse | FloraLod::Distant) => 1,
        (_, FloraLod::Close | FloraLod::Middle) => 2,
    }
}

fn flora_indices_per_glyph(topology: FloraTopology, lod: FloraLod) -> u32 {
    flora_boxes_per_glyph(topology, lod) * FLORA_INDICES_PER_BOX
}

fn flora_triangles_per_glyph(topology: FloraTopology, lod: FloraLod) -> u64 {
    u64::from(flora_indices_per_glyph(topology, lod) / 3)
}

fn append_flora_indices(indices: &mut Vec<u16>, boxes_per_glyph: u32) {
    for glyph in 0..MAX_FLORA_CANDIDATES_PER_INSTANCE {
        for primitive in 0..boxes_per_glyph {
            let box_vertex = (glyph * boxes_per_glyph + primitive) * FLORA_VERTICES_PER_BOX;
            for face in 0..6 {
                let face_vertex = box_vertex + face * 4;
                for corner in [0, 1, 2, 0, 2, 3] {
                    indices.push(
                        u16::try_from(face_vertex + corner)
                            .expect("flora index layout fits in u16"),
                    );
                }
            }
        }
    }
}

fn flora_index_data() -> Vec<u16> {
    let total_indices = FLORA_SINGLE_INDEX_COUNT + FLORA_COMPOUND_INDEX_COUNT;
    let mut indices = Vec::with_capacity(total_indices as usize);
    append_flora_indices(&mut indices, 1);
    append_flora_indices(&mut indices, 2);
    debug_assert_eq!(indices.len(), total_indices as usize);
    indices
}

fn flora_index_range(topology: FloraTopology, lod: FloraLod) -> std::ops::Range<u32> {
    let candidates = flora_candidate_target(topology, lod);
    assert!(candidates <= MAX_FLORA_CANDIDATES_PER_INSTANCE);
    let first = if flora_boxes_per_glyph(topology, lod) == 1 {
        0
    } else {
        FLORA_COMPOUND_INDEX_OFFSET
    };
    first..first + candidates * flora_indices_per_glyph(topology, lod)
}

fn flora_frame_distance(frame: FloraFrameGpu, cam_pos: DVec3) -> f64 {
    let local = frame.model.inverse().transform_point3(cam_pos);
    let closest = DVec3::new(
        local.x.clamp(0.0, 32.0),
        local.y.clamp(frame.local_y_min, frame.local_y_max),
        local.z.clamp(0.0, 32.0),
    );
    frame.model.transform_point3(closest).distance(cam_pos)
}

/// Canonical lattice address mirrored by `flora.wgsl`. The cap is irrelevant
/// to close geometry (which runs on the detailed cut) but prevents shifts from
/// overflowing if a custom caller supplies a very coarse fallback cell.
#[cfg(test)]
fn canonical_flora_lattice(cell: CellId, candidate: u32) -> (u32, u32) {
    let shift = 19u8.saturating_sub(cell.level).min(7);
    let edge = 16u32 << shift;
    let total = edge * edge;
    let flat = candidate.wrapping_mul(40_503) % total;
    let scale = 1u32 << shift;
    let reference_cells = 1u32 << (cell.level + shift);
    (
        (cell.i << shift) * 16 + flat % edge,
        (reference_cells - (cell.j + 1) * scale) * 16 + flat / edge,
    )
}

struct VisibleFloraUnit<'a> {
    cell: CellId,
    descriptor_index: usize,
    descriptor: &'a FloraCell,
    frame: FloraFrameGpu,
    terrain_fade: f32,
    distance_m: f64,
    assignments: [FloraLodAssignment; 2],
    assignment_count: usize,
}

struct FloraDraw {
    topology: FloraTopology,
    lod: FloraLod,
    instances: std::ops::Range<u32>,
}

struct PreparedFlora {
    instances: Vec<FloraInstance>,
    draws: Vec<FloraDraw>,
    stats: FloraRendererStats,
}

fn make_flora_instance(
    unit: &VisibleFloraUnit<'_>,
    assignment: FloraLodAssignment,
    cam_pos: DVec3,
) -> FloraInstance {
    let descriptor = unit.descriptor;
    let mut model = unit.frame.model;
    model.w_axis.x -= cam_pos.x;
    model.w_axis.y -= cam_pos.y;
    model.w_axis.z -= cam_pos.z;
    let model = model.to_cols_array().map(|value| value as f32);
    let mut ground_offsets_cm = [0u32; 11];
    for (index, offset) in descriptor.ground_cm.into_iter().enumerate() {
        let bits = (i32::from(offset).clamp(-4_096, 4_095) as u32) & FLORA_GROUND_OFFSET_MASK;
        let bit = index * FLORA_GROUND_OFFSET_BITS;
        let word = bit / u32::BITS as usize;
        let shift = bit % u32::BITS as usize;
        ground_offsets_cm[word] |= bits << shift;
        if shift + FLORA_GROUND_OFFSET_BITS > u32::BITS as usize {
            ground_offsets_cm[word + 1] |= bits >> (u32::BITS as usize - shift);
        }
    }
    let packed_identity = descriptor.archetype as u32
        | u32::from(descriptor.phenology_offset) << 8
        | u32::from(descriptor.curated_style) << 16
        | u32::from(descriptor.calendar as u8) << 24;
    let mut flora_rows = flora_row_gpu_data(
        unit.cell,
        descriptor.archetype,
        descriptor.row_heading_rad,
        descriptor.field_seed,
    );
    if descriptor.palette == vearth_gen::agriculture_catalog::CropKind::Tea.palette() {
        flora_rows = vearth_gen::flora::tea_row_gpu_data(
            unit.cell,
            descriptor.row_heading_rad,
            descriptor.field_seed,
        );
    }
    FloraInstance {
        model_x: [model[0], model[4], model[8], model[12]],
        model_y: [model[1], model[5], model[9], model[13]],
        model_z: [model[2], model[6], model[10], model[14]],
        identity: [
            descriptor.field_seed as u32,
            (descriptor.field_seed >> 32) as u32,
            unit.cell.i,
            unit.cell.j,
        ],
        state: [
            packed_identity,
            assignment.lod as u32,
            flora_active_candidates(descriptor.archetype, assignment.lod),
            u32::from(unit.cell.face.0)
                | u32::from(unit.cell.level) << 8
                | u32::from(descriptor.palette) << 16,
        ],
        params: [
            descriptor.row_heading_rad,
            f32::from(descriptor.base_density) / 255.0,
            unit.terrain_fade,
            assignment.fade,
        ],
        coverage: descriptor.coverage,
        ground_base_cm: descriptor.ground_base_cm,
        ground_offsets_cm,
        crop_rows: flora_rows,
        _padding: 0,
    }
}

fn prepare_flora(
    mut units: Vec<VisibleFloraUnit<'_>>,
    cam_pos: DVec3,
    resident_cells: usize,
) -> PreparedFlora {
    let distribution_key = |unit: &VisibleFloraUnit<'_>| {
        let mut key = u64::from(unit.cell.face.0)
            | u64::from(unit.cell.level) << 8
            | u64::from(unit.cell.i) << 16
            | u64::from(unit.cell.j) << 40;
        key ^= (unit.descriptor_index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        key ^= key >> 30;
        key = key.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        key ^= key >> 27;
        key.wrapping_mul(0x94d0_49bb_1331_11eb) ^ (key >> 31)
    };
    let lod_rank = |unit: &VisibleFloraUnit<'_>| {
        unit.assignments[..unit.assignment_count]
            .iter()
            .map(|assignment| assignment.lod as u8)
            .min()
            .unwrap_or(u8::MAX)
    };
    units.sort_unstable_by(|a, b| {
        let a_lod = lod_rank(a);
        let b_lod = lod_rank(b);
        a_lod.cmp(&b_lod).then_with(|| {
            if a_lod <= FloraLod::Middle as u8 {
                a.distance_m
                    .total_cmp(&b.distance_m)
                    .then_with(|| a.cell.cmp(&b.cell))
                    .then_with(|| a.descriptor_index.cmp(&b.descriptor_index))
            } else {
                // At aggregate LODs, deterministic field-wide sampling avoids
                // a circular nearest-first cutoff when the upload cap binds.
                distribution_key(a)
                    .cmp(&distribution_key(b))
                    .then_with(|| a.cell.cmp(&b.cell))
                    .then_with(|| a.descriptor_index.cmp(&b.descriptor_index))
            }
        })
    });
    let nearest_visible_m = units
        .iter()
        .map(|unit| unit.distance_m)
        .fold(f64::INFINITY, f64::min);
    let mut stats = FloraRendererStats {
        resident_cells,
        visible_cells: units.len(),
        glyph_budget: MAX_FLORA_GLYPHS,
        instance_upload_budget_bytes: MAX_FLORA_UPLOAD_BYTES,
        nearest_visible_m: if nearest_visible_m.is_finite() {
            nearest_visible_m
        } else {
            0.0
        },
        farthest_visible_m: units.iter().map(|unit| unit.distance_m).fold(0.0, f64::max),
        ..FloraRendererStats::default()
    };
    for unit in &units {
        stats.visible_by_archetype[unit.descriptor.archetype as usize] += 1;
        stats.visible_by_calendar[unit.descriptor.calendar as usize] += 1;
        if let Some(treatment) =
            vearth_gen::agriculture_catalog::treatment_for_palette(unit.descriptor.palette)
        {
            stats.visible_by_crop[treatment.kind as usize] += 1;
        }
        stats.curated_destination_matches += usize::from(unit.descriptor.curated_style != 0);
        if unit.descriptor.curated_style == 0 {
            stats.inferred_by_archetype[unit.descriptor.archetype as usize] += 1;
        } else if let Some(count) = stats
            .visible_by_registry
            .get_mut(usize::from(unit.descriptor.curated_style - 1))
        {
            *count += 1;
        }
        for assignment in &unit.assignments[..unit.assignment_count] {
            stats.visible_by_lod[assignment.lod as usize] += 1;
        }
    }

    // Budget admission is unit-atomic: both sides of a plant-LOD transition
    // are kept or suppressed together, which preserves complementary coverage.
    let mut admitted = 0usize;
    let mut admitted_glyphs = 0u32;
    let mut admitted_triangles = 0u64;
    let mut admitted_instances = 0usize;
    for (index, unit) in units.iter().enumerate() {
        let topology = flora_topology(unit.descriptor.archetype);
        let unit_glyphs = unit.assignments[..unit.assignment_count]
            .iter()
            .map(|assignment| flora_candidate_target(topology, assignment.lod))
            .sum::<u32>();
        let unit_triangles = unit.assignments[..unit.assignment_count]
            .iter()
            .map(|assignment| {
                u64::from(flora_candidate_target(topology, assignment.lod))
                    * flora_triangles_per_glyph(topology, assignment.lod)
            })
            .sum::<u64>();
        if admitted_glyphs.saturating_add(unit_glyphs) > MAX_FLORA_GLYPHS {
            stats.cells_suppressed_by_glyph_budget = units.len() - index;
            stats.suppression_reason = FloraSuppressionReason::GlyphBudget;
            break;
        }
        if admitted_triangles.saturating_add(unit_triangles) > MAX_FLORA_TRIANGLES {
            stats.cells_suppressed_by_triangle_budget = units.len() - index;
            stats.suppression_reason = FloraSuppressionReason::TriangleBudget;
            break;
        }
        if admitted_instances.saturating_add(unit.assignment_count) > MAX_FLORA_INSTANCES {
            stats.cells_suppressed_by_upload_budget = units.len() - index;
            stats.suppression_reason = FloraSuppressionReason::InstanceUploadBudget;
            break;
        }
        admitted += 1;
        admitted_glyphs += unit_glyphs;
        admitted_triangles += unit_triangles;
        admitted_instances += unit.assignment_count;
    }
    units.truncate(admitted);

    let mut grouped: BTreeMap<(FloraTopology, FloraLod), Vec<FloraInstance>> = BTreeMap::new();
    for unit in &units {
        let topology = flora_topology(unit.descriptor.archetype);
        for &assignment in &unit.assignments[..unit.assignment_count] {
            grouped
                .entry((topology, assignment.lod))
                .or_default()
                .push(make_flora_instance(unit, assignment, cam_pos));
        }
    }
    let mut instances = Vec::with_capacity(admitted_instances);
    let mut draws = Vec::with_capacity(grouped.len());
    for ((topology, lod), group) in grouped {
        let first = instances.len() as u32;
        instances.extend(group);
        draws.push(FloraDraw {
            topology,
            lod,
            instances: first..instances.len() as u32,
        });
    }
    for unit in &units {
        let coverage = unit
            .descriptor
            .coverage
            .iter()
            .map(|word| word.count_ones())
            .sum::<u32>() as f64
            / 256.0;
        let density = f64::from(unit.descriptor.base_density) / 255.0;
        let flower = matches!(
            unit.descriptor.archetype,
            vearth_gen::flora::FloraArchetype::FlowerCrop
                | vearth_gen::flora::FloraArchetype::OrnamentalBed
                | vearth_gen::flora::FloraArchetype::MeadowWildflowers
        );
        let crop = matches!(
            unit.descriptor.archetype,
            vearth_gen::flora::FloraArchetype::Cereal
                | vearth_gen::flora::FloraArchetype::BroadleafRows
                | vearth_gen::flora::FloraArchetype::TallRows
                | vearth_gen::flora::FloraArchetype::LowDenseCrop
        );
        for assignment in &unit.assignments[..unit.assignment_count] {
            let lod_index = assignment.lod as usize;
            let topology = flora_topology(unit.descriptor.archetype);
            let candidates = flora_candidate_target(topology, assignment.lod);
            let triangles =
                u64::from(candidates) * flora_triangles_per_glyph(topology, assignment.lod);
            stats.glyphs_by_lod[lod_index] += candidates;
            stats.triangles_by_lod[lod_index] += triangles;
            let fade_weight = f64::from(assignment.fade.abs());
            let active_candidates =
                flora_active_candidates(unit.descriptor.archetype, assignment.lod);
            let occupied = f64::from(active_candidates) * coverage * density * fade_weight;
            stats.estimated_occupied_glyphs += occupied;
            if flower {
                let heads = match assignment.lod {
                    // A close glyph has two closed primitives. Each primitive
                    // independently becomes either a seasonal flower head or
                    // a non-blooming stem/foliage cluster, so one head per
                    // occupied glyph is the bounded aggregate estimate.
                    FloraLod::Close => 1.0,
                    FloraLod::Middle | FloraLod::Coarse | FloraLod::Distant => 1.0,
                };
                stats.estimated_flower_heads += occupied * heads;
            }
            if crop {
                let represented_plants = match assignment.lod {
                    FloraLod::Close => 4.0,
                    FloraLod::Middle => 8.0,
                    FloraLod::Coarse => 24.0,
                    FloraLod::Distant => 64.0,
                };
                stats.estimated_crop_clusters += occupied;
                stats.estimated_crop_plant_equivalents += occupied * represented_plants;
            }
        }
    }
    stats.glyphs = stats.glyphs_by_lod.iter().sum();
    stats.triangles = stats.triangles_by_lod.iter().sum();
    stats.draw_calls = draws.len();
    stats.instance_upload_bytes = instances.len() * std::mem::size_of::<FloraInstance>();
    stats.admitted_instances = instances.len();
    debug_assert!(stats.glyphs <= MAX_FLORA_GLYPHS);
    debug_assert!(stats.triangles <= MAX_FLORA_TRIANGLES);
    debug_assert!(stats.draw_calls <= MAX_FLORA_DRAWS);
    debug_assert!(stats.instance_upload_bytes <= MAX_FLORA_UPLOAD_BYTES);
    PreparedFlora {
        instances,
        draws,
        stats,
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct NightLightVertex {
    center: [f32; 3],
    energy: f32,
    corner: [f32; 2],
    color: [u8; 4],
    bucket: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct StarInstance {
    direction_icrs: [f32; 3],
    magnitude: f32,
    bv: f32,
}

/// One cell to draw with its LOD crossfade state. `fade` in (0, 1] draws
/// the cell dithering *in* (keep pixels where hash < fade); `fade` in
/// [-1, 0) draws it dithering *out* with weight |fade| (keep pixels where
/// hash ≥ 1-|fade| — the exact complement, so a paired swap covers every
/// pixel exactly once). 1.0 is fully opaque.
#[derive(Clone, Copy)]
pub struct DrawCell {
    pub resident: ResidentId,
    pub cell: CellId,
    pub fade: f32,
}

impl DrawCell {
    pub fn opaque(resident: ResidentId, cell: CellId) -> Self {
        DrawCell {
            resident,
            cell,
            fade: 1.0,
        }
    }
}

pub struct ChunkGpu {
    detail: Option<(DVec3, f64, f64)>,
    detail_visible: std::cell::Cell<bool>,
    allocation: ArenaAllocation,
    /// Live payload bytes suballocated inside the shared arena pages.
    vertex_bytes: u64,
    index_bytes: u64,
    index_count: u32,
    ranges: Vec<RenderIndexRange>,
    material_ranges: Vec<MaterialRangeCounts>,
    shadow: Option<ShadowIndexGpu>,
    /// True when every referenced vertex fits the lossless 16-bit path.
    indices_u16: bool,
    /// False when a terrain vertex cannot be represented exactly in signed 8.8
    /// fixed point; that path retains full floats.
    packed_vertices: bool,
    /// f64 ECEF model transform, rebased per frame.
    model: DMat4,
    /// Immutable geodetic height modulo the shared 256 m facade/material period.
    construction_height_phase: f32,
    /// Unit normals of the positive local faces (not necessarily the model
    /// edge directions: cube-sphere u/v form a skewed horizontal basis).
    axes: [DVec3; 3],
    /// Signed cofactor magnitudes and X pitch from the submitted f32 model.
    /// Translation/rebasing cannot change these immutable vertex parameters.
    normal_parameters: [f32; 4],
    /// Bounding sphere in ECEF (center, radius) for frustum culling.
    pub bound_center: DVec3,
    pub bound_radius: f64,
    bound_box: Option<RenderBoxGpu>,
    has_calibration_landmark: bool,
    has_fragment_feedback: bool,
    has_rare_surface: bool,
}

struct MaterialRangeCounts {
    first_index: u32,
    index_count: u32,
    counts: Vec<(u8, u32)>,
}

// This opt-in diagnostic scans upload payloads once. Its main-thread cost is
// included in upload preparation, so census runs are not timing baselines.
fn material_range_counts(
    vertices: &TerrainVertices,
    indices: &TerrainIndices,
    ranges: &[RenderIndexRange],
) -> Vec<MaterialRangeCounts> {
    let spans = if ranges.is_empty() {
        vec![(0, indices.len() as u32)]
    } else {
        ranges
            .iter()
            .map(|range| (range.first_index, range.index_count))
            .collect()
    };
    let calibration_tag = u32::from(vearth_gen::landmarks::CALIBRATION_MATERIAL_FLAG) << 5;
    spans
        .into_iter()
        .map(|(first_index, index_count)| {
            let mut counts = [0u32; 256];
            for offset in (first_index as usize..(first_index + index_count) as usize).step_by(3) {
                let index = match indices {
                    TerrainIndices::U16(indices) => usize::from(indices[offset]),
                    TerrainIndices::U32(indices) => indices[offset] as usize,
                };
                let face_ao = match vertices {
                    TerrainVertices::Packed(vertices) => vertices[index].to_full().face_ao,
                    TerrainVertices::Full(vertices) => vertices[index].face_ao,
                };
                // The shader removes calibration tagging before decoding the
                // flat material carried by the triangle's first vertex.
                let material = ((face_ao & !calibration_tag) >> 5) & 255;
                counts[material as usize] += 3;
            }
            MaterialRangeCounts {
                first_index,
                index_count,
                counts: counts
                    .into_iter()
                    .enumerate()
                    .filter_map(|(material, count)| (count != 0).then_some((material as u8, count)))
                    .collect(),
            }
        })
        .collect()
}

#[derive(Clone, Copy)]
struct RenderBoxGpu {
    half_axes: [DVec3; 3],
    padding_m: f64,
}

impl RenderBoxGpu {
    fn new(frame: &ChunkFrame, local: crate::prepared_render::RenderLocalBox) -> Self {
        Self {
            half_axes: [
                frame.x_axis * (frame.grid_scale.x * local.half_extents[0]),
                frame.y_axis * (frame.grid_scale.y * local.half_extents[1]),
                frame.z_axis * (frame.grid_scale.z * local.half_extents[2]),
            ],
            padding_m: local.padding_m,
        }
    }

    fn support(&self, normal: DVec3, projection_padding: f64) -> f64 {
        self.half_axes
            .iter()
            .map(|axis| normal.dot(*axis).abs())
            .sum::<f64>()
            + self.padding_m
            + projection_padding
    }

    fn visible(
        &self,
        planes: &[(DVec3, f64); 4],
        relative_center: DVec3,
        projection_padding: f64,
    ) -> bool {
        planes.iter().all(|(normal, offset)| {
            normal.dot(relative_center) + offset + self.support(*normal, projection_padding) >= 0.0
        })
    }
}

fn sun_shadow_box_mask_for_updates(
    cascades: &[SunShadowCascade; SUN_SHADOW_CASCADES],
    center_ecef: DVec3,
    radius_m: f64,
    bounds: Option<&RenderBoxGpu>,
    projection_padding: f64,
    eligible_mask: u8,
) -> u8 {
    let mut mask =
        sun_shadow_cascade_mask_for_updates(cascades, center_ecef, radius_m, eligible_mask);
    if let Some(bounds) = bounds {
        for (index, cascade) in cascades.iter().enumerate() {
            if mask & (1 << index) == 0 {
                continue;
            }
            let relative = center_ecef - cascade.center_ecef;
            let x = bounds.support(cascade.right, projection_padding);
            let y = bounds.support(cascade.up, projection_padding);
            let z = bounds.support(cascade.forward, projection_padding);
            let depth = cascade.forward.dot(relative);
            if cascade.right.dot(relative).abs() > cascade.radius_m + x
                || cascade.up.dot(relative).abs() > cascade.radius_m + y
                || depth < -cascade.depth_extent_m - z
                || depth > cascade.radius_m + z
            {
                mask &= !(1 << index);
            }
        }
    }
    mask
}

fn sun_shadow_receiver_mask(
    supports: &[Option<ReceiverSupport>; SUN_SHADOW_CASCADES],
    center_ecef: DVec3,
    radius_m: f64,
    bounds: Option<&RenderBoxGpu>,
    projection_padding: f64,
    eligible_mask: u8,
) -> u8 {
    let mut mask = eligible_mask;
    for (index, support) in supports.iter().enumerate() {
        if mask & (1 << index) != 0
            && support.is_some_and(|support| {
                !support.intersects(center_ecef, |normal| {
                    bounds.map_or(radius_m, |bounds| {
                        bounds.support(normal, projection_padding)
                    })
                })
            })
        {
            mask &= !(1 << index);
        }
    }
    mask
}

fn sun_shadow_receiver_refresh_mask(
    rendered: &[Option<SunShadowCascade>; SUN_SHADOW_CASCADES],
    supports: &[Option<ReceiverSupport>; SUN_SHADOW_CASCADES],
    camera: DVec3,
    view_rot: DMat4,
    fov_y: f64,
    aspect: f64,
) -> u8 {
    let mut mask = 0;
    for index in 0..SUN_SHADOW_CASCADES {
        if let (Some(cascade), Some(support)) = (&rendered[index], &supports[index])
            && !support.covers(cascade, camera, view_rot, fov_y, aspect)
        {
            mask |= 1 << index;
        }
    }
    mask
}

const SUN_SHADOW_RECEIVER_RECOVERY_FRAMES: u64 = 8;

#[derive(Clone, Copy)]
struct SunShadowReceiverRecovery {
    probe: Option<ReceiverSupport>,
    stable_since: u64,
}

#[derive(Clone, Copy, Default)]
struct SunShadowReceiverPlan {
    supports: [Option<ReceiverSupport>; SUN_SHADOW_CASCADES],
    forced_refresh_mask: u8,
    reentry_dirty_mask: u8,
}

#[allow(clippy::too_many_arguments)]
fn plan_sun_shadow_receivers(
    rendered: &[Option<SunShadowCascade>; SUN_SHADOW_CASCADES],
    supports: &[Option<ReceiverSupport>; SUN_SHADOW_CASCADES],
    recovery: &mut [Option<SunShadowReceiverRecovery>; SUN_SHADOW_CASCADES],
    desired: &[SunShadowCascade; SUN_SHADOW_CASCADES],
    camera: DVec3,
    view_rot: DMat4,
    fov_y: f64,
    aspect: f64,
    frame: u64,
) -> SunShadowReceiverPlan {
    let forced_refresh_mask =
        sun_shadow_receiver_refresh_mask(rendered, supports, camera, view_rot, fov_y, aspect);
    let mut plan = SunShadowReceiverPlan {
        supports: desired
            .map(|cascade| ReceiverSupport::build(&cascade, camera, view_rot, fov_y, aspect)),
        forced_refresh_mask,
        reentry_dirty_mask: 0,
    };
    for index in 0..SUN_SHADOW_CASCADES {
        if forced_refresh_mask & (1 << index) != 0 {
            recovery[index] = Some(SunShadowReceiverRecovery {
                probe: plan.supports[index],
                stable_since: frame,
            });
            // A newly exposed receiver needs a complete map now. Keep the full
            // cascade through continuing motion instead of chasing narrow hulls.
            plan.supports[index] = None;
        } else if let Some(state) = &mut recovery[index] {
            let covered = state.probe.is_some_and(|probe| {
                probe.covers(&desired[index], camera, view_rot, fov_y, aspect)
            });
            if !covered {
                state.probe = plan.supports[index];
                state.stable_since = frame;
            }
            // One whole existing far-cascade cadence must fit a fixed probe.
            // This only admits reentry; rendered support still certifies reuse.
            if covered
                && frame.wrapping_sub(state.stable_since) >= SUN_SHADOW_RECEIVER_RECOVERY_FRAMES
                && plan.supports[index].is_some()
            {
                plan.reentry_dirty_mask |= 1 << index;
            } else {
                plan.supports[index] = None;
            }
        }
    }
    plan
}

fn commit_sun_shadow_receivers(
    supports: &mut [Option<ReceiverSupport>; SUN_SHADOW_CASCADES],
    recovery: &mut [Option<SunShadowReceiverRecovery>; SUN_SHADOW_CASCADES],
    plan: SunShadowReceiverPlan,
    update_mask: u8,
) {
    for index in 0..SUN_SHADOW_CASCADES {
        if update_mask & (1 << index) != 0 {
            supports[index] = plan.supports[index];
            if supports[index].is_some() {
                recovery[index] = None;
            }
        }
    }
}

struct ShadowIndexGpu {
    first_index: u32,
    index_count: u32,
    ranges: Vec<RenderIndexRange>,
    max_deviation_m: f32,
}

fn world_normal_parameters(model: [f32; 16]) -> [f32; 4] {
    let model = Mat4::from_cols_array(&model);
    let [x, y, z] = [
        model.x_axis.truncate(),
        model.y_axis.truncate(),
        model.z_axis.truncate(),
    ];
    let cofactors = [y.cross(z), z.cross(x), x.cross(y)];
    let sign = if x.dot(cofactors[0]) < 0.0 { -1.0 } else { 1.0 };
    [
        cofactors[0].length() * sign,
        cofactors[1].length() * sign,
        cofactors[2].length() * sign,
        x.length(),
    ]
}

fn shadow_proxy_resolved(radius_m: f64, max_deviation_m: f32) -> bool {
    radius_m * 2.0 / f64::from(SUN_SHADOW_RESOLUTION) >= f64::from(max_deviation_m) * 4.0
}

fn building_detail_visible(projected_px: f64, threshold_px: f64, was_visible: bool) -> bool {
    projected_px >= threshold_px * if was_visible { 0.88 } else { 1.0 }
}

fn landmark_aware_chunk_bounds(
    center: DVec3,
    radius: f64,
    landmark: bool,
    delta: Option<DMat4>,
) -> (DVec3, f64) {
    if landmark && let Some(delta) = delta {
        // Tagged landmarks share their chunk with untagged surroundings.
        // Include both positions for world visibility and shadow admission.
        let moved = delta.transform_point3(center);
        (
            (center + moved) * 0.5,
            radius + center.distance(moved) * 0.5,
        )
    } else {
        (center, radius)
    }
}

fn render_range_bounds(range: &RenderIndexRange, camera: DVec3) -> (DVec3, f64) {
    camera_relative_render_bounds(
        DVec3::from_array(range.bounds.center_ecef),
        range.bounds.radius_m,
        camera,
    )
}

fn camera_relative_render_bounds(center: DVec3, radius: f64, camera: DVec3) -> (DVec3, f64) {
    // Prepared bounds cover local model products. The renderer additionally
    // rounds camera-relative translation and projection to f32; that error
    // grows with distance even when a far-away range itself is very small.
    (
        center,
        radius + camera_relative_render_padding(center, radius, camera),
    )
}

fn camera_relative_render_padding(center: DVec3, radius: f64, camera: DVec3) -> f64 {
    (center.distance(camera) + radius) * f64::from(f32::EPSILON) * 8.0
}

impl ChunkGpu {
    fn admit_detail(&self, camera: DVec3, pixel_scale: f64) -> bool {
        let Some((center, radius, minimum)) = self.detail else {
            return true;
        };
        let pixels = 2.0 * radius * pixel_scale / center.distance(camera).max(1.0);
        let visible = building_detail_visible(pixels, minimum, self.detail_visible.get());
        self.detail_visible.set(visible);
        visible
    }
}

#[derive(Default)]
struct ResidentGpuSlot {
    generation: u32,
    cell: Option<CellGpu>,
}

/// Secondary generational storage for GPU allocations. Cell identity and
/// metadata remain in `ResidentStore`; this table can resolve only a live
/// `ResidentId`, so a stale draw cannot accidentally target a reused slot.
#[derive(Default)]
struct ResidentGpuStore {
    slots: Vec<ResidentGpuSlot>,
    len: usize,
    next_upload_generation: u64,
}

impl ResidentGpuStore {
    fn get(&self, id: ResidentId) -> Option<&CellGpu> {
        let slot = self.slots.get(id.slot())?;
        if slot.generation == id.generation() {
            slot.cell.as_ref()
        } else {
            None
        }
    }

    fn insert(&mut self, id: ResidentId, mut cell: CellGpu) -> Option<CellGpu> {
        self.next_upload_generation = self
            .next_upload_generation
            .checked_add(1)
            .expect("GPU upload generation exhausted");
        cell.upload_generation = self.next_upload_generation;
        if self.slots.len() <= id.slot() {
            self.slots
                .resize_with(id.slot() + 1, ResidentGpuSlot::default);
        }
        let slot = &mut self.slots[id.slot()];
        let old = slot.cell.take();
        debug_assert!(
            old.is_none() || slot.generation == id.generation(),
            "live GPU resident slot was reused without eviction"
        );
        slot.generation = id.generation();
        slot.cell = Some(cell);
        if old.is_none() {
            self.len += 1;
        }
        old
    }

    fn remove(&mut self, id: ResidentId) -> Option<CellGpu> {
        let slot = self.slots.get_mut(id.slot())?;
        if slot.generation != id.generation() {
            return None;
        }
        let old = slot.cell.take()?;
        self.len -= 1;
        Some(old)
    }

    fn contains(&self, id: ResidentId) -> bool {
        self.get(id).is_some()
    }

    fn len(&self) -> usize {
        self.len
    }
}

fn pack_indices_u16(indices: &[u32]) -> Option<Vec<u16>> {
    // Queue writes are four-byte aligned; retain u32 for odd-length index lists.
    indices
        .len()
        .is_multiple_of(2)
        .then(|| {
            indices
                .iter()
                .copied()
                .map(u16::try_from)
                .collect::<Result<Vec<_>, _>>()
                .ok()
        })
        .flatten()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ArenaAllocation {
    page: usize,
    vertex_offset: u64,
    vertex_bytes: u64,
    index_offset: u64,
    index_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FreeRange {
    offset: u64,
    size: u64,
}

#[derive(Clone, Debug)]
struct RangeAllocator {
    capacity: u64,
    free: Vec<FreeRange>,
}

impl RangeAllocator {
    fn new(size: u64) -> Self {
        Self {
            capacity: size,
            free: vec![FreeRange { offset: 0, size }],
        }
    }

    fn is_empty(&self) -> bool {
        self.free.as_slice()
            == [FreeRange {
                offset: 0,
                size: self.capacity,
            }]
    }

    fn allocate(&mut self, size: u64, alignment: u64) -> Option<u64> {
        let size = align_up(size, 4);
        for index in 0..self.free.len() {
            let range = self.free[index];
            let offset = align_up(range.offset, alignment);
            let end = offset.checked_add(size)?;
            if end > range.offset + range.size {
                continue;
            }
            let prefix = offset - range.offset;
            let suffix = range.offset + range.size - end;
            match (prefix > 0, suffix > 0) {
                (true, true) => {
                    self.free[index].size = prefix;
                    self.free.insert(
                        index + 1,
                        FreeRange {
                            offset: end,
                            size: suffix,
                        },
                    );
                }
                (true, false) => self.free[index].size = prefix,
                (false, true) => {
                    self.free[index] = FreeRange {
                        offset: end,
                        size: suffix,
                    }
                }
                (false, false) => {
                    self.free.remove(index);
                }
            }
            return Some(offset);
        }
        None
    }

    fn free(&mut self, offset: u64, size: u64) {
        let size = align_up(size, 4);
        let mut index = self.free.partition_point(|range| range.offset < offset);
        self.free.insert(index, FreeRange { offset, size });
        // Free ranges stay ordered and coalesced. Only the inserted range's
        // immediate neighbors can become adjacent after a release.
        if index > 0 && self.free[index - 1].offset + self.free[index - 1].size == offset {
            self.free[index - 1].size += size;
            self.free.remove(index);
            index -= 1;
        }
        if index + 1 < self.free.len()
            && self.free[index].offset + self.free[index].size == self.free[index + 1].offset
        {
            self.free[index].size += self.free[index + 1].size;
            self.free.remove(index + 1);
        }
    }
}

fn align_up(value: u64, alignment: u64) -> u64 {
    debug_assert!(alignment.is_power_of_two() || alignment.is_multiple_of(4));
    value.div_ceil(alignment) * alignment
}

struct BufferUploadStaging {
    device: wgpu::Device,
    belt: wgpu::util::StagingBelt,
    encoder: wgpu::CommandEncoder,
    cpu_write_ns: u64,
    polled_since_submit: bool,
    poll_failure_reported: bool,
}

impl BufferUploadStaging {
    fn new(device: &wgpu::Device) -> Self {
        Self {
            device: device.clone(),
            belt: wgpu::util::StagingBelt::new(device.clone(), STAGING_BELT_CHUNK_BYTES),
            encoder: Self::create_encoder(device),
            cpu_write_ns: 0,
            polled_since_submit: false,
            poll_failure_reported: false,
        }
    }

    fn create_encoder(device: &wgpu::Device) -> wgpu::CommandEncoder {
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("streaming buffer uploads"),
        })
    }

    fn write_buffer(&mut self, target: &wgpu::Buffer, offset: u64, data: &[u8]) {
        let started = Instant::now();
        if !self.polled_since_submit {
            if let Err(error) = self.device.poll(wgpu::PollType::Poll)
                && !self.poll_failure_reported
            {
                log::warn!("streaming upload staging poll failed: {error}");
                self.poll_failure_reported = true;
            }
            self.polled_since_submit = true;
        }
        let Some(size) =
            wgpu::BufferSize::new(align_up(data.len() as u64, wgpu::COPY_BUFFER_ALIGNMENT))
        else {
            return;
        };
        debug_assert_eq!(offset % wgpu::COPY_BUFFER_ALIGNMENT, 0);
        // An odd number of u16 triangles occupies 2 mod 4 bytes. Arena ranges
        // already reserve copy-aligned capacity; pad only the transfer, keeping
        // the index count and geometry unchanged.
        let mut destination = self
            .belt
            .write_buffer(&mut self.encoder, target, offset, size);
        destination.slice(..data.len()).copy_from_slice(data);
        destination
            .slice(data.len()..)
            .copy_from_slice(&[0; 4][..size.get() as usize - data.len()]);
        self.cpu_write_ns = self
            .cpu_write_ns
            .saturating_add(started.elapsed().as_nanos() as u64);
    }

    fn take_cpu_write_ns(&mut self) -> u64 {
        std::mem::take(&mut self.cpu_write_ns)
    }

    fn take_encoder(&mut self, device: &wgpu::Device) -> wgpu::CommandEncoder {
        std::mem::replace(&mut self.encoder, Self::create_encoder(device))
    }

    fn finish_and_recall_on_submit(&mut self, encoder: &wgpu::CommandEncoder) {
        self.belt.finish_and_recall_on_submit(encoder);
        self.polled_since_submit = false;
    }
}

// Resident allocations keep page indices until their last range is freed.
// Empty slots are reusable, but removing one page never moves another page.
// Dropping its buffers releases our ownership; command buffers retain their
// own references until GPU work completes. Do not call Buffer::destroy here.
struct ArenaPages<T> {
    slots: Vec<Option<(T, u64)>>,
    vacant: Vec<usize>,
    count: usize,
    capacity_bytes: u64,
}

impl<T> Default for ArenaPages<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            vacant: Vec::new(),
            count: 0,
            capacity_bytes: 0,
        }
    }
}

impl<T> ArenaPages<T> {
    fn insert(&mut self, page: T, capacity_bytes: u64) -> usize {
        let index = if let Some(index) = self.vacant.pop() {
            debug_assert!(self.slots[index].is_none());
            self.slots[index] = Some((page, capacity_bytes));
            index
        } else {
            let index = self.slots.len();
            self.slots.push(Some((page, capacity_bytes)));
            index
        };
        self.count += 1;
        self.capacity_bytes += capacity_bytes;
        index
    }

    fn remove(&mut self, index: usize) -> Option<T> {
        let (page, capacity_bytes) = self.slots.get_mut(index)?.take()?;
        self.vacant.push(index);
        self.count -= 1;
        self.capacity_bytes -= capacity_bytes;
        Some(page)
    }

    fn iter_mut(&mut self) -> impl Iterator<Item = (usize, &mut T)> {
        self.slots
            .iter_mut()
            .enumerate()
            .filter_map(|(index, slot)| slot.as_mut().map(|(page, _)| (index, page)))
    }

    fn len(&self) -> usize {
        self.count
    }
}

impl<T> std::ops::Index<usize> for ArenaPages<T> {
    type Output = T;
    fn index(&self, index: usize) -> &Self::Output {
        &self.slots[index]
            .as_ref()
            .expect("resident allocation requires a live GPU page")
            .0
    }
}

impl<T> std::ops::IndexMut<usize> for ArenaPages<T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.slots[index]
            .as_mut()
            .expect("resident allocation requires a live GPU page")
            .0
    }
}

fn index_stream_layout(world_bytes: u64, shadow_bytes: u64) -> (u64, u64) {
    let shadow_offset = align_up(world_bytes, wgpu::COPY_BUFFER_ALIGNMENT);
    let total = if shadow_bytes == 0 {
        world_bytes
    } else {
        shadow_offset + shadow_bytes
    };
    (shadow_offset, total)
}

struct MeshArenaPage {
    vertex: wgpu::Buffer,
    index: wgpu::Buffer,
    vertices: RangeAllocator,
    indices: RangeAllocator,
}

struct MeshArena {
    label: &'static str,
    vertex_stride: u64,
    pages: ArenaPages<MeshArenaPage>,
}

impl MeshArena {
    fn new(label: &'static str, vertex_stride: u64) -> Self {
        Self {
            label,
            vertex_stride,
            pages: ArenaPages::default(),
        }
    }

    fn allocate_and_upload(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut BufferUploadStaging,
        vertices: &[u8],
        indices: &[u8],
        shadow_indices: &[u8],
        index_alignment: u64,
    ) -> ArenaAllocation {
        let vertex_bytes = vertices.len() as u64;
        let (shadow_offset, index_bytes) =
            index_stream_layout(indices.len() as u64, shadow_indices.len() as u64);
        let allocation = self
            .try_allocate(vertex_bytes, index_bytes, index_alignment)
            .unwrap_or_else(|| {
                self.add_page(device, vertex_bytes, index_bytes);
                self.try_allocate(vertex_bytes, index_bytes, index_alignment)
                    .expect("new mesh arena page fits requested chunk")
            });
        let page = &self.pages[allocation.page];
        uploads.write_buffer(&page.vertex, allocation.vertex_offset, vertices);
        uploads.write_buffer(&page.index, allocation.index_offset, indices);
        if !shadow_indices.is_empty() {
            uploads.write_buffer(
                &page.index,
                allocation.index_offset + shadow_offset,
                shadow_indices,
            );
        }
        allocation
    }

    fn try_allocate(
        &mut self,
        vertex_bytes: u64,
        index_bytes: u64,
        index_alignment: u64,
    ) -> Option<ArenaAllocation> {
        for (page_index, page) in self.pages.iter_mut() {
            let Some(vertex_offset) = page.vertices.allocate(vertex_bytes, self.vertex_stride)
            else {
                continue;
            };
            if let Some(index_offset) = page.indices.allocate(index_bytes, index_alignment) {
                return Some(ArenaAllocation {
                    page: page_index,
                    vertex_offset,
                    vertex_bytes,
                    index_offset,
                    index_bytes,
                });
            }
            page.vertices.free(vertex_offset, vertex_bytes);
        }
        None
    }

    fn add_page(&mut self, device: &wgpu::Device, vertex_bytes: u64, index_bytes: u64) {
        let max = device.limits().max_buffer_size;
        let vertex_capacity = ARENA_VERTEX_PAGE_BYTES
            .max(align_up(vertex_bytes, self.vertex_stride))
            .min(max);
        let index_capacity = ARENA_INDEX_PAGE_BYTES
            .max(align_up(index_bytes, 4))
            .min(max);
        assert!(vertex_capacity >= vertex_bytes && index_capacity >= index_bytes);
        let vertex = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(self.label),
            size: vertex_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let index = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(self.label),
            size: index_capacity,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.pages.insert(
            MeshArenaPage {
                vertex,
                index,
                vertices: RangeAllocator::new(vertex_capacity),
                indices: RangeAllocator::new(index_capacity),
            },
            vertex_capacity + index_capacity,
        );
    }

    fn free(&mut self, allocation: ArenaAllocation) -> bool {
        let page = &mut self.pages[allocation.page];
        page.vertices
            .free(allocation.vertex_offset, allocation.vertex_bytes);
        page.indices
            .free(allocation.index_offset, allocation.index_bytes);
        if page.vertices.is_empty() && page.indices.is_empty() {
            drop(self.pages.remove(allocation.page));
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VertexAllocation {
    page: usize,
    offset: u64,
    bytes: u64,
}

struct VertexArenaPage {
    buffer: wgpu::Buffer,
    vertices: RangeAllocator,
}

struct VertexArena {
    label: &'static str,
    stride: u64,
    pages: ArenaPages<VertexArenaPage>,
}

impl VertexArena {
    fn new(label: &'static str, stride: u64) -> Self {
        Self {
            label,
            stride,
            pages: ArenaPages::default(),
        }
    }

    fn allocate_and_upload(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut BufferUploadStaging,
        vertices: &[u8],
    ) -> VertexAllocation {
        let bytes = vertices.len() as u64;
        let allocation = self.try_allocate(bytes).unwrap_or_else(|| {
            let capacity = LIGHT_ARENA_PAGE_BYTES
                .max(align_up(bytes, self.stride))
                .min(device.limits().max_buffer_size);
            assert!(capacity >= bytes);
            self.pages.insert(
                VertexArenaPage {
                    buffer: device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some(self.label),
                        size: capacity,
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    vertices: RangeAllocator::new(capacity),
                },
                capacity,
            );
            self.try_allocate(bytes)
                .expect("new vertex arena page fits requested allocation")
        });
        uploads.write_buffer(
            &self.pages[allocation.page].buffer,
            allocation.offset,
            vertices,
        );
        allocation
    }

    fn try_allocate(&mut self, bytes: u64) -> Option<VertexAllocation> {
        self.pages.iter_mut().find_map(|(page, arena)| {
            arena
                .vertices
                .allocate(bytes, self.stride)
                .map(|offset| VertexAllocation {
                    page,
                    offset,
                    bytes,
                })
        })
    }

    fn free(&mut self, allocation: VertexAllocation) -> bool {
        let page = &mut self.pages[allocation.page];
        page.vertices.free(allocation.offset, allocation.bytes);
        if page.vertices.is_empty() {
            drop(self.pages.remove(allocation.page));
            true
        } else {
            false
        }
    }
}

pub struct CellGpu {
    upload_generation: u64,
    horizon: vearth_core::lod::CellHorizon,
    pub chunks: Vec<ChunkGpu>,
    lights: Option<NightLightGpu>,
    trees: Vec<TreeInstanceGpu>,
    /// CPU-resident immutable descriptors used to build procedural flora draws.
    flora: Vec<FloraCell>,
    flora_frame: Option<FloraFrameGpu>,
}

#[derive(Clone, Copy)]
struct TreeInstanceGpu {
    profile: TreeProfileId,
    basis: DMat3,
    origin: DVec3,
    height_m: f32,
    color_scale: f32,
}

struct TreeAssetGpu {
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    lod_indices: [std::ops::Range<u32>; 3],
    index_format: wgpu::IndexFormat,
    height_m: f32,
    radius_m: f32,
    ready_at: Instant,
}

enum TreeAssetState {
    Pending,
    Ready(TreeAssetGpu),
}

struct BuiltTreeAsset {
    profile: TreeProfileId,
    vertices: Vec<SurfaceVertex>,
    indices: BuiltTreeIndices,
    lod_indices: [std::ops::Range<u32>; 3],
    height_m: f32,
    radius_m: f32,
}

enum BuiltTreeIndices {
    U16(Vec<u16>),
    U32(Vec<u32>),
}

fn build_tree_asset(profile: TreeProfileId) -> BuiltTreeAsset {
    let mut mesh = vearth_core::mesh::MeshBuffers::default();
    let mut height_m = 0.0_f32;
    let mut radius_m = 0.0_f32;
    let lod_indices = std::array::from_fn(|lod| {
        let asset = vearth_gen::tree_mesh::build_tree_mesh(profile, lod as u8);
        if lod == 0 {
            height_m = asset.height_m;
        }
        radius_m = radius_m.max(asset.radius_m);
        let first_vertex = mesh.vertices.len() as u32;
        let first_index = mesh.indices.len() as u32;
        mesh.vertices.extend(asset.mesh.vertices);
        mesh.indices
            .extend(asset.mesh.indices.into_iter().map(|i| i + first_vertex));
        first_index..mesh.indices.len() as u32
    });
    let indices = match pack_indices_u16(&mesh.indices) {
        Some(indices) => BuiltTreeIndices::U16(indices),
        None => BuiltTreeIndices::U32(mesh.indices),
    };
    BuiltTreeAsset {
        profile,
        vertices: mesh.vertices,
        indices,
        lod_indices,
        height_m,
        radius_m,
    }
}

struct NightLightGpu {
    allocation: VertexAllocation,
    vertex_count: u32,
    model: DMat4,
    axes: [DVec3; 3],
    bound_center: DVec3,
    bound_radius: f64,
}

fn night_light_cell_can_contribute(
    enabled: bool,
    lighting: Lighting,
    bound_center: DVec3,
    bound_radius: f64,
) -> bool {
    if !enabled {
        return false;
    }
    // Every proxy center lies inside this ECEF sphere. Bound its smallest
    // solar dot product before normalizing, using the largest possible radius
    // from Earth's center. The shader is exactly dark at sine >= 0.07; 0.08
    // leaves a conservative margin for camera-relative f32 transforms.
    // A cell crossing the terminator remains admitted even in a daytime view.
    let minimum_solar_projection = bound_center.dot(lighting.direction_ecef) - bound_radius;
    let maximum_geocentric_radius = bound_center.length() + bound_radius;
    minimum_solar_projection <= 0.08 * maximum_geocentric_radius
}

/// One pre-uploaded player or marker mesh. Player UVs use a separate stream
/// so the shared world/shadow position format retains its existing stride.
struct PlayerPartGpu {
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    uvbuf: Option<wgpu::Buffer>,
    vertex_bytes: u64,
    index_bytes: u64,
    index_count: u32,
}

/// Normalized flight inputs for continuously varying engine presentation.
#[derive(Clone, Copy)]
pub struct VehicleEffects {
    pub kind: crate::vehicles::VehicleKind,
    pub throttle: f32,
    pub boost: f32,
}

/// The player drawn this frame, using a rebased f64 model and normal frame.
pub struct PlayerDraw {
    pub model: DMat4,
    pub axes: [DVec3; 3],
    pub parts: Vec<usize>,
    pub effects: Option<VehicleEffects>,
}

/// GPU mesh residency. Byte totals are live mesh payloads inside the arenas;
/// buffer counts are physical arena pages plus standalone player/light
/// buffers. Opaque driver overhead and non-mesh resources are excluded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuMeshStats {
    pub world_cells: usize,
    pub world_chunks: usize,
    pub player_parts: usize,
    pub vertex_buffers: usize,
    pub index_buffers: usize,
    pub vertex_bytes: u64,
    pub index_bytes: u64,
    /// The share of `vertex_bytes + index_bytes` held by player-model parts.
    pub player_bytes: u64,
    /// Owned terrain/light arena buffer capacities, including free ranges.
    /// This excludes standalone assets and driver-retained in-flight buffers;
    /// it is allocation capacity rather than measured physical GPU memory.
    pub arena_capacity_bytes: u64,
    /// Visual descriptor residency is reported beside mesh residency but is
    /// CPU memory and deliberately excluded from `total_bytes()`.
    pub flora_cells: usize,
    pub flora_descriptor_bytes: u64,
}

impl GpuMeshStats {
    pub fn buffer_count(self) -> usize {
        self.vertex_buffers + self.index_buffers
    }

    pub fn total_bytes(self) -> u64 {
        self.vertex_bytes + self.index_bytes
    }

    fn add_world_cell(&mut self, footprint: MeshFootprint) {
        self.world_cells += 1;
        self.world_chunks += footprint.chunks;
        self.vertex_buffers += footprint.vertex_buffers;
        self.index_buffers += footprint.index_buffers;
        self.vertex_bytes += footprint.vertex_bytes;
        self.index_bytes += footprint.index_bytes;
        self.flora_cells += footprint.flora_cells;
        self.flora_descriptor_bytes += footprint.flora_descriptor_bytes;
    }

    fn remove_world_cell(&mut self, footprint: MeshFootprint) {
        self.world_cells -= 1;
        self.world_chunks -= footprint.chunks;
        self.vertex_buffers -= footprint.vertex_buffers;
        self.index_buffers -= footprint.index_buffers;
        self.vertex_bytes -= footprint.vertex_bytes;
        self.index_bytes -= footprint.index_bytes;
        self.flora_cells -= footprint.flora_cells;
        self.flora_descriptor_bytes -= footprint.flora_descriptor_bytes;
    }

    /// Bytes of world cells, which the resident index accounts for.
    pub fn world_bytes(self) -> u64 {
        self.total_bytes() - self.player_bytes
    }

    fn add_player_parts(&mut self, footprint: MeshFootprint) {
        self.player_bytes += footprint.total_bytes();
        self.player_parts += footprint.chunks;
        self.vertex_buffers += footprint.vertex_buffers;
        self.index_buffers += footprint.index_buffers;
        self.vertex_bytes += footprint.vertex_bytes;
        self.index_bytes += footprint.index_bytes;
    }

    fn remove_player_parts(&mut self, footprint: MeshFootprint) {
        self.player_bytes -= footprint.total_bytes();
        self.player_parts -= footprint.chunks;
        self.vertex_buffers -= footprint.vertex_buffers;
        self.index_buffers -= footprint.index_buffers;
        self.vertex_bytes -= footprint.vertex_bytes;
        self.index_bytes -= footprint.index_bytes;
    }
}

/// Mesh uploads admitted since the preceding render call. This makes burst
/// size visible to frame telemetry without changing `upload_cell`'s API.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuUploadStats {
    pub world_cells: usize,
    pub world_chunks: usize,
    pub player_parts: usize,
    pub vertex_buffers: usize,
    pub index_buffers: usize,
    pub vertex_bytes: u64,
    pub index_bytes: u64,
    /// CPU time spent packing/validating upload payloads and resident records.
    pub preparation_ns: u64,
    /// CPU enqueue/staging time spent in upload calls (not GPU execution).
    pub queue_upload_ns: u64,
}

impl GpuUploadStats {
    pub fn buffer_count(self) -> usize {
        self.vertex_buffers + self.index_buffers
    }

    pub fn total_bytes(self) -> u64 {
        self.vertex_bytes + self.index_bytes
    }

    fn record_world_cell(&mut self, footprint: MeshFootprint) {
        self.world_cells += 1;
        self.world_chunks += footprint.chunks;
        self.vertex_buffers += footprint.vertex_buffers;
        self.index_buffers += footprint.index_buffers;
        self.vertex_bytes += footprint.vertex_bytes;
        self.index_bytes += footprint.index_bytes;
    }

    fn record_player_parts(&mut self, footprint: MeshFootprint) {
        self.player_parts += footprint.chunks;
        self.vertex_buffers += footprint.vertex_buffers;
        self.index_buffers += footprint.index_buffers;
        self.vertex_bytes += footprint.vertex_bytes;
        self.index_bytes += footprint.index_bytes;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct MeshFootprint {
    chunks: usize,
    vertex_buffers: usize,
    index_buffers: usize,
    vertex_bytes: u64,
    index_bytes: u64,
    flora_cells: usize,
    flora_descriptor_bytes: u64,
}

impl MeshFootprint {
    fn total_bytes(self) -> u64 {
        self.vertex_bytes + self.index_bytes
    }

    fn from_chunks(chunks: &[ChunkGpu]) -> Self {
        MeshFootprint {
            chunks: chunks.len(),
            // Terrain allocations share arena pages; page buffer creation is
            // accounted separately when an arena grows.
            vertex_buffers: 0,
            index_buffers: 0,
            vertex_bytes: chunks.iter().map(|c| c.vertex_bytes).sum(),
            index_bytes: chunks.iter().map(|c| c.index_bytes).sum(),
            flora_cells: 0,
            flora_descriptor_bytes: 0,
        }
    }

    fn from_cell(cell: &CellGpu) -> Self {
        let mut footprint = Self::from_chunks(&cell.chunks);
        footprint.flora_cells = cell.flora.len();
        footprint.flora_descriptor_bytes =
            (cell.flora.capacity() * std::mem::size_of::<FloraCell>()) as u64;
        if let Some(lights) = &cell.lights {
            // Light allocations share arena pages; page creation is counted
            // separately when the arena grows.
            footprint.vertex_bytes += lights.allocation.bytes;
        }
        footprint
    }

    fn from_player_parts(parts: &[PlayerPartGpu]) -> Self {
        MeshFootprint {
            chunks: parts.len(),
            vertex_buffers: parts.len() + parts.iter().filter(|part| part.uvbuf.is_some()).count(),
            index_buffers: parts.len(),
            vertex_bytes: parts.iter().map(|p| p.vertex_bytes).sum(),
            index_bytes: parts.iter().map(|p| p.index_bytes).sum(),
            flora_cells: 0,
            flora_descriptor_bytes: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DrawAdmission {
    world: usize,
    player: usize,
    dropped_world: usize,
    dropped_player: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ArenaBatch {
    opaque: bool,
    rare_surfaces: bool,
    packed: bool,
    indices_u16: bool,
    landmark: bool,
    page: usize,
    command_start: usize,
    command_count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ShadowArenaBatch {
    cascade: usize,
    packed: bool,
    indices_u16: bool,
    page: usize,
    command_start: usize,
    command_count: u32,
}

/// Landmark pipeline, rare-surface shading, packed vertices, u16 indices and
/// arena page. One group shares a pipeline family and vertex/index buffers.
type WorldGroupKey = (bool, bool, bool, bool, usize);

#[derive(Clone, Copy)]
struct WorldDrawRange<'a> {
    chunk: &'a ChunkGpu,
    slot: u32,
    landmark_pipeline: bool,
    opaque_pipeline: bool,
    first_index: u32,
    index_count: u32,
    base_vertex: i32,
}

fn submitted_material_indices(draws: &[WorldDrawRange<'_>]) -> MaterialIndexCounts {
    let mut result = MaterialIndexCounts::default();
    for draw in draws {
        let end = draw.first_index + draw.index_count;
        let mut counted = 0u64;
        for range in &draw.chunk.material_ranges {
            if range.first_index >= draw.first_index && range.first_index + range.index_count <= end
            {
                for &(material, count) in &range.counts {
                    result.0[usize::from(material)] += u64::from(count);
                    counted += u64::from(count);
                }
            }
        }
        debug_assert_eq!(counted, u64::from(draw.index_count));
    }
    result
}

fn opaque_world_eligible(chunk: &ChunkGpu, fade: f32) -> bool {
    // Equality also excludes NaN/infinite fades; every transition retains
    // the original complementary screen-door coverage path.
    fade == 1.0 && !chunk.has_calibration_landmark && !chunk.has_fragment_feedback
}

fn append_world_draw_ranges<'a>(
    out: &mut Vec<WorldDrawRange<'a>>,
    chunk: &'a ChunkGpu,
    slot: u32,
    landmark_pipeline: bool,
    opaque_pipeline: bool,
    range_visible: impl FnMut(&RenderIndexRange) -> bool,
) {
    append_index_draw_ranges(
        out,
        chunk,
        slot,
        landmark_pipeline,
        opaque_pipeline,
        0,
        chunk.index_count,
        &chunk.ranges,
        range_visible,
    );
}

fn append_shadow_draw_ranges<'a>(
    out: &mut Vec<WorldDrawRange<'a>>,
    chunk: &'a ChunkGpu,
    slot: u32,
    cascade: &SunShadowCascade,
    range_visible: impl FnMut(&RenderIndexRange) -> bool,
) {
    if let Some(shadow) = &chunk.shadow
        && shadow_proxy_resolved(cascade.radius_m, shadow.max_deviation_m)
    {
        append_index_draw_ranges(
            out,
            chunk,
            slot,
            chunk.has_calibration_landmark,
            false,
            shadow.first_index,
            shadow.index_count,
            &shadow.ranges,
            range_visible,
        );
    } else {
        append_world_draw_ranges(
            out,
            chunk,
            slot,
            chunk.has_calibration_landmark,
            false,
            range_visible,
        );
    }
}

fn append_index_draw_ranges<'a>(
    out: &mut Vec<WorldDrawRange<'a>>,
    chunk: &'a ChunkGpu,
    slot: u32,
    landmark_pipeline: bool,
    opaque_pipeline: bool,
    first_index: u32,
    index_count: u32,
    ranges: &[RenderIndexRange],
    mut range_visible: impl FnMut(&RenderIndexRange) -> bool,
) {
    if index_count == 0 {
        return;
    }
    if ranges.is_empty() {
        out.push(WorldDrawRange {
            chunk,
            slot,
            landmark_pipeline,
            opaque_pipeline,
            first_index,
            index_count,
            base_vertex: 0,
        });
        return;
    }
    let first_draw = out.len();
    for range in ranges {
        if !range_visible(range) {
            continue;
        }
        let offset = first_index + range.first_index;
        // Neighboring visible spans can share one command. This preserves
        // index order and never bridges a rejected spatial range.
        if out.len() > first_draw
            && let Some(last) = out.last_mut()
            && last.first_index + last.index_count == offset
        {
            last.index_count += range.index_count;
        } else {
            out.push(WorldDrawRange {
                chunk,
                slot,
                landmark_pipeline,
                opaque_pipeline,
                first_index: offset,
                index_count: range.index_count,
                base_vertex: 0,
            });
        }
    }
}

fn build_world_indirect_batches(
    grouped: BTreeMap<WorldGroupKey, Vec<WorldDrawRange<'_>>>,
    command_capacity: usize,
    full_stride: u64,
    packed_stride: u64,
) -> (
    Vec<wgpu::util::DrawIndexedIndirectArgs>,
    Vec<ArenaBatch>,
    u64,
) {
    let mut commands = Vec::with_capacity(command_capacity);
    let mut batches: Vec<ArenaBatch> = Vec::with_capacity(grouped.len());
    let mut opaque_indices = 0;
    for ((landmark, rare_surfaces, packed, indices_u16, page), draws) in grouped {
        let first_batch = batches.len();
        let stride = if packed { packed_stride } else { full_stride };
        let index_stride = if indices_u16 { 2 } else { 4 };
        for draw in draws {
            let command_start = commands.len();
            commands.push(wgpu::util::DrawIndexedIndirectArgs {
                index_count: draw.index_count,
                instance_count: 1,
                first_index: (draw.chunk.allocation.index_offset / index_stride) as u32
                    + draw.first_index,
                base_vertex: (draw.chunk.allocation.vertex_offset / stride) as i32
                    + draw.base_vertex,
                first_instance: draw.slot,
            });
            opaque_indices += u64::from(draw.index_count) * u64::from(draw.opaque_pipeline);
            // Pipeline eligibility cannot reorder commands: only consecutive
            // runs inside the existing page/format group may be combined.
            if batches.len() > first_batch
                && let Some(last) = batches.last_mut()
                && last.opaque == draw.opaque_pipeline
            {
                last.command_count += 1;
            } else {
                batches.push(ArenaBatch {
                    opaque: draw.opaque_pipeline,
                    rare_surfaces,
                    packed,
                    indices_u16,
                    landmark,
                    page,
                    command_start,
                    command_count: 1,
                });
            }
        }
    }
    (commands, batches, opaque_indices)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LightArenaBatch {
    page: usize,
    command_start: usize,
    command_count: u32,
}

/// Reserve scarce draw slots for the on-screen player, then fill the rest
/// with already-priority-ordered world chunks. Kept pure for boundary tests.
fn admit_draws(visible_world: usize, valid_player: usize) -> DrawAdmission {
    let player = valid_player.min(MAX_DRAWS);
    let world = visible_world.min(MAX_DRAWS - player);
    DrawAdmission {
        world,
        player,
        dropped_world: visible_world - world,
        dropped_player: valid_player - player,
    }
}

/// Per-bucket partition of unity for additive light LODs. A descendant's
/// transition alpha removes the same share from the ancestor bucket that
/// contains it. The descendant then contributes that share using its finer
/// distribution, avoiding the brightness surge a normal overlapping
/// additive crossfade would produce.
fn effective_light_fades(cell: CellId, alpha: f32, draws: &HashMap<CellId, f32>) -> [[f32; 4]; 4] {
    let mut remaining = [[1.0f32; 4]; 4];
    for (bz, row) in remaining.iter_mut().enumerate() {
        for (bx, value) in row.iter_mut().enumerate() {
            for shift in 1..=crate::streaming::MAX_STREAM_LEVEL.saturating_sub(cell.level) {
                let scale = 1u32 << shift;
                let ri = (((2 * bx + 1) as u32 * scale) / 8).min(scale - 1);
                let rz = (((2 * bz + 1) as u32 * scale) / 8).min(scale - 1);
                // Cell j increases with cube-face +v, while ChunkFrame local
                // +z is -v. Flip the sampled descendant row.
                let rj = scale - 1 - rz;
                let descendant = CellId {
                    face: cell.face,
                    level: cell.level + shift,
                    i: cell.i * scale + ri,
                    j: cell.j * scale + rj,
                };
                if let Some(descendant_alpha) = draws.get(&descendant) {
                    *value *= 1.0 - descendant_alpha.abs().clamp(0.0, 1.0);
                }
            }
        }
    }
    let alpha = alpha.abs().clamp(0.0, 1.0);
    remaining.map(|row| row.map(|uncovered| alpha * uncovered))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GpuTimings {
    pub(crate) total_ms: f64,
    pub(crate) shadow_ms: f64,
    pub(crate) world_ms: f64,
    pub(crate) flora_ms: Option<f64>,
    pub(crate) atmosphere_ms: f64,
    pub(crate) cloud_ms: f64,
    pub(crate) precipitation_ms: f64,
    pub(crate) tonemap_ms: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GpuTimingSample {
    pub(crate) id: u64,
    pub(crate) timings: GpuTimings,
}

/// Every decoded slot from one nonblocking collection. The readback ring is
/// the capacity bound; batches are returned with the render that collects them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct GpuTimingBatch {
    samples: [Option<GpuTimingSample>; GPU_TIMING_RING_SIZE],
}

impl GpuTimingBatch {
    pub(crate) fn iter(&self) -> impl Iterator<Item = GpuTimingSample> + '_ {
        self.samples.iter().flatten().copied()
    }

    fn push(&mut self, sample: GpuTimingSample) {
        *self
            .samples
            .iter_mut()
            .find(|entry| entry.is_none())
            .expect("one collection cannot exceed the readback ring") = Some(sample);
    }
}

// Replay needs every valid completion, including IDs older than the HUD's
// latest value. Keep those delivery and presentation decisions independent.
fn accept_gpu_timing(
    latest_frame_id: &mut u64,
    latest: &mut Option<GpuTimings>,
    completed: Option<&mut GpuTimingBatch>,
    frame_id: u64,
    ticks: [u64; GPU_QUERY_COUNT as usize],
    period_ns: f64,
) {
    let newer = frame_id > *latest_frame_id;
    if !newer && completed.is_none() {
        return;
    }
    let Some(timings) = decode_gpu_timings(ticks, period_ns) else {
        return;
    };
    if newer {
        *latest_frame_id = frame_id;
        *latest = Some(timings);
    }
    if let Some(completed) = completed {
        completed.push(GpuTimingSample {
            id: frame_id,
            timings,
        });
    }
}

// Pass-boundary intervals are latency, not additive exclusive GPU time. Metal
// measures render passes from vertex start to fragment end; those stages can
// overlap adjacent passes. Only total_ms spans the frame's critical path.
// Reject reversed timestamps after a device reset or malformed readback.
fn decode_gpu_timings(
    ticks: [u64; GPU_QUERY_COUNT as usize],
    period_ns: f64,
) -> Option<GpuTimings> {
    if !period_ns.is_finite() || period_ns <= 0.0 {
        return None;
    }
    let shadow_ticks =
        ticks[SHADOW_END_QUERY as usize].checked_sub(ticks[SHADOW_BEGIN_QUERY as usize])?;
    let world_ticks =
        ticks[WORLD_END_QUERY as usize].checked_sub(ticks[WORLD_BEGIN_QUERY as usize])?;
    let atmosphere_ticks =
        ticks[ATMOSPHERE_END_QUERY as usize].checked_sub(ticks[ATMOSPHERE_BEGIN_QUERY as usize])?;
    let cloud_ticks =
        ticks[CLOUD_END_QUERY as usize].checked_sub(ticks[CLOUD_BEGIN_QUERY as usize])?;
    let precipitation_ticks = ticks[PRECIPITATION_END_QUERY as usize]
        .checked_sub(ticks[PRECIPITATION_BEGIN_QUERY as usize])?;
    let tone_ticks =
        ticks[TONEMAP_END_QUERY as usize].checked_sub(ticks[TONEMAP_BEGIN_QUERY as usize])?;
    let total_ticks =
        ticks[TONEMAP_END_QUERY as usize].checked_sub(ticks[SHADOW_BEGIN_QUERY as usize])?;
    let to_ms = |duration: u64| duration as f64 * period_ns * 1e-6;
    Some(GpuTimings {
        total_ms: to_ms(total_ticks),
        shadow_ms: to_ms(shadow_ticks),
        world_ms: to_ms(world_ticks),
        // Apple GPUs expose pass boundaries, so flora sharing the world pass
        // has no independently measurable interval.
        flora_ms: None,
        atmosphere_ms: to_ms(atmosphere_ticks),
        cloud_ms: to_ms(cloud_ticks),
        precipitation_ms: to_ms(precipitation_ticks),
        tonemap_ms: to_ms(tone_ticks),
    })
}

/// Timestamp queries are useful only when both the feature and queue-timeline
/// query resolution are available. The latter keeps collection from stalling
/// the device's CPU timeline on downlevel adapters.
fn timestamp_query_feature(
    adapter_features: wgpu::Features,
    downlevel_flags: wgpu::DownlevelFlags,
) -> wgpu::Features {
    if adapter_features.contains(wgpu::Features::TIMESTAMP_QUERY)
        && downlevel_flags.contains(wgpu::DownlevelFlags::NONBLOCKING_QUERY_RESOLVE)
    {
        wgpu::Features::TIMESTAMP_QUERY
    } else {
        wgpu::Features::empty()
    }
}

pub(crate) fn renderer_features(
    adapter_features: wgpu::Features,
    downlevel_flags: wgpu::DownlevelFlags,
) -> wgpu::Features {
    let indirect = if downlevel_flags.contains(wgpu::DownlevelFlags::INDIRECT_EXECUTION) {
        adapter_features & wgpu::Features::INDIRECT_FIRST_INSTANCE
    } else {
        wgpu::Features::empty()
    };
    let world_half = if WorldInterface::from_env() == WorldInterface::Half {
        adapter_features & wgpu::Features::SHADER_F16
    } else {
        wgpu::Features::empty()
    };
    // WebGPU guarantees one and four samples; two needs adapter support.
    let world_samples = if crate::world_aa::requested_samples() == 2 {
        adapter_features & wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
    } else {
        wgpu::Features::empty()
    };
    timestamp_query_feature(adapter_features, downlevel_flags)
        | indirect
        | world_half
        | world_samples
}

fn surface_primitive_state(cull_backfaces: bool) -> wgpu::PrimitiveState {
    wgpu::PrimitiveState {
        topology: wgpu::PrimitiveTopology::TriangleList,
        front_face: wgpu::FrontFace::Ccw,
        cull_mode: cull_backfaces.then_some(wgpu::Face::Back),
        ..Default::default()
    }
}

struct GpuTimingSlot {
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    state: Arc<AtomicU8>,
    frame_id: u64,
}

/// A small asynchronous readback ring. A busy ring drops instrumentation for
/// that frame rather than making frame submission wait for the GPU.
struct GpuTiming {
    query_set: wgpu::QuerySet,
    slots: Vec<GpuTimingSlot>,
    timestamp_period_ns: f64,
    next_slot: usize,
    next_frame_id: u64,
    latest_frame_id: u64,
    latest: Option<GpuTimings>,
    capture_completed: bool,
    completed: Option<GpuTimingBatch>,
    map_failure_reported: bool,
    poll_failure_reported: bool,
}

impl GpuTiming {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }

        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("frame timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: GPU_QUERY_COUNT,
        });
        let slots = (0..GPU_TIMING_RING_SIZE)
            .map(|_| GpuTimingSlot {
                resolve: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("frame timestamp resolve"),
                    size: GPU_QUERY_BYTES,
                    usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                }),
                readback: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("frame timestamp readback"),
                    size: GPU_QUERY_BYTES,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                state: Arc::new(AtomicU8::new(TIMING_IDLE)),
                frame_id: 0,
            })
            .collect();
        Some(GpuTiming {
            query_set,
            slots,
            timestamp_period_ns: queue.get_timestamp_period() as f64,
            next_slot: 0,
            next_frame_id: 1,
            latest_frame_id: 0,
            latest: None,
            capture_completed: false,
            completed: None,
            map_failure_reported: false,
            poll_failure_reported: false,
        })
    }

    fn prepare_frame(&mut self, device: &wgpu::Device) -> Option<usize> {
        if let Err(error) = device.poll(wgpu::PollType::Poll)
            && !self.poll_failure_reported
        {
            log::warn!("non-blocking GPU timestamp poll failed: {error}");
            self.poll_failure_reported = true;
        }
        self.completed = self.collect_ready();

        for offset in 0..self.slots.len() {
            let index = (self.next_slot + offset) % self.slots.len();
            let slot = &mut self.slots[index];
            if slot
                .state
                .compare_exchange(
                    TIMING_IDLE,
                    TIMING_PENDING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                slot.frame_id = self.next_frame_id;
                self.next_frame_id += 1;
                self.next_slot = (index + 1) % self.slots.len();
                return Some(index);
            }
        }
        None
    }

    fn collect_ready(&mut self) -> Option<GpuTimingBatch> {
        let mut completed = self.capture_completed.then(GpuTimingBatch::default);
        for slot in &self.slots {
            match slot.state.load(Ordering::Acquire) {
                TIMING_READY => {
                    let mapped = match slot.readback.slice(..).get_mapped_range() {
                        Ok(mapped) => mapped,
                        Err(error) => {
                            slot.readback.unmap();
                            slot.state.store(TIMING_IDLE, Ordering::Release);
                            if !self.map_failure_reported {
                                log::warn!(
                                    "GPU timestamp readback range failed ({error}); later frames will retry"
                                );
                                self.map_failure_reported = true;
                            }
                            continue;
                        }
                    };
                    let mut ticks = [0u64; GPU_QUERY_COUNT as usize];
                    for (tick, bytes) in ticks.iter_mut().zip(mapped.chunks_exact(8)) {
                        *tick = u64::from_le_bytes(
                            bytes
                                .try_into()
                                .expect("timestamp chunks have exactly eight bytes"),
                        );
                    }
                    drop(mapped);
                    slot.readback.unmap();
                    accept_gpu_timing(
                        &mut self.latest_frame_id,
                        &mut self.latest,
                        completed.as_mut(),
                        slot.frame_id,
                        ticks,
                        self.timestamp_period_ns,
                    );
                    slot.state.store(TIMING_IDLE, Ordering::Release);
                }
                TIMING_FAILED => {
                    slot.readback.unmap();
                    slot.state.store(TIMING_IDLE, Ordering::Release);
                    if !self.map_failure_reported {
                        log::warn!("GPU timestamp readback failed; later frames will retry");
                        self.map_failure_reported = true;
                    }
                }
                _ => {}
            }
        }
        completed
    }

    fn schedule_readback(&self, encoder: &wgpu::CommandEncoder, slot_index: usize) {
        let slot = &self.slots[slot_index];
        let state = Arc::clone(&slot.state);
        encoder.map_buffer_on_submit(&slot.readback, wgpu::MapMode::Read, .., move |result| {
            state.store(
                if result.is_ok() {
                    TIMING_READY
                } else {
                    TIMING_FAILED
                },
                Ordering::Release,
            );
        });
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FloraSuppressionReason {
    #[default]
    None,
    GlyphBudget,
    TriangleBudget,
    InstanceUploadBudget,
}

impl FloraSuppressionReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::GlyphBudget => "glyph_budget",
            Self::TriangleBudget => "triangle_budget",
            Self::InstanceUploadBudget => "instance_upload_budget",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FloraRendererStats {
    /// CPU-resident descriptors, including cells outside the visible cut.
    pub resident_cells: usize,
    /// Unique descriptors surviving terrain-cut, horizon, frustum, and the
    /// 1.5 km geometry handoff, before hard-budget fallback.
    pub visible_cells: usize,
    pub visible_by_lod: [usize; FloraLod::COUNT],
    pub visible_by_archetype: [usize; vearth_gen::flora::FloraArchetype::COUNT],
    pub visible_by_calendar: [usize; vearth_gen::flora::CultivationCalendar::COUNT],
    pub visible_by_crop: [usize; vearth_gen::agriculture_catalog::CROP_TREATMENTS.len()],
    pub visible_by_registry: [usize; vearth_gen::flower_destinations::FLOWER_DESTINATION_COUNT],
    pub inferred_by_archetype: [usize; vearth_gen::flora::FloraArchetype::COUNT],
    /// Candidate slots submitted. Coverage and density can collapse a slot in
    /// the shader, but GPU work can never exceed this conservative count.
    pub glyphs: u32,
    pub glyphs_by_lod: [u32; FloraLod::COUNT],
    pub glyph_budget: u32,
    pub triangles: u64,
    pub triangles_by_lod: [u64; FloraLod::COUNT],
    /// Expected occupied candidates after descriptor coverage and authored
    /// density, before seasonal shader suppression.
    pub estimated_occupied_glyphs: f64,
    /// Expected rendered flower-head solids. Close/middle commercial glyphs
    /// intentionally carry two heads; aggregate LODs carry one volume.
    pub estimated_flower_heads: f64,
    /// Expected occupied annual-crop glyphs after coverage and authored
    /// density. Each glyph is one bounded procedural cluster.
    pub estimated_crop_clusters: f64,
    /// Conceptual plants represented by those clusters. This does not create
    /// CPU objects or GPU instances; it makes crop-density changes observable.
    pub estimated_crop_plant_equivalents: f64,
    pub nearest_visible_m: f64,
    pub farthest_visible_m: f64,
    pub draw_calls: usize,
    pub admitted_instances: usize,
    pub instance_upload_bytes: usize,
    pub instance_upload_budget_bytes: usize,
    pub cells_suppressed_by_glyph_budget: usize,
    pub cells_suppressed_by_triangle_budget: usize,
    pub cells_suppressed_by_upload_budget: usize,
    pub suppression_reason: FloraSuppressionReason,
    pub curated_destination_matches: usize,
    /// CPU time for culling, deterministic budget admission, packing, and
    /// topology/LOD grouping.
    pub cpu_preparation_ms: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct FrameStats {
    /// Requested draw cells supplied by the streaming controller.
    pub requested_cells: usize,
    /// Requested cells present in GPU residency.
    pub resident_draw_cells: usize,
    /// Requested cells absent from GPU residency.
    pub missing_draw_cells: usize,
    /// Resident chunks considered before horizon/frustum culling.
    pub candidate_chunks: usize,
    /// Chunks surviving horizon and frustum culling, before capacity admission.
    pub visible_chunks: usize,
    /// World chunks submitted to the GPU.
    pub submitted_chunks: usize,
    /// Exact index and emitted-vertex populations in submitted world meshes.
    pub submitted_world_indices: u64,
    /// Optional material census of these exact admitted world index ranges.
    pub world_material_indices: Option<MaterialIndexCounts>,
    /// Submitted world indices by fragment pipeline; their sum is the total.
    pub opaque_world_indices: u64,
    pub feedback_world_indices: u64,
    pub submitted_world_vertices: u64,
    /// Contiguous pipeline runs within arena-page/vertex-format groups.
    /// With indirect support, each run is one CPU render-pass command.
    pub submitted_world_batches: usize,
    pub indirect_world_batches: bool,
    /// World chunks using the rare mixed landmark path and lossless u16
    /// index path respectively.
    pub submitted_landmark_chunks: usize,
    pub submitted_u16_index_chunks: usize,
    /// Exact ordinary + landmark per-draw bytes uploaded this frame.
    pub world_draw_upload_bytes: usize,
    /// Visible world chunks explicitly omitted because `MAX_DRAWS` was full.
    pub dropped_chunks: usize,
    /// Player parts omitted because `MAX_DRAWS` was full.
    pub dropped_player_parts: usize,
    /// Player-part indices that did not name an uploaded non-empty mesh.
    pub invalid_player_parts: usize,
    /// Uploaded non-empty player parts submitted this frame.
    pub submitted_player_parts: usize,
    /// Distinct physical trees admitted to the fine renderer. A tree with
    /// two complementary geometry LOD submissions is counted once.
    pub submitted_tree_instances: usize,
    /// Draw calls used to submit all admitted tree LOD representations.
    pub submitted_tree_batches: usize,
    pub tree_instancing_active: bool,
    /// Exact bytes written to the dedicated per-tree storage buffer.
    pub tree_instance_upload_bytes: usize,
    /// Independent night-light cell draws submitted/dropped at their own
    /// capacity boundary. A non-zero drop is a failed visual-continuity
    /// invariant even when the opaque world remains within its draw cap.
    pub submitted_light_cells: usize,
    pub submitted_light_batches: usize,
    pub dropped_light_cells: usize,
    /// Per-draw storage slots available to world chunks + player parts.
    pub draw_capacity: usize,
    /// Cumulative explicitly dropped draws since renderer creation.
    pub draw_overflow_total: u64,
    /// Whether the cached atmosphere depth bind group was rebuilt this frame.
    pub atmo_bind_group_rebuilt: bool,
    /// Exact logical bytes retained by the half-resolution cloud history.
    pub cloud_working_set_bytes: u64,
    /// Bounded directional-Sun shadow state and exact submitted population.
    pub sun_shadow_configured: bool,
    pub sun_shadow_active: bool,
    pub sun_shadow_cascades: usize,
    pub sun_shadow_updated_cascades: usize,
    pub sun_shadow_caster_chunks: usize,
    pub sun_shadow_caster_chunks_by_cascade: [usize; SUN_SHADOW_CASCADES],
    pub sun_shadow_caster_indices: u64,
    pub sun_shadow_caster_indices_by_cascade: [u64; SUN_SHADOW_CASCADES],
    pub sun_shadow_batches: usize,
    pub sun_shadow_capacity_drops: usize,
    pub sun_shadow_working_set_bytes: u64,
    pub cpu_sun_shadow_preparation_ms: f64,
    /// Monotonic id of the asynchronously collected GPU timestamp sample.
    /// Consumers can discard repeated values when rendering faster than the
    /// readback ring advances.
    pub gpu_timing_sample_id: Option<u64>,
    /// All valid completions collected by this render when explicitly enabled.
    /// Unlike the latest HUD sample, these IDs may arrive out of order.
    pub(crate) gpu_timing_completed: Option<GpuTimingBatch>,
    /// Timing slot issued for this render. Join a future completed sample with
    /// this ID; neither ID is a presentation or replay frame number.
    pub gpu_timing_issued_id: Option<u64>,
    /// Latest asynchronously collected GPU duration spanning world,
    /// atmosphere, and tone-map passes. `None` means timestamps are
    /// unsupported or not ready yet.
    pub gpu_total_ms: Option<f64>,
    /// Latest asynchronously collected Sun-shadow depth-pass duration.
    pub gpu_shadow_ms: Option<f64>,
    /// Latest asynchronously collected sky preparation and combined world/flora duration.
    pub gpu_world_ms: Option<f64>,
    /// Unavailable independently: procedural flora is included in `gpu_world_ms`.
    pub gpu_flora_ms: Option<f64>,
    /// Latest asynchronously collected atmosphere render-pass duration.
    pub gpu_atmosphere_ms: Option<f64>,
    /// Latest asynchronously collected global cloud-shell duration.
    pub gpu_cloud_ms: Option<f64>,
    /// Latest asynchronously collected near-precipitation duration.
    pub gpu_precipitation_ms: Option<f64>,
    /// Latest asynchronously collected tone-map, grading, and saturation duration.
    pub gpu_tonemap_ms: Option<f64>,
    /// Mesh uploads since the preceding render call.
    pub uploads: GpuUploadStats,
    /// Exact mesh-buffer residency at submission time.
    pub mesh: GpuMeshStats,
    pub flora: FloraRendererStats,
    /// Independently measured render-thread preparation stages.
    pub cpu_visibility_ms: f64,
    pub cpu_world_draw_construction_ms: f64,
    pub cpu_sorting_ms: f64,
    pub cpu_uniform_construction_ms: f64,
    pub cpu_grouping_ms: f64,
    pub cpu_indirect_construction_ms: f64,
    pub upload_preparation_ms: f64,
    pub queue_upload_ms: f64,
    /// Legacy alias for `submitted_chunks`.
    pub draws: usize,
    pub culled: usize,
    /// Chunks skipped by the draw-time behind-the-planet test.
    pub culled_horizon: usize,
    /// Submitted ordinary and landmark surface triangles, excluding trees,
    /// flora, sky, player parts and effects.
    pub world_triangles: u64,
    /// Submitted tree geometry, including both representations during a fade.
    pub tree_triangles: u64,
    pub triangles: u64,
}

/// Cached full-resolution linear-light target and its tone-map binding.
/// Rebuilt when render dimensions or the caller's depth attachment change.
struct HdrTarget {
    multisample: Option<crate::world_aa::Targets>,
    width: u32,
    height: u32,
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    tone_bg: wgpu::BindGroup,
    depth: wgpu::TextureView,
}

struct SunShadowResources {
    _texture: wgpu::Texture,
    layer_views: [wgpu::TextureView; SUN_SHADOW_CASCADES],
    _timing_texture: wgpu::Texture,
    timing_view: wgpu::TextureView,
    receiver_buf: wgpu::Buffer,
    receiver_bg: wgpu::BindGroup,
    pass_bufs: [wgpu::Buffer; SUN_SHADOW_CASCADES],
    pass_bgs: [wgpu::BindGroup; SUN_SHADOW_CASCADES],
    model_buf: wgpu::Buffer,
    model_bg: wgpu::BindGroup,
    full_pipeline: wgpu::RenderPipeline,
    player_pipeline: wgpu::RenderPipeline,
    packed_pipeline: wgpu::RenderPipeline,
    indirect_buf: wgpu::Buffer,
    configured: bool,
    working_set_bytes: u64,
    /// Stable regional origin for light-space texel snapping. It is rebased
    /// only after regional travel/teleport, never continuously with the camera.
    snap_anchors_ecef: [Option<DVec3>; SUN_SHADOW_CASCADES],
    rendered_cascades: [Option<SunShadowCascade>; SUN_SHADOW_CASCADES],
    receiver_culling: bool,
    rendered_receiver_support: [Option<ReceiverSupport>; SUN_SHADOW_CASCADES],
    receiver_recovery: [Option<SunShadowReceiverRecovery>; SUN_SHADOW_CASCADES],
    caster_populations: [SunShadowCasterPopulation; SUN_SHADOW_CASCADES],
    candidate_scratch: Vec<SunShadowCandidate>,
    update_frame: u64,
    active_last_frame: bool,
}

fn create_sun_shadow_receiver_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Sun shadow receiver"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(
                        std::mem::size_of::<SunShadowU>() as u64
                    ),
                },
                count: None,
            },
        ],
    })
}

fn create_sun_shadow_resources(
    device: &wgpu::Device,
    receiver_bgl: &wgpu::BindGroupLayout,
    configured: bool,
) -> SunShadowResources {
    let texture_resolution = if configured { SUN_SHADOW_RESOLUTION } else { 1 };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("two-cascade Sun shadow depth array"),
        size: wgpu::Extent3d {
            width: texture_resolution,
            height: texture_resolution,
            depth_or_array_layers: SUN_SHADOW_CASCADES as u32,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SUN_SHADOW_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let array_view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("Sun shadow receiver array"),
        format: None,
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        usage: None,
        aspect: wgpu::TextureAspect::DepthOnly,
        base_mip_level: 0,
        mip_level_count: Some(1),
        base_array_layer: 0,
        array_layer_count: Some(SUN_SHADOW_CASCADES as u32),
    });
    let layer_views = std::array::from_fn(|index| {
        texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("Sun shadow cascade layer"),
            format: None,
            dimension: Some(wgpu::TextureViewDimension::D2),
            usage: None,
            aspect: wgpu::TextureAspect::DepthOnly,
            base_mip_level: 0,
            mip_level_count: Some(1),
            base_array_layer: index as u32,
            array_layer_count: Some(1),
        })
    });
    // Timestamp query pairs need a render pass even on frames where no
    // cascade updates. Keep that pass isolated from both cached depth layers
    // and full-resolution HDR targets.
    let timing_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Sun shadow timing sentinel"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let timing_view = timing_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Sun shadow comparison sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        compare: Some(wgpu::CompareFunction::LessEqual),
        ..Default::default()
    });
    let inactive_receiver = SunShadowU {
        light_view_proj_0: DMat4::IDENTITY.to_cols_array().map(|value| value as f32),
        light_view_proj_1: DMat4::IDENTITY.to_cols_array().map(|value| value as f32),
        splits: [0.0; 4],
        tuning: [1.0 / SUN_SHADOW_RESOLUTION as f32, 0.0, 0.0, 0.0],
    };
    let receiver_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Sun shadow receiver uniform"),
        contents: bytemuck::bytes_of(&inactive_receiver),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let receiver_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Sun shadow receiver"),
        layout: receiver_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&array_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: receiver_buf.as_entire_binding(),
            },
        ],
    });
    let pass_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Sun shadow pass"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<ShadowPassU>() as u64),
            },
            count: None,
        }],
    });
    let model_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Sun shadow models"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<ShadowModelU>() as u64),
            },
            count: None,
        }],
    });
    let identity_pass = ShadowPassU {
        light_view_proj: DMat4::IDENTITY.to_cols_array().map(|value| value as f32),
    };
    let pass_bufs = std::array::from_fn(|_| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Sun shadow cascade pass uniform"),
            contents: bytemuck::bytes_of(&identity_pass),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    });
    let pass_bgs = std::array::from_fn(|index| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Sun shadow cascade pass"),
            layout: &pass_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: pass_bufs[index].as_entire_binding(),
            }],
        })
    });
    let model_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Sun shadow model transforms"),
        size: (MAX_SHADOW_MODELS * std::mem::size_of::<ShadowModelU>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let model_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Sun shadow model transforms"),
        layout: &model_bgl,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: model_buf.as_entire_binding(),
        }],
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Sun shadow depth"),
        source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("shadow.wgsl"))),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Sun shadow depth"),
        bind_group_layouts: &[Some(&pass_bgl), Some(&model_bgl)],
        immediate_size: 0,
    });
    let depth_stencil = wgpu::DepthStencilState {
        format: SUN_SHADOW_FORMAT,
        depth_write_enabled: Some(true),
        depth_compare: Some(wgpu::CompareFunction::LessEqual),
        stencil: Default::default(),
        bias: wgpu::DepthBiasState {
            constant: 2,
            slope_scale: 2.0,
            clamp: 0.0,
        },
    };
    let full_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("full-float Sun shadow depth"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_full"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<SurfaceVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Uint32],
            })],
        },
        fragment: None,
        primitive: surface_primitive_state(true),
        depth_stencil: Some(depth_stencil.clone()),
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    let player_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("two-sided player Sun shadow depth"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_full"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<SurfaceVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Uint32],
            })],
        },
        fragment: None,
        primitive: surface_primitive_state(false),
        depth_stencil: Some(depth_stencil.clone()),
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    let packed_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("packed Sun shadow depth"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_packed"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<PackedSurfaceVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Uint32, 1 => Uint32],
            })],
        },
        fragment: None,
        primitive: surface_primitive_state(true),
        depth_stencil: Some(depth_stencil),
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    let indirect_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Sun shadow indirect draws"),
        size: (MAX_SHADOW_COMMANDS * std::mem::size_of::<wgpu::util::DrawIndexedIndirectArgs>())
            as u64,
        usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    SunShadowResources {
        _texture: texture,
        layer_views,
        _timing_texture: timing_texture,
        timing_view,
        receiver_buf,
        receiver_bg,
        pass_bufs,
        pass_bgs,
        model_buf,
        model_bg,
        full_pipeline,
        player_pipeline,
        packed_pipeline,
        indirect_buf,
        configured,
        working_set_bytes: if configured {
            SUN_SHADOW_WORKING_SET_BYTES
        } else {
            SUN_SHADOW_CASCADES as u64 * SUN_SHADOW_TEXEL_BYTES
        },
        snap_anchors_ecef: [None; SUN_SHADOW_CASCADES],
        rendered_cascades: [None; SUN_SHADOW_CASCADES],
        receiver_culling: vearth_platform::env::var("VEARTH_SHADOW_RECEIVER_CULL")
            .map_or(true, |value| value == "1"),
        rendered_receiver_support: [None; SUN_SHADOW_CASCADES],
        receiver_recovery: [None; SUN_SHADOW_CASCADES],
        caster_populations: std::array::from_fn(|_| SunShadowCasterPopulation::default()),
        candidate_scratch: Vec::new(),
        update_frame: 0,
        active_last_frame: false,
    }
}

struct CloudBuffer {
    _color_texture: wgpu::Texture,
    color_view: wgpu::TextureView,
    _meta_texture: wgpu::Texture,
    meta_view: wgpu::TextureView,
    sample_bg: wgpu::BindGroup,
}

/// Ping-ponged half-resolution cloud radiance plus logarithmic depth metadata.
/// The sampled buffer is the preceding temporally resolved frame; the other
/// buffer receives the current integration and becomes history after submit.
struct CloudTargets {
    width: u32,
    height: u32,
    buffers: [CloudBuffer; 2],
    update_width: u32,
    update_height: u32,
    update: CloudBuffer,
    history_index: usize,
}

#[derive(Clone, Copy)]
struct CloudHistoryState {
    camera_ecef: DVec3,
    view_proj: DMat4,
    weather_time: f32,
    lightning_flash: f32,
    // The previous frame's inputs. A whole reset threshold crossed in one
    // frame is a discontinuity; gradual change shortens accumulation instead.
    weather: WeatherRenderState,
    lighting: Lighting,
}

fn cloud_target_dimensions(width: u32, height: u32) -> (u32, u32) {
    (width.max(1).div_ceil(2), height.max(1).div_ceil(2))
}

/// Exact logical allocation of the ping-ponged cloud color and metadata
/// textures. Backend row/heap alignment is intentionally not included.
pub fn cloud_working_set_bytes(width: u32, height: u32) -> u64 {
    let (width, height) = cloud_target_dimensions(width, height);
    let update_width = width.div_ceil(CLOUD_INTERLEAVE_SIZE);
    let update_height = height.div_ceil(CLOUD_INTERLEAVE_SIZE);
    let history_pixels = u64::from(width) * u64::from(height) * CLOUD_HISTORY_BUFFERS;
    let update_pixels = u64::from(update_width) * u64::from(update_height);
    (history_pixels + update_pixels) * (CLOUD_COLOR_BYTES_PER_PIXEL + CLOUD_META_BYTES_PER_PIXEL)
}

fn create_cloud_buffer(
    device: &wgpu::Device,
    sample_bgl: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    width: u32,
    height: u32,
    index: usize,
) -> CloudBuffer {
    let color_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(match index {
            0 => "half-resolution cloud color A",
            1 => "half-resolution cloud color B",
            _ => "interleaved cloud update color",
        }),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: CLOUD_COLOR_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let color_view = color_texture.create_view(&Default::default());
    let meta_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(match index {
            0 => "half-resolution cloud depth A",
            1 => "half-resolution cloud depth B",
            _ => "interleaved cloud update depth",
        }),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: CLOUD_META_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let meta_view = meta_texture.create_view(&Default::default());
    let sample_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(match index {
            0 => "sample cloud history A",
            1 => "sample cloud history B",
            _ => "sample interleaved cloud update",
        }),
        layout: sample_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&color_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&meta_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });
    CloudBuffer {
        _color_texture: color_texture,
        color_view,
        _meta_texture: meta_texture,
        meta_view,
        sample_bg,
    }
}

fn create_cloud_targets(
    device: &wgpu::Device,
    sample_bgl: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    width: u32,
    height: u32,
) -> CloudTargets {
    let (width, height) = cloud_target_dimensions(width, height);
    let update_width = width.div_ceil(CLOUD_INTERLEAVE_SIZE);
    let update_height = height.div_ceil(CLOUD_INTERLEAVE_SIZE);
    CloudTargets {
        width,
        height,
        buffers: [
            create_cloud_buffer(device, sample_bgl, sampler, width, height, 0),
            create_cloud_buffer(device, sample_bgl, sampler, width, height, 1),
        ],
        update_width,
        update_height,
        update: create_cloud_buffer(device, sample_bgl, sampler, update_width, update_height, 2),
        history_index: 0,
    }
}

fn wrapped_weather_delta(current: f32, previous: f32) -> f32 {
    let mut delta = (current - previous).rem_euclid(65_536.0);
    if delta > 32_768.0 {
        delta -= 65_536.0;
    }
    delta
}

fn cloud_history_compatible(
    previous: CloudHistoryState,
    camera_ecef: DVec3,
    weather: WeatherRenderState,
    lighting: Lighting,
) -> bool {
    let elapsed = wrapped_weather_delta(weather.weather2[0], previous.weather_time);
    let old = previous.weather;
    previous.camera_ecef.distance(camera_ecef) <= 100_000.0
        && elapsed.abs() <= CLOUD_HISTORY_MAX_WEATHER_STEP_SECONDS
        // Cirrus and orbital clouds still read the modulo weather clock.
        && (weather.weather2[0] - previous.weather_time).abs() <= 32_768.0
        && (weather.weather2[1] - old.weather2[1]).abs() <= f32::EPSILON
        && (weather.weather3[3] - old.weather3[3]).abs() <= f32::EPSILON
        && (weather.surface[3] - old.surface[3]).abs() <= f32::EPSILON
        && cloud_appearance_step(old, weather, previous.lighting, lighting) < 1.0
}

/// Largest per-frame change in the inputs that shape cloud appearance, as a
/// fraction of the change that visibly outdates history.
fn cloud_appearance_step(
    old: WeatherRenderState,
    new: WeatherRenderState,
    old_light: Lighting,
    new_light: Lighting,
) -> f32 {
    let degrees = |a: DVec3, b: DVec3| a.dot(b).clamp(-1.0, 1.0).acos().to_degrees() as f32;
    let cover = (0..3)
        .map(|i| (new.weather5[i] - old.weather5[i]).abs())
        .fold(0.0, f32::max);
    [
        degrees(new_light.direction_ecef, old_light.direction_ecef),
        degrees(new_light.moon_direction_ecef, old_light.moon_direction_ecef),
        (new_light.moon_illumination - old_light.moon_illumination).abs() as f32 / 0.02,
        (new.weather3[0] - old.weather3[0]).abs() / 100.0,
        (new.weather3[1] - old.weather3[1]).abs() / 200.0,
        cover / 0.06,
        (new.weather1[3] - old.weather1[3]).abs() / 0.06,
        (new.weather0[1] - old.weather0[1]).abs() / 0.10,
        (new.surface[2] - old.surface[2]).abs() / 0.05,
        (new.weather6[0] - old.weather6[0]).abs() / 0.05,
        (new.weather0[2].max(120.0) / old.weather0[2].max(120.0) - 1.0).abs() / 0.20,
    ]
    .into_iter()
    .fold(0.0, f32::max)
}

/// History sample cap that keeps accumulated history within a quarter of a
/// visible appearance change, given that each texel refreshes every sixteen
/// frames. Normal time keeps the full 32 samples; 600x keeps very few.
fn cloud_history_sample_cap(appearance_step: f32) -> f32 {
    (0.25 / (16.0 * appearance_step.max(1e-6))).clamp(1.0, 32.0)
}

/// Every cloud tiling (four times a WGSL scale) divides this period, so
/// wrapping the accumulated drift shifts each pattern by whole tiles.
const CLOUD_DRIFT_PERIOD_M: f64 = 166_400.0;

/// The ENU wind at a position in ECEF, matching `cloud_common_wind`, so the
/// shader's history step equals the renderer's drift increment.
fn cloud_wind_ecef(position_ecef: DVec3, wind_enu: [f32; 4]) -> DVec3 {
    let up = position_ecef.normalize();
    let mut east = DVec3::Z.cross(up);
    if east.length_squared() < 1e-5 {
        east = DVec3::X.cross(up);
    }
    let east = east.normalize();
    let north = up.cross(east).normalize();
    east * f64::from(wind_enu[0]) + north * f64::from(wind_enu[1]) + up * f64::from(wind_enu[2])
}

/// Integrates the wind over simulated time. Wind times a clock jumps the whole
/// field whenever the wind changes; the integral stays continuous at any rate.
fn advance_cloud_drift(drift: DVec3, wind_ecef: DVec3, elapsed_seconds: f64) -> DVec3 {
    (drift + wind_ecef * elapsed_seconds).rem_euclid(DVec3::splat(CLOUD_DRIFT_PERIOD_M))
}

/// Surface-independent surface renderer + GPU residency.
pub struct Renderer {
    world_aa: crate::world_aa::WorldAa,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    rare_surfaces: SurfacePipelines,
    common_surfaces: SurfacePipelines,
    /// Same surface shading/depth path as terrain, but two-sided for small
    /// player meshes whose authored shells must remain readable from above.
    player_pipeline: wgpu::RenderPipeline,
    vehicle_effects: crate::vehicle_effects::VehicleEffectRenderer,
    landmark_pipeline: wgpu::RenderPipeline,
    packed_landmark_pipeline: wgpu::RenderPipeline,
    tree_pipeline: wgpu::RenderPipeline,
    flora_pipeline: wgpu::RenderPipeline,
    sun_shadows: SunShadowResources,
    night_pipeline: wgpu::RenderPipeline,
    aurora_pipeline: wgpu::RenderPipeline,
    atmo_pipeline: wgpu::RenderPipeline,
    cloud_integrate_pipeline: wgpu::RenderPipeline,
    cloud_resolve_pipeline: wgpu::RenderPipeline,
    cloud_composite_pipeline: wgpu::RenderPipeline,
    precipitation_pipeline: wgpu::RenderPipeline,
    star_pipeline: wgpu::RenderPipeline,
    tone_pipeline: wgpu::RenderPipeline,
    tone_bgl: wgpu::BindGroupLayout,
    post_buf: wgpu::Buffer,
    post_bg: wgpu::BindGroup,
    hdr: Option<HdrTarget>,
    _grade_lut_texture: wgpu::Texture,
    grade_lut_view: wgpu::TextureView,
    grade_sampler: wgpu::Sampler,
    cloud_sample_bgl: wgpu::BindGroupLayout,
    cloud_sampler: wgpu::Sampler,
    cloud_targets: Option<CloudTargets>,
    cloud_history: Option<CloudHistoryState>,
    cloud_frame_index: u32,
    cloud_drift: DVec3,
    cloud_drift_utc: Option<f64>,
    atmo_depth_bgl: wgpu::BindGroupLayout,
    /// Cached by texture-view identity. A resize supplies a distinct view,
    /// which replaces this pair on the next render; holding the clone keeps
    /// the bound view alive for exactly as long as the bind group needs it.
    atmo_depth_bg: Option<(wgpu::TextureView, wgpu::BindGroup)>,
    _atmo_lut_texture: wgpu::Texture,
    atmo_lut_view: wgpu::TextureView,
    atmo_lut_sampler: wgpu::Sampler,
    sky_model_buf: wgpu::Buffer,
    sky_environment: crate::sky_environment::SkyEnvironment,
    cloud_shadows: crate::cloud_shadows::CloudShadows,
    _climate_texture: wgpu::Texture,
    climate_view: wgpu::TextureView,
    climate_sampler: wgpu::Sampler,
    _cloud_shape_texture: wgpu::Texture,
    cloud_shape_view: wgpu::TextureView,
    cloud_shape_sampler: wgpu::Sampler,
    milky_way_view: wgpu::TextureView,
    milky_way_sampler: wgpu::Sampler,
    star_buf: wgpu::Buffer,
    star_magnitudes: Vec<f32>,
    globals_buf: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    chunk_buf: wgpu::Buffer,
    chunk_bg: wgpu::BindGroup,
    landmark_chunk_buf: wgpu::Buffer,
    landmark_chunk_bg: wgpu::BindGroup,
    light_chunk_buf: wgpu::Buffer,
    light_chunk_bg: wgpu::BindGroup,
    tree_chunk_buf: wgpu::Buffer,
    tree_chunk_bg: wgpu::BindGroup,
    flora_instance_buf: wgpu::Buffer,
    flora_instance_bg: wgpu::BindGroup,
    flora_index_buf: wgpu::Buffer,
    atlas_bg: wgpu::BindGroup,
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    vehicle_tex: wgpu::Texture,
    packed_arena: MeshArena,
    full_arena: MeshArena,
    light_arena: VertexArena,
    buffer_uploads: BufferUploadStaging,
    indirect_buf: wgpu::Buffer,
    light_indirect_buf: wgpu::Buffer,
    indirect_first_instance: bool,
    resident: ResidentGpuStore,
    tree_assets: HashMap<TreeProfileId, TreeAssetState>,
    tree_asset_users: HashMap<TreeProfileId, usize>,
    tree_asset_requests: mpsc::Sender<TreeProfileId>,
    tree_asset_results: mpsc::Receiver<BuiltTreeAsset>,
    player_parts: Vec<PlayerPartGpu>,
    mesh_stats: GpuMeshStats,
    pending_uploads: GpuUploadStats,
    material_stats_enabled: bool,
    draw_overflow_total: u64,
    draw_overflow_active: bool,
    sun_shadow_overflow_active: bool,
    building_night_lights_enabled: bool,
    shadows: ShadowPreference,
    clouds_visible: bool,
    precipitation_visible: bool,
    post_process_settings: PostProcessSettings,
    /// Rigid ECEF source-placement -> edited-placement transform. Landmark
    /// vertices are tagged during the calibrator's one-time generation.
    landmark_delta_ecef: Option<DMat4>,
    gpu_timing: Option<GpuTiming>,
}

fn landmark_model_basis(lat: f64, lon: f64, heading: f64) -> (DMat3, DVec3) {
    let (sin_lat, cos_lat) = lat.sin_cos();
    let (sin_lon, cos_lon) = lon.sin_cos();
    let east = DVec3::new(-sin_lon, cos_lon, 0.0);
    let north = DVec3::new(-sin_lat * cos_lon, -sin_lat * sin_lon, cos_lat);
    let up = DVec3::new(cos_lat * cos_lon, cos_lat * sin_lon, sin_lat);
    let (sin_heading, cos_heading) = heading.sin_cos();
    let model_x = east * cos_heading - north * sin_heading;
    let model_z = east * sin_heading + north * cos_heading;
    (DMat3::from_cols(model_x, up, model_z), up)
}

fn live_tree_instance(instance: TreeInstance) -> TreeInstanceGpu {
    let basis = vearth_gen::tree_render::instance_basis(
        instance.lat_rad,
        instance.lon_rad,
        f64::from(instance.yaw_rad),
    );
    let up = basis.y_axis;
    let origin = vearth_core::ellipsoid::geodetic_to_ecef(
        instance.lat_rad,
        instance.lon_rad,
        f64::from(instance.elevation_m),
    ) + up * 0.04;
    TreeInstanceGpu {
        profile: instance.profile,
        basis,
        origin,
        height_m: instance.height_m,
        color_scale: near_tree_color_scale(instance.profile, instance.seed),
    }
}

fn live_tree_model(instance: &TreeInstanceGpu, asset: &TreeAssetGpu) -> (DMat4, f64) {
    let scale =
        (f64::from(instance.height_m) / f64::from(asset.height_m.max(0.1))).clamp(0.35, 2.2);
    (
        DMat4::from_cols(
            (instance.basis.x_axis * scale).extend(0.0),
            (instance.basis.y_axis * scale).extend(0.0),
            (instance.basis.z_axis * scale).extend(0.0),
            instance.origin.extend(1.0),
        ),
        scale,
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TreeLodAssignment {
    lod: usize,
    alpha: f32,
}

fn tree_lod_assignments(projected_height_px: f64) -> ([TreeLodAssignment; 2], usize) {
    let steady = |lod| ([TreeLodAssignment { lod, alpha: 1.0 }; 2], 1);
    let transition = |lod, lower, upper| {
        let t = ((projected_height_px - lower) / (upper - lower)) as f32;
        let alpha = t * t * (3.0 - 2.0 * t);
        (
            [
                TreeLodAssignment { lod, alpha },
                TreeLodAssignment {
                    lod: lod + 1,
                    alpha: alpha - 1.0,
                },
            ],
            2,
        )
    };
    if projected_height_px >= 165.0 {
        steady(0)
    } else if projected_height_px > 135.0 {
        transition(0, 135.0, 165.0)
    } else if projected_height_px >= 50.0 {
        steady(1)
    } else if projected_height_px > 40.0 {
        transition(1, 40.0, 50.0)
    } else {
        steady(2)
    }
}

const TREE_READINESS_FADE: vearth_platform::time::Duration =
    vearth_platform::time::Duration::from_millis(240);

fn tree_readiness_blend(age: vearth_platform::time::Duration) -> f32 {
    let t = (age.as_secs_f32() / TREE_READINESS_FADE.as_secs_f32()).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn ready_tree_cell_blend(
    farthest_tree_m: f64,
    readiness: impl IntoIterator<Item = Option<vearth_platform::time::Duration>>,
) -> f32 {
    let mut blend = 1.0_f32;
    let mut any = false;
    for age in readiness {
        let Some(age) = age else {
            return 0.0;
        };
        any = true;
        blend = blend.min(tree_readiness_blend(age));
    }
    if any {
        near_tree_blend(farthest_tree_m) * blend
    } else {
        0.0
    }
}

fn tree_cell_readiness_blend(
    trees: &[TreeInstanceGpu],
    camera: DVec3,
    mut readiness: impl FnMut(TreeProfileId) -> Option<vearth_platform::time::Duration>,
) -> f32 {
    let mut farthest_sq = 0.0_f64;
    for tree in trees {
        let distance_sq = tree.origin.distance_squared(camera);
        // The fallback transition is cell-atomic: any tree outside the outer
        // radius makes every near-tree instance in this cell exactly invisible.
        if distance_sq >= NEAR_TREE_FADE_OUTER_M * NEAR_TREE_FADE_OUTER_M {
            return 0.0;
        }
        farthest_sq = farthest_sq.max(distance_sq);
    }
    let farthest_m = farthest_sq.sqrt();
    if near_tree_blend(farthest_m) == 0.0 {
        return 0.0;
    }
    ready_tree_cell_blend(farthest_m, trees.iter().map(|tree| readiness(tree.profile)))
}

fn admit_tree_cells(
    cells: impl Iterator<Item = CellId> + Clone,
    capacity: usize,
) -> HashSet<CellId> {
    let mut counts = HashMap::<CellId, usize>::new();
    for cell in cells.clone() {
        *counts.entry(cell).or_default() += 1;
    }
    let mut accepted = HashSet::new();
    let mut considered = HashSet::new();
    let mut count = 0;
    for cell in cells {
        if considered.insert(cell) && counts[&cell] <= capacity.saturating_sub(count) {
            count += counts[&cell];
            accepted.insert(cell);
        }
    }
    accepted
}

fn distinct_tree_instance_count(instances: impl IntoIterator<Item = (CellId, usize)>) -> usize {
    instances.into_iter().collect::<HashSet<_>>().len()
}

fn tree_distance_bin(distance_sq: f64) -> u16 {
    (distance_sq.sqrt() / TREE_DISTANCE_BIN_M).floor() as u16
}

fn tree_batch_key(profile: TreeProfileId, distance_sq: f64) -> (u16, u8) {
    (tree_distance_bin(distance_sq), profile as u8)
}

fn should_batch_tree_instances(instance_count: usize, batch_count: usize) -> bool {
    instance_count >= TREE_INSTANCE_BATCH_THRESHOLD
        && batch_count.saturating_mul(4) <= instance_count.saturating_mul(3)
}

fn near_tree_blend(distance_m: f64) -> f32 {
    let t = ((NEAR_TREE_FADE_OUTER_M - distance_m)
        / (NEAR_TREE_FADE_OUTER_M - NEAR_TREE_FADE_INNER_M))
        .clamp(0.0, 1.0) as f32;
    t * t * (3.0 - 2.0 * t)
}

fn landmark_delta_ecef(value: vearth_gen::landmarks::CalibrationOverride) -> DMat4 {
    let landmark = vearth_gen::landmarks::LANDMARKS
        .iter()
        .find(|landmark| landmark.name == value.name)
        .expect("calibration landmark exists");
    let resolved = vearth_gen::landmarks::resolve_calibration(value);
    let source_lat = landmark.lat_deg.to_radians();
    let source_lon = landmark.lon_deg.to_radians();
    let target_lat = resolved.lat_deg.to_radians();
    let target_lon = resolved.lon_deg.to_radians();
    let (source_basis, _) =
        landmark_model_basis(source_lat, source_lon, landmark.heading_deg.to_radians());
    let (target_basis, target_up) =
        landmark_model_basis(target_lat, target_lon, resolved.heading_deg.to_radians());
    let rotation = target_basis * source_basis.transpose();
    let source_origin = vearth_core::ellipsoid::geodetic_to_ecef(source_lat, source_lon, 0.0);
    let target_origin = vearth_core::ellipsoid::geodetic_to_ecef(target_lat, target_lon, 0.0)
        + target_up * resolved.base_offset_m;
    let translation = target_origin - rotation * source_origin;
    DMat4::from_cols(
        rotation.x_axis.extend(0.0),
        rotation.y_axis.extend(0.0),
        rotation.z_axis.extend(0.0),
        translation.extend(1.0),
    )
}

fn construction_material_origin(origin: DVec3) -> [f32; 3] {
    let (_, _, altitude) = vearth_core::ellipsoid::ecef_to_geodetic(origin);
    [0.0, altitude.rem_euclid(256.0) as f32, 0.0]
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorldInterface {
    Original,
    Compact,
    Half,
}

impl WorldInterface {
    fn from_value(value: Option<&str>) -> Self {
        match value {
            None => Self::Half,
            Some("compact") => Self::Compact,
            Some("half") => Self::Half,
            _ => Self::Original,
        }
    }

    fn from_env() -> Self {
        Self::from_value(
            vearth_platform::env::var("VEARTH_WORLD_INTERFACE")
                .ok()
                .as_deref(),
        )
    }

    fn for_device(self, features: wgpu::Features) -> Self {
        if self == Self::Half && !features.contains(wgpu::Features::SHADER_F16) {
            Self::Original
        } else {
            self
        }
    }
}

fn highrise_shader_profiles() -> String {
    use std::fmt::Write;
    let mut source = String::from(
        "struct HighriseProfile { floor_m: f32, glazing: vec2<f32>, roughness: f32, glass: vec3<f32>, coarse_glass: f32, };\nfn highrise_profile(system: u32) -> HighriseProfile { switch system {\n",
    );
    for (index, system) in vearth_gen::highrise::System::ALL.into_iter().enumerate() {
        let profile = system.profile();
        let roughness = match system {
            vearth_gen::highrise::System::SilverOffice => 0.17,
            vearth_gen::highrise::System::BronzeOffice => 0.27,
            _ => 0.22,
        };
        let glass = profile.glass.map(|channel| {
            let c = f64::from(channel) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        });
        // Coarse RGB already includes the actual source-edge area. Only the
        // unresolved reflection/emission uses this bounded typical-bay prior.
        let coarse_glass = (1.0 - 2.0 * profile.pier_m / profile.bay_m.max(4.0)).clamp(0.0, 1.0)
            * if system.residential() {
                profile.glazing.1 - profile.glazing.0
            } else {
                0.72 + 0.28 * (profile.glazing.1 - profile.glazing.0)
            };
        writeln!(
            source,
            "{case} {{ return HighriseProfile({:.8}, vec2<f32>({:.8}, {:.8}), {:.8}, vec3<f32>({:.8}, {:.8}, {:.8}), {:.8}); }}",
            profile.floor_m,
            profile.glazing.0,
            profile.glazing.1,
            roughness,
            glass[0], glass[1], glass[2], coarse_glass,
            case = if index == 7 {
                "default:".to_string()
            } else {
                format!("case {index}u:")
            }
        )
        .unwrap();
    }
    source.push_str("} }\n");
    source
}

/// Fragment options for world-surface pipelines. World meshes never carry
/// vehicle materials, so that shader path is always compiled out. Only chunks
/// flagged `has_rare_surface` need the rare-surface paths.
fn world_fragment_options(rare_surfaces: bool) -> wgpu::PipelineCompilationOptions<'static> {
    const RARE: &[(&str, f64)] = &[("VEHICLE_SURFACES", 0.0), ("RARE_SURFACES", 1.0)];
    const COMMON: &[(&str, f64)] = &[("VEHICLE_SURFACES", 0.0), ("RARE_SURFACES", 0.0)];
    wgpu::PipelineCompilationOptions {
        constants: if rare_surfaces { RARE } else { COMMON },
        ..Default::default()
    }
}

/// Ordinary world-surface pipelines for one `RARE_SURFACES` specialization.
struct SurfacePipelines {
    feedback: wgpu::RenderPipeline,
    opaque: wgpu::RenderPipeline,
    packed_feedback: wgpu::RenderPipeline,
    packed_opaque: wgpu::RenderPipeline,
}

impl SurfacePipelines {
    fn select(&self, packed: bool, opaque: bool) -> &wgpu::RenderPipeline {
        match (packed, opaque) {
            (false, false) => &self.feedback,
            (false, true) => &self.opaque,
            (true, false) => &self.packed_feedback,
            (true, true) => &self.packed_opaque,
        }
    }
}

/// These shaders deliberately take implicit-derivative samples inside
/// branches whose conditions vary per pixel (skipping work, see the shading
/// notes). WGSL's uniformity analysis rejects that; Metal, Vulkan and D3D
/// evaluate it as written, which is the intended result.
const DERIVATIVES_IN_BRANCHES: &str = "diagnostic(off, derivative_uniformity);";

fn world_shader_source() -> String {
    let highrise_profiles = highrise_shader_profiles();
    [
        DERIVATIVES_IN_BRANCHES,
        highrise_profiles.as_str(),
        include_str!("sky_palette.wgsl"),
        include_str!("water.wgsl"),
        include_str!("sky_environment_common.wgsl"),
        include_str!("sky_environment_sample.wgsl"),
        include_str!("construction_material.wgsl"),
        include_str!("terrain_material.wgsl"),
        include_str!("aerial_perspective.wgsl"),
        include_str!("cloud_shadow_common.wgsl"),
        include_str!("cloud_shadow_sample.wgsl"),
        include_str!("shader.wgsl"),
    ]
    .join("\n")
}

fn world_shader_source_with_interface(interface: WorldInterface) -> String {
    let source = world_shader_source();
    match interface {
        WorldInterface::Original => source,
        WorldInterface::Compact => format!(
            "{source}\nalias WorldScalar = f32;\n{}",
            include_str!("world_interface.wgsl"),
        ),
        WorldInterface::Half => format!(
            "enable f16;\n{source}\nalias WorldScalar = f16;\n{}",
            include_str!("world_interface.wgsl"),
        ),
    }
}

fn atmosphere_shader_source() -> String {
    [
        DERIVATIVES_IN_BRANCHES,
        include_str!("aerial_perspective.wgsl"),
        include_str!("cloud_common.wgsl"),
        include_str!("cloud_shadow_common.wgsl"),
        include_str!("cloud_shadow_sample.wgsl"),
        include_str!("solar_optics.wgsl"),
        include_str!("sky_palette.wgsl"),
        include_str!("atmosphere.wgsl"),
    ]
    .join("\n")
}

fn flora_shader_source() -> String {
    let cards =
        vearth_platform::env::var("VEARTH_CROP_GEOMETRY").is_ok_and(|value| value == "ribbons");
    flora_shader_source_with_ribbons(cards)
}

fn flora_shader_source_with_ribbons(ribbons: bool) -> String {
    let crop_geometry = format!(
        "const CROP_RIBBONS: bool = {ribbons};\nconst CROP_DISTANT_GRID_EDGE: u32 = {CROP_DISTANT_GRID_EDGE}u;\n"
    );
    let crop_catalog = crop_shader_catalog();
    [
        DERIVATIVES_IN_BRANCHES,
        crop_geometry.as_str(),
        crop_catalog.as_str(),
        include_str!("aerial_perspective.wgsl"),
        include_str!("cloud_shadow_common.wgsl"),
        include_str!("cloud_shadow_sample.wgsl"),
        include_str!("flora.wgsl"),
    ]
    .join("\n")
}

fn crop_shader_catalog() -> String {
    use std::fmt::Write;
    let mut source = String::from(
        "struct CropTreatment { green: vec3<f32>, mature: vec3<f32>, soil: vec3<f32>, height: f32, evergreen: bool };\nfn crop_treatment(palette: u32) -> CropTreatment { switch palette {\n",
    );
    for (index, treatment) in vearth_gen::agriculture_catalog::CROP_TREATMENTS
        .iter()
        .enumerate()
    {
        let rgb = |color: [u8; 3]| {
            format!(
                "vec3<f32>({:.8}, {:.8}, {:.8})",
                f32::from(color[0]) / 255.0,
                f32::from(color[1]) / 255.0,
                f32::from(color[2]) / 255.0,
            )
        };
        let case = if index == 0 {
            "default:".to_owned()
        } else {
            format!("case {}u:", index + 16)
        };
        writeln!(
            source,
            "{case} {{ return CropTreatment({}, {}, {}, {:.8}, {}); }}",
            rgb(treatment.green),
            rgb(treatment.mature),
            rgb(treatment.soil),
            treatment.height_scale,
            treatment.kind == vearth_gen::agriculture_catalog::CropKind::Tea
        )
        .unwrap();
    }
    source.push_str("} }\n");
    source
}

impl Renderer {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        // Material texels are device-independent; prepare them while the main
        // thread creates cloud resources and validates the world shaders.
        let material_atlas = vearth_platform::thread::Builder::new()
            .name("vearth-material-atlas".into())
            .spawn(crate::atlas::build)
            .ok();
        let world_aa = crate::world_aa::WorldAa::new(&device, DEPTH_FORMAT);
        let requested_world_interface = WorldInterface::from_env();
        let world_interface = requested_world_interface.for_device(device.features());
        if requested_world_interface != world_interface {
            log::warn!("half world interface unavailable without SHADER_F16; using original");
        }
        log::info!("opaque world interface: {world_interface:?}");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("surface"),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(world_shader_source_with_interface(
                world_interface,
            ))),
        });
        let flora_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("procedural flora"),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(flora_shader_source())),
        });
        let sun_shadow_receiver_bgl = create_sun_shadow_receiver_layout(&device);
        let sun_shadows_configured = !vearth_platform::env::var("VEARTH_DISABLE_SUN_SHADOWS")
            .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "yes"));
        let sun_shadows =
            create_sun_shadow_resources(&device, &sun_shadow_receiver_bgl, sun_shadows_configured);

        let globals_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<
                            crate::cloud_shadows::Frame,
                        >() as u64),
                    },
                    count: None,
                },
            ],
        });
        let chunk_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("chunk"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(TERRAIN_U_SIZE),
                },
                count: None,
            }],
        });
        let landmark_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("calibration landmark chunks"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(LANDMARK_U_SIZE),
                },
                count: None,
            }],
        });
        let light_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("night-light cells"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(LIGHT_U_SIZE),
                },
                count: None,
            }],
        });
        let tree_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("near-tree instances"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(TREE_U_SIZE),
                },
                count: None,
            }],
        });
        let flora_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("procedural flora instances"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(FLORA_INSTANCE_SIZE),
                },
                count: None,
            }],
        });

        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sky_model_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Hosek-Wilkie all-sky coefficients"),
            contents: bytemuck::bytes_of(&sky_model_lighting(0.5, 0.08, 0.5, 0.0).uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let cloud_shape_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tiled volumetric cloud shape"),
            size: wgpu::Extent3d {
                width: CLOUD_SHAPE_SIZE,
                height: CLOUD_SHAPE_SIZE,
                depth_or_array_layers: CLOUD_SHAPE_SIZE,
            },
            mip_level_count: CLOUD_SHAPE_MIP_LEVELS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, pixels) in cloud_shape_mip_chain(build_cloud_shape_volume())
            .iter()
            .enumerate()
        {
            let side = CLOUD_SHAPE_SIZE >> level;
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &cloud_shape_texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(CLOUD_SHAPE_BYTES_PER_ROW),
                    rows_per_image: Some(side),
                },
                wgpu::Extent3d {
                    width: side,
                    height: side,
                    depth_or_array_layers: side,
                },
            );
        }
        let cloud_shape_view = cloud_shape_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("tiled volumetric cloud shape view"),
            dimension: Some(wgpu::TextureViewDimension::D3),
            ..Default::default()
        });
        let cloud_shape_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("tiled volumetric cloud shape sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let sky_environment =
            crate::sky_environment::SkyEnvironment::new(&device, &globals_buf, &sky_model_buf);
        let cloud_shadows = crate::cloud_shadows::CloudShadows::new(
            &device,
            &globals_buf,
            &cloud_shape_view,
            &cloud_shape_sampler,
        );
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &globals_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: sky_model_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&sky_environment.view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sky_environment.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&cloud_shadows.view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&cloud_shadows.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: cloud_shadows.uniform.as_entire_binding(),
                },
            ],
        });
        let chunk_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chunk uniforms"),
            size: TERRAIN_U_SIZE * MAX_DRAWS as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let chunk_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("chunk"),
            layout: &chunk_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: chunk_buf.as_entire_binding(),
            }],
        });
        let landmark_chunk_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("calibration landmark chunk uniforms"),
            size: LANDMARK_U_SIZE * MAX_DRAWS as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let landmark_chunk_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("calibration landmark chunks"),
            layout: &landmark_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 2,
                resource: landmark_chunk_buf.as_entire_binding(),
            }],
        });
        let light_chunk_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("night-light cell uniforms"),
            size: LIGHT_U_SIZE * MAX_LIGHT_DRAWS as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let light_chunk_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("night-light cell uniforms"),
            layout: &light_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: light_chunk_buf.as_entire_binding(),
            }],
        });
        let tree_chunk_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("near-tree uniforms"),
            size: TREE_U_SIZE * MAX_TREE_DRAWS as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let tree_chunk_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("near-tree uniforms"),
            layout: &tree_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 1,
                resource: tree_chunk_buf.as_entire_binding(),
            }],
        });
        let flora_instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("procedural flora instances"),
            size: MAX_FLORA_UPLOAD_BYTES as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let flora_instance_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("procedural flora instances"),
            layout: &flora_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: flora_instance_buf.as_entire_binding(),
            }],
        });
        let flora_indices = flora_index_data();
        let flora_index_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("procedural flora cuboid indices"),
            contents: bytemuck::cast_slice(&flora_indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        // Physical material families share tone/gradient/roughness tiles.
        // The full mip chain keeps color and relief stable under minification.
        let atlas_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("atlas"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let atlas_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("physical material atlas"),
            size: wgpu::Extent3d {
                width: crate::atlas::TILE as u32,
                height: crate::atlas::TILE as u32,
                depth_or_array_layers: crate::atlas::LAYERS as u32,
            },
            mip_level_count: crate::atlas::MIPS as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let material_atlas = material_atlas.map_or_else(crate::atlas::build, |worker| {
            worker.join().expect("material atlas generation")
        });
        for (mip, layers) in material_atlas.iter().enumerate() {
            let side = (crate::atlas::TILE >> mip) as u32;
            for (layer, data) in layers.iter().enumerate() {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &atlas_tex,
                        mip_level: mip as u32,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: layer as u32,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    data,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(side * crate::atlas::CHANNELS as u32),
                        rows_per_image: Some(side),
                    },
                    wgpu::Extent3d {
                        width: side,
                        height: side,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
        drop(material_atlas);
        // Linear filtering preserves painted gradients at close range;
        // anisotropy protects slanted road and ground texture footprints.
        let atlas_samp = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("physical material atlas"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 4,
            ..Default::default()
        });
        // The app writes the layers once they are decoded off the render
        // thread (`vehicles::DeferredVehicles`); tools call
        // `write_vehicle_textures`.
        let vehicle_tex = crate::vehicles::textures::create(&device);
        let atlas_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("atlas"),
            layout: &atlas_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &atlas_tex.create_view(&wgpu::TextureViewDescriptor::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&atlas_samp),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        &vehicle_tex.create_view(&wgpu::TextureViewDescriptor::default()),
                    ),
                },
            ],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("surface"),
            bind_group_layouts: &[
                Some(&globals_bgl),
                Some(&chunk_bgl),
                Some(&atlas_bgl),
                Some(&sun_shadow_receiver_bgl),
            ],
            immediate_size: 0,
        });
        let create_pipeline = |entry, label, rare_surfaces| {
            let compact_world = entry == "fs_opaque" && world_interface != WorldInterface::Original;
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(if compact_world { "vs_world" } else { "vs_main" }),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<SurfaceVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x3,
                            1 => Uint32,
                            2 => Uint8x4,
                        ],
                    })],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(if compact_world {
                        "fs_world_opaque"
                    } else {
                        entry
                    }),
                    compilation_options: world_fragment_options(rare_surfaces),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: HDR_FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: surface_primitive_state(true),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Greater), // reversed-Z
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: world_aa.state(),
                multiview_mask: None,
                cache: None,
            })
        };
        let unpacked_surfaces = |rare_surfaces| {
            (
                create_pipeline("fs_main", "surface", rare_surfaces),
                create_pipeline("fs_opaque", "opaque surface", rare_surfaces),
            )
        };
        let (rare_feedback, rare_opaque) = unpacked_surfaces(true);
        let (common_feedback, common_opaque) = unpacked_surfaces(false);
        // Player meshes are small, separately submitted, and frequently seen
        // from above at oblique angles. Keep the world's backface-culling
        // optimization intact while rendering only these authored shells as
        // two-sided geometry.
        let player_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("authored player surfaces"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_player"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<SurfaceVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x3,
                            1 => Uint32,
                            2 => Uint8x4,
                        ],
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: 8,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![3 => Float32x2],
                    }),
                ],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: surface_primitive_state(false),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let tree_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("near-tree instances"),
            bind_group_layouts: &[
                Some(&globals_bgl),
                Some(&tree_bgl),
                Some(&atlas_bgl),
                Some(&sun_shadow_receiver_bgl),
            ],
            immediate_size: 0,
        });
        let tree_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("near-tree instances"),
            layout: Some(&tree_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_tree"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<SurfaceVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3,
                        1 => Uint32,
                        2 => Uint8x4,
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: world_fragment_options(false),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: world_aa.state(),
            multiview_mask: None,
            cache: None,
        });
        let flora_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("procedural flora"),
            bind_group_layouts: &[
                Some(&globals_bgl),
                Some(&flora_bgl),
                Some(&sun_shadow_receiver_bgl),
                Some(&atlas_bgl),
            ],
            immediate_size: 0,
        });
        let flora_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("procedural flora"),
            layout: Some(&flora_layout),
            vertex: wgpu::VertexState {
                module: &flora_shader,
                entry_point: Some("vs_flora"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &flora_shader,
                entry_point: Some("fs_flora"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            // Flora is closed, outward-wound geometry suitable for backface culling.
            primitive: surface_primitive_state(true),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: world_aa.state(),
            multiview_mask: None,
            cache: None,
        });
        let create_packed_pipeline = |entry, label, rare_surfaces| {
            let compact_world = entry == "fs_opaque" && world_interface != WorldInterface::Original;
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(if compact_world {
                        "vs_world_packed"
                    } else {
                        "vs_packed"
                    }),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<PackedSurfaceVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Uint32,
                            1 => Uint32,
                            2 => Uint8x4,
                        ],
                    })],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(if compact_world {
                        "fs_world_opaque"
                    } else {
                        entry
                    }),
                    compilation_options: world_fragment_options(rare_surfaces),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: HDR_FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: world_aa.state(),
                multiview_mask: None,
                cache: None,
            })
        };
        let packed_surfaces = |feedback, opaque, rare_surfaces| SurfacePipelines {
            feedback,
            opaque,
            packed_feedback: create_packed_pipeline("fs_main", "packed terrain", rare_surfaces),
            packed_opaque: create_packed_pipeline(
                "fs_opaque",
                "opaque packed terrain",
                rare_surfaces,
            ),
        };
        let rare_surfaces = packed_surfaces(rare_feedback, rare_opaque, true);
        let common_surfaces = packed_surfaces(common_feedback, common_opaque, false);

        let landmark_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("calibration landmark surfaces"),
            bind_group_layouts: &[
                Some(&globals_bgl),
                Some(&landmark_bgl),
                Some(&atlas_bgl),
                Some(&sun_shadow_receiver_bgl),
            ],
            immediate_size: 0,
        });
        let landmark_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("calibration landmark surfaces"),
            layout: Some(&landmark_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_landmark"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<SurfaceVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3,
                        1 => Uint32,
                        2 => Uint8x4,
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: world_fragment_options(true),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: world_aa.state(),
            multiview_mask: None,
            cache: None,
        });
        let packed_landmark_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("packed calibration landmark surfaces"),
                layout: Some(&landmark_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_packed_landmark"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<PackedSurfaceVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Uint32,
                            1 => Uint32,
                            2 => Uint8x4,
                        ],
                    })],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: world_fragment_options(true),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: HDR_FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: world_aa.state(),
                multiview_mask: None,
                cache: None,
            });

        let night_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("night lights"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("night_lights.wgsl"))),
        });
        let night_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("night lights"),
            bind_group_layouts: &[Some(&globals_bgl), Some(&light_bgl)],
            immediate_size: 0,
        });
        let night_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("night lights"),
            layout: Some(&night_layout),
            vertex: wgpu::VertexState {
                module: &night_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<NightLightVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3,
                        1 => Float32,
                        2 => Float32x2,
                        3 => Unorm8x4,
                        4 => Uint32,
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &night_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent::OVER,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: world_aa.state(),
            multiview_mask: None,
            cache: None,
        });

        // Quiet polar aurora: procedural triangle-strip curtains generated
        // from vertex/instance indices. They share the main HDR/depth pass so
        // terrain and the globe occlude them correctly at every camera scale.
        let aurora_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("aurora curtains"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("aurora.wgsl"))),
        });
        let aurora_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("aurora curtains"),
            bind_group_layouts: &[Some(&globals_bgl)],
            immediate_size: 0,
        });
        let aurora_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("aurora curtains"),
            layout: Some(&aurora_layout),
            vertex: wgpu::VertexState {
                module: &aurora_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &aurora_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent::OVER,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: world_aa.state(),
            multiview_mask: None,
            cache: None,
        });

        // ESO/S. Brunier all-sky panorama, prepared offline into a compact
        // diffuse sRGB texture. Longitude repeats; latitude clamps at the
        // celestial poles.
        let (milky_way_width, milky_way_height, milky_way_pixels) = crate::sky::milky_way_texture();
        let milky_way_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Milky Way diffuse all-sky map"),
            size: wgpu::Extent3d {
                width: milky_way_width,
                height: milky_way_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &milky_way_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            milky_way_pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(milky_way_width * 4),
                rows_per_image: Some(milky_way_height),
            },
            wgpu::Extent3d {
                width: milky_way_width,
                height: milky_way_height,
                depth_or_array_layers: 1,
            },
        );
        let milky_way_view = milky_way_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let milky_way_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Milky Way sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // Atmosphere pass: fullscreen additive, reads depth and the all-sky
        // Milky Way map.
        let atmo_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("atmosphere"),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(atmosphere_shader_source())),
        });
        let atmo_lut_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bounded clear-sky lookup"),
            size: wgpu::Extent3d {
                width: ATMO_LUT_WIDTH,
                height: ATMO_LUT_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &atmo_lut_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &build_atmosphere_lut(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(ATMO_LUT_WIDTH * 4),
                rows_per_image: Some(ATMO_LUT_HEIGHT),
            },
            wgpu::Extent3d {
                width: ATMO_LUT_WIDTH,
                height: ATMO_LUT_HEIGHT,
                depth_or_array_layers: 1,
            },
        );
        let atmo_lut_view = atmo_lut_texture.create_view(&Default::default());
        let atmo_lut_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bounded clear-sky lookup sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let (climate_width, climate_height, climate_layers, climate_pixels) =
            ClimateAtlas::bundled().gpu_texture();
        let climate_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("observed monthly climate atlas"),
            size: wgpu::Extent3d {
                width: climate_width,
                height: climate_height,
                depth_or_array_layers: climate_layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &climate_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &climate_pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(climate_width * 4),
                rows_per_image: Some(climate_height),
            },
            wgpu::Extent3d {
                width: climate_width,
                height: climate_height,
                depth_or_array_layers: climate_layers,
            },
        );
        let climate_view = climate_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("observed monthly climate atlas array view"),
            format: None,
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            usage: None,
            aspect: wgpu::TextureAspect::All,
            base_mip_level: 0,
            mip_level_count: None,
            base_array_layer: 0,
            array_layer_count: Some(climate_layers),
        });
        let climate_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("climate atlas sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let atmo_depth_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("atmo depth"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(
                            std::mem::size_of::<SkyModelUniform>() as u64,
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let cloud_sample_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("temporal cloud history"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let cloud_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("temporal cloud linear sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let atmo_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("atmosphere"),
            bind_group_layouts: &[Some(&globals_bgl), Some(&atmo_depth_bgl)],
            immediate_size: 0,
        });
        let atmo_ms_layout = (world_aa.samples > 1).then(|| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("multisample atmosphere"),
                bind_group_layouts: &[
                    Some(&globals_bgl),
                    Some(&atmo_depth_bgl),
                    Some(&world_aa.layout),
                ],
                immediate_size: 0,
            })
        });
        let atmo_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("atmosphere"),
            layout: Some(atmo_ms_layout.as_ref().unwrap_or(&atmo_layout)),
            vertex: wgpu::VertexState {
                module: &atmo_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &atmo_shader,
                entry_point: Some(if world_aa.samples > 1 {
                    "fs_main_ms"
                } else {
                    "fs_main"
                }),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent::OVER,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let cloud_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("half-resolution temporal clouds"),
            bind_group_layouts: &[
                Some(&globals_bgl),
                Some(&atmo_depth_bgl),
                Some(&cloud_sample_bgl),
            ],
            immediate_size: 0,
        });
        let cloud_resolve_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("interleaved cloud history resolve"),
            bind_group_layouts: &[
                Some(&globals_bgl),
                Some(&atmo_depth_bgl),
                Some(&cloud_sample_bgl),
                Some(&cloud_sample_bgl),
            ],
            immediate_size: 0,
        });
        let cloud_integrate_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("interleaved temporal cloud integration"),
                layout: Some(&cloud_layout),
                vertex: wgpu::VertexState {
                    module: &atmo_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &atmo_shader,
                    entry_point: Some("fs_cloud_march_interleaved"),
                    compilation_options: Default::default(),
                    targets: &[
                        Some(wgpu::ColorTargetState {
                            format: CLOUD_COLOR_FORMAT,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        }),
                        Some(wgpu::ColorTargetState {
                            format: CLOUD_META_FORMAT,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        }),
                    ],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
        let cloud_resolve_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("half-resolution interleaved cloud history resolve"),
                layout: Some(&cloud_resolve_layout),
                vertex: wgpu::VertexState {
                    module: &atmo_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &atmo_shader,
                    entry_point: Some("fs_cloud_resolve_interleaved"),
                    compilation_options: Default::default(),
                    targets: &[
                        Some(wgpu::ColorTargetState {
                            format: CLOUD_COLOR_FORMAT,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        }),
                        Some(wgpu::ColorTargetState {
                            format: CLOUD_META_FORMAT,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        }),
                    ],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
        let cloud_composite_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("full-resolution depth-aware cloud upscale"),
                layout: Some(&cloud_layout),
                vertex: wgpu::VertexState {
                    module: &atmo_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &atmo_shader,
                    entry_point: Some("fs_cloud_composite"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: HDR_FORMAT,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
        let precipitation_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("near precipitation"),
                layout: Some(&atmo_layout),
                vertex: wgpu::VertexState {
                    module: &atmo_shader,
                    entry_point: Some("vs_precipitation"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &atmo_shader,
                    entry_point: Some("fs_precipitation_world"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: HDR_FORMAT,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });

        // Real stars: one six-vertex screen-space billboard per Hipparcos
        // entry. The fragment pass shares atmosphere depth, so loaded terrain
        // occludes stars without requiring a far-depth approximation.
        let star_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("stars"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("stars.wgsl"))),
        });
        let star_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("stars"),
            bind_group_layouts: &[Some(&globals_bgl), Some(&atmo_depth_bgl)],
            immediate_size: 0,
        });
        let star_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("stars"),
            layout: Some(&star_layout),
            vertex: wgpu::VertexState {
                module: &star_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<StarInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3,
                        1 => Float32,
                        2 => Float32,
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &star_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent::OVER,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let stars: Vec<StarInstance> = crate::sky::bright_stars()
            .into_iter()
            .map(|star| StarInstance {
                direction_icrs: star.direction_icrs,
                magnitude: star.magnitude,
                bv: star.bv,
            })
            .collect();
        let star_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Hipparcos bright stars"),
            contents: bytemuck::cast_slice(&stars),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let star_magnitudes = stars.iter().map(|star| star.magnitude).collect();

        // Color grading and perceptual saturation remain presentation choices
        // inside the existing tone-map pass; neither needs an intermediate
        // render target.
        let post_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post-process settings"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(
                        std::mem::size_of::<PostProcessUniform>() as u64,
                    ),
                },
                count: None,
            }],
        });
        let post_process_settings = PostProcessSettings::default();
        let post_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("post-process settings"),
            contents: bytemuck::bytes_of(&PostProcessUniform::new(post_process_settings)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let post_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post-process settings"),
            layout: &post_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: post_buf.as_entire_binding(),
            }],
        });
        let grade_lut_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("creative color-grade LUT"),
            size: wgpu::Extent3d {
                width: COLOR_GRADE_LUT_SIZE,
                height: COLOR_GRADE_LUT_SIZE,
                depth_or_array_layers: COLOR_GRADE_LUT_SIZE,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &grade_lut_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &build_color_grade_lut(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(COLOR_GRADE_LUT_SIZE * 4),
                rows_per_image: Some(COLOR_GRADE_LUT_SIZE),
            },
            wgpu::Extent3d {
                width: COLOR_GRADE_LUT_SIZE,
                height: COLOR_GRADE_LUT_SIZE,
                depth_or_array_layers: COLOR_GRADE_LUT_SIZE,
            },
        );
        let grade_lut_view = grade_lut_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("creative color-grade LUT"),
            dimension: Some(wgpu::TextureViewDimension::D3),
            ..Default::default()
        });
        let grade_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("creative color-grade LUT sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let tone_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tone map"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("tonemap.wgsl"))),
        });
        let tone_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tone-map post-process resources"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let tone_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tone map"),
            bind_group_layouts: &[Some(&globals_bgl), Some(&tone_bgl), Some(&post_bgl)],
            immediate_size: 0,
        });
        let tone_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tone map"),
            layout: Some(&tone_layout),
            vertex: wgpu::VertexState {
                module: &tone_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &tone_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        let gpu_timing = GpuTiming::new(&device, &queue);
        let indirect_supported = device
            .features()
            .contains(wgpu::Features::INDIRECT_FIRST_INSTANCE);
        let force_direct_draws = vearth_platform::env::var("VEARTH_FORCE_DIRECT_DRAWS")
            .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "on"));
        let rollout = FinalPipelinePolicy::for_gpu(GpuCapabilities {
            indirect_first_instance: indirect_supported && !force_direct_draws,
            timestamp_queries: device.features().contains(wgpu::Features::TIMESTAMP_QUERY),
        });
        let indirect_first_instance = matches!(
            rollout.terrain_submission,
            TerrainSubmission::IndirectFirstInstance
        );
        log::info!(
            "terrain GPU arenas enabled; rollout class {} (indirect multi-draw {})",
            rollout.capability_class(),
            if indirect_first_instance {
                "enabled"
            } else if force_direct_draws {
                "disabled by VEARTH_FORCE_DIRECT_DRAWS"
            } else {
                "unavailable (direct arena fallback)"
            },
        );
        let indirect_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain indirect draws"),
            size: (MAX_WORLD_COMMANDS * std::mem::size_of::<wgpu::util::DrawIndexedIndirectArgs>())
                as u64,
            usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let light_indirect_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("night-light indirect draws"),
            size: (MAX_LIGHT_DRAWS * std::mem::size_of::<wgpu::util::DrawIndirectArgs>()) as u64,
            usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let (tree_asset_requests, tree_request_rx) = mpsc::channel::<TreeProfileId>();
        let (tree_result_tx, tree_asset_results) = mpsc::channel::<BuiltTreeAsset>();
        vearth_platform::thread::Builder::new()
            .name("vearth-tree-assets".into())
            .spawn(move || {
                while let Ok(profile) = tree_request_rx.recv() {
                    if tree_result_tx.send(build_tree_asset(profile)).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn near-tree asset worker");
        let buffer_uploads = BufferUploadStaging::new(&device);

        let vehicle_effects = crate::vehicle_effects::VehicleEffectRenderer::new(
            &device,
            &globals_bgl,
            &atmo_depth_bgl,
            HDR_FORMAT,
        );

        Renderer {
            device,
            queue,
            rare_surfaces,
            common_surfaces,
            player_pipeline,
            vehicle_effects,
            landmark_pipeline,
            packed_landmark_pipeline,
            tree_pipeline,
            flora_pipeline,
            sun_shadows,
            night_pipeline,
            aurora_pipeline,
            atmo_pipeline,
            cloud_integrate_pipeline,
            cloud_resolve_pipeline,
            cloud_composite_pipeline,
            precipitation_pipeline,
            star_pipeline,
            tone_pipeline,
            tone_bgl,
            post_buf,
            post_bg,
            world_aa,
            hdr: None,
            _grade_lut_texture: grade_lut_texture,
            grade_lut_view,
            grade_sampler,
            cloud_sample_bgl,
            cloud_sampler,
            cloud_targets: None,
            cloud_history: None,
            cloud_frame_index: 0,
            cloud_drift: DVec3::ZERO,
            cloud_drift_utc: None,
            atmo_depth_bgl,
            atmo_depth_bg: None,
            _atmo_lut_texture: atmo_lut_texture,
            atmo_lut_view,
            atmo_lut_sampler,
            sky_model_buf,
            sky_environment,
            cloud_shadows,
            _climate_texture: climate_texture,
            climate_view,
            climate_sampler,
            _cloud_shape_texture: cloud_shape_texture,
            cloud_shape_view,
            cloud_shape_sampler,
            milky_way_view,
            milky_way_sampler,
            star_buf,
            star_magnitudes,
            globals_buf,
            globals_bg,
            chunk_buf,
            chunk_bg,
            landmark_chunk_buf,
            landmark_chunk_bg,
            light_chunk_buf,
            light_chunk_bg,
            tree_chunk_buf,
            tree_chunk_bg,
            flora_instance_buf,
            flora_instance_bg,
            flora_index_buf,
            atlas_bg,
            vehicle_tex,
            packed_arena: MeshArena::new(
                "packed terrain vertex/index arena",
                std::mem::size_of::<PackedSurfaceVertex>() as u64,
            ),
            full_arena: MeshArena::new(
                "full-float terrain vertex/index arena",
                std::mem::size_of::<SurfaceVertex>() as u64,
            ),
            light_arena: VertexArena::new(
                "night-light vertex arena",
                std::mem::size_of::<NightLightVertex>() as u64,
            ),
            buffer_uploads,
            indirect_buf,
            light_indirect_buf,
            indirect_first_instance,
            resident: ResidentGpuStore::default(),
            tree_assets: HashMap::new(),
            tree_asset_users: HashMap::new(),
            tree_asset_requests,
            tree_asset_results,
            player_parts: Vec::new(),
            mesh_stats: GpuMeshStats::default(),
            pending_uploads: GpuUploadStats::default(),
            material_stats_enabled: vearth_platform::env::var("VEARTH_RENDER_MATERIAL_STATS")
                .is_ok_and(|value| value == "1"),
            draw_overflow_total: 0,
            draw_overflow_active: false,
            sun_shadow_overflow_active: false,
            building_night_lights_enabled: true,
            shadows: ShadowPreference::default(),
            clouds_visible: true,
            precipitation_visible: true,
            post_process_settings,
            landmark_delta_ecef: None,
            gpu_timing,
        }
    }

    pub fn set_building_night_lights_enabled(&mut self, enabled: bool) {
        self.building_night_lights_enabled = enabled;
    }

    pub fn building_night_lights_enabled(&self) -> bool {
        self.building_night_lights_enabled
    }

    pub fn shadows(&self) -> ShadowPreference {
        self.shadows
    }

    pub fn set_shadows(&mut self, shadows: ShadowPreference) {
        self.shadows = shadows;
    }

    /// Show or hide cloud geometry without changing the authoritative weather
    /// sampled by the atmosphere, surface, wind, or precipitation systems.
    pub fn set_clouds_visible(&mut self, visible: bool) {
        self.clouds_visible = visible;
    }

    pub fn clouds_visible(&self) -> bool {
        self.clouds_visible
    }

    /// Show or hide near-camera rain, snow, mixed precipitation, and hail.
    /// The sampled precipitation and all of its non-visual effects remain live.
    pub fn set_precipitation_visible(&mut self, visible: bool) {
        self.precipitation_visible = visible;
    }

    pub fn precipitation_visible(&self) -> bool {
        self.precipitation_visible
    }

    pub fn post_process_settings(&self) -> PostProcessSettings {
        self.post_process_settings
    }

    pub fn set_post_process_settings(&mut self, settings: PostProcessSettings) {
        self.post_process_settings = settings;
        self.queue.write_buffer(
            &self.post_buf,
            0,
            bytemuck::bytes_of(&PostProcessUniform::new(self.post_process_settings)),
        );
    }

    fn update_post_process_settings(&mut self, update: impl FnOnce(&mut PostProcessSettings)) {
        let mut settings = self.post_process_settings;
        update(&mut settings);
        self.set_post_process_settings(settings);
    }

    pub fn set_color_grading_enabled(&mut self, enabled: bool) {
        self.update_post_process_settings(|settings| settings.color_grading = enabled);
    }

    pub fn set_filmic_saturation_enabled(&mut self, enabled: bool) {
        self.update_post_process_settings(|settings| settings.filmic_saturation = enabled);
    }

    /// Update the calibrator's landmark transform without touching resident
    /// terrain, vector data, GPU mesh buffers or the generation queue.
    pub fn set_landmark_calibration(
        &mut self,
        value: Option<vearth_gen::landmarks::CalibrationOverride>,
    ) {
        self.landmark_delta_ecef = value.map(landmark_delta_ecef);
    }

    /// Exact logical GPU-buffer residency for world and player meshes.
    pub fn mesh_stats(&self) -> GpuMeshStats {
        self.mesh_stats
    }

    /// Upload the player-model meshes once; `PlayerDraw::parts` indexes
    /// into this list in upload order.
    pub fn upload_player_parts(&mut self, parts: &[vearth_core::mesh::MeshBuffers]) {
        let uvs: Vec<_> = parts
            .iter()
            .map(|mesh| vec![[0.0; 2]; mesh.vertices.len()])
            .collect();
        self.upload_player_parts_with_uv(parts, &uvs);
    }

    pub fn write_vehicle_texture_layer(&self, layer: u32, levels: &[Vec<u8>]) {
        crate::vehicles::textures::write_layer(&self.queue, &self.vehicle_tex, layer, levels);
    }

    /// Decodes and writes every vehicle texture layer on this thread.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn write_vehicle_textures(&self) {
        for layer in 0..crate::vehicles::textures::LAYERS {
            self.write_vehicle_texture_layer(
                layer,
                &crate::vehicles::textures::decode_layer(layer),
            );
        }
    }

    pub fn upload_player_parts_with_uv(
        &mut self,
        parts: &[vearth_core::mesh::MeshBuffers],
        uvs: &[Vec<[f32; 2]>],
    ) {
        assert_eq!(parts.len(), uvs.len(), "one UV stream per player part");
        let old = MeshFootprint::from_player_parts(&self.player_parts);
        self.mesh_stats.remove_player_parts(old);
        let uploaded: Vec<PlayerPartGpu> = parts
            .iter()
            .zip(uvs)
            .map(|(mb, uv)| {
                assert_eq!(mb.vertices.len(), uv.len(), "one UV per mesh vertex");
                let vbuf = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("player vb"),
                        contents: bytemuck::cast_slice(&mb.vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                let uvbuf = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("player UVs"),
                        contents: bytemuck::cast_slice(uv),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                let ibuf = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("player ib"),
                        contents: bytemuck::cast_slice(&mb.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    });
                PlayerPartGpu {
                    vertex_bytes: vbuf.size() + uvbuf.size(),
                    index_bytes: ibuf.size(),
                    vbuf,
                    ibuf,
                    uvbuf: Some(uvbuf),
                    index_count: mb.indices.len() as u32,
                }
            })
            .collect();
        let footprint = MeshFootprint::from_player_parts(&uploaded);
        self.mesh_stats.add_player_parts(footprint);
        self.pending_uploads.record_player_parts(footprint);
        self.player_parts = uploaded;
    }

    fn retain_tree_profiles(&mut self, trees: &[TreeInstanceGpu]) {
        let profiles: HashSet<_> = trees.iter().map(|tree| tree.profile).collect();
        for profile in profiles {
            let users = self.tree_asset_users.entry(profile).or_default();
            *users += 1;
        }
    }

    fn request_tree_profile(&mut self, profile: TreeProfileId) {
        if self.tree_assets.contains_key(&profile) {
            return;
        }
        self.tree_assets.insert(profile, TreeAssetState::Pending);
        if self.tree_asset_requests.send(profile).is_err() {
            self.tree_assets.remove(&profile);
            log::warn!("near-tree asset worker stopped before {profile:?} could be built");
        }
    }

    fn release_tree_profiles(&mut self, trees: &[TreeInstanceGpu]) {
        let profiles: HashSet<_> = trees.iter().map(|tree| tree.profile).collect();
        for profile in profiles {
            let remove = self
                .tree_asset_users
                .get_mut(&profile)
                .is_some_and(|users| {
                    *users = users.saturating_sub(1);
                    *users == 0
                });
            if remove {
                self.tree_asset_users.remove(&profile);
                self.tree_assets.remove(&profile);
            }
        }
    }

    fn collect_tree_assets(&mut self) {
        while let Ok(built) = self.tree_asset_results.try_recv() {
            if self
                .tree_asset_users
                .get(&built.profile)
                .copied()
                .unwrap_or_default()
                == 0
            {
                self.tree_assets.remove(&built.profile);
                continue;
            }
            let vbuf = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("tree surface vertices"),
                    contents: bytemuck::cast_slice(&built.vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            let (ibuf, index_format) = match built.indices {
                BuiltTreeIndices::U16(indices) => (
                    self.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("16-bit near-tree ib"),
                            contents: bytemuck::cast_slice(&indices),
                            usage: wgpu::BufferUsages::INDEX,
                        }),
                    wgpu::IndexFormat::Uint16,
                ),
                BuiltTreeIndices::U32(indices) => (
                    self.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("32-bit near-tree ib"),
                            contents: bytemuck::cast_slice(&indices),
                            usage: wgpu::BufferUsages::INDEX,
                        }),
                    wgpu::IndexFormat::Uint32,
                ),
            };
            let ready_at = match self.tree_assets.get(&built.profile) {
                Some(TreeAssetState::Ready(asset)) => asset.ready_at,
                _ => Instant::now(),
            };
            self.tree_assets.insert(
                built.profile,
                TreeAssetState::Ready(TreeAssetGpu {
                    vbuf,
                    ibuf,
                    lod_indices: built.lod_indices,
                    index_format,
                    height_m: built.height_m,
                    radius_m: built.radius_m,
                    ready_at,
                }),
            );
        }
    }

    /// Poll completed near-tree builds and count locally referenced species
    /// still waiting for an uploaded asset; ready assets fade in separately.
    pub fn pending_tree_assets(&mut self) -> usize {
        self.collect_tree_assets();
        self.tree_assets
            .values()
            .filter(|state| matches!(state, TreeAssetState::Pending))
            .count()
    }

    /// Remaining wall-clock fade for ready trees that can be instanced in
    /// this view. Offline captures wait before collecting render samples so
    /// device speed cannot change their geometry mix or timing distribution.
    pub fn near_tree_readiness_delay(
        &self,
        cam_pos: DVec3,
        draw_cells: &[DrawCell],
    ) -> vearth_platform::time::Duration {
        let now = Instant::now();
        draw_cells
            .iter()
            .filter_map(|draw| self.resident.get(draw.resident))
            .filter(|cell| {
                cell.trees
                    .iter()
                    .all(|tree| tree.origin.distance(cam_pos) < NEAR_TREE_FADE_OUTER_M)
            })
            .flat_map(|cell| &cell.trees)
            .filter_map(|tree| match self.tree_assets.get(&tree.profile) {
                Some(TreeAssetState::Ready(asset)) => Some(
                    TREE_READINESS_FADE
                        .saturating_sub(now.saturating_duration_since(asset.ready_at)),
                ),
                _ => None,
            })
            .max()
            .unwrap_or_default()
    }

    /// Queue only species that can enter the near-tree transition band.
    /// Streaming may retain many distant cells; they must not create
    /// background asset work merely by becoming resident.
    pub fn prepare_near_tree_assets(&mut self, cam_pos: DVec3, draw_cells: &[DrawCell]) {
        let needed: HashSet<_> = draw_cells
            .iter()
            .filter_map(|draw| self.resident.get(draw.resident))
            .flat_map(|cell| &cell.trees)
            .filter(|tree| tree.origin.distance(cam_pos) <= NEAR_TREE_FADE_OUTER_M)
            .map(|tree| tree.profile)
            .collect();
        for profile in needed {
            self.request_tree_profile(profile);
        }
        self.collect_tree_assets();
    }

    /// Upload one cell and return its exact logical vertex+index buffer bytes.
    pub fn upload_prepared_cell(
        &mut self,
        resident: ResidentId,
        prepared: &PreparedRenderCell,
    ) -> u64 {
        self.upload_packed_cell(resident, prepared)
    }

    /// Upload a worker-validated artifact. This path performs only bounded
    /// metadata checks, arena allocation, byte copies, and registration: it
    /// never scans vertices, generates meshes, packs vertices, or narrows
    /// indices.
    pub fn upload_packed_cell(&mut self, resident: ResidentId, packed: &PackedRenderCell) -> u64 {
        let upload_started = Instant::now();
        let arena_pages_before = self.packed_arena.pages.len() + self.full_arena.pages.len();
        let light_pages_before = self.light_arena.pages.len();
        let mut chunks = Vec::with_capacity(packed.chunks.len());
        for chunk in &packed.chunks {
            assert!(chunk.ranges.len() <= MAX_RENDER_INDEX_RANGES);
            debug_assert!(chunk.ranges.is_empty() || !chunk.has_calibration_landmark);
            let vertex_data = chunk.vertex_bytes();
            let index_data = chunk.index_bytes();
            let dummy_vertex = [0u8; std::mem::size_of::<PackedSurfaceVertex>()];
            let dummy_index = [0u8; 4];
            let vertex_data = if vertex_data.is_empty() {
                &dummy_vertex[..]
            } else {
                vertex_data
            };
            let index_data = if index_data.is_empty() {
                &dummy_index[..]
            } else {
                index_data
            };
            let shadow_data = chunk.shadow.as_ref().map_or(&[][..], |shadow| {
                assert_eq!(shadow.indices.format(), chunk.indices.format());
                assert!(shadow.ranges.len() <= MAX_RENDER_INDEX_RANGES);
                shadow.indices.bytes()
            });
            let indices_u16 = chunk.indices.format() == TerrainIndexFormat::Uint16;
            let packed_vertices = chunk.vertices.format() == TerrainVertexFormat::Packed12;
            let index_alignment = if indices_u16 { 2 } else { 4 };
            let allocation = if packed_vertices {
                self.packed_arena.allocate_and_upload(
                    &self.device,
                    &mut self.buffer_uploads,
                    vertex_data,
                    index_data,
                    shadow_data,
                    index_alignment,
                )
            } else {
                self.full_arena.allocate_and_upload(
                    &self.device,
                    &mut self.buffer_uploads,
                    vertex_data,
                    index_data,
                    shadow_data,
                    index_alignment,
                )
            };
            let frame = ChunkFrame::of(chunk.addr);
            chunks.push(ChunkGpu {
                detail: chunk.detail.map(|detail| {
                    (
                        frame
                            .model_matrix()
                            .transform_point3(DVec3::from_array(detail.center.map(f64::from))),
                        f64::from(detail.radius_m),
                        f64::from(detail.min_screen_px),
                    )
                }),
                detail_visible: std::cell::Cell::new(false),
                allocation,
                vertex_bytes: allocation.vertex_bytes,
                index_bytes: allocation.index_bytes,
                index_count: chunk.indices.len() as u32,
                ranges: chunk.ranges.clone(),
                material_ranges: if self.material_stats_enabled {
                    material_range_counts(&chunk.vertices, &chunk.indices, &chunk.ranges)
                } else {
                    Vec::new()
                },
                shadow: chunk.shadow.as_ref().map(|shadow| ShadowIndexGpu {
                    first_index: (index_stream_layout(
                        index_data.len() as u64,
                        shadow_data.len() as u64,
                    )
                    .0 / index_alignment) as u32,
                    index_count: shadow.indices.len() as u32,
                    ranges: shadow.ranges.clone(),
                    max_deviation_m: shadow.max_deviation_m,
                }),
                indices_u16,
                packed_vertices,
                model: frame.model_matrix(),
                construction_height_phase: construction_material_origin(
                    frame.model_matrix().w_axis.truncate(),
                )[1],
                axes: frame.face_normals(),
                normal_parameters: world_normal_parameters(
                    frame
                        .model_matrix()
                        .to_cols_array()
                        .map(|value| value as f32),
                ),
                bound_center: DVec3::from_array(chunk.bounds.center_ecef),
                bound_radius: chunk.bounds.radius_m,
                bound_box: (!chunk.has_calibration_landmark)
                    .then(|| RenderBoxGpu::new(&frame, chunk.local_bounds)),
                has_calibration_landmark: chunk.has_calibration_landmark,
                has_fragment_feedback: chunk.has_fragment_feedback,
                has_rare_surface: chunk.has_rare_surface,
            });
        }
        let new_arena_pages =
            self.packed_arena.pages.len() + self.full_arena.pages.len() - arena_pages_before;
        let lights = packed.night_lights.as_ref().map(|lights| {
            let allocation = self.light_arena.allocate_and_upload(
                &self.device,
                &mut self.buffer_uploads,
                bytemuck::cast_slice(&lights.vertices),
            );
            let frame = ChunkFrame::of(ChunkAddr {
                cell: packed.cell(),
                k: 0,
            });
            NightLightGpu {
                allocation,
                vertex_count: lights.vertices.len() as u32,
                model: frame.model_matrix(),
                axes: frame.face_normals(),
                bound_center: DVec3::from_array(lights.bounds.center_ecef),
                bound_radius: lights.bounds.radius_m,
            }
        });
        let new_light_pages = self.light_arena.pages.len() - light_pages_before;
        let trees = packed
            .tree_instances
            .iter()
            .copied()
            .map(live_tree_instance)
            .collect();
        let flora_frame = packed.flora_bounds.map(|bounds| {
            let frame = ChunkFrame::of(ChunkAddr {
                cell: packed.cell(),
                k: 0,
            });
            FloraFrameGpu {
                model: frame.model_matrix(),
                bound_center: DVec3::from_array(bounds.center_ecef),
                bound_radius: bounds.radius_m,
                local_y_min: bounds.local_y_min,
                local_y_max: bounds.local_y_max,
            }
        });
        let cell_gpu = CellGpu {
            upload_generation: 0,
            horizon: vearth_core::lod::CellHorizon::new(packed.cell()),
            chunks,
            lights,
            trees,
            flora: packed.flora.clone(),
            flora_frame,
        };
        let footprint = MeshFootprint::from_cell(&cell_gpu);
        self.retain_tree_profiles(&cell_gpu.trees);
        if let Some(old) = self.resident.insert(resident, cell_gpu) {
            self.release_cell(old);
        }
        self.mesh_stats.add_world_cell(footprint);
        self.refresh_arena_capacity();
        self.mesh_stats.vertex_buffers += new_arena_pages;
        self.mesh_stats.index_buffers += new_arena_pages;
        self.mesh_stats.vertex_buffers += new_light_pages;
        self.pending_uploads.record_world_cell(footprint);
        self.pending_uploads.vertex_buffers += new_arena_pages;
        self.pending_uploads.index_buffers += new_arena_pages;
        self.pending_uploads.vertex_buffers += new_light_pages;
        let queue_upload_ns = self.buffer_uploads.take_cpu_write_ns();
        let total_ns = upload_started.elapsed().as_nanos() as u64;
        self.pending_uploads.queue_upload_ns = self
            .pending_uploads
            .queue_upload_ns
            .saturating_add(queue_upload_ns);
        self.pending_uploads.preparation_ns = self
            .pending_uploads
            .preparation_ns
            .saturating_add(total_ns.saturating_sub(queue_upload_ns));
        footprint.total_bytes()
    }

    fn refresh_arena_capacity(&mut self) {
        self.mesh_stats.arena_capacity_bytes = self.packed_arena.pages.capacity_bytes
            + self.full_arena.pages.capacity_bytes
            + self.light_arena.pages.capacity_bytes;
    }

    fn release_cell(&mut self, cell: CellGpu) {
        let footprint = MeshFootprint::from_cell(&cell);
        for chunk in &cell.chunks {
            let released = if chunk.packed_vertices {
                self.packed_arena.free(chunk.allocation)
            } else {
                self.full_arena.free(chunk.allocation)
            };
            if released {
                self.mesh_stats.vertex_buffers -= 1;
                self.mesh_stats.index_buffers -= 1;
            }
        }
        if let Some(light) = &cell.lights
            && self.light_arena.free(light.allocation)
        {
            self.mesh_stats.vertex_buffers -= 1;
        }
        self.release_tree_profiles(&cell.trees);
        self.mesh_stats.remove_world_cell(footprint);
        self.refresh_arena_capacity();
    }

    pub fn evict(&mut self, residents: &[ResidentId]) {
        for &resident in residents {
            if let Some(old) = self.resident.remove(resident) {
                self.release_cell(old);
            }
        }
    }

    /// Enable fixed-size completion batches for replay attribution. This does
    /// not change query issue cadence, slot reuse, or nonblocking collection.
    pub(crate) fn set_gpu_timing_capture(&mut self, enabled: bool) {
        if let Some(timing) = self.gpu_timing.as_mut() {
            timing.capture_completed = enabled;
            if !enabled {
                timing.completed = None;
            }
        }
    }

    pub fn contains_resident(&self, resident: ResidentId) -> bool {
        self.resident.contains(resident)
    }

    pub fn resident_count(&self) -> usize {
        self.resident.len()
    }

    /// Encode one frame into `target`. `draw_cells` must be resident.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        width: u32,
        height: u32,
        cam_pos: DVec3,
        view_rot: DMat4,
        fov_y: f64,
        draw_cells: &[DrawCell],
        lighting: Lighting,
        weather: WeatherRenderState,
        player: Option<&PlayerDraw>,
    ) -> FrameStats {
        self.prepare_near_tree_assets(cam_pos, draw_cells);
        let timing_slot = self
            .gpu_timing
            .as_mut()
            .and_then(|timing| timing.prepare_frame(&self.device));
        let gpu_timings = self.gpu_timing.as_ref().and_then(|timing| timing.latest);
        let gpu_timing_sample_id = self
            .gpu_timing
            .as_ref()
            .and_then(|timing| timing.latest.map(|_| timing.latest_frame_id));
        let cloud_active = self.clouds_visible && weather.surface[3] >= 0.0;

        let hdr_rebuilt = self
            .hdr
            .as_ref()
            .is_none_or(|hdr| hdr.width != width || hdr.height != height || hdr.depth != *depth);
        if hdr_rebuilt {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("HDR scene"),
                size: wgpu::Extent3d {
                    width: width.max(1),
                    height: height.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let tone_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("tone-map resources"),
                layout: &self.tone_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&self.grade_lut_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.grade_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(depth),
                    },
                ],
            });
            self.hdr = Some(HdrTarget {
                multisample: self.world_aa.targets(
                    &self.device,
                    width,
                    height,
                    HDR_FORMAT,
                    DEPTH_FORMAT,
                ),
                width,
                height,
                _texture: texture,
                view,
                tone_bg,
                depth: depth.clone(),
            });
        }
        let hdr_view = self
            .hdr
            .as_ref()
            .expect("HDR target initialized")
            .view
            .clone();
        let tone_bg = self
            .hdr
            .as_ref()
            .expect("HDR target initialized")
            .tone_bg
            .clone();

        let (cloud_width, cloud_height) = cloud_target_dimensions(width, height);
        let cloud_targets_rebuilt = cloud_active
            && self.cloud_targets.as_ref().is_none_or(|targets| {
                targets.width != cloud_width || targets.height != cloud_height
            });
        if cloud_targets_rebuilt {
            self.cloud_targets = Some(create_cloud_targets(
                &self.device,
                &self.cloud_sample_bgl,
                &self.cloud_sampler,
                width,
                height,
            ));
            self.cloud_history = None;
            self.cloud_frame_index = 0;
        }

        let atmo_bind_group_rebuilt = self
            .atmo_depth_bg
            .as_ref()
            .is_none_or(|(cached_depth, _)| cached_depth != depth);
        if atmo_bind_group_rebuilt {
            let depth_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("atmosphere sky resources"),
                layout: &self.atmo_depth_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&self.milky_way_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.milky_way_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&self.atmo_lut_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(&self.atmo_lut_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(&self.climate_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::Sampler(&self.climate_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: self.sky_model_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 8,
                        resource: wgpu::BindingResource::TextureView(&self.cloud_shape_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 9,
                        resource: wgpu::BindingResource::Sampler(&self.cloud_shape_sampler),
                    },
                ],
            });
            self.atmo_depth_bg = Some((depth.clone(), depth_bg));
        }

        let aspect = width as f64 / height as f64;
        let proj =
            glam::dcamera::rh::proj::directx::perspective_infinite_reverse(fov_y, aspect, 0.05);
        let view_proj = proj * view_rot;
        let vp32 = Mat4::from_cols_array(&view_proj.to_cols_array().map(|v| v as f32));

        let previous_cloud_state = self.cloud_history;
        let cloud_history_valid = cloud_active
            && !cloud_targets_rebuilt
            && previous_cloud_state.is_some_and(|previous| {
                cloud_history_compatible(previous, cam_pos, weather, lighting)
            });
        let previous_view_proj =
            previous_cloud_state.map_or(view_proj, |previous| previous.view_proj);
        let camera_to_previous =
            previous_cloud_state.map_or(DVec3::ZERO, |previous| cam_pos - previous.camera_ecef);
        let drift_elapsed = self
            .cloud_drift_utc
            .map_or(0.0, |previous| lighting.utc_seconds - previous);
        self.cloud_drift = advance_cloud_drift(
            self.cloud_drift,
            cloud_wind_ecef(cam_pos, weather.wind_low),
            drift_elapsed,
        );
        self.cloud_drift_utc = Some(lighting.utc_seconds);
        let history_sample_cap = previous_cloud_state.map_or(32.0, |previous| {
            cloud_history_sample_cap(cloud_appearance_step(
                previous.weather,
                weather,
                previous.lighting,
                lighting,
            ))
        });

        let inv_vp = view_proj.inverse();
        let ground_r = vearth_core::ellipsoid::surface_point_along(cam_pos).length();
        let surface_point = vearth_core::ellipsoid::surface_point_along(cam_pos);
        let observer_up = vearth_core::ellipsoid::geodetic_normal(surface_point);
        let solar_sine = observer_up.dot(lighting.direction_ecef) as f32;
        let sun_shadow_preparation_started = Instant::now();
        let camera_altitude_m = cam_pos.distance(surface_point);
        let shadow_policy_active = sun_shadow_policy_enabled(
            self.sun_shadows.configured,
            self.shadows,
            f64::from(solar_sine),
            camera_altitude_m,
            weather.weather5[3],
        );
        let desired_sun_shadow_cascades = if shadow_policy_active {
            if self
                .sun_shadows
                .snap_anchors_ecef
                .iter()
                .any(Option::is_none)
            {
                self.sun_shadows.snap_anchors_ecef = [Some(cam_pos); SUN_SHADOW_CASCADES];
            }
            let mut shadow_snap_anchors = self
                .sun_shadows
                .snap_anchors_ecef
                .map(|anchor| anchor.expect("Sun shadow snap anchors initialized"));
            let mut desired = build_sun_shadow_cascades(
                cam_pos,
                view_rot,
                fov_y,
                aspect,
                lighting.direction_ecef,
                camera_altitude_m,
                shadow_snap_anchors,
            );
            if let Some(provisional) = desired
                && shadow_snap_anchors
                    .iter()
                    .zip(provisional)
                    .any(|(anchor, cascade)| anchor.distance(cam_pos) > cascade.radius_m)
            {
                for index in 0..SUN_SHADOW_CASCADES {
                    let cascade = provisional[index];
                    if shadow_snap_anchors[index].distance(cam_pos) > cascade.radius_m {
                        let texel_m = 2.0 * cascade.radius_m / f64::from(SUN_SHADOW_RESOLUTION);
                        // Keep the anchor within one cascade footprint, which
                        // bounds basis-rotation error in texels independently of
                        // physical cascade size. Moving by integer light-space
                        // texels preserves the existing grid phase at the rebase.
                        shadow_snap_anchors[index] = snap_sun_shadow_center(
                            cam_pos,
                            shadow_snap_anchors[index],
                            cascade.right,
                            cascade.up,
                            texel_m,
                        );
                    }
                }
                self.sun_shadows.snap_anchors_ecef = shadow_snap_anchors.map(Some);
                desired = build_sun_shadow_cascades(
                    cam_pos,
                    view_rot,
                    fov_y,
                    aspect,
                    lighting.direction_ecef,
                    camera_altitude_m,
                    shadow_snap_anchors,
                );
            }
            desired
        } else {
            None
        };
        let receiver_plan = if self.sun_shadows.receiver_culling {
            desired_sun_shadow_cascades.map_or_else(SunShadowReceiverPlan::default, |cascades| {
                plan_sun_shadow_receivers(
                    &self.sun_shadows.rendered_cascades,
                    &self.sun_shadows.rendered_receiver_support,
                    &mut self.sun_shadows.receiver_recovery,
                    &cascades,
                    cam_pos,
                    view_rot,
                    fov_y,
                    aspect,
                    self.sun_shadows.update_frame,
                )
            })
        } else {
            SunShadowReceiverPlan::default()
        };
        // A narrowed cached map may be reused only while its stored support
        // contains every currently sampled receiver and its filter footprint.
        let receiver_refresh_mask = receiver_plan.forced_refresh_mask;
        let mut sun_shadow_preparation_ns = sun_shadow_preparation_started.elapsed().as_nanos();
        let mut sky_lighting = sky_model_lighting(
            solar_sine,
            weather.surface[2],
            weather.weather0[1],
            weather.weather1[1],
        );
        let palette_solar_sine = cam_pos.normalize().dot(lighting.direction_ecef) as f32;
        let view_sine = 3.0_f32.to_radians().sin();
        let effective_solar_sine = palette_solar_sine.max(0.001);
        let sun_horizontal = (1.0 - palette_solar_sine * palette_solar_sine)
            .max(0.0)
            .sqrt();
        let effective_length =
            (sun_horizontal * sun_horizontal + effective_solar_sine * effective_solar_sine).sqrt();
        for (sample, azimuth_cosine) in sky_lighting
            .horizon_radiance
            .iter_mut()
            .zip([1.0, 0.0, -1.0])
        {
            let cosine_gamma = (view_sine * effective_solar_sine
                + 3.0_f32.to_radians().cos() * sun_horizontal * azimuth_cosine)
                / effective_length;
            let graded = sunset_sky_grade_cpu(
                [sample[0], sample[1], sample[2]],
                view_sine,
                palette_solar_sine,
                cosine_gamma,
                camera_altitude_m as f32,
                weather.surface[2],
                weather.weather0[1],
            );
            sample[..3].copy_from_slice(&graded);
        }
        self.queue.write_buffer(
            &self.sky_model_buf,
            0,
            bytemuck::bytes_of(&sky_lighting.uniform),
        );
        self.cloud_shadows.update(
            &self.queue,
            cam_pos,
            lighting.direction_ecef,
            ground_r,
            cloud_active,
            weather.weather5[0],
        );
        let sky_visibility = SkyVisibility::at(cam_pos, lighting);
        let base_exposure = {
            let x = ((observer_up.dot(lighting.direction_ecef) + 0.10) / 0.30).clamp(0.0, 1.0);
            let daylight = x * x * (3.0 - 2.0 * x);
            // A pristine moonless site permits more visual adaptation than a
            // city or moonlit sky. Keep the range restrained: this is an
            // unaided-eye simulation, not a long-exposure camera response.
            let dark_quality =
                f64::from(((sky_visibility.limiting_magnitude - 4.5) / 3.1).clamp(0.0, 1.0));
            let dark_exposure = 1.55 + 0.58 * dark_quality;
            dark_exposure + (0.88 - dark_exposure) * daylight
        };
        // Bright aerosol and humid haze prompt visual/camera adaptation. A
        // small exposure reduction preserves chromatic contrast instead of
        // letting the filmic shoulder compress every wavelength toward white.
        let visibility_haze =
            (18_000.0 / f64::from(weather.weather0[2]).max(120.0)).clamp(0.0, 1.0);
        let haze_adaptation = f64::from(weather.surface[2]) * 0.14
            + f64::from(weather.weather0[1]) * visibility_haze * 0.11;
        let exposure = base_exposure * (1.0 - haze_adaptation.clamp(0.0, 0.22));
        // The catalog is magnitude-sorted. Avoid processing tens of thousands
        // of faint vertices when the current city/Moon/twilight budget could
        // not reveal them; directional domes only reduce this upper bound.
        let star_draw_count =
            crate::sky::star_draw_count(&self.star_magnitudes, sky_visibility.limiting_magnitude);
        let (sin_gmst, cos_gmst) = greenwich_sidereal_time(lighting.utc_seconds).sin_cos();
        self.queue.write_buffer(
            &self.globals_buf,
            0,
            bytemuck::bytes_of(&Globals {
                view_proj: vp32.to_cols_array(),
                sun_dir: [
                    lighting.direction_ecef.x as f32,
                    lighting.direction_ecef.y as f32,
                    lighting.direction_ecef.z as f32,
                    1.0,
                ],
                moon_dir: [
                    lighting.moon_direction_ecef.x as f32,
                    lighting.moon_direction_ecef.y as f32,
                    lighting.moon_direction_ecef.z as f32,
                    lighting.moon_illumination as f32,
                ],
                // Night, twilight, Moon, cloud cover and receiver orientation
                // remain shader-owned; this is the shared daylight baseline.
                ambient: [
                    sky_lighting.daylight_ambient[0],
                    sky_lighting.daylight_ambient[1],
                    sky_lighting.daylight_ambient[2],
                    exposure as f32,
                ],
                inv_view_proj: inv_vp.to_cols_array().map(|v| v as f32),
                planet_center: [
                    -cam_pos.x as f32,
                    -cam_pos.y as f32,
                    -cam_pos.z as f32,
                    ground_r as f32,
                ],
                viewport: [
                    width.max(1) as f32,
                    height.max(1) as f32,
                    (fov_y * 0.5).tan() as f32,
                    f32::from(self.building_night_lights_enabled),
                ],
                sky: [
                    cos_gmst as f32,
                    sin_gmst as f32,
                    lighting.utc_seconds.rem_euclid(4096.0) as f32,
                    sky_visibility.local_glow,
                ],
                horizon_glow_0: sky_visibility.horizon_glow[0..4]
                    .try_into()
                    .expect("four horizon samples"),
                horizon_glow_1: sky_visibility.horizon_glow[4..8]
                    .try_into()
                    .expect("four horizon samples"),
                weather0: weather.weather0,
                weather1: weather.weather1,
                wind: weather.wind,
                surface: weather.surface,
                weather2: weather.weather2,
                weather3: weather.weather3,
                weather4: weather.weather4,
                weather5: weather.weather5,
                weather6: weather.weather6,
                wind_low: weather.wind_low,
                wind_mid: weather.wind_mid,
                wind_upper: weather.wind_upper,
                prev_view_proj: previous_view_proj.to_cols_array().map(|value| value as f32),
                camera_to_prev: [
                    camera_to_previous.x as f32,
                    camera_to_previous.y as f32,
                    camera_to_previous.z as f32,
                    drift_elapsed as f32,
                ],
                material_origin: [
                    cam_pos.x.rem_euclid(8192.0) as f32,
                    cam_pos.y.rem_euclid(8192.0) as f32,
                    cam_pos.z.rem_euclid(8192.0) as f32,
                    f32::from(cloud_active),
                ],
                aerial_sunward: sky_lighting.horizon_radiance[0],
                aerial_side: sky_lighting.horizon_radiance[1],
                aerial_away: sky_lighting.horizon_radiance[2],
                cloud_drift: [
                    self.cloud_drift.x as f32,
                    self.cloud_drift.y as f32,
                    self.cloud_drift.z as f32,
                    history_sample_cap,
                ],
                temporal: [
                    f32::from(cloud_history_valid),
                    // Keep the 4x4 slot index in the low bits, but do not
                    // repeat the same four ray offsets every 64 frames.
                    (self.cloud_frame_index & 4095) as f32,
                    previous_cloud_state.map_or(0.0, |previous| previous.lightning_flash),
                    lighting.utc_seconds.rem_euclid(3_600.0) as f32,
                ],
            }),
        );

        let planes = frustum_planes(&view_proj);
        let tree_pixel_scale = f64::from(height) / (2.0 * (fov_y * 0.5).tan());

        struct VisibleChunk<'a> {
            cell: CellId,
            chunk_index: usize,
            chunk: &'a ChunkGpu,
            fade: f32,
            distance_sq: f64,
        }

        struct VisibleLight<'a> {
            cell: CellId,
            light: &'a NightLightGpu,
            fade: f32,
            distance_sq: f64,
        }

        struct VisibleTree<'a> {
            cell: CellId,
            tree_index: usize,
            lod: usize,
            lod_alpha: f32,
            profile: TreeProfileId,
            asset: &'a TreeAssetGpu,
            model: DMat4,
            axes: [DVec3; 3],
            fade: f32,
            near_blend: f32,
            color_scale: f32,
            distance_sq: f64,
        }

        struct ShadowChunk<'a> {
            key: SunShadowCasterKey,
            cell: CellId,
            chunk_index: usize,
            chunk: &'a ChunkGpu,
            cascade_mask: u8,
            distance_sq: f64,
        }

        let visibility_started = Instant::now();
        let horizon = vearth_core::lod::HorizonCuller::new(cam_pos);
        let mut horizon_draws = Vec::with_capacity(draw_cells.len());
        let mut visible: Vec<VisibleChunk<'_>> = Vec::new();
        let mut visible_lights: Vec<VisibleLight<'_>> = Vec::new();
        let mut visible_trees: Vec<VisibleTree<'_>> = Vec::new();
        let mut shadow_chunks: Vec<ShadowChunk<'_>> = Vec::new();
        let mut shadow_candidates = std::mem::take(&mut self.sun_shadows.candidate_scratch);
        shadow_candidates.clear();
        let mut caster_probes: [SunShadowCasterProbe; SUN_SHADOW_CASCADES] =
            std::array::from_fn(|_| SunShadowCasterProbe::default());
        let caster_probe_epoch = self.sun_shadows.update_frame.wrapping_add(1);
        let population_cascades = desired_sun_shadow_cascades.map(|desired| {
            std::array::from_fn(|index| {
                self.sun_shadows.rendered_cascades[index].unwrap_or(desired[index])
            })
        });
        // Population changes retain the established cadence. Receiver support
        // escape must refresh before reuse, including otherwise ineligible slots.
        let possible_shadow_updates = desired_sun_shadow_cascades.map_or(0, |desired| {
            select_sun_shadow_updates(
                &self.sun_shadows.rendered_cascades,
                &desired,
                0b11,
                self.sun_shadows.active_last_frame,
                self.sun_shadows.update_frame,
                false,
            ) | receiver_refresh_mask
        });
        let mut near_tree_cells: HashMap<CellId, f32> = HashMap::new();
        let mut resident_draw_cells = 0usize;
        let mut missing_draw_cells = 0usize;
        let mut candidate_chunks = 0usize;
        let mut culled = 0usize;
        let mut culled_horizon = 0usize;
        for dc in draw_cells {
            let Some(cg) = self.resident.get(dc.resident) else {
                missing_draw_cells += 1;
                continue;
            };
            resident_draw_cells += 1;
            candidate_chunks += cg.chunks.len();
            // Behind-the-planet test at draw time: the wanted set applies
            // it at streaming cadence (~120 ms), but fallback ancestors and
            // fast camera motion can put fully-hidden cells in the draw
            // list between refreshes.
            if horizon.culled_cap(cg.horizon) {
                culled_horizon += cg.chunks.len();
                continue;
            }
            horizon_draws.push((dc, cg));
            if let Some(light) = &cg.lights
                && night_light_cell_can_contribute(
                    self.building_night_lights_enabled,
                    lighting,
                    light.bound_center,
                    light.bound_radius,
                )
            {
                let rel = light.bound_center - cam_pos;
                if sphere_visible(&planes, rel, light.bound_radius) {
                    visible_lights.push(VisibleLight {
                        cell: dc.cell,
                        light,
                        fade: dc.fade,
                        distance_sq: rel.length_squared(),
                    });
                }
            }
            let near_blend = tree_cell_readiness_blend(&cg.trees, cam_pos, |profile| {
                match self.tree_assets.get(&profile) {
                    Some(TreeAssetState::Ready(asset)) => {
                        Some(visibility_started.saturating_duration_since(asset.ready_at))
                    }
                    _ => None,
                }
            });
            if near_blend > 0.0 {
                near_tree_cells.insert(dc.cell, near_blend);
                for (tree_index, tree) in cg.trees.iter().enumerate() {
                    let Some(TreeAssetState::Ready(asset)) = self.tree_assets.get(&tree.profile)
                    else {
                        continue;
                    };
                    let (model, scale) = live_tree_model(tree, asset);
                    let height = f64::from(asset.height_m) * scale;
                    let center = tree.origin + tree.basis.y_axis * (height * 0.5);
                    let radius = (f64::from(asset.radius_m) * scale).hypot(height * 0.5);
                    let rel = center - cam_pos;
                    if sphere_visible(&planes, rel, radius) {
                        let projected_height = height * tree_pixel_scale / rel.length().max(1.0);
                        let (assignments, count) = tree_lod_assignments(projected_height);
                        for assignment in &assignments[..count] {
                            visible_trees.push(VisibleTree {
                                cell: dc.cell,
                                tree_index,
                                lod: assignment.lod,
                                lod_alpha: assignment.alpha,
                                profile: tree.profile,
                                asset,
                                model,
                                axes: [tree.basis.x_axis, tree.basis.y_axis, tree.basis.z_axis],
                                fade: dc.fade,
                                near_blend,
                                color_scale: tree.color_scale,
                                distance_sq: rel.length_squared(),
                            });
                        }
                    }
                }
            }
            for (chunk_index, ch) in cg.chunks.iter().enumerate() {
                if !ch.admit_detail(cam_pos, tree_pixel_scale) {
                    continue;
                }
                let (bound_center, bound_radius) = landmark_aware_chunk_bounds(
                    ch.bound_center,
                    ch.bound_radius,
                    ch.has_calibration_landmark,
                    self.landmark_delta_ecef,
                );
                let projection_padding =
                    camera_relative_render_padding(bound_center, bound_radius, cam_pos);
                let bound_radius = bound_radius + projection_padding;
                if let (Some(desired), Some(rendered)) =
                    (&desired_sun_shadow_cascades, &population_cascades)
                    && ch.index_count != 0
                    && possible_shadow_updates != 0
                {
                    let key = SunShadowCasterKey {
                        resident: dc.resident,
                        upload_generation: cg.upload_generation,
                        chunk_index,
                    };
                    let rendered_mask = sun_shadow_box_mask_for_updates(
                        rendered,
                        bound_center,
                        bound_radius,
                        ch.bound_box.as_ref(),
                        projection_padding,
                        possible_shadow_updates,
                    );
                    let rendered_mask = sun_shadow_receiver_mask(
                        &self.sun_shadows.rendered_receiver_support,
                        bound_center,
                        bound_radius,
                        ch.bound_box.as_ref(),
                        projection_padding,
                        rendered_mask,
                    );
                    for cascade in 0..SUN_SHADOW_CASCADES {
                        if rendered_mask & (1 << cascade) != 0 {
                            caster_probes[cascade].observe(
                                &mut self.sun_shadows.caster_populations[cascade],
                                key,
                                ch.has_calibration_landmark,
                                caster_probe_epoch,
                            );
                        }
                    }
                    let cascade_mask = sun_shadow_box_mask_for_updates(
                        desired,
                        bound_center,
                        bound_radius,
                        ch.bound_box.as_ref(),
                        projection_padding,
                        possible_shadow_updates,
                    );
                    let cascade_mask = sun_shadow_receiver_mask(
                        &receiver_plan.supports,
                        bound_center,
                        bound_radius,
                        ch.bound_box.as_ref(),
                        projection_padding,
                        cascade_mask,
                    );
                    if cascade_mask != 0 {
                        shadow_candidates.push(SunShadowCandidate {
                            key,
                            cell: dc.cell,
                            cascade_mask,
                            distance_sq: bound_center.distance_squared(cam_pos),
                        });
                    }
                }
                let rel = bound_center - cam_pos;
                if !sphere_visible(&planes, rel, bound_radius)
                    || ch
                        .bound_box
                        .as_ref()
                        .is_some_and(|bounds| !bounds.visible(&planes, rel, projection_padding))
                {
                    culled += 1;
                    continue;
                }
                visible.push(VisibleChunk {
                    cell: dc.cell,
                    chunk_index,
                    chunk: ch,
                    fade: dc.fade,
                    distance_sq: rel.length_squared(),
                });
            }
        }

        let shadow_caster_selection_started = Instant::now();
        let caster_dirty_mask =
            caster_probes
                .iter()
                .enumerate()
                .fold(0_u8, |mask, (cascade, probe)| {
                    mask | (u8::from(!probe.matches(
                        &self.sun_shadows.caster_populations[cascade],
                        self.landmark_delta_ecef,
                    )) << cascade)
                });
        let sun_shadow_update_mask = if let Some(desired) = desired_sun_shadow_cascades {
            let update_mask = select_sun_shadow_updates(
                &self.sun_shadows.rendered_cascades,
                &desired,
                caster_dirty_mask | receiver_plan.reentry_dirty_mask,
                self.sun_shadows.active_last_frame,
                self.sun_shadows.update_frame,
                player.is_some() || self.sun_shadows.caster_populations[0].had_player,
            ) | receiver_refresh_mask;
            for (index, cascade) in desired.iter().copied().enumerate() {
                if update_mask & (1 << index) != 0 {
                    self.sun_shadows.rendered_cascades[index] = Some(cascade);
                    self.queue.write_buffer(
                        &self.sun_shadows.pass_bufs[index],
                        0,
                        bytemuck::bytes_of(&ShadowPassU {
                            light_view_proj: cascade
                                .light_view_proj
                                .to_cols_array()
                                .map(|value| value as f32),
                        }),
                    );
                }
            }
            commit_sun_shadow_receivers(
                &mut self.sun_shadows.rendered_receiver_support,
                &mut self.sun_shadows.receiver_recovery,
                receiver_plan,
                update_mask,
            );
            self.sun_shadows.active_last_frame = true;
            self.sun_shadows.update_frame = self.sun_shadows.update_frame.wrapping_add(1);
            update_mask
        } else {
            self.sun_shadows.active_last_frame = false;
            0
        };
        let rendered_sun_shadow_cascades = if shadow_policy_active
            && let [Some(near), Some(far)] = self.sun_shadows.rendered_cascades
        {
            Some([near, far])
        } else {
            None
        };
        let sun_shadow_active = rendered_sun_shadow_cascades.is_some();
        let shadow_uniform = if let Some(cascades) = &rendered_sun_shadow_cascades {
            let near_end = cascades[0].split_far_m as f32;
            let far_end = cascades[1].split_far_m as f32;
            let inverse_near_depth = (2.0 * cascades[0].depth_extent_m).recip() as f32;
            let near_matrix = sun_shadow_matrix(
                cascades[0].center_ecef - cam_pos,
                cascades[0].right,
                cascades[0].up,
                cascades[0].forward,
                cascades[0].radius_m,
                cascades[0].depth_extent_m,
            );
            let far_matrix = sun_shadow_matrix(
                cascades[1].center_ecef - cam_pos,
                cascades[1].right,
                cascades[1].up,
                cascades[1].forward,
                cascades[1].radius_m,
                cascades[1].depth_extent_m,
            );
            SunShadowU {
                light_view_proj_0: near_matrix.to_cols_array().map(|value| value as f32),
                light_view_proj_1: far_matrix.to_cols_array().map(|value| value as f32),
                splits: [near_end, far_end, near_end * 0.84, far_end * 0.90],
                tuning: [
                    1.0 / SUN_SHADOW_RESOLUTION as f32,
                    self.shadows.strength(),
                    0.35 * inverse_near_depth,
                    0.75 * inverse_near_depth,
                ],
            }
        } else {
            SunShadowU {
                light_view_proj_0: DMat4::IDENTITY.to_cols_array().map(|value| value as f32),
                light_view_proj_1: DMat4::IDENTITY.to_cols_array().map(|value| value as f32),
                splits: [0.0; 4],
                tuning: [1.0 / SUN_SHADOW_RESOLUTION as f32, 0.0, 0.0, 0.0],
            }
        };
        self.queue.write_buffer(
            &self.sun_shadows.receiver_buf,
            0,
            bytemuck::bytes_of(&shadow_uniform),
        );
        for candidate in &shadow_candidates {
            let cascade_mask = candidate.cascade_mask & sun_shadow_update_mask;
            if cascade_mask != 0 {
                let cell = self
                    .resident
                    .get(candidate.key.resident)
                    .expect("classified shadow resident remains live for this frame");
                let chunk = &cell.chunks[candidate.key.chunk_index];
                shadow_chunks.push(ShadowChunk {
                    key: candidate.key,
                    cell: candidate.cell,
                    chunk_index: candidate.key.chunk_index,
                    chunk,
                    cascade_mask,
                    distance_sq: candidate.distance_sq,
                });
            }
        }
        sun_shadow_preparation_ns += shadow_caster_selection_started.elapsed().as_nanos();

        let flora_preparation_started = Instant::now();
        let mut visible_flora: Vec<VisibleFloraUnit<'_>> = Vec::new();
        for &(dc, cg) in &horizon_draws {
            let Some(frame) = cg.flora_frame else {
                continue;
            };
            let rel = frame.bound_center - cam_pos;
            if !sphere_visible(&planes, rel, frame.bound_radius) {
                continue;
            }
            // Distance to the closest point in the flora-bearing cell prism
            // admits an intersecting edge without treating a large cell's
            // horizontal bound as if vegetation extended upward to camera.
            let distance_m = flora_frame_distance(frame, cam_pos);
            let (assignments, assignment_count) = flora_lod_assignments(distance_m);
            if assignment_count == 0 {
                continue;
            }
            for (descriptor_index, descriptor) in cg.flora.iter().enumerate() {
                visible_flora.push(VisibleFloraUnit {
                    cell: dc.cell,
                    descriptor_index,
                    descriptor,
                    frame,
                    terrain_fade: dc.fade,
                    distance_m,
                    assignments,
                    assignment_count,
                });
            }
        }
        let resident_flora_cells = self.mesh_stats.flora_cells;
        let mut prepared_flora = prepare_flora(visible_flora, cam_pos, resident_flora_cells);
        prepared_flora.stats.cpu_preparation_ms =
            flora_preparation_started.elapsed().as_secs_f64() * 1e3;
        let cpu_visibility_ms = visibility_started.elapsed().as_secs_f64() * 1e3;

        // The app's fade map is hash-backed, so its iteration order changes
        // across processes. Front-to-back order makes admission deterministic
        // and improves early depth rejection; canonical address + k-order are
        // stable tie-breakers. Depth testing keeps visual output unchanged.
        let sorting_started = Instant::now();
        visible.sort_unstable_by(|a, b| {
            a.chunk
                .detail
                .is_some()
                .cmp(&b.chunk.detail.is_some())
                .then_with(|| a.distance_sq.total_cmp(&b.distance_sq))
                .then_with(|| a.cell.cmp(&b.cell))
                .then_with(|| a.chunk_index.cmp(&b.chunk_index))
        });
        visible_lights.sort_unstable_by(|a, b| {
            a.distance_sq
                .total_cmp(&b.distance_sq)
                .then_with(|| a.cell.cmp(&b.cell))
        });
        visible_trees.sort_unstable_by(|a, b| {
            a.distance_sq
                .total_cmp(&b.distance_sq)
                .then_with(|| a.cell.cmp(&b.cell))
                .then_with(|| a.lod.cmp(&b.lod))
        });
        shadow_chunks.sort_unstable_by(|a, b| {
            a.chunk
                .detail
                .is_some()
                .cmp(&b.chunk.detail.is_some())
                .then_with(|| {
                    a.cascade_mask
                        .trailing_zeros()
                        .cmp(&b.cascade_mask.trailing_zeros())
                })
                .then_with(|| a.distance_sq.total_cmp(&b.distance_sq))
                .then_with(|| a.cell.cmp(&b.cell))
                .then_with(|| a.chunk_index.cmp(&b.chunk_index))
        });
        let (admitted_shadow_chunks, sun_shadow_capacity_drops) =
            sun_shadow_capacity(shadow_chunks.len());
        shadow_chunks.truncate(admitted_shadow_chunks);
        if sun_shadow_capacity_drops > 0 {
            if !self.sun_shadow_overflow_active {
                log::error!(
                    "Sun shadow caster capacity exceeded: explicitly dropping {sun_shadow_capacity_drops} distant chunk(s) above {MAX_SHADOW_CHUNKS}"
                );
            }
            self.sun_shadow_overflow_active = true;
        } else if self.sun_shadow_overflow_active {
            log::info!("Sun shadow caster population recovered below {MAX_SHADOW_CHUNKS}");
            self.sun_shadow_overflow_active = false;
        }
        if visible_trees.len() > MAX_TREE_DRAWS {
            log::warn!(
                "near-tree draw capacity exceeded: dropping {} distant instance(s)",
                visible_trees.len() - MAX_TREE_DRAWS
            );
            // Admission is per cell, matching the fallback's transition
            // uniform. Dropping one instance must never leave a hole where
            // that cell has already hidden its streamed tree surfaces.
            let accepted =
                admit_tree_cells(visible_trees.iter().map(|tree| tree.cell), MAX_TREE_DRAWS);
            near_tree_cells.retain(|cell, _| accepted.contains(cell));
            visible_trees.retain(|tree| accepted.contains(&tree.cell));
        }
        let dropped_light_cells = visible_lights.len().saturating_sub(MAX_LIGHT_DRAWS);
        if dropped_light_cells > 0 {
            log::error!(
                "night-light draw capacity exceeded: dropping {dropped_light_cells} cell(s)"
            );
            visible_lights.truncate(MAX_LIGHT_DRAWS);
        }
        let visible_chunks = visible.len();
        let cpu_sorting_ms = sorting_started.elapsed().as_secs_f64() * 1e3;

        // Reserve capacity for the on-screen player before admitting world
        // chunks. Previously an overflowing world silently consumed all slots
        // and made the drone disappear.
        let mut invalid_player_parts = 0usize;
        let requested_player_parts = player.map_or(&[][..], |pd| pd.parts.as_slice());
        let mut admitted_player_parts: Vec<usize> =
            Vec::with_capacity(requested_player_parts.len());
        for &pi in requested_player_parts {
            if self
                .player_parts
                .get(pi)
                .is_some_and(|p| p.index_count != 0)
            {
                admitted_player_parts.push(pi);
            } else {
                invalid_player_parts += 1;
            }
        }
        let player_parts_requested = admitted_player_parts.len();
        let admission = admit_draws(visible.len(), player_parts_requested);
        let dropped_chunks = admission.dropped_world;
        admitted_player_parts.truncate(admission.player);
        let dropped_player_parts = admission.dropped_player;
        visible.truncate(admission.world);

        let dropped_this_frame = dropped_chunks + dropped_player_parts;
        if dropped_this_frame > 0 {
            self.draw_overflow_total += dropped_this_frame as u64;
            if !self.draw_overflow_active {
                log::error!(
                    "renderer draw capacity exceeded: {} visible world chunks + {} player parts, capacity {}; explicitly dropped {} world chunks and {} player parts",
                    visible_chunks,
                    player_parts_requested,
                    MAX_DRAWS,
                    dropped_chunks,
                    dropped_player_parts,
                );
            }
            self.draw_overflow_active = true;
        } else if self.draw_overflow_active {
            log::info!("renderer draw capacity recovered below {MAX_DRAWS} slots");
            self.draw_overflow_active = false;
        }
        if invalid_player_parts > 0 {
            log::warn!(
                "renderer skipped {invalid_player_parts} invalid or empty player-part draw(s)"
            );
        }

        let submitted_chunks = visible.len();
        let submitted_landmark_chunks = visible
            .iter()
            .filter(|candidate| candidate.chunk.has_calibration_landmark)
            .count();
        let submitted_u16_index_chunks = visible
            .iter()
            .filter(|candidate| candidate.chunk.indices_u16)
            .count();
        let draw_construction_started = Instant::now();
        let mut draws: Vec<WorldDrawRange<'_>> =
            Vec::with_capacity(visible.len().saturating_mul(6));
        let mut prepared_visible = Vec::with_capacity(visible.len());
        let mut ordinary_slot = 0u32;
        let mut landmark_slot = 0u32;
        for candidate in &visible {
            let landmark = candidate.chunk.has_calibration_landmark;
            let slot = if landmark {
                let slot = landmark_slot;
                landmark_slot += 1;
                slot
            } else {
                let slot = ordinary_slot;
                ordinary_slot += 1;
                slot
            };
            // Spatial ranges share the admitted parent model slot. Culling
            // them here cannot displace another parent at the draw limit.
            append_world_draw_ranges(
                &mut draws,
                candidate.chunk,
                slot,
                landmark,
                opaque_world_eligible(candidate.chunk, candidate.fade),
                |range| {
                    let (center, radius) = render_range_bounds(range, cam_pos);
                    sphere_visible(&planes, center - cam_pos, radius)
                },
            );
            prepared_visible.push((candidate, slot, landmark));
        }
        debug_assert!(draws.len() <= MAX_WORLD_COMMANDS);
        let world_material_indices = self
            .material_stats_enabled
            .then(|| submitted_material_indices(&draws));
        let cpu_world_draw_construction_ms =
            draw_construction_started.elapsed().as_secs_f64() * 1e3;

        let uniforms_started = Instant::now();
        let admitted_draws = submitted_chunks + admitted_player_parts.len();
        let mut uniforms: Vec<u8> = Vec::with_capacity(admitted_draws * TERRAIN_U_SIZE as usize);
        let mut landmark_uniforms: Vec<u8> = Vec::new();
        for (candidate, slot, landmark) in prepared_visible {
            let ch = candidate.chunk;
            let mut model_rel = ch.model;
            model_rel.w_axis.x -= cam_pos.x;
            model_rel.w_axis.y -= cam_pos.y;
            model_rel.w_axis.z -= cam_pos.z;
            let model = model_rel.to_cols_array().map(|value| value as f32);
            let bx = [
                ch.axes[0].x as f32,
                ch.axes[0].y as f32,
                ch.axes[0].z as f32,
                0.0,
            ];
            let by = [
                ch.axes[1].x as f32,
                ch.axes[1].y as f32,
                ch.axes[1].z as f32,
                ch.construction_height_phase,
            ];
            let bz = [
                ch.axes[2].x as f32,
                ch.axes[2].y as f32,
                ch.axes[2].z as f32,
                0.0,
            ];
            let fade = [
                candidate.fade,
                near_tree_cells
                    .get(&candidate.cell)
                    .copied()
                    .unwrap_or_default(),
                0.0,
                0.0,
            ];
            if landmark {
                let mut landmark_model_rel = self
                    .landmark_delta_ecef
                    .map_or(ch.model, |delta| delta * ch.model);
                landmark_model_rel.w_axis.x -= cam_pos.x;
                landmark_model_rel.w_axis.y -= cam_pos.y;
                landmark_model_rel.w_axis.z -= cam_pos.z;
                let u = LandmarkU {
                    model,
                    bx,
                    by,
                    bz,
                    fade,
                    landmark_model: landmark_model_rel.to_cols_array().map(|value| value as f32),
                };
                debug_assert_eq!(
                    slot,
                    (landmark_uniforms.len() as u64 / LANDMARK_U_SIZE) as u32
                );
                landmark_uniforms.extend_from_slice(bytemuck::bytes_of(&u));
            } else {
                let u = TerrainU {
                    model,
                    bx,
                    by,
                    bz,
                    fade,
                }
                .with_world_normal_parameters(ch.normal_parameters);
                debug_assert_eq!(slot, (uniforms.len() as u64 / TERRAIN_U_SIZE) as u32);
                uniforms.extend_from_slice(bytemuck::bytes_of(&u));
            }
        }

        // Player parts share the chunk uniform buffer (rebased f64 model,
        // frame axes for normals, no LOD fade). Never culled — the follow
        // cam keeps the drone on screen and the mesh is tiny.
        let mut player_draws: Vec<(usize, u32)> = Vec::with_capacity(admitted_player_parts.len());
        if let Some(pd) = player {
            for pi in admitted_player_parts {
                let mut model_rel = pd.model;
                model_rel.w_axis.x -= cam_pos.x;
                model_rel.w_axis.y -= cam_pos.y;
                model_rel.w_axis.z -= cam_pos.z;
                let u = TerrainU {
                    model: model_rel.to_cols_array().map(|v| v as f32),
                    bx: [
                        pd.axes[0].x as f32,
                        pd.axes[0].y as f32,
                        pd.axes[0].z as f32,
                        0.0,
                    ],
                    by: [
                        pd.axes[1].x as f32,
                        pd.axes[1].y as f32,
                        pd.axes[1].z as f32,
                        0.0,
                    ],
                    bz: [
                        pd.axes[2].x as f32,
                        pd.axes[2].y as f32,
                        pd.axes[2].z as f32,
                        0.0,
                    ],
                    fade: [1.0, 0.0, 0.0, 0.0],
                };
                let slot = (uniforms.len() as u64 / TERRAIN_U_SIZE) as u32;
                uniforms.extend_from_slice(bytemuck::bytes_of(&u));
                player_draws.push((pi, slot));
            }
        }
        let shadow_models_started = Instant::now();
        for cascade in 0..SUN_SHADOW_CASCADES {
            if sun_shadow_update_mask & (1 << cascade) != 0 {
                let population = &mut self.sun_shadows.caster_populations[cascade];
                population.keys.clear();
                population.landmark_delta = None;
                population.had_player = false;
            }
        }
        let mut shadow_models =
            Vec::with_capacity(shadow_chunks.len() + usize::from(player.is_some()));
        let mut shadow_draws: [Vec<WorldDrawRange<'_>>; SUN_SHADOW_CASCADES] =
            std::array::from_fn(|_| Vec::new());
        let mut submitted_shadow_parent_chunks = 0;
        let mut sun_shadow_caster_chunks_by_cascade = [0; SUN_SHADOW_CASCADES];
        for candidate in &shadow_chunks {
            let mut parent_submitted = false;
            let chunk = candidate.chunk;
            let mut model_rel = chunk.model;
            model_rel.w_axis.x -= cam_pos.x;
            model_rel.w_axis.y -= cam_pos.y;
            model_rel.w_axis.z -= cam_pos.z;
            let mut landmark_model_rel = self
                .landmark_delta_ecef
                .map_or(chunk.model, |delta| delta * chunk.model);
            landmark_model_rel.w_axis.x -= cam_pos.x;
            landmark_model_rel.w_axis.y -= cam_pos.y;
            landmark_model_rel.w_axis.z -= cam_pos.z;
            let slot = shadow_models.len() as u32;
            shadow_models.push(ShadowModelU {
                model: model_rel.to_cols_array().map(|value| value as f32),
                landmark_model: landmark_model_rel.to_cols_array().map(|value| value as f32),
            });
            for (cascade, draws) in shadow_draws.iter_mut().enumerate() {
                if candidate.cascade_mask & (1 << cascade) != 0 {
                    let commands_before = draws.len();
                    append_shadow_draw_ranges(
                        draws,
                        chunk,
                        slot,
                        &rendered_sun_shadow_cascades
                            .as_ref()
                            .expect("updated shadows have rendered projections")[cascade],
                        |range| {
                            let (center, radius) = render_range_bounds(range, cam_pos);
                            sun_shadow_cascade_mask_for_updates(
                                rendered_sun_shadow_cascades
                                    .as_ref()
                                    .expect("updated shadows have rendered projections"),
                                center,
                                radius,
                                1 << cascade,
                            ) != 0
                                && self.sun_shadows.rendered_receiver_support[cascade]
                                    .is_none_or(|support| support.intersects(center, |_| radius))
                        },
                    );
                    if draws.len() > commands_before {
                        sun_shadow_caster_chunks_by_cascade[cascade] += 1;
                        parent_submitted = true;
                    }
                    // Cache admitted parents, including one whose tight ranges
                    // all cull. Ranges, projection and receiver support remain
                    // fixed until refresh; parent/upload identity is sufficient.
                    // Parents dropped by capacity remain dirty.
                    let population = &mut self.sun_shadows.caster_populations[cascade];
                    population.keys.insert(candidate.key, caster_probe_epoch);
                    if chunk.has_calibration_landmark {
                        population.landmark_delta =
                            shadow_landmark_delta(true, self.landmark_delta_ecef);
                    }
                }
            }
            submitted_shadow_parent_chunks += usize::from(parent_submitted);
        }
        // The player uses one rigid model for all authored mesh parts and
        // casts only into the high-detail near cascade.
        let shadow_player_slot = if sun_shadow_update_mask & 1 != 0
            && !player_draws.is_empty()
            && let Some(pd) = player
        {
            let mut model_rel = pd.model;
            model_rel.w_axis.x -= cam_pos.x;
            model_rel.w_axis.y -= cam_pos.y;
            model_rel.w_axis.z -= cam_pos.z;
            let model = model_rel.to_cols_array().map(|value| value as f32);
            let slot = shadow_models.len() as u32;
            shadow_models.push(ShadowModelU {
                model,
                landmark_model: model,
            });
            Some(slot)
        } else {
            None
        };
        if sun_shadow_update_mask & 1 != 0 {
            self.sun_shadows.caster_populations[0].had_player = shadow_player_slot.is_some();
        }
        debug_assert!(shadow_models.len() <= MAX_SHADOW_MODELS);
        sun_shadow_preparation_ns += shadow_models_started.elapsed().as_nanos();
        let tree_submission_count = visible_trees.len();
        let tree_instance_count = distinct_tree_instance_count(
            visible_trees
                .iter()
                .map(|tree| (tree.cell, tree.tree_index)),
        );
        let mut tree_groups: BTreeMap<(u16, u8, usize), Vec<&VisibleTree<'_>>> = BTreeMap::new();
        if tree_submission_count >= TREE_INSTANCE_BATCH_THRESHOLD {
            for candidate in &visible_trees {
                tree_groups
                    .entry({
                        let key = tree_batch_key(candidate.profile, candidate.distance_sq);
                        (key.0, key.1, candidate.lod)
                    })
                    .or_default()
                    .push(candidate);
            }
        }
        // Require a meaningful draw reduction, not just enough trees. This
        // keeps unusually heterogeneous stands on the simpler ordered path.
        let tree_instancing_active =
            should_batch_tree_instances(tree_submission_count, tree_groups.len());
        let mut tree_uniforms: Vec<u8> =
            Vec::with_capacity(tree_submission_count * TREE_U_SIZE as usize);
        let mut tree_draws: Vec<(&TreeAssetGpu, std::ops::Range<u32>, std::ops::Range<u32>)> =
            Vec::with_capacity(tree_submission_count);
        let mut append_tree = |candidate: &VisibleTree<'_>| {
            let mut model_rel = candidate.model;
            model_rel.w_axis.x -= cam_pos.x;
            model_rel.w_axis.y -= cam_pos.y;
            model_rel.w_axis.z -= cam_pos.z;
            let u = TreeU {
                model: model_rel.to_cols_array().map(|value| value as f32),
                bx: [
                    candidate.axes[0].x as f32,
                    candidate.axes[0].y as f32,
                    candidate.axes[0].z as f32,
                    candidate.lod_alpha,
                ],
                by: [
                    candidate.axes[1].x as f32,
                    candidate.axes[1].y as f32,
                    candidate.axes[1].z as f32,
                    0.0,
                ],
                bz: [
                    candidate.axes[2].x as f32,
                    candidate.axes[2].y as f32,
                    candidate.axes[2].z as f32,
                    0.0,
                ],
                fade: [
                    candidate.fade,
                    candidate.near_blend,
                    candidate.color_scale,
                    NEAR_TREE_VOXEL_M,
                ],
            };
            let slot = (tree_uniforms.len() as u64 / TREE_U_SIZE) as u32;
            tree_uniforms.extend_from_slice(bytemuck::bytes_of(&u));
            slot
        };
        if tree_instancing_active {
            // The list is already front-to-back. Coarse distance bins preserve
            // that ordering while species grouping makes each run instanced.
            for candidates in tree_groups.into_values() {
                let asset = candidates[0].asset;
                let first = append_tree(candidates[0]);
                for candidate in &candidates[1..] {
                    append_tree(candidate);
                }
                tree_draws.push((
                    asset,
                    asset.lod_indices[candidates[0].lod].clone(),
                    first..first + candidates.len() as u32,
                ));
            }
        } else {
            // At low counts, avoid batching work and retain exact depth order.
            for candidate in &visible_trees {
                let slot = append_tree(candidate);
                tree_draws.push((
                    candidate.asset,
                    candidate.asset.lod_indices[candidate.lod].clone(),
                    slot..slot + 1,
                ));
            }
        }
        // Fade ownership includes offscreen relatives, but only visible lights
        // consume it. Resident membership cannot change during this frame.
        let resident_draws: HashMap<CellId, f32> = if visible_lights.is_empty() {
            HashMap::new()
        } else {
            draw_cells
                .iter()
                .filter(|dc| missing_draw_cells == 0 || self.resident.get(dc.resident).is_some())
                .map(|dc| (dc.cell, dc.fade))
                .collect()
        };
        let mut light_uniforms: Vec<u8> =
            Vec::with_capacity(visible_lights.len() * LIGHT_U_SIZE as usize);
        let mut light_draws: Vec<(&NightLightGpu, u32)> = Vec::with_capacity(visible_lights.len());
        for candidate in visible_lights {
            let light = candidate.light;
            let mut model_rel = light.model;
            model_rel.w_axis.x -= cam_pos.x;
            model_rel.w_axis.y -= cam_pos.y;
            model_rel.w_axis.z -= cam_pos.z;
            let u = LightU {
                model: model_rel.to_cols_array().map(|v| v as f32),
                bx: [
                    light.axes[0].x as f32,
                    light.axes[0].y as f32,
                    light.axes[0].z as f32,
                    0.0,
                ],
                by: [
                    light.axes[1].x as f32,
                    light.axes[1].y as f32,
                    light.axes[1].z as f32,
                    0.0,
                ],
                bz: [
                    light.axes[2].x as f32,
                    light.axes[2].y as f32,
                    light.axes[2].z as f32,
                    0.0,
                ],
                fade: [candidate.fade, 0.0, 0.0, 0.0],
                light_fade: effective_light_fades(candidate.cell, candidate.fade, &resident_draws),
            };
            let slot = (light_uniforms.len() as u64 / LIGHT_U_SIZE) as u32;
            light_uniforms.extend_from_slice(bytemuck::bytes_of(&u));
            light_draws.push((light, slot));
        }
        let cpu_uniform_construction_ms = uniforms_started.elapsed().as_secs_f64() * 1e3;
        let queue_writes_started = Instant::now();
        if !uniforms.is_empty() {
            self.queue.write_buffer(&self.chunk_buf, 0, &uniforms);
        }
        if !landmark_uniforms.is_empty() {
            self.queue
                .write_buffer(&self.landmark_chunk_buf, 0, &landmark_uniforms);
        }
        if !light_uniforms.is_empty() {
            self.queue
                .write_buffer(&self.light_chunk_buf, 0, &light_uniforms);
        }
        if !tree_uniforms.is_empty() {
            self.queue
                .write_buffer(&self.tree_chunk_buf, 0, &tree_uniforms);
        }
        if !shadow_models.is_empty() {
            self.queue.write_buffer(
                &self.sun_shadows.model_buf,
                0,
                bytemuck::cast_slice(&shadow_models),
            );
        }
        if !prepared_flora.instances.is_empty() {
            self.queue.write_buffer(
                &self.flora_instance_buf,
                0,
                bytemuck::cast_slice(&prepared_flora.instances),
            );
        }
        let mut frame_queue_upload_ns = queue_writes_started.elapsed().as_nanos() as u64;

        // Group chunks by arena page and vertex format. Each group binds one
        // shared vertex/index pair and becomes one multi-draw command on
        // adapters supporting non-zero indirect first-instance values.
        let grouping_started = Instant::now();
        let mut grouped: BTreeMap<WorldGroupKey, Vec<WorldDrawRange<'_>>> = BTreeMap::new();
        for &draw in &draws {
            grouped
                .entry((
                    draw.landmark_pipeline,
                    draw.chunk.has_rare_surface,
                    draw.chunk.packed_vertices,
                    draw.chunk.indices_u16,
                    draw.chunk.allocation.page,
                ))
                .or_default()
                .push(draw);
        }
        let mut light_grouped: BTreeMap<usize, Vec<(&NightLightGpu, u32)>> = BTreeMap::new();
        for &(light, slot) in &light_draws {
            light_grouped
                .entry(light.allocation.page)
                .or_default()
                .push((light, slot));
        }
        let shadow_grouping_started = Instant::now();
        let mut shadow_grouped: BTreeMap<(usize, bool, bool, usize), Vec<WorldDrawRange<'_>>> =
            BTreeMap::new();
        for (cascade, draws) in shadow_draws.iter().enumerate() {
            for &draw in draws {
                shadow_grouped
                    .entry((
                        cascade,
                        draw.chunk.packed_vertices,
                        draw.chunk.indices_u16,
                        draw.chunk.allocation.page,
                    ))
                    .or_default()
                    .push(draw);
            }
        }
        sun_shadow_preparation_ns += shadow_grouping_started.elapsed().as_nanos();
        let cpu_grouping_ms = grouping_started.elapsed().as_secs_f64() * 1e3;
        let indirect_started = Instant::now();
        let (indirect_commands, arena_batches, opaque_world_indices) = build_world_indirect_batches(
            grouped,
            draws.len(),
            self.full_arena.vertex_stride,
            self.packed_arena.vertex_stride,
        );
        let shadow_indirect_started = Instant::now();
        let mut shadow_indirect_commands =
            Vec::with_capacity(shadow_draws.iter().map(Vec::len).sum::<usize>());
        let mut shadow_arena_batches = Vec::with_capacity(shadow_grouped.len());
        for ((cascade, packed, indices_u16, page), chunks) in shadow_grouped {
            let command_start = shadow_indirect_commands.len();
            let stride = if packed {
                self.packed_arena.vertex_stride
            } else {
                self.full_arena.vertex_stride
            };
            let index_stride = if indices_u16 { 2 } else { 4 };
            for draw in chunks {
                shadow_indirect_commands.push(wgpu::util::DrawIndexedIndirectArgs {
                    index_count: draw.index_count,
                    instance_count: 1,
                    first_index: (draw.chunk.allocation.index_offset / index_stride) as u32
                        + draw.first_index,
                    base_vertex: (draw.chunk.allocation.vertex_offset / stride) as i32
                        + draw.base_vertex,
                    first_instance: draw.slot,
                });
            }
            shadow_arena_batches.push(ShadowArenaBatch {
                cascade,
                packed,
                indices_u16,
                page,
                command_start,
                command_count: (shadow_indirect_commands.len() - command_start) as u32,
            });
        }
        debug_assert!(shadow_indirect_commands.len() <= MAX_SHADOW_COMMANDS);
        sun_shadow_preparation_ns += shadow_indirect_started.elapsed().as_nanos();
        let mut light_indirect_commands = Vec::with_capacity(light_draws.len());
        let mut light_arena_batches = Vec::with_capacity(light_grouped.len());
        for (page, lights) in light_grouped {
            let command_start = light_indirect_commands.len();
            for (light, slot) in lights {
                light_indirect_commands.push(wgpu::util::DrawIndirectArgs {
                    vertex_count: light.vertex_count,
                    instance_count: 1,
                    first_vertex: (light.allocation.offset / self.light_arena.stride) as u32,
                    first_instance: slot,
                });
            }
            light_arena_batches.push(LightArenaBatch {
                page,
                command_start,
                command_count: (light_indirect_commands.len() - command_start) as u32,
            });
        }
        if self.indirect_first_instance && !indirect_commands.is_empty() {
            self.queue.write_buffer(
                &self.indirect_buf,
                0,
                bytemuck::cast_slice(&indirect_commands),
            );
        }
        if self.indirect_first_instance && !light_indirect_commands.is_empty() {
            self.queue.write_buffer(
                &self.light_indirect_buf,
                0,
                bytemuck::cast_slice(&light_indirect_commands),
            );
        }
        if self.indirect_first_instance && !shadow_indirect_commands.is_empty() {
            self.queue.write_buffer(
                &self.sun_shadows.indirect_buf,
                0,
                bytemuck::cast_slice(&shadow_indirect_commands),
            );
        }
        let cpu_indirect_construction_ms = indirect_started.elapsed().as_secs_f64() * 1e3;
        let cpu_sun_shadow_preparation_ms = sun_shadow_preparation_ns as f64 * 1e-6;

        let mut triangles = 0u64;
        let world_triangles;
        let mut tree_triangles = 0u64;
        // Streaming mesh copies were accumulated while the application
        // admitted cells. Submit them before the render passes in this same
        // command buffer, then immediately replace the encoder for uploads
        // admitted ahead of the next frame.
        let mut encoder = self.buffer_uploads.take_encoder(&self.device);
        if sun_shadow_update_mask != 0 {
            let command_size = std::mem::size_of::<wgpu::util::DrawIndexedIndirectArgs>() as u64;
            let first_updated_cascade = (0..SUN_SHADOW_CASCADES)
                .find(|cascade| sun_shadow_update_mask & (1 << cascade) != 0)
                .expect("non-empty Sun shadow update mask");
            let last_updated_cascade = (0..SUN_SHADOW_CASCADES)
                .rev()
                .find(|cascade| sun_shadow_update_mask & (1 << cascade) != 0)
                .expect("non-empty Sun shadow update mask");
            for cascade in 0..SUN_SHADOW_CASCADES {
                if sun_shadow_update_mask & (1 << cascade) == 0 {
                    continue;
                }
                let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("directional Sun shadow cascade"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.sun_shadows.layer_views[cascade],
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: timing_slot.map(|_| wgpu::RenderPassTimestampWrites {
                        query_set: &self
                            .gpu_timing
                            .as_ref()
                            .expect("a timing slot requires GPU timing resources")
                            .query_set,
                        beginning_of_pass_write_index: (cascade == first_updated_cascade)
                            .then_some(SHADOW_BEGIN_QUERY),
                        end_of_pass_write_index: (cascade == last_updated_cascade)
                            .then_some(SHADOW_END_QUERY),
                    }),
                    ..Default::default()
                });
                rp.set_bind_group(0, &self.sun_shadows.pass_bgs[cascade], &[]);
                rp.set_bind_group(1, &self.sun_shadows.model_bg, &[]);
                for batch in shadow_arena_batches
                    .iter()
                    .filter(|batch| batch.cascade == cascade)
                {
                    rp.set_pipeline(if batch.packed {
                        &self.sun_shadows.packed_pipeline
                    } else {
                        &self.sun_shadows.full_pipeline
                    });
                    let arena = if batch.packed {
                        &self.packed_arena
                    } else {
                        &self.full_arena
                    };
                    let page = &arena.pages[batch.page];
                    rp.set_vertex_buffer(0, page.vertex.slice(..));
                    rp.set_index_buffer(
                        page.index.slice(..),
                        if batch.indices_u16 {
                            wgpu::IndexFormat::Uint16
                        } else {
                            wgpu::IndexFormat::Uint32
                        },
                    );
                    let commands = &shadow_indirect_commands
                        [batch.command_start..batch.command_start + batch.command_count as usize];
                    if self.indirect_first_instance {
                        rp.multi_draw_indexed_indirect(
                            &self.sun_shadows.indirect_buf,
                            batch.command_start as u64 * command_size,
                            batch.command_count,
                        );
                    } else {
                        for command in commands {
                            rp.draw_indexed(
                                command.first_index..command.first_index + command.index_count,
                                command.base_vertex,
                                command.first_instance..command.first_instance + 1,
                            );
                        }
                    }
                }
                if cascade == 0
                    && let Some(slot) = shadow_player_slot
                {
                    rp.set_pipeline(&self.sun_shadows.player_pipeline);
                    for (part_index, _) in &player_draws {
                        let part = &self.player_parts[*part_index];
                        rp.set_vertex_buffer(0, part.vbuf.slice(..));
                        rp.set_index_buffer(part.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                        rp.draw_indexed(0..part.index_count, 0, slot..slot + 1);
                    }
                }
            }
        } else if timing_slot.is_some() {
            // Publish an exact zero-work sample without touching either
            // cached shadow layer or clearing a full-resolution color target.
            let _rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("inactive Sun shadow timing"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.sun_shadows.timing_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: Some(wgpu::RenderPassTimestampWrites {
                    query_set: &self
                        .gpu_timing
                        .as_ref()
                        .expect("a timing slot requires GPU timing resources")
                        .query_set,
                    beginning_of_pass_write_index: Some(SHADOW_BEGIN_QUERY),
                    end_of_pass_write_index: Some(SHADOW_END_QUERY),
                }),
                ..Default::default()
            });
        }
        self.sky_environment.encode(
            &mut encoder,
            timing_slot.map(|_| wgpu::ComputePassTimestampWrites {
                query_set: &self
                    .gpu_timing
                    .as_ref()
                    .expect("a timing slot requires GPU timing resources")
                    .query_set,
                beginning_of_pass_write_index: Some(WORLD_BEGIN_QUERY),
                end_of_pass_write_index: None,
            }),
        );
        self.cloud_shadows.encode(&mut encoder);
        {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("world and procedural flora"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: self
                        .hdr
                        .as_ref()
                        .unwrap()
                        .multisample
                        .as_ref()
                        .map_or(&hdr_view, |targets| &targets.color),
                    resolve_target: (self.world_aa.samples > 1).then_some(&hdr_view),
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.002,
                            g: 0.003,
                            b: 0.008,
                            a: 1.0,
                        }),
                        store: if self.world_aa.samples > 1 {
                            wgpu::StoreOp::Discard
                        } else {
                            wgpu::StoreOp::Store
                        },
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: self
                        .hdr
                        .as_ref()
                        .unwrap()
                        .multisample
                        .as_ref()
                        .map_or(depth, |targets| &targets.depth),
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0), // reversed-Z far
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: timing_slot.map(|_| wgpu::RenderPassTimestampWrites {
                    query_set: &self
                        .gpu_timing
                        .as_ref()
                        .expect("a timing slot requires GPU timing resources")
                        .query_set,
                    beginning_of_pass_write_index: None,
                    end_of_pass_write_index: Some(WORLD_END_QUERY),
                }),
                ..Default::default()
            });
            rp.set_bind_group(0, &self.globals_bg, &[]);
            rp.set_bind_group(2, &self.atlas_bg, &[]);
            rp.set_bind_group(3, &self.sun_shadows.receiver_bg, &[]);
            let command_size = std::mem::size_of::<wgpu::util::DrawIndexedIndirectArgs>() as u64;
            for batch in &arena_batches {
                rp.set_pipeline(match (batch.landmark, batch.packed) {
                    (true, false) => &self.landmark_pipeline,
                    (true, true) => &self.packed_landmark_pipeline,
                    (false, packed) if batch.rare_surfaces => {
                        self.rare_surfaces.select(packed, batch.opaque)
                    }
                    (false, packed) => self.common_surfaces.select(packed, batch.opaque),
                });
                rp.set_bind_group(
                    1,
                    if batch.landmark {
                        &self.landmark_chunk_bg
                    } else {
                        &self.chunk_bg
                    },
                    &[],
                );
                let arena = if batch.packed {
                    &self.packed_arena
                } else {
                    &self.full_arena
                };
                let page = &arena.pages[batch.page];
                rp.set_vertex_buffer(0, page.vertex.slice(..));
                rp.set_index_buffer(
                    page.index.slice(..),
                    if batch.indices_u16 {
                        wgpu::IndexFormat::Uint16
                    } else {
                        wgpu::IndexFormat::Uint32
                    },
                );
                let commands = &indirect_commands
                    [batch.command_start..batch.command_start + batch.command_count as usize];
                if self.indirect_first_instance {
                    rp.multi_draw_indexed_indirect(
                        &self.indirect_buf,
                        batch.command_start as u64 * command_size,
                        batch.command_count,
                    );
                } else {
                    for command in commands {
                        rp.draw_indexed(
                            command.first_index..command.first_index + command.index_count,
                            command.base_vertex,
                            command.first_instance..command.first_instance + 1,
                        );
                    }
                }
                triangles += commands
                    .iter()
                    .map(|command| u64::from(command.index_count) / 3)
                    .sum::<u64>();
            }
            world_triangles = triangles;
            rp.set_bind_group(1, &self.tree_chunk_bg, &[]);
            rp.set_pipeline(&self.tree_pipeline);
            for (tree, indices, instances) in &tree_draws {
                rp.set_vertex_buffer(0, tree.vbuf.slice(..));
                rp.set_index_buffer(tree.ibuf.slice(..), tree.index_format);
                rp.draw_indexed(indices.clone(), 0, instances.clone());
                let count = indices.len() as u64 / 3 * instances.len() as u64;
                triangles += count;
                tree_triangles += count;
            }
            if !light_draws.is_empty() {
                rp.set_pipeline(&self.night_pipeline);
                rp.set_bind_group(0, &self.globals_bg, &[]);
                rp.set_bind_group(1, &self.light_chunk_bg, &[]);
                let light_command_size = std::mem::size_of::<wgpu::util::DrawIndirectArgs>() as u64;
                for batch in &light_arena_batches {
                    rp.set_vertex_buffer(0, self.light_arena.pages[batch.page].buffer.slice(..));
                    let commands = &light_indirect_commands
                        [batch.command_start..batch.command_start + batch.command_count as usize];
                    if self.indirect_first_instance {
                        rp.multi_draw_indirect(
                            &self.light_indirect_buf,
                            batch.command_start as u64 * light_command_size,
                            batch.command_count,
                        );
                    } else {
                        for command in commands {
                            rp.draw(
                                command.first_vertex..command.first_vertex + command.vertex_count,
                                command.first_instance..command.first_instance + 1,
                            );
                        }
                    }
                    triangles += commands
                        .iter()
                        .map(|command| u64::from(command.vertex_count) / 3)
                        .sum::<u64>();
                }
            }
            rp.set_pipeline(&self.aurora_pipeline);
            rp.set_bind_group(0, &self.globals_bg, &[]);
            rp.draw(0..(AURORA_SEGMENTS + 1) * 2, 0..AURORA_CURTAINS);
            triangles += u64::from(AURORA_SEGMENTS) * 2 * u64::from(AURORA_CURTAINS);
            // Flora remains after aurora in draw order and uses these same
            // HDR/depth attachments without sampling them. Store once before
            // atmosphere samples depth; no intermediate attachment reload.
            if !prepared_flora.draws.is_empty() {
                rp.set_pipeline(&self.flora_pipeline);
                rp.set_bind_group(0, &self.globals_bg, &[]);
                rp.set_bind_group(1, &self.flora_instance_bg, &[]);
                rp.set_bind_group(2, &self.sun_shadows.receiver_bg, &[]);
                rp.set_bind_group(3, &self.atlas_bg, &[]);
                rp.set_index_buffer(self.flora_index_buf.slice(..), wgpu::IndexFormat::Uint16);
                for draw in &prepared_flora.draws {
                    rp.draw_indexed(
                        flora_index_range(draw.topology, draw.lod),
                        0,
                        draw.instances.clone(),
                    );
                }
            }
        }
        triangles += prepared_flora.stats.triangles;
        if let Some(targets) = &self.hdr.as_ref().unwrap().multisample {
            self.world_aa.resolve_depth(&mut encoder, targets, depth);
        }
        // Atmosphere: separate pass so the depth buffer can be sampled.
        {
            let depth_bg = &self
                .atmo_depth_bg
                .as_ref()
                .expect("atmosphere depth bind group initialized above")
                .1;
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("atmosphere"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &hdr_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: timing_slot.map(|_| wgpu::RenderPassTimestampWrites {
                    query_set: &self
                        .gpu_timing
                        .as_ref()
                        .expect("a timing slot requires GPU timing resources")
                        .query_set,
                    beginning_of_pass_write_index: Some(ATMOSPHERE_BEGIN_QUERY),
                    end_of_pass_write_index: Some(ATMOSPHERE_END_QUERY),
                }),
                ..Default::default()
            });
            rp.set_pipeline(&self.atmo_pipeline);
            rp.set_bind_group(0, &self.globals_bg, &[]);
            rp.set_bind_group(1, depth_bg, &[]);
            if let Some(targets) = &self.hdr.as_ref().unwrap().multisample {
                rp.set_bind_group(2, &targets.group, &[]);
            }
            rp.draw(0..3, 0..1);
            if star_draw_count > 0 {
                rp.set_pipeline(&self.star_pipeline);
                rp.set_vertex_buffer(0, self.star_buf.slice(..));
                rp.draw(0..6, 0..star_draw_count);
                triangles += u64::from(star_draw_count) * 2;
            }
        }
        // One phase of a 4x4 pattern receives a deep cloud march each frame.
        // A cheap half-resolution pass reprojects the other fifteen phases
        // and resolves the new samples into ping-ponged history. The result
        // is then reconstructed at full resolution against opaque scene depth.
        let mut cloud_write_index = None;
        if cloud_active {
            let cloud_targets = self
                .cloud_targets
                .as_ref()
                .expect("active cloud targets initialized above");
            let cloud_history_index = cloud_targets.history_index;
            let write_index = 1 - cloud_history_index;
            cloud_write_index = Some(write_index);
            let cloud_history_bg = cloud_targets.buffers[cloud_history_index].sample_bg.clone();
            let cloud_write_bg = cloud_targets.buffers[write_index].sample_bg.clone();
            let cloud_write_color = cloud_targets.buffers[write_index].color_view.clone();
            let cloud_write_meta = cloud_targets.buffers[write_index].meta_view.clone();
            let cloud_update_bg = cloud_targets.update.sample_bg.clone();
            let cloud_update_color = cloud_targets.update.color_view.clone();
            let cloud_update_meta = cloud_targets.update.meta_view.clone();

            let depth_bg = &self
                .atmo_depth_bg
                .as_ref()
                .expect("atmosphere depth bind group initialized above")
                .1;
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("interleaved temporal cloud integration"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &cloud_update_color,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &cloud_update_meta,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: None,
                timestamp_writes: timing_slot.map(|_| wgpu::RenderPassTimestampWrites {
                    query_set: &self
                        .gpu_timing
                        .as_ref()
                        .expect("a timing slot requires GPU timing resources")
                        .query_set,
                    beginning_of_pass_write_index: Some(CLOUD_BEGIN_QUERY),
                    end_of_pass_write_index: None,
                }),
                ..Default::default()
            });
            rp.set_pipeline(&self.cloud_integrate_pipeline);
            rp.set_bind_group(0, &self.globals_bg, &[]);
            rp.set_bind_group(1, depth_bg, &[]);
            rp.set_bind_group(2, &cloud_history_bg, &[]);
            rp.draw(0..3, 0..1);
            drop(rp);

            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("half-resolution interleaved cloud history resolve"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &cloud_write_color,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &cloud_write_meta,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                ..Default::default()
            });
            rp.set_pipeline(&self.cloud_resolve_pipeline);
            rp.set_bind_group(0, &self.globals_bg, &[]);
            rp.set_bind_group(1, depth_bg, &[]);
            rp.set_bind_group(2, &cloud_history_bg, &[]);
            rp.set_bind_group(3, &cloud_update_bg, &[]);
            rp.draw(0..3, 0..1);
            drop(rp);

            let depth_bg = &self
                .atmo_depth_bg
                .as_ref()
                .expect("atmosphere depth bind group initialized above")
                .1;
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("full-resolution depth-aware cloud upscale"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &hdr_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: timing_slot.map(|_| wgpu::RenderPassTimestampWrites {
                    query_set: &self
                        .gpu_timing
                        .as_ref()
                        .expect("a timing slot requires GPU timing resources")
                        .query_set,
                    beginning_of_pass_write_index: None,
                    end_of_pass_write_index: Some(CLOUD_END_QUERY),
                }),
                ..Default::default()
            });
            rp.set_pipeline(&self.cloud_composite_pipeline);
            rp.set_bind_group(0, &self.globals_bg, &[]);
            rp.set_bind_group(1, depth_bg, &[]);
            rp.set_bind_group(2, &cloud_write_bg, &[]);
            rp.draw(0..3, 0..1);
        } else if timing_slot.is_some() {
            // Preserve a valid timing pair without touching cloud targets or
            // executing either cloud shader while clouds are clear/hidden.
            let _rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("inactive cloud timing"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &hdr_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: Some(wgpu::RenderPassTimestampWrites {
                    query_set: &self
                        .gpu_timing
                        .as_ref()
                        .expect("a timing slot requires GPU timing resources")
                        .query_set,
                    beginning_of_pass_write_index: Some(CLOUD_BEGIN_QUERY),
                    end_of_pass_write_index: Some(CLOUD_END_QUERY),
                }),
                ..Default::default()
            });
        }
        // The player is a dynamic foreground object, not part of the static
        // world depth used to reproject clouds. Drawing it before the cloud
        // passes stamps its moving silhouette into sparse cloud history and
        // leaves a trail when the camera or craft moves. Composite clouds
        // against terrain first, then draw the player normally into the same
        // HDR/depth targets so it still occludes sky, clouds and world.
        if !player_draws.is_empty() {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("player after temporal clouds"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &hdr_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                ..Default::default()
            });
            rp.set_pipeline(&self.player_pipeline);
            rp.set_bind_group(0, &self.globals_bg, &[]);
            rp.set_bind_group(1, &self.chunk_bg, &[]);
            rp.set_bind_group(2, &self.atlas_bg, &[]);
            rp.set_bind_group(3, &self.sun_shadows.receiver_bg, &[]);
            for (pi, offset) in &player_draws {
                let part = &self.player_parts[*pi];
                if part.index_count == 0 {
                    continue;
                }
                rp.set_vertex_buffer(0, part.vbuf.slice(..));
                rp.set_vertex_buffer(1, part.uvbuf.as_ref().expect("player UV stream").slice(..));
                rp.set_index_buffer(part.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                rp.draw_indexed(0..part.index_count, 0, *offset..*offset + 1);
                triangles += part.index_count as u64 / 3;
            }
        }
        self.vehicle_effects.prepare(
            &self.queue,
            player.filter(|_| !player_draws.is_empty()),
            cam_pos,
        );
        triangles += self.vehicle_effects.render(
            &mut encoder,
            &hdr_view,
            &self.globals_bg,
            &self
                .atmo_depth_bg
                .as_ref()
                .expect("scene depth initialized")
                .1,
        );
        // Near rain/snow is independently bounded and measured.
        {
            let depth_bg = &self
                .atmo_depth_bg
                .as_ref()
                .expect("atmosphere depth bind group initialized above")
                .1;
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("near precipitation"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &hdr_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: timing_slot.map(|_| wgpu::RenderPassTimestampWrites {
                    query_set: &self
                        .gpu_timing
                        .as_ref()
                        .expect("a timing slot requires GPU timing resources")
                        .query_set,
                    beginning_of_pass_write_index: Some(PRECIPITATION_BEGIN_QUERY),
                    end_of_pass_write_index: Some(PRECIPITATION_END_QUERY),
                }),
                ..Default::default()
            });
            rp.set_pipeline(&self.precipitation_pipeline);
            rp.set_bind_group(0, &self.globals_bg, &[]);
            rp.set_bind_group(1, depth_bg, &[]);
            if self.precipitation_visible && weather.surface[3] >= 0.0 && weather.weather0[3] > 0.03
            {
                rp.draw(0..4, 0..PRECIPITATION_INSTANCES);
            }
        }
        // Filmic HDR resolve, 3D creative grade and perceptual saturation run
        // together in the existing tone-map pass.
        {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("tone map"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: timing_slot.map(|_| wgpu::RenderPassTimestampWrites {
                    query_set: &self
                        .gpu_timing
                        .as_ref()
                        .expect("a timing slot requires GPU timing resources")
                        .query_set,
                    beginning_of_pass_write_index: Some(TONEMAP_BEGIN_QUERY),
                    end_of_pass_write_index: Some(TONEMAP_END_QUERY),
                }),
                ..Default::default()
            });
            rp.set_pipeline(&self.tone_pipeline);
            rp.set_bind_group(0, &self.globals_bg, &[]);
            rp.set_bind_group(1, &tone_bg, &[]);
            rp.set_bind_group(2, &self.post_bg, &[]);
            rp.draw(0..3, 0..1);
        }
        if let Some(slot_index) = timing_slot {
            let timing = self
                .gpu_timing
                .as_ref()
                .expect("a timing slot requires GPU timing resources");
            let slot = &timing.slots[slot_index];
            encoder.resolve_query_set(&timing.query_set, 0..GPU_QUERY_COUNT, &slot.resolve, 0);
            encoder.copy_buffer_to_buffer(&slot.resolve, 0, &slot.readback, 0, GPU_QUERY_BYTES);
            timing.schedule_readback(&encoder, slot_index);
        }
        self.buffer_uploads.finish_and_recall_on_submit(&encoder);
        let submit_started = Instant::now();
        self.queue.submit([encoder.finish()]);
        frame_queue_upload_ns =
            frame_queue_upload_ns.saturating_add(submit_started.elapsed().as_nanos() as u64);
        if cloud_active {
            self.cloud_targets
                .as_mut()
                .expect("cloud targets initialized above")
                .history_index = cloud_write_index.expect("active clouds wrote a target");
            self.cloud_history = Some(CloudHistoryState {
                camera_ecef: cam_pos,
                view_proj,
                weather_time: weather.weather2[0],
                lightning_flash: weather.weather2[3],
                weather,
                lighting,
            });
            self.cloud_frame_index = self.cloud_frame_index.wrapping_add(1);
        } else {
            self.cloud_history = None;
            self.cloud_frame_index = 0;
        }
        let uploads = std::mem::take(&mut self.pending_uploads);
        let upload_preparation_ms = uploads.preparation_ns as f64 * 1e-6;
        let queue_upload_ms = uploads
            .queue_upload_ns
            .saturating_add(frame_queue_upload_ns) as f64
            * 1e-6;
        let allocated_cloud_bytes = self.cloud_targets.as_ref().map_or(0, |targets| {
            let history_pixels =
                u64::from(targets.width) * u64::from(targets.height) * CLOUD_HISTORY_BUFFERS;
            let update_pixels = u64::from(targets.update_width) * u64::from(targets.update_height);
            (history_pixels + update_pixels)
                * (CLOUD_COLOR_BYTES_PER_PIXEL + CLOUD_META_BYTES_PER_PIXEL)
        });
        let submitted_world_indices = indirect_commands
            .iter()
            .map(|command| u64::from(command.index_count))
            .sum::<u64>();
        debug_assert!(submitted_world_indices.is_multiple_of(3));
        debug_assert!(
            world_material_indices.is_none_or(|counts| counts.total() == submitted_world_indices)
        );
        let submitted_world_vertices = visible
            .iter()
            .map(|candidate| {
                let chunk = candidate.chunk;
                let stride = if chunk.packed_vertices {
                    std::mem::size_of::<PackedSurfaceVertex>()
                } else {
                    std::mem::size_of::<SurfaceVertex>()
                };
                chunk.vertex_bytes / stride as u64
            })
            .sum();
        let sun_shadow_player_indices = shadow_player_slot.map_or(0, |_| {
            player_draws
                .iter()
                .map(|(part_index, _)| u64::from(self.player_parts[*part_index].index_count))
                .sum::<u64>()
        });
        let sun_shadow_caster_indices = shadow_indirect_commands
            .iter()
            .map(|command| u64::from(command.index_count))
            .sum::<u64>()
            + sun_shadow_player_indices;
        let mut sun_shadow_caster_indices_by_cascade = std::array::from_fn(|cascade| {
            shadow_draws[cascade]
                .iter()
                .map(|draw| u64::from(draw.index_count))
                .sum::<u64>()
        });
        sun_shadow_caster_indices_by_cascade[0] += sun_shadow_player_indices;
        self.sun_shadows.candidate_scratch = shadow_candidates;
        FrameStats {
            requested_cells: draw_cells.len(),
            resident_draw_cells,
            missing_draw_cells,
            candidate_chunks,
            visible_chunks,
            submitted_chunks,
            submitted_world_indices,
            world_material_indices,
            opaque_world_indices,
            feedback_world_indices: submitted_world_indices - opaque_world_indices,
            submitted_world_vertices,
            submitted_world_batches: arena_batches.len(),
            indirect_world_batches: self.indirect_first_instance,
            submitted_landmark_chunks,
            submitted_u16_index_chunks,
            world_draw_upload_bytes: uniforms.len() + landmark_uniforms.len(),
            dropped_chunks,
            dropped_player_parts,
            invalid_player_parts,
            submitted_player_parts: player_draws.len(),
            submitted_tree_instances: tree_instance_count,
            submitted_tree_batches: tree_draws.len(),
            tree_instancing_active,
            tree_instance_upload_bytes: tree_uniforms.len(),
            submitted_light_cells: light_draws.len(),
            submitted_light_batches: light_arena_batches.len(),
            dropped_light_cells,
            draw_capacity: MAX_DRAWS,
            draw_overflow_total: self.draw_overflow_total,
            atmo_bind_group_rebuilt,
            cloud_working_set_bytes: allocated_cloud_bytes,
            sun_shadow_configured: self.sun_shadows.configured,
            sun_shadow_active,
            sun_shadow_cascades: usize::from(sun_shadow_active) * SUN_SHADOW_CASCADES,
            sun_shadow_updated_cascades: sun_shadow_update_mask.count_ones() as usize,
            sun_shadow_caster_chunks: submitted_shadow_parent_chunks,
            sun_shadow_caster_chunks_by_cascade,
            sun_shadow_caster_indices,
            sun_shadow_caster_indices_by_cascade,
            sun_shadow_batches: shadow_arena_batches.len()
                + usize::from(shadow_player_slot.is_some()),
            sun_shadow_capacity_drops,
            sun_shadow_working_set_bytes: self.sun_shadows.working_set_bytes,
            cpu_sun_shadow_preparation_ms,
            gpu_timing_sample_id,
            gpu_timing_completed: self.gpu_timing.as_ref().and_then(|timing| timing.completed),
            gpu_timing_issued_id: timing_slot
                .map(|slot| self.gpu_timing.as_ref().unwrap().slots[slot].frame_id),
            gpu_total_ms: gpu_timings.map(|timing| timing.total_ms),
            gpu_shadow_ms: gpu_timings.map(|timing| timing.shadow_ms),
            gpu_world_ms: gpu_timings.map(|timing| timing.world_ms),
            gpu_flora_ms: gpu_timings.and_then(|timing| timing.flora_ms),
            gpu_atmosphere_ms: gpu_timings.map(|timing| timing.atmosphere_ms),
            gpu_cloud_ms: gpu_timings.map(|timing| timing.cloud_ms),
            gpu_precipitation_ms: gpu_timings.map(|timing| timing.precipitation_ms),
            gpu_tonemap_ms: gpu_timings.map(|timing| timing.tonemap_ms),
            uploads,
            mesh: self.mesh_stats,
            flora: prepared_flora.stats,
            cpu_visibility_ms,
            cpu_world_draw_construction_ms,
            cpu_sorting_ms,
            cpu_uniform_construction_ms,
            cpu_grouping_ms,
            cpu_indirect_construction_ms,
            upload_preparation_ms,
            queue_upload_ms,
            draws: submitted_chunks,
            culled,
            culled_horizon,
            world_triangles,
            tree_triangles,
            triangles,
        }
    }
}

/// Windowed renderer: surface + depth management around `Renderer`.
pub struct Gfx {
    pub renderer: Renderer,
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    view_format: wgpu::TextureFormat,
    depth_view: wgpu::TextureView,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceFrameError {
    Timeout,
    Occluded,
    Outdated,
    Lost,
    Validation,
}

impl Gfx {
    pub async fn new(window: &crate::host::Window) -> Result<Self, Box<dyn std::error::Error>> {
        let size = window.inner_size();
        #[cfg(not(target_arch = "wasm32"))]
        let (instance, surface) = {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
                    Box::new(window.winit_arc()),
                ));
            let surface = instance.create_surface(window.winit_arc())?;
            (instance, surface)
        };
        #[cfg(target_arch = "wasm32")]
        let (instance, surface) = {
            let instance = wgpu::Instance::new(
                wgpu::InstanceDescriptor::new_without_display_handle_from_env(),
            );
            let surface = instance.create_surface(wgpu::SurfaceTarget::OffscreenCanvas(
                window.canvas().clone(),
            ))?;
            (instance, surface)
        };
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await?;
        let requested_features = renderer_features(
            adapter.features(),
            adapter.get_downlevel_capabilities().flags,
        );
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("vearth"),
                required_features: requested_features,
                ..Default::default()
            })
            .await?;
        // The page otherwise freezes on its last frame. Browsers lose the
        // device mostly when the GPU runs out of memory or its driver resets.
        #[cfg(target_arch = "wasm32")]
        device.set_device_lost_callback(|reason, message| {
            if reason != wgpu::DeviceLostReason::Destroyed {
                crate::host::post_to_page(
                    "fatal",
                    format!(
                        "the graphics device was lost ({message}). This usually means the \
                         GPU ran out of memory. Reload, and consider Settings, World, \
                         Render resolution: Performance"
                    )
                    .into(),
                );
            }
        });
        if requested_features.contains(wgpu::Features::TIMESTAMP_QUERY) {
            log::info!("non-blocking GPU timestamp telemetry enabled");
        } else {
            log::info!("GPU timestamp telemetry unavailable on this adapter");
        }
        if requested_features.contains(wgpu::Features::INDIRECT_FIRST_INSTANCE) {
            log::info!("batched indirect terrain draws enabled");
        } else {
            log::info!("indirect first-instance unavailable; using arena-backed direct draws");
        }

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
        // Render both the world and egui through the sRGB view. egui supports
        // this with its linear-framebuffer shader; sharing the view preserves
        // Low Poly Earth's established translucent blending and shadow appearance.
        let view_format = format.add_srgb_suffix();
        let present_mode = if vearth_platform::env::var("VEARTH_PRESENT_MODE")
            .is_ok_and(|value| value.eq_ignore_ascii_case("fifo"))
        {
            wgpu::PresentMode::Fifo
        } else {
            wgpu::PresentMode::AutoVsync
        };
        log::info!("surface present mode: {present_mode:?}");
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: if view_format == format {
                vec![]
            } else {
                vec![view_format]
            },
        };
        surface.configure(&device, &config);
        let depth_view = create_depth(&device, config.width, config.height);
        let renderer = Renderer::new(device, queue, view_format);
        Ok(Gfx {
            renderer,
            surface,
            config,
            view_format,
            depth_view,
        })
    }

    pub fn view_format(&self) -> wgpu::TextureFormat {
        self.view_format
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        // Clamp to a sane surface size (browser resize loops, if any slip
        // through, must not allocate gigapixel textures).
        self.config.width = width.clamp(1, 8192);
        self.config.height = height.clamp(1, 8192);
        self.surface.configure(&self.renderer.device, &self.config);
        self.depth_view =
            create_depth(&self.renderer.device, self.config.width, self.config.height);
    }

    /// Acquire the swapchain frame. Render the world with `render_world`,
    /// optionally paint overlays onto `FrameTarget::view`, then `present`.
    pub fn begin_frame(&mut self) -> Result<FrameTarget, SurfaceFrameError> {
        let output = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(output)
            | wgpu::CurrentSurfaceTexture::Suboptimal(output) => output,
            wgpu::CurrentSurfaceTexture::Timeout => return Err(SurfaceFrameError::Timeout),
            wgpu::CurrentSurfaceTexture::Occluded => return Err(SurfaceFrameError::Occluded),
            wgpu::CurrentSurfaceTexture::Outdated => return Err(SurfaceFrameError::Outdated),
            wgpu::CurrentSurfaceTexture::Lost => return Err(SurfaceFrameError::Lost),
            wgpu::CurrentSurfaceTexture::Validation => return Err(SurfaceFrameError::Validation),
        };
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.view_format),
            ..Default::default()
        });
        Ok(FrameTarget { output, view })
    }

    /// Present a lightweight launch frame while provider discovery and cache
    /// setup continue off the event-loop thread.
    pub fn begin_launch_frame(&mut self, failed: bool) -> Result<FrameTarget, SurfaceFrameError> {
        let clear = if failed {
            wgpu::Color {
                r: 0.045,
                g: 0.006,
                b: 0.008,
                a: 1.0,
            }
        } else {
            wgpu::Color {
                r: 0.0024,
                g: 0.00335,
                b: 0.00335,
                a: 1.0,
            }
        };
        self.begin_cleared_frame("launch frame", clear)
    }

    /// Acquire and initialize a frame for the opaque map-only UI.
    pub fn begin_map_frame(&mut self) -> Result<FrameTarget, SurfaceFrameError> {
        self.begin_cleared_frame("map frame", wgpu::Color::BLACK)
    }

    fn begin_cleared_frame(
        &mut self,
        label: &'static str,
        clear: wgpu::Color,
    ) -> Result<FrameTarget, SurfaceFrameError> {
        let target = self.begin_frame()?;
        let mut encoder = self
            .renderer
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        self.renderer.queue.submit([encoder.finish()]);
        Ok(target)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_world(
        &mut self,
        target: &FrameTarget,
        cam_pos: DVec3,
        view_rot: DMat4,
        fov_y: f64,
        draw_cells: &[DrawCell],
        lighting: Lighting,
        weather: WeatherRenderState,
        player: Option<&PlayerDraw>,
    ) -> FrameStats {
        self.renderer.render(
            &target.view,
            &self.depth_view,
            self.config.width,
            self.config.height,
            cam_pos,
            view_rot,
            fov_y,
            draw_cells,
            lighting,
            weather,
            player,
        )
    }

    pub fn frame(
        &mut self,
        cam_pos: DVec3,
        view_rot: DMat4,
        fov_y: f64,
        draw_cells: &[DrawCell],
        lighting: Lighting,
    ) -> Result<FrameStats, SurfaceFrameError> {
        let target = self.begin_frame()?;
        let weather = crate::weather::WeatherModel::new(Default::default())
            .render_state(cam_pos, lighting.utc_seconds);
        let stats = self.render_world(
            &target, cam_pos, view_rot, fov_y, draw_cells, lighting, weather, None,
        );
        target.present(&self.renderer.queue);
        Ok(stats)
    }
}

pub struct FrameTarget {
    output: wgpu::SurfaceTexture,
    pub view: wgpu::TextureView,
}

impl FrameTarget {
    pub fn present(self, queue: &wgpu::Queue) {
        queue.present(self.output);
    }
}

pub fn create_depth(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

/// Left/right/bottom/top planes of `m` as (normal, d), inward-positive.
/// The infinite-reverse projection has no far plane, and its near row is
/// degenerate ((0,0,0,z_near) — normalizing it would produce NaNs); the four
/// side planes intersect at the camera, so they also exclude everything
/// behind it.
fn frustum_planes(m: &DMat4) -> [(DVec3, f64); 4] {
    let r = |i: usize| DVec3::new(m.x_axis[i], m.y_axis[i], m.z_axis[i]);
    let w = DVec3::new(m.x_axis.w, m.y_axis.w, m.z_axis.w);
    let dw = m.w_axis.w;
    let d = |i: usize| m.w_axis[i];
    let mut planes = [
        (w + r(0), dw + d(0)), // left
        (w - r(0), dw - d(0)), // right
        (w + r(1), dw + d(1)), // bottom
        (w - r(1), dw - d(1)), // top
    ];
    for (n, pd) in &mut planes {
        let len = n.length();
        *n /= len;
        *pd /= len;
    }
    planes
}

fn sphere_visible(planes: &[(DVec3, f64); 4], center: DVec3, radius: f64) -> bool {
    planes
        .iter()
        .all(|(n, d)| n.dot(center) + d + radius >= 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Explicit destruction exercises the renderer resource boundary because
    /// backend devices have no portable fault-injection API. This is ignored in
    /// ordinary CI because it requires a native graphics adapter.
    #[test]
    #[ignore = "requires a native GPU adapter"]
    fn gpu_device_destroy_and_renderer_recreate() {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .expect("native adapter");
        let features = renderer_features(
            adapter.features(),
            adapter.get_downlevel_capabilities().flags,
        );
        let request_device = || {
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("vearth device recreation test"),
                required_features: features,
                ..Default::default()
            }))
            .expect("device")
        };

        let (first_device, first_queue) = request_device();
        let first = Renderer::new(
            first_device,
            first_queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        first.device.destroy();
        drop(first);

        let (replacement_device, replacement_queue) = request_device();
        let replacement = Renderer::new(
            replacement_device,
            replacement_queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        replacement
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    }
    fn empty_cell_gpu() -> CellGpu {
        CellGpu {
            upload_generation: 0,
            horizon: vearth_core::lod::CellHorizon::new(CellId::root(
                vearth_core::cubesphere::Face(0),
            )),
            chunks: Vec::new(),
            lights: None,
            trees: Vec::new(),
            flora: Vec::new(),
            flora_frame: None,
        }
    }

    #[test]
    fn gpu_resident_slots_reject_stale_generations() {
        let mut residents = ResidentGpuStore::default();
        let first = ResidentId::for_test(3, 7);
        assert!(residents.insert(first, empty_cell_gpu()).is_none());
        assert!(residents.contains(first));
        assert_eq!(residents.len(), 1);
        assert!(residents.remove(ResidentId::for_test(3, 6)).is_none());
        assert!(residents.contains(first));
        assert!(residents.remove(first).is_some());
        assert!(!residents.contains(first));

        let replacement = ResidentId::for_test(3, 8);
        assert!(residents.insert(replacement, empty_cell_gpu()).is_none());
        assert!(residents.get(first).is_none());
        assert!(residents.get(replacement).is_some());
    }

    #[test]
    fn specialized_draw_records_have_exact_storage_strides() {
        assert_eq!(std::mem::size_of::<TreeU>(), TREE_U_SIZE as usize);
        assert_eq!(std::mem::size_of::<TerrainU>(), TERRAIN_U_SIZE as usize);
        assert_eq!(std::mem::size_of::<LandmarkU>(), LANDMARK_U_SIZE as usize);
        assert_eq!(std::mem::size_of::<LightU>(), LIGHT_U_SIZE as usize);
        assert_eq!(
            std::mem::size_of::<FloraInstance>(),
            FLORA_INSTANCE_SIZE as usize
        );
        assert_eq!(
            MAX_FLORA_INSTANCES,
            MAX_FLORA_UPLOAD_BYTES / std::mem::size_of::<FloraInstance>()
        );
        assert!(
            MAX_FLORA_UPLOAD_BYTES - MAX_FLORA_INSTANCES * std::mem::size_of::<FloraInstance>()
                < std::mem::size_of::<FloraInstance>()
        );
    }

    fn flora_test_descriptor() -> FloraCell {
        FloraCell {
            archetype: vearth_gen::flora::FloraArchetype::Cereal,
            calendar: vearth_gen::flora::CultivationCalendar::TemperateSummer,
            field_seed: 0x1234_5678_9abc_def0,
            row_heading_rad: 0.75,
            palette: 3,
            base_density: 250,
            coverage: [u32::MAX; 8],
            ground_base_cm: 120,
            ground_cm: [0; 25],
            phenology_offset: 17,
            curated_style: 0,
        }
    }

    fn flora_test_unit(
        descriptor: &FloraCell,
        index: usize,
        lod: FloraLod,
        distance_m: f64,
    ) -> VisibleFloraUnit<'_> {
        VisibleFloraUnit {
            cell: CellId {
                face: vearth_core::cubesphere::Face(0),
                level: 19,
                i: index as u32,
                j: 7,
            },
            descriptor_index: 0,
            descriptor,
            frame: FloraFrameGpu {
                model: DMat4::IDENTITY,
                bound_center: DVec3::ZERO,
                bound_radius: 20.0,
                local_y_min: 0.0,
                local_y_max: 4.0,
            },
            terrain_fade: 1.0,
            distance_m,
            assignments: [
                FloraLodAssignment { lod, fade: 1.0 },
                FloraLodAssignment {
                    lod: FloraLod::Coarse,
                    fade: 0.0,
                },
            ],
            assignment_count: 1,
        }
    }

    #[test]
    fn flora_lod_bands_are_complementary_and_end_at_surface_handoff() {
        let keeps = |alpha: f32, noise: f32| {
            alpha >= 1.0
                || if alpha >= 0.0 {
                    noise < alpha
                } else {
                    noise >= 1.0 + alpha
                }
        };
        for distance in [50.1, 60.0, 69.9, 180.1, 200.0, 219.9, 550.1, 600.0, 649.9] {
            let (assignments, count) = flora_lod_assignments(distance);
            assert_eq!(count, 2);
            for noise in [0.0, 0.13, 0.49, 0.5, 0.87, 0.999] {
                assert_ne!(
                    keeps(assignments[0].fade, noise),
                    keeps(assignments[1].fade, noise),
                    "distance {distance}, noise {noise}"
                );
            }
        }
        assert_eq!(flora_lod_assignments(4_999.99).1, 1);
        assert_eq!(flora_lod_assignments(5_999.99).1, 1);
        assert_eq!(flora_lod_assignments(6_000.0).1, 0);
    }

    #[test]
    fn flora_distance_uses_the_ground_prism_not_the_cell_sphere() {
        let frame = FloraFrameGpu {
            model: DMat4::IDENTITY,
            bound_center: DVec3::new(16.0, 2.0, 16.0),
            // Deliberately larger than the camera distance: sphere-distance
            // gating would incorrectly classify this orbital view as close.
            bound_radius: 2_000.0,
            local_y_min: 0.0,
            local_y_max: 4.0,
        };
        assert!(
            (flora_frame_distance(frame, DVec3::new(16.0, 1_004.0, 16.0)) - 1_000.0).abs() < 1.0e-9
        );
        assert_eq!(
            flora_frame_distance(frame, DVec3::new(20.0, 2.0, 20.0)),
            0.0
        );
    }

    #[test]
    fn canonical_parent_candidates_are_a_subset_of_child_candidates() {
        let parent = CellId {
            face: vearth_core::cubesphere::Face(0),
            level: 18,
            i: 42_000,
            j: 81_000,
        };
        let child_points: HashSet<_> = parent
            .children()
            .into_iter()
            .flat_map(|child| {
                (0..256).map(move |candidate| canonical_flora_lattice(child, candidate))
            })
            .collect();
        for candidate in 0..512 {
            assert!(
                child_points.contains(&canonical_flora_lattice(parent, candidate)),
                "parent candidate {candidate} reorganized at terrain refinement"
            );
        }
    }

    #[test]
    fn multi_kilometre_field_is_bounded_without_plant_allocations() {
        let descriptor = flora_test_descriptor();
        let units = (0..1_000)
            .map(|index| flora_test_unit(&descriptor, index, FloraLod::Close, index as f64))
            .collect();
        let prepared = prepare_flora(units, DVec3::ZERO, 1_000);
        assert!(prepared.stats.glyphs > 0);
        assert!(!prepared.instances.is_empty());
        assert_eq!(
            prepared.stats.visible_by_calendar
                [vearth_gen::flora::CultivationCalendar::TemperateSummer as usize],
            1_000
        );
        assert_eq!(
            prepared.stats.visible_by_registry,
            [0; vearth_gen::flower_destinations::FLOWER_DESTINATION_COUNT]
        );
        assert_eq!(prepared.stats.cells_suppressed_by_glyph_budget, 0);
        assert!(prepared.stats.cells_suppressed_by_triangle_budget > 0);
        assert_eq!(prepared.stats.cells_suppressed_by_upload_budget, 0);
        assert_eq!(
            prepared.stats.suppression_reason,
            FloraSuppressionReason::TriangleBudget
        );
        assert_eq!(prepared.stats.glyph_budget, MAX_FLORA_GLYPHS);
        assert!(prepared.stats.triangles <= MAX_FLORA_TRIANGLES);
        assert!(prepared.stats.draw_calls <= MAX_FLORA_DRAWS);
        assert!(prepared.stats.instance_upload_bytes <= MAX_FLORA_UPLOAD_BYTES);
        // The only CPU/GPU records are cell descriptors/instances.
        assert_eq!(
            prepared.stats.instance_upload_bytes,
            prepared.instances.len() * std::mem::size_of::<FloraInstance>()
        );
    }

    #[test]
    fn managed_perennial_stress_is_bounded_by_cells_not_tree_vine_or_post_counts() {
        use vearth_gen::flora::FloraArchetype;

        for archetype in [FloraArchetype::Orchard, FloraArchetype::Vineyard] {
            let mut descriptor = flora_test_descriptor();
            descriptor.archetype = archetype;
            let units = (0..2_000)
                .map(|index| flora_test_unit(&descriptor, index, FloraLod::Close, index as f64))
                .collect();
            let prepared = prepare_flora(units, DVec3::ZERO, 2_000);
            assert_eq!(prepared.draws.len(), 1);
            assert!(
                prepared.stats.cells_suppressed_by_glyph_budget
                    + prepared.stats.cells_suppressed_by_triangle_budget
                    + prepared.stats.cells_suppressed_by_upload_budget
                    > 0
            );
            assert!(prepared.stats.glyphs <= MAX_FLORA_GLYPHS);
            assert!(prepared.stats.triangles <= MAX_FLORA_TRIANGLES);
            assert!(prepared.stats.instance_upload_bytes <= MAX_FLORA_UPLOAD_BYTES);
            assert_eq!(
                prepared.stats.instance_upload_bytes,
                prepared.instances.len() * std::mem::size_of::<FloraInstance>()
            );
        }
    }

    #[test]
    fn flora_upload_gate_is_independent_of_the_glyph_gate() {
        let descriptor = flora_test_descriptor();
        let units = (0..1_000)
            .map(|index| flora_test_unit(&descriptor, index, FloraLod::Coarse, index as f64))
            .collect();
        let prepared = prepare_flora(units, DVec3::ZERO, 1_000);
        assert_eq!(prepared.instances.len(), MAX_FLORA_INSTANCES);
        assert_eq!(
            prepared.stats.instance_upload_bytes,
            MAX_FLORA_INSTANCES * FLORA_INSTANCE_SIZE as usize
        );
        assert_eq!(
            prepared.stats.glyphs,
            MAX_FLORA_INSTANCES as u32
                * flora_candidate_target(FloraTopology::CropFlower, FloraLod::Coarse)
        );
        assert_eq!(
            prepared.stats.cells_suppressed_by_upload_budget,
            1_000 - MAX_FLORA_INSTANCES
        );
        assert_eq!(prepared.stats.cells_suppressed_by_glyph_budget, 0);
        assert_eq!(
            prepared.stats.suppression_reason,
            FloraSuppressionReason::InstanceUploadBudget
        );
        assert_eq!(
            prepared.stats.instance_upload_budget_bytes,
            MAX_FLORA_UPLOAD_BYTES
        );
    }

    #[test]
    fn aggregate_budget_sampling_spans_the_field_and_preserves_near_cells() {
        let descriptor = flora_test_descriptor();
        let mut units = (0..1_000)
            .map(|index| {
                flora_test_unit(&descriptor, index, FloraLod::Distant, 700.0 + index as f64)
            })
            .collect::<Vec<_>>();
        units.extend((1_000..1_010).map(|index| {
            flora_test_unit(&descriptor, index, FloraLod::Close, index as f64 - 995.0)
        }));
        let prepared = prepare_flora(units, DVec3::ZERO, 1_010);
        let selected = prepared
            .instances
            .iter()
            .map(|instance| instance.identity[2])
            .collect::<HashSet<_>>();
        assert!((1_000..1_010).all(|index| selected.contains(&index)));
        assert!(
            selected.iter().any(|index| *index < 100)
                && selected.iter().any(|index| (450..550).contains(index))
                && selected.iter().any(|index| *index > 900),
            "aggregate sampling must cover the front, middle and back of the field"
        );
    }

    #[test]
    fn flower_surfaces_are_neutral_and_close_structure_survives_without_heads() {
        let source = include_str!("shader.wgsl");
        assert!(source.contains("field_runtime_presence = 0.0"));
        assert!(!source.contains("0.025 + bloom * 0.975"));

        let flora = include_str!("flora.wgsl");
        assert!(flora.contains("let flower_head_present = hash_unit(candidate_hash("));
        assert!(flora.contains("if (flower_head_present) {"));
        assert!(flora.contains("A non-blooming"));
    }

    #[test]
    fn orchard_aggregates_are_bounded_full_height_rows_not_flat_scaled_patches() {
        let source = include_str!("flora.wgsl");
        let aggregate = source
            .split("} else if (orchard) {")
            .nth(2)
            .expect("orchard aggregate branch")
            .split("} else if (vineyard) {")
            .next()
            .expect("orchard branch end")
            .split("if (aggregate) {")
            .nth(1)
            .expect("orchard aggregate body")
            .split("} else if (primitive > 0u) {")
            .next()
            .expect("orchard aggregate body end");
        assert!(aggregate.contains("form.trunk_height_m + form.crown_height_m"));
        assert!(aggregate.contains("aggregate_height_m * 0.5"));
        assert!(aggregate.contains("min(form.crown_along_m * distance_scale, 10.0)"));
        assert!(aggregate.contains("min(form.crown_across_m * distance_scale, 5.5)"));
        assert!(!aggregate.contains("terrain_lod_scale"));
        assert!(!aggregate.contains("season.density"));
    }

    #[test]
    fn middle_crop_flower_clusters_are_single_solids_while_managed_forms_stay_compound() {
        assert_eq!(
            flora_triangles_per_glyph(FloraTopology::CropFlower, FloraLod::Middle),
            12
        );
        for topology in [FloraTopology::Orchard, FloraTopology::Vineyard] {
            assert_eq!(flora_triangles_per_glyph(topology, FloraLod::Middle), 24);
        }
        let source = include_str!("flora.wgsl");
        assert!(source.contains("let compound_middle = archetype >= 7u && lod == 1u"));
        assert!(source.contains("((crop || flower) && lod >= 1u)"));
    }

    #[test]
    fn flora_cuboid_indices_reuse_face_vertices_without_changing_triangle_order() {
        let indices = flora_index_data();
        let expected_len = MAX_FLORA_CANDIDATES_PER_INSTANCE as usize
            * (FLORA_INDICES_PER_BOX + FLORA_INDICES_PER_BOX * 2) as usize;
        assert_eq!(indices.len(), expected_len);

        let one_box = &indices[..FLORA_INDICES_PER_BOX as usize];
        for face in 0..6u16 {
            let base = face * 4;
            assert_eq!(
                &one_box[face as usize * 6..face as usize * 6 + 6],
                &[base, base + 1, base + 2, base, base + 2, base + 3],
            );
        }
        let legacy_corners = [0u16, 1, 2, 0, 2, 3];
        for (legacy_vertex, &indexed_vertex) in one_box.iter().enumerate() {
            let legacy_face = legacy_vertex as u16 / 6;
            let legacy_corner = legacy_corners[legacy_vertex % 6];
            assert_eq!(indexed_vertex / 4, legacy_face);
            assert_eq!(indexed_vertex % 4, legacy_corner);
        }

        let compound_start = FLORA_COMPOUND_INDEX_OFFSET as usize;
        let first_compound =
            &indices[compound_start..compound_start + (FLORA_INDICES_PER_BOX * 2) as usize];
        assert_eq!(&first_compound[..FLORA_INDICES_PER_BOX as usize], one_box,);
        assert_eq!(
            first_compound
                .iter()
                .copied()
                .max()
                .expect("compound glyph has indices"),
            47,
        );

        for topology in [
            FloraTopology::CropFlower,
            FloraTopology::Orchard,
            FloraTopology::Vineyard,
        ] {
            for lod in [
                FloraLod::Close,
                FloraLod::Middle,
                FloraLod::Coarse,
                FloraLod::Distant,
            ] {
                let candidates = flora_candidate_target(topology, lod);
                assert!(candidates <= MAX_FLORA_CANDIDATES_PER_INSTANCE);
                let range = flora_index_range(topology, lod);
                assert_eq!(
                    range.end - range.start,
                    candidates * flora_indices_per_glyph(topology, lod),
                );
                let referenced = &indices[range.start as usize..range.end as usize];
                assert_eq!(
                    u32::from(*referenced.iter().max().expect("draw has indices")) + 1,
                    candidates * flora_boxes_per_glyph(topology, lod) * FLORA_VERTICES_PER_BOX,
                );
            }
        }
    }

    #[test]
    fn distant_crop_flower_aggregates_preserve_coverage_with_fewer_solids() {
        assert_eq!(
            flora_active_candidates(
                vearth_gen::flora::FloraArchetype::FlowerCrop,
                FloraLod::Distant
            ),
            12
        );
        let source = include_str!("flora.wgsl");
        assert!(source.contains("nominal_cell_m * 0.067"));
    }

    #[test]
    fn distant_crop_canopy_matches_ground_planes_and_shared_draw_capacity() {
        use vearth_gen::flora::FloraArchetype;
        let mut crop = flora_test_descriptor();
        crop.archetype = FloraArchetype::Cereal;
        crop.ground_base_cm = 0;
        for (index, sample) in crop.ground_cm.iter_mut().enumerate() {
            let (x, z) = (index % 5, index / 5);
            *sample = (((x * 13 + z * 7) % 5) * 100) as i16;
        }
        let interpolated_canopy = |columns: u32, rows: u32, x: f32, z: f32| {
            let gx = x * columns as f32;
            let gz = z * rows as f32;
            let ix = (gx.floor() as u32).min(columns - 1);
            let iz = (gz.floor() as u32).min(rows - 1);
            let (fx, fz) = (gx - ix as f32, gz - iz as f32);
            let h = |dx, dz| {
                crop.ground_height_m(
                    (ix + dx) as f32 / columns as f32,
                    (iz + dz) as f32 / rows as f32,
                )
            };
            let (a, b, c, d) = (h(0, 0), h(1, 0), h(0, 1), h(1, 1));
            if fz <= fx {
                a + (b - a) * fx + (d - b) * fz
            } else {
                a + (d - c) * fx + (c - a) * fz
            }
        };
        let mut old_error = 0.0_f32;
        for z in 0..=16 {
            for x in 0..=16 {
                let (x, z) = (x as f32 / 16.0, z as f32 / 16.0);
                let ground = crop.ground_height_m(x, z);
                assert!(
                    (interpolated_canopy(CROP_DISTANT_GRID_EDGE, CROP_DISTANT_GRID_EDGE, x, z)
                        - ground)
                        .abs()
                        < 1.0e-5
                );
                old_error = old_error.max((interpolated_canopy(3, 4, x, z) - ground).abs());
            }
        }
        assert!(
            old_error > 0.5,
            "fixture must expose the misaligned overview grid"
        );
        let mut flower = crop.clone();
        flower.archetype = FloraArchetype::FlowerCrop;
        let prepared = prepare_flora(
            vec![
                flora_test_unit(&crop, 1, FloraLod::Distant, 1000.0),
                flora_test_unit(&flower, 2, FloraLod::Distant, 1000.0),
            ],
            DVec3::ZERO,
            2,
        );
        assert_eq!(prepared.draws.len(), 1);
        assert!(
            prepared
                .instances
                .iter()
                .any(|instance| instance.state[2] == 16)
        );
        assert!(
            prepared
                .instances
                .iter()
                .any(|instance| instance.state[2] == 12)
        );
        assert_eq!(prepared.stats.glyphs, 32);
        assert!(prepared.stats.glyphs <= MAX_FLORA_GLYPHS);
        assert!(prepared.stats.triangles <= MAX_FLORA_TRIANGLES);
        assert_eq!(
            flora_index_range(FloraTopology::CropFlower, FloraLod::Distant).len(),
            16 * FLORA_INDICES_PER_BOX as usize
        );
    }

    #[test]
    fn worst_case_topology_lod_mix_stays_below_the_draw_gate() {
        use vearth_gen::flora::FloraArchetype;
        let keys: HashSet<_> = [
            FloraArchetype::Cereal,
            FloraArchetype::BroadleafRows,
            FloraArchetype::TallRows,
            FloraArchetype::LowDenseCrop,
            FloraArchetype::FlowerCrop,
            FloraArchetype::OrnamentalBed,
            FloraArchetype::MeadowWildflowers,
            FloraArchetype::Orchard,
            FloraArchetype::Vineyard,
        ]
        .into_iter()
        .flat_map(|archetype| {
            [
                FloraLod::Close,
                FloraLod::Middle,
                FloraLod::Coarse,
                FloraLod::Distant,
            ]
            .map(move |lod| (flora_topology(archetype), lod))
        })
        .collect();
        assert_eq!(keys.len(), 12);
        assert!(keys.len() <= MAX_FLORA_DRAWS);
    }

    #[test]
    fn generic_crop_clusters_represent_more_plants_than_flower_glyphs() {
        let crop_descriptor = flora_test_descriptor();
        let crop_units = vec![flora_test_unit(&crop_descriptor, 0, FloraLod::Close, 20.0)];
        let crop = prepare_flora(crop_units, DVec3::ZERO, 1);

        let mut flower_descriptor = crop_descriptor.clone();
        flower_descriptor.archetype = vearth_gen::flora::FloraArchetype::FlowerCrop;
        flower_descriptor.base_density = 232;
        let flower_units = vec![flora_test_unit(
            &flower_descriptor,
            0,
            FloraLod::Close,
            20.0,
        )];
        let flower = prepare_flora(flower_units, DVec3::ZERO, 1);

        assert!(crop.stats.estimated_crop_clusters > 0.0);
        assert_eq!(flower.stats.estimated_crop_clusters, 0.0);
        assert!(
            crop.stats.estimated_crop_plant_equivalents > flower.stats.estimated_flower_heads * 2.0,
            "crop plant equivalents {:.0} should substantially exceed flower heads {:.0}",
            crop.stats.estimated_crop_plant_equivalents,
            flower.stats.estimated_flower_heads,
        );
    }

    #[test]
    fn generic_flower_shader_palette_matches_the_generator_surface_palette() {
        let source = include_str!("flora.wgsl");
        for palette in 0..4 {
            for color in vearth_gen::flora::generic_flower_palette_linear(palette) {
                let literal = format!(
                    "vec3<f32>({:.6}, {:.6}, {:.6})",
                    color[0], color[1], color[2]
                );
                assert!(
                    source.contains(&literal),
                    "shader is missing generator palette color {literal}"
                );
            }
        }
    }

    #[test]
    fn surface_shaders_share_shadowed_key_and_hemisphere_fill() {
        let terrain = include_str!("shader.wgsl");
        let flora = include_str!("flora.wgsl");
        for (shader, geometric_ndl) in [(terrain, "shadow_ndl"), (flora, "ndl")] {
            // Shadow bias follows geometry, not texture-derived bump normals.
            assert!(shader.contains(&format!(
                "let key_visibility = sun_visibility(in.world_pos, {geometric_ndl});"
            )));
            assert!(shader.contains("mix(0.08, 1.0, key_visibility)"));
            assert!(shader.contains("illustrative_fill(ambient, dot(n, up), fill_daylight)"));
            assert!(shader.contains("return mix(1.0, visibility, sun_shadow.tuning.y);"));
            assert!(shader.contains("sun_shadow.tuning.y <= 0.0 || ndl <= 0.0"));
            assert!(shader.contains("let moon_visibility = moon_horizon * moon_phase;"));
            assert!(!shader.contains("sunlight_overlay"));
            // Shadows attenuate only the Sun: night fill and emissive
            // material responses remain independent of the shadow map.
            assert!(!shader.contains("albedo *= sun_visibility"));
        }
        assert!(terrain.contains("let base_lighting = (ambient + base_sun + base_moon) * ao;"));
        assert!(flora.contains("ambient + base_direct + moon"));
        let function_body = |source: &'static str, name: &str| {
            let start = source.find(&format!("fn {name}(")).unwrap();
            let body = &source[start..];
            let mut depth = 0_i32;
            let mut entered = false;
            let end = body
                .char_indices()
                .find_map(|(index, c)| {
                    if c == '{' {
                        depth += 1;
                        entered = true;
                    }
                    if c == '}' {
                        depth -= 1;
                    }
                    (entered && depth == 0).then_some(index + 1)
                })
                .unwrap();
            &body[..end]
        };
        for name in ["illustrative_diffuse", "illustrative_fill"] {
            assert_eq!(function_body(terrain, name), function_body(flora, name));
        }
    }

    #[test]
    fn flora_shader_is_vertex_index_procedural_static_and_opaque() {
        let source = include_str!("flora.wgsl");
        assert!(source.contains("@builtin(vertex_index) vertex_index"));
        assert!(source.contains("@builtin(instance_index) instance_index"));
        assert!(source.contains("fn coverage_at"));
        assert!(source.contains("fn ground_height"));
        assert!(source.contains("fn crop_form"));
        assert!(source.contains("fn flower_form"));
        assert!(source.contains("fn orchard_form"));
        assert!(source.contains("fn vineyard_form"));
        assert!(source.contains("fn crop_head_scale"));
        assert!(source.contains("fn box_vertex"));
        assert!(source.contains("six independently shaded faces"));
        assert!(source.contains("vertices_per_glyph = select(24u, 48u"));
        assert!(source.contains("static index buffer emits each face as 0,1,2,0,2,3"));
        assert!(!source.contains("vertices_per_glyph = select(36u, 72u"));
        assert!(source.contains("fn flower_head_color"));
        assert!(source.contains("fn curated_flower_color"));
        assert!(source.contains("fn generic_flower_color"));
        assert!(source.contains("fn crop_field_color"));
        assert!(source.contains("fn crop_canopy_vertex"));
        assert!(source.contains("case 30u:"));
        assert!(source.contains("fn flora_season_state"));
        assert!(source.contains("existence_threshold *= season.density"));
        assert!(source.contains("height_m *= season.height"));
        assert!(source.contains("g.weather4.x"));
        assert!(source.contains("g.weather1.y"));
        assert!(source.contains("g.weather6.w"));
        assert!(source.contains("g.viewport.y"));
        assert!(source.contains("row_center_m"));
        assert!(source.contains("row_origin_phase"));
        assert!(source.contains("perturb_origin_phase"));
        assert!(source.contains("along_center_m"));
        assert!(source.contains("managed_inside"));
        assert!(source.contains("form.post_period > 0u"));
        assert!(source.contains("fn dither_keeps"));
        assert!(!source.contains("g.wind"));
        assert!(!source.contains("texture_2d<f32>"));
        assert!(source.contains("var leaf_atlas: texture_2d_array<f32>"));
        let material_start = source.find("fn leaf_surface(").unwrap();
        let material_end = source[material_start..].find("@fragment").unwrap() + material_start;
        let material = &source[material_start..material_end];
        assert_eq!(material.matches("textureSampleGrad(").count(), 3);
        assert!(!material.contains("discard") && !material.contains("painted.a"));
        assert!(source.contains("apply_aerial_perspective(lit, in.world_pos, 0.0), 1.0"));
        assert!(source.contains("texture_depth_2d_array"));
        assert!(source.contains("textureSampleCompare"));
    }

    #[test]
    fn flora_instance_uses_alignment_tail_for_field_wide_crop_rows() {
        let mut descriptor = flora_test_descriptor();
        for (index, value) in descriptor.ground_cm.iter_mut().enumerate() {
            *value = (index as i32 * 317 - 3_700) as i16;
        }
        descriptor.ground_cm[0] = i16::MIN;
        descriptor.ground_cm[24] = i16::MAX;
        let unit = flora_test_unit(&descriptor, 0, FloraLod::Close, 20.0);
        let instance = make_flora_instance(
            &unit,
            FloraLodAssignment {
                lod: FloraLod::Close,
                fade: 1.0,
            },
            DVec3::ZERO,
        );
        let phases =
            crop_row_cell_phases(unit.cell, descriptor.row_heading_rad, descriptor.field_seed);
        assert_eq!(
            instance.crop_rows,
            [
                crop_row_spacing_m(descriptor.field_seed).to_bits(),
                phases[0].to_bits(),
                phases[1].to_bits(),
            ]
        );
        assert_eq!(
            std::mem::size_of::<FloraInstance>(),
            FLORA_INSTANCE_SIZE as usize
        );
        assert_eq!(
            instance.state[0] >> 24,
            u32::from(descriptor.calendar as u8)
        );
        assert_eq!(
            (instance.state[3] >> 16) & 0xff,
            u32::from(descriptor.palette)
        );
        assert_eq!(instance.ground_base_cm, descriptor.ground_base_cm);
        for (index, expected) in descriptor.ground_cm.into_iter().enumerate() {
            let bit = index * FLORA_GROUND_OFFSET_BITS;
            let word = bit / u32::BITS as usize;
            let shift = bit % u32::BITS as usize;
            let mut packed = instance.ground_offsets_cm[word] >> shift;
            if shift + FLORA_GROUND_OFFSET_BITS > u32::BITS as usize {
                packed |= instance.ground_offsets_cm[word + 1] << (u32::BITS as usize - shift);
            }
            let raw = packed & FLORA_GROUND_OFFSET_MASK;
            let decoded = ((raw << (32 - FLORA_GROUND_OFFSET_BITS)) as i32)
                >> (32 - FLORA_GROUND_OFFSET_BITS);
            assert_eq!(decoded, i32::from(expected).clamp(-4_096, 4_095));
        }
    }

    #[test]
    fn flower_crop_instance_carries_rows_and_regional_color_band_alignment() {
        let mut descriptor = flora_test_descriptor();
        descriptor.archetype = vearth_gen::flora::FloraArchetype::FlowerCrop;
        let unit = flora_test_unit(&descriptor, 0, FloraLod::Middle, 70.0);
        let instance = make_flora_instance(
            &unit,
            FloraLodAssignment {
                lod: FloraLod::Middle,
                fade: 1.0,
            },
            DVec3::ZERO,
        );
        assert_eq!(
            instance.crop_rows,
            vearth_gen::flora::flora_row_gpu_data(
                unit.cell,
                descriptor.archetype,
                descriptor.row_heading_rad,
                descriptor.field_seed,
            )
        );
        assert_eq!(
            f32::from_bits(instance.crop_rows[0]),
            vearth_gen::flora::flower_row_spacing_m(descriptor.field_seed)
        );
        let packed_band = instance.crop_rows[2];
        let encoded_spacing_m = (packed_band & 0xffff) as f32 * 0.01;
        assert!(
            (encoded_spacing_m - vearth_gen::flora::flower_band_spacing_m(descriptor.field_seed))
                .abs()
                <= 0.005
        );
    }

    #[test]
    fn managed_instances_carry_coherent_row_and_along_slot_alignment() {
        use vearth_gen::flora::{
            FloraArchetype, managed_along_spacing_m, managed_row_cell_phases, managed_row_spacing_m,
        };

        for archetype in [FloraArchetype::Orchard, FloraArchetype::Vineyard] {
            let mut descriptor = flora_test_descriptor();
            descriptor.archetype = archetype;
            let unit = flora_test_unit(&descriptor, 0, FloraLod::Close, 20.0);
            let instance = make_flora_instance(
                &unit,
                FloraLodAssignment {
                    lod: FloraLod::Close,
                    fade: 1.0,
                },
                DVec3::ZERO,
            );
            let phases = managed_row_cell_phases(
                unit.cell,
                archetype,
                descriptor.row_heading_rad,
                descriptor.field_seed,
            )
            .unwrap();
            assert_eq!(
                f32::from_bits(instance.crop_rows[0]),
                managed_row_spacing_m(archetype, descriptor.field_seed).unwrap()
            );
            assert_eq!(f32::from_bits(instance.crop_rows[1]), phases[0]);
            let packed_along = instance.crop_rows[2];
            assert!(
                ((packed_along & 0xffff) as f32 * 0.01
                    - managed_along_spacing_m(archetype, descriptor.field_seed).unwrap())
                .abs()
                    <= 0.005
            );
            let encoded_phase = (packed_along >> 16) as f32 / 65_536.0;
            let phase_error = (encoded_phase - phases[1])
                .abs()
                .min(1.0 - (encoded_phase - phases[1]).abs());
            assert!(phase_error <= 1.0 / 65_536.0);
        }
    }

    #[test]
    fn ordinary_shader_does_not_reference_landmark_work() {
        let source = include_str!("shader.wgsl");
        let ordinary = source
            .split_once("fn surface_vertex")
            .unwrap()
            .1
            .split_once("fn landmark_vertex")
            .unwrap()
            .0;
        assert!(!ordinary.contains("landmark_model"));
        assert!(!ordinary.contains("is_landmark"));
        assert!(source.contains("fn vs_landmark"));
        assert!(source.contains("fn vs_packed_landmark"));
    }

    #[test]
    fn surface_colors_use_a_discrete_srgb_lookup_without_power_operations() {
        let source = include_str!("shader.wgsl");
        assert!(source.contains("const SRGB_TO_LINEAR: array<f32, 256>"));
        let lookup = source
            .split_once("fn srgb_to_linear_u8")
            .unwrap()
            .1
            .split_once("fn surface_vertex")
            .unwrap()
            .0;
        assert!(!lookup.contains("pow("));
        assert!(source.contains("@location(2) rgba: vec4<u32>"));
    }

    #[test]
    fn terrain_indices_use_u16_only_when_lossless_and_write_aligned() {
        let packed = pack_indices_u16(&[0, 1, 65_535, 2]).unwrap();
        assert_eq!(packed, [0, 1, 65_535, 2]);
        assert!(pack_indices_u16(&[0, 1, 65_536, 2]).is_none());
        assert!(pack_indices_u16(&[0, 1, 2]).is_none());
        assert_eq!(bytemuck::cast_slice::<_, u8>(&packed).len() % 4, 0);
    }

    #[test]
    fn tree_surface_lods_preserve_normals_and_fit_compact_indices() {
        for profile in vearth_gen::vegetation::TREE_PROFILES {
            let built = build_tree_asset(profile.id);
            assert!(
                built
                    .vertices
                    .iter()
                    .all(|v| vearth_core::mesh::decode_surface_normal(v.face_ao).is_some())
            );
            let BuiltTreeIndices::U16(indices) = built.indices else {
                panic!("bounded tree asset");
            };
            assert!(
                indices
                    .iter()
                    .all(|i| usize::from(*i) < built.vertices.len())
            );
            assert_eq!(built.lod_indices[0].start, 0);
            assert_eq!(built.lod_indices[0].end, built.lod_indices[1].start);
            assert_eq!(built.lod_indices[1].end, built.lod_indices[2].start);
            assert_eq!(built.lod_indices[2].end as usize, indices.len());
            for (range, budget) in built.lod_indices.iter().zip([3000, 1000, 300]) {
                assert!(!range.is_empty() && range.len() / 3 <= budget);
            }
        }
        assert_eq!(tree_lod_assignments(180.0).0[0].lod, 0);
        assert_eq!(tree_lod_assignments(80.0).0[0].lod, 1);
        assert_eq!(tree_lod_assignments(20.0).0[0].lod, 2);
    }

    #[test]
    fn geometric_tree_lod_masks_preserve_exactly_one_representation_through_both_fades() {
        let keep = |alpha: f32, noise: f32| {
            if alpha >= 0.0 {
                noise < alpha
            } else {
                noise >= 1.0 + alpha
            }
        };
        for pixels in (0..=440).map(|n| f64::from(n) * 0.5) {
            let (assignments, count) = tree_lod_assignments(pixels);
            assert!((1..=2).contains(&count));
            if count == 2 {
                assert_eq!(assignments[1].lod, assignments[0].lod + 1);
            }
            for sample in 0..128 {
                let noise = sample as f32 / 128.0;
                assert_eq!(
                    assignments[..count]
                        .iter()
                        .filter(|a| keep(a.alpha, noise))
                        .count(),
                    1,
                    "LOD coverage failed at {pixels}px, noise {noise}"
                );
                for near_blend in [0.0, 0.25, 0.5, 1.0] {
                    let lod_noise = ((sample * 53 + 17) % 128) as f32 / 128.0;
                    let streamed = usize::from(noise >= near_blend);
                    let instanced = assignments[..count]
                        .iter()
                        .filter(|a| noise < near_blend && keep(a.alpha, lod_noise))
                        .count();
                    assert_eq!(
                        streamed + instanced,
                        1,
                        "nested tree transitions opened a hole"
                    );
                }
            }
        }
        let weights = |pixels| {
            let (assignments, count) = tree_lod_assignments(pixels);
            let mut result = [0.0; 3];
            for a in &assignments[..count] {
                result[a.lod] = a.alpha.abs();
            }
            result
        };
        for boundary in [40.0, 45.0, 50.0, 135.0, 150.0, 165.0] {
            let before = weights(boundary - 0.001);
            let after = weights(boundary + 0.001);
            assert!(
                before
                    .into_iter()
                    .zip(after)
                    .all(|(a, b)| (a - b).abs() < 0.001)
            );
        }
    }

    #[test]
    fn night_light_admission_retains_terminator_cells_and_honors_global_switches() {
        let lighting = Lighting {
            direction_ecef: DVec3::X,
            moon_direction_ecef: -DVec3::X,
            moon_illumination: 1.0,
            utc_seconds: 0.0,
        };
        let earth_radius = 6_378_137.0;
        let at_solar_sine =
            |sine: f64| DVec3::new(sine, (1.0 - sine * sine).sqrt(), 0.0) * earth_radius;
        assert!(!night_light_cell_can_contribute(
            false,
            lighting,
            -DVec3::X * earth_radius,
            100.0,
        ));
        assert!(!night_light_cell_can_contribute(
            true,
            lighting,
            DVec3::X * earth_radius,
            100.0,
        ));
        assert!(night_light_cell_can_contribute(
            true,
            lighting,
            -DVec3::X * earth_radius,
            100.0,
        ));
        assert!(night_light_cell_can_contribute(
            true,
            lighting,
            at_solar_sine(0.07),
            100.0,
        ));
        assert!(!night_light_cell_can_contribute(
            true,
            lighting,
            at_solar_sine(0.10),
            100.0,
        ));
        assert!(night_light_cell_can_contribute(
            true,
            lighting,
            at_solar_sine(0.10),
            300_000.0,
        ));
    }

    #[test]
    fn every_rejected_light_sphere_is_fully_dark_in_the_shader() {
        let lighting = Lighting {
            direction_ecef: DVec3::new(0.7, -0.4, 0.3).normalize(),
            moon_direction_ecef: -DVec3::X,
            moon_illumination: 1.0,
            utc_seconds: 0.0,
        };
        let mut rejected = 0;
        for latitude in -8..=8 {
            for longitude in 0..36 {
                let center = vearth_core::ellipsoid::geodetic_to_ecef(
                    (latitude as f64 * 10.0).to_radians(),
                    (longitude as f64 * 10.0).to_radians(),
                    1000.0,
                );
                for radius in [0.0, 100.0, 10_000.0, 300_000.0, 2_000_000.0] {
                    if night_light_cell_can_contribute(true, lighting, center, radius) {
                        continue;
                    }
                    rejected += 1;
                    // Probe a dense sphere, including distant cells spanning
                    // different solar elevations than the camera locality.
                    for z in -8..=8 {
                        for azimuth in 0..24 {
                            let polar = z as f64 / 8.0;
                            let angle = azimuth as f64 * std::f64::consts::TAU / 24.0;
                            let horizontal = (1.0 - polar * polar).sqrt();
                            let position = center
                                + DVec3::new(
                                    horizontal * angle.cos(),
                                    horizontal * angle.sin(),
                                    polar,
                                ) * radius;
                            assert!(position.normalize().dot(lighting.direction_ecef) >= 0.07);
                        }
                    }
                }
            }
        }
        assert!(rejected > 1000);
    }

    #[test]
    fn tree_cell_readiness_early_out_preserves_exact_transition_and_asset_updates() {
        use vearth_platform::time::Duration;
        let instance = |origin, profile| TreeInstanceGpu {
            profile,
            basis: DMat3::IDENTITY,
            origin,
            height_m: 12.0,
            color_scale: 1.0,
        };
        let camera = DVec3::new(6_378_137.125, -98_100.0, 15_900.25);
        for distance in [0.0, 280.0, 320.0, 359.999, 360.0, 360.001, 9000.0] {
            let trees = [
                instance(camera + DVec3::Y * 12.0, TreeProfileId::EnglishOak),
                instance(camera + DVec3::X * distance, TreeProfileId::NorwaySpruce),
                instance(camera + DVec3::Z * 24.0, TreeProfileId::EnglishOak),
            ];
            for spruce_age in [
                None,
                Some(Duration::ZERO),
                Some(Duration::from_millis(37)),
                Some(Duration::from_millis(120)),
                Some(Duration::from_secs(1)),
            ] {
                let age = |profile| {
                    if profile == TreeProfileId::NorwaySpruce {
                        spruce_age
                    } else {
                        Some(Duration::from_secs(1))
                    }
                };
                let farthest = trees
                    .iter()
                    .map(|tree| tree.origin.distance_squared(camera))
                    .fold(0.0_f64, f64::max)
                    .sqrt();
                let expected =
                    ready_tree_cell_blend(farthest, trees.iter().map(|tree| age(tree.profile)));
                let mut lookups = 0;
                let actual = tree_cell_readiness_blend(&trees, camera, |profile| {
                    lookups += 1;
                    age(profile)
                });
                assert_eq!(actual.to_bits(), expected.to_bits());
                if distance >= NEAR_TREE_FADE_OUTER_M {
                    assert_eq!(
                        lookups, 0,
                        "out-of-range cells must not inspect asset readiness"
                    );
                }
            }
        }
        assert_eq!(
            tree_cell_readiness_blend(&[], camera, |_| panic!("empty cell has no asset")),
            0.0
        );
    }

    #[test]
    fn tree_readiness_waits_for_every_profile_then_fades_without_per_tree_state() {
        use vearth_platform::time::Duration;
        let old = Some(Duration::from_secs(1));
        assert_eq!(ready_tree_cell_blend(0.0, []), 0.0);
        assert_eq!(ready_tree_cell_blend(0.0, [old, None]), 0.0);
        assert_eq!(ready_tree_cell_blend(0.0, [old, Some(Duration::ZERO)]), 0.0);
        let mut previous = 0.0;
        for ms in 0..=240 {
            let blend = ready_tree_cell_blend(320.0, [old, Some(Duration::from_millis(ms))]);
            assert!(blend >= previous && blend <= 0.5);
            previous = blend;
        }
        assert_eq!(previous, near_tree_blend(320.0));
        assert!(
            (ready_tree_cell_blend(0.0, [old, Some(Duration::from_millis(120))]) - 0.5).abs()
                < 1e-6
        );
        assert_eq!(ready_tree_cell_blend(360.0, [old]), 0.0);
        for age_ms in [0, 1, 37, 120, 239, 240, 500] {
            let age = Duration::from_millis(age_ms);
            let delay = TREE_READINESS_FADE.saturating_sub(age);
            assert_eq!(
                tree_readiness_blend(age + delay),
                1.0,
                "offline settling must finish the fade before any measured frame"
            );
            assert!(delay <= TREE_READINESS_FADE);
            assert_eq!(delay.is_zero(), age >= TREE_READINESS_FADE);
        }
    }

    #[test]
    fn tree_instance_metric_counts_physical_trees_through_lod_blends_and_admission() {
        let cell = CellId {
            face: vearth_core::cubesphere::Face(0),
            level: 19,
            i: 31,
            j: 31,
        };
        for pixels in [20.0, 40.1, 45.0, 49.9, 80.0, 135.1, 150.0, 164.9, 180.0] {
            let (_, submissions) = tree_lod_assignments(pixels);
            let ids = std::iter::repeat_n((cell, 7), submissions).collect::<Vec<_>>();
            assert_eq!(distinct_tree_instance_count(ids.iter().copied()), 1);
            assert_eq!(
                ids.len(),
                if (40.0..50.0).contains(&pixels) || (135.0..165.0).contains(&pixels) {
                    2
                } else {
                    1
                },
                "the upload budget must still count both transition representations"
            );
        }
        let other_cell = CellId { i: 32, ..cell };
        let ids = [(cell, 0), (cell, 0), (cell, 1), (other_cell, 0)];
        assert_eq!(distinct_tree_instance_count(ids), 3);
        let admitted = admit_tree_cells(ids.iter().map(|(cell, _)| *cell), 3);
        assert_eq!(admitted, HashSet::from([cell]));
        let submissions: Vec<_> = ids
            .into_iter()
            .filter(|(cell, _)| admitted.contains(cell))
            .collect();
        assert_eq!(submissions.len(), 3);
        assert_eq!(distinct_tree_instance_count(submissions), 2);
        assert_eq!(distinct_tree_instance_count([]), 0);
    }

    #[test]
    fn tree_admission_counts_both_lod_draws_and_never_partially_hides_a_cell() {
        let cell = |i| CellId {
            face: vearth_core::cubesphere::Face(0),
            level: 19,
            i,
            j: 31,
        };
        let [a, b, c] = [cell(31), cell(32), cell(33)];
        // Two transitioning trees require four uniform/draw instances, even
        // though they are only two physical objects in the first cell.
        let draws = [a, a, a, a, b, b, c];
        let accepted = admit_tree_cells(draws.into_iter(), 5);
        assert_eq!(accepted, HashSet::from([a, c]));
        assert_eq!(
            draws.iter().filter(|cell| accepted.contains(cell)).count(),
            5
        );
        for capacity in 0..=draws.len() {
            let accepted = admit_tree_cells(draws.into_iter(), capacity);
            assert!(draws.iter().filter(|cell| accepted.contains(cell)).count() <= capacity);
            let fallback_blends: HashMap<_, _> = [a, b, c]
                .into_iter()
                .filter(|cell| accepted.contains(cell))
                .map(|cell| (cell, 0.6_f32))
                .collect();
            for cell in [a, b, c] {
                if !accepted.contains(&cell) {
                    assert_eq!(fallback_blends.get(&cell).copied().unwrap_or_default(), 0.0);
                }
            }
        }
    }

    #[test]
    fn tree_batch_key_groups_species_inside_coarse_depth_bands() {
        let oak_near = tree_batch_key(TreeProfileId::EnglishOak, 39.0_f64.powi(2));
        let oak_same_band = tree_batch_key(TreeProfileId::EnglishOak, 1.0);
        let oak_next_band = tree_batch_key(TreeProfileId::EnglishOak, 40.0_f64.powi(2));
        let beech_near = tree_batch_key(TreeProfileId::EuropeanBeech, 1.0);
        assert_eq!(oak_near, oak_same_band);
        assert_ne!(oak_near, oak_next_band);
        assert_ne!(oak_near, beech_near);
        assert!(
            oak_near < oak_next_band,
            "batch order must remain near-to-far"
        );
    }

    #[test]
    fn tree_batching_requires_enough_instances_and_a_meaningful_draw_reduction() {
        assert!(!should_batch_tree_instances(
            TREE_INSTANCE_BATCH_THRESHOLD - 1,
            1
        ));
        assert!(!should_batch_tree_instances(16, 13));
        assert!(should_batch_tree_instances(16, 12));
        assert!(should_batch_tree_instances(2_048, 64));
    }

    #[test]
    fn near_tree_blend_is_smooth_monotonic_and_bounded() {
        assert_eq!(near_tree_blend(NEAR_TREE_FADE_OUTER_M + 1.0), 0.0);
        assert_eq!(near_tree_blend(NEAR_TREE_FADE_OUTER_M), 0.0);
        assert_eq!(near_tree_blend(NEAR_TREE_FADE_INNER_M), 1.0);
        assert_eq!(near_tree_blend(NEAR_TREE_FADE_INNER_M - 1.0), 1.0);
        let outer_half = near_tree_blend(340.0);
        let midpoint = near_tree_blend(320.0);
        let inner_half = near_tree_blend(300.0);
        assert!(0.0 < outer_half && outer_half < midpoint);
        assert!((midpoint - 0.5).abs() < f32::EPSILON);
        assert!(midpoint < inner_half && inner_half < 1.0);
    }

    #[test]
    fn tree_shader_uses_compact_record_and_precomputed_axes() {
        let source = include_str!("shader.wgsl");
        assert!(source.contains("struct TreeU"));
        assert!(source.contains("let t = trees[tree_index]"));
        assert!(source.contains("n = t.bx.xyz"));
        assert!(source.contains("n = t.by.xyz"));
        assert!(source.contains("n = t.bz.xyz"));
        assert!(source.contains("out.sample_pitch_m = length(t.model[0].xyz) * t.fade.w"));
        assert!(source.contains("out.color = srgb_to_linear_u8(rgba.rgb) * t.fade.z"));
        assert!(source.contains("n = surface_normal(face_ao, t.model, n)"));
        assert!(source.contains("pixel_noise >= near_blend"));
        assert!(source.contains("pixel_noise < near_blend"));
        assert!(source.contains("out.tree_lod_alpha = t.bx.w"));
        assert!(source.contains("lod_noise >= 1.0 + alpha"));
        assert!(source.contains("lod_noise < alpha"));
        assert!(source.contains("ao = mix(0.68, 1.0, in.ao)"));
        assert!(!source.contains("normalize(t.model"));
    }

    #[test]
    fn arena_pages_release_capacity_without_moving_live_identifiers() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Page {
            id: usize,
            dropped: Arc<AtomicUsize>,
        }
        impl Drop for Page {
            fn drop(&mut self) {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
        let dropped = Arc::new(AtomicUsize::new(0));
        let mut pages = ArenaPages::default();
        let a = pages.insert(
            Page {
                id: 10,
                dropped: dropped.clone(),
            },
            100,
        );
        let b = pages.insert(
            Page {
                id: 20,
                dropped: dropped.clone(),
            },
            200,
        );
        let c = pages.insert(
            Page {
                id: 30,
                dropped: dropped.clone(),
            },
            300,
        );
        assert_eq!(pages.capacity_bytes, 600);
        drop(pages.remove(b));
        assert_eq!(dropped.load(Ordering::Relaxed), 1);
        assert_eq!(pages.len(), 2);
        assert_eq!(pages.capacity_bytes, 400);
        assert_eq!((pages[a].id, pages[c].id), (10, 30));
        assert!(pages.remove(b).is_none());
        // Repeated travel reuses empty slots without growing the index table
        // or changing either surviving allocation's page identifier.
        for id in 100..1100 {
            let replacement = pages.insert(
                Page {
                    id,
                    dropped: dropped.clone(),
                },
                500,
            );
            assert_eq!(replacement, b);
            assert_eq!(pages.slots.len(), 3);
            assert_eq!(pages.capacity_bytes, 900);
            assert_eq!((pages[a].id, pages[c].id), (10, 30));
            drop(pages.remove(replacement));
        }
        drop(pages.remove(a));
        drop(pages.remove(c));
        assert_eq!(pages.len(), 0);
        assert_eq!(pages.capacity_bytes, 0);
        assert_eq!(dropped.load(Ordering::Relaxed), 1003);
    }

    #[test]
    fn arena_reclamation_waits_for_all_aligned_allocations() {
        let mut allocator = RangeAllocator::new(256);
        assert!(allocator.is_empty());
        let a = allocator.allocate(13, 12).unwrap();
        let b = allocator.allocate(9, 20).unwrap();
        let c = allocator.allocate(7, 4).unwrap();
        assert!(!allocator.is_empty());
        allocator.free(b, 9);
        allocator.free(a, 13);
        assert!(!allocator.is_empty());
        allocator.free(c, 7);
        assert!(allocator.is_empty());
        assert_eq!(allocator.allocate(256, 4), Some(0));
        assert!(!allocator.is_empty());
        allocator.free(0, 256);
        assert!(allocator.is_empty());
    }

    #[test]
    fn arena_gpu_reclamation_preserves_live_ranges_and_encoded_commands() {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            eprintln!("arena lifetime GPU test skipped: no adapter");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("arena lifetime fixture"),
            ..Default::default()
        }))
        .expect("arena test device");
        let make_buffer = || {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("small test arena page"),
                size: 64,
                usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let make_page = || MeshArenaPage {
            vertex: make_buffer(),
            index: make_buffer(),
            vertices: RangeAllocator::new(64),
            indices: RangeAllocator::new(64),
        };
        let mut arena = MeshArena::new("test", 12);
        arena.pages.insert(make_page(), 128);
        arena.pages.insert(make_page(), 128);
        let a = arena.try_allocate(48, 16, 4).unwrap();
        let b = arena.try_allocate(48, 16, 4).unwrap();
        assert_ne!(a.page, b.page);
        // Failed paired allocation must roll back the vertex range before
        // deciding whether an existing page is empty.
        assert!(arena.try_allocate(12, 52, 4).is_none());
        assert_eq!(
            arena.pages[a.page].vertices.free,
            vec![FreeRange {
                offset: 48,
                size: 16
            }]
        );
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("arena lifetime readback"),
            size: 144,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        queue.write_buffer(&arena.pages[a.page].vertex, a.vertex_offset, &[11; 48]);
        queue.write_buffer(&arena.pages[b.page].vertex, b.vertex_offset, &[22; 48]);
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(
            &arena.pages[a.page].vertex,
            a.vertex_offset,
            &readback,
            0,
            48,
        );
        assert!(arena.free(a));
        assert_eq!(arena.pages.capacity_bytes, 128);
        assert_eq!(arena.pages.len(), 1);
        assert_eq!(arena.pages.insert(make_page(), 128), a.page);
        let replacement = arena.try_allocate(48, 16, 4).unwrap();
        assert_eq!(replacement.page, a.page);
        queue.write_buffer(
            &arena.pages[replacement.page].vertex,
            replacement.vertex_offset,
            &[33; 48],
        );
        encoder.copy_buffer_to_buffer(
            &arena.pages[b.page].vertex,
            b.vertex_offset,
            &readback,
            48,
            48,
        );
        encoder.copy_buffer_to_buffer(
            &arena.pages[replacement.page].vertex,
            replacement.vertex_offset,
            &readback,
            96,
            48,
        );
        assert!(arena.free(b));
        assert!(arena.free(replacement));
        assert_eq!(arena.pages.len(), 0);
        assert_eq!(arena.pages.capacity_bytes, 0);
        // All owning pages have gone before submit. Encoded buffer references
        // must still address their original data, including the reused slot.
        queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                tx.send(result).unwrap();
            });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("arena GPU wait");
        rx.recv_timeout(vearth_platform::time::Duration::from_secs(10))
            .unwrap()
            .unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        assert_eq!(&mapped[0..48], &[11; 48]);
        assert_eq!(&mapped[48..96], &[22; 48]);
        assert_eq!(&mapped[96..144], &[33; 48]);
        drop(mapped);
        readback.unmap();

        let mut lights = VertexArena::new("test lights", 12);
        lights.pages.insert(
            VertexArenaPage {
                buffer: make_buffer(),
                vertices: RangeAllocator::new(64),
            },
            64,
        );
        let first = lights.try_allocate(12).unwrap();
        let second = lights.try_allocate(24).unwrap();
        assert!(!lights.free(first));
        assert_eq!(lights.pages.capacity_bytes, 64);
        assert!(lights.free(second));
        assert_eq!(lights.pages.len(), 0);
        assert_eq!(lights.pages.capacity_bytes, 0);
    }

    #[test]
    fn arena_range_allocator_aligns_reuses_and_coalesces() {
        let mut allocator = RangeAllocator::new(256);
        let a = allocator.allocate(20, 20).unwrap();
        let b = allocator.allocate(24, 12).unwrap();
        let c = allocator.allocate(40, 20).unwrap();
        assert_eq!(a % 20, 0);
        assert_eq!(b % 12, 0);
        assert_eq!(c % 20, 0);
        allocator.free(b, 24);
        assert_eq!(allocator.allocate(12, 12), Some(b));
        allocator.free(a, 20);
        allocator.free(b, 12);
        allocator.free(c, 40);
        // Freeing every live allocation restores a single full page range.
        assert_eq!(
            allocator.free,
            vec![FreeRange {
                offset: 0,
                size: 256
            }]
        );
    }

    #[test]
    fn arena_range_allocator_matches_first_fit_reference_under_fragmentation() {
        fn reference_allocate(free: &mut Vec<FreeRange>, size: u64, alignment: u64) -> Option<u64> {
            let size = align_up(size, 4);
            for index in 0..free.len() {
                let range = free[index];
                let offset = align_up(range.offset, alignment);
                let end = offset.checked_add(size)?;
                if end > range.offset + range.size {
                    continue;
                }
                free.remove(index);
                if offset > range.offset {
                    free.insert(
                        index,
                        FreeRange {
                            offset: range.offset,
                            size: offset - range.offset,
                        },
                    );
                }
                let suffix = range.offset + range.size - end;
                if suffix > 0 {
                    free.push(FreeRange {
                        offset: end,
                        size: suffix,
                    });
                }
                free.sort_unstable_by_key(|range| range.offset);
                return Some(offset);
            }
            None
        }
        fn reference_free(free: &mut Vec<FreeRange>, offset: u64, size: u64) {
            free.push(FreeRange {
                offset,
                size: align_up(size, 4),
            });
            free.sort_unstable_by_key(|range| range.offset);
            let mut merged: Vec<FreeRange> = Vec::new();
            for range in free.drain(..) {
                if let Some(last) = merged.last_mut()
                    && last.offset + last.size == range.offset
                {
                    last.size += range.size;
                } else {
                    merged.push(range);
                }
            }
            *free = merged;
        }
        for capacity in [4096, 65536] {
            let mut allocator = RangeAllocator::new(capacity);
            let mut reference = allocator.free.clone();
            let mut live = Vec::new();
            let mut random = 0x10_ba51_u64;
            for _ in 0..20000 {
                random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                if random % 3 == 0 && !live.is_empty() {
                    let (offset, size) = live.swap_remove((random as usize >> 8) % live.len());
                    allocator.free(offset, size);
                    reference_free(&mut reference, offset, size);
                } else {
                    let alignment = [2, 4, 12, 20, 32][(random as usize >> 12) % 5];
                    let size = 1 + (random >> 24) % 1024;
                    let actual = allocator.allocate(size, alignment);
                    assert_eq!(actual, reference_allocate(&mut reference, size, alignment));
                    if let Some(offset) = actual {
                        live.push((offset, size));
                    }
                }
                assert_eq!(allocator.free, reference);
            }
            for (offset, size) in live {
                allocator.free(offset, size);
                reference_free(&mut reference, offset, size);
                assert_eq!(allocator.free, reference);
            }
            assert_eq!(
                allocator.free,
                vec![FreeRange {
                    offset: 0,
                    size: capacity
                }]
            );
        }
    }

    #[test]
    fn indirect_batching_requires_execution_and_first_instance_support() {
        let feature = wgpu::Features::INDIRECT_FIRST_INSTANCE;
        assert!(
            renderer_features(feature, wgpu::DownlevelFlags::INDIRECT_EXECUTION).contains(feature)
        );
        assert!(renderer_features(feature, wgpu::DownlevelFlags::empty()).is_empty());
    }

    #[test]
    fn ordinary_terrain_record_is_half_the_previous_shared_stride() {
        assert_eq!(std::mem::size_of::<TerrainU>() as u64, 128);
    }

    #[test]
    fn landmark_delta_maps_model_coordinates_between_enu_frames() {
        let landmark = vearth_gen::landmarks::LANDMARKS
            .iter()
            .find(|landmark| landmark.name == "eiffel-tower")
            .unwrap();
        let value = vearth_gen::landmarks::CalibrationOverride {
            name: landmark.name,
            east_m: 7.5,
            north_m: -4.0,
            up_m: 2.25,
            heading_deg: landmark.heading_deg + 23.0,
        };
        let resolved = vearth_gen::landmarks::resolve_calibration(value);
        let source_lat = landmark.lat_deg.to_radians();
        let source_lon = landmark.lon_deg.to_radians();
        let target_lat = resolved.lat_deg.to_radians();
        let target_lon = resolved.lon_deg.to_radians();
        let (source_basis, _) =
            landmark_model_basis(source_lat, source_lon, landmark.heading_deg.to_radians());
        let (target_basis, target_up) =
            landmark_model_basis(target_lat, target_lon, resolved.heading_deg.to_radians());
        let source_origin = vearth_core::ellipsoid::geodetic_to_ecef(source_lat, source_lon, 0.0);
        let target_origin = vearth_core::ellipsoid::geodetic_to_ecef(target_lat, target_lon, 0.0)
            + target_up * resolved.base_offset_m;
        let model_point = DVec3::new(12.0, 31.0, -18.0);
        let actual =
            landmark_delta_ecef(value).transform_point3(source_origin + source_basis * model_point);
        let expected = target_origin + target_basis * model_point;
        assert!(
            actual.distance(expected) < 1.0e-6,
            "{actual:?} != {expected:?}"
        );
    }

    #[test]
    fn flora_shader_rejects_only_whole_absent_primitives_before_expensive_vertex_work() {
        let shader = include_str!("flora.wgsl");
        let start = shader.find("fn vs_flora(").unwrap();
        let end = shader[start..].find("fn pixel_hash(").unwrap() + start;
        let vertex = &shader[start..end];
        let padding_rejection = vertex.find("if (glyph >= f.state.z)").unwrap();
        let glyph_rejection = vertex.find("if (!exists)").unwrap();
        let primitive_rejection = vertex.find("if (!primitive_exists)").unwrap();
        assert!(glyph_rejection < vertex.find("let root_y_m = ground_height").unwrap());
        assert!(padding_rejection < glyph_rejection);
        assert!(primitive_rejection < vertex.find("var shape = box_vertex").unwrap());
        assert!(primitive_rejection < vertex.find("out.color = archetype_color").unwrap());
        assert_eq!(
            vertex
                .matches("return absent_flora(f.params.z, f.params.w, archetype);")
                .count(),
            3
        );
        let existence = &vertex[vertex.find("let exists =").unwrap()..glyph_rejection];
        assert!(
            !existence.contains("primitive_vertex") && !existence.contains("box_corner_vertex")
        );
        let absent = &shader[shader.find("fn absent_flora(").unwrap()..start];
        assert!(absent.contains("out.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0)"));
        for field in [
            "color",
            "normal",
            "world_pos",
            "terrain_fade",
            "plant_fade",
            "archetype",
        ] {
            assert!(
                absent.contains(&format!("out.{field} =")),
                "uninitialized {field}"
            );
        }
        assert!(absent.contains("out.terrain_fade = terrain_fade"));
        assert!(absent.contains("out.plant_fade = plant_fade"));
    }

    #[test]
    fn material_atlas_endpoints_preserve_gradients_and_terrain_reuses_its_selector() {
        let shader = include_str!("shader.wgsl");
        let varied_start = shader.find("fn varied_material(").unwrap();
        let varied_end = shader.find("fn sample_material(").unwrap();
        let varied = &shader[varied_start..varied_end];
        let low = varied.find("if (blend <= 0.0)").unwrap();
        let high = varied.find("if (blend >= 1.0)").unwrap();
        let interior = varied.find("let a =").unwrap();
        assert!(varied.contains("smoothstep(0.15, 0.85, fract(selector))"));
        assert!(varied[low..high].contains(
            "return textureSampleGrad(atlas_tex, atlas_samp, uv + material_offset(index), i32(layer), dx, dy)"
        ));
        assert!(varied[high..interior].contains(
            "return textureSampleGrad(atlas_tex, atlas_samp, uv + material_offset(index + 1u), i32(layer), dx, dy)"
        ));
        assert_eq!(varied[interior..].matches("textureSampleGrad(").count(), 2);
        assert!(varied[interior..].contains("return mix(a, b, blend)"));
        assert!(!varied.contains("textureSample("));

        let wrapper_start = shader.find("fn sample_material_grad(").unwrap();
        let selected_start = shader.find("fn sample_material_selected_grad(").unwrap();
        let selected_end = shader.find("struct VehicleMaterial").unwrap();
        let wrapper = &shader[wrapper_start..selected_start];
        assert!(
            wrapper.find("if (detail <= 0.0)").unwrap()
                < wrapper.find("material_selector(position_m)").unwrap()
        );
        assert!(!shader[selected_start..selected_end].contains("material_selector("));
        let terrain = include_str!("terrain_material.wgsl");
        assert_eq!(terrain.matches("material_selector(position_m)").count(), 1);
        assert!(terrain.contains("let broad = selector / 7.0 - 0.5"));
        assert!(terrain.contains("sample_material_selected_grad(position_m, normal, family + 2u,"));
        assert!(terrain.contains("TERRAIN_PERIODS[family], detail, world_dx, world_dy, selector)"));
    }

    #[test]
    fn checked_in_shaders_parse_and_validate_without_a_gpu() {
        let world = world_shader_source();
        let flora = flora_shader_source();
        let ribbons = flora_shader_source_with_ribbons(true);
        let atmosphere = atmosphere_shader_source();
        for (name, source) in [
            ("world", world.as_str()),
            ("procedural flora", flora.as_str()),
            ("crop ribbons", ribbons.as_str()),
            ("Sun shadow depth", include_str!("shadow.wgsl")),
            ("night lights", include_str!("night_lights.wgsl")),
            ("aurora curtains", include_str!("aurora.wgsl")),
            ("atmosphere", atmosphere.as_str()),
            ("stars", include_str!("stars.wgsl")),
            ("vehicle exhaust", include_str!("vehicle_effects.wgsl")),
            ("tone map", include_str!("tonemap.wgsl")),
        ] {
            let module = naga::front::wgsl::parse_str(source).unwrap_or_else(|error| {
                panic!(
                    "{name} WGSL did not parse:\n{}",
                    error.emit_to_string(source)
                )
            });
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|error| panic!("{name} WGSL did not validate: {error}"));
        }
    }

    #[test]
    fn post_process_defaults_enable_each_independent_effect() {
        let settings = PostProcessSettings::default();
        assert!(settings.color_grading);
        assert!(settings.filmic_saturation);
        assert_eq!(
            PostProcessUniform::new(settings).flags,
            [1u32, 1u32, 0u32, 0u32]
        );
        assert_eq!(std::mem::size_of::<PostProcessUniform>(), 16);

        let disabled = PostProcessUniform::new(PostProcessSettings {
            color_grading: false,
            filmic_saturation: false,
        });
        assert_eq!(disabled.flags, [0u32; 4]);
    }

    #[test]
    fn color_grade_lut_is_complete_and_creative_not_identity() {
        let lut = build_color_grade_lut();
        assert_eq!(lut.len(), (COLOR_GRADE_LUT_SIZE.pow(3) * 4) as usize);
        assert!(lut.chunks_exact(4).all(|pixel| pixel[3] == 255));

        let middle = COLOR_GRADE_LUT_SIZE / 2;
        let offset = (((middle * COLOR_GRADE_LUT_SIZE + middle) * COLOR_GRADE_LUT_SIZE + middle)
            * 4) as usize;
        let identity = (middle as f32 / (COLOR_GRADE_LUT_SIZE - 1) as f32 * 255.0).round() as u8;
        assert_ne!(&lut[offset..offset + 3], &[identity; 3]);
    }

    #[test]
    fn tone_map_shader_covers_lut_and_perceptual_saturation() {
        let tone = include_str!("tonemap.wgsl");
        assert!(tone.contains("var grade_lut: texture_3d"));
        assert!(tone.contains("fn linear_rgb_to_oklab"));
        assert!(tone.contains("fn compress_gamut"));
        assert!(tone.contains("fn filmic_saturation"));
        assert!(!tone.contains("bloom"));
        assert!(!tone.contains("heat"));
    }

    #[test]
    fn world_surfaces_keep_backface_culling_while_player_surfaces_are_two_sided() {
        assert_eq!(
            surface_primitive_state(true).cull_mode,
            Some(wgpu::Face::Back)
        );
        assert_eq!(surface_primitive_state(false).cull_mode, None);
    }

    #[test]
    fn cloud_history_is_half_resolution_and_within_1080p_budget() {
        assert_eq!(cloud_target_dimensions(1920, 1080), (960, 540));
        assert_eq!(cloud_target_dimensions(1919, 1079), (960, 540));
        assert_eq!(cloud_target_dimensions(1, 1), (1, 1));
        assert_eq!(cloud_working_set_bytes(1920, 1080), 17_107_200);
        assert!(cloud_working_set_bytes(1920, 1080) <= 24 * 1024 * 1024);
    }

    #[test]
    fn cloud_lattice_matches_every_generated_voxel_before_byte_quantization() {
        for cells in [4, 5, 8, 10, 16, 20] {
            let lattice = CloudWorleyLattice::new(cells);
            for z in 0..CLOUD_SHAPE_SIZE {
                for y in 0..CLOUD_SHAPE_SIZE {
                    for x in 0..CLOUD_SHAPE_SIZE {
                        let point =
                            [x, y, z].map(|axis| (axis as f32 + 0.5) / CLOUD_SHAPE_SIZE as f32);
                        assert_eq!(
                            lattice.sample(point).to_bits(),
                            cloud_worley_f1_tiled(point, cells as i32).to_bits(),
                            "octave {cells}, voxel {x},{y},{z}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn cloud_shape_volume_is_dense_bounded_and_padded_for_upload() {
        let pixels = build_cloud_shape_volume();
        assert_eq!(
            pixels.len(),
            (CLOUD_SHAPE_BYTES_PER_ROW * CLOUD_SHAPE_SIZE * CLOUD_SHAPE_SIZE) as usize
        );
        let mut ranges = [(u8::MAX, u8::MIN); 4];
        let slice_stride = (CLOUD_SHAPE_BYTES_PER_ROW * CLOUD_SHAPE_SIZE) as usize;
        for z in 0..CLOUD_SHAPE_SIZE as usize {
            for y in 0..CLOUD_SHAPE_SIZE as usize {
                for x in 0..CLOUD_SHAPE_SIZE as usize {
                    let offset = z * slice_stride + y * CLOUD_SHAPE_BYTES_PER_ROW as usize + x * 4;
                    for channel in 0..4 {
                        ranges[channel].0 = ranges[channel].0.min(pixels[offset + channel]);
                        ranges[channel].1 = ranges[channel].1.max(pixels[offset + channel]);
                    }
                }
            }
        }
        for (minimum, maximum) in ranges {
            assert!(maximum.saturating_sub(minimum) > 96, "{minimum}..{maximum}");
        }
    }

    #[test]
    fn cloud_shader_calibration_matches_the_generated_shape_volume() {
        let source = include_str!("cloud_common.wgsl");
        let constant = |name: &str| -> f32 {
            source
                .split(&format!("const {name}: f32 = "))
                .nth(1)
                .and_then(|rest| rest.split(';').next())
                .unwrap_or_else(|| panic!("{name}"))
                .trim()
                .parse()
                .unwrap()
        };
        let detail_mean = constant("CLOUD_DETAIL_MEAN");
        let deciles: Vec<f32> = source
            .split("const CLOUD_COVERAGE_DECILES = array<f32, 11>(")
            .nth(1)
            .and_then(|rest| rest.split(')').next())
            .expect("coverage decile table")
            .split(',')
            .map(|value| value.trim().parse().unwrap())
            .collect();
        assert_eq!(deciles.len(), 11);
        let pixels = build_cloud_shape_volume();
        let side = CLOUD_SHAPE_SIZE as usize;
        let row = CLOUD_SHAPE_BYTES_PER_ROW as usize;
        let mut body = Vec::with_capacity(side.pow(3));
        let mut detail_sum = 0.0;
        for z in 0..side {
            for y in 0..side {
                for x in 0..side {
                    let offset = (z * side + y) * row + x * 4;
                    let [r, g, _, a] = [0, 1, 2, 3].map(|c| f32::from(pixels[offset + c]) / 255.0);
                    // The shader's body field before height profiles.
                    body.push(r * 0.74 + g * 0.26);
                    detail_sum += a;
                }
            }
        }
        let levels = cloud_shape_mip_chain(pixels.clone());
        let cluster_level = constant("CLOUD_CLUSTER_LOD") as usize;
        let cluster_side = side >> cluster_level;
        let mut cluster: Vec<f32> = (0..cluster_side.pow(3))
            .map(|index| {
                let (x, y, z) = (
                    index % cluster_side,
                    index / cluster_side % cluster_side,
                    index / cluster_side.pow(2),
                );
                f32::from(levels[cluster_level][(z * cluster_side + y) * row + x * 4 + 2]) / 255.0
            })
            .collect();
        cluster.sort_by(f32::total_cmp);
        let cluster_median = cluster[cluster.len() / 2];
        assert!(
            (cluster_median - constant("CLOUD_CLUSTER_MEDIAN")).abs() < 0.01,
            "cluster median {cluster_median}"
        );
        let measured_detail = detail_sum / body.len() as f32;
        assert!(
            (measured_detail - detail_mean).abs() < 0.005,
            "detail mean {measured_detail} != shader {detail_mean}"
        );
        for (index, threshold) in deciles.iter().enumerate().take(10).skip(1) {
            let coverage = index as f32 / 10.0;
            let covered =
                body.iter().filter(|value| **value > *threshold).count() as f32 / body.len() as f32;
            assert!(
                (covered - coverage).abs() < 0.02,
                "threshold {threshold} covers {covered}, expected {coverage}"
            );
        }
    }

    #[test]
    fn cloud_shape_mips_filter_all_three_axes_and_preserve_constant_channels() {
        let side = CLOUD_SHAPE_SIZE as usize;
        let row = CLOUD_SHAPE_BYTES_PER_ROW as usize;
        let mut base = vec![0; row * side * side];
        for z in 0..side {
            for y in 0..side {
                for x in 0..side {
                    let offset = (z * side + y) * row + x * 4;
                    base[offset..offset + 4].copy_from_slice(&[
                        ((x & 1) * 255) as u8,
                        ((y & 1) * 255) as u8,
                        ((z & 1) * 255) as u8,
                        64,
                    ]);
                }
            }
        }
        let levels = cloud_shape_mip_chain(base.clone());
        assert_eq!(levels[0], base);
        assert_eq!(levels.len(), CLOUD_SHAPE_MIP_LEVELS as usize);
        for (level, pixels) in levels.iter().enumerate().skip(1) {
            let side = side >> level;
            assert_eq!(pixels.len(), row * side * side);
            for z in 0..side {
                for y in 0..side {
                    for x in 0..side {
                        let offset = (z * side + y) * row + x * 4;
                        assert_eq!(&pixels[offset..offset + 4], &[128, 128, 128, 64]);
                    }
                }
            }
        }
    }

    #[test]
    fn cloud_history_handles_weather_clock_wrap_and_discontinuities() {
        assert!((wrapped_weather_delta(1.0, 65_535.0) - 2.0).abs() < f32::EPSILON);
        assert!((wrapped_weather_delta(65_535.0, 1.0) + 2.0).abs() < f32::EPSILON);

        let camera = DVec3::new(6_371_000.0, 0.0, 2_000.0);
        let mut weather = WeatherRenderState {
            weather0: [0.0; 4],
            weather1: [0.0; 4],
            wind: [0.0; 4],
            surface: [0.0; 4],
            weather2: [1.0, 0.25, 0.0, 0.0],
            weather3: [800.0, 3_200.0, 0.0, 1.0],
            weather4: [0.0; 4],
            weather5: [0.0; 4],
            weather6: [0.0; 4],
            wind_low: [0.0; 4],
            wind_mid: [0.0; 4],
            wind_upper: [0.0; 4],
        };
        let lighting = Lighting {
            direction_ecef: DVec3::X,
            moon_direction_ecef: DVec3::Y,
            moon_illumination: 0.5,
            utc_seconds: 0.0,
        };
        let previous = CloudHistoryState {
            camera_ecef: camera,
            view_proj: DMat4::IDENTITY,
            weather_time: 1.0,
            lightning_flash: 0.0,
            weather,
            lighting,
        };
        assert!(cloud_history_compatible(
            previous, camera, weather, lighting
        ));
        weather.weather2[0] = 59.0;
        assert!(cloud_history_compatible(
            previous, camera, weather, lighting
        ));
        weather.weather2[0] = 92.0;
        assert!(!cloud_history_compatible(
            previous, camera, weather, lighting
        ));
        weather.weather2[0] = 1.0;
        assert!(!cloud_history_compatible(
            previous,
            camera + DVec3::X * 100_001.0,
            weather,
            lighting,
        ));
        weather.weather2[1] = 0.5;
        assert!(!cloud_history_compatible(
            previous, camera, weather, lighting
        ));
        weather = previous.weather;
        weather.weather5[0] = 0.04;
        assert!(cloud_history_compatible(
            previous, camera, weather, lighting
        ));
        weather.weather5[0] += 0.04;
        assert!(!cloud_history_compatible(
            previous, camera, weather, lighting
        ));
        weather = previous.weather;
        weather.weather1[3] = 0.6;
        assert!(!cloud_history_compatible(
            previous, camera, weather, lighting
        ));
        weather = previous.weather;
        weather.surface[3] = 2.0;
        assert!(!cloud_history_compatible(
            previous, camera, weather, lighting
        ));
        weather = previous.weather;
        let sunset = Lighting {
            direction_ecef: DVec3::new(0.98, 0.2, 0.0).normalize(),
            ..lighting
        };
        assert!(!cloud_history_compatible(previous, camera, weather, sunset));
        let before_wrap = CloudHistoryState {
            weather_time: 65_535.0,
            ..previous
        };
        assert!(!cloud_history_compatible(
            before_wrap,
            camera,
            weather,
            lighting
        ));
        let late = CloudHistoryState {
            weather_time: 50_000.0,
            ..previous
        };
        weather.weather2[0] = 50_000.0;
        assert!(cloud_history_compatible(late, camera, weather, lighting));
        // The drift integrates wind changes, so they no longer move the field
        // away from its reprojected position.
        weather.wind_low[0] = 5.0;
        assert!(cloud_history_compatible(late, camera, weather, lighting));
    }

    #[test]
    fn cloud_drift_integrates_wind_and_wraps_on_every_tiling() {
        let source = include_str!("cloud_common.wgsl");
        for name in [
            "CLOUD_BODY_SCALE_M",
            "CLOUD_DETAIL_SCALE_M",
            "CLOUD_CLUSTER_SCALE_M",
        ] {
            let scale: f64 = source
                .split(&format!("const {name}: f32 = "))
                .nth(1)
                .and_then(|rest| rest.split(';').next())
                .unwrap_or_else(|| panic!("{name}"))
                .trim()
                .parse()
                .unwrap();
            let tiles = CLOUD_DRIFT_PERIOD_M / (4.0 * scale);
            assert!(
                (tiles - tiles.round()).abs() < 1e-9,
                "{name}: the drift wrap must shift whole tiles, got {tiles}"
            );
        }
        let camera = DVec3::new(6_372_000.0, 0.0, 0.0);
        let east = cloud_wind_ecef(camera, [10.0, 0.0, 0.0, 0.0]);
        assert!(east.dot(camera.normalize()).abs() < 1e-9);
        assert!((east.length() - 10.0).abs() < 1e-9);
        // A changing wind moves the field by its integral, one frame at a time.
        let mut drift = DVec3::ZERO;
        let mut previous = drift;
        for frame in 0..600 {
            let wind = cloud_wind_ecef(camera, [8.0 + frame as f32 * 0.01, 3.0, 0.0, 0.0]);
            drift = advance_cloud_drift(drift, wind, 10.0);
            let step = (drift - previous).to_array().map(|axis| {
                let wrapped = axis.rem_euclid(CLOUD_DRIFT_PERIOD_M);
                wrapped.min(CLOUD_DRIFT_PERIOD_M - wrapped)
            });
            assert!(
                DVec3::from_array(step).length() <= wind.length() * 10.0 + 1e-6,
                "frame {frame} moved the field by more than the wind"
            );
            assert!(drift.min_element() >= 0.0 && drift.max_element() < CLOUD_DRIFT_PERIOD_M);
            previous = drift;
        }
    }

    #[test]
    fn fast_time_shortens_cloud_history_instead_of_resetting_it() {
        let camera = DVec3::new(6_372_000.0, 0.0, 0.0);
        let weather = WeatherRenderState {
            weather0: [0.0, 0.6, 20_000.0, 0.0],
            weather1: [0.0; 4],
            wind: [0.0; 4],
            surface: [0.0, 0.0, 0.1, 1.0],
            weather2: [1_000.0, 0.25, 0.0, 0.0],
            weather3: [1_500.0, 3_200.0, 0.0, 1.0],
            weather4: [0.0; 4],
            weather5: [0.4, 0.2, 0.1, 2.0],
            weather6: [0.0; 4],
            wind_low: [8.0, 3.0, 0.0, 0.0],
            wind_mid: [0.0; 4],
            wind_upper: [0.0; 4],
        };
        let lighting = |seconds: f64| {
            let angle = seconds * std::f64::consts::TAU / 86_400.0;
            Lighting {
                direction_ecef: DVec3::new(angle.cos(), angle.sin(), 0.0),
                moon_direction_ecef: DVec3::Y,
                moon_illumination: 0.5,
                utc_seconds: seconds,
            }
        };
        let previous = CloudHistoryState {
            camera_ecef: camera,
            view_proj: DMat4::IDENTITY,
            weather_time: 1_000.0,
            lightning_flash: 0.0,
            weather,
            lighting: lighting(0.0),
        };
        for (rate, (min_cap, max_cap)) in
            [(1.0, (32.0, 32.0)), (60.0, (2.0, 6.0)), (600.0, (1.0, 1.0))]
        {
            let elapsed = rate / 60.0;
            let mut next = weather;
            next.weather2[0] += elapsed as f32;
            let light = lighting(elapsed);
            assert!(
                cloud_history_compatible(previous, camera, next, light),
                "{rate}x must not reset history"
            );
            let cap = cloud_history_sample_cap(cloud_appearance_step(
                previous.weather,
                next,
                previous.lighting,
                light,
            ));
            assert!((min_cap..=max_cap).contains(&cap), "{rate}x cap {cap}");
        }
    }

    #[test]
    fn cloud_shader_has_temporal_integration_and_depth_aware_composite() {
        let source = include_str!("atmosphere.wgsl");
        let terrain_source = include_str!("shader.wgsl");
        let renderer_source = include_str!("gfx.rs");
        assert!(source.contains("fn fs_cloud_march_interleaved"));
        assert!(source.contains("fn fs_cloud_resolve_interleaved"));
        assert!(source.contains("fn coarse_cloud_fallback"));
        assert!(
            source.contains("return coarse_cloud_fallback(coordinate, phase, opaque_distance)")
        );
        assert!(source.contains("let night_stability ="));
        assert!(source.contains("base_feedback = mix(base_feedback, night_feedback"));
        assert!(source.contains("update_coordinate * 4u + cloud_interleave_phase()"));
        assert!(source.contains("fn reproject_cloud_history"));
        assert!(source.contains("fn fs_cloud_composite"));
        assert!(source.contains("encode_cloud_distance(conservative_front)"));
        assert!(source.contains("encode_scene_distance(opaque_distance)"));
        assert!(source.contains("fn reconstruction_depth_weight"));
        assert!(source.contains("textureLoad(cloud_history_color"));
        assert!(source.contains("textureLoad(cloud_history_meta"));
        assert!(source.contains("current_has_terrain != history_has_terrain"));
        assert!(source.contains("let near_occluder_limit = clamp"));
        assert!(source.contains("let orbital_depth_mode = observer_altitude"));
        assert!(!source.contains("let below_or_inside_layer = observer_altitude"));
        assert!(!source.contains("ground_obscuring_weather"));
        assert!(!source.contains("if terrain_pixel && below_or_inside_layer"));
        assert!(source.contains("> max(cloud_top + 20000.0, 30000.0)"));
        let cloud_pass = renderer_source
            .rfind("label: Some(\"full-resolution depth-aware cloud upscale\")")
            .expect("cloud composite pass");
        let player_pass = renderer_source
            .rfind("label: Some(\"player after temporal clouds\")")
            .expect("post-cloud player pass");
        assert!(player_pass > cloud_pass);
        assert!(!source.contains("observer_altitude > cloud_top + 1000.0"));
        assert!(source.contains("let aerial_lod = smoothstep(45000.0, 130000.0"));
        assert!(source.contains("!cloud_common_depth_visible(opaque_distance, front_distance)"));
        assert!(source.contains("let step_budget = select(40u, select(72u, 128u"));
        assert!(
            source.contains("clamp(u32(ceil(span / 50.0)), min(16u, step_budget), step_budget)")
        );
        assert!(source.contains("let step_m = span / f32(steps)"));
        assert!(!source.contains("dense_mode"));
        assert!(source.contains("segment.x + (f32(i) + step_jitter) * step_m"));
        assert!(source.contains("let optical_step_m = min(step_m, 900.0)"));
        assert!(source.contains("var cloud_shape_tex: texture_3d<f32>"));
        assert!(source.contains("fn cloud_sun_visibility"));
        assert!(source.contains("light = cloud_sun_visibility("));
        assert!(source.contains("cloud_sunlight_column(point + light_dir * probe_reach)"));
        assert!(atmosphere_shader_source().contains("fn cloud_sunlight_column"));
        // Segment bounds, including the scene-depth clip, are numerically
        // tested through the shared helper in tests/cloud_material.rs.
        assert!(source.contains("let segment = cloud_common_local_segment(-center, rd,"));
        // Water reflects the sky only; cloud cover shades it without a march.
        assert!(!include_str!("sky_environment.wgsl").contains("cloud_common_"));
        assert!(!source.contains("if inner.x > 0.0 {"));
        assert!(source.contains("fn scene_distance_ndc"));
        assert!(source.contains("opaque_distance, in.pos.xy"));
        assert!(source.contains("relative, ground_r, base_m, top_m, g.weather5.x"));
        assert!(source.contains("direction, ground_r, base_m, top_m, g.weather5.x"));
        assert!(source.contains("if i >= steps"));
        assert!(include_str!("aerial_perspective.wgsl").contains("fn aerial_weather_medium"));
        assert!(terrain_source.contains("apply_aerial_perspective"));
        assert!(include_str!("flora.wgsl").contains("apply_aerial_perspective"));
    }

    #[test]
    fn precipitation_is_world_space_and_earth_down() {
        let source = include_str!("atmosphere.wgsl");
        assert!(source.contains("fn vs_precipitation"));
        assert!(source.contains("fn fs_precipitation_world"));
        assert!(source.contains("g.wind_low.z - effective_fall_speed"));
    }

    #[test]
    fn clear_sky_has_no_radial_solar_or_ice_halo_shapes() {
        let source = include_str!("atmosphere.wgsl");
        assert!(source.contains("textureSampleLevel(atmosphere_lut"));
        assert!(source.contains("fn fitted_sky_channel"));
        assert!(source.contains("fn fitted_clear_sky"));
        assert!(source.contains("let solar_hemisphere"));
        assert!(!source.contains("sun_halo"));
        assert!(!source.contains("let sunward"));
        assert!(!source.contains("halo22"));
        assert!(!source.contains("preetham_"));
        assert!(!source.contains("physical_multi_scattering"));
    }

    #[test]
    fn facade_shadow_gating_rejects_quantized_normal_false_sunlight() {
        let sun = crate::solar::sun_direction_ecef(1_774_008_000.0 + 3.0 * 3600.0);
        // The stage's south-facing wall at the equinox is geometrically
        // backlit, but 7-bit octahedral encoding tilts its normal into the Sun.
        let decoded = DVec3::new(1.0, 1.0, -125.0).normalize();
        assert!(decoded.dot(sun) > 0.0);
        for triangle in [
            [DVec3::ZERO, DVec3::Y * 4.0, (DVec3::X + DVec3::Y) * 4.0],
            [DVec3::ZERO, (DVec3::X + DVec3::Y) * 4.0, DVec3::X * 4.0],
        ] {
            let plane = (triangle[1] - triangle[0]).cross(triangle[2] - triangle[0]);
            let facing = plane.dot(decoded).signum();
            assert!(plane.dot(sun) * facing / plane.length() < 0.0);
        }
        let shader = include_str!("shader.wgsl");
        assert!(shader.contains("let plane = cross(world_dx, world_dy);"));
        assert!(shader.contains("let n = surface_material.normal;"));
        assert!(shader.contains("let key_visibility = sun_visibility(in.world_pos, shadow_ndl);"));
    }

    #[test]
    fn display_dither_preserves_dark_pigments_within_one_encoded_step() {
        let srgb = |linear: f64| {
            if linear <= 0.0031308 {
                linear * 12.92
            } else {
                1.055 * linear.powf(1.0 / 2.4) - 0.055
            }
        };
        let mut worst_old = 0.0_f64;
        for index in 0..=65535 {
            let linear = f64::from(index) / 65535.0;
            let step = (1.0 / 12.92_f64).max(1.3 * linear.sqrt() + 0.9749 * linear);
            let low = srgb((linear - step / 510.0).max(0.0));
            let high = srgb((linear + step / 510.0).min(1.0));
            let encoded_span = (high - low) * 255.0;
            assert!(
                encoded_span <= 1.001,
                "dither exceeds one display code at {linear}"
            );
            if (0.005..0.99).contains(&linear) {
                assert!(
                    encoded_span > 0.70,
                    "dither no longer covers a useful display interval"
                );
            }
            worst_old = worst_old.max(
                (srgb((linear + 0.5 / 255.0).min(1.0)) - srgb((linear - 0.5 / 255.0).max(0.0)))
                    * 255.0,
            );
        }
        assert!(
            worst_old > 12.0,
            "fixture must expose amplified linear-light dither"
        );
        let shader = include_str!("tonemap.wgsl");
        assert!(shader.contains("1.3 * sqrt(max(mapped, vec3<f32>(0.0))) + 0.9749 * mapped"));
        assert!(shader.contains("mapped += display_step *"));
    }

    #[test]
    fn construction_derivatives_keep_subpixel_relief_independent_of_texture_phase() {
        // Four-metre material shown over about 1000 pixels: adding the largest
        // legal world phase before differentiation loses visible slope precision.
        let step = 0.0037_f64;
        let mut shifted_error = 0.0_f32;
        let mut relative_error = 0.0_f32;
        for pixel in 0..1024 {
            let a = (f64::from(pixel) * step) as f32;
            let b = (f64::from(pixel + 1) * step) as f32;
            let expected = step / 4.0;
            let shifted = ((b + 8191.0) / 4.0) - ((a + 8191.0) / 4.0);
            let relative = (b - a) / 4.0;
            shifted_error = shifted_error.max((f64::from(shifted) / expected - 1.0).abs() as f32);
            relative_error =
                relative_error.max((f64::from(relative) / expected - 1.0).abs() as f32);
        }
        assert!(
            shifted_error > 0.10,
            "fixture must expose phase-induced slope noise"
        );
        assert!(relative_error < 0.0002);
        let shader = include_str!("construction_material.wgsl");
        assert!(!shader.contains("dpdx(uv)") && !shader.contains("dpdy(uv)"));
        assert_eq!(shader.matches("textureSampleGrad(").count(), 1);
        for axis in ["xy", "yz", "xz"] {
            assert!(shader.contains(&format!("uv_dx = world_dx.{axis} / 4.0;")));
            assert!(shader.contains(&format!("uv_dy = world_dy.{axis} / 4.0;")));
        }
    }

    #[test]
    fn construction_courses_agree_between_neighbouring_chunk_origins() {
        let phase_distance = |a: f64, b: f64| {
            let d = (a - b).rem_euclid(256.0);
            d.min(256.0 - d)
        };
        for (latitude, longitude) in [
            (0.0_f64, 12.0_f64),
            (45.4, 12.3),
            (35.0, 135.0),
            (-33.9, 18.4),
            (85.0, 179.99999),
            (89.9999, -179.99999),
        ] {
            let point = vearth_core::ellipsoid::geodetic_to_ecef(
                latitude.to_radians(),
                longitude.to_radians(),
                83.25,
            );
            let radial = point.normalize();
            let east = DVec3::new(-radial.y, radial.x, 0.0).normalize();
            let north = radial.cross(east);
            let exact = construction_material_origin(point)[1];
            for offset in [-128.0_f64, -16.0, -0.13, 0.0, 0.13, 16.0, 128.0] {
                let origin = point + east * offset + north * (offset * 0.73) + radial * 8.0;
                let base = construction_material_origin(origin)[1];
                // The geometric ellipsoid gradient is evaluated in f32 by the
                // vertex shader; compare to exact f64 geodetic altitude.
                let origin32 = origin.as_vec3();
                let up = (origin32 * glam::Vec3::new(1.0, 1.0, 1.0067395)).normalize();
                let delta = (point - origin).as_vec3();
                let rise = delta.dot(up);
                let curvature = (delta.length_squared() - rise * rise) / (2.0 * origin32.length());
                let actual = base + rise + curvature;
                assert!(
                    phase_distance(f64::from(actual), f64::from(exact)) < 0.002,
                    "construction course seam at {latitude},{longitude}, offset {offset}"
                );
            }
        }
    }

    #[test]
    fn aerial_horizon_samples_track_solar_direction_and_atmospheric_state() {
        let low = sky_model_lighting(0.04, 0.10, 0.5, 0.0);
        let high = sky_model_lighting(0.85, 0.10, 0.5, 0.0);
        let haze = sky_model_lighting(0.04, 0.5, 0.9, 0.0);
        let luma = |c: [f32; 4]| c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722;
        assert!(luma(low.horizon_radiance[0]) > luma(low.horizon_radiance[2]));
        assert_ne!(low.horizon_radiance, high.horizon_radiance);
        assert_ne!(low.horizon_radiance, haze.horizon_radiance);
        for state in [low, high, haze] {
            for ray in state.horizon_radiance {
                assert_eq!(ray[3], 0.0);
                assert!(ray[..3].iter().all(|v| v.is_finite() && *v >= 0.0));
            }
        }
    }

    #[test]
    fn fitted_sky_state_tracks_aerosol_and_stays_uniform_aligned() {
        let clean = sky_model_lighting(0.5, 0.04, 0.35, 0.0);
        let polluted = sky_model_lighting(0.5, 0.55, 0.82, 0.0);
        assert_eq!(std::mem::size_of::<SkyModelUniform>(), 128);
        assert_ne!(clean.uniform.values, polluted.uniform.values);
        assert_ne!(clean.daylight_ambient, polluted.daylight_ambient);
    }

    #[test]
    fn sky_ambient_is_finite_bounded_and_leaves_night_fading_to_the_shaders() {
        for solar in [-1.0, 0.0, 0.01, 0.15, 0.5, 0.866, 1.0] {
            for aerosol in [0.0, 0.08, 0.55, 1.0] {
                for humidity in [0.0, 0.85, 1.0] {
                    for snow in [0.0, 1.0] {
                        let lighting = sky_model_lighting(solar, aerosol, humidity, snow);
                        assert!(
                            lighting
                                .uniform
                                .values
                                .iter()
                                .flatten()
                                .all(|v| v.is_finite())
                        );
                        assert!(
                            lighting
                                .daylight_ambient
                                .into_iter()
                                .all(|v| v.is_finite() && v > 0.0 && v <= 0.400_001)
                        );
                    }
                }
            }
        }
        let horizon = sky_model_lighting(0.0, 0.08, 0.5, 0.0);
        let night = sky_model_lighting(-0.8, 0.08, 0.5, 0.0);
        assert_eq!(night.uniform.values, horizon.uniform.values);
        assert_eq!(night.daylight_ambient, horizon.daylight_ambient);
        let fallback = sky_model_lighting(f32::NAN, f32::INFINITY, f32::NAN, f32::NAN);
        let default = sky_model_lighting(0.5, 0.08, 0.5, 0.0);
        assert_eq!(fallback.uniform.values, default.uniform.values);
        assert_eq!(fallback.daylight_ambient, default.daylight_ambient);
    }

    #[test]
    fn sky_ambient_tracks_daylight_haze_and_snow_without_repainting_materials() {
        let luminance = |rgb: [f32; 3]| rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
        let noon = sky_model_lighting(0.866, 0.08, 0.45, 0.0).daylight_ambient;
        let dawn = sky_model_lighting(0.035, 0.08, 0.6, 0.0).daylight_ambient;
        let polluted = sky_model_lighting(0.866, 0.55, 0.82, 0.0).daylight_ambient;
        let snow = sky_model_lighting(0.866, 0.08, 0.45, 1.0).daylight_ambient;
        assert!((0.21..0.25).contains(&luminance(noon)));
        assert!(noon[2] > noon[1] && noon[1] > noon[0]);
        assert!(luminance(dawn) < luminance(noon) * 0.7);
        assert!(dawn[2] / dawn[0] < noon[2] / noon[0]);
        assert!(polluted[2] / polluted[0] < (noon[2] / noon[0]) * 0.8);
        assert!(luminance(polluted) > luminance(noon));
        assert!(luminance(snow) > luminance(noon));
        assert!(luminance(snow) < 0.32 && luminance(polluted) < 0.32);
        println!(
            "daylight ambient: noon={noon:.5?}, dawn={dawn:.5?}, polluted={polluted:.5?}, snow={snow:.5?}"
        );
    }

    #[test]
    fn sky_ambient_quadrature_matches_dense_solid_angle_integration() {
        use hw_skymodel::rgb::Channel;
        // Independent uniform-solid-angle integration of L * cos(theta).
        // The production path instead uses nonuniform Gaussian nodes.
        let reference = |state: &SkyState, elevation: f32| {
            let mut sum = [0.0_f64; 3];
            for row in 0..128 {
                let mu = (row as f32 + 0.5) / 128.0;
                let theta = mu.acos();
                let radius = (1.0 - mu * mu).sqrt();
                for column in 0..128 {
                    let phi = (column as f32 + 0.5) * std::f32::consts::TAU / 128.0;
                    let gamma = (mu * elevation.sin() + radius * phi.cos() * elevation.cos())
                        .clamp(-1.0, 1.0)
                        .acos();
                    for (channel, value) in [Channel::R, Channel::G, Channel::B]
                        .into_iter()
                        .zip(&mut sum)
                    {
                        *value +=
                            f64::from(state.radiance(theta, gamma, channel).max(0.0) * 2.0 * mu);
                    }
                }
            }
            sum.map(|v| (v / (128.0 * 128.0)) as f32)
        };
        for solar in [0.0_f32, 0.01, 0.035, 0.15, 0.5, 0.866, 1.0] {
            for turbidity in [1.0, 2.0, 6.0, 10.0] {
                for snow in [0.0, 1.0] {
                    let elevation = solar.asin();
                    let state = SkyState::new(&SkyParams {
                        elevation,
                        turbidity,
                        albedo: [0.12 + 0.63 * snow, 0.13 + 0.65 * snow, 0.15 + 0.67 * snow],
                    })
                    .unwrap();
                    let expected = reference(&state, elevation);
                    let actual = sky_diffuse_average(&state, elevation);
                    for channel in 0..3 {
                        let relative =
                            ((actual[channel] - expected[channel]) / expected[channel]).abs();
                        assert!(
                            relative < 0.025,
                            "solar={solar} turbidity={turbidity} snow={snow} channel={channel}: actual={actual:?}, expected={expected:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn light_lod_fades_partition_parent_energy_by_spatial_bucket() {
        let parent = CellId {
            face: vearth_core::cubesphere::Face(0),
            level: 12,
            i: 17,
            j: 23,
        };
        let child = parent.children()[0];
        let child_draw = DrawCell {
            resident: ResidentId::for_test(0, 1),
            cell: child,
            fade: 0.25,
        };
        let draws = HashMap::from([(child_draw.cell, child_draw.fade)]);
        let parent_weights = effective_light_fades(parent, 1.0, &draws);
        let child_weights = effective_light_fades(child, child_draw.fade, &draws);

        // children()[0] is low j (+v), which maps to high local z because
        // ChunkFrame local +z is -v. It covers the lower-left 2x2 buckets.
        for z in 0..4 {
            for x in 0..4 {
                let covered = z >= 2 && x < 2;
                let expected_parent = if covered { 0.75 } else { 1.0 };
                assert!((parent_weights[z][x] - expected_parent).abs() < 1.0e-6);
                if covered {
                    assert!((parent_weights[z][x] + child_weights[z][x] - 1.0).abs() < 1.0e-6);
                }
            }
        }

        let grandchild = child.children()[0];
        let nested = HashMap::from([(child, 0.5), (grandchild, 0.5)]);
        let parent_nested = effective_light_fades(parent, 1.0, &nested);
        let child_nested = effective_light_fades(child, 0.5, &nested);
        let grandchild_nested = effective_light_fades(grandchild, 0.5, &nested);
        assert!(
            (parent_nested[3][0] + child_nested[3][0] + grandchild_nested[3][0] - 1.0).abs()
                < 1.0e-6,
            "nested transitions must remain a partition of unity"
        );
    }

    #[test]
    fn mesh_accounting_adds_and_removes_exact_footprints() {
        let world = MeshFootprint {
            chunks: 3,
            vertex_buffers: 3,
            index_buffers: 3,
            vertex_bytes: 1_004,
            index_bytes: 508,
            flora_cells: 2,
            flora_descriptor_bytes: 224,
        };
        let player = MeshFootprint {
            chunks: 2,
            vertex_buffers: 2,
            index_buffers: 2,
            vertex_bytes: 128,
            index_bytes: 64,
            ..MeshFootprint::default()
        };

        let mut stats = GpuMeshStats::default();
        stats.add_world_cell(world);
        stats.add_player_parts(player);
        assert_eq!(stats.world_cells, 1);
        assert_eq!(stats.world_chunks, 3);
        assert_eq!(stats.player_parts, 2);
        assert_eq!(stats.buffer_count(), 10);
        assert_eq!(stats.total_bytes(), 1_704);
        assert_eq!(stats.flora_cells, 2);
        assert_eq!(stats.flora_descriptor_bytes, 224);

        stats.remove_world_cell(world);
        assert_eq!(stats.world_cells, 0);
        assert_eq!(stats.world_chunks, 0);
        assert_eq!(stats.player_parts, 2);
        assert_eq!(stats.buffer_count(), 4);
        assert_eq!(stats.total_bytes(), 192);

        stats.remove_player_parts(player);
        assert_eq!(stats, GpuMeshStats::default());
    }

    #[test]
    fn upload_accounting_is_cumulative_until_frame_drain() {
        let world = MeshFootprint {
            chunks: 2,
            vertex_buffers: 2,
            index_buffers: 2,
            vertex_bytes: 400,
            index_bytes: 200,
            ..MeshFootprint::default()
        };
        let player = MeshFootprint {
            chunks: 1,
            vertex_buffers: 1,
            index_buffers: 1,
            vertex_bytes: 80,
            index_bytes: 40,
            ..MeshFootprint::default()
        };
        let mut uploads = GpuUploadStats::default();
        uploads.record_world_cell(world);
        uploads.record_world_cell(world);
        uploads.record_player_parts(player);
        assert_eq!(uploads.world_cells, 2);
        assert_eq!(uploads.world_chunks, 4);
        assert_eq!(uploads.player_parts, 1);
        assert_eq!(uploads.buffer_count(), 10);
        assert_eq!(uploads.total_bytes(), 1_320);

        let drained = std::mem::take(&mut uploads);
        assert_eq!(drained.total_bytes(), 1_320);
        assert_eq!(uploads, GpuUploadStats::default());
    }

    #[test]
    fn draw_admission_reserves_player_capacity_and_reports_every_drop() {
        let admission = admit_draws(MAX_DRAWS + 10, 3);
        assert_eq!(admission.world, MAX_DRAWS - 3);
        assert_eq!(admission.player, 3);
        assert_eq!(admission.dropped_world, 13);
        assert_eq!(admission.dropped_player, 0);

        let admission = admit_draws(12, MAX_DRAWS + 4);
        assert_eq!(admission.world, 0);
        assert_eq!(admission.player, MAX_DRAWS);
        assert_eq!(admission.dropped_world, 12);
        assert_eq!(admission.dropped_player, 4);
    }

    #[test]
    fn timestamp_feature_requires_nonblocking_resolve() {
        let supported = timestamp_query_feature(
            wgpu::Features::TIMESTAMP_QUERY,
            wgpu::DownlevelFlags::NONBLOCKING_QUERY_RESOLVE,
        );
        assert_eq!(supported, wgpu::Features::TIMESTAMP_QUERY);

        assert!(
            timestamp_query_feature(
                wgpu::Features::empty(),
                wgpu::DownlevelFlags::NONBLOCKING_QUERY_RESOLVE,
            )
            .is_empty()
        );
        assert!(
            timestamp_query_feature(
                wgpu::Features::TIMESTAMP_QUERY,
                wgpu::DownlevelFlags::empty(),
            )
            .is_empty()
        );
    }

    #[test]
    fn timestamp_collection_delivers_every_valid_slot_without_regressing_latest() {
        let ticks = [100, 110, 120, 130, 140, 150, 160, 170, 180, 190, 200, 210];
        let mut latest_id = 10;
        let mut latest = decode_gpu_timings(ticks, 1.0);
        let mut completed = GpuTimingBatch::default();
        for id in [12, 8, 11] {
            accept_gpu_timing(
                &mut latest_id,
                &mut latest,
                Some(&mut completed),
                id,
                ticks,
                id as f64,
            );
        }
        assert_eq!(
            completed.iter().map(|sample| sample.id).collect::<Vec<_>>(),
            [12, 8, 11]
        );
        assert_eq!(completed.iter().count(), GPU_TIMING_RING_SIZE);
        assert_eq!(latest_id, 12);
        assert_eq!(latest, decode_gpu_timings(ticks, 12.0));
        for sample in completed.iter() {
            assert_eq!(
                Some(sample.timings),
                decode_gpu_timings(ticks, sample.id as f64)
            );
        }
        // A later collection can contain an older issued slot. It must still
        // be delivered, while an empty/invalid collection publishes no rows.
        let mut next = GpuTimingBatch::default();
        let mut invalid = ticks;
        invalid[1] = 0;
        accept_gpu_timing(
            &mut latest_id,
            &mut latest,
            Some(&mut next),
            20,
            invalid,
            1.0,
        );
        assert_eq!(next.iter().count(), 0);
        assert_eq!(latest_id, 12);
        accept_gpu_timing(&mut latest_id, &mut latest, Some(&mut next), 9, ticks, 9.0);
        assert_eq!(next.iter().map(|sample| sample.id).collect::<Vec<_>>(), [9]);
        assert_eq!(latest_id, 12);
        assert_eq!(latest, decode_gpu_timings(ticks, 12.0));
        assert_eq!(GpuTimingBatch::default().iter().count(), 0);
        // Ordinary rendering retains the existing latest-only HUD behavior.
        accept_gpu_timing(&mut latest_id, &mut latest, None, 7, ticks, 7.0);
        assert_eq!(latest_id, 12);
        accept_gpu_timing(&mut latest_id, &mut latest, None, 14, ticks, 14.0);
        assert_eq!(latest_id, 14);
        assert_eq!(latest, decode_gpu_timings(ticks, 14.0));
    }

    #[test]
    fn timestamp_ticks_decode_combined_world_flora_without_inventing_separate_time() {
        let timings = decode_gpu_timings(
            [
                100, 600, 650, 1_250, 1_300, 1_800, 1_850, 2_050, 2_100, 2_250, 2_300, 2_500,
            ],
            2.0,
        )
        .unwrap();
        assert!((timings.shadow_ms - 0.001).abs() < f64::EPSILON);
        assert!((timings.world_ms - 0.0012).abs() < f64::EPSILON);
        assert_eq!(timings.flora_ms, None);
        assert!((timings.atmosphere_ms - 0.001).abs() < f64::EPSILON);
        assert!((timings.cloud_ms - 0.0004).abs() < f64::EPSILON);
        assert!((timings.precipitation_ms - 0.0003).abs() < f64::EPSILON);
        assert!((timings.tonemap_ms - 0.0004).abs() < f64::EPSILON);
        assert!((timings.total_ms - 0.0048).abs() < f64::EPSILON);
    }

    #[test]
    fn timestamp_decode_rejects_invalid_period_and_order() {
        let valid = [100, 110, 120, 130, 140, 150, 160, 170, 180, 190, 200, 210];
        for pair in 0..GPU_QUERY_COUNT as usize / 2 {
            let mut reversed = valid;
            reversed[pair * 2 + 1] = reversed[pair * 2] - 1;
            assert!(decode_gpu_timings(reversed, 1.0).is_none());
        }
        assert!(decode_gpu_timings(valid, 0.0).is_none());
        assert!(decode_gpu_timings(valid, f64::NAN).is_none());
    }

    #[test]
    fn sun_shadow_policy_has_explicit_night_orbit_overcast_and_fallback_gates() {
        assert_eq!(SUN_SHADOW_WORKING_SET_BYTES, 9 * 1024 * 1024);
        assert_eq!(
            sun_shadow_capacity(MAX_SHADOW_CHUNKS - 1),
            (MAX_SHADOW_CHUNKS - 1, 0)
        );
        assert_eq!(
            sun_shadow_capacity(MAX_SHADOW_CHUNKS + 7),
            (MAX_SHADOW_CHUNKS, 7)
        );
        for shadows in ShadowPreference::ALL {
            assert_eq!(
                sun_shadow_policy_enabled(true, shadows, 0.5, 900.0, 2.0),
                shadows != ShadowPreference::Off,
            );
            assert!(!sun_shadow_policy_enabled(false, shadows, 0.5, 900.0, 2.0));
            assert!(!sun_shadow_policy_enabled(true, shadows, 0.0, 900.0, 2.0));
            assert!(!sun_shadow_policy_enabled(
                true, shadows, 0.5, 25_000.0, 2.0
            ));
            assert!(!sun_shadow_policy_enabled(true, shadows, 0.5, 900.0, 9.0));
        }
    }

    #[test]
    fn sun_shadow_cascades_are_bounded_snapped_and_include_offscreen_casters() {
        let cam = DVec3::new(vearth_core::ellipsoid::WGS84_A + 900.0, 0.0, 0.0);
        let sun = DVec3::new(0.70, 0.35, 0.62).normalize();
        let cascades = build_sun_shadow_cascades(
            cam,
            DMat4::IDENTITY,
            1.0,
            16.0 / 9.0,
            sun,
            900.0,
            [cam; SUN_SHADOW_CASCADES],
        )
        .expect("valid regional shadow cascades");
        assert!(cascades[0].split_far_m < cascades[1].split_far_m);
        for cascade in &cascades {
            let center_relative = cascade.center_ecef - cam;
            let projected = cascade.light_view_proj * center_relative.extend(1.0);
            assert!(projected.is_finite());
            assert!(projected.x.abs() < 1e-9 && projected.y.abs() < 1e-9);
            assert!((projected.z - 0.5).abs() < 1e-9);
        }
        assert_ne!(
            sun_shadow_cascade_mask(&cascades, cascades[0].center_ecef, 1.0),
            0
        );
        let outside = cascades[1].center_ecef + cascades[1].right * (cascades[1].radius_m * 2.5);
        assert_eq!(sun_shadow_cascade_mask(&cascades, outside, 1.0), 0);

        // Moving by much less than one shadow texel retains the same snapped
        // ECEF centre. A fixed ECEF receiver therefore projects identically.
        let moved_cam = cam + cascades[0].right * 0.001;
        let moved = build_sun_shadow_cascades(
            moved_cam,
            DMat4::IDENTITY,
            1.0,
            16.0 / 9.0,
            sun,
            900.0,
            [cam; SUN_SHADOW_CASCADES],
        )
        .unwrap();
        assert!(moved[0].center_ecef.distance(cascades[0].center_ecef) < 1e-6);
        let receiver = cascades[0].center_ecef + cascades[0].right * 20.0;
        let before = cascades[0].light_view_proj * (receiver - cam).extend(1.0);
        let after = moved[0].light_view_proj * (receiver - moved_cam).extend(1.0);
        assert!(before.distance(after) < 1e-9);

        let rendered = [Some(cascades[0]), Some(cascades[1])];
        assert!(!sun_shadow_requires_full_refresh(&rendered, &moved));
        assert!(!sun_shadow_projection_requires_refresh(
            &cascades[0],
            &moved[0]
        ));
        assert_eq!(
            select_sun_shadow_updates(&rendered, &moved, 0, true, 0, false),
            0
        );
        assert_eq!(
            select_sun_shadow_updates(&rendered, &moved, 0, true, 0, true),
            0b01
        );
        assert_eq!(
            select_sun_shadow_updates(&rendered, &moved, 0b11, true, 0, false),
            0b01
        );
        assert_eq!(
            select_sun_shadow_updates(&rendered, &moved, 0b11, true, 7, false),
            0b10
        );
        assert_eq!(
            select_sun_shadow_updates(&rendered, &moved, 0, false, 3, false),
            0b11
        );

        let translated_cam = cam + cascades[0].right * 10.0;
        let translated = build_sun_shadow_cascades(
            translated_cam,
            DMat4::IDENTITY,
            1.0,
            16.0 / 9.0,
            sun,
            900.0,
            [cam; SUN_SHADOW_CASCADES],
        )
        .unwrap();
        assert!(sun_shadow_projection_requires_refresh(
            &cascades[0],
            &translated[0]
        ));
        assert_eq!(
            select_sun_shadow_updates(&rendered, &translated, 0, true, 0, false),
            0b01
        );
        let mut teleported = moved;
        teleported[0].center_ecef += teleported[0].right * teleported[0].radius_m;
        assert!(sun_shadow_requires_full_refresh(&rendered, &teleported));

        let remote_cam = cam + cascades[0].right * (cascades[1].radius_m + 1.0);
        let provisional = build_sun_shadow_cascades(
            remote_cam,
            DMat4::IDENTITY,
            1.0,
            16.0 / 9.0,
            sun,
            900.0,
            [cam; SUN_SHADOW_CASCADES],
        )
        .unwrap();
        let rebased_anchors = std::array::from_fn(|index| {
            let cascade = provisional[index];
            let texel_m = 2.0 * cascade.radius_m / f64::from(SUN_SHADOW_RESOLUTION);
            snap_sun_shadow_center(remote_cam, cam, cascade.right, cascade.up, texel_m)
        });
        let rebased = build_sun_shadow_cascades(
            remote_cam,
            DMat4::IDENTITY,
            1.0,
            16.0 / 9.0,
            sun,
            900.0,
            rebased_anchors,
        )
        .unwrap();
        for index in 0..SUN_SHADOW_CASCADES {
            let change = sun_shadow_projection_change(&provisional[index], &rebased[index]);
            assert!(change.xy_texels < 1e-6);
            assert!(change.depth_units < 1e-6);
        }
    }

    #[test]
    fn sun_shadow_snap_phase_is_regional_not_earth_radius_scaled() {
        let earth_radius = vearth_core::ellipsoid::WGS84_A;
        let anchor = DVec3::new(earth_radius, 0.0, 0.0);
        let raw_center = anchor + DVec3::new(1_000.0, 2_000.0, 300.0);
        let right = DVec3::Y;
        // One 60 Hz frame of terrestrial rotation at 1x time.
        let angle = std::f64::consts::TAU / 86_164.090_5 / 60.0;
        let rotated_right = DVec3::new(angle.sin(), angle.cos(), 0.0);
        let texel_m = 3.0;

        let regional_phase_delta =
            (rotated_right.dot(raw_center - anchor) - right.dot(raw_center - anchor)) / texel_m;
        let absolute_phase_delta =
            (rotated_right.dot(raw_center) - right.dot(raw_center)) / texel_m;
        assert!(regional_phase_delta.abs() < 0.001);
        assert!(absolute_phase_delta.abs() > 2.0);

        let snapped = snap_sun_shadow_center(raw_center, anchor, right, DVec3::Z, texel_m);
        let relative = snapped - anchor;
        assert!((right.dot(relative) / texel_m).fract().abs() < 1e-9);
        assert!((DVec3::Z.dot(relative) / texel_m).fract().abs() < 1e-9);
    }

    #[test]
    fn sun_shadow_refresh_tracks_projected_change_not_nominal_time_scale() {
        let cam = DVec3::new(vearth_core::ellipsoid::WGS84_A + 900.0, 0.0, 0.0);
        let sun = DVec3::new(0.70, 0.35, 0.62).normalize();
        let rotate_ecef = |direction: DVec3, angle: f64| {
            let (sin, cos) = angle.sin_cos();
            DVec3::new(
                cos * direction.x - sin * direction.y,
                sin * direction.x + cos * direction.y,
                direction.z,
            )
        };
        let initial = build_sun_shadow_cascades(
            cam,
            DMat4::IDENTITY,
            1.0,
            16.0 / 9.0,
            sun,
            900.0,
            [cam; SUN_SHADOW_CASCADES],
        )
        .unwrap();
        let earth_rotation_per_second = std::f64::consts::TAU / 86_164.090_5;
        let near_update_interval_seconds = 2.0 / 60.0;
        let one_x = build_sun_shadow_cascades(
            cam,
            DMat4::IDENTITY,
            1.0,
            16.0 / 9.0,
            rotate_ecef(
                sun,
                earth_rotation_per_second * near_update_interval_seconds,
            ),
            900.0,
            [cam; SUN_SHADOW_CASCADES],
        )
        .unwrap();
        let accelerated = build_sun_shadow_cascades(
            cam,
            DMat4::IDENTITY,
            1.0,
            16.0 / 9.0,
            rotate_ecef(
                sun,
                earth_rotation_per_second * near_update_interval_seconds * 600.0,
            ),
            900.0,
            [cam; SUN_SHADOW_CASCADES],
        )
        .unwrap();

        let one_x_change = sun_shadow_projection_change(&initial[0], &one_x[0]);
        assert!(one_x_change.xy_texels < SUN_SHADOW_PROJECTION_UPDATE_TEXELS);
        assert!(one_x_change.depth_units < SUN_SHADOW_DEPTH_UPDATE_UNITS);
        assert!(!sun_shadow_projection_requires_refresh(
            &initial[0],
            &one_x[0]
        ));
        assert!(sun_shadow_projection_requires_refresh(
            &initial[0],
            &accelerated[0]
        ));
    }

    fn shadow_key(slot: u32, upload_generation: u64, chunk_index: usize) -> SunShadowCasterKey {
        SunShadowCasterKey {
            resident: ResidentId::for_test(slot, 7),
            upload_generation,
            chunk_index,
        }
    }
    fn shadow_population(
        keys: &[(SunShadowCasterKey, bool)],
        delta: Option<DMat4>,
    ) -> SunShadowCasterPopulation {
        SunShadowCasterPopulation {
            keys: keys.iter().map(|&(key, _)| (key, 0)).collect(),
            landmark_delta: shadow_landmark_delta(
                keys.iter().any(|&(_, landmark)| landmark),
                delta,
            ),
            had_player: false,
        }
    }
    fn population_matches(
        cached: &mut SunShadowCasterPopulation,
        keys: &[(SunShadowCasterKey, bool)],
        epoch: u64,
        delta: Option<DMat4>,
    ) -> bool {
        let mut probe = SunShadowCasterProbe::default();
        for &(key, landmark) in keys {
            probe.observe(cached, key, landmark, epoch);
        }
        probe.matches(cached, delta)
    }
    fn separated_shadow_cascades() -> [SunShadowCascade; 2] {
        [DVec3::ZERO, DVec3::X * 10_000.0].map(|center_ecef| SunShadowCascade {
            light_view_proj: DMat4::IDENTITY,
            center_ecef,
            right: DVec3::X,
            up: DVec3::Y,
            forward: DVec3::Z,
            radius_m: 100.0,
            depth_extent_m: 500.0,
            split_far_m: 100.0,
        })
    }
    #[test]
    fn sun_shadow_population_is_exact_order_independent_and_upload_sensitive() {
        let a = (shadow_key(3, 17, 0), false);
        let b = (shadow_key(4, 18, 2), false);
        let mut cache = shadow_population(&[a, b], None);
        assert!(population_matches(&mut cache, &[b, a, a, b], 1, None));
        assert!(population_matches(&mut cache, &[a, b], 2, None));
        assert!(
            !population_matches(&mut cache, &[a], 3, None),
            "removal must invalidate"
        );
        assert!(
            !population_matches(&mut cache, &[a, (shadow_key(4, 18, 3), false)], 4, None),
            "detail chunk identity must invalidate"
        );
        assert!(
            !population_matches(&mut cache, &[a, (shadow_key(4, 19, 2), false)], 5, None),
            "same resident reupload must invalidate"
        );
        let next_slot = (
            SunShadowCasterKey {
                resident: ResidentId::for_test(4, 8),
                ..b.0
            },
            false,
        );
        assert!(
            !population_matches(&mut cache, &[a, next_slot], 6, None),
            "reused resident slot must invalidate"
        );
        assert!(population_matches(&mut cache, &[b, a], 7, None));
    }
    #[test]
    fn gpu_reupload_under_same_resident_gets_a_new_shadow_generation() {
        let mut residents = ResidentGpuStore::default();
        let id = ResidentId::for_test(2, 3);
        residents.insert(id, empty_cell_gpu());
        let first = residents.get(id).unwrap().upload_generation;
        assert!(residents.insert(id, empty_cell_gpu()).is_some());
        let second = residents.get(id).unwrap().upload_generation;
        assert!(second > first);
        residents.remove(id);
        let reused = ResidentId::for_test(2, 4);
        residents.insert(reused, empty_cell_gpu());
        assert!(residents.get(reused).unwrap().upload_generation > second);
        assert!(residents.get(id).is_none());
    }
    #[test]
    fn sun_shadow_population_changes_are_local_to_admitted_cascades() {
        let cascades = separated_shadow_cascades();
        let near = shadow_key(1, 1, 0);
        let far = shadow_key(2, 2, 0);
        let remote = shadow_key(3, 3, 0);
        let mut caches = [
            shadow_population(&[(near, false)], None),
            shadow_population(&[(far, false)], None),
        ];
        let mut probe_frame = |candidates: &[(SunShadowCasterKey, DVec3, bool)], epoch| {
            let mut probes: [SunShadowCasterProbe; 2] =
                std::array::from_fn(|_| SunShadowCasterProbe::default());
            for &(key, center, admitted) in candidates {
                if !admitted {
                    continue;
                }
                let mask = sun_shadow_cascade_mask(&cascades, center, 1.0);
                for i in 0..2 {
                    if mask & (1 << i) != 0 {
                        probes[i].observe(&mut caches[i], key, false, epoch);
                    }
                }
            }
            (0..2).fold(0_u8, |mask, i| {
                mask | (u8::from(!probes[i].matches(&caches[i], None)) << i)
            })
        };
        let base = [
            (near, cascades[0].center_ecef, true),
            (far, cascades[1].center_ecef, true),
        ];
        assert_eq!(
            probe_frame(&[base[1], (remote, DVec3::Y * 100_000.0, true), base[0]], 1),
            0,
            "distant streaming must not invalidate either map"
        );
        assert_eq!(
            probe_frame(&[(near, cascades[0].center_ecef, false), base[1]], 2),
            1,
            "detail/horizon admission removal must invalidate near only"
        );
        assert_eq!(
            probe_frame(&[base[0], (far, cascades[1].center_ecef, false)], 3),
            2,
            "detail/horizon admission removal must invalidate far only"
        );
        assert_eq!(
            probe_frame(
                &[
                    (shadow_key(1, 9, 0), cascades[0].center_ecef, true),
                    base[1]
                ],
                4
            ),
            1
        );
        assert_eq!(probe_frame(&base, 5), 0);
    }
    #[test]
    fn sun_shadow_cache_uses_rendered_support_until_projection_refresh() {
        let rendered = separated_shadow_cascades();
        let mut desired = rendered;
        desired[0].center_ecef.x += 0.003;
        let point = DVec3::new(100.002, 0.0, 0.0);
        assert_eq!(sun_shadow_cascade_mask(&rendered, point, 0.0), 0);
        assert_eq!(sun_shadow_cascade_mask(&desired, point, 0.0), 1);
        assert!(!sun_shadow_projection_requires_refresh(
            &rendered[0],
            &desired[0]
        ));
        let cached = [Some(rendered[0]), Some(rendered[1])];
        assert_eq!(
            select_sun_shadow_updates(&cached, &desired, 0, true, 0, false),
            0
        );
        assert_eq!(
            select_sun_shadow_updates(&cached, &desired, 1, true, 0, false),
            1,
            "using desired membership too early would spuriously refresh"
        );
    }
    #[test]
    fn sun_shadow_landmark_pose_only_invalidates_maps_with_tagged_casters() {
        let a = (shadow_key(1, 1, 0), false);
        let b = (shadow_key(2, 2, 0), true);
        let mut ordinary = shadow_population(&[a], None);
        let mut landmark = shadow_population(&[b], None);
        let delta = Some(DMat4::from_translation(DVec3::X * 200.0));
        assert!(population_matches(&mut ordinary, &[a], 1, delta));
        assert!(!population_matches(&mut landmark, &[b], 1, delta));
        let (center, radius) = landmark_aware_chunk_bounds(DVec3::ZERO, 3.0, true, delta);
        assert_eq!(center, DVec3::X * 100.0);
        assert_eq!(radius, 103.0);
        assert!(center.distance(DVec3::ZERO) + 3.0 <= radius);
        assert!(center.distance(DVec3::X * 200.0) + 3.0 <= radius);
        assert_eq!(
            landmark_aware_chunk_bounds(DVec3::ZERO, 3.0, false, delta),
            (DVec3::ZERO, 3.0)
        );
        let mut moved = shadow_population(&[b], delta);
        assert!(population_matches(&mut moved, &[b], 2, delta));
        assert!(!population_matches(&mut moved, &[b], 3, None));
    }
    #[test]
    fn sun_shadow_capacity_limits_cannot_make_dropped_population_look_current() {
        let a = (shadow_key(1, 1, 0), false);
        let b = (shadow_key(2, 2, 0), false);
        let mut capped = shadow_population(&[a], None);
        assert!(!population_matches(&mut capped, &[a, b], 1, None));
        assert!(!population_matches(&mut capped, &[b, a], 2, None));
        assert!(population_matches(&mut capped, &[a], 3, None));
        let mut complete = shadow_population(&[a, b], None);
        assert!(population_matches(&mut complete, &[b, a], 4, None));
    }
    #[test]
    fn receiver_escape_admits_new_offscreen_casters_before_certifying_a_full_map() {
        let cascade = SunShadowCascade {
            light_view_proj: DMat4::IDENTITY,
            center_ecef: DVec3::ZERO,
            right: DVec3::Z,
            up: DVec3::Y,
            forward: -DVec3::X,
            radius_m: 20.0,
            depth_extent_m: 1_000.0,
            split_far_m: 10.0,
        };
        let desired = [cascade; SUN_SHADOW_CASCADES];
        let rendered = desired.map(Some);
        let mut supports = desired.map(|cascade| {
            ReceiverSupport::build(&cascade, DVec3::ZERO, DMat4::IDENTITY, 1.0, 1.0)
        });
        let mut recovery = [None; SUN_SHADOW_CASCADES];
        // A caster along the Sun ray lies outside the old view but shadows a
        // receiver exposed by the turn. A full map must include it immediately.
        let caster = DVec3::new(500.0, 0.0, 5.0);
        let eligible = sun_shadow_cascade_mask(&desired, caster, 0.25);
        assert_eq!(eligible, 3);
        assert_eq!(
            sun_shadow_receiver_mask(&supports, caster, 0.25, None, 0.0, eligible),
            0
        );
        let turned = DMat4::from_rotation_y(std::f64::consts::PI);
        let plan = plan_sun_shadow_receivers(
            &rendered,
            &supports,
            &mut recovery,
            &desired,
            DVec3::ZERO,
            turned,
            1.0,
            1.0,
            1,
        );
        let scheduled = select_sun_shadow_updates(&rendered, &desired, 0, true, 1, false);
        assert_eq!(scheduled, 0);
        assert_eq!(plan.forced_refresh_mask, 3);
        assert!(plan.supports.iter().all(Option::is_none));
        assert_eq!(
            sun_shadow_receiver_mask(&plan.supports, caster, 0.25, None, 0.0, eligible),
            3
        );
        assert!(supports.iter().all(Option::is_some));
        commit_sun_shadow_receivers(
            &mut supports,
            &mut recovery,
            plan,
            scheduled | plan.forced_refresh_mask,
        );
        assert!(supports.iter().all(Option::is_none));
        assert!(recovery.iter().all(Option::is_some));
    }

    #[test]
    fn receiver_recovery_holds_full_maps_through_turns_and_reenters_each_normal_slot() {
        let cascade = SunShadowCascade {
            light_view_proj: DMat4::IDENTITY,
            center_ecef: DVec3::ZERO,
            right: DVec3::Z,
            up: DVec3::Y,
            forward: -DVec3::X,
            radius_m: 20.0,
            depth_extent_m: 1_000.0,
            split_far_m: 10.0,
        };
        let desired = [cascade; SUN_SHADOW_CASCADES];
        let rendered = desired.map(Some);
        let initial_support = desired.map(|cascade| {
            ReceiverSupport::build(&cascade, DVec3::ZERO, DMat4::IDENTITY, 1.0, 1.0)
        });
        let mut supports = initial_support;
        let mut recovery = [None; SUN_SHADOW_CASCADES];
        for frame in 1..=16 {
            let view = DMat4::from_rotation_y(if frame % 2 == 0 {
                0.0
            } else {
                std::f64::consts::PI
            });
            let plan = plan_sun_shadow_receivers(
                &rendered,
                &supports,
                &mut recovery,
                &desired,
                DVec3::ZERO,
                view,
                1.0,
                1.0,
                frame,
            );
            assert_eq!(plan.forced_refresh_mask, if frame == 1 { 3 } else { 0 });
            assert_eq!(plan.reentry_dirty_mask, 0);
            assert!(plan.supports.iter().all(Option::is_none));
            commit_sun_shadow_receivers(
                &mut supports,
                &mut recovery,
                plan,
                plan.forced_refresh_mask,
            );
        }
        for frame in 17..=31 {
            let plan = plan_sun_shadow_receivers(
                &rendered,
                &supports,
                &mut recovery,
                &desired,
                DVec3::ZERO,
                DMat4::IDENTITY,
                1.0,
                1.0,
                frame,
            );
            assert_eq!(plan.forced_refresh_mask, 0);
            let mask = select_sun_shadow_updates(
                &rendered,
                &desired,
                plan.reentry_dirty_mask,
                true,
                frame,
                false,
            );
            assert_eq!(
                mask,
                match frame {
                    24 => 1,
                    31 => 2,
                    _ => 0,
                }
            );
            commit_sun_shadow_receivers(&mut supports, &mut recovery, plan, mask);
            assert_eq!(supports[0].is_some(), frame >= 24);
            assert_eq!(supports[1].is_some(), frame >= 31);
        }
        assert!(recovery.iter().all(Option::is_none));
        // Recovery must restore the original steady-view admission, including
        // upstream casters, rejected side geometry and conservative bounds.
        for caster in [
            DVec3::new(500.0, 0.0, -5.0),
            DVec3::new(500.0, 0.0, 5.0),
            DVec3::new(500.0, 20.0, -5.0),
        ] {
            for radius in [0.0, 0.25, 10.0] {
                assert_eq!(
                    sun_shadow_receiver_mask(&supports, caster, radius, None, 0.0, 3),
                    sun_shadow_receiver_mask(&initial_support, caster, radius, None, 0.0, 3),
                );
            }
        }
    }

    #[test]
    fn receiver_reentry_probe_resets_on_translation_and_invalid_projection() {
        let cascade = SunShadowCascade {
            light_view_proj: DMat4::IDENTITY,
            center_ecef: DVec3::ZERO,
            right: DVec3::Z,
            up: DVec3::Y,
            forward: -DVec3::X,
            radius_m: 20.0,
            depth_extent_m: 1_000.0,
            split_far_m: 10.0,
        };
        let desired = [cascade; SUN_SHADOW_CASCADES];
        let rendered = desired.map(Some);
        let supports = [None; SUN_SHADOW_CASCADES];
        let initial_probe =
            ReceiverSupport::build(&cascade, DVec3::ZERO, DMat4::IDENTITY, 1.0, 1.0);
        let mut recovery = [Some(SunShadowReceiverRecovery {
            probe: initial_probe,
            stable_since: 0,
        }); SUN_SHADOW_CASCADES];
        let moved = DVec3::Y * 3.0;
        let plan = plan_sun_shadow_receivers(
            &rendered,
            &supports,
            &mut recovery,
            &desired,
            moved,
            DMat4::IDENTITY,
            1.0,
            1.0,
            8,
        );
        assert_eq!(plan.reentry_dirty_mask, 0);
        assert!(plan.supports.iter().all(Option::is_none));
        assert!(
            recovery
                .iter()
                .all(|state| state.unwrap().stable_since == 8)
        );
        let plan = plan_sun_shadow_receivers(
            &rendered,
            &supports,
            &mut recovery,
            &desired,
            moved,
            DMat4::IDENTITY,
            f64::NAN,
            1.0,
            16,
        );
        assert_eq!(plan.reentry_dirty_mask, 0);
        assert!(plan.supports.iter().all(Option::is_none));
        assert!(recovery.iter().all(|state| state.unwrap().probe.is_none()));
        let plan = plan_sun_shadow_receivers(
            &rendered,
            &supports,
            &mut recovery,
            &desired,
            moved,
            DMat4::IDENTITY,
            1.0,
            1.0,
            17,
        );
        assert_eq!(plan.reentry_dirty_mask, 0);
        let plan = plan_sun_shadow_receivers(
            &rendered,
            &supports,
            &mut recovery,
            &desired,
            moved,
            DMat4::IDENTITY,
            1.0,
            1.0,
            25,
        );
        assert_eq!(plan.reentry_dirty_mask, 3);
        assert_eq!(plan.forced_refresh_mask, 0);
    }

    #[test]
    fn sun_shadow_per_cascade_dirty_state_keeps_update_cadence_and_clears_removed_player() {
        let desired = separated_shadow_cascades();
        let rendered = desired.map(Some);
        for frame in 0..16 {
            let eligible = match frame % 8 {
                0 | 2 | 4 | 6 => 1,
                7 => 2,
                _ => 0,
            };
            let possible = select_sun_shadow_updates(&rendered, &desired, 3, true, frame, false);
            assert_eq!(possible, eligible);
            for dirty in 0..4 {
                assert_eq!(
                    select_sun_shadow_updates(&rendered, &desired, dirty, true, frame, false),
                    dirty & eligible
                );
            }
            let mut cached = SunShadowCasterPopulation {
                had_player: true,
                ..Default::default()
            };
            assert_eq!(
                select_sun_shadow_updates(&rendered, &desired, 0, true, frame, cached.had_player),
                eligible & 1,
                "removal must clear on the next existing near slot"
            );
            cached.had_player = false;
            assert_eq!(
                select_sun_shadow_updates(&rendered, &desired, 0, true, frame, cached.had_player),
                0
            );
            assert_eq!(
                select_sun_shadow_updates(&rendered, &desired, 0, false, frame, false),
                3
            );
            for mask in 0..4 {
                assert_eq!(
                    sun_shadow_cascade_mask_for_updates(&desired, DVec3::ZERO, 1.0, mask),
                    1 & mask
                );
            }
        }
    }

    fn ranged_chunk(indices_u16: bool, ranges: Vec<RenderIndexRange>) -> ChunkGpu {
        ChunkGpu {
            detail: None,
            detail_visible: std::cell::Cell::new(false),
            allocation: ArenaAllocation {
                page: 0,
                vertex_offset: 120,
                vertex_bytes: 240,
                index_offset: 48,
                index_bytes: 96,
            },
            vertex_bytes: 240,
            index_bytes: 96,
            index_count: ranges
                .last()
                .map_or(24, |range| range.first_index + range.index_count),
            ranges,
            material_ranges: Vec::new(),
            shadow: None,
            indices_u16,
            packed_vertices: true,
            model: DMat4::IDENTITY,
            construction_height_phase: 0.0,
            axes: [DVec3::X, DVec3::Y, DVec3::Z],
            normal_parameters: [1.0; 4],
            bound_center: DVec3::ZERO,
            bound_radius: 20_000.0,
            bound_box: None,
            has_calibration_landmark: false,
            has_fragment_feedback: false,
            has_rare_surface: false,
        }
    }
    #[test]
    fn material_census_matches_actual_culled_and_coalesced_ranges() {
        let tagged_window = u32::from(vearth_gen::landmarks::CALIBRATION_MATERIAL_FLAG | 15) << 5;
        let vertices = [tagged_window, 12 << 5, 3 << 5, 18 << 5]
            .into_iter()
            .flat_map(|face_ao| {
                [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]].map(|pos| SurfaceVertex {
                    pos,
                    face_ao,
                    rgba: [80, 90, 100, 255],
                })
            })
            .collect::<Vec<_>>();
        let mut ranges = [0, 6, 9]
            .into_iter()
            .map(|first_index| render_range(first_index, DVec3::ZERO, 1.0))
            .collect::<Vec<_>>();
        ranges[1].index_count = 3;
        ranges[2].index_count = 3;
        let packed = vertices
            .iter()
            .copied()
            .map(PackedSurfaceVertex::try_from_full)
            .collect::<Option<Vec<_>>>()
            .unwrap();
        for vertices in [
            TerrainVertices::Full(vertices),
            TerrainVertices::Packed(packed),
        ] {
            for indices in [
                TerrainIndices::U16((0..12).collect()),
                TerrainIndices::U32((0..12).collect()),
            ] {
                let mut chunk = ranged_chunk(true, ranges.clone());
                chunk.material_ranges = material_range_counts(&vertices, &indices, &ranges);
                let mut draws = Vec::new();
                append_world_draw_ranges(&mut draws, &chunk, 0, false, true, |range| {
                    range.first_index != 6
                });
                let counts = submitted_material_indices(&draws);
                assert_eq!(counts.total(), 9);
                assert_eq!(counts.0[15], 3);
                assert_eq!(counts.0[12], 3);
                assert_eq!(counts.0[3], 0);
                assert_eq!(counts.0[18], 3);
                draws.clear();
                append_world_draw_ranges(&mut draws, &chunk, 0, false, true, |_| true);
                assert_eq!(draws.len(), 1);
                assert_eq!(submitted_material_indices(&draws).total(), 12);
                assert_eq!(submitted_material_indices(&draws).0[3], 3);
                chunk.ranges.clear();
                chunk.material_ranges = material_range_counts(&vertices, &indices, &[]);
                let mut whole = Vec::new();
                append_world_draw_ranges(&mut whole, &chunk, 0, true, false, |_| true);
                assert_eq!(submitted_material_indices(&whole).total(), 12);
            }
        }
    }

    #[test]
    fn opaque_world_eligibility_retains_every_transition_and_special_path() {
        for packed in [false, true] {
            let mut chunk = ranged_chunk(true, Vec::new());
            chunk.packed_vertices = packed;
            for calibration in [false, true] {
                chunk.has_calibration_landmark = calibration;
                for feedback in [false, true] {
                    chunk.has_fragment_feedback = feedback;
                    for fade in [
                        f32::NAN,
                        f32::NEG_INFINITY,
                        -1.0,
                        -0.5,
                        -0.0,
                        0.0,
                        0.5,
                        f32::from_bits(1.0f32.to_bits() - 1),
                        1.0,
                        f32::from_bits(1.0f32.to_bits() + 1),
                        f32::INFINITY,
                    ] {
                        assert_eq!(
                            opaque_world_eligible(&chunk, fade),
                            fade == 1.0 && !calibration && !feedback
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn opaque_world_batches_preserve_the_original_indirect_command_stream() {
        let chunks: Vec<_> = (0..32)
            .map(|i| {
                let mut chunk = ranged_chunk(
                    i & 1 == 0,
                    vec![
                        render_range(0, DVec3::ZERO, 1.0),
                        render_range(6, DVec3::ZERO, 1.0),
                        render_range(12, DVec3::ZERO, 1.0),
                    ],
                );
                chunk.packed_vertices = i & 2 == 0;
                chunk.has_calibration_landmark = i % 11 == 0;
                chunk.has_fragment_feedback = i % 7 == 0;
                chunk.has_rare_surface = i % 5 == 0;
                chunk.allocation.page = i % 3;
                chunk.allocation.vertex_offset =
                    (i as u64 + 1) * if chunk.packed_vertices { 12 } else { 20 };
                chunk.allocation.index_offset = (i as u64 + 1) * 8;
                chunk
            })
            .collect();
        for mask in 0..256u32 {
            let mut grouped = BTreeMap::new();
            // Unsorted source order makes the existing ordered group policy
            // observable independently of the new contiguous-run builder.
            for i in (0..32).map(|n| (n * 13) % 32) {
                let chunk = &chunks[i];
                let fade = if mask & (1 << (i % 8)) != 0 {
                    1.0
                } else {
                    -0.5
                };
                let opaque = opaque_world_eligible(chunk, fade);
                let key = (
                    chunk.has_calibration_landmark,
                    chunk.has_rare_surface,
                    chunk.packed_vertices,
                    chunk.indices_u16,
                    chunk.allocation.page,
                );
                append_world_draw_ranges(
                    grouped.entry(key).or_insert_with(Vec::new),
                    chunk,
                    i as u32,
                    chunk.has_calibration_landmark,
                    opaque,
                    |range| range.first_index != 6,
                );
            }
            let mut expected_commands = Vec::new();
            let mut expected_flags = Vec::new();
            let mut expected_keys = Vec::new();
            for (&key, draws) in &grouped {
                let (_, _, packed, indices_u16, _) = key;
                let stride = if packed { 12 } else { 20 };
                let index_stride = if indices_u16 { 2 } else { 4 };
                for draw in draws {
                    expected_commands.push(wgpu::util::DrawIndexedIndirectArgs {
                        index_count: draw.index_count,
                        instance_count: 1,
                        first_index: (draw.chunk.allocation.index_offset / index_stride) as u32
                            + draw.first_index,
                        base_vertex: (draw.chunk.allocation.vertex_offset / stride) as i32
                            + draw.base_vertex,
                        first_instance: draw.slot,
                    });
                    expected_flags.push(draw.opaque_pipeline);
                    expected_keys.push(key);
                }
            }
            let expected_opaque: u64 = expected_commands
                .iter()
                .zip(&expected_flags)
                .filter(|(_, opaque)| **opaque)
                .map(|(c, _)| u64::from(c.index_count))
                .sum();
            let (commands, batches, opaque_indices) =
                build_world_indirect_batches(grouped, expected_commands.len(), 20, 12);
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&commands),
                bytemuck::cast_slice::<_, u8>(&expected_commands)
            );
            assert_eq!(opaque_indices, expected_opaque);
            assert!(batches.len() <= commands.len());
            let mut end = 0;
            for (i, batch) in batches.iter().enumerate() {
                assert_eq!(batch.command_start, end);
                end += batch.command_count as usize;
                let key = (
                    batch.landmark,
                    batch.rare_surfaces,
                    batch.packed,
                    batch.indices_u16,
                    batch.page,
                );
                assert!(
                    expected_keys[batch.command_start..end]
                        .iter()
                        .all(|k| *k == key)
                );
                assert!(
                    expected_flags[batch.command_start..end]
                        .iter()
                        .all(|flag| *flag == batch.opaque)
                );
                assert!(!batch.landmark || !batch.opaque);
                if i > 0 {
                    let previous = &batches[i - 1];
                    let previous_key = (
                        previous.landmark,
                        previous.rare_surfaces,
                        previous.packed,
                        previous.indices_u16,
                        previous.page,
                    );
                    if previous_key == key {
                        assert_ne!(previous.opaque, batch.opaque);
                    }
                }
            }
            assert_eq!(end, commands.len());
        }
    }

    #[test]
    fn opaque_world_contiguous_runs_do_not_reorder_alternating_coverage() {
        let chunk = ranged_chunk(false, Vec::new());
        for flags in [
            vec![],
            vec![true],
            vec![false; 8],
            vec![true; 8],
            vec![true, false, true, false, true, false, true, false],
            vec![true, true, false, false, true],
        ] {
            let mut draws = Vec::new();
            for (slot, opaque) in flags.iter().copied().enumerate() {
                append_world_draw_ranges(&mut draws, &chunk, slot as u32, false, opaque, |_| true);
            }
            let mut grouped = BTreeMap::new();
            grouped.insert((false, false, true, false, 0), draws);
            let (commands, batches, opaque_indices) =
                build_world_indirect_batches(grouped, flags.len(), 20, 12);
            assert_eq!(
                commands
                    .iter()
                    .map(|c| c.first_instance)
                    .collect::<Vec<_>>(),
                (0..flags.len() as u32).collect::<Vec<_>>()
            );
            assert_eq!(
                batches.len(),
                flags
                    .iter()
                    .enumerate()
                    .filter(|(i, flag)| *i == 0 || flags[*i - 1] != **flag)
                    .count()
            );
            assert_eq!(
                opaque_indices,
                24 * flags.iter().filter(|flag| **flag).count() as u64
            );
        }
    }

    fn render_range(first_index: u32, center: DVec3, radius_m: f64) -> RenderIndexRange {
        RenderIndexRange {
            first_index,
            index_count: 6,
            bounds: crate::prepared_render::RenderBounds {
                center_ecef: center.to_array(),
                radius_m,
            },
        }
    }
    #[test]
    fn render_ranges_cull_only_rejected_spans_and_share_the_parent_model() {
        let projection =
            glam::dcamera::rh::proj::directx::perspective_infinite_reverse(1.0, 1.6, 0.05);
        let planes = frustum_planes(&projection);
        let camera = DVec3::new(6_378_137.0, 100.0, -70.0);
        for indices_u16 in [false, true] {
            let chunk = ranged_chunk(
                indices_u16,
                vec![
                    render_range(0, camera + DVec3::new(0.0, 0.0, -100.0), 2.0),
                    render_range(6, camera + DVec3::new(1000.0, 0.0, -100.0), 2.0),
                    render_range(12, camera + DVec3::new(10.0, 0.0, -100.0), 2.0),
                    // A large triangle may cross the grouping cut: its complete
                    // bound keeps it even when its center is outside the frustum.
                    render_range(18, camera + DVec3::new(1000.0, 0.0, -100.0), 1100.0),
                ],
            );
            let mut draws = Vec::new();
            append_world_draw_ranges(&mut draws, &chunk, 31, false, false, |range| {
                sphere_visible(
                    &planes,
                    DVec3::from_array(range.bounds.center_ecef) - camera,
                    range.bounds.radius_m,
                )
            });
            assert_eq!(
                draws
                    .iter()
                    .map(|draw| (draw.first_index, draw.index_count))
                    .collect::<Vec<_>>(),
                [(0, 6), (12, 12)]
            );
            assert!(draws.iter().all(|draw| draw.slot == 31
                && draw.base_vertex == 0
                && std::ptr::eq(draw.chunk, &chunk)));
            assert_eq!(draws.iter().map(|draw| draw.index_count).sum::<u32>(), 18);
            let mut all = Vec::new();
            append_world_draw_ranges(&mut all, &chunk, 31, false, false, |_| true);
            assert_eq!(all.len(), 1);
            assert_eq!((all[0].first_index, all[0].index_count), (0, 24));
            let mut none = Vec::new();
            append_world_draw_ranges(&mut none, &chunk, 31, false, false, |_| false);
            assert!(none.is_empty());
        }
    }
    #[test]
    fn render_ranges_keep_unsplit_calibration_batches_and_never_merge_parent_slots() {
        let mut legacy = ranged_chunk(false, Vec::new());
        legacy.has_calibration_landmark = true;
        let split = ranged_chunk(true, vec![render_range(0, DVec3::ZERO, 1.0)]);
        let mut draws = Vec::new();
        append_world_draw_ranges(&mut draws, &legacy, 7, true, false, |_| {
            panic!("unsplit bounds were already admitted")
        });
        append_world_draw_ranges(&mut draws, &split, 8, false, false, |_| true);
        assert_eq!(draws.len(), 2);
        assert_eq!(
            (draws[0].slot, draws[0].first_index, draws[0].index_count),
            (7, 0, 24)
        );
        assert!(draws[0].landmark_pipeline);
        assert_eq!(
            (draws[1].slot, draws[1].first_index, draws[1].index_count),
            (8, 0, 6)
        );
        assert!(!draws[1].landmark_pipeline);
    }
    #[test]
    fn render_ranges_shadow_culling_uses_each_actual_layer_without_changing_parent_identity() {
        let layers = separated_shadow_cascades();
        let chunk = ranged_chunk(
            false,
            vec![
                render_range(0, layers[0].center_ecef, 1.0),
                render_range(6, layers[1].center_ecef, 1.0),
                render_range(12, DVec3::Y * 20_000.0, 1.0),
                render_range(18, DVec3::X * 5000.0, 5001.0),
            ],
        );
        let mut layers_draws: [Vec<WorldDrawRange<'_>>; 2] = std::array::from_fn(|_| Vec::new());
        for (cascade, draws) in layers_draws.iter_mut().enumerate() {
            append_world_draw_ranges(draws, &chunk, 4, false, false, |range| {
                sun_shadow_cascade_mask_for_updates(
                    &layers,
                    DVec3::from_array(range.bounds.center_ecef),
                    range.bounds.radius_m,
                    1 << cascade,
                ) != 0
            });
        }
        assert_eq!(
            layers_draws[0]
                .iter()
                .map(|draw| (draw.first_index, draw.index_count))
                .collect::<Vec<_>>(),
            [(0, 6), (18, 6)]
        );
        assert_eq!(
            layers_draws[1]
                .iter()
                .map(|draw| (draw.first_index, draw.index_count))
                .collect::<Vec<_>>(),
            [(6, 6), (18, 6)]
        );
        assert!(layers_draws.iter().flatten().all(|draw| draw.slot == 4));
        let parent = (shadow_key(3, 7, 0), false);
        let mut cache = shadow_population(&[parent], None);
        assert!(population_matches(&mut cache, &[parent], 1, None));
        assert!(!population_matches(
            &mut cache,
            &[(shadow_key(3, 8, 0), false)],
            2,
            None
        ));
    }
    #[test]
    fn render_ranges_cover_camera_relative_roundoff_at_large_distances() {
        let range = render_range(0, DVec3::new(6_378_137.125, 12.34, -98.76), 0.05);
        let center = DVec3::from_array(range.bounds.center_ecef);
        let mut unpadded_failures = 0;
        for camera in [
            center + DVec3::new(10.0, 20.0, -30.0),
            DVec3::new(-6_378_137.0, 5_432_198.0, -3_456_789.0),
            DVec3::new(-1.0e9, 2.0e9, -3.0e9),
        ] {
            let (actual_center, radius) = render_range_bounds(&range, camera);
            assert_eq!(
                (actual_center, radius),
                camera_relative_render_bounds(center, range.bounds.radius_m, camera)
            );
            assert_eq!(actual_center, center);
            assert!(radius >= range.bounds.radius_m);
            for direction in [
                DVec3::ZERO,
                DVec3::X,
                -DVec3::X,
                DVec3::Y,
                -DVec3::Y,
                DVec3::Z,
                -DVec3::Z,
            ] {
                let relative_vertex = center + direction * range.bounds.radius_m - camera;
                let projected_f32 = relative_vertex.as_vec3().as_dvec3();
                let distance = projected_f32.distance(center - camera);
                unpadded_failures += usize::from(distance > range.bounds.radius_m);
                assert!(
                    distance <= radius,
                    "camera-relative conversion escaped the tight range"
                );
            }
        }
        assert!(
            unpadded_failures > 0,
            "fixture must exercise actual f32 translation loss"
        );
    }

    #[test]
    fn tight_parent_bounds_cover_camera_relative_models_and_landmark_union() {
        use crate::prepared_render::{pack_surface_cell, test_surface};
        use vearth_core::{cell::ChunkAddr, cubesphere::Face, frame::ChunkFrame};
        for face in 0..6 {
            for level in [0, 17, 30] {
                let cell = CellId {
                    face: Face(face),
                    level,
                    i: (1 << level) - 1,
                    j: 0,
                };
                for large in [false, true] {
                    let mut source = test_surface(cell);
                    source.batches[0].addr.k = -3;
                    let factor = if large { 1.0e6 } else { 1.0 };
                    for (i, vertex) in source.batches[0].mesh.vertices.iter_mut().enumerate() {
                        vertex.pos = [
                            (i as f32 * 0.0625 - 0.25) * factor,
                            (i as f32 * 0.03125 + 0.5) * factor,
                            (i as f32 * 0.125 - 0.375) * factor,
                        ];
                        if i % 2 == 0 {
                            vertex.face_ao |= 128 << 5;
                        }
                    }
                    let vertices = source.batches[0].mesh.vertices.clone();
                    let prepared = pack_surface_cell(source).unwrap();
                    let chunk = &prepared.chunks[0];
                    let frame = ChunkFrame::of(ChunkAddr { cell, k: -3 });
                    let model = frame.model_matrix();
                    let delta = DMat4::from_translation(DVec3::new(100.25, -200.125, 300.5))
                        * DMat4::from_rotation_z(0.3);
                    for landmark in [None, Some(delta)] {
                        let (center, radius) = landmark_aware_chunk_bounds(
                            DVec3::from_array(chunk.bounds.center_ecef),
                            chunk.bounds.radius_m,
                            chunk.has_calibration_landmark,
                            landmark,
                        );
                        for camera in [
                            frame.origin + DVec3::new(10.0, 20.0, -30.0),
                            -frame.origin,
                            DVec3::new(-1.0e9, 2.0e9, -3.0e9),
                        ] {
                            let projection_padding =
                                camera_relative_render_padding(center, radius, camera);
                            let (_, radius) = camera_relative_render_bounds(center, radius, camera);
                            for vertex in &vertices {
                                let mut actual_model = if vertex.face_ao & (128 << 5) != 0 {
                                    landmark.map_or(model, |delta| delta * model)
                                } else {
                                    model
                                };
                                actual_model.w_axis -= camera.extend(0.0);
                                let actual = glam::Mat4::from_cols_array(
                                    &actual_model.to_cols_array().map(|value| value as f32),
                                )
                                .transform_point3(glam::Vec3::from_array(vertex.pos))
                                .as_dvec3();
                                assert!(
                                    actual.distance(center - camera) <= radius,
                                    "{cell:?} large={large}: rounded model escaped parent bound"
                                );
                                if landmark.is_none() {
                                    let bounds = RenderBoxGpu::new(&frame, chunk.local_bounds);
                                    for i in 0..32 {
                                        let normal = DVec3::new(
                                            (i as f64 * 0.7).sin(),
                                            (i as f64 * 0.3).cos(),
                                            (i as f64 * 1.1).sin(),
                                        )
                                        .normalize();
                                        assert!(
                                            normal.dot(actual - (center - camera)).abs()
                                                <= bounds.support(normal, projection_padding),
                                            "{cell:?} large={large}: rounded vertex escaped box support"
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn render_box_rejects_flat_offscreen_geometry_inside_its_sphere() {
        use crate::prepared_render::{pack_surface_cell, test_surface};
        let cell = CellId {
            face: vearth_core::cubesphere::Face(4),
            level: 13,
            i: 7530,
            j: 4449,
        };
        let mut source = test_surface(cell);
        source.batches[0].mesh.vertices = [
            [0.0, 0.0, 0.0],
            [32.0, 0.0, 0.0],
            [32.0, 0.0, 32.0],
            [0.0, 0.0, 32.0],
        ]
        .into_iter()
        .map(|pos| SurfaceVertex {
            pos,
            face_ao: 2,
            rgba: [30, 40, 50, 255],
        })
        .collect();
        source.batches[0].mesh.indices = vec![0, 1, 2, 0, 2, 3];
        let vertices = source.batches[0].mesh.vertices.clone();
        let chunk = &pack_surface_cell(source).unwrap().chunks[0];
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let camera = crate::camera::Camera {
            lat: 52.369_f64.to_radians(),
            lon: 4.908_f64.to_radians(),
            alt_m: 250.0,
            heading: 90_f64.to_radians(),
            pitch: 55_f64.to_radians(),
            fov_y: 55_f64.to_radians(),
            aspect: 1600.0 / 909.0,
        };
        let projection = glam::dcamera::rh::proj::directx::perspective_infinite_reverse(
            camera.fov_y,
            camera.aspect,
            0.05,
        );
        let planes = frustum_planes(&(projection * camera.view_rotation()));
        let center = DVec3::from_array(chunk.bounds.center_ecef);
        let relative = center - camera.eye_ecef();
        let padding =
            camera_relative_render_padding(center, chunk.bounds.radius_m, camera.eye_ecef());
        let bounds = RenderBoxGpu::new(&frame, chunk.local_bounds);
        assert!(sphere_visible(
            &planes,
            relative,
            chunk.bounds.radius_m + padding
        ));
        assert!(!bounds.visible(&planes, relative, padding));
        assert!(planes.iter().any(|(normal, offset)| {
            vertices.iter().all(|vertex| {
                normal.dot(
                    frame.local_to_ecef(DVec3::from_array(vertex.pos.map(f64::from)))
                        - camera.eye_ecef(),
                ) + offset
                    < -400.0
            })
        }));
        // Moving one source vertex into view must expand the source box;
        // the sphere regression cannot justify dropping an entire owner.
        let inside = camera.eye_ecef() + camera.forward_ecef() * 500.0;
        let mut source = test_surface(cell);
        source.batches[0].mesh.vertices[0].pos = frame.ecef_to_local(inside).as_vec3().to_array();
        let chunk = &pack_surface_cell(source).unwrap().chunks[0];
        let center = DVec3::from_array(chunk.bounds.center_ecef);
        let padding =
            camera_relative_render_padding(center, chunk.bounds.radius_m, camera.eye_ecef());
        assert!(RenderBoxGpu::new(&frame, chunk.local_bounds).visible(
            &planes,
            center - camera.eye_ecef(),
            padding
        ));
    }

    #[test]
    fn render_box_shadow_masks_use_exact_slabs_and_retained_projection() {
        let rendered = separated_shadow_cascades();
        let bounds = RenderBoxGpu {
            half_axes: [DVec3::X * 80.0, DVec3::Y * 0.25, DVec3::Z * 80.0],
            padding_m: 0.01,
        };
        let center = DVec3::Y * 150.0;
        let radius = 114.0;
        assert_eq!(
            sun_shadow_cascade_mask_for_updates(&rendered, center, radius, 3),
            1
        );
        assert_eq!(
            sun_shadow_box_mask_for_updates(&rendered, center, radius, Some(&bounds), 0.01, 3),
            0
        );
        let mut desired = rendered;
        desired[0].center_ecef.y += 100.0;
        assert_eq!(
            sun_shadow_box_mask_for_updates(&desired, center, radius, Some(&bounds), 0.01, 3),
            1
        );
        for mask in 0..4 {
            assert_eq!(
                sun_shadow_box_mask_for_updates(&rendered, center, radius, None, 0.01, mask),
                sun_shadow_cascade_mask_for_updates(&rendered, center, radius, mask)
            );
        }
        let bounds = RenderBoxGpu {
            half_axes: [
                DVec3::new(80.0, 20.0, 0.0),
                DVec3::new(-1.0, 4.0, 2.0),
                DVec3::new(8.0, 0.0, 70.0),
            ],
            padding_m: 0.01,
        };
        for center in [
            DVec3::ZERO,
            DVec3::new(100.0, 100.0, -500.0),
            DVec3::new(-100.0, -100.0, 100.0),
            DVec3::new(10_100.0, 100.0, 100.0),
        ] {
            let mask =
                sun_shadow_box_mask_for_updates(&rendered, center, 200.0, Some(&bounds), 0.01, 3);
            for x in [-1.0, 1.0] {
                for y in [-1.0, 1.0] {
                    for z in [-1.0, 1.0] {
                        let point = center
                            + bounds.half_axes[0] * x
                            + bounds.half_axes[1] * y
                            + bounds.half_axes[2] * z;
                        let point_mask =
                            sun_shadow_cascade_mask_for_updates(&rendered, point, 0.0, 3);
                        assert_eq!(mask & point_mask, point_mask);
                    }
                }
            }
        }
    }

    #[test]
    fn render_ranges_expand_command_capacity_without_spending_parent_admission_slots() {
        let count = MAX_RENDER_INDEX_RANGES;
        let chunk = ranged_chunk(
            false,
            (0..count)
                .map(|i| render_range(i as u32 * 6, DVec3::ZERO, 1.0))
                .collect(),
        );
        let admitted = admit_draws(MAX_DRAWS, 5);
        assert_eq!(admitted.world, MAX_DRAWS - 5);
        assert_eq!(admitted.dropped_world, 5);
        assert_eq!(admitted.player, 5);
        let mut draws = Vec::new();
        for slot in 0..admitted.world {
            append_world_draw_ranges(&mut draws, &chunk, slot as u32, false, false, |range| {
                range.first_index / 6 % 2 == 0
            });
        }
        assert_eq!(draws.len(), admitted.world * count.div_ceil(2));
        assert_eq!(draws.len(), admitted.world * MAX_COMMANDS_PER_PARENT);
        assert!(draws.len() <= MAX_WORLD_COMMANDS);
        assert!(draws.len() * SUN_SHADOW_CASCADES <= MAX_SHADOW_COMMANDS);
        assert_eq!(draws.last().unwrap().slot, admitted.world as u32 - 1);
        assert_eq!(
            sun_shadow_capacity(MAX_SHADOW_CHUNKS),
            (MAX_SHADOW_CHUNKS, 0)
        );
        assert_eq!(MAX_SHADOW_MODELS, MAX_DRAWS + 1);
    }

    #[test]
    fn render_ranges_command_bound_holds_for_every_small_visibility_pattern() {
        for count in 1..=12 {
            let chunk = ranged_chunk(
                false,
                (0..count)
                    .map(|i| render_range(i * 6, DVec3::ZERO, 1.0))
                    .collect(),
            );
            let mut maximum = 0;
            for mask in 0..(1u32 << count) {
                let mut draws = Vec::new();
                append_world_draw_ranges(&mut draws, &chunk, 7, false, false, |range| {
                    mask & (1 << (range.first_index / 6)) != 0
                });
                maximum = maximum.max(draws.len());
                assert!(draws.len() <= (count as usize).div_ceil(2));
                assert!(draws.iter().all(|draw| draw.slot == 7));
            }
            assert_eq!(maximum, (count as usize).div_ceil(2));
        }
    }

    #[test]
    fn shadow_subset_keeps_world_geometry_and_uses_the_actual_cascade_resolution() {
        let mut chunk = ranged_chunk(true, vec![render_range(0, DVec3::ZERO, 1.0)]);
        chunk.shadow = Some(ShadowIndexGpu {
            first_index: 6,
            index_count: 3,
            ranges: Vec::new(),
            max_deviation_m: 0.03125,
        });
        let mut layer = separated_shadow_cascades()[0];
        layer.radius_m = 96.0;
        assert!(shadow_proxy_resolved(layer.radius_m, 0.03125));
        let mut world = Vec::new();
        append_world_draw_ranges(&mut world, &chunk, 7, false, false, |_| true);
        assert_eq!((world[0].first_index, world[0].index_count), (0, 6));
        let mut far = Vec::new();
        append_shadow_draw_ranges(&mut far, &chunk, 7, &layer, |_| true);
        assert_eq!((far[0].first_index, far[0].index_count), (6, 3));
        // A future desired projection cannot change an already rendered layer's
        // proxy selection. Its actual radius remains the deciding scale.
        layer.radius_m -= 0.001;
        let mut near = Vec::new();
        append_shadow_draw_ranges(&mut near, &chunk, 7, &layer, |_| true);
        assert_eq!((near[0].first_index, near[0].index_count), (0, 6));
        assert!(
            world
                .iter()
                .chain(&far)
                .chain(&near)
                .all(|draw| draw.slot == 7
                    && draw.base_vertex == 0
                    && std::ptr::eq(draw.chunk, &chunk))
        );
    }

    #[test]
    fn shadow_subset_ranges_share_vertices_and_apply_the_index_stream_offset_once() {
        for indices_u16 in [true, false] {
            let stride = if indices_u16 { 2 } else { 4 };
            let mut chunk = ranged_chunk(indices_u16, Vec::new());
            chunk.index_count = 27;
            let (byte_offset, total) = index_stream_layout(27 * stride, 24 * stride);
            chunk.index_bytes = total;
            chunk.allocation.index_bytes = total;
            chunk.shadow = Some(ShadowIndexGpu {
                first_index: (byte_offset / stride) as u32,
                index_count: 24,
                ranges: (0..4)
                    .map(|i| render_range(i * 6, DVec3::ZERO, 1.0))
                    .collect(),
                max_deviation_m: 0.025,
            });
            let mut draws = Vec::new();
            append_shadow_draw_ranges(
                &mut draws,
                &chunk,
                19,
                &separated_shadow_cascades()[0],
                |range| range.first_index != 6,
            );
            let stream_index = (byte_offset / stride) as u32;
            assert_eq!(
                draws
                    .iter()
                    .map(|draw| (draw.first_index, draw.index_count))
                    .collect::<Vec<_>>(),
                [(stream_index, 6), (stream_index + 12, 12)]
            );
            let base_vertex =
                chunk.allocation.vertex_offset / std::mem::size_of::<PackedSurfaceVertex>() as u64;
            for draw in &draws {
                let page_first_index =
                    chunk.allocation.index_offset / stride + u64::from(draw.first_index);
                assert!(page_first_index * stride >= chunk.allocation.index_offset + byte_offset);
                assert!(u64::from(draw.first_index + draw.index_count) * stride <= total);
                assert_eq!(base_vertex + draw.base_vertex as u64, base_vertex);
                assert_eq!(draw.slot, 19);
            }
            let footprint = MeshFootprint::from_chunks(std::slice::from_ref(&chunk));
            assert_eq!(footprint.index_bytes, total);
            assert_eq!(footprint.vertex_bytes, chunk.vertex_bytes);
            assert_eq!(footprint.chunks, 1);
        }
    }

    #[test]
    fn shadow_subset_upload_alignment_and_release_cover_both_streams() {
        for stride in [2, 4] {
            for world_count in [3, 6, 9, 12] {
                for shadow_count in [0, 3, 6] {
                    let world_bytes = world_count * stride;
                    let shadow_bytes = shadow_count * stride;
                    let (offset, total) = index_stream_layout(world_bytes, shadow_bytes);
                    assert_eq!(offset % wgpu::COPY_BUFFER_ALIGNMENT, 0);
                    assert!(offset >= world_bytes);
                    if shadow_bytes == 0 {
                        assert_eq!(total, world_bytes);
                    } else {
                        assert_eq!(total, offset + shadow_bytes);
                        assert!(align_up(world_bytes, 4) <= offset);
                    }
                    let mut allocator = RangeAllocator::new(1024);
                    let first = allocator.allocate(total, stride).unwrap();
                    let next = allocator.allocate(12, 4).unwrap();
                    assert!(next >= first + align_up(total, 4));
                    if shadow_bytes != 0 {
                        assert!(first + offset + align_up(shadow_bytes, 4) <= next);
                    }
                    allocator.free(first, total);
                    allocator.free(next, 12);
                    assert!(
                        allocator.is_empty(),
                        "both index streams must release with their parent"
                    );
                }
            }
        }
    }

    #[test]
    fn shadow_subset_empty_is_distinct_from_absent() {
        let mut chunk = ranged_chunk(false, Vec::new());
        let layer = separated_shadow_cascades()[0];
        let mut full = Vec::new();
        append_shadow_draw_ranges(&mut full, &chunk, 4, &layer, |_| true);
        assert_eq!(full[0].index_count, 24);
        drop(full);
        chunk.shadow = Some(ShadowIndexGpu {
            first_index: 24,
            index_count: 0,
            ranges: Vec::new(),
            max_deviation_m: 0.025,
        });
        let mut empty = Vec::new();
        append_shadow_draw_ranges(&mut empty, &chunk, 4, &layer, |_| true);
        assert!(empty.is_empty());
        let mut world = Vec::new();
        append_world_draw_ranges(&mut world, &chunk, 4, false, false, |_| true);
        assert_eq!(world[0].index_count, 24);
    }

    #[test]
    fn building_detail_hysteresis_prevents_threshold_chatter() {
        assert!(!building_detail_visible(71.0, 72.0, false));
        assert!(building_detail_visible(72.0, 72.0, false));
        assert!(building_detail_visible(66.0, 72.0, true));
        assert!(!building_detail_visible(62.0, 72.0, true));
    }

    #[test]
    fn frustum_accepts_ahead_rejects_behind() {
        let proj = glam::dcamera::rh::proj::directx::perspective_infinite_reverse(1.0, 1.6, 0.05);
        // Camera at origin looking down -Z (glam view identity).
        let planes = frustum_planes(&proj);
        assert!(sphere_visible(&planes, DVec3::new(0.0, 0.0, -100.0), 1.0));
        assert!(!sphere_visible(&planes, DVec3::new(0.0, 0.0, 100.0), 1.0));
        assert!(!sphere_visible(
            &planes,
            DVec3::new(1000.0, 0.0, -100.0),
            1.0
        ));
        // Huge sphere enclosing the camera is visible.
        assert!(sphere_visible(
            &planes,
            DVec3::new(0.0, 0.0, 500.0),
            10_000.0
        ));
        // No NaNs anywhere.
        for (n, d) in &planes {
            assert!(n.is_finite() && d.is_finite());
        }
    }
}

#[cfg(test)]
#[path = "surface_normal_tests.rs"]
mod surface_normal_tests;

#[cfg(test)]
#[path = "opaque_shader_tests.rs"]
mod opaque_shader_tests;

#[cfg(test)]
#[path = "world_interface_tests.rs"]
mod world_interface_tests;

#[cfg(test)]
#[path = "city_lighting_tests.rs"]
mod city_lighting_tests;
