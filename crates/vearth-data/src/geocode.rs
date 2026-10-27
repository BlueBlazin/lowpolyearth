//! Place search over the bundled GeoNames-derived index, with no search
//! service or network fallback.
//!
//! The native app compiles the whole index in and searches it in place. The
//! browser downloads the same index split into shards ([`shards`]) and
//! fetches only those a query needs, with identical results.

use crate::DataError;
#[cfg(not(target_arch = "wasm32"))]
use crate::place_search::ArtifactMetadata;
use crate::place_search::{
    self, MapPlace, PlaceKind, SearchIndex, SearchResult, TYPO_MINIMUM_BYTES,
};

/// Native only: the browser loads [`shards`] of it instead.
#[cfg(not(target_arch = "wasm32"))]
fn place_search_bytes() -> &'static [u8] {
    include_bytes!("../assets/place-search-v1.bin")
}

/// Results per query.
const RESULTS: usize = 6;

/// The population threshold of the map shard; [`ShardedIndex::map_places`]
/// serves this threshold or higher.
pub const MAP_SHARD_MINIMUM_POPULATION: u32 = 3_000;

const CONTEXT_SHARD: &str = "context";
const MAP_SHARD: &str = "map";

#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub display_name: String,
    pub lat_deg: f64,
    pub lon_deg: f64,
    /// Fly-to altitude with close settlement framing or an authored landmark view.
    pub view_altitude_m: f64,
}

#[derive(Clone, Copy)]
pub struct Geocoder {
    #[cfg(not(target_arch = "wasm32"))]
    index: SearchIndex<'static>,
    #[cfg(target_arch = "wasm32")]
    index: ShardedIndex,
}

impl Default for Geocoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Geocoder {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new() -> Self {
        Self::from_static_bytes(place_search_bytes())
            .expect("the checked-in place-search asset must be valid")
    }

    /// Searches the shards the site publishes under `generated/place-search/`.
    #[cfg(target_arch = "wasm32")]
    pub fn new() -> Self {
        Self {
            index: ShardedIndex::new(|name| {
                vearth_platform::assets::try_load(&format!("generated/place-search/{name}.bin"))
                    .map_err(|error| {
                        DataError::Decode(format!("place search shard {name}: {error}"))
                    })
            }),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_static_bytes(bytes: &'static [u8]) -> Result<Self, DataError> {
        Ok(Self {
            index: SearchIndex::open(bytes)?,
        })
    }

    /// Blocking search, run on a worker thread. It never contacts a search
    /// service; in the browser it may download index shards.
    pub fn search(&self, query: &str) -> Result<Vec<Place>, DataError> {
        self.index
            .search(query, RESULTS)?
            .into_iter()
            .map(|result| Ok(place(result)))
            .collect()
    }

    /// Build the map's offline settlement-label catalog. This can scan the
    /// fixed-width record table, so callers should run it on a worker thread.
    pub fn map_places(&self, minimum_population: u32) -> Result<Vec<MapPlace>, DataError> {
        self.index.map_places(minimum_population)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn metadata(&self) -> ArtifactMetadata<'static> {
        self.index.metadata()
    }
}

/// Loads a shard by name: `Ok(None)` when [`shards`] produced no such shard.
pub type ShardLoader = fn(&str) -> Result<Option<&'static [u8]>, DataError>;

/// The place index split by [`shards`], loaded one shard at a time. Every
/// query reads at most three shards and returns exactly what
/// [`SearchIndex::search`] returns on the whole index.
#[derive(Clone, Copy)]
pub struct ShardedIndex {
    load: ShardLoader,
}

impl ShardedIndex {
    pub fn new(load: ShardLoader) -> Self {
        Self { load }
    }

    fn shard(&self, name: &str) -> Result<Option<SearchIndex<'static>>, DataError> {
        (self.load)(name)?.map(SearchIndex::open).transpose()
    }

    fn required_shard(&self, name: &str) -> Result<SearchIndex<'static>, DataError> {
        self.shard(name)?
            .ok_or_else(|| DataError::Decode(format!("place search: no {name} shard")))
    }

