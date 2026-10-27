//! Compact, deterministic offline place-search artifact.
//!
//! The file is a small header followed by fixed-width place records, a
//! lexicographically sorted alias table, and one interned UTF-8 string pool.
//! Runtime lookups binary-search the alias table and borrow all strings from
//! the compiled asset. Opening the index therefore allocates no dataset-sized
//! structures and performs no I/O.

use crate::DataError;
use std::collections::{BTreeMap, HashMap};
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

pub const MAGIC: &[u8; 8] = b"VEARTHPS";
pub const FORMAT_VERSION: u32 = 1;
pub const HEADER_LEN: usize = 72;
pub const RECORD_LEN: usize = 40;
pub const ALIAS_LEN: usize = 12;
pub const MAX_INSTALLED_BYTES: usize = 150 * 1024 * 1024;

const FLAG_CURATED: u8 = 1 << 0;
const FLAG_SEAT: u8 = 1 << 1;
const FLAG_PRIMARY_TIER: u8 = 1 << 2;
const ALIAS_PRIMARY: u8 = 1 << 0;
const ALIAS_ENGLISH: u8 = 1 << 1;
const ARTIFACT_REPRESENTATIVE: u16 = 1 << 0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PlaceKind {
    City = 0,
    Town = 1,
    Village = 2,
    Locality = 3,
    Capital = 4,
    AdministrativeSeat = 5,
    Landmark = 6,
}

impl PlaceKind {
    pub fn from_feature_code(code: &str, population: u64) -> Self {
        match code {
            "PPLC" => Self::Capital,
            "PPLA" | "PPLA2" | "PPLA3" | "PPLA4" => Self::AdministrativeSeat,
            "PPLF" | "PPLL" => Self::Locality,
            _ if population >= 100_000 => Self::City,
            _ if population >= 10_000 => Self::Town,
            _ => Self::Village,
        }
    }

    fn decode(value: u8) -> Result<Self, DataError> {
        match value {
            0 => Ok(Self::City),
            1 => Ok(Self::Town),
            2 => Ok(Self::Village),
            3 => Ok(Self::Locality),
            4 => Ok(Self::Capital),
            5 => Ok(Self::AdministrativeSeat),
            6 => Ok(Self::Landmark),
            _ => Err(DataError::Decode(format!(
                "place search: unknown kind {value}"
            ))),
        }
    }
}

