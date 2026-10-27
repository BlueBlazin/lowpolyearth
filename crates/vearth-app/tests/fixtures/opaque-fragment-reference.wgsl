// Independent pre-specialization fragment body; shared shader helpers and ABI are supplied by the test.
@fragment
fn fs_reference(in: VsOut) -> @location(0) vec4<f32> {
    let vehicle_uv_dx = dpdx(in.vehicle_uv);
    let vehicle_uv_dy = dpdy(in.vehicle_uv);
    let pixel_noise = pixel_hash(vec2<u32>(floor(in.clip.xy)));
    // LOD crossfade: screen-door dither on direction-free pixel noise.
    // Fading in (a > 0) keeps pixels where h < a; fading out (a < 0,
    // weight |a|) keeps the exact complement h >= 1-|a|, so a paired
    // refine/merge swap writes every pixel exactly once — no z-fight, no
    // punch-through. a == 1 (steady state) skips the whole block.
    let a = in.fade;
    if (a < 1.0) {
        let keep = select(pixel_noise >= 1.0 + a, pixel_noise < a, 0.0 <= a);
        if (!keep) {
            discard;
        }
    }
    // Emissive (booster flames): self-lit, no sun/AO/fog.
    if (in.emissive > 0.5) {
        return vec4<f32>(in.color * 1.4, 1.0);
    }
    let material_id = (in.face_mat >> 5u) & 255u;
    let is_highrise = material_id >= 103u && material_id <= 118u;
    let blended_terrain = dot(in.terrain_weights, vec4<f32>(1.0)) > 0.0;
    let is_vehicle = material_id >= 96u && material_id <= 102u;
    // Semantic foliage ids retain season and motion while sharing one leaf-art
    // layer. Material id is not necessarily the atlas layer.
    let is_foliage = material_id >= 22u && material_id < 66u;
    let is_tree_material = material_id == 13u || is_foliage;
    let is_near_tree = in.tree_transition < 0.0;
    if (is_tree_material) {
        let near_blend = select(
            clamp(in.tree_transition, 0.0, 1.0),
            clamp(-in.tree_transition - 1.0, 0.0, 1.0),
            is_near_tree,
        );
        // Complementary masks retain exactly one representation at every
        // pixel while the detailed mesh replaces the streamed fallback.
        let keep_tree = select(
            pixel_noise >= near_blend,
            pixel_noise < near_blend,
            is_near_tree,
        );
        if (!keep_tree) {
            discard;
        }
        if (in.tree_lod_alpha < 1.0) {
            // An independent screen-space mask preserves both adjacent-LOD
            // coverage and the streamed/instanced tree transition above.
            let lod_noise = pixel_hash(vec2<u32>(floor(in.clip.xy)) + vec2<u32>(173u, 941u));
            let alpha = in.tree_lod_alpha;
            let keep_lod = select(lod_noise >= 1.0 + alpha, (lod_noise < alpha), 0.0 <= alpha);
            if (!keep_lod) { discard; }
        }
    }
    var layer = select(min(material_id, 22u), 14u, is_foliage);
    // Expanded persistent surface semantics reuse physical art rather than
    // consuming one atlas layer per authored colour family. Keep this mapping
    // in sync with atlas::material_traits.
    switch material_id {
        case 66u: { layer = 2u; } // soil -> sand/aggregate grain
        case 67u: { layer = 2u; } // salt/gypsum -> fine granular crust
        case 68u: { layer = 5u; } // ice -> snow/ice grain
        case 69u: { layer = 4u; } // volcanic -> rock grain
        case 70u: { layer = 3u; } // wetland/peat -> organic ground
        case 71u: { layer = 16u; } // aviation beacon -> painted metal
        case 72u: { layer = 3u; } // growing crop -> vegetated ground
        case 73u: { layer = 3u; } // flower field -> vegetated ground
        case 74u: { layer = 3u; } // orchard surface -> vegetated ground
        case 75u: { layer = 3u; } // vineyard surface -> vegetated ground
        case 76u: { layer = 3u; } // meadow surface -> vegetated ground
        case 77u: { layer = 2u; } // dormant field -> dry aggregate
        case 78u: { layer = 2u; } // harvested field -> soil/stubble
        case 79u: { layer = 16u; } // landmark lamp -> painted metal
        case 80u: { layer = 16u; } // landmark sparkle fixture -> painted metal
        case 81u: { layer = 23u; }
        case 82u: { layer = 24u; }
        case 83u: { layer = 25u; }
        case 84u: { layer = 26u; }
        case 85u: { layer = 27u; }
        case 86u: { layer = 28u; }
        case 87u: { layer = 29u; }
        case 88u: { layer = 30u; }
        case 89u: { layer = 31u; }
        case 90u: { layer = 32u; }
        case 91u: { layer = 33u; }
        case 92u: { layer = 34u; }
        case 93u: { layer = 35u; }
        case 94u, 95u: { layer = 16u; }
        case 119u: { layer = 36u; }
        case 120u: { layer = 37u; }
        case 121u: { layer = 38u; }
        case 122u: { layer = 39u; }
        case 123u: { layer = 40u; }
        case 124u: { layer = 41u; }
        case 125u: { layer = 42u; }
        case 126u: { layer = 43u; }
        case 127u: { layer = 44u; }
        default: {}
    }
    // Bits 13..14 are material-scoped; water and vehicle context is unchanged.
    if (((in.face_mat >> 13u) & 3u) == 1u) {
        switch material_id {
            case 87u, 88u: { layer = 45u; }
            case 85u: { layer = 46u; }
            case 82u: { layer = 47u; }
            case 84u: { layer = 48u; }
            case 90u: { layer = 49u; }
            case 93u: { layer = 53u; }
            default: {}
        }
    }
    let patterned_azulejo = ((in.face_mat >> 13u) & 3u) == 2u && material_id == 93u;
    if patterned_azulejo { layer = 54u; }
    let matte_corrugated = ((in.face_mat >> 13u) & 3u) == 2u
        && (material_id == 87u || material_id == 88u);
    if matte_corrugated { layer = 45u; }
    if ((in.face_mat >> 13u) & 3u) == 2u && material_id == 84u { layer = 50u; }
    let named_roof_finish = material_id == 12u && ((in.face_mat >> 13u) & 3u) != 0u;
    if named_roof_finish {
        switch ((in.face_mat >> 13u) & 3u) {
            case 1u: { layer = 51u; }
            case 2u: { layer = 52u; }
            case 3u: { layer = 33u; }
            default: {}
        }
    }
    let tile_stamped_metal = ((in.face_mat >> 13u) & 3u) == 3u && material_id == 88u;
    if tile_stamped_metal { layer = 27u; }
    var period_m = 16.0;
    switch layer {
        case 3u: { period_m = 32.0; }
        case 4u: { period_m = 64.0; }
        case 14u: { period_m = 8.0; }
        case 11u, 12u: { period_m = 4.0; }
        case 13u: { period_m = 2.0; }
        case 37u: { period_m = 8.0; }
        case 38u: { period_m = 4.0; }
        case 7u: { period_m = 8.0; }
        case 10u, 39u, 42u, 44u: { period_m = 4.0; }
        case 40u, 41u, 43u: { period_m = 2.0; }
        default: {}
    }
    let material_detail = 1.0 - smoothstep(48.0, 512.0, in.sample_pitch_m);
    var base_normal = normalize(in.base_normal);
    if (material_id == 1u) {
        let ellipsoid = in.world_pos - g.planet_center.xyz;
        base_normal = normalize(ellipsoid * vec3<f32>(1.0, 1.0, 1.0067395));
    }
    let material_position = in.world_pos + g.material_origin.xyz;
    let world_dx = dpdx(in.world_pos);
    let world_dy = dpdy(in.world_pos);
    let water_footprint = max(length(world_dx), length(world_dy));
    var tower = HighriseSample(0.0, in.color, 0.75, vec3<f32>(0.0));
    if is_highrise { tower = highrise_material(in, material_id); }
    var surface_material: MaterialSample;
    var water = WaterSurface(base_normal, 0.15, 0.5);
    let water_kind = (in.face_mat >> 13u) & 3u;
    let water_shelter = water_exposure(water_kind);
    if (material_id == 94u) {
        surface_material = MaterialSample(0.0, base_normal, 1.0);
    } else if (is_highrise) {
        surface_material = MaterialSample(0.0, base_normal, tower.roughness);
    } else if (material_id == 95u) {
        surface_material = MaterialSample(0.0, base_normal, 0.28);
    } else if (is_vehicle) {
        surface_material = MaterialSample(0.0, base_normal, vehicle_material(material_id).roughness);
    } else if (material_id == 1u) {
        if (material_detail > 0.0) {
            water = water_surface(material_position, base_normal,
                (length(g.wind.xy) + g.wind.w * 0.25) * water_shelter.x,
                g.surface.y * water_shelter.y, g.weather2.x, water_footprint);
            // Constant binary-rational drift moves by whole 8192 m periods at
            // the 65536 s weather-clock wrap, including the material selector.
            let drift = vec3<f32>(0.125, -0.25, 0.125) * g.weather2.x;
            let residual = sample_material(material_position + drift,
                base_normal, 1u, 16.0, material_detail);
            let wind_energy = clamp((length(g.wind.xy) + g.wind.w * 0.25) * water_shelter.x / 22.0, 0.0, 1.0);
            let sea_energy = clamp(g.surface.y * water_shelter.y, 0.0, 1.0);
            let residual_strength = (0.30 + 1.15 * sea_energy + 0.55 * wind_energy) * 0.50;
            let carrier_gradient = base_normal
                - water.normal / max(dot(water.normal, base_normal), 0.01);
            let residual_gradient = base_normal
                - residual.normal / max(dot(residual.normal, base_normal), 0.01);
            water.normal = normalize(base_normal - carrier_gradient * (0.60 * material_detail)
                - residual_gradient * residual_strength);
            let base_roughness = 0.095 + 0.075 * wind_energy + 0.10 * sea_energy;
            let carrier_variance = max(pow(water.roughness, 4.0) - pow(base_roughness, 4.0), 0.0)
                * (0.36 * material_detail * material_detail);
            // Atlas alpha includes unresolved residual slope variance. Its
            // base is the encoded 31/255 value of authored roughness 0.12.
            let residual_variance = max(pow(residual.roughness, 4.0) - pow(31.0 / 255.0, 4.0), 0.0);
            water.roughness = clamp(water_roughness_from_slope_variance(base_roughness,
                carrier_variance + residual_variance * residual_strength * residual_strength
                    * material_detail * material_detail), 0.095, 0.50);
        }
        surface_material = MaterialSample(0.0, water.normal, water.roughness);
    } else if (blended_terrain) {
        surface_material = terrain_material(material_position, base_normal,
            in.terrain_weights, material_detail, world_dx, world_dy);
    } else if ((material_id >= 81u && material_id <= 93u) || material_id == 119u || named_roof_finish) {
        surface_material = construction_material(in.construction_height, material_position, in.world_pos,
            base_normal, layer, material_id, world_dx, world_dy, material_detail);
        if matte_corrugated {
            surface_material.roughness = max(surface_material.roughness, 0.9);
        }
        if tile_stamped_metal {
            // A continuous coated sheet shares the crown profile, not ceramic firing.
            surface_material.tone *= 0.25;
            let filtered_variance = max(pow(surface_material.roughness, 4.0) - pow(0.67, 4.0), 0.0);
            surface_material.roughness = pow(pow(0.51, 4.0) + filtered_variance, 0.25);
        }
    } else {
        surface_material = sample_material(material_position,
            base_normal, layer, period_m, material_detail);
    }
    let tone = surface_material.tone;
    let tonal_detail = vec3<f32>(1.0)
        + min(tone, 0.0) * vec3<f32>(0.68, 0.58, 0.42)
        + max(tone, 0.0) * vec3<f32>(0.55, 0.42, 0.20);
    let n = surface_material.normal;
    let sun_dir = normalize(g.sun_dir.xyz);
    let moon_dir = normalize(g.moon_dir.xyz);
    let cycle = g.sun_dir.w > 0.5;
    let pc = g.planet_center.xyz;
    let stable_ecef = in.world_pos - pc;
    // The uniform weather sample describes the camera locality. Transient
    // wet/snow/wind effects must not recolor a continent when this fragment
    // belongs to terrain sampled at kilometre-scale orbital spacing.
    let local_weather_detail = 1.0 - smoothstep(48.0, 512.0, in.sample_pitch_m);

    // Local solar elevation is evaluated per fragment. From orbit this lets
    // noon, a warm terminator and night coexist on the same visible globe;
    // it also prevents east/west faces on the night side seeing the Sun.
    let up = normalize(in.world_pos - pc);
    let solar_alt = dot(up, sun_dir);
    let horizon = select(1.0, smoothstep(-0.015, 0.025, solar_alt), cycle);
    let ndl = max(dot(n, sun_dir), 0.0) * horizon;
    let base_ndl = max(dot(base_normal, sun_dir), 0.0) * horizon;
    let lunar_alt = dot(up, moon_dir);
    let moon_horizon = smoothstep(-0.01, 0.025, lunar_alt);
    let moon_phase = select(0.0, g.moon_dir.w, cycle);
    let moon_visibility = moon_horizon * moon_phase;
    // Keep creases readable without turning already-shadowed cliffs black.
    var ao = mix(0.62, 1.0, in.ao);
    if ((is_foliage || material_id == 14u) && (in.face_mat & 0x80000000u) != 0u) {
        ao = mix(0.68, 1.0, in.ao);
    }

    var direct_color = vec3<f32>(1.68, 1.59, 1.36);
    var ambient = g.ambient.rgb;
    if (cycle) {
        // Blue wavelengths travel the longest atmospheric path when the Sun
        // is low, leaving warm direct light. The curve is art-directed to
        // keep golden hour legible across the surface palette.
        let sun_height = max(solar_alt, 0.0);
        let daylight = smoothstep(0.035, 0.32, sun_height);
        let low_sun = vec3<f32>(1.42, 0.66, 0.24);
        direct_color = mix(low_sun, vec3<f32>(1.68, 1.59, 1.36), daylight);
        direct_color *= mix(0.62, 1.0, smoothstep(0.0, 0.20, sun_height));

        // Warm light stays directional; unlit faces retain a cool sky fill.
        // Civil twilight fades into a deliberately faint blue night floor.
        let day_fill = smoothstep(-0.10, 0.16, solar_alt);
        let twilight = smoothstep(-0.22, -0.015, solar_alt)
            * (1.0 - smoothstep(0.06, 0.28, solar_alt));
        // Preserve a navigable blue-hour floor even without Moon light; the
        // scene should read as night without crushing terrain silhouettes.
        ambient = mix(vec3<f32>(0.020, 0.028, 0.058), g.ambient.rgb, day_fill)
            + twilight * vec3<f32>(0.025, 0.010, 0.040);
        // The Moon remains a restrained cool key rather than a second Sun.
        // A small hemispheric lift keeps surface silhouettes readable while
        // the directional term gives moon-facing facets a crisp blue edge.
        ambient += vec3<f32>(0.020, 0.032, 0.070) * moon_visibility;
    }
    let local_overcast = smoothstep(2.4, 8.5, g.weather5.w) * local_weather_detail;
    direct_color *= mix(1.0, 0.18, local_overcast);
    ambient = mix(
        ambient,
        vec3<f32>(0.13, 0.15, 0.18) * (0.38 + 0.62 * max(solar_alt, 0.0)),
        local_overcast * 0.72,
    );
    // A single shadowed key light gives terrain, structures and tree facets
    // the same form. Hemisphere fill keeps downward and side-facing surfaces
    // readable without flattening the crown's baked local occlusion.
    var shadow_ndl = base_ndl;
    if ((material_id >= 81u && material_id <= 93u) || (material_id >= 119u && material_id <= 121u) || named_roof_finish) {
        // Visibility follows the triangle's plane at grazing light even when
        // the packed artistic normal tilts across the illumination boundary.
        let plane = cross(world_dx, world_dy);
        let facing = select(-1.0, 1.0, dot(plane, base_normal) >= 0.0);
        shadow_ndl = max(dot(plane, sun_dir) * facing, 0.0)
            / max(length(plane), 0.00000001) * horizon;
    }
    let key_visibility = sun_visibility(in.world_pos, shadow_ndl);
    let style_detail = 1.0 - smoothstep(48.0, 512.0, in.sample_pitch_m);
    let diffuse = mix(base_ndl,
        illustrative_diffuse(dot(n, sun_dir)) * horizon, style_detail);
    let base_sun = direct_color * diffuse * mix(0.08, 1.0, key_visibility);
    let fill_daylight = select(1.0, smoothstep(-0.10, 0.16, solar_alt), cycle);
    ambient = mix(ambient, illustrative_fill(ambient, dot(n, up), fill_daylight), style_detail);
    let base_moon = vec3<f32>(0.070, 0.100, 0.180)
        * max(dot(base_normal, moon_dir), 0.0) * moon_visibility;
    let normal_dx = dpdx(n);
    let normal_dy = dpdy(n);
    let normal_variance = min(0.25, max(dot(normal_dx, normal_dx), dot(normal_dy, normal_dy)));
    if (is_vehicle) {
        let vehicle_lit = vehicle_lighting(in, material_id, n, up, ambient, direct_color,
            key_visibility, horizon, moon_visibility, ao, normal_variance, vehicle_uv_dx, vehicle_uv_dy);
        return vec4<f32>(apply_aerial_perspective(vehicle_lit, in.world_pos, in.sample_pitch_m), 1.0);
    }
    if (material_id == 95u) {
        let metal_lit = sculpture_metal_lighting(in, n, up, ambient, direct_color,
            key_visibility, horizon, moon_visibility, ao, normal_variance);
        return vec4<f32>(apply_aerial_perspective(metal_lit, in.world_pos, in.sample_pitch_m), 1.0);
    }
    var albedo = select(in.color * tonal_detail, tower.color, is_highrise);
    if patterned_azulejo { albedo = azulejo_pattern_color(in.color, surface_material.tone); }
    if material_id == 1u {
        albedo = water_bed_color(albedo, material_position, water_footprint, water_kind);
    }
    var motion_light = 1.0;
    if (is_foliage) {
        let semantic = material_id - 22u;
        let season = semantic / 4u;
        let motion = semantic % 4u;
        let phase = g.surface.x;
        let autumn = smoothstep(0.56, 0.69, phase)
            * (1.0 - smoothstep(0.84, 0.94, phase));
        let winter = smoothstep(0.80, 0.96, phase)
            * (1.0 - smoothstep(0.08, 0.20, phase));
        let spring = smoothstep(0.08, 0.18, phase)
            * (1.0 - smoothstep(0.28, 0.38, phase));
        switch season {
            case 0u: { // evergreen needle
                albedo *= mix(vec3<f32>(0.82, 0.92, 0.80), vec3<f32>(1.0), 1.0 - winter * 0.18);
            }
            case 1u: { // evergreen broadleaf
                albedo *= mix(vec3<f32>(0.82, 0.90, 0.76), vec3<f32>(1.02, 1.04, 0.98), g.weather1.x);
            }
            case 2u: { albedo = mix(albedo, vec3<f32>(0.50, 0.29, 0.065), autumn * 0.82); }
            case 3u: { albedo = mix(albedo, vec3<f32>(0.62, 0.20, 0.055), autumn * 0.82); }
            case 4u: { albedo = mix(albedo, vec3<f32>(0.42, 0.055, 0.028), autumn * 0.88); }
            case 5u: { albedo = mix(albedo, vec3<f32>(0.55, 0.32, 0.06), autumn * 0.90); }
            case 6u: { // drought-deciduous follows moisture, not latitude
                let dry = clamp((1.0 - g.weather0.y) * (1.0 - g.weather1.x), 0.0, 1.0);
                albedo = mix(albedo, vec3<f32>(0.34, 0.26, 0.105), dry * 0.72);
            }
            case 7u: { albedo = mix(albedo, vec3<f32>(1.0, 0.48, 0.66), spring * 0.70); }
            case 8u: { albedo = mix(albedo, vec3<f32>(1.0, 0.72, 0.78), spring * 0.78); }
            case 9u: { albedo *= mix(vec3<f32>(0.78, 0.84, 0.65), vec3<f32>(1.04, 1.08, 0.98), g.weather1.x); }
            default: { albedo *= mix(vec3<f32>(0.90, 0.88, 0.72), vec3<f32>(1.0), g.weather0.y); }
        }
        // Stable shading shimmer: geometry, depth and collision never move.
        let wind_speed = length(g.wind.xy) + g.wind.w * 0.35;
        var amplitude = 0.018;
        switch motion {
            case 1u: { amplitude = 0.040; }
            case 2u: { amplitude = 0.060; }
            case 3u: { amplitude = 0.085; }
            default: {}
        }
        let distant_fade = 1.0 - smoothstep(18.0, 90.0, in.sample_pitch_m);
        let phase_motion = dot(stable_ecef * 0.0025, vec3<f32>(0.71, 0.37, 0.53))
            + g.weather2.x * (0.7 + wind_speed * 0.08);
        motion_light += sin(phase_motion) * amplitude
            * smoothstep(0.8, 12.0, wind_speed) * distant_fade;
    }
    var field_runtime_presence = 1.0;
    if (material_id >= 72u && material_id <= 78u) { // semantic field surfaces
        let local_phase = g.surface.x;
        let astronomical_phase = g.weather6.w;
        let moisture = clamp(max(g.weather1.x, g.weather6.y), 0.0, 1.0);
        let monsoonal = smoothstep(0.58, 0.78, g.weather0.y)
            * smoothstep(0.20, 0.54, moisture);
        let tropical = smoothstep(18.0, 24.0, g.weather0.x) * (1.0 - monsoonal);
        let tropical_cycle = fract(astronomical_phase * 2.0);
        let temperate_growth = smoothstep(0.10, 0.25, local_phase)
            * (1.0 - smoothstep(0.67, 0.84, local_phase));
        let temperate_bloom = smoothstep(0.24, 0.36, local_phase)
            * (1.0 - smoothstep(0.52, 0.62, local_phase));
        let temperate_mature = smoothstep(0.46, 0.57, local_phase)
            * (1.0 - smoothstep(0.70, 0.80, local_phase));
        let temperate_senescent = smoothstep(0.62, 0.76, local_phase)
            * (1.0 - smoothstep(0.91, 0.98, local_phase));
        let temperate_stubble = smoothstep(0.74, 0.82, local_phase)
            * (1.0 - smoothstep(0.94, 0.995, local_phase));
        let tropical_growth = smoothstep(0.05, 0.18, tropical_cycle)
            * (1.0 - smoothstep(0.66, 0.82, tropical_cycle)) * moisture;
        let tropical_bloom = smoothstep(0.27, 0.38, tropical_cycle)
            * (1.0 - smoothstep(0.54, 0.64, tropical_cycle)) * moisture;
        let monsoon_growth = smoothstep(0.20, 0.58, moisture);
        let growth = mix(
            temperate_growth,
            mix(tropical_growth, monsoon_growth, monsoonal),
            max(tropical, monsoonal),
        );
        let bloom = mix(
            temperate_bloom,
            mix(tropical_bloom, monsoon_growth * 0.72, monsoonal),
            max(tropical, monsoonal),
        );
        let mature = mix(temperate_mature, 1.0 - growth, max(tropical, monsoonal));
        let senescent = max(
            temperate_senescent * (1.0 - max(tropical, monsoonal)),
            g.weather4.x * 0.76,
        );
        let stubble = temperate_stubble * (1.0 - max(tropical, monsoonal));
        let annual_structure = max(max(growth, mature), max(senescent, stubble));
        field_runtime_presence = annual_structure;
        // Seasonal material targets are linear albedo, matching decoded vertex colors.
        if (material_id == 72u) { // annual crop
            // The regional catalog owns soil color; field descriptors own the
            // crop calendar. An independent terrain clock would turn a green
            // rice/vegetable field into ripe cereal beneath its living canopy.
            field_runtime_presence = 0.0;
        } else if (material_id == 73u) { // commercial/formal flower surface
            // Exact flower calendar/palette state lives in FloraCell, not the
            // terrain vertex. Keep the observed/procedural substrate neutral;
            // close plants and distant aggregates provide all synthetic
            // seasonal flower colour without a mismatched terrain clock.
            albedo = mix(albedo, vec3<f32>(0.12596739, 0.08898153, 0.04696420), 0.08);
            field_runtime_presence = 0.0;
        } else if (material_id == 74u) { // orchard
            albedo = mix(albedo, vec3<f32>(0.76776876, 0.46236137, 0.50707851), temperate_bloom * 0.48);
            albedo = mix(albedo, vec3<f32>(0.38005634, 0.10048151, 0.04323364), senescent * 0.72);
            albedo = mix(vec3<f32>(0.12596739, 0.08898153, 0.04696420), albedo, 0.18 + growth * 0.82);
            field_runtime_presence = max(growth, max(temperate_bloom, senescent));
        } else if (material_id == 75u) { // vineyard
            // Alleys retain their substrate through the vine's leaf cycle.
            field_runtime_presence = 0.0;
        } else if (material_id == 76u) { // meadow
            albedo = mix(albedo, vec3<f32>(0.17886750, 0.26327341, 0.09462963), growth * 0.26);
            albedo = mix(albedo, vec3<f32>(0.33034371, 0.27383842, 0.11928019), senescent * 0.46);
            field_runtime_presence = max(growth, senescent);
        } else if (material_id == 77u) { // dormant rows
            albedo = mix(albedo, vec3<f32>(0.14731900, 0.22341399, 0.07828832), growth * 0.38);
            field_runtime_presence = growth;
        } else { // harvested/stubble
            albedo = mix(albedo, vec3<f32>(0.24286681, 0.15487249, 0.06372380), 0.28 + (1.0 - growth) * 0.52);
            field_runtime_presence = max(stubble, growth);
        }
    }
    if (blended_terrain || material_id == 3u || material_id == 70u
        || (material_id >= 72u && material_id <= 76u)) { // vegetated ground response
        let vegetation_weight = select(1.0, in.terrain_weights.y, blended_terrain);
        // Humid, drought-free vegetation stays green between rain events.
        // Fields still retain their lifecycle presence above.
        let growing_season = smoothstep(0.06, 0.22, g.surface.x)
            * (1.0 - smoothstep(0.74, 0.94, g.surface.x));
        let upward_vegetation = smoothstep(0.12, 0.55, dot(base_normal, up));
        let climate_green = g.weather0.y * (1.0 - g.weather4.x)
            * smoothstep(-8.0, 8.0, g.weather0.x) * growing_season
            * upward_vegetation;
        let green_flush = max(g.weather1.x * g.weather0.y, climate_green * 0.60)
            * local_weather_detail
            * select(1.0, field_runtime_presence, material_id >= 72u && material_id <= 78u);
        let dry = select(
            clamp((1.0 - g.weather0.y) * max(g.weather0.x / 32.0, 0.0), 0.0, 1.0),
            0.0,
            material_id == 70u && !blended_terrain,
        ) * local_weather_detail;
        // Pigment response preserves source variation and dark grass detail.
        // Crop lifecycle colours retain their separate, restrained flush.
        if (blended_terrain || material_id == 3u || material_id == 70u) {
            let healthy_grass = smoothstep(0.08, 0.42, green_flush) * vegetation_weight;
            albedo *= mix(vec3<f32>(1.0), vec3<f32>(0.86, 0.97, 0.78), healthy_grass);
        } else {
            albedo = mix(albedo, vec3<f32>(0.25, 0.34, 0.12), green_flush * 0.12 * vegetation_weight);
        }
        albedo = mix(albedo, vec3<f32>(0.40, 0.30, 0.14), dry * 0.26 * vegetation_weight);
    }
    let base_lighting = (ambient + base_sun + base_moon) * ao;
    var lit = albedo * motion_light * base_lighting;

    let view = -normalize(in.world_pos);
    let half_direction = normalize(view + sun_dir);
    let roughness = clamp(surface_material.roughness, 0.12, 1.0);
    // Unresolved normal variation broadens the lobe instead of producing
    // isolated hot pixels that spatial edge smoothing cannot reconstruct.
    let specular_power = clamp(2.0 / (pow(roughness, 4.0) + normal_variance) - 2.0, 2.0, select(192.0, 2048.0, material_id == 1u));
    let fresnel = select(0.035 + 0.965 * pow(1.0 - max(dot(view, half_direction), 0.0), 5.0),
        water_fresnel(dot(view, half_direction)), material_id == 1u);
    let specular = pow(max(dot(n, half_direction), 0.0), specular_power)
        * ((specular_power + 2.0) * 0.125) * fresnel
        * ndl * key_visibility * material_detail;
    // Rough stone and plaster have a broad, weak response; metal and water
    // preserve a tighter highlight. Relief changes light, not geometry.
    if (!(is_foliage || material_id == 14u) && material_id != 1u) {
        let reflectance = select(vec3<f32>(0.55), albedo * 3.0, material_id == 16u);
        lit += direct_color * reflectance * specular;
    }
    if (is_foliage || material_id == 14u) {
        let backlight = pow(max(dot(-view, sun_dir), 0.0), 3.0) * horizon;
        lit += albedo * direct_color * backlight * 0.085 * ao
            * mix(0.35, 1.0, key_visibility);
    }

    // Runtime surface response never changes chunks or cache keys.
    let emissive_surface = material_id == 15u || (material_id >= 18u && material_id <= 21u)
        || material_id == 71u || material_id == 79u || material_id == 80u || material_id == 94u;
    let glass_surface = material_id == 15u || material_id == 18u || is_highrise;
    let submerged = material_id == 1u;
    // Smooth terrain uses the same continuous frozen coverage for weather;
    // a flat owner id must not reintroduce a seam through rain or snow.
    let weather_coverage = clamp(1.0 - in.terrain_weights.w, 0.0, 1.0);
    let wettable_surface = blended_terrain || (material_id != 67u && material_id != 68u);
    let snow_eligible_surface = blended_terrain || material_id != 68u;
    let wet_coverage = select(1.0, weather_coverage, blended_terrain);
    if (!emissive_surface && !submerged && wettable_surface) {
        let wet = g.weather1.x * local_weather_detail * wet_coverage;
        lit *= mix(1.0, 0.70, wet);
        let broad_reflection = pow(
            max(dot(reflect(-normalize(in.world_pos), base_normal), up), 0.0),
            10.0,
        );
        lit += vec3<f32>(0.16, 0.22, 0.30) * broad_reflection * wet;
    }
    if (!emissive_surface && !submerged && snow_eligible_surface) {
        let snow = g.weather1.y * local_weather_detail * wet_coverage
            * smoothstep(0.35, 0.78, dot(base_normal, up));
        lit = mix(
            lit,
            vec3<f32>(0.72, 0.80, 0.88)
                * (ambient + base_sun * 0.72),
            snow,
        );
    }
    if (submerged) {
        let sea = g.surface.y * water_shelter.y * local_weather_detail;
        var reflected_sky = vec3<f32>(0.0);
        if (material_detail > 0.0) {
            reflected_sky = sample_reflected_environment(reflect(-view, n));
        }
        let reflection = water_fresnel(max(dot(n, view), 0.0));
        // Depth/sediment colours describe the opaque water body. Fresnel
        // replaces that response with the sky as the viewing angle grazes it.
        lit *= mix(1.0, 0.76, sea);
        lit += water_crest_scattering(albedo, water.crest, dot(n, view), dot(-view, sun_dir),
            direct_color * horizon * key_visibility) * material_detail;
        lit = mix(lit, reflected_sky, reflection * material_detail);
        lit += direct_color * specular * 0.35;
    }
    if (glass_surface && material_detail > 0.0) {
        let environment = sample_reflected_environment(reflect(-view, n));
        let glass_fresnel = 0.04 + 0.96 * pow(1.0 - max(dot(n, view), 0.0), 5.0);
        // Reflection replaces part of the dark interior response. A minimum
        // sky-colour floor would brighten every distant window independently
        // of incidence, erasing the authored facade and joinery palette.
        let coverage = select(1.0, tower.glass, is_highrise);
        let reflection = select(0.06 + 0.72 * glass_fresnel, 0.025 + 0.30 * glass_fresnel, is_highrise);
        lit = mix(lit, environment, reflection * material_detail * coverage);
    }

    if is_highrise && cycle && g.viewport.w > 0.5 {
        let night = 1.0 - smoothstep(-0.10, 0.07, solar_alt);
        lit += tower.emission * night;
    }

    // Treat the discharge as a localized cloud-height source instead of a
    // global color lift. Nearby faces turn toward it, range falls off through
    // the storm, and the existing fog below attenuates the result.
    if g.weather2.w > 0.002 {
        let strike_latitude = g.weather4.z;
        let strike_longitude = g.weather4.w;
        let strike_cos_latitude = cos(strike_latitude);
        let strike_direction = vec3<f32>(
            strike_cos_latitude * cos(strike_longitude),
            strike_cos_latitude * sin(strike_longitude),
            sin(strike_latitude),
        );
        let strike_ground = g.planet_center.xyz
            + strike_direction * g.planet_center.w;
        let electrical_source = strike_ground + strike_direction
            * mix(g.weather3.x, g.weather3.y, 0.42);
        let toward_source = electrical_source - in.world_pos;
        let source_distance = length(toward_source);
        let facing = max(dot(n, toward_source / max(source_distance, 1.0)), 0.0);
        let range_falloff = exp(-source_distance / 24000.0);
        let electrical_light = (0.10 + facing * 0.52) * range_falloff * g.weather2.w;
        lit += albedo * vec3<f32>(0.46, 0.57, 0.88) * electrical_light;
    }
    // Lit panes already have source-fitted opening geometry. Emission fills
    // that opening without a second grid inside it. Legacy facade proxies
    // retain their area coverage; horizontal surfaces never emit.
    var window_coverage = 0.0;
    switch layer {
        case 18u: { window_coverage = 1.0; }
        case 19u: { window_coverage = 0.1008; }
        case 20u: { window_coverage = 0.20; }
        case 21u: { window_coverage = 0.3456; }
        default: {}
    }
    if (cycle && g.viewport.w > 0.5 && window_coverage > 0.0) {
        let vertical_face = 1.0 - smoothstep(0.30, 0.72, abs(dot(n, up)));
        let night = 1.0 - smoothstep(-0.10, 0.07, solar_alt);
        // Match the warm residential highrise light's color and radiance.
        lit += vec3<f32>(0.86, 0.58, 0.29) * 0.16 * window_coverage * night * vertical_face;
    }

    // Observation-tower obstruction lamp. Unlike facade apertures, the
    // entire tiny cap emits so it remains readable from aircraft distance;
    // daylight retains the physical red-painted fixture without a glow.
    if (cycle && g.viewport.w > 0.5 && material_id == 71u) {
        let night = 1.0 - smoothstep(-0.10, 0.07, solar_alt);
        lit += vec3<f32>(1.15, 0.012, 0.006) * night;
    }

    // Curated-landmark fixtures retain their authored surface colors by day and
    // become emissive only at night. Eiffel sparkle fixtures join the warm
    // architectural wash between shows, then switch to a sparse on/off
    // multi-colour duty cycle for five minutes from every simulation-clock
    // hour mark. Each fixture has a stable phase but re-rolls several times
    // per second, producing glitter rather than a synchronized tower pulse.
    if (cycle && g.viewport.w > 0.5 && (material_id == 79u || material_id == 80u)) {
        let night = 1.0 - smoothstep(-0.10, 0.07, solar_alt);
        let hourly_show = material_id == 80u && g.temporal.w < 300.0;
        if (!hourly_show) {
            let steady_strength = select(0.92, 0.54, material_id == 80u);
            lit += vec3<f32>(1.24, 0.43, 0.055) * night * steady_strength;
        } else {
            let fixture_phase = dot(
                floor(abs(in.local_pos) * 0.5),
                vec3<f32>(12.9898, 78.233, 37.719),
            );
            let flash_tick = floor(g.weather2.x * 8.0);
            let flash_hash = fract(sin(fixture_phase + flash_tick * 17.17) * 43758.5453);
            let colour_hash = fract(sin(fixture_phase * 1.71 + flash_tick * 3.11) * 24634.6345);
            var sparkle_colour = vec3<f32>(1.30, 0.18, 0.62);
            if (colour_hash < 0.25) {
                sparkle_colour = vec3<f32>(0.12, 0.82, 1.42);
            } else if (colour_hash < 0.50) {
                sparkle_colour = vec3<f32>(0.24, 1.24, 0.38);
            } else if (colour_hash < 0.75) {
                sparkle_colour = vec3<f32>(1.48, 0.72, 0.08);
            }
            // A fixture below the threshold contributes no emission at all.
            // Roughly one third are lit on any tick; the independently
            // hashed next tick turns some off while lighting others.
            let flash = smoothstep(0.67, 0.78, flash_hash)
                * (1.35 + smoothstep(0.78, 0.98, flash_hash) * 1.25);
            lit += sparkle_colour * night * flash;
        }
    }

    if (g.viewport.w > 0.5 && material_id == 94u) {
        let night = 1.0 - smoothstep(-0.10, 0.07, solar_alt);
        lit += in.color * (2.4 * night);
    }
    return vec4<f32>(apply_aerial_perspective(lit, in.world_pos, in.sample_pitch_m), 1.0);
}
