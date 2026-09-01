use contracts::{
    EmbeddingSpace, IdentityMatch, SpeakerEmbeddingObservation, SpeakerEmbeddingSample,
    SpeakerLibraryDocument, SpeakerProfile, SpeakerSampleSource, SPEAKER_LIBRARY_SCHEMA_VERSION,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, thiserror::Error)]
pub enum SpeakerLibraryError {
    #[error("speaker library I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("speaker library JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("speaker profile is invalid: {0}")]
    InvalidProfile(String),
    #[error("speaker profile was not found: {0}")]
    ProfileNotFound(String),
    #[error("speaker embedding space is incompatible")]
    IncompatibleEmbeddingSpace,
    #[error("speaker library lock is poisoned")]
    LockPoisoned,
}

pub trait SpeakerProfileStore: Send + Sync {
    fn load(&self) -> Result<SpeakerLibraryDocument, SpeakerLibraryError>;
    fn replace(&self, document: &SpeakerLibraryDocument) -> Result<(), SpeakerLibraryError>;
}

pub struct JsonSpeakerProfileStore {
    path: PathBuf,
    io_lock: Mutex<()>,
}

impl JsonSpeakerProfileStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            io_lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl SpeakerProfileStore for JsonSpeakerProfileStore {
    fn load(&self) -> Result<SpeakerLibraryDocument, SpeakerLibraryError> {
        let _guard = self
            .io_lock
            .lock()
            .map_err(|_| SpeakerLibraryError::LockPoisoned)?;
        match fs::read(&self.path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(SpeakerLibraryDocument::default())
            }
            Err(error) => Err(error.into()),
        }
    }

    fn replace(&self, document: &SpeakerLibraryDocument) -> Result<(), SpeakerLibraryError> {
        let _guard = self
            .io_lock
            .lock()
            .map_err(|_| SpeakerLibraryError::LockPoisoned)?;
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let temporary = parent.join(format!(
            ".{}.{}.tmp",
            self.path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("speaker-library"),
            uuid::Uuid::new_v4()
        ));
        let bytes = serde_json::to_vec_pretty(document)?;
        let result = (|| -> Result<(), SpeakerLibraryError> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &self.path)?;
            if let Ok(directory) = fs::File::open(parent) {
                let _ = directory.sync_all();
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

#[derive(Default)]
pub struct InMemorySpeakerProfileStore {
    document: Mutex<SpeakerLibraryDocument>,
}

impl SpeakerProfileStore for InMemorySpeakerProfileStore {
    fn load(&self) -> Result<SpeakerLibraryDocument, SpeakerLibraryError> {
        self.document
            .lock()
            .map(|document| document.clone())
            .map_err(|_| SpeakerLibraryError::LockPoisoned)
    }

    fn replace(&self, document: &SpeakerLibraryDocument) -> Result<(), SpeakerLibraryError> {
        *self
            .document
            .lock()
            .map_err(|_| SpeakerLibraryError::LockPoisoned)? = document.clone();
        Ok(())
    }
}

#[derive(Clone)]
pub struct SpeakerLibraryService {
    store: Arc<dyn SpeakerProfileStore>,
    transaction_lock: Arc<Mutex<()>>,
}

impl SpeakerLibraryService {
    pub fn new(store: Arc<dyn SpeakerProfileStore>) -> Self {
        Self {
            store,
            transaction_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn profiles(&self) -> Result<Vec<SpeakerProfile>, SpeakerLibraryError> {
        let document = self.store.load()?;
        validate_document(&document)?;
        Ok(document.profiles)
    }

    pub fn enroll(
        &self,
        display_name: impl Into<String>,
        observation: &SpeakerEmbeddingObservation,
        source: SpeakerSampleSource,
        match_threshold: f32,
    ) -> Result<SpeakerProfile, SpeakerLibraryError> {
        validate_threshold(match_threshold)?;
        validate_observation(observation)?;
        let display_name = display_name.into().trim().to_string();
        if display_name.is_empty() {
            return Err(SpeakerLibraryError::InvalidProfile(
                "display name must not be empty".into(),
            ));
        }
        let _guard = self
            .transaction_lock
            .lock()
            .map_err(|_| SpeakerLibraryError::LockPoisoned)?;
        let mut document = self.store.load()?;
        validate_document(&document)?;
        let timestamp = now_ms();
        let profile = SpeakerProfile {
            id: uuid::Uuid::new_v4().to_string(),
            display_name,
            aliases: Vec::new(),
            embedding_space: observation.embedding_space.clone(),
            samples: vec![sample_from_observation(observation, source)],
            match_threshold,
            created_at_ms: timestamp,
            updated_at_ms: timestamp,
        };
        document.profiles.push(profile.clone());
        self.store.replace(&document)?;
        Ok(profile)
    }

    pub fn append_observation(
        &self,
        profile_id: &str,
        observation: &SpeakerEmbeddingObservation,
        source: SpeakerSampleSource,
    ) -> Result<SpeakerProfile, SpeakerLibraryError> {
        validate_observation(observation)?;
        let _guard = self
            .transaction_lock
            .lock()
            .map_err(|_| SpeakerLibraryError::LockPoisoned)?;
        let mut document = self.store.load()?;
        validate_document(&document)?;
        let profile = document
            .profiles
            .iter_mut()
            .find(|profile| profile.id == profile_id)
            .ok_or_else(|| SpeakerLibraryError::ProfileNotFound(profile_id.into()))?;
        if !profile
            .embedding_space
            .compatible_with(&observation.embedding_space)
        {
            return Err(SpeakerLibraryError::IncompatibleEmbeddingSpace);
        }
        profile
            .samples
            .push(sample_from_observation(observation, source));
        profile.samples.sort_by(|left, right| {
            right
                .quality
                .total_cmp(&left.quality)
                .then_with(|| right.created_at_ms.cmp(&left.created_at_ms))
        });
        profile.samples.truncate(32);
        profile.updated_at_ms = now_ms();
        let updated = profile.clone();
        self.store.replace(&document)?;
        Ok(updated)
    }

    pub fn rename(
        &self,
        profile_id: &str,
        display_name: impl Into<String>,
    ) -> Result<SpeakerProfile, SpeakerLibraryError> {
        let display_name = display_name.into().trim().to_string();
        if display_name.is_empty() {
            return Err(SpeakerLibraryError::InvalidProfile(
                "display name must not be empty".into(),
            ));
        }
        let _guard = self
            .transaction_lock
            .lock()
            .map_err(|_| SpeakerLibraryError::LockPoisoned)?;
        let mut document = self.store.load()?;
        validate_document(&document)?;
        let profile = document
            .profiles
            .iter_mut()
            .find(|profile| profile.id == profile_id)
            .ok_or_else(|| SpeakerLibraryError::ProfileNotFound(profile_id.into()))?;
        profile.display_name = display_name;
        profile.updated_at_ms = now_ms();
        let updated = profile.clone();
        self.store.replace(&document)?;
        Ok(updated)
    }

    pub fn delete(&self, profile_id: &str) -> Result<bool, SpeakerLibraryError> {
        let _guard = self
            .transaction_lock
            .lock()
            .map_err(|_| SpeakerLibraryError::LockPoisoned)?;
        let mut document = self.store.load()?;
        validate_document(&document)?;
        let before = document.profiles.len();
        document.profiles.retain(|profile| profile.id != profile_id);
        if before == document.profiles.len() {
            return Ok(false);
        }
        self.store.replace(&document)?;
        Ok(true)
    }
}

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

pub fn apply_identity_matches(
    result: &mut contracts::DiarizationResult,
    library: &SpeakerLibraryService,
    resolver: &ClusterIdentityResolver,
) -> Result<Vec<IdentityMatch>, SpeakerLibraryError> {
    let matches = resolver.resolve(&result.embedding_observations, &library.profiles()?)?;
    let by_cluster = matches
        .iter()
        .map(|identity| (identity.cluster_id, identity))
        .collect::<BTreeMap<_, _>>();
    for segment in &mut result.segments {
        let Ok(cluster_id) = segment
            .speaker_id
            .strip_prefix("speaker-")
            .unwrap_or_default()
            .parse::<u32>()
        else {
            continue;
        };
        if let Some(identity) = by_cluster.get(&cluster_id) {
            segment.speaker_profile_id = Some(identity.profile_id.clone());
            segment.speaker_label = Some(identity.display_name.clone());
        }
    }
    Ok(matches)
}

fn observation_centroids(
    observations: &[SpeakerEmbeddingObservation],
) -> BTreeMap<u32, (EmbeddingSpace, Vec<f32>)> {
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
            if !items
                .iter()
                .all(|item| item.embedding_space.compatible_with(&first.embedding_space))
            {
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

pub fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return f32::NEG_INFINITY;
    }
    let dot = left.iter().zip(right).map(|(l, r)| l * r).sum::<f32>();
    let left_norm = left.iter().map(|value| value * value).sum::<f32>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f32>().sqrt();
    if left_norm <= f32::EPSILON || right_norm <= f32::EPSILON {
        f32::NEG_INFINITY
    } else {
        dot / (left_norm * right_norm)
    }
}

fn weighted_centroid<'a>(items: impl Iterator<Item = (&'a [f32], f32)>) -> Option<Vec<f32>> {
    let mut sum = Vec::new();
    let mut total_weight = 0.0;
    for (vector, quality) in items {
        let normalized = normalize(vector)?;
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
    normalize(&sum)
}

fn normalize(vector: &[f32]) -> Option<Vec<f32>> {
    if vector.is_empty() || vector.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    (norm > f32::EPSILON).then(|| vector.iter().map(|value| value / norm).collect())
}

fn sample_from_observation(
    observation: &SpeakerEmbeddingObservation,
    source: SpeakerSampleSource,
) -> SpeakerEmbeddingSample {
    SpeakerEmbeddingSample {
        id: uuid::Uuid::new_v4().to_string(),
        vector: observation.vector.clone(),
        quality: observation.quality,
        source,
        created_at_ms: now_ms(),
    }
}

fn validate_document(document: &SpeakerLibraryDocument) -> Result<(), SpeakerLibraryError> {
    if document.schema_version != SPEAKER_LIBRARY_SCHEMA_VERSION {
        return Err(SpeakerLibraryError::InvalidProfile(format!(
            "unsupported speaker library schema {}",
            document.schema_version
        )));
    }
    let mut ids = BTreeSet::new();
    for profile in &document.profiles {
        if !ids.insert(&profile.id)
            || profile.id.is_empty()
            || profile.display_name.trim().is_empty()
        {
            return Err(SpeakerLibraryError::InvalidProfile(
                "profile id and display name must be unique and non-empty".into(),
            ));
        }
        validate_threshold(profile.match_threshold)?;
        if profile.samples.is_empty() {
            return Err(SpeakerLibraryError::InvalidProfile(format!(
                "profile {} has no embedding samples",
                profile.id
            )));
        }
        for sample in &profile.samples {
            validate_vector(&sample.vector, profile.embedding_space.dimension)?;
            if !(0.0..=1.0).contains(&sample.quality) || !sample.quality.is_finite() {
                return Err(SpeakerLibraryError::InvalidProfile(
                    "sample quality must be finite and between 0 and 1".into(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_observation(
    observation: &SpeakerEmbeddingObservation,
) -> Result<(), SpeakerLibraryError> {
    validate_vector(&observation.vector, observation.embedding_space.dimension)?;
    if observation.quality.is_finite() && (0.0..=1.0).contains(&observation.quality) {
        Ok(())
    } else {
        Err(SpeakerLibraryError::InvalidProfile(
            "observation quality must be finite and between 0 and 1".into(),
        ))
    }
}

fn validate_vector(vector: &[f32], dimension: usize) -> Result<(), SpeakerLibraryError> {
    if dimension == 0 || vector.len() != dimension || normalize(vector).is_none() {
        return Err(SpeakerLibraryError::InvalidProfile(
            "embedding vector has an invalid dimension or norm".into(),
        ));
    }
    Ok(())
}

fn validate_threshold(threshold: f32) -> Result<(), SpeakerLibraryError> {
    if threshold.is_finite() && (-1.0..=1.0).contains(&threshold) {
        Ok(())
    } else {
        Err(SpeakerLibraryError::InvalidProfile(
            "match threshold must be finite and between -1 and 1".into(),
        ))
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::{DiarizationResult, ModelRef};

    fn embedding_space() -> EmbeddingSpace {
        EmbeddingSpace {
            backend_id: "shared-test".into(),
            model_id: "embedding".into(),
            model_revision: Some("1".into()),
            dimension: 2,
            sample_rate_hz: 16_000,
            frontend: contracts::EmbeddingFrontend {
                extractor: contracts::EmbeddingFeatureExtractor::KaldiFbank,
                mel_bins: 80,
                frame_length_ms: 25,
                frame_shift_ms: 10,
                mean_normalization: false,
                dither: false,
                overlap_aware: false,
            },
            legacy_feature_config: None,
            normalization: contracts::EmbeddingNormalization::L2,
        }
    }

    #[test]
    fn shared_identity_service_labels_results_from_any_backend() {
        let store = Arc::new(InMemorySpeakerProfileStore::default());
        let library = SpeakerLibraryService::new(store);
        let observation = SpeakerEmbeddingObservation {
            id: "observation".into(),
            cluster_id: 0,
            start_ms: 0,
            end_ms: 1_000,
            vector: vec![1.0, 0.0],
            quality: 1.0,
            embedding_space: embedding_space(),
        };
        library
            .enroll(
                "Alice",
                &observation,
                SpeakerSampleSource {
                    project_id: None,
                    source_segment_id: None,
                    start_ms: 0,
                    end_ms: 1_000,
                },
                0.7,
            )
            .unwrap();
        let mut result = DiarizationResult {
            backend: crate::diarization_descriptor("test", &[]),
            model: ModelRef {
                backend_id: "test".into(),
                model_id: "model".into(),
                revision: None,
                artifact_path: "fixture".into(),
            },
            segments: vec![domain::DiarizationSegment {
                id: "segment".into(),
                speaker_id: "speaker-0".into(),
                speaker_profile_id: None,
                speaker_label: None,
                start_ms: 0,
                end_ms: 1_000,
                confidence: Some(1.0),
            }],
            speaker_count: Some(1),
            embedding_observations: vec![observation],
        };

        let matches = apply_identity_matches(
            &mut result,
            &library,
            &ClusterIdentityResolver::new(IdentityResolverConfig {
                default_threshold: 0.7,
                ambiguity_margin: 0.03,
            }),
        )
        .unwrap();

        assert_eq!(matches[0].display_name, "Alice");
        assert_eq!(result.segments[0].speaker_label.as_deref(), Some("Alice"));
    }
}
