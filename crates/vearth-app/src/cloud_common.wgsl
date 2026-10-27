// Shared world-space cloud body and lighting for the visible sky and its
// cached reflection. Callers own weather sampling, ray budgets and bindings.
struct CloudContext {
    ground_r: f32,
    base_m: f32,
    top_m: f32,
    coverage: f32,
    convection: f32,
    quality: f32,
    seed: f32,
    // Accumulated wind advection in ECEF metres, wrapped by the renderer on a
    // period every tiling below divides.
    drift: vec3<f32>,
    // Density multiplier for the weather's water content; 1 in fair weather.
    water: f32,
};

struct CloudSample {
    density: f32,
    // Height within the sampled cell, 0 at its base and 1 at its crown.
    height_fraction: f32,
};

// Metres of path per unit optical density. Sunlight reaching a sample inside
// cloud arrives mostly by multiple scattering, so the light path uses a longer
// scale than the view ray; single-scattering extinction would over-darken it.
const CLOUD_VIEW_EXTINCTION_M: f32 = 440.0;
const CLOUD_LIGHT_EXTINCTION_M: f32 = 1320.0;
// Each tiling repeats every four scales. Body (20.8 km), detail (1.3 km)
// and cluster (166.4 km) tiles all divide the renderer's 166.4 km drift
// period, so wrapping the drift shifts every pattern by whole tiles. The
// scales are fixed: the coordinates are planet-centred, so a scale that
// followed the weather would slide the whole field as the weather changed.
const CLOUD_BODY_SCALE_M: f32 = 5200.0;
// The shape volume's A channel is edge detail (about 20 m texels).
// Displacement is centred on the channel mean, so light probes, skipped and
// unresolved detail, and every quality tier see the same average surface.
// gfx.rs tests check the mean against the volume.
const CLOUD_DETAIL_SCALE_M: f32 = 325.0;
const CLOUD_DETAIL_MEAN: f32 = 0.476;
// Body-field displacement per unit of centred detail at the stratiform scale.
const CLOUD_DETAIL_RELIEF: f32 = 0.3;
// Clustering reads the B channel from mip 2 (about 10 km texels, a
// cache-resident 16^3 level). Its median there maps to the neutral 0.5;
// gfx.rs tests check the median against the generated volume.
const CLOUD_CLUSTER_SCALE_M: f32 = 41600.0;
const CLOUD_CLUSTER_LOD: f32 = 2.0;
const CLOUD_CLUSTER_MEDIAN: f32 = 0.427;

/// Water-content multiplier from the weather's cloud optical depth, which
/// already includes rain. Deep, wet decks block far more light, so their
/// bases turn grey through extinction rather than an artistic tint.
fn cloud_common_water(optical_depth: f32) -> f32 {
    return mix(1.0, 3.0, smoothstep(2.5, 9.0, optical_depth));
}

fn cloud_common_wind(up: vec3<f32>, wind_enu: vec3<f32>) -> vec3<f32> {
    var east = cross(vec3<f32>(0.0, 0.0, 1.0), up);
    if dot(east, east) < 1e-5 {
        east = cross(vec3<f32>(1.0, 0.0, 0.0), up);
    }
    east = normalize(east);
    let north = normalize(cross(up, east));
    return east * wind_enu.x + north * wind_enu.y + up * wind_enu.z;
}

/// The sample carried back along the accumulated wind drift, which history
/// reprojection advances by the same per-frame step.
fn cloud_common_drifted(relative_ecef: vec3<f32>, context: CloudContext) -> vec3<f32> {
    return relative_ecef - context.drift;
}

fn cloud_common_shape(drifted: vec3<f32>, context: CloudContext,
        scale_m: f32, lod: f32, shape_tex: texture_3d<f32>, shape_sampler: sampler) -> vec4<f32> {
    // Four broad cells span the tiled volume.
    let coordinates = (drifted / scale_m
        + vec3<f32>(context.seed * 19.1, context.seed * 7.7, context.seed * 13.3)) * 0.25;
    return textureSampleLevel(shape_tex, shape_sampler, coordinates, lod);
}

