use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use vearth_data::place_search::{
    BuildMetadata, BuildRecord, PlaceKind, SearchIndex, encode, normalize,
};

const DEFAULT_CURATED: &str = "crates/vearth-data/assets/curated-places.tsv";
const DEFAULT_OUTPUT: &str = "crates/vearth-data/assets/place-search-v1.bin";
const DEFAULT_LONG_TAIL_PERMYRIAD: u32 = 2_500;

#[derive(Clone, Debug)]
struct GeoRow {
    geoname_id: u32,
    name: String,
    ascii_name: String,
    latitude_deg: f64,
    longitude_deg: f64,
    feature_code: String,
    country_code: String,
    admin1_code: String,
    population: u64,
    primary_tier: bool,
}

#[derive(Default)]
struct EnglishNames {
    preferred: BTreeSet<String>,
    ordinary: BTreeSet<String>,
    short: BTreeSet<String>,
}

impl EnglishNames {
    fn display_name<'a>(&'a self, fallback: &'a str) -> &'a str {
        self.preferred
            .iter()
            .next()
            .or_else(|| self.ordinary.iter().next())
            .map_or(fallback, String::as_str)
    }

    fn aliases(&self) -> impl Iterator<Item = &String> {
        self.preferred
            .iter()
            .chain(&self.ordinary)
            .chain(&self.short)
    }
}

/// Writes the browser's place-search shards (see
/// `vearth_data::geocode::shards`) as `<name>.bin` files, replacing the
/// output directory.
pub fn shard(argv: &[String]) -> Result<(), Box<dyn Error>> {
    let value = |flag: &str| {
        argv.iter()
            .position(|argument| argument == flag)
            .and_then(|index| argv.get(index + 1))
            .cloned()
    };
    let input = PathBuf::from(value("--input").unwrap_or_else(|| DEFAULT_OUTPUT.into()));
    let output =
        PathBuf::from(value("--output").ok_or("shard-place-search requires --output DIR")?);
    let bytes = std::fs::read(&input)?;
    let index = SearchIndex::open(&bytes)?;
    index.verify_checksum()?;
    let shards = vearth_data::geocode::shards(&index)?;
    if output.exists() {
        std::fs::remove_dir_all(&output)?;
    }
    std::fs::create_dir_all(&output)?;
    let mut total = 0;
    let mut largest = (0, String::new());
    for (name, shard) in &shards {
        std::fs::write(output.join(format!("{name}.bin")), shard)?;
        total += shard.len();
        largest = largest.max((shard.len(), name.clone()));
    }
    log::info!(
        "wrote {} place-search shards to {}: {:.1} MiB, largest {} at {:.1} MiB",
        shards.len(),
        output.display(),
        total as f64 / 1048576.0,
        largest.1,
        largest.0 as f64 / 1048576.0
    );
    Ok(())
}