#[derive(Clone, Debug)]
pub struct BuildRecord {
    pub geoname_id: u32,
    pub display_name: String,
    pub aliases: Vec<String>,
    pub latitude_deg: f64,
    pub longitude_deg: f64,
    pub country: String,
    pub admin_region: String,
    pub kind: PlaceKind,
    pub population: u64,
    pub rank_boost: i16,
    pub view_altitude_m: Option<u32>,
    pub curated: bool,
    pub administrative_seat: bool,
    pub primary_tier: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct BuildMetadata<'a> {
    pub dataset_date: &'a str,
    pub source: &'a str,
    pub representative: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtifactMetadata<'a> {
    pub dataset_date: &'a str,
    pub source: &'a str,
    pub representative: bool,
    pub record_count: usize,
    pub alias_count: usize,
    pub installed_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct EncodedString {
    offset: u32,
    len: u16,
}

#[derive(Default)]
struct StringPool {
    bytes: Vec<u8>,
    entries: HashMap<String, EncodedString>,
}

impl StringPool {
    fn intern(&mut self, value: &str) -> Result<EncodedString, DataError> {
        if let Some(encoded) = self.entries.get(value) {
            return Ok(*encoded);
        }
        let len = u16::try_from(value.len())
            .map_err(|_| DataError::Decode("place search: string exceeds 65535 bytes".into()))?;
        let offset = u32::try_from(self.bytes.len())
            .map_err(|_| DataError::Decode("place search: string pool exceeds 4 GiB".into()))?;
        self.bytes.extend_from_slice(value.as_bytes());
        let encoded = EncodedString { offset, len };
        self.entries.insert(value.to_owned(), encoded);
        Ok(encoded)
    }
}

#[derive(Clone, Debug)]
struct AliasBuild {
    key: String,
    record_id: u32,
    flags: u8,
}

/// Normalized queries shorter than this match only by prefix.
pub const TYPO_MINIMUM_BYTES: usize = 4;

/// Shard buckets for names whose first character is not an ASCII letter or
/// digit. Typo tolerance never changes the first character, so any grouping
/// by first character keeps searches exact.
const FIRST_CHAR_BUCKETS: u32 = 64;

/// The name shard holding every alias that starts with the first character
/// of `normalized` (a [`normalize`]d query or alias key).
pub fn first_char_shard(normalized: &str) -> Option<String> {
    let first = normalized.chars().next()?;
    Some(if first.is_ascii_alphanumeric() {
        format!("c1-{first}")
    } else {
        format!("c1-x{:02}", u32::from(first) % FIRST_CHAR_BUCKETS)
    })
}

/// The smaller name shard holding every alias that starts with the first
/// two characters of `normalized`, when both are ASCII letters or digits.
pub fn two_char_shard(normalized: &str) -> Option<String> {
    let mut chars = normalized.chars();
    let (first, second) = (chars.next()?, chars.next()?);
    (first.is_ascii_alphanumeric() && second.is_ascii_alphanumeric())
        .then(|| format!("c2-{first}{second}"))
}

/// Normalize English search text while preserving a readable display name
/// elsewhere. NFKD folds compatibility characters; combining marks,
/// punctuation, repeated whitespace, and case are removed deterministically.
pub fn normalize(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut pending_space = false;
    for ch in value.nfkd() {
        if is_combining_mark(ch) {
            continue;
        }
        for lower in ch.to_lowercase() {
            if lower.is_alphanumeric() {
                if pending_space && !out.is_empty() {
                    out.push(' ');
                }
                pending_space = false;
                out.push(lower);
            } else {
                pending_space = true;
            }
        }
    }
    out
}

pub fn derive_view_altitude_m(kind: PlaceKind, population: u64) -> u32 {
    if kind == PlaceKind::Landmark {
        return 900;
    }
    let population_altitude = match population {
        5_000_000.. => 90_000,
        1_000_000.. => 55_000,
        250_000.. => 32_000,
        50_000.. => 18_000,
        10_000.. => 10_000,
        1_000.. => 5_000,
        1.. => 2_500,
        _ => 1_800,
    };
    match kind {
        PlaceKind::Capital => population_altitude.max(35_000),
        PlaceKind::AdministrativeSeat => population_altitude.max(8_000),
        PlaceKind::Locality => population_altitude.min(8_000),
        _ => population_altitude,
    }
}

/// Encode records into the checked-in runtime format.
pub fn encode(
    mut records: Vec<BuildRecord>,
    metadata: BuildMetadata<'_>,
) -> Result<Vec<u8>, DataError> {
    records.sort_by(|a, b| {
        a.geoname_id
            .cmp(&b.geoname_id)
            .then_with(|| normalize(&a.display_name).cmp(&normalize(&b.display_name)))
            .then_with(|| a.latitude_deg.total_cmp(&b.latitude_deg))
            .then_with(|| a.longitude_deg.total_cmp(&b.longitude_deg))
    });

    let record_count = u32::try_from(records.len())
        .map_err(|_| DataError::Decode("place search: too many records".into()))?;
    let mut pool = StringPool::default();
    let dataset_date = pool.intern(metadata.dataset_date)?;
    let source = pool.intern(metadata.source)?;
    let mut encoded_records = Vec::with_capacity(records.len() * RECORD_LEN);
    let mut aliases = Vec::new();

    for (record_index, record) in records.iter().enumerate() {
        if !(-90.0..=90.0).contains(&record.latitude_deg)
            || !(-180.0..=180.0).contains(&record.longitude_deg)
        {
            return Err(DataError::Decode(format!(
                "place search: invalid coordinate for {}",
                record.display_name
            )));
        }
        let display = pool.intern(&record.display_name)?;
        let country = pool.intern(&record.country)?;
        let admin = pool.intern(&record.admin_region)?;
        let country_len = u8::try_from(country.len).map_err(|_| {
            DataError::Decode(format!(
                "place search: country label too long for {}",
                record.display_name
            ))
        })?;
        let admin_len = u8::try_from(admin.len).map_err(|_| {
            DataError::Decode(format!(
                "place search: admin label too long for {}",
                record.display_name
            ))
        })?;
        let latitude_e7 = (record.latitude_deg * 10_000_000.0).round() as i32;
        let longitude_e7 = (record.longitude_deg * 10_000_000.0).round() as i32;
        let population = record.population.min(u32::MAX as u64) as u32;
        let view_altitude = record
            .view_altitude_m
            .unwrap_or_else(|| derive_view_altitude_m(record.kind, record.population));
        let mut flags = 0;
        if record.curated {
            flags |= FLAG_CURATED;
        }
        if record.administrative_seat {
            flags |= FLAG_SEAT;
        }
        if record.primary_tier {
            flags |= FLAG_PRIMARY_TIER;
        }
        put_u32(&mut encoded_records, display.offset);
        put_u32(&mut encoded_records, country.offset);
        put_u32(&mut encoded_records, admin.offset);
        put_i32(&mut encoded_records, latitude_e7);
        put_i32(&mut encoded_records, longitude_e7);
        put_u32(&mut encoded_records, population);
        put_u32(&mut encoded_records, view_altitude);
        put_u16(&mut encoded_records, display.len);
        encoded_records.push(country_len);
        encoded_records.push(admin_len);
        encoded_records.push(record.kind as u8);
        encoded_records.push(flags);
        put_i16(&mut encoded_records, record.rank_boost);
        put_u32(&mut encoded_records, record.geoname_id);

        let record_id = record_index as u32;
        let primary_key = normalize(&record.display_name);
        let mut unique = BTreeMap::<String, u8>::new();
        if !primary_key.is_empty() {
            unique.insert(primary_key, ALIAS_PRIMARY);
        }
        for alias in &record.aliases {
            let key = normalize(alias);
            if !key.is_empty() {
                unique
                    .entry(key)
                    .and_modify(|flags| *flags |= ALIAS_ENGLISH)
                    .or_insert(ALIAS_ENGLISH);
            }
        }
        for (key, flags) in unique {
            aliases.push(AliasBuild {
                key,
                record_id,
                flags,
            });
        }
    }
    aliases.sort_by(|a, b| {
        a.key
            .cmp(&b.key)
            .then_with(|| a.record_id.cmp(&b.record_id))
    });
    aliases.dedup_by(|a, b| a.key == b.key && a.record_id == b.record_id);

    let mut encoded_aliases = Vec::with_capacity(aliases.len() * ALIAS_LEN);
    for alias in &aliases {
        let key = pool.intern(&alias.key)?;
        put_u32(&mut encoded_aliases, key.offset);
        put_u32(&mut encoded_aliases, alias.record_id);
        put_u16(&mut encoded_aliases, key.len);
        encoded_aliases.push(alias.flags);
        encoded_aliases.push(0);
    }

    write_artifact(Sections {
        records: &encoded_records,
        record_count,
        aliases: &encoded_aliases,
        alias_count: aliases.len() as u32,
        pool: &pool,
        dataset_date,
        source,
        representative: metadata.representative,
    })
}

/// Encoded sections of one artifact, assembled by [`write_artifact`].
struct Sections<'p> {
    records: &'p [u8],
    record_count: u32,
    aliases: &'p [u8],
    alias_count: u32,
    pool: &'p StringPool,
    dataset_date: EncodedString,
    source: EncodedString,
    representative: bool,
}