/// Horizontal clustering of cloud cells, 0 in clear patches and 1 in dense
/// clusters. It reads only the drifting column, so a cluster spans the whole
/// layer. Footprints that cannot resolve it see the neutral value.
fn cloud_common_cluster(drifted: vec3<f32>, context: CloudContext, footprint_m: f32,
        shape_tex: texture_3d<f32>, shape_sampler: sampler) -> f32 {
    let column = normalize(drifted) * context.ground_r + vec3<f32>(91000.0, -37000.0, 53000.0);
    let field = cloud_common_shape(column, context, CLOUD_CLUSTER_SCALE_M, CLOUD_CLUSTER_LOD,
        shape_tex, shape_sampler).b;
    let resolved = 1.0 - smoothstep(0.1, 0.5, footprint_m / (CLOUD_CLUSTER_SCALE_M * 0.5));
    return mix(0.5, smoothstep(CLOUD_CLUSTER_MEDIAN - 0.2, CLOUD_CLUSTER_MEDIAN + 0.2, field),
        resolved);
}

/// Body-field values exceeded by each tenth of the shape volume. Full
/// coverage closes every gap. gfx.rs tests check them against the volume.
const CLOUD_COVERAGE_DECILES = array<f32, 11>(0.800, 0.614, 0.528, 0.467, 0.421, 0.381,
    0.343, 0.305, 0.263, 0.209, 0.000);

/// Piecewise-linear through the deciles using constant indices only, so no
/// per-sample array is materialised.
fn cloud_common_coverage_threshold(coverage: f32) -> f32 {
    let p = clamp(coverage, 0.0, 1.0) * 10.0;
    let d = CLOUD_COVERAGE_DECILES;
    return d[0] + (d[1] - d[0]) * clamp(p, 0.0, 1.0)
        + (d[2] - d[1]) * clamp(p - 1.0, 0.0, 1.0)
        + (d[3] - d[2]) * clamp(p - 2.0, 0.0, 1.0)
        + (d[4] - d[3]) * clamp(p - 3.0, 0.0, 1.0)
        + (d[5] - d[4]) * clamp(p - 4.0, 0.0, 1.0)
        + (d[6] - d[5]) * clamp(p - 5.0, 0.0, 1.0)
        + (d[7] - d[6]) * clamp(p - 6.0, 0.0, 1.0)
        + (d[8] - d[7]) * clamp(p - 7.0, 0.0, 1.0)
        + (d[9] - d[8]) * clamp(p - 8.0, 0.0, 1.0)
        + (d[10] - d[9]) * clamp(p - 9.0, 0.0, 1.0);
}

fn cloud_common_sample_filtered(relative_ecef: vec3<f32>, context: CloudContext,
        footprint_m: f32, shape_tex: texture_3d<f32>, shape_sampler: sampler) -> CloudSample {
    let height = length(relative_ecef) - context.ground_r;
    if context.coverage <= 0.001 || height <= context.base_m || height >= context.top_m {
        return CloudSample(0.0, 0.0);
    }
    let cluster = cloud_common_cluster(cloud_common_drifted(relative_ecef, context), context,
        footprint_m, shape_tex, shape_sampler);
    return cloud_common_sample_clustered(relative_ecef, context, footprint_m, cluster,
        shape_tex, shape_sampler);
}

