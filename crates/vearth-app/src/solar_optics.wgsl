// Shared direct-solar extinction for the visible atmosphere and reflected clouds.
// Both callers supply the same weather uniform and sample altitude in metres.

fn visibility_haze_factor() -> f32 {
    return clamp(18000.0 / max(g.weather0.z, 120.0), 0.0, 1.0);
}

/// Mineral/absorbing aerosol is strongest in dry, drought-stressed air and
/// is scavenged by recent rain. This is a physical state, not a color preset.
fn dust_optical_load() -> f32 {
    let humidity = clamp(g.weather0.y, 0.0, 1.0);
    let wetness = clamp(g.weather1.x, 0.0, 1.0);
    let drought = clamp(g.weather4.x, 0.0, 1.0);
    let aerosol = clamp(g.surface.z, 0.0, 1.0);
    let rain_washout = wetness
        * (1.0 - smoothstep(0.15, 2.0, g.weather0.w));
    return aerosol * g.weather6.x * pow(1.0 - humidity, 0.72)
        * (0.34 + 0.66 * drought) * (1.0 - rain_washout * 0.62);
}

/// Persistent urban/industrial aerosol is distinct from mineral dust. At the
/// Delhi baseline it should wash blue contrast toward a warm neutral sky;
/// clean coastal air remains largely Rayleigh-blue.
fn pollution_optical_load() -> f32 {
    let aerosol = clamp(g.surface.z, 0.0, 1.0);
    let dust_fraction = clamp(g.weather6.x, 0.0, 1.0);
    let humidity = clamp(g.weather0.y, 0.0, 1.0);
    let anthropogenic = aerosol * (1.0 - dust_fraction * 0.72);
    return smoothstep(0.07, 0.38, anthropogenic)
        * (0.72 + humidity * 0.28);
}

/// Humidity grows soluble aerosol droplets, increasing nearly neutral Mie
/// scattering and broadening the bright region around the Sun.
fn humid_optical_load() -> f32 {
    let humidity = clamp(g.weather0.y, 0.0, 1.0);
    return clamp(
        pow(humidity, 2.35) * (0.24 + 0.76 * visibility_haze_factor())
            + clamp(g.weather0.w / 18.0, 0.0, 1.0) * 0.18
            + pollution_optical_load() * 0.22,
        0.0,
        1.0,
    );
}

fn optical_air_mass(sine_altitude: f32) -> f32 {
    if sine_altitude > 0.0 {
        // Compact Kasten-like horizon correction. It approaches ~38 rather
        // than diverging when the Sun touches the geometric horizon.
        return clamp(1.0 / (
            sine_altitude + 0.026 * exp(-11.0 * sine_altitude)
        ), 1.0, 46.0);
    }
    // Below the horizon only elevated atmosphere remains illuminated.
    return clamp(38.5 + (-sine_altitude) * 210.0, 38.5, 76.0);
}

/// RGB transmittance of direct sunlight at an atmospheric sample. Molecular
/// blue extinction, the green-biased Chappuis ozone band, mineral aerosol,
/// and hygroscopic aerosol are integrated through the solar air mass.
fn solar_transmittance(sine_altitude: f32, sample_altitude_m: f32) -> vec3<f32> {
    let horizon_lift = sqrt(max(2.0 * sample_altitude_m / 6371000.0, 0.0));
    let effective_altitude = sine_altitude + horizon_lift;
    let visible = smoothstep(-0.065, 0.006, effective_altitude);
    let molecular = vec3<f32>(0.018, 0.045, 0.105);
    let ozone = vec3<f32>(0.006, 0.022, 0.004);
    let dust = vec3<f32>(0.020, 0.058, 0.145)
        * dust_optical_load() * 1.85;
    let humid = vec3<f32>(0.030, 0.032, 0.035)
        * humid_optical_load();
    let pollution = vec3<f32>(0.038, 0.055, 0.086)
        * pollution_optical_load() * 1.85;
    let extinction = molecular + ozone + dust + humid + pollution;
    return exp(-extinction * optical_air_mass(effective_altitude)) * visible;
}

