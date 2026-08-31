use crate::{IdentityMatch, SpeakerEmbeddingObservation, SpeakerLibraryError, SpeakerProfile};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy)]
pub struct IdentityResolverConfig {
    pub default_threshold: f32,
    pub ambiguity_margin: f32,
}

impl Default for IdentityResolverConfig {
    fn default() -> Self {
        Self {
            default_threshold: 0.72,
            ambiguity_margin: 0.03,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ClusterIdentityResolver {
    config: IdentityResolverConfig,
}

impl ClusterIdentityResolver {
    pub fn new(config: IdentityResolverConfig) -> Self {
        Self { config }
    }

    pub fn resolve(
        &self,
        observations: &[SpeakerEmbeddingObservation],
        profiles: &[SpeakerProfile],
    ) -> Result<Vec<IdentityMatch>, SpeakerLibraryError> {
        let centroids = observation_centroids(observations);
        let mut candidates = Vec::new();
        for (cluster_id, (space, centroid)) in centroids {
            let mut cluster_candidates = profiles
                .iter()
                .filter(|profile| profile.embedding_space.compatible_with(&space))
                .filter_map(|profile| {
                    profile_centroid(profile).map(|profile_vector| {
                        let score = cosine_similarity(&centroid, &profile_vector);
                        let threshold = profile.match_threshold.max(self.config.default_threshold);
                        (profile, score, threshold)
                    })
                })
                .filter(|(_, score, threshold)| *score >= *threshold)
                .collect::<Vec<_>>();
            cluster_candidates.sort_by(|left, right| right.1.total_cmp(&left.1));
            let Some((profile, score, threshold)) = cluster_candidates.first().copied() else {
                continue;
            };
            if let Some((_, second_score, _)) = cluster_candidates.get(1) {
                if score - *second_score < self.config.ambiguity_margin {
                    continue;
                }
            }
            candidates.push(IdentityMatch {
                cluster_id,
                profile_id: profile.id.clone(),
                display_name: profile.display_name.clone(),
                score,
                threshold,
            });
        }

        candidates.sort_by(|left, right| right.score.total_cmp(&left.score));
        let mut used_profiles = BTreeSet::new();
        let mut used_clusters = BTreeSet::new();
        candidates.retain(|candidate| {
            used_profiles.insert(candidate.profile_id.clone())
                && used_clusters.insert(candidate.cluster_id)
        });
        candidates.sort_by_key(|candidate| candidate.cluster_id);
        Ok(candidates)
    }
}

pub fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return f32::NEG_INFINITY;
    }
    let dot = left
        .iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum::<f32>();
    let left_norm = left.iter().map(|value| value * value).sum::<f32>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f32>().sqrt();
    if left_norm <= f32::EPSILON || right_norm <= f32::EPSILON {
        f32::NEG_INFINITY
    } else {
        dot / (left_norm * right_norm)
    }
}

fn observation_centroids(
    observations: &[SpeakerEmbeddingObservation],
) -> BTreeMap<u32, (crate::EmbeddingSpace, Vec<f32>)> {
    let mut grouped: BTreeMap<u32, Vec<&SpeakerEmbeddingObservation>> = BTreeMap::new();
    for observation in observations {
        if observation.vector.iter().all(|value| value.is_finite()) {
            grouped
                .entry(observation.cluster_id)
                .or_default()
                .push(observation);
        }
    }
    grouped
        .into_iter()
        .filter_map(|(cluster_id, items)| {
            let first = items.first()?;
            let compatible = items
                .iter()
                .all(|item| item.embedding_space.compatible_with(&first.embedding_space));
            if !compatible {
                return None;
            }
            weighted_centroid(items.iter().map(|item| (&item.vector[..], item.quality)))
                .map(|centroid| (cluster_id, (first.embedding_space.clone(), centroid)))
        })
        .collect()
}

fn profile_centroid(profile: &SpeakerProfile) -> Option<Vec<f32>> {
    weighted_centroid(
        profile
            .samples
            .iter()
            .map(|sample| (&sample.vector[..], sample.quality)),
    )
}

fn weighted_centroid<'a>(items: impl Iterator<Item = (&'a [f32], f32)>) -> Option<Vec<f32>> {
    let mut sum: Vec<f32> = Vec::new();
    let mut total_weight = 0.0f32;
    for (vector, quality) in items {
        let normalized = l2_normalized(vector)?;
        if sum.is_empty() {
            sum.resize(normalized.len(), 0.0);
        }
        if sum.len() != normalized.len() {
            return None;
        }
        let weight = quality.max(0.01);
        for (target, value) in sum.iter_mut().zip(normalized) {
            *target += value * weight;
        }
        total_weight += weight;
    }
    if total_weight <= f32::EPSILON {
        return None;
    }
    for value in &mut sum {
        *value /= total_weight;
    }
    l2_normalized(&sum)
}

fn l2_normalized(vector: &[f32]) -> Option<Vec<f32>> {
    if vector.is_empty() || vector.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm <= f32::EPSILON {
        None
    } else {
        Some(vector.iter().map(|value| value / norm).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        EmbeddingNormalization, EmbeddingSpace, SpeakerEmbeddingSample, SpeakerSampleSource,
    };

    fn space() -> EmbeddingSpace {
        EmbeddingSpace {
            backend_id: "diarization-native".into(),
            model_id: "wespeaker".into(),
            model_revision: Some("1".into()),
            dimension: 2,
            sample_rate_hz: 16_000,
            feature_config: "fbank".into(),
            normalization: EmbeddingNormalization::L2,
        }
    }

    fn profile(id: &str, name: &str, vector: Vec<f32>) -> SpeakerProfile {
        SpeakerProfile {
            id: id.into(),
            display_name: name.into(),
            aliases: Vec::new(),
            embedding_space: space(),
            samples: vec![SpeakerEmbeddingSample {
                id: format!("sample-{id}"),
                vector,
                quality: 1.0,
                source: SpeakerSampleSource {
                    project_id: None,
                    source_segment_id: None,
                    start_ms: 0,
                    end_ms: 1_000,
                },
                created_at_ms: 0,
            }],
            match_threshold: 0.7,
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    fn observation(cluster_id: u32, vector: Vec<f32>) -> SpeakerEmbeddingObservation {
        SpeakerEmbeddingObservation {
            id: format!("observation-{cluster_id}"),
            cluster_id,
            start_ms: 0,
            end_ms: 1_000,
            vector,
            quality: 1.0,
            embedding_space: space(),
        }
    }

    #[test]
    fn matches_known_speakers_and_leaves_distant_clusters_anonymous() {
        let resolver = ClusterIdentityResolver::new(IdentityResolverConfig::default());
        let observations = vec![
            observation(0, vec![0.99, 0.01]),
            observation(1, vec![0.0, -1.0]),
        ];

        let matches = resolver
            .resolve(&observations, &[profile("alice", "Alice", vec![1.0, 0.0])])
            .unwrap();

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].cluster_id, 0);
        assert_eq!(matches[0].profile_id, "alice");
    }

    #[test]
    fn refuses_an_ambiguous_identity_match() {
        let resolver = ClusterIdentityResolver::new(IdentityResolverConfig {
            default_threshold: 0.7,
            ambiguity_margin: 0.05,
        });
        let profiles = vec![
            profile("alice", "Alice", vec![1.0, 0.0]),
            profile("alicia", "Alicia", vec![0.999, 0.001]),
        ];

        let matches = resolver
            .resolve(&[observation(0, vec![1.0, 0.0])], &profiles)
            .unwrap();

        assert!(matches.is_empty());
    }
}
