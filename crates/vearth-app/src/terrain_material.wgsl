// Corner weights describe physical response; the source column's semantic id
// remains separate. Multiplicative height weighting cannot introduce a material
// absent from the source triangle and preserves pure-material endpoints.
const TERRAIN_PERIODS: array<f32, 4> = array<f32, 4>(16.0, 32.0, 64.0, 16.0);

fn terrain_material(position_m: vec3<f32>, normal: vec3<f32>, source_weights: vec4<f32>,
                    detail: f32, world_dx: vec3<f32>, world_dy: vec3<f32>) -> MaterialSample {
    if (detail <= 0.0) { return MaterialSample(0.0, normal, 1.0); }
    let footprint = max(length(world_dx), length(world_dy));
    let meso_visibility = 1.0 - smoothstep(16.0, 96.0, footprint);
    let macro_visibility = 1.0 - smoothstep(128.0, 768.0, footprint);
    // Integer frequency ratios preserve the 8192 m floating-origin period.
    // These modulate existing surface character without drawing geographic
    // boundaries or assigning an unsupported rock/soil type.
    let selector = material_selector(position_m);
    let broad = selector / 7.0 - 0.5;
    let weathering = material_selector(position_m * 4.0) / 7.0 - 0.5;
    var total = 0.0;
    var tone = 0.0;
    var gradient = vec3<f32>(0.0);
    var roughness_fourth = 0.0;
    for (var family = 0u; family < 4u; family++) {
        let source_weight = max(source_weights[family], 0.0);
        if (source_weight <= 0.0) { continue; }
        let surface = sample_material_selected_grad(position_m, normal, family + 2u,
            TERRAIN_PERIODS[family], detail, world_dx, world_dy, selector);
        let weight = source_weight * exp2(surface.tone * 1.6 * meso_visibility);
        let surface_gradient = normal - surface.normal / max(dot(surface.normal, normal), 0.1);
        total += weight;
        tone += surface.tone * weight;
        gradient += surface_gradient * weight;
        roughness_fourth += pow(surface.roughness, 4.0) * weight;
    }
    let inverse_weight = 1.0 / max(total, 1e-5);
    let weights = max(source_weights, vec4<f32>(0.0));
    let character = dot(weights, vec4<f32>(0.075, 0.11, 0.16, 0.035));
    let regional_tone = broad * character * macro_visibility
        + weathering * character * 0.55 * meso_visibility;
    return MaterialSample(tone * inverse_weight + regional_tone * detail,
        normalize(normal - gradient * inverse_weight),
        sqrt(sqrt(max(roughness_fourth * inverse_weight, 0.0001))));
}
