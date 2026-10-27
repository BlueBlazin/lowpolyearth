//! Protomaps PMTiles source descriptor and Version 4 compatibility policy.
//!
//! Metadata is read once while a source is constructed. Terrain generation
//! never performs a metadata read, and both blocking and async constructors
//! feed the exact same parser and compatibility decision.

use crate::DataError;
use crate::pmtiles::{Compression, Header, TILE_TYPE_MVT};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum FieldType {
    String,
    Number,
    Boolean,
    Unknown(String),
}

impl FieldType {
    fn parse(value: &Value) -> Self {
        let Some(value) = value.as_str() else {
            return Self::Unknown(canonical_json(value));
        };
        match value.to_ascii_lowercase().as_str() {
            "string" => Self::String,
            "number" => Self::Number,
            "boolean" => Self::Boolean,
            _ => Self::Unknown(value.to_string()),
        }
    }

    fn label(&self) -> &str {
        match self {
            Self::String => "String",
            Self::Number => "Number",
            Self::Boolean => "Boolean",
            Self::Unknown(value) => value,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorLayerSchema {
    pub id: String,
    pub min_zoom: u8,
    pub max_zoom: u8,
    pub fields: BTreeMap<String, FieldType>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compatibility {
    Compatible,
    CompatibleWithUnknowns,
    Incompatible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueSeverity {
    Unknown,
    Incompatible,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibilityIssue {
    pub severity: IssueSeverity,
    pub layer: Option<String>,
    pub field: Option<String>,
    pub message: String,
}

impl CompatibilityIssue {
    fn unknown(layer: Option<&str>, field: Option<&str>, message: impl Into<String>) -> Self {
        Self {
            severity: IssueSeverity::Unknown,
            layer: layer.map(str::to_string),
            field: field.map(str::to_string),
            message: message.into(),
        }
    }

    fn incompatible(layer: Option<&str>, field: Option<&str>, message: impl Into<String>) -> Self {
        Self {
            severity: IssueSeverity::Incompatible,
            layer: layer.map(str::to_string),
            field: field.map(str::to_string),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDescriptor {
    pub build_tag: String,
    pub pmtiles_spec_version: u8,
    pub tile_type: u8,
    pub archive_min_zoom: u8,
    pub archive_max_zoom: u8,
    pub metadata_name: Option<String>,
    pub schema_version: Option<String>,
    pub layers: BTreeMap<String, VectorLayerSchema>,
    pub metadata_fingerprint: u64,
    pub terrain_contract_fingerprint: u64,
    pub compatibility: Compatibility,
    pub issues: Vec<CompatibilityIssue>,
}

impl SourceDescriptor {
    pub fn from_metadata(
        build_tag: impl Into<String>,
        header: &Header,
        metadata_json: &str,
    ) -> Result<Self, DataError> {
        let build_tag = build_tag.into();
        let metadata: Value = serde_json::from_str(metadata_json).map_err(|error| {
            DataError::Decode(format!(
                "protomaps source {build_tag}: malformed metadata JSON: {error}"
            ))
        })?;
        if !metadata.is_object() {
            return Err(DataError::Decode(format!(
                "protomaps source {build_tag}: metadata root must be an object"
            )));
        }

        let canonical_contract = format!(
            "pmtiles={};tile_type={};archive_zoom={}:{};internal={};tile={};metadata={}",
            header.spec_version,
            header.tile_type,
            header.min_zoom,
            header.max_zoom,
            compression_label(header.internal_compression),
            compression_label(header.tile_compression),
            canonical_json(&metadata)
        );
        let metadata_fingerprint = fnv1a(canonical_contract.as_bytes());
        let metadata_name = metadata
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string);
        let schema_version = metadata
            .get("version")
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut issues = Vec::new();
        let mut layers = BTreeMap::new();

        match metadata.get("vector_layers") {
            Some(Value::Array(entries)) => {
                for entry in entries {
                    let Some(object) = entry.as_object() else {
                        issues.push(CompatibilityIssue::incompatible(
                            None,
                            None,
                            "vector_layers entry is not an object",
                        ));
                        continue;
                    };
                    let Some(id) = object.get("id").and_then(Value::as_str) else {
                        issues.push(CompatibilityIssue::incompatible(
                            None,
                            None,
                            "vector_layers entry has no string id",
                        ));
                        continue;
                    };
                    let min_zoom = zoom_value(object.get("minzoom"));
                    let max_zoom = zoom_value(object.get("maxzoom"));
                    let fields = parse_fields(id, object.get("fields"), &mut issues);
                    let (Some(min_zoom), Some(max_zoom)) = (min_zoom, max_zoom) else {
                        issues.push(CompatibilityIssue::incompatible(
                            Some(id),
                            None,
                            "layer minzoom/maxzoom must be integers in 0..=30",
                        ));
                        continue;
                    };
                    if min_zoom > max_zoom {
                        issues.push(CompatibilityIssue::incompatible(
                            Some(id),
                            None,
                            format!("invalid authored zoom range {min_zoom}..{max_zoom}"),
                        ));
                    }
                    let layer = VectorLayerSchema {
                        id: id.to_string(),
                        min_zoom,
                        max_zoom,
                        fields,
                    };
                    if layers.insert(id.to_string(), layer).is_some() {
                        issues.push(CompatibilityIssue::incompatible(
                            Some(id),
                            None,
                            "duplicate layer declaration",
                        ));
                    }
                }
            }
            _ => issues.push(CompatibilityIssue::incompatible(
                None,
                None,
                "metadata has no vector_layers array",
            )),
        }

        validate_header(header, &mut issues);
        validate_schema_version(schema_version.as_deref(), &mut issues);
        validate_layers(&layers, header, &mut issues);
        let compatibility = compatibility_from_issues(&issues);
        let terrain_contract_fingerprint = terrain_contract_fingerprint(header, &layers);

        Ok(Self {
            build_tag,
            pmtiles_spec_version: header.spec_version,
            tile_type: header.tile_type,
            archive_min_zoom: header.min_zoom,
            archive_max_zoom: header.max_zoom,
            metadata_name,
            schema_version,
            layers,
            metadata_fingerprint,
            terrain_contract_fingerprint,
            compatibility,
            issues,
        })
    }

    /// Persistent terrain identity includes the immutable archive revision,
    /// only the metadata contract consumed by physical adapters/planning, and
    /// the independently prepared bridge catalogue. Full metadata and context
    /// presentation remain diagnostic and cannot invalidate terrain by accident.
    pub fn terrain_dataset_identity(&self, bridge_catalog_tag: &str) -> String {
        format!(
            "{}-terrain-{:016x}-{bridge_catalog_tag}",
            self.build_tag, self.terrain_contract_fingerprint
        )
    }

    pub fn diagnostic_identity(&self, bridge_catalog_tag: &str) -> String {
        format!(
            "{}-semantic-v{}-metadata-{:016x}-terrain-{:016x}-{bridge_catalog_tag}",
            self.build_tag,
            crate::protomaps_semantics::SEMANTIC_SCHEMA_VERSION,
            self.metadata_fingerprint,
            self.terrain_contract_fingerprint,
        )
    }

    pub fn diagnostic_summary(&self) -> String {
        let mut summary = format!(
            "build {} PMTiles v{} MVT z{}-{} metadata {:016x} terrain {:016x} semantic v{} schema {}: {:?}",
            self.build_tag,
            self.pmtiles_spec_version,
            self.archive_min_zoom,
            self.archive_max_zoom,
            self.metadata_fingerprint,
            self.terrain_contract_fingerprint,
            crate::protomaps_semantics::SEMANTIC_SCHEMA_VERSION,
            self.schema_version.as_deref().unwrap_or("unknown"),
            self.compatibility
        );
        for issue in self.issues.iter().take(8) {
            summary.push_str("; ");
            if let Some(layer) = &issue.layer {
                summary.push_str("layer ");
                summary.push_str(layer);
                if let Some(field) = &issue.field {
                    summary.push('.');
                    summary.push_str(field);
                }
                summary.push_str(": ");
            }
            summary.push_str(&issue.message);
        }
        if self.issues.len() > 8 {
            let _ = write!(summary, "; +{} more", self.issues.len() - 8);
        }
        summary
    }
}

#[derive(Clone, Copy)]
enum RequiredType {
    String,
    Number,
    Boolean,
}

impl RequiredType {
    fn matches(self, actual: &FieldType) -> bool {
        matches!(
            (self, actual),
            (Self::String, FieldType::String)
                | (Self::Number, FieldType::Number)
                | (Self::Boolean, FieldType::Boolean)
        )
    }

    fn label(self) -> &'static str {
        match self {
            Self::String => "String",
            Self::Number => "Number",
            Self::Boolean => "Boolean",
        }
    }
}

struct RequiredLayer {
    id: &'static str,
    min_zoom: u8,
    max_zoom: u8,
    fields: &'static [(&'static str, RequiredType)],
}

const REQUIRED_LAYERS: &[RequiredLayer] = &[
    RequiredLayer {
        id: "boundaries",
        min_zoom: 0,
        max_zoom: 15,
        fields: &[
            ("kind", RequiredType::String),
            ("kind_detail", RequiredType::Number),
        ],
    },
    RequiredLayer {
        id: "buildings",
        min_zoom: 11,
        max_zoom: 15,
        fields: &[
            ("kind", RequiredType::String),
            ("height", RequiredType::Number),
            ("min_height", RequiredType::Number),
            ("addr_housenumber", RequiredType::String),
            ("layer", RequiredType::Number),
        ],
    },
    RequiredLayer {
        id: "earth",
        min_zoom: 0,
        max_zoom: 15,
        fields: &[("kind", RequiredType::String)],
    },
    RequiredLayer {
        id: "landcover",
        min_zoom: 0,
        max_zoom: 7,
        fields: &[("kind", RequiredType::String)],
    },
    RequiredLayer {
        id: "landuse",
        min_zoom: 2,
        max_zoom: 15,
        fields: &[("kind", RequiredType::String)],
    },
    RequiredLayer {
        id: "places",
        min_zoom: 1,
        max_zoom: 15,
        fields: &[
            ("kind", RequiredType::String),
            ("kind_detail", RequiredType::String),
            ("capital", RequiredType::String),
            ("population", RequiredType::Number),
            ("wikidata", RequiredType::String),
        ],
    },
    RequiredLayer {
        id: "pois",
        min_zoom: 5,
        max_zoom: 15,
        fields: &[
            ("kind", RequiredType::String),
            ("kind_detail", RequiredType::String),
        ],
    },
    RequiredLayer {
        id: "roads",
        min_zoom: 3,
        max_zoom: 15,
        fields: &[
            ("kind", RequiredType::String),
            ("kind_detail", RequiredType::String),
            ("is_bridge", RequiredType::Boolean),
            ("is_tunnel", RequiredType::Boolean),
            ("oneway", RequiredType::String),
            ("ref", RequiredType::String),
            ("network", RequiredType::String),
            ("shield_text", RequiredType::String),
            ("service", RequiredType::String),
        ],
    },
    RequiredLayer {
        id: "water",
        min_zoom: 0,
        max_zoom: 15,
        fields: &[
            ("kind", RequiredType::String),
            ("kind_detail", RequiredType::String),
            ("bridge", RequiredType::String),
            ("tunnel", RequiredType::String),
            ("layer", RequiredType::Number),
        ],
    },
];

/// Metadata inputs that can change generated physical output or the
/// source zoom selected for it. Arbitrary context/map additions intentionally
/// do not enter the terrain cache identity.
const TERRAIN_CONTRACT_FIELDS: &[(&str, &[&str])] = &[
    (
        "buildings",
        &["height", "height_m", "kind", "min_height", "min_height_m"],
    ),
    ("landcover", &["kind"]),
    ("landuse", &["kind"]),
    ("pois", &["kind"]),
    (
        "roads",
        &[
            "is_bridge",
            "is_tunnel",
            "is_link",
            "kind",
            "kind_detail",
            "name",
        ],
    ),
    (
        "water",
        &[
            "alkaline",
            "bridge",
            "intermittent",
            "kind",
            "kind_detail",
            "layer",
            "min_zoom",
            "reservoir",
            "tunnel",
        ],
    ),
];

fn terrain_contract_fingerprint(
    header: &Header,
    layers: &BTreeMap<String, VectorLayerSchema>,
) -> u64 {
    let mut contract = format!(
        "pmtiles={};tile_type={};archive_zoom={}:{};internal={};tile={}",
        header.spec_version,
        header.tile_type,
        header.min_zoom,
        header.max_zoom,
        compression_label(header.internal_compression),
        compression_label(header.tile_compression),
    );
    for &(layer_name, fields) in TERRAIN_CONTRACT_FIELDS {
        let _ = write!(contract, ";layer={layer_name}");
        let Some(layer) = layers.get(layer_name) else {
            contract.push_str("@missing");
            continue;
        };
        let _ = write!(contract, "@{}:{}", layer.min_zoom, layer.max_zoom);
        for &field in fields {
            let _ = write!(contract, ";{field}=");
            match layer.fields.get(field) {
                Some(field_type) => contract.push_str(field_type.label()),
                None => contract.push_str("missing"),
            }
        }
    }
    fnv1a(contract.as_bytes())
}

fn zoom_value(value: Option<&Value>) -> Option<u8> {
    value?
        .as_u64()
        .and_then(|zoom| u8::try_from(zoom).ok())
        .filter(|zoom| *zoom <= 30)
}

fn parse_fields(
    layer: &str,
    value: Option<&Value>,
    issues: &mut Vec<CompatibilityIssue>,
) -> BTreeMap<String, FieldType> {
    let Some(Value::Object(fields)) = value else {
        issues.push(CompatibilityIssue::incompatible(
            Some(layer),
            None,
            "layer has no fields object",
        ));
        return BTreeMap::new();
    };
    fields
        .iter()
        .map(|(name, value)| (name.clone(), FieldType::parse(value)))
        .collect()
}

fn validate_header(header: &Header, issues: &mut Vec<CompatibilityIssue>) {
    if header.spec_version != 3 {
        issues.push(CompatibilityIssue::incompatible(
            None,
            None,
            format!("unsupported PMTiles version {}", header.spec_version),
        ));
    }
    if header.tile_type != TILE_TYPE_MVT {
        issues.push(CompatibilityIssue::incompatible(
            None,
            None,
            format!("archive tile type {} is not MVT", header.tile_type),
        ));
    }
    if let Compression::Unsupported(value) = header.internal_compression {
        issues.push(CompatibilityIssue::incompatible(
            None,
            None,
            format!("unsupported PMTiles internal compression {value}"),
        ));
    }
    if let Compression::Unsupported(value) = header.tile_compression {
        issues.push(CompatibilityIssue::incompatible(
            None,
            None,
            format!("unsupported MVT tile compression {value}"),
        ));
    }
    if header.min_zoom > header.max_zoom {
        issues.push(CompatibilityIssue::incompatible(
            None,
            None,
            format!(
                "invalid archive zoom range {}..{}",
                header.min_zoom, header.max_zoom
            ),
        ));
    }
}

fn compression_label(compression: Compression) -> String {
    match compression {
        Compression::None => "none".to_string(),
        Compression::Gzip => "gzip".to_string(),
        Compression::Unsupported(value) => format!("unsupported-{value}"),
    }
}

fn validate_schema_version(version: Option<&str>, issues: &mut Vec<CompatibilityIssue>) {
    match version {
        Some(version) if version.split('.').next() == Some("4") => {}
        Some(version) => issues.push(CompatibilityIssue::incompatible(
            None,
            Some("version"),
            format!("unsupported basemap schema version {version}; expected Version 4"),
        )),
        None => issues.push(CompatibilityIssue::unknown(
            None,
            Some("version"),
            "metadata does not declare a basemap schema version",
        )),
    }
}

fn validate_layers(
    layers: &BTreeMap<String, VectorLayerSchema>,
    header: &Header,
    issues: &mut Vec<CompatibilityIssue>,
) {
    let required_ids: HashSet<_> = REQUIRED_LAYERS.iter().map(|layer| layer.id).collect();
    for required in REQUIRED_LAYERS {
        let Some(actual) = layers.get(required.id) else {
            issues.push(CompatibilityIssue::incompatible(
                Some(required.id),
                None,
                "required Version 4 layer is missing",
            ));
            continue;
        };
        if actual.min_zoom > required.min_zoom || actual.max_zoom < required.max_zoom {
            issues.push(CompatibilityIssue::incompatible(
                Some(required.id),
                None,
                format!(
                    "authored zoom range {}..{} does not cover required {}..{}",
                    actual.min_zoom, actual.max_zoom, required.min_zoom, required.max_zoom
                ),
            ));
        }
        if actual.min_zoom < header.min_zoom || actual.max_zoom > header.max_zoom {
            issues.push(CompatibilityIssue::incompatible(
                Some(required.id),
                None,
                format!(
                    "declared zoom range {}..{} falls outside archive {}..{}",
                    actual.min_zoom, actual.max_zoom, header.min_zoom, header.max_zoom
                ),
            ));
        }
        for &(field, expected) in required.fields {
            match actual.fields.get(field) {
                Some(value) if expected.matches(value) => {}
                Some(value) => issues.push(CompatibilityIssue::incompatible(
                    Some(required.id),
                    Some(field),
                    format!(
                        "type {} is incompatible; expected {}",
                        value.label(),
                        expected.label()
                    ),
                )),
                None => issues.push(CompatibilityIssue::incompatible(
                    Some(required.id),
                    Some(field),
                    format!("required {} field is missing", expected.label()),
                )),
            }
        }
        for (field, field_type) in &actual.fields {
            if !required.fields.iter().any(|(name, _)| name == field)
                && !known_optional_field(required.id, field)
            {
                issues.push(CompatibilityIssue::unknown(
                    Some(required.id),
                    Some(field),
                    format!("additive field with declared type {}", field_type.label()),
                ));
            } else if matches!(field_type, FieldType::Unknown(_)) {
                issues.push(CompatibilityIssue::unknown(
                    Some(required.id),
                    Some(field),
                    format!("unknown declared field type {}", field_type.label()),
                ));
            }
        }
    }
    for layer in layers
        .keys()
        .filter(|id| !required_ids.contains(id.as_str()))
    {
        issues.push(CompatibilityIssue::unknown(
            Some(layer),
            None,
            "additive vector layer",
        ));
    }
}

fn known_optional_field(layer: &str, field: &str) -> bool {
    if matches!(field, "name" | "name2" | "name3" | "min_zoom" | "sort_rank")
        || field.starts_with("name:")
        || field.starts_with("pgf:name")
        || matches!(field, "script" | "script2" | "script3")
    {
        return true;
    }
    match layer {
        "boundaries" => matches!(field, "brk_a3" | "disputed"),
        "buildings" => matches!(field, "kind_detail"),
        "places" => {
            matches!(field, "population_rank" | "ref" | "sort_key") || field.starts_with("ref:")
        }
        "pois" => matches!(field, "elevation" | "iata"),
        "roads" => {
            matches!(field, "access" | "is_link" | "route")
                || field.starts_with("network_")
                || field.starts_with("shield_text_")
        }
        _ => false,
    }
}

fn compatibility_from_issues(issues: &[CompatibilityIssue]) -> Compatibility {
    if issues
        .iter()
        .any(|issue| issue.severity == IssueSeverity::Incompatible)
    {
        Compatibility::Incompatible
    } else if issues.is_empty() {
        Compatibility::Compatible
    } else {
        Compatibility::CompatibleWithUnknowns
    }
}

fn canonical_json(value: &Value) -> String {
    fn write_value(value: &Value, parent_key: Option<&str>, out: &mut String) {
        match value {
            Value::Null => out.push_str("null"),
            Value::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            Value::Number(value) => out.push_str(&value.to_string()),
            Value::String(value) => out.push_str(&serde_json::to_string(value).unwrap()),
            Value::Array(values) => {
                let mut values: Vec<_> = values.iter().collect();
                if parent_key == Some("vector_layers") {
                    values.sort_by(|left, right| {
                        left.get("id")
                            .and_then(Value::as_str)
                            .cmp(&right.get("id").and_then(Value::as_str))
                    });
                }
                out.push('[');
                for (index, value) in values.into_iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    write_value(value, None, out);
                }
                out.push(']');
            }
            Value::Object(values) => {
                let mut keys: Vec<_> = values.keys().collect();
                keys.sort_unstable();
                out.push('{');
                for (index, key) in keys.into_iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    out.push_str(&serde_json::to_string(key).unwrap());
                    out.push(':');
                    write_value(&values[key], Some(key), out);
                }
                out.push('}');
            }
        }
    }

    let mut output = String::new();
    write_value(value, None, &mut output);
    output
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3)
    })
}