fn write_artifact(sections: Sections<'_>) -> Result<Vec<u8>, DataError> {
    let Sections {
        records: encoded_records,
        record_count,
        aliases: encoded_aliases,
        alias_count,
        pool,
        dataset_date,
        source,
        representative,
    } = sections;
    let records_offset = HEADER_LEN;
    let aliases_offset = records_offset + encoded_records.len();
    let strings_offset = aliases_offset + encoded_aliases.len();
    let total_len = strings_offset
        .checked_add(pool.bytes.len())
        .ok_or_else(|| DataError::Decode("place search: artifact size overflow".into()))?;
    if total_len > MAX_INSTALLED_BYTES {
        return Err(DataError::Decode(format!(
            "place search: generated artifact is {total_len} bytes; budget is {MAX_INSTALLED_BYTES}"
        )));
    }
    let mut out = Vec::with_capacity(total_len);
    out.extend_from_slice(MAGIC);
    put_u32(&mut out, FORMAT_VERSION);
    put_u32(&mut out, HEADER_LEN as u32);
    put_u32(&mut out, record_count);
    put_u32(&mut out, alias_count);
    put_u32(&mut out, records_offset as u32);
    put_u32(&mut out, aliases_offset as u32);
    put_u32(&mut out, strings_offset as u32);
    put_u32(&mut out, pool.bytes.len() as u32);
    put_u32(&mut out, dataset_date.offset);
    put_u16(&mut out, dataset_date.len);
    put_u16(
        &mut out,
        if representative {
            ARTIFACT_REPRESENTATIVE
        } else {
            0
        },
    );
    put_u32(&mut out, source.offset);
    put_u16(&mut out, source.len);
    put_u16(&mut out, 0);
    put_u64(
        &mut out,
        fnv1a64(encoded_records, encoded_aliases, &pool.bytes),
    );
    put_u32(&mut out, total_len as u32);
    put_u32(&mut out, 0);
    debug_assert_eq!(out.len(), HEADER_LEN);
    out.extend_from_slice(encoded_records);
    out.extend_from_slice(encoded_aliases);
    out.extend_from_slice(&pool.bytes);
    Ok(out)
}

#[derive(Clone, Copy)]
pub struct SearchIndex<'a> {
    bytes: &'a [u8],
    record_count: usize,
    alias_count: usize,
    records_offset: usize,
    aliases_offset: usize,
    strings_offset: usize,
    strings_len: usize,
    dataset_date: EncodedString,
    source: EncodedString,
    representative: bool,
    expected_hash: u64,
}

#[derive(Clone, Copy, Debug)]
struct RecordRef<'a> {
    display_name: &'a str,
    country: &'a str,
    admin_region: &'a str,
    latitude_e7: i32,
    longitude_e7: i32,
    population: u32,
    view_altitude_m: u32,
    kind: PlaceKind,
    flags: u8,
    rank_boost: i16,
    geoname_id: u32,
}

#[derive(Clone, Copy, Debug)]
struct AliasRef<'a> {
    key: &'a str,
    record_id: usize,
    flags: u8,
}

#[derive(Clone, Copy, Debug)]
struct Candidate {
    record_id: usize,
    exact: bool,
    fuzzy: bool,
    alias_flags: u8,
}