/// Density with the cluster value supplied by the caller. Clusters vary over
/// tens of kilometres, so marches may interpolate them along a ray and let
/// nearby light probes reuse the view sample's value.
fn cloud_common_sample_clustered(relative_ecef: vec3<f32>, context: CloudContext,
        footprint_m: f32, cluster: f32, shape_tex: texture_3d<f32>,
        shape_sampler: sampler) -> CloudSample {
    let empty = CloudSample(0.0, 0.0);
    let height = length(relative_ecef) - context.ground_r;
    if context.coverage <= 0.001 || height <= context.base_m || height >= context.top_m {
        return empty;
    }
    let convection = clamp(context.convection, 0.0, 1.0);
    let thickness = max(context.top_m - context.base_m, 1.0);
    let h = (height - context.base_m) / thickness;
    let drifted = cloud_common_drifted(relative_ecef, context);
    // Clear patches also build lower; air above every possible crown is
    // skipped unread.
    let cluster_top = mix(0.82, 1.0, cluster);
    if h >= cluster_top {
        return empty;
    }
    // Clusters raise local coverage and clear patches lower it. The power
    // keeps overcast closed and stays monotonic in the weather coverage.
    let coverage = pow(clamp(context.coverage, 0.0, 1.0), exp2(1.0 - 2.0 * cluster));
    if coverage <= 0.002 {
        return empty;
    }
    let scale_m = CLOUD_BODY_SCALE_M;
    let side = f32(textureDimensions(shape_tex).x);
    let lod = clamp(log2(max(footprint_m / (scale_m * 4.0 / side), 1.0)),
        0.0, f32(textureNumLevels(shape_tex) - 1u));
    let shape = cloud_common_shape(drifted, context, scale_m, lod, shape_tex, shape_sampler);
    // Each Worley billow cell sets its own crown. Stratiform cells keep
    // nearly level tops; convective cells build heaps of varied height.
    let crown_spread = mix(0.2, 0.55, smoothstep(0.08, 0.7, convection));
    let column_top = cluster_top
        * (1.0 - crown_spread * (1.0 - smoothstep(0.30, 0.66, shape.g)));
    let local_height = h / column_top;
    if local_height >= 1.0 {
        return empty;
    }
    // A level condensation base, crisper for convective cloud, and a circular
    // crown profile. Scaling the field before the coverage threshold turns
    // the crown profile into rounded tops.
    let base_ramp_m = mix(110.0, 45.0, smoothstep(0.1, 0.6, convection));
    let base_profile = smoothstep(0.0, base_ramp_m, height - context.base_m);
    let crown = max(local_height - 0.45, 0.0) / 0.55;
    let crown_profile = sqrt(max(1.0 - crown * crown, 0.0));
    var field = (shape.r * 0.74 + shape.g * 0.26) * base_profile * crown_profile;
    // Preserve fractional occupancy when minification removes unresolved
    // density variation; a sharp threshold of the averaged noise would erase
    // broken cloud fields at the first coarse mip. The edge is also no
    // narrower than the footprint, up to the same bound (the body field
    // changes by about 2.1 per body scale), so sparse shadow and far-field
    // probes cannot alias it.
    // Centring the ramp's visible part on the coverage contour must still
    // leave a zero field empty, or overcast would lose its level base.
    let ramp = max(0.05 + min(lod * 0.04, 0.20), min(footprint_m * 2.1 / scale_m, 0.25));
    let threshold = max(cloud_common_coverage_threshold(coverage) - 0.3 * ramp, 0.01);
    // Edge detail moves the surface by up to about 150 m rather than
    // thinning a soft rim: inverse Worley pulls cell centres in under the
    // base (wisps) and pushes them out above it (rounded lobes). Clamping to
    // the channel's central range bounds the band that fetches.
    let relief = CLOUD_DETAIL_RELIEF;
    let reach = relief * 0.22;
    if field + reach <= threshold {
        return empty;
    }
    if field - reach < threshold + ramp {
        var detail = CLOUD_DETAIL_MEAN;
        let detail_lod = log2(max(footprint_m / (CLOUD_DETAIL_SCALE_M * 4.0 / side), 1.0));
        // Lobes are sub-pixel once the footprint passes about 150 m.
        if context.quality > 0.5 && detail_lod < 3.0 {
            let fetched = cloud_common_shape(drifted + vec3<f32>(1730.0, -910.0, 540.0),
                context, CLOUD_DETAIL_SCALE_M, detail_lod, shape_tex, shape_sampler).a;
            detail = mix(fetched, CLOUD_DETAIL_MEAN, smoothstep(2.0, 3.0, detail_lod));
        }
        let billowy = smoothstep(0.08, 0.25, local_height) * 2.0 - 1.0;
        field += clamp(detail - CLOUD_DETAIL_MEAN, -0.22, 0.22) * billowy * relief;
    }
    let density = clamp((field - threshold) / ramp, 0.0, 1.0);
    return CloudSample(density * smoothstep(0.001, 0.045, context.coverage) * context.water,
        local_height);
}

