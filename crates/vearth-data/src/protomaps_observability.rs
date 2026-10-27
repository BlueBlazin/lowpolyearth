//! Bounded source-to-consumer observability for Protomaps semantics.

use crate::mvt::{GeomType, Value};
use crate::protomaps_semantics::{
    LandcoverKind, LanduseKind, LayerFacts, NormalizedValue, SemanticTile, SourceFact,
    WaterGeometryRole, WaterKind,
};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

const LAYER_COUNT: usize = 12;
const GEOMETRY_COUNT: usize = 3;
const VALUE_CLASS_COUNT: usize = 4;
const DISCARD_REASON_COUNT: usize = 9;
const WATER_KIND_COUNT: usize = 19;
const WATER_ROLE_COUNT: usize = 4;
const WATER_OUTCOME_COUNT: usize = 5;
const TILE_OUTCOME_COUNT: usize = 5;
const UNKNOWN_EXAMPLE_LIMIT: usize = 16;
const UNKNOWN_VALUE_LIMIT: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(usize)]
pub enum SemanticLayerFamily {
    Boundaries,
    Buildings,
    Earth,
    Landcover,
    Landuse,
    Places,
    Pois,
    Roads,
    Water,
    Natural,
    PhysicalPoint,
    Other,
}

impl SemanticLayerFamily {
    pub fn from_name(name: &str) -> Self {
        match name {
            "boundaries" => Self::Boundaries,
            "buildings" => Self::Buildings,
            "earth" => Self::Earth,
            "landcover" => Self::Landcover,
            "landuse" => Self::Landuse,
            "places" => Self::Places,
            "pois" => Self::Pois,
            "roads" => Self::Roads,
            "water" => Self::Water,
            "natural" => Self::Natural,
            "physical_point" => Self::PhysicalPoint,
            _ => Self::Other,
        }
    }