impl<'a> SearchIndex<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<Self, DataError> {
        if bytes.len() < HEADER_LEN || &bytes[0..8] != MAGIC {
            return Err(DataError::Decode(
                "place search: missing or invalid magic".into(),
            ));
        }
        if read_u32(bytes, 8)? != FORMAT_VERSION {
            return Err(DataError::Decode(
                "place search: unsupported format version".into(),
            ));
        }
        if read_u32(bytes, 12)? as usize != HEADER_LEN {
            return Err(DataError::Decode(
                "place search: invalid header length".into(),
            ));
        }
        let record_count = read_u32(bytes, 16)? as usize;
        let alias_count = read_u32(bytes, 20)? as usize;
        let records_offset = read_u32(bytes, 24)? as usize;
        let aliases_offset = read_u32(bytes, 28)? as usize;
        let strings_offset = read_u32(bytes, 32)? as usize;
        let strings_len = read_u32(bytes, 36)? as usize;
        let dataset_date = EncodedString {
            offset: read_u32(bytes, 40)?,
            len: read_u16(bytes, 44)?,
        };
        let flags = read_u16(bytes, 46)?;
        let source = EncodedString {
            offset: read_u32(bytes, 48)?,
            len: read_u16(bytes, 52)?,
        };
        let expected_hash = read_u64(bytes, 56)?;
        let total_len = read_u32(bytes, 64)? as usize;
        let records_end = records_offset
            .checked_add(record_count.saturating_mul(RECORD_LEN))
            .ok_or_else(|| DataError::Decode("place search: record offset overflow".into()))?;
        let aliases_end = aliases_offset
            .checked_add(alias_count.saturating_mul(ALIAS_LEN))
            .ok_or_else(|| DataError::Decode("place search: alias offset overflow".into()))?;
        let strings_end = strings_offset
            .checked_add(strings_len)
            .ok_or_else(|| DataError::Decode("place search: string offset overflow".into()))?;
        if records_offset != HEADER_LEN
            || records_end != aliases_offset
            || aliases_end != strings_offset
            || strings_end != bytes.len()
            || total_len != bytes.len()
            || bytes.len() > MAX_INSTALLED_BYTES
        {
            return Err(DataError::Decode(
                "place search: corrupt section bounds".into(),
            ));
        }
        let index = Self {
            bytes,
            record_count,
            alias_count,
            records_offset,
            aliases_offset,
            strings_offset,
            strings_len,
            dataset_date,
            source,
            representative: flags & ARTIFACT_REPRESENTATIVE != 0,
            expected_hash,
        };
        index.string(dataset_date)?;
        index.string(source)?;
        if alias_count != 0 {
            index.alias(0)?;
            index.alias(alias_count - 1)?;
        }
        if record_count != 0 {
            index.record(0)?;
            index.record(record_count - 1)?;
        }
        Ok(index)
    }

    /// Full artifact integrity check for release verification. Ordinary open
    /// intentionally validates only section bounds and edge records, avoiding
    /// a startup scan that would page the entire 100+ MiB asset into memory.
    pub fn verify_checksum(&self) -> Result<(), DataError> {
        let records_end = self.aliases_offset;
        let aliases_end = self.strings_offset;
        let strings_end = self.strings_offset + self.strings_len;
        let actual = fnv1a64(
            &self.bytes[self.records_offset..records_end],
            &self.bytes[self.aliases_offset..aliases_end],
            &self.bytes[self.strings_offset..strings_end],
        );
        if actual == self.expected_hash {
            Ok(())
        } else {
            Err(DataError::Decode(
                "place search: content checksum mismatch".into(),
            ))
        }
    }

    pub fn metadata(&self) -> ArtifactMetadata<'a> {
        ArtifactMetadata {
            dataset_date: self.string(self.dataset_date).unwrap_or("invalid"),
            source: self.string(self.source).unwrap_or("invalid"),
            representative: self.representative,
            record_count: self.record_count,
            alias_count: self.alias_count,
            installed_bytes: self.bytes.len(),
        }
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, DataError> {
        let query = normalize(query);
        if query.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        match self.search_normalized_names(&query, limit, true)? {
            Some(results) => Ok(results),
            None => self.search_context(&query, limit),
        }
    }

    /// Name and alias matches only: prefix matches, or one-edit typo matches
    /// among aliases sharing the query's first character. `None` when no
    /// name matched, where [`Self::search`] falls back to
    /// [`Self::search_by_context`].
    pub fn search_names(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Option<Vec<SearchResult>>, DataError> {
        let query = normalize(query);
        if query.is_empty() || limit == 0 {
            return Ok(Some(Vec::new()));
        }
        self.search_normalized_names(&query, limit, true)
    }

    /// Prefix matches of names and aliases, without typo tolerance; `None`
    /// when nothing matched. Exact on an index holding every alias with the
    /// query's leading characters, as served to browsers in pieces.
    pub fn search_name_prefixes(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Option<Vec<SearchResult>>, DataError> {
        let query = normalize(query);
        if query.is_empty() || limit == 0 {
            return Ok(Some(Vec::new()));
        }
        self.search_normalized_names(&query, limit, false)
    }

    /// Records whose country or administrative region matches the query.
    pub fn search_by_context(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>, DataError> {
        let query = normalize(query);
        if query.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        self.search_context(&query, limit)
    }

    fn search_normalized_names(
        &self,
        query: &str,
        limit: usize,
        typo_tolerant: bool,
    ) -> Result<Option<Vec<SearchResult>>, DataError> {
        let start = self.lower_bound(query)?;
        let mut by_record = BTreeMap::<usize, Candidate>::new();
        for index in start..self.alias_count {
            let alias = self.alias(index)?;
            if !alias.key.starts_with(&query) {
                break;
            }
            let candidate = Candidate {
                record_id: alias.record_id,
                exact: alias.key == query,
                fuzzy: false,
                alias_flags: alias.flags,
            };
            by_record
                .entry(alias.record_id)
                .and_modify(|current| {
                    if candidate.exact && !current.exact {
                        *current = candidate;
                    } else {
                        current.alias_flags |= candidate.alias_flags;
                    }
                })
                .or_insert(candidate);
            if by_record.len() >= 8192 {
                break;
            }
        }

        // Small, index-free typo tolerance. It runs only when ordinary prefix
        // matching found nothing and only across aliases sharing the first
        // normalized character, so it adds no trigram artifact and stays
        // bounded for interactive queries.
        if typo_tolerant && by_record.is_empty() && query.len() >= TYPO_MINIMUM_BYTES {
            let first = &query[..query.char_indices().nth(1).map_or(query.len(), |(i, _)| i)];
            let fuzzy_start = self.lower_bound(first)?;
            for index in fuzzy_start..self.alias_count {
                let alias = self.alias(index)?;
                if !alias.key.starts_with(first) {
                    break;
                }
                if edit_distance_one_prefix(&query, alias.key) {
                    by_record.entry(alias.record_id).or_insert(Candidate {
                        record_id: alias.record_id,
                        exact: false,
                        fuzzy: true,
                        alias_flags: alias.flags,
                    });
                    if by_record.len() >= 512 {
                        break;
                    }
                }
            }
        }

        if by_record.is_empty() {
            return Ok(None);
        }

        let mut ranked = Vec::with_capacity(by_record.len());
        for candidate in by_record.into_values() {
            let record = self.record(candidate.record_id)?;
            ranked.push((rank_key(record, candidate), candidate, record));
        }
        ranked.sort_by_key(|candidate| std::cmp::Reverse(candidate.0));
        ranked.truncate(limit);
        ranked
            .into_iter()
            .map(|(_, candidate, record)| Ok(search_result(record, candidate.fuzzy)))
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }

    // Country and administrative-region labels are visible in formatted
    // results, so they must also be searchable. They are deliberately not
    // duplicated into the alias table for every one of the 1.7M records: on
    // the uncommon no-name-match path, scan the fixed-width records and
    // normalize each interned context string only once. Keeping only the
    // best `limit` records bounds query memory even for large countries.
    fn search_context(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, DataError> {
        let mut matches = HashMap::<EncodedString, u8>::new();
        let mut ranked = Vec::with_capacity(limit);
        for record_id in 0..self.record_count {
            let offset = self.records_offset + record_id * RECORD_LEN;
            let country =
                EncodedString {
                    offset: read_u32(self.bytes, offset + 4)?,
                    len: u16::from(*self.bytes.get(offset + 30).ok_or_else(|| {
                        DataError::Decode("place search: truncated record".into())
                    })?),
                };
            let admin =
                EncodedString {
                    offset: read_u32(self.bytes, offset + 8)?,
                    len: u16::from(*self.bytes.get(offset + 31).ok_or_else(|| {
                        DataError::Decode("place search: truncated record".into())
                    })?),
                };
            let country_match = self.context_match(country, query, &mut matches)?;
            let admin_match = self.context_match(admin, query, &mut matches)?;
            let match_quality = country_match.max(admin_match);
            if match_quality == 0 {
                continue;
            }

            let candidate = Candidate {
                record_id,
                exact: match_quality == 2,
                fuzzy: false,
                alias_flags: 0,
            };
            let record = self.record(record_id)?;
            let entry = (rank_key(record, candidate), candidate, record);
            if ranked.len() < limit {
                ranked.push(entry);
            } else if let Some((worst_index, _)) = ranked
                .iter()
                .enumerate()
                .min_by_key(|(_, current)| current.0)
                && entry.0 > ranked[worst_index].0
            {
                ranked[worst_index] = entry;
            }
        }
        ranked.sort_by_key(|candidate| std::cmp::Reverse(candidate.0));
        ranked
            .into_iter()
            .map(|(_, candidate, record)| Ok(search_result(record, candidate.fuzzy)))
            .collect()
    }

    fn context_match(
        &self,
        encoded: EncodedString,
        query: &str,
        matches: &mut HashMap<EncodedString, u8>,
    ) -> Result<u8, DataError> {
        if encoded.len == 0 {
            return Ok(0);
        }
        if let Some(result) = matches.get(&encoded) {
            return Ok(*result);
        }
        let context = normalize(self.string(encoded)?);
        let result = if context == query {
            2
        } else if context.starts_with(query) {
            1
        } else {
            0
        };
        matches.insert(encoded, result);
        Ok(result)
    }

    /// A self-contained artifact with the aliases at `positions` (ascending
    /// alias-table positions, as enumerated by [`Self::alias_keys`]) and the
    /// records they name. Records keep their relative order, so every search
    /// ranking over them is unchanged.
    pub fn names_subset(&self, positions: &[usize]) -> Result<Vec<u8>, DataError> {
        let mut aliases = Vec::with_capacity(positions.len());
        let mut records = std::collections::BTreeSet::new();
        for &index in positions {
            let alias = self.alias(index)?;
            records.insert(alias.record_id);
            aliases.push(alias);
        }
        self.encode_subset(&records, &aliases)
    }

    /// A self-contained artifact for [`Self::search_by_context`] with at most
    /// `limit` results: for every country and administrative-region label,
    /// the `limit` best records carrying it. Any query's best records among
    /// all labels it matches are among these.
    pub fn context_subset(&self, limit: usize) -> Result<Vec<u8>, DataError> {
        let mut best = HashMap::<EncodedString, Vec<(RankKey, usize)>>::new();
        for record_id in 0..self.record_count {
            let record = self.record(record_id)?;
            let key = rank_key(
                record,
                Candidate {
                    record_id,
                    exact: false,
                    fuzzy: false,
                    alias_flags: 0,
                },
            );
            let offset = self.records_offset + record_id * RECORD_LEN;
            for label in [
                EncodedString {
                    offset: read_u32(self.bytes, offset + 4)?,
                    len: u16::from(self.bytes[offset + 30]),
                },
                EncodedString {
                    offset: read_u32(self.bytes, offset + 8)?,
                    len: u16::from(self.bytes[offset + 31]),
                },
            ] {
                if label.len == 0 {
                    continue;
                }
                let entries = best.entry(label).or_default();
                entries.push((key, record_id));
                if entries.len() > limit {
                    entries.sort_by_key(|entry| std::cmp::Reverse(entry.0));
                    entries.truncate(limit);
                }
            }
        }
        let records = best
            .into_values()
            .flat_map(|entries| entries.into_iter().map(|(_, record_id)| record_id))
            .collect();
        self.encode_subset(&records, &[])
    }

    /// A self-contained artifact holding the records [`Self::map_places`]
    /// returns for `minimum_population`.
    pub fn map_subset(&self, minimum_population: u32) -> Result<Vec<u8>, DataError> {
        let mut records = std::collections::BTreeSet::new();
        for record_id in 0..self.record_count {
            if self.is_map_place(record_id, minimum_population)? {
                records.insert(record_id);
            }
        }
        self.encode_subset(&records, &[])
    }

    fn encode_subset(
        &self,
        records: &std::collections::BTreeSet<usize>,
        aliases: &[AliasRef<'_>],
    ) -> Result<Vec<u8>, DataError> {
        let mut pool = StringPool::default();
        let dataset_date = pool.intern(self.string(self.dataset_date)?)?;
        let source = pool.intern(self.string(self.source)?)?;
        let mut new_ids = HashMap::with_capacity(records.len());
        let mut encoded_records = Vec::with_capacity(records.len() * RECORD_LEN);
        for (new_id, &record_id) in records.iter().enumerate() {
            new_ids.insert(record_id, new_id as u32);
            let record = self.record(record_id)?;
            let display = pool.intern(record.display_name)?;
            let country = pool.intern(record.country)?;
            let admin = pool.intern(record.admin_region)?;
            put_u32(&mut encoded_records, display.offset);
            put_u32(&mut encoded_records, country.offset);
            put_u32(&mut encoded_records, admin.offset);
            put_i32(&mut encoded_records, record.latitude_e7);
            put_i32(&mut encoded_records, record.longitude_e7);
            put_u32(&mut encoded_records, record.population);
            put_u32(&mut encoded_records, record.view_altitude_m);
            put_u16(&mut encoded_records, display.len);
            // Label lengths fit a byte in the source artifact.
            encoded_records.push(country.len as u8);
            encoded_records.push(admin.len as u8);
            encoded_records.push(record.kind as u8);
            encoded_records.push(record.flags);
            put_i16(&mut encoded_records, record.rank_boost);
            put_u32(&mut encoded_records, record.geoname_id);
        }
        let mut encoded_aliases = Vec::with_capacity(aliases.len() * ALIAS_LEN);
        for alias in aliases {
            let key = pool.intern(alias.key)?;
            put_u32(&mut encoded_aliases, key.offset);
            put_u32(&mut encoded_aliases, new_ids[&alias.record_id]);
            put_u16(&mut encoded_aliases, key.len);
            encoded_aliases.push(alias.flags);
            encoded_aliases.push(0);
        }
        write_artifact(Sections {
            records: &encoded_records,
            record_count: records.len() as u32,
            aliases: &encoded_aliases,
            alias_count: aliases.len() as u32,
            pool: &pool,
            dataset_date,
            source,
            representative: self.representative,
        })
    }

    /// Whether [`Self::map_places`] lists a record. The fixed-width numeric
    /// fields are checked first so decoding and allocating names is limited
    /// to the relatively small display set.
    fn is_map_place(&self, record_id: usize, minimum_population: u32) -> Result<bool, DataError> {
        let offset = self.records_offset + record_id * RECORD_LEN;
        let population = read_u32(self.bytes, offset + 20)?;
        let kind = PlaceKind::decode(
            *self
                .bytes
                .get(offset + 32)
                .ok_or_else(|| DataError::Decode("place search: truncated record".into()))?,
        )?;
        Ok(
            (population >= minimum_population || kind == PlaceKind::Capital)
                && matches!(
                    kind,
                    PlaceKind::City
                        | PlaceKind::Town
                        | PlaceKind::Village
                        | PlaceKind::Capital
                        | PlaceKind::AdministrativeSeat
                ),
        )
    }

    /// Every normalized alias key, in sorted order.
    pub fn alias_keys(&self) -> impl Iterator<Item = Result<&'a str, DataError>> + '_ {
        (0..self.alias_count).map(|index| self.alias(index).map(|alias| alias.key))
    }

    /// Extract the settlement records suitable for a geographic map label
    /// catalog. Unlike [`Self::search`], labels intentionally expose only the
    /// primary place name: administrative regions and countries are not
    /// composed into the result.
    pub fn map_places(&self, minimum_population: u32) -> Result<Vec<MapPlace>, DataError> {
        let mut places = Vec::new();
        for record_id in 0..self.record_count {
            if !self.is_map_place(record_id, minimum_population)? {
                continue;
            }
            let record = self.record(record_id)?;
            let population = record.population;
            let kind = record.kind;
            places.push(MapPlace {
                name: record.display_name.to_owned(),
                latitude_deg: record.latitude_e7 as f64 / 10_000_000.0,
                longitude_deg: record.longitude_e7 as f64 / 10_000_000.0,
                population,
                kind,
                geoname_id: record.geoname_id,
            });
        }
        Ok(places)
    }

    fn lower_bound(&self, query: &str) -> Result<usize, DataError> {
        let mut low = 0;
        let mut high = self.alias_count;
        while low < high {
            let mid = low + (high - low) / 2;
            if self.alias(mid)?.key < query {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        Ok(low)
    }

    fn alias(&self, index: usize) -> Result<AliasRef<'a>, DataError> {
        if index >= self.alias_count {
            return Err(DataError::Decode(
                "place search: alias index out of range".into(),
            ));
        }
        let offset = self.aliases_offset + index * ALIAS_LEN;
        let key = EncodedString {
            offset: read_u32(self.bytes, offset)?,
            len: read_u16(self.bytes, offset + 8)?,
        };
        let record_id = read_u32(self.bytes, offset + 4)? as usize;
        if record_id >= self.record_count {
            return Err(DataError::Decode(
                "place search: alias record out of range".into(),
            ));
        }
        Ok(AliasRef {
            key: self.string(key)?,
            record_id,
            flags: *self
                .bytes
                .get(offset + 10)
                .ok_or_else(|| DataError::Decode("place search: truncated alias".into()))?,
        })
    }

    fn record(&self, index: usize) -> Result<RecordRef<'a>, DataError> {
        if index >= self.record_count {
            return Err(DataError::Decode(
                "place search: record index out of range".into(),
            ));
        }
        let offset = self.records_offset + index * RECORD_LEN;
        let display = EncodedString {
            offset: read_u32(self.bytes, offset)?,
            len: read_u16(self.bytes, offset + 28)?,
        };
        let country = EncodedString {
            offset: read_u32(self.bytes, offset + 4)?,
            len: *self
                .bytes
                .get(offset + 30)
                .ok_or_else(|| DataError::Decode("place search: truncated record".into()))?
                as u16,
        };
        let admin = EncodedString {
            offset: read_u32(self.bytes, offset + 8)?,
            len: *self
                .bytes
                .get(offset + 31)
                .ok_or_else(|| DataError::Decode("place search: truncated record".into()))?
                as u16,
        };
        Ok(RecordRef {
            display_name: self.string(display)?,
            country: self.string(country)?,
            admin_region: self.string(admin)?,
            latitude_e7: read_i32(self.bytes, offset + 12)?,
            longitude_e7: read_i32(self.bytes, offset + 16)?,
            population: read_u32(self.bytes, offset + 20)?,
            view_altitude_m: read_u32(self.bytes, offset + 24)?,
            kind: PlaceKind::decode(
                *self
                    .bytes
                    .get(offset + 32)
                    .ok_or_else(|| DataError::Decode("place search: truncated record".into()))?,
            )?,
            flags: *self
                .bytes
                .get(offset + 33)
                .ok_or_else(|| DataError::Decode("place search: truncated record".into()))?,
            rank_boost: read_i16(self.bytes, offset + 34)?,
            geoname_id: read_u32(self.bytes, offset + 36)?,
        })
    }

    fn string(&self, encoded: EncodedString) -> Result<&'a str, DataError> {
        let start = self
            .strings_offset
            .checked_add(encoded.offset as usize)
            .ok_or_else(|| DataError::Decode("place search: string offset overflow".into()))?;
        let end = start
            .checked_add(encoded.len as usize)
            .ok_or_else(|| DataError::Decode("place search: string length overflow".into()))?;
        if start < self.strings_offset
            || end > self.strings_offset + self.strings_len
            || end > self.bytes.len()
        {
            return Err(DataError::Decode(
                "place search: string outside pool".into(),
            ));
        }
        std::str::from_utf8(&self.bytes[start..end])
            .map_err(|error| DataError::Decode(format!("place search: invalid UTF-8: {error}")))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SearchResult {
    pub display_name: String,
    pub latitude_deg: f64,
    pub longitude_deg: f64,
    pub view_altitude_m: u32,
    pub kind: PlaceKind,
    pub population: u32,
    pub geoname_id: u32,
    pub fuzzy_match: bool,
}

/// A settlement label for the world map. `name` is deliberately the primary
/// place name only, never the formatted search name with region or country
/// context.
#[derive(Clone, Debug, PartialEq)]
pub struct MapPlace {
    pub name: String,
    pub latitude_deg: f64,
    pub longitude_deg: f64,
    pub population: u32,
    pub kind: PlaceKind,
    pub geoname_id: u32,
}

fn search_result(record: RecordRef<'_>, fuzzy_match: bool) -> SearchResult {
    SearchResult {
        display_name: format_display(record),
        latitude_deg: record.latitude_e7 as f64 / 10_000_000.0,
        longitude_deg: record.longitude_e7 as f64 / 10_000_000.0,
        view_altitude_m: record.view_altitude_m,
        kind: record.kind,
        population: record.population,
        geoname_id: record.geoname_id,
        fuzzy_match,
    }
}

fn format_display(record: RecordRef<'_>) -> String {
    let mut parts = Vec::with_capacity(3);
    parts.push(record.display_name);
    if !record.admin_region.is_empty()
        && record.admin_region != record.display_name
        && record.admin_region != record.country
    {
        parts.push(record.admin_region);
    }
    if !record.country.is_empty() {
        parts.push(record.country);
    }
    parts.join(", ")
}

type RankKey = (u8, u8, u8, i16, u32, u8, u32);

fn rank_key(record: RecordRef<'_>, candidate: Candidate) -> RankKey {
    (
        u8::from(candidate.exact),
        u8::from(record.flags & FLAG_CURATED != 0),
        u8::from(record.flags & FLAG_SEAT != 0),
        record.rank_boost,
        record.population,
        u8::from(candidate.alias_flags & ALIAS_PRIMARY != 0),
        u32::MAX - candidate.record_id as u32,
    )
}

fn edit_distance_one_prefix(query: &str, candidate: &str) -> bool {
    if !query.is_ascii() || !candidate.is_ascii() {
        return false;
    }
    let q = query.as_bytes();
    let c = candidate.as_bytes();
    let min = q.len().saturating_sub(1);
    let max = (q.len() + 1).min(c.len());
    (min..=max).any(|length| edit_distance_at_most_one(q, &c[..length]))
}

fn edit_distance_at_most_one(left: &[u8], right: &[u8]) -> bool {
    let length_delta = left.len().abs_diff(right.len());
    if length_delta > 1 {
        return false;
    }
    if left.len() == right.len() {
        return left
            .iter()
            .zip(right)
            .filter(|(a, b)| a != b)
            .take(2)
            .count()
            <= 1;
    }
    let (short, long) = if left.len() < right.len() {
        (left, right)
    } else {
        (right, left)
    };
    let mut short_index = 0;
    let mut long_index = 0;
    let mut skipped = false;
    while short_index < short.len() && long_index < long.len() {
        if short[short_index] == long[long_index] {
            short_index += 1;
            long_index += 1;
        } else if skipped {
            return false;
        } else {
            skipped = true;
            long_index += 1;
        }
    }
    true
}

fn fnv1a64(parts: &[u8], aliases: &[u8], strings: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in parts.iter().chain(aliases).chain(strings) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_i16(out: &mut Vec<u8>, value: i16) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, DataError> {
    Ok(u16::from_le_bytes(
        bytes
            .get(offset..offset + 2)
            .ok_or_else(|| DataError::Decode("place search: truncated u16".into()))?
            .try_into()
            .unwrap(),
    ))
}
fn read_i16(bytes: &[u8], offset: usize) -> Result<i16, DataError> {
    Ok(i16::from_le_bytes(
        bytes
            .get(offset..offset + 2)
            .ok_or_else(|| DataError::Decode("place search: truncated i16".into()))?
            .try_into()
            .unwrap(),
    ))
}
fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, DataError> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or_else(|| DataError::Decode("place search: truncated u32".into()))?
            .try_into()
            .unwrap(),
    ))
}
fn read_i32(bytes: &[u8], offset: usize) -> Result<i32, DataError> {
    Ok(i32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or_else(|| DataError::Decode("place search: truncated i32".into()))?
            .try_into()
            .unwrap(),
    ))
}
fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, DataError> {
    Ok(u64::from_le_bytes(
        bytes
            .get(offset..offset + 8)
            .ok_or_else(|| DataError::Decode("place search: truncated u64".into()))?
            .try_into()
            .unwrap(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_record(
        id: u32,
        name: &str,
        aliases: &[&str],
        population: u64,
        curated: bool,
    ) -> BuildRecord {
        BuildRecord {
            geoname_id: id,
            display_name: name.into(),
            aliases: aliases.iter().map(|value| (*value).into()).collect(),
            latitude_deg: 48.0 + id as f64 / 100.0,
            longitude_deg: 2.0,
            country: "Testland".into(),
            admin_region: "Central".into(),
            kind: if curated {
                PlaceKind::Landmark
            } else {
                PlaceKind::City
            },
            population,
            rank_boost: 0,
            view_altitude_m: None,
            curated,
            administrative_seat: false,
            primary_tier: true,
        }
    }

    fn index(records: Vec<BuildRecord>) -> SearchIndex<'static> {
        let bytes = encode(
            records,
            BuildMetadata {
                dataset_date: "2026-07-31",
                source: "unit test",
                representative: true,
            },
        )
        .unwrap();
        SearchIndex::open(Box::leak(bytes.into_boxed_slice())).unwrap()
    }

    #[test]
    fn normalization_folds_case_diacritics_punctuation_and_space() {
        assert_eq!(normalize("  Café—de  l'Œuvre  "), "cafe de l œuvre");
        assert_eq!(normalize("MÜNCHEN"), "munchen");
    }

    #[test]
    fn aliases_prefixes_ranking_and_coordinates_round_trip() {
        let index = index(vec![
            test_record(1, "München", &["Munich"], 1_500_000, false),
            test_record(2, "Munich Museum", &[], 0, true),
        ]);
        let exact = index.search("munich", 6).unwrap();
        assert_eq!(exact[0].display_name, "München, Central, Testland");
        assert!((exact[0].latitude_deg - 48.01).abs() < 0.000_000_1);
        let prefix = index.search("muni", 6).unwrap();
        assert_eq!(prefix[0].kind, PlaceKind::Landmark);
        assert_eq!(prefix[1].population, 1_500_000);
    }

    #[test]
    fn displayed_country_and_admin_context_are_searchable() {
        let mut faroe = test_record(1, "Tórshavn", &[], 14_000, false);
        faroe.country = "Faroe Islands".into();
        faroe.admin_region = "Streymoy".into();
        faroe.administrative_seat = true;
        let mut klaksvik = test_record(2, "Klaksvík", &[], 5_000, false);
        klaksvik.country = "Faroe Islands".into();
        klaksvik.admin_region = "Norðoyar".into();
        let index = index(vec![faroe, klaksvik]);

        let country = index.search("Faroe Islands", 6).unwrap();
        assert_eq!(country.len(), 2);
        assert!(country[0].display_name.starts_with("Tórshavn"));
        assert!(
            country
                .iter()
                .all(|place| place.display_name.ends_with("Faroe Islands"))
        );

        let admin = index.search("norð", 6).unwrap();
        assert_eq!(admin.len(), 1);
        assert!(admin[0].display_name.starts_with("Klaksvík, Norðoyar"));
    }

    #[test]
    fn limited_typo_matching_works_without_a_secondary_index() {
        let index = index(vec![test_record(1, "Sydney", &[], 5_000_000, false)]);
        let result = index.search("sydny", 6).unwrap();
        assert_eq!(result[0].display_name, "Sydney, Central, Testland");
        assert!(result[0].fuzzy_match);
    }

    #[test]
    fn view_scale_tracks_kind_and_population() {
        assert_eq!(derive_view_altitude_m(PlaceKind::Landmark, 0), 900);
        assert_eq!(derive_view_altitude_m(PlaceKind::Village, 0), 1_800);
        assert_eq!(
            derive_view_altitude_m(PlaceKind::AdministrativeSeat, 500),
            8_000
        );
        assert!(
            derive_view_altitude_m(PlaceKind::City, 6_000_000)
                > derive_view_altitude_m(PlaceKind::Town, 60_000)
        );
    }

    #[test]
    fn corruption_and_size_budget_are_enforced() {
        let mut bytes = encode(
            vec![test_record(1, "Paris", &[], 2_000_000, false)],
            BuildMetadata {
                dataset_date: "2026-07-31",
                source: "unit test",
                representative: false,
            },
        )
        .unwrap();
        bytes[HEADER_LEN] ^= 1;
        assert!(
            SearchIndex::open(&bytes)
                .unwrap()
                .verify_checksum()
                .is_err()
        );
        assert_eq!(MAX_INSTALLED_BYTES, 157_286_400);
    }

    #[test]
    fn encoding_is_byte_for_byte_deterministic() {
        let records = vec![
            test_record(9, "Florence", &["Firenze"], 360_000, false),
            test_record(2, "Farm Tomita", &["Tomita Farm"], 0, true),
        ];
        let metadata = BuildMetadata {
            dataset_date: "2026-07-31",
            source: "unit test",
            representative: false,
        };
        assert_eq!(
            encode(records.clone(), metadata).unwrap(),
            encode(records, metadata).unwrap()
        );
    }

    #[test]
    fn map_places_are_settlements_with_primary_names_only() {
        let mut capital = test_record(1, "Small Capital", &[], 100, false);
        capital.kind = PlaceKind::Capital;
        let mut locality = test_record(2, "Broad Region", &[], 2_000_000, false);
        locality.kind = PlaceKind::Locality;
        let index = index(vec![
            capital,
            locality,
            test_record(3, "Map Town", &[], 12_000, false),
            test_record(4, "Tiny Village", &[], 400, false),
        ]);

        let places = index.map_places(3_000).unwrap();
        assert_eq!(places.len(), 2);
        assert!(places.iter().any(|place| place.name == "Small Capital"));
        assert!(places.iter().any(|place| place.name == "Map Town"));
        assert!(places.iter().all(|place| !place.name.contains(',')));
    }
}