fn cloud_common_sample(relative_ecef: vec3<f32>, context: CloudContext,
        shape_tex: texture_3d<f32>, shape_sampler: sampler) -> CloudSample {
    return cloud_common_sample_filtered(relative_ecef, context, 0.0, shape_tex, shape_sampler);
}

fn cloud_common_column(direction: vec3<f32>, context: CloudContext,
        shape_tex: texture_3d<f32>, shape_sampler: sampler) -> f32 {
    // One representative height low enough to lie inside every cell type
    // retains the weather, advection, body scale and coverage contour during
    // the aerial representation handoff.
    let height = mix(context.base_m, context.top_m, 0.15);
    return cloud_common_sample(direction * (context.ground_r + height),
        context, shape_tex, shape_sampler).density;
}

fn cloud_common_optical_density(density: f32) -> f32 {
    return density * smoothstep(0.035, 0.42, density);
}

fn cloud_common_light_transmittance(optical_depth_m: f32) -> f32 {
    return exp(-max(optical_depth_m, 0.0) / CLOUD_LIGHT_EXTINCTION_M);
}

fn cloud_common_view_transmittance(optical_depth_m: f32) -> f32 {
    return exp(-max(optical_depth_m, 0.0) / CLOUD_VIEW_EXTINCTION_M);
}

fn cloud_common_extinction(density: f32, distance_m: f32) -> f32 {
    return cloud_common_view_transmittance(
        cloud_common_optical_density(density) * min(distance_m, 900.0));
}

fn cloud_common_near_light_optical_depth(relative_ecef: vec3<f32>, context: CloudContext,
        sun_dir: vec3<f32>, origin_density: f32, cluster: f32,
        shape_tex: texture_3d<f32>, shape_sampler: sampler) -> f32 {
    // A fixed long probe skips shallow crowns and reports full sunlight across
    // their interiors. Keep the first probe within a fraction of layer depth;
    // the visible sky and cached reflection use the same density and distance.
    let distance_m = clamp((context.top_m - context.base_m) * 0.18, 160.0, 650.0);
    var light_context = context;
    light_context.quality = 0.0;
    let density = cloud_common_sample_clustered(relative_ecef + sun_dir * distance_m,
        light_context, 0.0, cluster, shape_tex, shape_sampler).density;
    return cloud_common_optical_density(origin_density) * distance_m * 0.5
        + cloud_common_optical_density(density) * distance_m * 2.5;
}

fn cloud_common_phase(cos_theta: f32, light_visibility: f32,
        segment_transmittance: f32) -> f32 {
    // Transmittance-dependent HG approximates multiple scattering: dense
    // interiors scatter broadly; exposed backlit boundaries form a silver rim.
    let exposed = clamp(light_visibility * segment_transmittance, 0.0, 1.0);
    let asymmetry = mix(-0.15, 0.78, smoothstep(0.08, 0.98, exposed));
    let denominator = max(1.0 + asymmetry * asymmetry
        - 2.0 * asymmetry * clamp(cos_theta, -1.0, 1.0), 0.025);
    let phase = (1.0 - asymmetry * asymmetry)
        / (denominator * sqrt(denominator));
    return min(phase * mix(2.16, 1.0, exposed), 4.5);
}

fn cloud_common_lighting(cos_theta: f32, density: f32, height_fraction: f32,
        light_visibility: f32, segment_transmittance: f32,
        direct_sun: vec3<f32>, ambient: vec3<f32>) -> vec3<f32> {
    let visibility = clamp(light_visibility, 0.0, 1.0);
    let phase = cloud_common_phase(cos_theta, visibility, segment_transmittance);
    let powder = 1.0 - exp(-max(density, 0.0) * 3.5);
    let sky_access = mix(0.38, 1.0, smoothstep(0.02, 0.90, height_fraction));
    let indirect = ambient * sky_access * (0.64 + 0.36 * sqrt(visibility));
    let direct = direct_sun * (visibility * phase * 0.52
        + pow(visibility, 0.45) * powder * 0.28);
    return indirect + direct;
}

