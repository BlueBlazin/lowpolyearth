use super::*;
use std::collections::HashSet;

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    crate::ui::configure_fonts(&ctx);
    let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
    ctx
}

fn title(key: &str, primary: &str, local: Option<(&str, &str)>) -> ArrivalTitle {
    ArrivalTitle {
        profile: catalog::PROFILES.iter().find(|p| p.key == key).unwrap(),
        primary: primary.into(),
        region: Some("Region".into()),
        localized_names: local
            .into_iter()
            .map(|(a, b)| (a.into(), b.into()))
            .collect(),
    }
}

#[test]
fn authored_catalog_has_real_country_guards_and_varied_typography() {
    let contexts = vearth_data::admin_regions::AdminRegionAtlas::bundled();
    let countries: HashSet<_> = contexts.contexts().map(|r| r.country_name).collect();
    let mut keys = HashSet::new();
    let mut faces = HashSet::new();
    let mut compositions = HashSet::new();
    assert!(
        catalog::PROFILES
            .iter()
            .filter(|p| p.center[2] > 0.)
            .count()
            >= 50
    );
    for p in catalog::PROFILES {
        assert!(keys.insert(p.key), "duplicate {}", p.key);
        for country in p.countries.split('|') {
            assert!(
                countries.contains(country),
                "{}: unknown country {country}",
                p.key
            );
        }
        assert!(p.center[0].abs() <= 90. && p.center[1].abs() <= 180.);
        assert!(p.center[2] >= 0. && p.center[2] <= 550.);
        assert!((48. ..=95.).contains(&p.type_settings[0]));
        faces.insert(p.face);
        compositions.insert((p.face, p.layout, p.finish, p.case));
    }
    assert!(faces.len() >= 16);
    assert!(compositions.len() >= 50);
}

#[test]
fn geography_selects_locally_and_never_spills_across_borders_or_oceans() {
    for (country, lat, lon, key) in [
        ("India", 26.912, 75.787, "jaipur"),
        ("India", 19.076, 72.878, "mumbai"),
        ("Japan", 35.012, 135.768, "kyoto"),
        ("Japan", 35.676, 139.65, "tokyo"),
        ("Senegal", 14.717, -17.467, "dakar"),
        ("Australia", -25.344, 131.037, "uluru"),
        ("Brazil", -22.907, -43.173, "rio"),
        ("Argentina", -34.604, -58.382, "buenos-aires"),
        ("India", 12.972, 77.595, "indian-regions"),
    ] {
        assert_eq!(catalog::select(Some(country), lat, lon).key, key);
    }
    assert_eq!(catalog::select(None, 35., 135.).key, "world");
    assert_eq!(catalog::select(Some("India"), f64::NAN, 77.).key, "world");
    assert_eq!(
        catalog::select(Some("Japan"), 26.912, 75.787).key,
        "japanese-regions"
    );
    assert_eq!(catalog::select(Some("Bhutan"), 26.912, 75.787).key, "world");
}

#[test]
fn native_names_require_source_language_script_and_complete_font_coverage() {
    let ctx = context();
    for (key, en, lang, local) in [
        ("jaipur", "Jaipur", "hi", "जयपुर"),
        ("mumbai", "Mumbai", "mr", "मुंबई"),
        ("kolkata", "Kolkata", "bn", "কলকাতা"),
        ("chennai", "Chennai", "ta", "சென்னை"),
        ("hyderabad", "Hyderabad", "te", "హైదరాబాద్"),
        ("kyoto", "Kyoto", "ja", "京都"),
        ("seoul", "Seoul", "ko", "서울"),
        ("bangkok", "Bangkok", "th", "กรุงเทพมหานคร"),
        ("addis-ababa", "Addis Ababa", "am", "አዲስ አበባ"),
    ] {
        let t = title(key, en, Some((lang, local)));
        let (text, _, native) = display_text(&ctx, &t);
        assert!(native, "{key} failed native coverage");
        assert_eq!(text, local);
        let g = layout(&ctx, &t, Rect::from_min_size(Pos2::ZERO, vec2(1600., 900.)));
        assert!(!g.main.is_empty());
        assert!(g.main.mesh_bounds.is_finite());
        let (_, _, absent) = display_text(&ctx, &title(key, en, None));
        assert!(!absent);
    }
    for local in [
        ("hi", "जयपुर"),
        ("mr", "\u{e000}"),
        ("mr", "مومباي"),
        ("mr", "東京"),
    ] {
        assert!(!display_text(&ctx, &title("mumbai", "Mumbai", Some(local))).2);
    }
    assert!(!display_text(&ctx, &title("cairo", "Cairo", Some(("ar", "القاهرة")))).2);
}

#[test]
fn arrival_snapshot_keeps_names_and_does_not_translate_a_region_with_a_city_label() {
    let location = NamedLocation {
        id: 1,
        name: "Jaipur, Rajasthan".into(),
        primary_name: Some("Jaipur".into()),
        region_name: Some("Rajasthan".into()),
        localized_names: vec![("hi".into(), "जयपुर".into())],
        distance_m: 0.,
    };
    let t = ArrivalTitle::new(
        &location,
        Some((26.912_f64.to_radians(), 75.787_f64.to_radians())),
    );
    assert_eq!(t.profile.key, "jaipur");
    assert_eq!(t.primary, "Jaipur");
    assert_eq!(t.region.as_deref(), Some("Rajasthan"));
    let region = NamedLocation {
        name: "Rajasthan".into(),
        primary_name: None,
        localized_names: vec![],
        ..location
    };
    let t = ArrivalTitle::new(&region, None);
    assert_eq!(t.primary, "Rajasthan");
    assert_eq!(t.region, None);
    assert!(t.localized_names.is_empty());
}

#[test]
fn layouts_fit_long_names_small_windows_and_are_stationary_through_the_reveal() {
    let ctx = context();
    for dimensions in [
        vec2(1600., 900.),
        vec2(800., 450.),
        vec2(390., 700.),
        vec2(640., 320.),
    ] {
        let viewport = Rect::from_min_size(Pos2::ZERO, dimensions);
        for p in catalog::PROFILES {
            for name in [
                "Rio de Janeiro",
                "Lauterbrunnen",
                "San Cristóbal de las Casas",
                "A remarkably long geographic place name near a beautiful mountain range",
            ] {
                let t = ArrivalTitle {
                    profile: p,
                    primary: name.into(),
                    region: Some("A long administrative region name".into()),
                    localized_names: vec![],
                };
                let g = layout(&ctx, &t, viewport);
                assert!(
                    viewport.contains_rect(g.bounds),
                    "{} {:?}: {:?}",
                    p.key,
                    dimensions,
                    g.bounds
                );
                let again = layout(&ctx, &t, viewport);
                assert_eq!(g.main_pos, again.main_pos);
                assert_eq!(g.bounds, again.bounds);
                assert_eq!(g.main.mesh_bounds, again.main.mesh_bounds);
            }
        }
    }
    assert_eq!(stacked("Buenos Aires"), "Buenos\nAires");
    assert_eq!(stacked("जयपुर"), "जयपुर");
}

#[test]
fn reveal_endpoints_have_zero_velocity() {
    assert_eq!(smoothstep(-1.), 0.);
    assert_eq!(smoothstep(1.1), 1.);
    assert!(smoothstep(0.001) < 0.00001);
    assert!(1. - smoothstep(0.999) < 0.00001);
}