    pub const ALL: [Self; LAYER_COUNT] = [
        Self::Boundaries,
        Self::Buildings,
        Self::Earth,
        Self::Landcover,
        Self::Landuse,
        Self::Places,
        Self::Pois,
        Self::Roads,
        Self::Water,
        Self::Natural,
        Self::PhysicalPoint,
        Self::Other,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Boundaries => "boundaries",
            Self::Buildings => "buildings",
            Self::Earth => "earth",
            Self::Landcover => "landcover",
            Self::Landuse => "landuse",
            Self::Places => "places",
            Self::Pois => "pois",
            Self::Roads => "roads",
            Self::Water => "water",
            Self::Natural => "natural",
            Self::PhysicalPoint => "physical_point",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum SemanticValueClass {
    Missing,
    Known,
    Unknown,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum SemanticDiscardReason {
    UnsupportedGeometry,
    UnsupportedSemantic,
    RankOrMinimumZoom,
    InvalidValue,
    QueryCap,
    ConsumerBudget,
    ContextOnly,
    SurfaceIneligible,
    DeferredPhysical,
}

impl SemanticDiscardReason {
    pub const ALL: [Self; DISCARD_REASON_COUNT] = [
        Self::UnsupportedGeometry,
        Self::UnsupportedSemantic,
        Self::RankOrMinimumZoom,
        Self::InvalidValue,
        Self::QueryCap,
        Self::ConsumerBudget,
        Self::ContextOnly,
        Self::SurfaceIneligible,
        Self::DeferredPhysical,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::UnsupportedGeometry => "unsupported_geometry",
            Self::UnsupportedSemantic => "unsupported_semantic",
            Self::RankOrMinimumZoom => "rank_or_minimum_zoom",
            Self::InvalidValue => "invalid_value",
            Self::QueryCap => "query_cap",
            Self::ConsumerBudget => "consumer_budget",
            Self::ContextOnly => "context_only",
            Self::SurfaceIneligible => "surface_ineligible",
            Self::DeferredPhysical => "deferred_physical",
        }
    }
}

/// Bounded, feature-aware hydrography outcomes. `Unknown` and `Invalid` are
/// diagnostic dimensions and may accompany one of the consumer outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum WaterSemanticOutcome {
    Consumed,
    Filtered,
    ContextOnly,
    Unknown,
    Invalid,
}

impl WaterSemanticOutcome {
    pub const ALL: [Self; WATER_OUTCOME_COUNT] = [
        Self::Consumed,
        Self::Filtered,
        Self::ContextOnly,
        Self::Unknown,
        Self::Invalid,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Consumed => "consumed",
            Self::Filtered => "filtered",
            Self::ContextOnly => "context_only",
            Self::Unknown => "unknown",
            Self::Invalid => "invalid",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum SemanticTileOutcome {
    Present,
    Absent,
    FetchFailure,
    DecodeFailure,
    QueryCap,
}

impl SemanticTileOutcome {
    pub const ALL: [Self; TILE_OUTCOME_COUNT] = [
        Self::Present,
        Self::Absent,
        Self::FetchFailure,
        Self::DecodeFailure,
        Self::QueryCap,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Absent => "absent",
            Self::FetchFailure => "fetch_failure",
            Self::DecodeFailure => "decode_failure",
            Self::QueryCap => "query_cap",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownSemanticExample {
    pub layer: SemanticLayerFamily,
    pub field: &'static str,
    pub value: String,
    pub count: u64,
}

#[derive(Debug, Clone)]
pub struct SemanticConsumerReport {
    enabled: bool,
    admitted: [u64; LAYER_COUNT],
    consumed: [u64; LAYER_COUNT],
    rejected: [u64; LAYER_COUNT],
    discarded: [u64; LAYER_COUNT],
    discard_reasons: [u64; LAYER_COUNT * DISCARD_REASON_COUNT],
    water_outcomes: [u64; WATER_KIND_COUNT * WATER_ROLE_COUNT * WATER_OUTCOME_COUNT],
}

impl Default for SemanticConsumerReport {
    fn default() -> Self {
        Self {
            enabled: true,
            admitted: [0; LAYER_COUNT],
            consumed: [0; LAYER_COUNT],
            rejected: [0; LAYER_COUNT],
            discarded: [0; LAYER_COUNT],
            discard_reasons: [0; LAYER_COUNT * DISCARD_REASON_COUNT],
            water_outcomes: [0; WATER_KIND_COUNT * WATER_ROLE_COUNT * WATER_OUTCOME_COUNT],
        }
    }
}

impl SemanticConsumerReport {
    pub fn water_outcome(
        &mut self,
        kind: WaterKind,
        role: WaterGeometryRole,
        outcome: WaterSemanticOutcome,
    ) {
        if !self.enabled {
            return;
        }
        self.water_outcomes[water_outcome_index(kind, role, outcome)] += 1;
    }

    pub fn consumed(&mut self, layer: SemanticLayerFamily) {
        if !self.enabled {
            return;
        }
        self.admitted[layer as usize] += 1;
        self.consumed[layer as usize] += 1;
    }

    pub fn discarded(&mut self, layer: SemanticLayerFamily, reason: SemanticDiscardReason) {
        if !self.enabled {
            return;
        }
        self.discarded[layer as usize] += 1;
        self.discard_reasons[layer as usize * DISCARD_REASON_COUNT + reason as usize] += 1;
    }

    pub fn rejected(&mut self, layer: SemanticLayerFamily, reason: SemanticDiscardReason) {
        if !self.enabled {
            return;
        }
        self.rejected[layer as usize] += 1;
        self.discard_reasons[layer as usize * DISCARD_REASON_COUNT + reason as usize] += 1;
    }

    pub(crate) fn discard_consumed(
        &mut self,
        layer: SemanticLayerFamily,
        reason: SemanticDiscardReason,
    ) {
        if !self.enabled {
            return;
        }
        let count = self.consumed[layer as usize];
        self.admitted[layer as usize] -= count;
        self.consumed[layer as usize] = 0;
        self.discarded[layer as usize] += count;
        self.discard_reasons[layer as usize * DISCARD_REASON_COUNT + reason as usize] += count;
    }

    pub(crate) fn discard_one_consumed(
        &mut self,
        layer: SemanticLayerFamily,
        reason: SemanticDiscardReason,
    ) {
        if !self.enabled {
            return;
        }
        debug_assert!(self.consumed[layer as usize] > 0);
        self.admitted[layer as usize] = self.admitted[layer as usize].saturating_sub(1);
        self.consumed[layer as usize] = self.consumed[layer as usize].saturating_sub(1);
        self.discarded[layer as usize] += 1;
        self.discard_reasons[layer as usize * DISCARD_REASON_COUNT + reason as usize] += 1;
    }

    pub(crate) fn reclassify_water_outcome(
        &mut self,
        kind: WaterKind,
        role: WaterGeometryRole,
        from: WaterSemanticOutcome,
        to: WaterSemanticOutcome,
    ) {
        if !self.enabled {
            return;
        }
        let from_index = water_outcome_index(kind, role, from);
        debug_assert!(self.water_outcomes[from_index] > 0);
        self.water_outcomes[from_index] = self.water_outcomes[from_index].saturating_sub(1);
        self.water_outcomes[water_outcome_index(kind, role, to)] += 1;
    }
}

struct SemanticTotals {
    source: [u64; LAYER_COUNT * GEOMETRY_COUNT],
    normalized: [u64; LAYER_COUNT * GEOMETRY_COUNT],
    kind_values: [u64; LAYER_COUNT * VALUE_CLASS_COUNT],
    detail_values: [u64; LAYER_COUNT * VALUE_CLASS_COUNT],
    admitted: [u64; LAYER_COUNT],
    consumed: [u64; LAYER_COUNT],
    rejected: [u64; LAYER_COUNT],
    discarded: [u64; LAYER_COUNT],
    discard_reasons: [u64; LAYER_COUNT * DISCARD_REASON_COUNT],
    water_outcomes: [u64; WATER_KIND_COUNT * WATER_ROLE_COUNT * WATER_OUTCOME_COUNT],
    tile_outcomes: [u64; TILE_OUTCOME_COUNT],
    unknown_examples: Vec<UnknownSemanticExample>,
    unknown_overflow: u64,
    known_values: BTreeMap<(SemanticLayerFamily, &'static str, &'static str), u64>,
}

impl Default for SemanticTotals {
    fn default() -> Self {
        Self {
            source: [0; LAYER_COUNT * GEOMETRY_COUNT],
            normalized: [0; LAYER_COUNT * GEOMETRY_COUNT],
            kind_values: [0; LAYER_COUNT * VALUE_CLASS_COUNT],
            detail_values: [0; LAYER_COUNT * VALUE_CLASS_COUNT],
            admitted: [0; LAYER_COUNT],
            consumed: [0; LAYER_COUNT],
            rejected: [0; LAYER_COUNT],
            discarded: [0; LAYER_COUNT],
            discard_reasons: [0; LAYER_COUNT * DISCARD_REASON_COUNT],
            water_outcomes: [0; WATER_KIND_COUNT * WATER_ROLE_COUNT * WATER_OUTCOME_COUNT],
            tile_outcomes: [0; TILE_OUTCOME_COUNT],
            unknown_examples: Vec::new(),
            unknown_overflow: 0,
            known_values: BTreeMap::new(),
        }
    }
}

pub struct SemanticTelemetry {
    build_tag: String,
    metadata_fingerprint: u64,
    enabled: AtomicBool,
    totals: Mutex<SemanticTotals>,
}

impl SemanticTelemetry {
    pub fn new(build_tag: impl Into<String>, metadata_fingerprint: u64) -> Self {
        Self {
            build_tag: build_tag.into(),
            metadata_fingerprint,
            enabled: AtomicBool::new(false),
            totals: Mutex::new(SemanticTotals::default()),
        }
    }

    pub fn new_enabled(build_tag: impl Into<String>, metadata_fingerprint: u64) -> Self {
        let telemetry = Self::new(build_tag, metadata_fingerprint);
        telemetry.set_enabled(true);
        telemetry
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn consumer_report(&self) -> SemanticConsumerReport {
        SemanticConsumerReport {
            enabled: self.enabled(),
            ..SemanticConsumerReport::default()
        }
    }

    pub fn record_normalized_tile(&self, tile: &SemanticTile) {
        if !self.enabled() {
            return;
        }
        let mut local = SemanticTotals::default();
        for layer in tile.semantic_layers() {
            let family = SemanticLayerFamily::from_name(&layer.name);
            for feature in &layer.features {
                let geometry = geometry_index(feature.geometry_type);
                local.source[family as usize * GEOMETRY_COUNT + geometry] += 1;
                local.normalized[family as usize * GEOMETRY_COUNT + geometry] += 1;
                let (kind, detail) = feature.kind_and_detail();
                record_value_class(&mut local.kind_values, family, value_class(kind));
                record_value_class(&mut local.detail_values, family, value_class(detail));
                record_example(&mut local, tile, layer, "kind", kind);
                record_example(&mut local, tile, layer, "kind_detail", detail);
                record_known_value(&mut local, tile, layer, family, "kind", kind);
                record_known_value(&mut local, tile, layer, family, "kind_detail", detail);
                if let LayerFacts::Water(facts) = &feature.facts {
                    let kind = facts.class.unwrap_or(WaterKind::Unknown);
                    if facts.class == Some(WaterKind::Unknown)
                        || facts.detail_class
                            == Some(crate::protomaps_semantics::WaterKindDetail::Unknown)
                    {
                        local.water_outcomes[water_outcome_index(
                            kind,
                            facts.geometry_role,
                            WaterSemanticOutcome::Unknown,
                        )] += 1;
                    }
                    let invalid = [
                        facts.physical.kind,
                        facts.physical.kind_detail,
                        facts.physical.bridge,
                        facts.physical.tunnel,
                        facts.physical.layer,
                        facts.physical.intermittent,
                        facts.physical.reservoir,
                        facts.physical.alkaline,
                    ]
                    .into_iter()
                    .flatten()
                    .any(|fact| fact.normalized == NormalizedValue::Invalid);
                    if invalid {
                        local.water_outcomes[water_outcome_index(
                            kind,
                            facts.geometry_role,
                            WaterSemanticOutcome::Invalid,
                        )] += 1;
                    }
                }
            }
        }
        local.tile_outcomes[SemanticTileOutcome::Present as usize] = 1;
        self.merge(local);
    }

    pub fn record_consumer(&self, report: &SemanticConsumerReport) {
        if !self.enabled() || !report.enabled {
            return;
        }
        let mut totals = self.totals.lock().unwrap();
        add_arrays(&mut totals.admitted, &report.admitted);
        add_arrays(&mut totals.consumed, &report.consumed);
        add_arrays(&mut totals.rejected, &report.rejected);
        add_arrays(&mut totals.discarded, &report.discarded);
        add_arrays(&mut totals.discard_reasons, &report.discard_reasons);
        add_arrays(&mut totals.water_outcomes, &report.water_outcomes);
    }

    pub fn record_tile_outcome(&self, outcome: SemanticTileOutcome) {
        if !self.enabled() {
            return;
        }
        self.totals.lock().unwrap().tile_outcomes[outcome as usize] += 1;
    }

    pub fn record_query_cap(&self, layer: SemanticLayerFamily) {
        if !self.enabled() {
            return;
        }
        let mut totals = self.totals.lock().unwrap();
        totals.tile_outcomes[SemanticTileOutcome::QueryCap as usize] += 1;
        totals.discard_reasons
            [layer as usize * DISCARD_REASON_COUNT + SemanticDiscardReason::QueryCap as usize] += 1;
    }

    pub fn snapshot(&self) -> SemanticReportSnapshot {
        let totals = self.totals.lock().unwrap();
        SemanticReportSnapshot {
            build_tag: self.build_tag.clone(),
            semantic_schema_version: crate::protomaps_semantics::SEMANTIC_SCHEMA_VERSION,
            metadata_fingerprint: self.metadata_fingerprint,
            source: totals.source,
            normalized: totals.normalized,
            kind_values: totals.kind_values,
            detail_values: totals.detail_values,
            admitted: totals.admitted,
            consumed: totals.consumed,
            rejected: totals.rejected,
            discarded: totals.discarded,
            discard_reasons: totals.discard_reasons,
            water_outcomes: totals.water_outcomes,
            tile_outcomes: totals.tile_outcomes,
            unknown_examples: totals.unknown_examples.clone(),
            unknown_overflow: totals.unknown_overflow,
            known_values: totals.known_values.clone(),
        }
    }

    fn merge(&self, local: SemanticTotals) {
        let mut totals = self.totals.lock().unwrap();
        add_arrays(&mut totals.source, &local.source);
        add_arrays(&mut totals.normalized, &local.normalized);
        add_arrays(&mut totals.kind_values, &local.kind_values);
        add_arrays(&mut totals.detail_values, &local.detail_values);
        add_arrays(&mut totals.water_outcomes, &local.water_outcomes);
        add_arrays(&mut totals.tile_outcomes, &local.tile_outcomes);
        for example in local.unknown_examples {
            if let Some(existing) = totals.unknown_examples.iter_mut().find(|existing| {
                existing.layer == example.layer
                    && existing.field == example.field
                    && existing.value == example.value
            }) {
                existing.count += example.count;
            } else if totals.unknown_examples.len() < UNKNOWN_EXAMPLE_LIMIT {
                totals.unknown_examples.push(example);
            } else {
                totals.unknown_overflow += example.count;
            }
        }
        totals.unknown_overflow += local.unknown_overflow;
        for (key, count) in local.known_values {
            *totals.known_values.entry(key).or_default() += count;
        }
    }
}

#[derive(Debug, Clone)]
pub struct SemanticReportSnapshot {
    pub build_tag: String,
    pub semantic_schema_version: u32,
    pub metadata_fingerprint: u64,
    source: [u64; LAYER_COUNT * GEOMETRY_COUNT],
    normalized: [u64; LAYER_COUNT * GEOMETRY_COUNT],
    kind_values: [u64; LAYER_COUNT * VALUE_CLASS_COUNT],
    detail_values: [u64; LAYER_COUNT * VALUE_CLASS_COUNT],
    admitted: [u64; LAYER_COUNT],
    consumed: [u64; LAYER_COUNT],
    rejected: [u64; LAYER_COUNT],
    discarded: [u64; LAYER_COUNT],
    discard_reasons: [u64; LAYER_COUNT * DISCARD_REASON_COUNT],
    water_outcomes: [u64; WATER_KIND_COUNT * WATER_ROLE_COUNT * WATER_OUTCOME_COUNT],
    tile_outcomes: [u64; TILE_OUTCOME_COUNT],
    pub unknown_examples: Vec<UnknownSemanticExample>,
    pub unknown_overflow: u64,
    known_values: BTreeMap<(SemanticLayerFamily, &'static str, &'static str), u64>,
}

impl SemanticReportSnapshot {
    pub fn layer_geometry_source(&self, layer: SemanticLayerFamily, geometry: GeomType) -> u64 {
        self.source[layer as usize * GEOMETRY_COUNT + geometry_index(geometry)]
    }

    pub fn layer_source(&self, layer: SemanticLayerFamily) -> u64 {
        let start = layer as usize * GEOMETRY_COUNT;
        self.source[start..start + GEOMETRY_COUNT].iter().sum()
    }

    pub fn layer_normalized(&self, layer: SemanticLayerFamily) -> u64 {
        let start = layer as usize * GEOMETRY_COUNT;
        self.normalized[start..start + GEOMETRY_COUNT].iter().sum()
    }

    pub fn layer_consumed(&self, layer: SemanticLayerFamily) -> u64 {
        self.consumed[layer as usize]
    }

    pub fn layer_admitted(&self, layer: SemanticLayerFamily) -> u64 {
        self.admitted[layer as usize]
    }

    pub fn layer_discarded(&self, layer: SemanticLayerFamily) -> u64 {
        self.discarded[layer as usize]
    }

    pub fn layer_rejected(&self, layer: SemanticLayerFamily) -> u64 {
        self.rejected[layer as usize]
    }

    pub fn value_count(
        &self,
        layer: SemanticLayerFamily,
        detail: bool,
        class: SemanticValueClass,
    ) -> u64 {
        let values = if detail {
            &self.detail_values
        } else {
            &self.kind_values
        };
        values[layer as usize * VALUE_CLASS_COUNT + class as usize]
    }

    pub fn discard_count(&self, layer: SemanticLayerFamily, reason: SemanticDiscardReason) -> u64 {
        self.discard_reasons[layer as usize * DISCARD_REASON_COUNT + reason as usize]
    }

    pub fn water_outcome_count(
        &self,
        kind: WaterKind,
        role: WaterGeometryRole,
        outcome: WaterSemanticOutcome,
    ) -> u64 {
        self.water_outcomes[water_outcome_index(kind, role, outcome)]
    }

    pub fn tile_outcome(&self, outcome: SemanticTileOutcome) -> u64 {
        self.tile_outcomes[outcome as usize]
    }

    pub fn known_value_count(
        &self,
        layer: SemanticLayerFamily,
        field: &'static str,
        value: &'static str,
    ) -> u64 {
        self.known_values
            .get(&(layer, field, value))
            .copied()
            .unwrap_or(0)
    }
}

fn water_outcome_index(
    kind: WaterKind,
    role: WaterGeometryRole,
    outcome: WaterSemanticOutcome,
) -> usize {
    (kind as usize * WATER_ROLE_COUNT + role as usize) * WATER_OUTCOME_COUNT + outcome as usize
}

impl fmt::Display for SemanticReportSnapshot {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            output,
            "protomaps build={} semantic=v{} metadata={:016x}",
            self.build_tag, self.semantic_schema_version, self.metadata_fingerprint
        )?;
        for layer in SemanticLayerFamily::ALL {
            let source = self.layer_source(layer);
            let normalized = self.layer_normalized(layer);
            let admitted = self.layer_admitted(layer);
            let consumed = self.layer_consumed(layer);
            let rejected = self.layer_rejected(layer);
            let discarded = self.layer_discarded(layer);
            if source + normalized + admitted + consumed + rejected + discarded > 0 {
                writeln!(
                    output,
                    "{}: source={} [point={} line={} polygon={}] normalized={} admitted={} consumed={} rejected={} discarded={} kind[k/u/i]={}/{}/{} detail[k/u/i]={}/{}/{}",
                    layer.label(),
                    source,
                    self.layer_geometry_source(layer, GeomType::Point),
                    self.layer_geometry_source(layer, GeomType::Line),
                    self.layer_geometry_source(layer, GeomType::Polygon),
                    normalized,
                    admitted,
                    consumed,
                    rejected,
                    discarded,
                    self.value_count(layer, false, SemanticValueClass::Known),
                    self.value_count(layer, false, SemanticValueClass::Unknown),
                    self.value_count(layer, false, SemanticValueClass::Invalid),
                    self.value_count(layer, true, SemanticValueClass::Known),
                    self.value_count(layer, true, SemanticValueClass::Unknown),
                    self.value_count(layer, true, SemanticValueClass::Invalid),
                )?;
                let reason_total: u64 = SemanticDiscardReason::ALL
                    .iter()
                    .map(|&reason| self.discard_count(layer, reason))
                    .sum();
                if reason_total > 0 {
                    write!(output, "  reasons:")?;
                    for reason in SemanticDiscardReason::ALL {
                        let count = self.discard_count(layer, reason);
                        if count > 0 {
                            write!(output, " {}={}", reason.label(), count)?;
                        }
                    }
                    writeln!(output)?;
                }
            }
        }
        for kind in WaterKind::ALL {
            for role in WaterGeometryRole::ALL {
                for outcome in WaterSemanticOutcome::ALL {
                    let count = self.water_outcome_count(kind, role, outcome);
                    if count > 0 {
                        writeln!(
                            output,
                            "water.{}.{}.{}={}",
                            kind.label(),
                            role.label(),
                            outcome.label(),
                            count
                        )?;
                    }
                }
            }
        }
        write!(output, "tiles:")?;
        for outcome in SemanticTileOutcome::ALL {
            write!(
                output,
                " {}={}",
                outcome.label(),
                self.tile_outcome(outcome)
            )?;
        }
        for example in &self.unknown_examples {
            write!(
                output,
                "\nunknown {}.{}={} count={}",
                example.layer.label(),
                example.field,
                example.value,
                example.count
            )?;
        }
        if self.unknown_overflow > 0 {
            write!(output, "\nunknown_overflow={}", self.unknown_overflow)?;
        }
        Ok(())
    }
}

fn geometry_index(geometry: GeomType) -> usize {
    match geometry {
        GeomType::Point => 0,
        GeomType::Line => 1,
        GeomType::Polygon => 2,
    }
}

fn value_class(fact: Option<SourceFact>) -> SemanticValueClass {
    match fact.map(|fact| fact.normalized) {
        None => SemanticValueClass::Missing,
        Some(NormalizedValue::UnknownText) => SemanticValueClass::Unknown,
        Some(NormalizedValue::Invalid) => SemanticValueClass::Invalid,
        Some(_) => SemanticValueClass::Known,
    }
}

fn record_value_class(
    counts: &mut [u64; LAYER_COUNT * VALUE_CLASS_COUNT],
    layer: SemanticLayerFamily,
    class: SemanticValueClass,
) {
    counts[layer as usize * VALUE_CLASS_COUNT + class as usize] += 1;
}

fn record_example(
    totals: &mut SemanticTotals,
    tile: &SemanticTile,
    layer: &crate::protomaps_semantics::SemanticLayer,
    field: &'static str,
    fact: Option<SourceFact>,
) {
    let Some(fact) = fact else { return };
    if !matches!(
        fact.normalized,
        NormalizedValue::UnknownText | NormalizedValue::Invalid
    ) {
        return;
    }
    let value = match tile.source_value(layer, fact.source) {
        Some(Value::Str(value)) => bounded_value(value),
        Some(value) => bounded_value(&format!("{value:?}")),
        None => "<invalid-index>".to_string(),
    };
    let family = SemanticLayerFamily::from_name(&layer.name);
    if let Some(existing) = totals.unknown_examples.iter_mut().find(|existing| {
        existing.layer == family && existing.field == field && existing.value == value
    }) {
        existing.count += 1;
    } else if totals.unknown_examples.len() < UNKNOWN_EXAMPLE_LIMIT {
        totals.unknown_examples.push(UnknownSemanticExample {
            layer: family,
            field,
            value,
            count: 1,
        });
    } else {
        totals.unknown_overflow += 1;
    }
}

fn record_known_value(
    totals: &mut SemanticTotals,
    tile: &SemanticTile,
    layer: &crate::protomaps_semantics::SemanticLayer,
    family: SemanticLayerFamily,
    field: &'static str,
    fact: Option<SourceFact>,
) {
    let Some(fact) = fact else { return };
    if fact.normalized != NormalizedValue::Text {
        return;
    }
    let Some(value) = tile.source_text(layer, fact) else {
        return;
    };
    if let Some(value) = known_value(family, field, value) {
        *totals
            .known_values
            .entry((family, field, value))
            .or_default() += 1;
    }
}

fn known_value(layer: SemanticLayerFamily, field: &str, value: &str) -> Option<&'static str> {
    use SemanticLayerFamily as Layer;
    match (layer, field, value) {
        (Layer::Boundaries, "kind", "country") => Some("country"),
        (Layer::Boundaries, "kind", "region") => Some("region"),
        (Layer::Boundaries, "kind", "county") => Some("county"),
        (Layer::Boundaries, "kind", "locality") => Some("locality"),
        (Layer::Buildings, "kind", "building") => Some("building"),
        (Layer::Buildings, "kind", "building_part") => Some("building_part"),
        (Layer::Buildings, "kind", "part") => Some("part"),
        (Layer::Buildings, "kind", "address") => Some("address"),
        (Layer::Buildings, "kind_detail", "roof") => Some("roof"),
        (Layer::Earth, "kind", "land") => Some("land"),
        (Layer::Earth, "kind", "cliff") => Some("cliff"),
        (Layer::Landcover, "kind", value) => LandcoverKind::known_source(value),
        (Layer::Landuse, "kind", value) => LanduseKind::known_source(value),
        (Layer::Places, "kind", "locality") => Some("locality"),
        (Layer::Places, "kind", "region") => Some("region"),
        (Layer::Places, "kind", "country") => Some("country"),
        (Layer::Places, "kind_detail", value) => match value {
            "city" => Some("city"),
            "town" => Some("town"),
            "village" => Some("village"),
            "hamlet" => Some("hamlet"),
            "state" => Some("state"),
            "province" => Some("province"),
            "county" => Some("county"),
            _ => None,
        },
        (Layer::Pois, "kind", value) => match value {
            "viewpoint" => Some("viewpoint"),
            "attraction" => Some("attraction"),
            "restaurant" => Some("restaurant"),
            "tree" => Some("tree"),
            _ => None,
        },
        (Layer::Roads, "kind", value) => match value {
            "highway" => Some("highway"),
            "major_road" => Some("major_road"),
            "medium_road" => Some("medium_road"),
            "minor_road" => Some("minor_road"),
            "path" => Some("path"),
            "rail" => Some("rail"),
            "aeroway" => Some("aeroway"),
            "ferry" => Some("ferry"),
            _ => None,
        },
        (Layer::Roads, "kind_detail", value) => match value {
            "motorway" => Some("motorway"),
            "motorway_link" => Some("motorway_link"),
            "trunk" => Some("trunk"),
            "trunk_link" => Some("trunk_link"),
            "primary" => Some("primary"),
            "primary_link" => Some("primary_link"),
            "secondary" => Some("secondary"),
            "secondary_link" => Some("secondary_link"),
            "tertiary" => Some("tertiary"),
            "tertiary_link" => Some("tertiary_link"),
            "service" => Some("service"),
            "driveway" => Some("driveway"),
            "parking_aisle" => Some("parking_aisle"),
            "alley" => Some("alley"),
            "drive-through" => Some("drive-through"),
            "emergency_access" => Some("emergency_access"),
            "subway" => Some("subway"),
            _ => None,
        },
        (Layer::Water, "kind", value) => match value {
            "ocean" => Some("ocean"),
            "lake" => Some("lake"),
            "river" => Some("river"),
            "canal" => Some("canal"),
            "stream" => Some("stream"),
            "water" => Some("water"),
            _ => None,
        },
        (Layer::Water, "kind_detail", value) => match value {
            "river" => Some("river"),
            "canal" => Some("canal"),
            "stream" => Some("stream"),
            _ => None,
        },
        _ => None,
    }
}

fn bounded_value(value: &str) -> String {
    value.chars().take(UNKNOWN_VALUE_LIMIT).collect()
}

fn add_arrays<const N: usize>(target: &mut [u64; N], source: &[u64; N]) {
    for (target, source) in target.iter_mut().zip(source) {
        *target += *source;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mvt;
    use crate::mvt::encode::FeatureSpec;

    #[test]
    fn unknown_examples_have_fixed_count_and_value_bounds() {
        let keys = ["kind"];
        let values: Vec<_> = (0..24)
            .map(|index| Value::Str(format!("unknown-{index}-{}", "x".repeat(100))))
            .collect();
        let features: Vec<_> = (0..24)
            .map(|index| FeatureSpec {
                id: Some(index),
                geom_type: GeomType::Polygon,
                paths: vec![vec![[0, 0], [10, 0], [10, 10]]],
                tags: vec![(0, index as u32)],
            })
            .collect();
        let bytes = mvt::encode::tile("water", 4096, &keys, &values, &features);
        let tile = SemanticTile::normalize_at(mvt::decode_tile(&bytes).unwrap(), (7, 1, 1));
        let telemetry = SemanticTelemetry::new_enabled("bounded", 1);
        telemetry.record_normalized_tile(&tile);
        telemetry.record_normalized_tile(&tile);
        let snapshot = telemetry.snapshot();
        assert_eq!(snapshot.unknown_examples.len(), UNKNOWN_EXAMPLE_LIMIT);
        assert_eq!(snapshot.unknown_overflow, 16);
        assert!(
            snapshot
                .unknown_examples
                .iter()
                .all(|example| example.value.chars().count() <= UNKNOWN_VALUE_LIMIT)
        );
        assert_eq!(
            snapshot.value_count(
                SemanticLayerFamily::Water,
                false,
                SemanticValueClass::Unknown,
            ),
            48
        );
    }

    #[test]
    fn observability_is_disabled_and_empty_by_default() {
        let keys = ["kind"];
        let values = [Value::Str("mystery".to_string())];
        let features = [FeatureSpec {
            id: Some(1),
            geom_type: GeomType::Polygon,
            paths: vec![vec![[0, 0], [10, 0], [10, 10]]],
            tags: vec![(0, 0)],
        }];
        let bytes = mvt::encode::tile("water", 4096, &keys, &values, &features);
        let tile = SemanticTile::normalize_at(mvt::decode_tile(&bytes).unwrap(), (7, 1, 1));
        let telemetry = SemanticTelemetry::new("disabled", 1);
        telemetry.record_normalized_tile(&tile);
        telemetry.record_tile_outcome(SemanticTileOutcome::Absent);
        telemetry.record_query_cap(SemanticLayerFamily::Water);
        let mut consumer = telemetry.consumer_report();
        consumer.consumed(SemanticLayerFamily::Water);
        telemetry.record_consumer(&consumer);

        let snapshot = telemetry.snapshot();
        assert_eq!(snapshot.layer_source(SemanticLayerFamily::Water), 0);
        assert_eq!(snapshot.tile_outcome(SemanticTileOutcome::Absent), 0);
        assert_eq!(snapshot.tile_outcome(SemanticTileOutcome::QueryCap), 0);
        assert!(snapshot.unknown_examples.is_empty());
    }
}