pub fn pack(argv: &[String]) -> Result<(), Box<dyn Error>> {
    let value = |flag: &str| {
        argv.iter()
            .position(|argument| argument == flag)
            .and_then(|index| argv.get(index + 1))
            .cloned()
    };
    let cities = PathBuf::from(
        value("--cities").ok_or("pack-place-search requires --cities cities500.txt")?,
    );
    let country_info = PathBuf::from(
        value("--country-info")
            .ok_or("pack-place-search requires --country-info countryInfo.txt")?,
    );
    let admin1 = PathBuf::from(
        value("--admin1").ok_or("pack-place-search requires --admin1 admin1CodesASCII.txt")?,
    );
    let all_countries = value("--all-countries").map(PathBuf::from);
    let alternate_names = value("--alternate-names").map(PathBuf::from);
    let curated = PathBuf::from(value("--curated").unwrap_or_else(|| DEFAULT_CURATED.into()));
    let output = PathBuf::from(value("--output").unwrap_or_else(|| DEFAULT_OUTPUT.into()));
    let dataset_date = value("--dataset-date").unwrap_or_else(|| "unknown".into());
    let sample_permyriad: u32 = value("--long-tail-sample-permyriad")
        .unwrap_or_else(|| DEFAULT_LONG_TAIL_PERMYRIAD.to_string())
        .parse()?;
    if sample_permyriad > 10_000 {
        return Err("--long-tail-sample-permyriad must be 0..=10000".into());
    }
    let include_subdivisions = argv.iter().any(|value| value == "--include-subdivisions");

    let countries = read_country_info(&country_info)?;
    let admins = read_admin1(&admin1)?;
    let mut rows = BTreeMap::<u32, GeoRow>::new();
    read_geonames(&cities, true, |row| {
        rows.insert(row.geoname_id, row);
    })?;
    let primary_ids: BTreeSet<u32> = rows.keys().copied().collect();
    let representative = all_countries.is_none();
    if let Some(path) = &all_countries {
        read_geonames(path, false, |mut row| {
            if primary_ids.contains(&row.geoname_id) {
                row.primary_tier = true;
            }
            if select_full_record(&row, sample_permyriad, include_subdivisions) {
                rows.insert(row.geoname_id, row);
            }
        })?;
    }

    let rich_ids: BTreeSet<u32> = rows
        .values()
        .filter(|row| row.primary_tier || row.population > 0 || is_seat(&row.feature_code))
        .map(|row| row.geoname_id)
        .collect();
    let english = alternate_names
        .as_deref()
        .map(|path| read_english_names(path, &rich_ids))
        .transpose()?
        .unwrap_or_default();

    let mut records = Vec::with_capacity(rows.len() + vearth_gen::landmarks::LANDMARKS.len() + 64);
    for row in rows.into_values() {
        let english_names = english.get(&row.geoname_id);
        let fallback = if row.name.trim().is_empty() {
            row.ascii_name.as_str()
        } else {
            row.name.as_str()
        };
        let display_name = english_names
            .map(|names| names.display_name(fallback))
            .unwrap_or(fallback)
            .to_owned();
        let mut aliases = BTreeSet::new();
        if normalize(&row.ascii_name) != normalize(&display_name) && !row.ascii_name.is_empty() {
            aliases.insert(row.ascii_name.clone());
        }
        if normalize(&row.name) != normalize(&display_name) && !row.name.is_empty() {
            aliases.insert(row.name.clone());
        }
        if let Some(names) = english_names {
            aliases.extend(
                names
                    .aliases()
                    .filter(|alias| normalize(alias) != normalize(&display_name))
                    .cloned(),
            );
        }
        let country = countries
            .get(&row.country_code)
            .cloned()
            .unwrap_or_else(|| row.country_code.clone());
        let admin_region = admins
            .get(&format!("{}.{}", row.country_code, row.admin1_code))
            .cloned()
            .unwrap_or_default();
        let kind = PlaceKind::from_feature_code(&row.feature_code, row.population);
        records.push(BuildRecord {
            geoname_id: row.geoname_id,
            display_name,
            aliases: aliases.into_iter().collect(),
            latitude_deg: row.latitude_deg,
            longitude_deg: row.longitude_deg,
            country,
            admin_region,
            kind,
            population: row.population,
            rank_boost: 0,
            view_altitude_m: None,
            curated: false,
            administrative_seat: is_seat(&row.feature_code),
            primary_tier: row.primary_tier,
        });
    }

    let curated_rows = if curated.exists() {
        read_curated(&curated)?
    } else {
        log::warn!(
            "optional curated place file {} is absent; continuing with LANDMARKS",
            curated.display()
        );
        Vec::new()
    };
    append_curated(&mut records, curated_rows)?;
    let source = if representative {
        "GeoNames cities500 + filtered English alternateNamesV2 + Low Poly Earth curated places"
    } else {
        "GeoNames cities500/allCountries + filtered English alternateNamesV2 + Low Poly Earth curated places"
    };
    let bytes = encode(
        records,
        BuildMetadata {
            dataset_date: &dataset_date,
            source,
            representative,
        },
    )?;
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output, &bytes)?;
    let index = SearchIndex::open(&bytes)?;
    index.verify_checksum()?;
    let metadata = index.metadata();
    println!(
        "place search: records={} aliases={} bytes={} ({:.2} MiB) representative={} date={}",
        metadata.record_count,
        metadata.alias_count,
        metadata.installed_bytes,
        metadata.installed_bytes as f64 / (1024.0 * 1024.0),
        metadata.representative,
        metadata.dataset_date
    );
    Ok(())
}

fn read_geonames(
    path: &Path,
    primary_tier: bool,
    mut emit: impl FnMut(GeoRow),
) -> Result<(), Box<dyn Error>> {
    for (line_number, line) in BufReader::new(File::open(path)?).lines().enumerate() {
        let line = line?;
        if line.is_empty() {
            continue;
        }
        let columns: Vec<&str> = line.split('\t').collect();
        if columns.len() < 19 {
            return Err(format!(
                "{}:{}: expected 19 GeoNames columns, got {}",
                path.display(),
                line_number + 1,
                columns.len()
            )
            .into());
        }
        if columns[6] != "P" {
            continue;
        }
        emit(GeoRow {
            geoname_id: columns[0].parse()?,
            name: columns[1].to_owned(),
            ascii_name: columns[2].to_owned(),
            latitude_deg: columns[4].parse()?,
            longitude_deg: columns[5].parse()?,
            feature_code: columns[7].to_owned(),
            country_code: columns[8].to_owned(),
            admin1_code: columns[10].to_owned(),
            population: columns[14].parse().unwrap_or(0),
            primary_tier,
        });
    }
    Ok(())
}