    /// Mirrors [`SearchIndex::search`]: name prefixes, then one-edit typos
    /// sharing the first character, then country and region labels.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, DataError> {
        let normalized = place_search::normalize(query);
        if normalized.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let mut names = place_search::first_char_shard(&normalized);
        if let Some(two_char) = place_search::two_char_shard(&normalized) {
            if let Some(shard) = self.shard(&two_char)?
                && let Some(results) = shard.search_name_prefixes(query, limit)?
            {
                return Ok(results);
            }
            if normalized.len() < TYPO_MINIMUM_BYTES {
                names = None;
            }
        }
        if let Some(names) = names
            && let Some(shard) = self.shard(&names)?
            && let Some(results) = shard.search_names(query, limit)?
        {
            return Ok(results);
        }
        self.required_shard(CONTEXT_SHARD)?
            .search_by_context(query, limit)
    }

    pub fn map_places(&self, minimum_population: u32) -> Result<Vec<MapPlace>, DataError> {
        if minimum_population < MAP_SHARD_MINIMUM_POPULATION {
            return Err(DataError::Decode(format!(
                "place search: the map shard starts at population {MAP_SHARD_MINIMUM_POPULATION}"
            )));
        }
        self.required_shard(MAP_SHARD)?
            .map_places(minimum_population)
    }
}

/// Splits `index` into the named shards [`ShardedIndex`] searches: aliases by
/// first character and, where both are ASCII letters or digits, by first
/// two characters; the best records per context label; and the map places.
pub fn shards(index: &SearchIndex<'_>) -> Result<Vec<(String, Vec<u8>)>, DataError> {
    let mut groups = std::collections::BTreeMap::<String, Vec<usize>>::new();
    for (position, key) in index.alias_keys().enumerate() {
        let key = key?;
        let first = place_search::first_char_shard(key)
            .ok_or_else(|| DataError::Decode("place search: empty alias key".into()))?;
        groups.entry(first).or_default().push(position);
        if let Some(two) = place_search::two_char_shard(key) {
            groups.entry(two).or_default().push(position);
        }
    }
    let mut shards = Vec::with_capacity(groups.len() + 2);
    for (name, positions) in groups {
        shards.push((name, index.names_subset(&positions)?));
    }
    shards.push((CONTEXT_SHARD.into(), index.context_subset(RESULTS)?));
    shards.push((
        MAP_SHARD.into(),
        index.map_subset(MAP_SHARD_MINIMUM_POPULATION)?,
    ));
    Ok(shards)
}

