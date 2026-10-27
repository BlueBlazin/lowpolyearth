use super::*;

#[test]
fn compact_world_interfaces_preserve_position_precision_and_opaque_coverage() {
    for interface in [WorldInterface::Compact, WorldInterface::Half] {
        let source = world_shader_source_with_interface(interface);
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let vertex = module
            .entry_points
            .iter()
            .find(|entry| entry.name == "vs_world")
            .unwrap();
        let naga::TypeInner::Struct { ref members, .. } =
            module.types[vertex.function.result.as_ref().unwrap().ty].inner
        else {
            panic!("world vertex result must be a struct");
        };
        for member in members {
            let name = member.name.as_deref().unwrap();
            let scalar = match module.types[member.ty].inner {
                naga::TypeInner::Scalar(scalar) | naga::TypeInner::Vector { scalar, .. } => scalar,
                _ => panic!("unsupported world varying {name}"),
            };
            if matches!(
                name,
                "clip" | "world_pos" | "local_pos" | "construction_height" | "sample_pitch_m"
            ) {
                assert_eq!(scalar, naga::Scalar::F32, "{name} lost position precision");
            }
        }
        let fragment = module
            .entry_points
            .iter()
            .find(|entry| entry.name == "fs_world_opaque")
            .unwrap();
        assert_eq!(
            opaque_shader_tests::inspect_calls(
                &module,
                &fragment.function.body,
                &mut std::collections::HashSet::new(),
            ),
            0,
            "world fragment must retain opaque hidden-surface removal"
        );
    }
}

#[test]
fn half_world_interface_falls_back_without_requested_device_support() {
    assert_eq!(WorldInterface::from_value(None), WorldInterface::Half);
    assert_eq!(
        WorldInterface::from_value(Some("unknown")),
        WorldInterface::Original
    );
    assert_eq!(
        WorldInterface::Half.for_device(wgpu::Features::empty()),
        WorldInterface::Original
    );
    assert_eq!(
        WorldInterface::Compact.for_device(wgpu::Features::empty()),
        WorldInterface::Compact
    );
    assert_eq!(
        WorldInterface::Half.for_device(wgpu::Features::SHADER_F16),
        WorldInterface::Half
    );
}

#[test]
fn compact_world_interface_preserves_native_full_and_packed_color_depth() {
    opaque_shader_tests::verify_world_interface(WorldInterface::Compact);
}

#[test]
fn half_world_interface_preserves_native_full_and_packed_color_depth() {
    opaque_shader_tests::verify_world_interface(WorldInterface::Half);
}