#[cfg(test)]
pub(crate) fn compatible_test_metadata_json() -> String {
    let layers: Vec<_> = REQUIRED_LAYERS
        .iter()
        .map(|required| {
            let fields: BTreeMap<_, _> = required
                .fields
                .iter()
                .map(|(name, field_type)| (*name, field_type.label()))
                .collect();
            serde_json::json!({
                "id": required.id,
                "minzoom": required.min_zoom,
                "maxzoom": required.max_zoom,
                "fields": fields,
            })
        })
        .collect();
    serde_json::json!({
        "name": "Protomaps Basemap",
        "version": "4.15.1",
        "vector_layers": layers,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pmtiles::{Compression, TILE_TYPE_MVT};

    fn header() -> Header {
        Header {
            spec_version: 3,
            root_offset: 127,
            root_len: 1,
            metadata_offset: 128,
            metadata_len: 1,
            leaf_offset: 129,
            tile_data_offset: 129,
            internal_compression: Compression::None,
            tile_compression: Compression::Gzip,
            tile_type: TILE_TYPE_MVT,
            min_zoom: 0,
            max_zoom: 15,
        }
    }

    fn layer(id: &str, minzoom: u8, maxzoom: u8, fields: &[(&str, &str)]) -> Value {
        serde_json::json!({
            "id": id,
            "minzoom": minzoom,
            "maxzoom": maxzoom,
            "fields": fields.iter().copied().collect::<BTreeMap<_, _>>(),
        })
    }

    fn expected_metadata() -> Value {
        serde_json::from_str(&compatible_test_metadata_json()).unwrap()
    }

    #[test]
    fn expected_version_four_metadata_is_compatible() {
        let descriptor = SourceDescriptor::from_metadata(
            "fixture",
            &header(),
            &serde_json::to_string(&expected_metadata()).unwrap(),
        )
        .unwrap();
        assert_eq!(descriptor.compatibility, Compatibility::Compatible);
        assert!(descriptor.issues.is_empty());
        assert_eq!(descriptor.layers["landcover"].max_zoom, 7);
        assert_eq!(
            descriptor.layers["boundaries"].fields["kind_detail"],
            FieldType::Number
        );
        assert_eq!(
            descriptor.layers["roads"].fields["is_bridge"],
            FieldType::Boolean
        );
        assert_eq!(
            descriptor.layers["water"].fields["bridge"],
            FieldType::String
        );
        let terrain_identity = descriptor.terrain_dataset_identity("bridges-none");
        assert!(terrain_identity.starts_with("fixture-terrain-"));
        assert!(terrain_identity.ends_with("-bridges-none"));
    }

    #[test]
    fn pinned_20260807_metadata_is_compatible() {
        let metadata = include_str!("../testdata/protomaps/20260807_metadata.json");
        assert_eq!(metadata.trim_end().len(), 8_189);
        let mut pinned_header = header();
        pinned_header.internal_compression = Compression::Gzip;
        let descriptor =
            SourceDescriptor::from_metadata("20260807", &pinned_header, metadata).unwrap();
        assert_eq!(descriptor.metadata_fingerprint, 0xf34a_c887_4e3d_6523);
        assert_eq!(descriptor.compatibility, Compatibility::Compatible);
        assert_eq!(descriptor.schema_version.as_deref(), Some("4.15.1"));
        assert_eq!(descriptor.layers.len(), 9);
        assert_eq!(descriptor.layers["buildings"].min_zoom, 11);
        assert_eq!(descriptor.layers["landcover"].max_zoom, 7);
        assert_eq!(descriptor.layers["roads"].fields.len(), 79);
        assert_eq!(
            descriptor.layers["water"].fields["bridge"],
            FieldType::String
        );
    }

    #[test]
    fn fingerprint_ignores_json_object_and_vector_layer_order() {
        let forward = expected_metadata();
        let mut reverse = expected_metadata();
        reverse["vector_layers"].as_array_mut().unwrap().reverse();
        let reverse = serde_json::json!({
            "vector_layers": reverse["vector_layers"].clone(),
            "version": "4.15.1",
            "name": "Protomaps Basemap",
        });
        let compact = SourceDescriptor::from_metadata(
            "fixture",
            &header(),
            &serde_json::to_string(&forward).unwrap(),
        )
        .unwrap();
        let formatted = SourceDescriptor::from_metadata(
            "fixture",
            &header(),
            &serde_json::to_string_pretty(&reverse).unwrap(),
        )
        .unwrap();
        assert_eq!(compact.metadata_fingerprint, formatted.metadata_fingerprint);
    }

    #[test]
    fn fingerprint_includes_normalized_archive_contract() {
        let metadata = serde_json::to_string(&expected_metadata()).unwrap();
        let baseline = SourceDescriptor::from_metadata("fixture", &header(), &metadata).unwrap();
        let mut different = header();
        different.tile_compression = Compression::None;
        let changed = SourceDescriptor::from_metadata("fixture", &different, &metadata).unwrap();
        assert_ne!(baseline.metadata_fingerprint, changed.metadata_fingerprint);

        different.tile_compression = Compression::Unsupported(99);
        let unsupported =
            SourceDescriptor::from_metadata("fixture", &different, &metadata).unwrap();
        assert_eq!(unsupported.compatibility, Compatibility::Incompatible);
        assert!(unsupported.diagnostic_summary().contains("compression 99"));
    }

    #[test]
    fn terrain_identity_excludes_context_metadata_but_includes_physical_contract_and_revision() {
        let metadata = expected_metadata();
        let baseline = SourceDescriptor::from_metadata(
            "fixture",
            &header(),
            &serde_json::to_string(&metadata).unwrap(),
        )
        .unwrap();

        let mut context_only = metadata.clone();
        context_only["future_map_theme"] = Value::String("night-v2".into());
        context_only["vector_layers"][0]["fields"]["future_boundary_label"] =
            Value::String("String".into());
        context_only
            .get_mut("vector_layers")
            .and_then(Value::as_array_mut)
            .unwrap()
            .push(layer("future_context", 0, 15, &[("kind", "String")]));
        let context_only = SourceDescriptor::from_metadata(
            "fixture",
            &header(),
            &serde_json::to_string(&context_only).unwrap(),
        )
        .unwrap();
        assert_ne!(
            baseline.metadata_fingerprint,
            context_only.metadata_fingerprint
        );
        assert_eq!(
            baseline.terrain_contract_fingerprint,
            context_only.terrain_contract_fingerprint
        );
        assert_eq!(
            baseline.terrain_dataset_identity("bridges-none"),
            context_only.terrain_dataset_identity("bridges-none")
        );

        // Names select documented local paving, while remaining optional
        // for compatibility with unnamed and generalized road fragments.
        let mut named_roads = metadata.clone();
        let roads = named_roads["vector_layers"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|layer| layer["id"] == "roads")
            .unwrap();
        roads["fields"]["name"] = Value::String("String".into());
        let named_roads = SourceDescriptor::from_metadata(
            "fixture",
            &header(),
            &serde_json::to_string(&named_roads).unwrap(),
        )
        .unwrap();
        assert_ne!(
            baseline.terrain_contract_fingerprint,
            named_roads.terrain_contract_fingerprint
        );

        let mut physical = metadata.clone();
        let buildings = physical["vector_layers"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|layer| layer["id"] == "buildings")
            .unwrap();
        buildings["fields"]["height_m"] = Value::String("Number".into());
        let physical = SourceDescriptor::from_metadata(
            "fixture",
            &header(),
            &serde_json::to_string(&physical).unwrap(),
        )
        .unwrap();
        assert_ne!(
            baseline.terrain_contract_fingerprint,
            physical.terrain_contract_fingerprint
        );
        assert_ne!(
            baseline.terrain_dataset_identity("bridges-none"),
            physical.terrain_dataset_identity("bridges-none")
        );

        let mut water_physical = metadata.clone();
        let water = water_physical["vector_layers"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|layer| layer["id"] == "water")
            .unwrap();
        water["fields"]["kind_detail"] = Value::String("Number".into());
        let water_physical = SourceDescriptor::from_metadata(
            "fixture",
            &header(),
            &serde_json::to_string(&water_physical).unwrap(),
        )
        .unwrap();
        assert_ne!(
            baseline.terrain_contract_fingerprint,
            water_physical.terrain_contract_fingerprint
        );
        assert_ne!(
            baseline.terrain_dataset_identity("bridges-none"),
            water_physical.terrain_dataset_identity("bridges-none")
        );

        let next_revision = SourceDescriptor::from_metadata(
            "next-build",
            &header(),
            &serde_json::to_string(&metadata).unwrap(),
        )
        .unwrap();
        assert_eq!(
            baseline.terrain_contract_fingerprint,
            next_revision.terrain_contract_fingerprint
        );
        assert_ne!(
            baseline.terrain_dataset_identity("bridges-none"),
            next_revision.terrain_dataset_identity("bridges-none")
        );
        assert!(
            baseline
                .diagnostic_identity("bridges-none")
                .contains("semantic-v6")
        );
    }

    #[test]
    fn additive_metadata_is_compatible_with_unknowns() {
        let mut metadata = expected_metadata();
        metadata["vector_layers"][0]["fields"]["future_field"] = Value::String("String".into());
        metadata["vector_layers"]
            .as_array_mut()
            .unwrap()
            .push(layer("future_layer", 0, 15, &[("kind", "String")]));
        let descriptor = SourceDescriptor::from_metadata(
            "additive",
            &header(),
            &serde_json::to_string(&metadata).unwrap(),
        )
        .unwrap();
        assert_eq!(
            descriptor.compatibility,
            Compatibility::CompatibleWithUnknowns
        );
        assert!(
            descriptor
                .issues
                .iter()
                .all(|issue| issue.severity == IssueSeverity::Unknown)
        );
        assert!(
            descriptor
                .issues
                .iter()
                .any(|issue| issue.field.as_deref() == Some("future_field"))
        );
        assert!(
            descriptor
                .issues
                .iter()
                .any(|issue| issue.layer.as_deref() == Some("future_layer"))
        );
    }

    #[test]
    fn missing_layer_and_wrong_required_type_are_incompatible() {
        let mut metadata = expected_metadata();
        metadata["vector_layers"]
            .as_array_mut()
            .unwrap()
            .retain(|layer| layer["id"] != "water");
        let roads = metadata["vector_layers"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|layer| layer["id"] == "roads")
            .unwrap();
        roads["fields"]["is_bridge"] = Value::String("String".into());
        let descriptor = SourceDescriptor::from_metadata(
            "broken",
            &header(),
            &serde_json::to_string(&metadata).unwrap(),
        )
        .unwrap();
        assert_eq!(descriptor.compatibility, Compatibility::Incompatible);
        assert!(
            descriptor
                .issues
                .iter()
                .any(|issue| issue.layer.as_deref() == Some("water"))
        );
        assert!(
            descriptor
                .issues
                .iter()
                .any(|issue| issue.field.as_deref() == Some("is_bridge"))
        );
        assert!(descriptor.diagnostic_summary().contains("layer water"));
    }

    #[test]
    fn malformed_metadata_reports_archive_identity() {
        let error = SourceDescriptor::from_metadata("bad-build", &header(), "{nope").unwrap_err();
        assert!(
            matches!(error, DataError::Decode(message) if message.contains("bad-build") && message.contains("malformed metadata"))
        );
    }
}
