//! Renderer capability selection and benchmark identity contracts.
//!
//! Production has one content policy on every supported adapter. The only GPU
//! branch is execution-equivalent whole-mesh submission: indirect
//! first-instance when available, otherwise direct draws. Timestamp queries
//! affect telemetry only.
//!
//! Capacity is not capability-dependent. Both terrain submission branches use
//! the same admitted resident set and report any hard draw or flora-budget
//! suppression through the stable render telemetry. Cache incompatibility is
//! also content-neutral: generation and column-codec boundaries rebuild only
//! the compact cached layer they own.

use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuCapabilities {
    pub indirect_first_instance: bool,
    pub timestamp_queries: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainSubmission {
    Direct,
    IndirectFirstInstance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FinalPipelinePolicy {
    pub terrain_submission: TerrainSubmission,
    pub timestamp_telemetry: bool,
}

impl FinalPipelinePolicy {
    pub const fn for_gpu(capabilities: GpuCapabilities) -> Self {
        Self {
            terrain_submission: if capabilities.indirect_first_instance {
                TerrainSubmission::IndirectFirstInstance
            } else {
                TerrainSubmission::Direct
            },
            timestamp_telemetry: capabilities.timestamp_queries,
        }
    }

    pub const fn capability_class(self) -> &'static str {
        match self.terrain_submission {
            TerrainSubmission::Direct => "direct-whole-mesh",
            TerrainSubmission::IndirectFirstInstance => "indirect-whole-mesh",
        }
    }
}

/// Identity that must match before two reports can be called an A/B pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BenchmarkIdentity {
    pub fixture_manifest_fingerprint: String,
    pub source_dataset: String,
    pub generation_version: u32,
    pub surface_codec_version: u32,
    pub render_schema: u32,
    pub sample_count: u32,
    pub budgets: BTreeMap<String, u64>,
}

pub fn ensure_comparable(a: &BenchmarkIdentity, b: &BenchmarkIdentity) -> Result<(), String> {
    (a == b).then_some(()).ok_or_else(|| {
        "A/B reports use different fixtures, source identity, schema, samples, or budgets".into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_capabilities_change_execution_but_not_content_policy() {
        for timestamp_queries in [false, true] {
            let direct = FinalPipelinePolicy::for_gpu(GpuCapabilities {
                indirect_first_instance: false,
                timestamp_queries,
            });
            let indirect = FinalPipelinePolicy::for_gpu(GpuCapabilities {
                indirect_first_instance: true,
                timestamp_queries,
            });
            assert_eq!(direct.terrain_submission, TerrainSubmission::Direct);
            assert_eq!(
                indirect.terrain_submission,
                TerrainSubmission::IndirectFirstInstance
            );
            assert_eq!(
                FinalPipelinePolicy {
                    terrain_submission: TerrainSubmission::IndirectFirstInstance,
                    ..direct
                },
                indirect
            );
        }
    }

    #[test]
    fn compatibility_versions_name_independent_boundaries() {
        assert_eq!(vearth_gen::GEN_VERSION, 211);
        assert_eq!(crate::chunk_cache::SURFACE_CODEC_VERSION, 4);
        assert_eq!(vearth_core::mesh::RENDER_SCHEMA_VERSION, 4);
    }

    #[test]
    fn ab_identity_rejects_changed_fixture_or_budget() {
        let base = BenchmarkIdentity {
            fixture_manifest_fingerprint: "abc".into(),
            source_dataset: "offline-v1".into(),
            generation_version: 108,
            surface_codec_version: 1,
            render_schema: 1,
            sample_count: 9,
            budgets: BTreeMap::from([("flora_glyphs".into(), 100_000)]),
        };
        assert!(ensure_comparable(&base, &base).is_ok());
        let mut changed = base.clone();
        changed.sample_count += 1;
        assert!(ensure_comparable(&base, &changed).is_err());
    }
}