fn select_full_record(row: &GeoRow, sample_permyriad: u32, include_subdivisions: bool) -> bool {
    if row.primary_tier {
        return true;
    }
    if matches!(row.feature_code.as_str(), "PPLQ" | "PPLW" | "PPLH") {
        return false;
    }
    if row.feature_code == "PPLX" && !include_subdivisions {
        return false;
    }
    if row.population > 0 || is_seat(&row.feature_code) {
        return true;
    }
    if matches!(row.feature_code.as_str(), "PPLL" | "PPLF") {
        return true;
    }
    row.feature_code == "PPL"
        && stable_sample(row.geoname_id, row.country_code.as_bytes()) < sample_permyriad
}

fn stable_sample(id: u32, country: &[u8]) -> u32 {
    let mut hash = 0x811c9dc5_u32 ^ id;
    for byte in country {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x01000193);
    }
    hash % 10_000
}

fn is_seat(feature_code: &str) -> bool {
    matches!(feature_code, "PPLC" | "PPLA" | "PPLA2" | "PPLA3" | "PPLA4")
}

fn read_country_info(path: &Path) -> Result<HashMap<String, String>, Box<dyn Error>> {
    let mut output = HashMap::new();
    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let columns: Vec<&str> = line.split('\t').collect();
        if columns.len() >= 5 {
            output.insert(columns[0].to_owned(), columns[4].to_owned());
        }
    }
    Ok(output)
}

fn read_admin1(path: &Path) -> Result<HashMap<String, String>, Box<dyn Error>> {
    let mut output = HashMap::new();
    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        let columns: Vec<&str> = line.split('\t').collect();
        if columns.len() >= 2 {
            output.insert(columns[0].to_owned(), columns[1].to_owned());
        }
    }
    Ok(output)
}

fn read_english_names(
    path: &Path,
    rich_ids: &BTreeSet<u32>,
) -> Result<HashMap<u32, EnglishNames>, Box<dyn Error>> {
    let mut output = HashMap::<u32, EnglishNames>::new();
    for (line_number, line) in BufReader::new(File::open(path)?).lines().enumerate() {
        let line = line?;
        let columns: Vec<&str> = line.split('\t').collect();
        if columns.len() < 8 {
            return Err(format!(
                "{}:{}: expected at least 8 alternateNamesV2 columns",
                path.display(),
                line_number + 1
            )
            .into());
        }
        if columns[2] != "en" || columns[7] == "1" || columns.get(6) == Some(&"1") {
            continue;
        }
        let geoname_id: u32 = columns[1].parse()?;
        if !rich_ids.contains(&geoname_id) {
            continue;
        }
        let name = columns[3].trim();
        if name.len() < 2 || name.len() > 120 || !name.chars().any(char::is_alphabetic) {
            continue;
        }
        let names = output.entry(geoname_id).or_default();
        if columns.get(4) == Some(&"1") {
            names.preferred.insert(name.to_owned());
        } else if columns.get(5) == Some(&"1") {
            names.short.insert(name.to_owned());
        } else {
            names.ordinary.insert(name.to_owned());
        }
    }
    Ok(output)
}

#[derive(Clone, Debug)]
struct CuratedRow {
    display_name: String,
    aliases: Vec<String>,
    latitude_deg: Option<f64>,
    longitude_deg: Option<f64>,
    country: String,
    admin_region: String,
    rank_boost: i16,
    view_altitude_m: u32,
    source_landmark: String,
}

fn read_curated(path: &Path) -> Result<Vec<CuratedRow>, Box<dyn Error>> {
    let mut output = Vec::new();
    for (line_number, line) in BufReader::new(File::open(path)?).lines().enumerate() {
        let line = line?;
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let columns: Vec<&str> = line.split('\t').collect();
        if !(8..=9).contains(&columns.len()) {
            return Err(format!(
                "{}:{}: expected 8 or 9 curated columns, got {}",
                path.display(),
                line_number + 1,
                columns.len()
            )
            .into());
        }
        output.push(CuratedRow {
            display_name: columns[0].to_owned(),
            aliases: columns[1]
                .split('|')
                .filter(|alias| !alias.is_empty())
                .map(str::to_owned)
                .collect(),
            latitude_deg: (!columns[2].is_empty())
                .then(|| columns[2].parse())
                .transpose()?,
            longitude_deg: (!columns[3].is_empty())
                .then(|| columns[3].parse())
                .transpose()?,
            country: columns[4].to_owned(),
            admin_region: columns[5].to_owned(),
            rank_boost: columns[6].parse()?,
            view_altitude_m: columns[7].parse()?,
            source_landmark: columns.get(8).copied().unwrap_or_default().to_owned(),
        });
    }
    Ok(output)
}

