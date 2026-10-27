use super::{
    Landmark, TerrainFlatten, heritage::TerrainCut, heritage_generated, island_generated,
    monument_generated, skyscraper_generated,
};

pub(super) fn models() -> impl Iterator<Item = &'static Landmark> {
    heritage_generated::HERITAGE_MODELS
        .iter()
        .chain(skyscraper_generated::SKYSCRAPER_MODELS)
        .chain(monument_generated::MONUMENT_MODELS)
        .chain(island_generated::ISLAND_MODELS)
}

macro_rules! patches {
    ($name:ident, $ty:ty) => {
        pub(super) fn $name(name: &str) -> &'static [$ty] {
            let island = island_generated::$name(name);
            if !island.is_empty() {
                return island;
            }
            let monument = monument_generated::$name(name);
            if !monument.is_empty() {
                return monument;
            }
            let heritage = heritage_generated::$name(name);
            if heritage.is_empty() {
                skyscraper_generated::$name(name)
            } else {
                heritage
            }
        }
    };
}

patches!(terrain_flatten, TerrainFlatten);
patches!(terrain_cuts, TerrainCut);
patches!(terrain_grades, TerrainCut);
patches!(water_cuts, TerrainCut);

pub(super) fn source_exclusion_polygons(name: &str) -> Option<&'static [&'static [[f64; 2]]]> {
    island_generated::source_exclusion_polygons(name)
        .or_else(|| heritage_generated::source_exclusion_polygons(name))
        .or_else(|| monument_generated::source_exclusion_polygons(name))
        .or_else(|| skyscraper_generated::source_exclusion_polygons(name))
}

pub(super) fn road_exclusion_polygons(name: &str) -> Option<&'static [&'static [[f64; 2]]]> {
    island_generated::road_exclusion_polygons(name)
        .or_else(|| heritage_generated::road_exclusion_polygons(name))
        .or_else(|| monument_generated::road_exclusion_polygons(name))
        .or_else(|| skyscraper_generated::road_exclusion_polygons(name))
}

pub(super) fn max_source_building_span(name: &str) -> Option<f64> {
    island_generated::max_source_building_span(name)
        .or_else(|| heritage_generated::max_source_building_span(name))
        .or_else(|| monument_generated::max_source_building_span(name))
        .or_else(|| skyscraper_generated::max_source_building_span(name))
}