fn cloud_common_scene_distance(surface: vec4<f32>) -> f32 {
    // Infinite reverse-Z reconstructs w = 1 / view-space depth. A near-camera
    // epsilon such as 1e-5 silently moves distant geometry onto a 100 km plane
    // and cuts cloud against that invented foreground surface.
    return length(surface.xyz) / max(abs(surface.w), 1e-12);
}

fn cloud_common_depth_visible(opaque_distance: f32, front_distance: f32) -> bool {
    // The volume is clipped against scene depth before marching. This guard
    // rejects a reconstructed neighbor's cloud when foreground geometry lies
    // in front of it, without hiding legitimate cloud over land or water.
    let depth_bias = max(0.75, opaque_distance * 0.0005);
    return front_distance <= 0.0 || opaque_distance + depth_bias >= front_distance;
}

fn cloud_common_history_stability(motion_pixels: f32) -> f32 {
    return 1.0 - smoothstep(0.15, 0.75, motion_pixels);
}

fn cloud_common_history_count(prior_count: f32, stability: f32) -> f32 {
    return mix(1.0, min(max(prior_count, 0.0) + 1.0, 32.0), stability);
}

fn cloud_common_history_feedback(prior_count: f32, moving_feedback: f32,
        stability: f32) -> f32 {
    let count = max(prior_count, 0.0);
    return mix(moving_feedback, min(count / (count + 1.0), 31.0 / 32.0), stability);
}

fn cloud_common_local_range(observer_altitude: f32, cloud_top: f32) -> f32 {
    return min(320000.0, 42000.0 + max(observer_altitude - cloud_top, 0.0) * 5.0);
}

/// Entry and exit distances along a unit ray from a planet-centred origin.
/// A miss returns (-1, -1), so a positive entry always means a real hit
/// ahead. Rays near eye level miss the planet sphere; a positive miss value
/// would clip their cloud march as if the ground were a metre away.
fn cloud_common_sphere_interval(origin: vec3<f32>, direction: vec3<f32>,
        radius: f32) -> vec2<f32> {
    let projection = dot(origin, direction);
    let radial = length(origin);
    let discriminant = projection * projection - (radial - radius) * (radial + radius);
    if discriminant < 0.0 { return vec2<f32>(-1.0); }
    let root = sqrt(discriminant);
    return vec2<f32>(-projection - root, -projection + root);
}

/// Visible interval of the bounded local volume along a unit ray from a
/// planet-centred origin. The visible sky and its cached reflection share it,
/// so both stop at the same layer, planet, range and scene-depth bounds.
/// An empty interval has y <= x.
fn cloud_common_local_segment(origin: vec3<f32>, direction: vec3<f32>,
        context: CloudContext, opaque_distance: f32) -> vec2<f32> {
    let empty = vec2<f32>(0.0, -1.0);
    let observer_altitude = max(length(origin) - context.ground_r, 0.0);
    let outer = cloud_common_sphere_interval(origin, direction, context.ground_r + context.top_m);
    if outer.y <= max(outer.x, 0.0) { return empty; }
    let inner = cloud_common_sphere_interval(origin, direction, context.ground_r + context.base_m);
    var start = max(outer.x, 0.0);
    var end = outer.y;
    if observer_altitude < context.base_m {
        if inner.y <= 0.0 { return empty; }
        start = inner.y;
    } else if inner.y > inner.x && inner.x > start {
        end = min(end, inner.x);
    }
    let ground = cloud_common_sphere_interval(origin, direction, context.ground_r);
    if ground.y > ground.x && ground.x > 0.0 { end = min(end, ground.x); }
    end = min(end, cloud_common_local_range(observer_altitude, context.top_m));
    // Clip against scene depth before integration, not only during upscale:
    // half-resolution rays would otherwise retain cloud behind mountains.
    let opaque_bias = max(0.75, opaque_distance * 0.0005);
    end = min(end, max(opaque_distance - opaque_bias, 0.0));
    return vec2<f32>(start, end);
}

struct CloudFarSample {
    density: f32,
    alpha: f32,
    distance: f32,
    sun_visibility: f32,
    position: vec3<f32>,
};