fn append_curated(
    records: &mut Vec<BuildRecord>,
    curated_rows: Vec<CuratedRow>,
) -> Result<(), Box<dyn Error>> {
    let mut by_source = HashMap::new();
    let mut coordinate_rows = Vec::new();
    for row in curated_rows {
        if row.source_landmark.is_empty() {
            coordinate_rows.push(row);
        } else if by_source.insert(row.source_landmark.clone(), row).is_some() {
            return Err("duplicate curated source_landmark".into());
        }
    }
    let mut next_id = 0xf000_0000_u32;
    for landmark in vearth_gen::landmarks::LANDMARKS {
        let row = by_source.remove(landmark.name);
        let display_name = row
            .as_ref()
            .map(|value| value.display_name.clone())
            .unwrap_or_else(|| title_landmark(landmark.name));
        let aliases = row
            .as_ref()
            .map(|value| value.aliases.clone())
            .unwrap_or_default();
        records.push(BuildRecord {
            geoname_id: next_id,
            display_name,
            aliases,
            latitude_deg: landmark.lat_deg,
            longitude_deg: landmark.lon_deg,
            country: row
                .as_ref()
                .map(|value| value.country.clone())
                .unwrap_or_default(),
            admin_region: row
                .as_ref()
                .map(|value| value.admin_region.clone())
                .unwrap_or_default(),
            kind: PlaceKind::Landmark,
            population: 0,
            rank_boost: row.as_ref().map_or(100, |value| value.rank_boost),
            view_altitude_m: Some(row.as_ref().map_or_else(
                || landmark_view_altitude(landmark),
                |value| value.view_altitude_m,
            )),
            curated: true,
            administrative_seat: false,
            primary_tier: true,
        });
        next_id += 1;
    }
    if let Some((unknown, _)) = by_source.into_iter().next() {
        return Err(format!("curated source_landmark {unknown:?} is not in LANDMARKS").into());
    }
    for row in coordinate_rows {
        records.push(BuildRecord {
            geoname_id: next_id,
            display_name: row.display_name,
            aliases: row.aliases,
            latitude_deg: row
                .latitude_deg
                .ok_or("curated row without source_landmark requires latitude")?,
            longitude_deg: row
                .longitude_deg
                .ok_or("curated row without source_landmark requires longitude")?,
            country: row.country,
            admin_region: row.admin_region,
            kind: PlaceKind::Landmark,
            population: 0,
            rank_boost: row.rank_boost,
            view_altitude_m: Some(row.view_altitude_m),
            curated: true,
            administrative_seat: false,
            primary_tier: true,
        });
        next_id += 1;
    }
    Ok(())
}

fn title_landmark(slug: &str) -> String {
    slug.split('-')
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        })
        .collect::<Vec<String>>()
        .join(" ")
}

fn landmark_view_altitude(landmark: &vearth_gen::landmarks::Landmark) -> u32 {
    let width = landmark.bbox_max[0] - landmark.bbox_min[0];
    let height = landmark.bbox_max[1] - landmark.bbox_min[1];
    let depth = landmark.bbox_max[2] - landmark.bbox_min[2];
    (width.max(height).max(depth) * 1.8)
        .round()
        .clamp(300.0, 12_000.0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_selection_excludes_dead_history_and_subdivisions() {
        let row = |code: &str, population| GeoRow {
            geoname_id: 42,
            name: "Test".into(),
            ascii_name: "Test".into(),
            latitude_deg: 0.0,
            longitude_deg: 0.0,
            feature_code: code.into(),
            country_code: "AU".into(),
            admin1_code: "02".into(),
            population,
            primary_tier: false,
        };
        assert!(!select_full_record(&row("PPLQ", 10), 10_000, false));
        assert!(!select_full_record(&row("PPLX", 10), 10_000, false));
        assert!(select_full_record(&row("PPLX", 10), 10_000, true));
        assert!(select_full_record(&row("PPLA4", 0), 0, false));
        assert!(select_full_record(&row("PPLL", 0), 0, false));
    }

    #[test]
    fn landmark_title_is_readable() {
        assert_eq!(title_landmark("taj-mahal"), "Taj Mahal");
        assert_eq!(title_landmark("sydney-opera-house"), "Sydney Opera House");
    }

    #[test]
    fn landmarks_remain_available_without_optional_curated_rows() {
        let mut records = Vec::new();
        append_curated(&mut records, Vec::new()).unwrap();
        assert_eq!(records.len(), vearth_gen::landmarks::LANDMARKS.len());
        assert!(
            records
                .iter()
                .any(|record| record.display_name == "Eiffel Tower")
        );
    }
}