fn place(result: SearchResult) -> Place {
    // The index frames whole settlements. Search arrivals use a closer view,
    // retaining the small-place floor and explicit landmark framing.
    let view_altitude_m = match result.kind {
        PlaceKind::Landmark => f64::from(result.view_altitude_m),
        _ => (f64::from(result.view_altitude_m) * 0.1).max(1_800.0),
    };
    Place {
        display_name: result.display_name,
        lat_deg: result.latitude_deg,
        lon_deg: result.longitude_deg,
        view_altitude_m,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics;
    use crate::place_search::PlaceKind;

    #[test]
    fn bundled_search_is_offline_and_has_valid_metadata() {
        let before = metrics::snapshot();
        let geocoder = Geocoder::new();
        let results = geocoder.search("Sydney").unwrap();
        let after = metrics::snapshot();
        assert!(!results.is_empty());
        assert_eq!(results[0].display_name.split(',').next(), Some("Sydney"));
        assert_eq!(before, after);
        assert!(geocoder.metadata().installed_bytes <= crate::place_search::MAX_INSTALLED_BYTES);
        SearchIndex::open(place_search_bytes())
            .unwrap()
            .verify_checksum()
            .unwrap();
    }

    #[test]
    fn bundled_golden_searches_are_stable() {
        let geocoder = Geocoder::new();
        for (query, expected) in [
            ("Taj Mahal", "Taj Mahal"),
            ("Farm Tomita", "Farm Tomita"),
            ("Sydney", "Sydney"),
            ("Tokyo", "Tokyo"),
            ("Munich", "Munich"),
            ("Florence", "Florence"),
            ("Hallstatt", "Hallstatt"),
            ("Sydney Harbour Bridge", "Sydney Harbour Bridge"),
        ] {
            let results = geocoder.search(query).unwrap();
            assert!(
                results
                    .first()
                    .is_some_and(|place| place.display_name.starts_with(expected)),
                "{query:?}: {results:?}"
            );
        }
        let paris = geocoder.search("Paris").unwrap();
        assert!(
            paris
                .iter()
                .any(|place| place.display_name.contains("France")),
            "{paris:?}"
        );
        assert!(
            paris
                .iter()
                .any(|place| place.display_name.contains("Texas")),
            "{paris:?}"
        );

        let faroe = geocoder.search("Faroe Islands").unwrap();
        assert!(
            faroe.first().is_some_and(|place| {
                place.display_name.starts_with("Tórshavn")
                    && place.display_name.contains("Faroe Islands")
            }),
            "{faroe:?}"
        );
    }

    #[test]
    fn search_arrivals_zoom_into_cities_and_preserve_landmark_views() {
        let geocoder = Geocoder::new();
        for (query, altitude_m) in [
            ("Rotterdam", 3_200.0),
            ("Tokyo", 9_000.0),
            ("Hallstatt", 1_800.0),
            ("Taj Mahal", 1_200.0),
            ("Farm Tomita", 700.0),
        ] {
            let result = geocoder.search(query).unwrap().remove(0);
            assert_eq!(result.view_altitude_m, altitude_m, "{query}");
        }
    }

    fn bundled_shard(name: &str) -> Result<Option<&'static [u8]>, DataError> {
        static SHARDS: std::sync::OnceLock<std::collections::HashMap<String, Vec<u8>>> =
            std::sync::OnceLock::new();
        let shards = SHARDS.get_or_init(|| {
            shards(&SearchIndex::open(place_search_bytes()).unwrap())
                .unwrap()
                .into_iter()
                .collect()
        });
        Ok(shards.get(name).map(Vec::as_slice))
    }

    #[test]
    fn sharded_search_matches_the_whole_index() {
        let whole = SearchIndex::open(place_search_bytes()).unwrap();
        let sharded = ShardedIndex::new(bundled_shard);
        let mut queries: Vec<String> = [
            "Paris",
            "paris france",
            "Sydney Harbour Bridge",
            "São Paulo",
            "Zürich",
            "東京",
            "Москва",
            "القاهرة",
            "Tokio",
            "Munchen",
            "Parus",
            "Syndey",
            "France",
            "Texas",
            "Faroe Islands",
            "Bavaria",
            "Tasmania",
            "new",
            "new y",
            "st",
            "s",
            "x",
            "q",
            "zz",
            "zzzz",
            "1",
            "10",
            "'s-Hertogenbosch",
            "  ",
            "-",
            "Île-de-France",
            "Ōsaka",
            "qqqqq",
            "xyzzy",
            "Wales",
            "united",
        ]
        .map(str::to_owned)
        .into();
        for key in whole.alias_keys().step_by(9_973) {
            let key = key.unwrap();
            let chars: Vec<char> = key.chars().collect();
            for length in [1, 2, 3, 5, chars.len()] {
                queries.push(chars[..length.min(chars.len())].iter().collect());
            }
            if chars.len() >= 5 {
                let mut typo = chars.clone();
                typo.swap(2, 3);
                queries.push(typo.into_iter().collect());
                let mut dropped = chars.clone();
                dropped.remove(3);
                queries.push(dropped.into_iter().collect());
            }
        }
        for query in &queries {
            assert_eq!(
                sharded.search(query, RESULTS).unwrap(),
                whole.search(query, RESULTS).unwrap(),
                "{query:?}"
            );
        }
        assert_eq!(
            sharded.map_places(3_000).unwrap(),
            whole.map_places(3_000).unwrap()
        );
    }

    #[test]
    fn bundled_map_catalog_uses_primary_settlement_names_only() {
        let places = Geocoder::new().map_places(3_000).unwrap();
        assert!((10_000..300_000).contains(&places.len()));
        assert!(places.iter().any(|place| place.name == "Sydney"));
        assert!(places.iter().all(|place| {
            (place.population >= 3_000 || place.kind == PlaceKind::Capital)
                && matches!(
                    place.kind,
                    PlaceKind::City
                        | PlaceKind::Town
                        | PlaceKind::Village
                        | PlaceKind::Capital
                        | PlaceKind::AdministrativeSeat
                )
        }));
    }
}