fn cloud_common_far_sample(origin: vec3<f32>, direction: vec3<f32>, context: CloudContext,
        sun_dir: vec3<f32>, opaque_distance: f32, angular_footprint: f32,
        shape_tex: texture_3d<f32>, shape_sampler: sampler) -> CloudFarSample {
    let empty = CloudFarSample(0.0, 0.0, 0.0, 1.0, origin);
    let observer_altitude = max(length(origin) - context.ground_r, 0.0);
    let flight_weight = 1.0 - smoothstep(65000.0, 130000.0, observer_altitude);
    if context.coverage <= 0.001 || flight_weight <= 0.001 { return empty; }
    let outer = cloud_common_sphere_interval(origin, direction, context.ground_r + context.top_m);
    if outer.y <= max(outer.x, 0.0) { return empty; }
    let inner = cloud_common_sphere_interval(origin, direction, context.ground_r + context.base_m);
    var start = max(outer.x, 0.0);
    var end = min(outer.y, opaque_distance);
    if observer_altitude < context.base_m {
        start = max(start, inner.y);
    } else if inner.y > inner.x && inner.x > start {
        end = min(end, inner.x);
    }
    let ground = cloud_common_sphere_interval(origin, direction, context.ground_r);
    if ground.y > ground.x && ground.x > 0.0 { end = min(end, ground.x); }
    let range = cloud_common_local_range(observer_altitude, context.top_m);
    start = max(start, range * 0.74);
    if end <= start { return empty; }
    var far_context = context;
    far_context.quality = 0.0;
    var optical_depth = 0.0;
    var density_sum = 0.0;
    var weighted_distance = 0.0;
    // A thin representative shell creates an abrupt, aliased tangent line.
    // Two quadrature samples cover the actual layer interval instead. Its
    // physical path and density both approach zero at the weather-layer limb,
    // including horizontal rays from cameras inside the upper cloud body.
    let step = (end - start) * 0.5;
    for (var i = 0u; i < 2u; i += 1u) {
        let distance = start + (f32(i) + 0.5) * step;
        let position = origin + direction * distance;
        let grazing = max(abs(dot(direction, normalize(position))), 0.035);
        let footprint = distance * angular_footprint / grazing;
        let density = cloud_common_sample_filtered(position, far_context, footprint,
            shape_tex, shape_sampler).density;
        let range_weight = smoothstep(range * 0.74, range, distance);
        let optical = cloud_common_optical_density(density) * min(step, 8000.0) * range_weight;
        optical_depth += optical;
        density_sum += density * optical;
        weighted_distance += distance * optical;
    }
    if optical_depth <= 0.001 { return empty; }
    let density = density_sum / optical_depth;
    let distance = weighted_distance / optical_depth;
    let position = origin + direction * distance;
    let height_fraction = clamp((length(position) - context.ground_r - context.base_m)
        / max(context.top_m - context.base_m, 1.0), 0.0, 1.0);
    // Far volumes spend both reads on view coverage. Estimate their remaining
    // overhead light path from the same bounded layer rather than adding a
    // third probe or sampling an unrelated fixed-altitude column.
    let light_path = (context.top_m - context.base_m) * (1.0 - height_fraction)
        / max(dot(normalize(position), sun_dir), 0.2);
    let sun_visibility = cloud_common_light_transmittance(
        cloud_common_optical_density(density) * min(light_path, 4000.0));
    let alpha = (1.0 - cloud_common_view_transmittance(optical_depth)) * flight_weight;
    return CloudFarSample(density, alpha, distance, sun_visibility, position);
}

fn cloud_common_ambient(solar_sine: f32, moon_sine: f32,
        moon_fraction: f32, dust_load: f32) -> vec3<f32> {
    let daylight = smoothstep(-0.08, 0.22, solar_sine);
    let twilight = smoothstep(-0.22, -0.01, solar_sine)
        * (1.0 - smoothstep(0.08, 0.26, solar_sine));
    return mix(vec3<f32>(0.038, 0.054, 0.083), vec3<f32>(0.34, 0.40, 0.51), daylight)
        + vec3<f32>(0.065, 0.09, 0.15) * max(moon_sine, 0.0) * moon_fraction
        + vec3<f32>(0.11, 0.035, 0.14) * twilight * (1.0 - dust_load * 0.72) * 0.30;
}
