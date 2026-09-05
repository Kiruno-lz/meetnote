use contracts::{
    EmbeddingSpace, IdentityMatch, SpeakerEmbeddingObservation, SpeakerEmbeddingSample,
    SpeakerLibraryDocument, SpeakerProfile, SpeakerProfileCandidate, SpeakerSampleSource,
    SPEAKER_LIBRARY_SCHEMA_VERSION,
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
        validate_source(&source)?;
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
        validate_source(&source)?;
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
        let config = IdentityResolverConfig::default();
        let centroid = profile_centroid(profile).ok_or_else(|| {
            SpeakerLibraryError::InvalidProfile("speaker profile has no valid centroid".into())
        })?;
        let similarity = cosine_similarity(&observation.vector, &centroid);
        if similarity < profile.match_threshold.max(config.default_threshold) {
            return Err(SpeakerLibraryError::InvalidProfile(
                "speaker observation is not similar enough to extend this profile".into(),
            ));
        }
        let combined = profile
            .samples
            .iter()
            .map(|sample| (&sample.vector[..], sample.quality))
            .chain(std::iter::once((
                &observation.vector[..],
                observation.quality,
            )))
            .collect::<Vec<_>>();
        if vector_dispersion(&combined)
            .is_none_or(|dispersion| dispersion > config.max_combined_dispersion)
        {
            return Err(SpeakerLibraryError::InvalidProfile(
                "speaker observation would make the profile distribution too wide".into(),
            ));
        }
        profile
            .samples
            .push(sample_from_observation(observation, source));
        retain_best_samples(&mut profile.samples);
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

    pub fn merge_profiles(
        &self,
        target_profile_id: &str,
        source_profile_id: &str,
    ) -> Result<SpeakerProfile, SpeakerLibraryError> {
        if target_profile_id == source_profile_id {
            return Err(SpeakerLibraryError::InvalidProfile(
                "cannot merge a speaker profile into itself".into(),
            ));
        }
        let _guard = self
            .transaction_lock
            .lock()
            .map_err(|_| SpeakerLibraryError::LockPoisoned)?;
        let mut document = self.store.load()?;
        validate_document(&document)?;
        let target_index = document
            .profiles
            .iter()
            .position(|profile| profile.id == target_profile_id)
            .ok_or_else(|| SpeakerLibraryError::ProfileNotFound(target_profile_id.into()))?;
        let source_index = document
            .profiles
            .iter()
            .position(|profile| profile.id == source_profile_id)
            .ok_or_else(|| SpeakerLibraryError::ProfileNotFound(source_profile_id.into()))?;
        let source = document.profiles[source_index].clone();
        let target = &document.profiles[target_index];
        if !target
            .embedding_space
            .compatible_with(&source.embedding_space)
        {
            return Err(SpeakerLibraryError::IncompatibleEmbeddingSpace);
        }
        let target_centroid = profile_centroid(target).ok_or_else(|| {
            SpeakerLibraryError::InvalidProfile("target profile has no valid centroid".into())
        })?;
        let source_centroid = profile_centroid(&source).ok_or_else(|| {
            SpeakerLibraryError::InvalidProfile("source profile has no valid centroid".into())
        })?;
        let config = IdentityResolverConfig::default();
        let merge_threshold = target
            .match_threshold
            .max(source.match_threshold)
            .max(config.default_threshold);
        if cosine_similarity(&target_centroid, &source_centroid) < merge_threshold {
            return Err(SpeakerLibraryError::InvalidProfile(
                "speaker profiles are too dissimilar to merge".into(),
            ));
        }
        let combined_vectors = target
            .samples
            .iter()
            .chain(source.samples.iter())
            .map(|sample| (&sample.vector[..], sample.quality))
            .collect::<Vec<_>>();
        if vector_dispersion(&combined_vectors)
            .is_none_or(|value| value > config.max_combined_dispersion)
        {
            return Err(SpeakerLibraryError::InvalidProfile(
                "merged speaker profile would exceed the dispersion limit".into(),
            ));
        }
        if !target
            .samples
            .iter()
            .chain(source.samples.iter())
            .any(|sample| source_is_recoverable(&sample.source))
        {
            return Err(SpeakerLibraryError::InvalidProfile(
                "merged speaker profile must retain recoverable audio evidence".into(),
            ));
        }
        let mut source_aliases = source.aliases.clone();
        source_aliases.push(source.display_name.clone());
        let target = &mut document.profiles[target_index];
        target.aliases.extend(source_aliases);
        target.aliases.sort();
        target.aliases.dedup();
        target.samples.extend(source.samples);
        retain_best_samples(&mut target.samples);
        target.updated_at_ms = now_ms();
        let updated = target.clone();
        document.profiles.remove(source_index);
        self.store.replace(&document)?;
        Ok(updated)
    }

    pub fn split_profile(
        &self,
        profile_id: &str,
        sample_ids: &[String],
        new_display_name: impl Into<String>,
    ) -> Result<(SpeakerProfile, SpeakerProfile), SpeakerLibraryError> {
        let new_display_name = new_display_name.into().trim().to_string();
        if new_display_name.is_empty() {
            return Err(SpeakerLibraryError::InvalidProfile(
                "split profile display name must not be empty".into(),
            ));
        }
        let selected = sample_ids.iter().collect::<BTreeSet<_>>();
        if selected.is_empty() {
            return Err(SpeakerLibraryError::InvalidProfile(
                "split profile requires at least one sample".into(),
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
        let mut moved = Vec::new();
        profile.samples.retain(|sample| {
            if selected.contains(&sample.id) {
                moved.push(sample.clone());
                false
            } else {
                true
            }
        });
        if moved.len() != selected.len() || profile.samples.is_empty() {
            return Err(SpeakerLibraryError::InvalidProfile(
                "split must reference existing samples and leave both profiles non-empty".into(),
            ));
        }
        let maximum_dispersion = IdentityResolverConfig::default().max_combined_dispersion;
        if sample_dispersion(&profile.samples)
            .is_none_or(|dispersion| dispersion > maximum_dispersion)
            || sample_dispersion(&moved).is_none_or(|dispersion| dispersion > maximum_dispersion)
        {
            return Err(SpeakerLibraryError::InvalidProfile(
                "split speaker profiles must each form a compact embedding distribution".into(),
            ));
        }
        if !profile
            .samples
            .iter()
            .any(|sample| source_is_recoverable(&sample.source))
            || !moved
                .iter()
                .any(|sample| source_is_recoverable(&sample.source))
        {
            return Err(SpeakerLibraryError::InvalidProfile(
                "split speaker profiles must each retain recoverable audio evidence".into(),
            ));
        }
        let timestamp = now_ms();
        profile.updated_at_ms = timestamp;
        let retained = profile.clone();
        let split = SpeakerProfile {
            id: uuid::Uuid::new_v4().to_string(),
            display_name: new_display_name,
            aliases: Vec::new(),
            embedding_space: profile.embedding_space.clone(),
            samples: moved,
            match_threshold: profile.match_threshold,
            created_at_ms: timestamp,
            updated_at_ms: timestamp,
        };
        document.profiles.push(split.clone());
        validate_document(&document)?;
        self.store.replace(&document)?;
        Ok((retained, split))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct IdentityResolverConfig {
    pub default_threshold: f32,
    pub ambiguity_margin: f32,
    pub min_distribution_score: f32,
    pub max_cluster_dispersion: f32,
    pub max_combined_dispersion: f32,
    pub min_cross_cluster_similarity: f32,
    pub high_confidence_quality: f32,
    pub max_same_profile_overlap_ms: u64,
    pub candidate_min_observations: usize,
    pub candidate_min_duration_ms: u64,
    pub candidate_min_quality: f32,
    pub candidate_max_dispersion: f32,
}

impl Default for IdentityResolverConfig {
    fn default() -> Self {
        Self {
            default_threshold: 0.72,
            ambiguity_margin: 0.04,
            min_distribution_score: 0.62,
            max_cluster_dispersion: 0.24,
            max_combined_dispersion: 0.28,
            min_cross_cluster_similarity: 0.68,
            high_confidence_quality: 0.75,
            max_same_profile_overlap_ms: 100,
            candidate_min_observations: 3,
            candidate_min_duration_ms: 3_000,
            candidate_min_quality: 0.70,
            candidate_max_dispersion: 0.22,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IdentityResolution {
    pub matches: Vec<IdentityMatch>,
    pub candidates: Vec<SpeakerProfileCandidate>,
}

struct ClusterEvidence<'a> {
    cluster_id: u32,
    space: EmbeddingSpace,
    centroid: Vec<f32>,
    observations: Vec<&'a SpeakerEmbeddingObservation>,
    dispersion: f32,
}

struct ProposedMatch<'a> {
    identity: IdentityMatch,
    cluster: &'a ClusterEvidence<'a>,
    profile: &'a SpeakerProfile,
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
        Ok(self.resolve_detailed(observations, profiles)?.matches)
    }

    pub fn resolve_detailed(
        &self,
        observations: &[SpeakerEmbeddingObservation],
        profiles: &[SpeakerProfile],
    ) -> Result<IdentityResolution, SpeakerLibraryError> {
        let clusters = cluster_evidence(observations)?;
        let mut proposed = Vec::new();
        for cluster in clusters.values() {
            let mut profile_scores = profiles
                .iter()
                .filter(|profile| profile.embedding_space.compatible_with(&cluster.space))
                .filter_map(|profile| {
                    profile_centroid(profile).map(|profile_vector| {
                        let score = cosine_similarity(&cluster.centroid, &profile_vector);
                        let threshold = profile.match_threshold.max(self.config.default_threshold);
                        let distribution_floor = cluster
                            .observations
                            .iter()
                            .map(|observation| {
                                cosine_similarity(&observation.vector, &profile_vector)
                            })
                            .fold(1.0f32, f32::min);
                        let combined =
                            profile
                                .samples
                                .iter()
                                .map(|sample| (&sample.vector[..], sample.quality))
                                .chain(cluster.observations.iter().map(|observation| {
                                    (&observation.vector[..], observation.quality)
                                }))
                                .collect::<Vec<_>>();
                        let combined_dispersion =
                            vector_dispersion(&combined).unwrap_or(f32::INFINITY);
                        (
                            profile,
                            score,
                            threshold,
                            distribution_floor,
                            combined_dispersion,
                        )
                    })
                })
                .collect::<Vec<_>>();
            profile_scores.sort_by(|left, right| right.1.total_cmp(&left.1));
            let Some((profile, score, threshold, distribution_floor, combined_dispersion)) =
                profile_scores.first().copied()
            else {
                continue;
            };
            let competition_margin = profile_scores
                .get(1)
                .map(|candidate| score - candidate.1)
                .unwrap_or(2.0);
            if score < threshold
                || distribution_floor < self.config.min_distribution_score
                || cluster.dispersion > self.config.max_cluster_dispersion
                || combined_dispersion > self.config.max_combined_dispersion
                || competition_margin < self.config.ambiguity_margin
            {
                continue;
            }
            proposed.push(ProposedMatch {
                identity: IdentityMatch {
                    cluster_id: cluster.cluster_id,
                    profile_id: profile.id.clone(),
                    display_name: profile.display_name.clone(),
                    score,
                    threshold,
                    distribution_floor,
                    cluster_dispersion: cluster.dispersion,
                    combined_dispersion,
                    competition_margin,
                },
                cluster,
                profile,
            });
        }
        let matches = self.accept_many_to_one(proposed);
        let matched_clusters = matches
            .iter()
            .map(|identity| identity.cluster_id)
            .collect::<BTreeSet<_>>();
        let candidates = clusters
            .values()
            .filter(|cluster| !matched_clusters.contains(&cluster.cluster_id))
            .filter_map(|cluster| self.profile_candidate(cluster))
            .collect();
        Ok(IdentityResolution {
            matches,
            candidates,
        })
    }

    fn accept_many_to_one(&self, proposed: Vec<ProposedMatch<'_>>) -> Vec<IdentityMatch> {
        let mut by_profile: BTreeMap<String, Vec<ProposedMatch<'_>>> = BTreeMap::new();
        for candidate in proposed {
            by_profile
                .entry(candidate.identity.profile_id.clone())
                .or_default()
                .push(candidate);
        }
        let mut accepted = Vec::new();
        for candidates in by_profile.values_mut() {
            candidates.sort_by(|left, right| right.identity.score.total_cmp(&left.identity.score));
            let mut accepted_clusters: Vec<&ClusterEvidence<'_>> = Vec::new();
            for candidate in candidates {
                if accepted_clusters.iter().any(|accepted| {
                    high_confidence_overlap(
                        accepted,
                        candidate.cluster,
                        self.config.high_confidence_quality,
                        self.config.max_same_profile_overlap_ms,
                    ) || cosine_similarity(&accepted.centroid, &candidate.cluster.centroid)
                        < self.config.min_cross_cluster_similarity
                }) {
                    continue;
                }
                let combined = candidate
                    .profile
                    .samples
                    .iter()
                    .map(|sample| (&sample.vector[..], sample.quality))
                    .chain(
                        accepted_clusters
                            .iter()
                            .flat_map(|cluster| cluster.observations.iter())
                            .chain(candidate.cluster.observations.iter())
                            .map(|observation| (&observation.vector[..], observation.quality)),
                    )
                    .collect::<Vec<_>>();
                let Some(dispersion) = vector_dispersion(&combined) else {
                    continue;
                };
                if dispersion > self.config.max_combined_dispersion {
                    continue;
                }
                let mut identity = candidate.identity.clone();
                identity.combined_dispersion = dispersion;
                accepted_clusters.push(candidate.cluster);
                accepted.push(identity);
            }
        }
        accepted.sort_by_key(|identity| identity.cluster_id);
        accepted
    }

    fn profile_candidate(&self, cluster: &ClusterEvidence<'_>) -> Option<SpeakerProfileCandidate> {
        let observation_count = cluster.observations.len();
        let duration_ms = cluster
            .observations
            .iter()
            .map(|observation| observation.end_ms.saturating_sub(observation.start_ms))
            .sum::<u64>();
        let mean_quality = cluster
            .observations
            .iter()
            .map(|observation| observation.quality)
            .sum::<f32>()
            / observation_count.max(1) as f32;
        if observation_count < self.config.candidate_min_observations
            || duration_ms < self.config.candidate_min_duration_ms
            || mean_quality < self.config.candidate_min_quality
            || cluster.dispersion > self.config.candidate_max_dispersion
        {
            return None;
        }
        let mut representative = cluster.observations.clone();
        representative.sort_by(|left, right| {
            representative_score(right, &cluster.centroid)
                .total_cmp(&representative_score(left, &cluster.centroid))
        });
        Some(SpeakerProfileCandidate {
            cluster_id: cluster.cluster_id,
            representative_observation_ids: representative
                .into_iter()
                .take(3)
                .map(|observation| observation.id.clone())
                .collect(),
            start_ms: cluster
                .observations
                .iter()
                .map(|observation| observation.start_ms)
                .min()
                .unwrap_or(0),
            end_ms: cluster
                .observations
                .iter()
                .map(|observation| observation.end_ms)
                .max()
                .unwrap_or(0),
            observation_count,
            mean_quality,
            dispersion: cluster.dispersion,
            embedding_space: cluster.space.clone(),
        })
    }
}

pub fn apply_identity_matches(
    result: &mut contracts::DiarizationResult,
    library: &SpeakerLibraryService,
    resolver: &ClusterIdentityResolver,
) -> Result<Vec<IdentityMatch>, SpeakerLibraryError> {
    let resolution =
        resolver.resolve_detailed(&result.embedding_observations, &library.profiles()?)?;
    result.speaker_profile_candidates = resolution.candidates;
    let matches = resolution.matches;
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

fn cluster_evidence(
    observations: &[SpeakerEmbeddingObservation],
) -> Result<BTreeMap<u32, ClusterEvidence<'_>>, SpeakerLibraryError> {
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
        .map(|(cluster_id, items)| {
            let first = items[0];
            if !items
                .iter()
                .all(|item| item.embedding_space.compatible_with(&first.embedding_space))
            {
                return Err(SpeakerLibraryError::IncompatibleEmbeddingSpace);
            }
            let vectors = items
                .iter()
                .map(|item| (&item.vector[..], item.quality))
                .collect::<Vec<_>>();
            let centroid = weighted_centroid(vectors.iter().copied()).ok_or_else(|| {
                SpeakerLibraryError::InvalidProfile(
                    "speaker cluster has no valid embedding centroid".into(),
                )
            })?;
            let dispersion = vector_dispersion(&vectors).ok_or_else(|| {
                SpeakerLibraryError::InvalidProfile(
                    "speaker cluster has invalid embedding dispersion".into(),
                )
            })?;
            Ok((
                cluster_id,
                ClusterEvidence {
                    cluster_id,
                    space: first.embedding_space.clone(),
                    centroid,
                    observations: items,
                    dispersion,
                },
            ))
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

fn vector_dispersion(items: &[(&[f32], f32)]) -> Option<f32> {
    let centroid = weighted_centroid(items.iter().copied())?;
    items
        .iter()
        .map(|(vector, _)| 1.0 - cosine_similarity(vector, &centroid))
        .try_fold(0.0f32, |maximum, distance| {
            distance.is_finite().then(|| maximum.max(distance.max(0.0)))
        })
}

fn high_confidence_overlap(
    left: &ClusterEvidence<'_>,
    right: &ClusterEvidence<'_>,
    minimum_quality: f32,
    maximum_overlap_ms: u64,
) -> bool {
    left.observations
        .iter()
        .filter(|observation| observation.quality >= minimum_quality)
        .any(|left| {
            right
                .observations
                .iter()
                .filter(|observation| observation.quality >= minimum_quality)
                .any(|right| {
                    left.end_ms
                        .min(right.end_ms)
                        .saturating_sub(left.start_ms.max(right.start_ms))
                        > maximum_overlap_ms
                })
        })
}

fn retain_best_samples(samples: &mut Vec<SpeakerEmbeddingSample>) {
    samples.sort_by(|left, right| {
        right
            .quality
            .total_cmp(&left.quality)
            .then_with(|| right.created_at_ms.cmp(&left.created_at_ms))
    });
    samples.truncate(32);
}

fn sample_dispersion(samples: &[SpeakerEmbeddingSample]) -> Option<f32> {
    vector_dispersion(
        &samples
            .iter()
            .map(|sample| (&sample.vector[..], sample.quality))
            .collect::<Vec<_>>(),
    )
}

fn representative_score(observation: &SpeakerEmbeddingObservation, centroid: &[f32]) -> f32 {
    cosine_similarity(&observation.vector, centroid) * 0.7 + observation.quality * 0.3
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
    let mut sample_ids = BTreeSet::new();
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
            if sample.id.is_empty() || !sample_ids.insert(&sample.id) {
                return Err(SpeakerLibraryError::InvalidProfile(
                    "speaker sample ids must be unique and non-empty".into(),
                ));
            }
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

fn validate_source(source: &SpeakerSampleSource) -> Result<(), SpeakerLibraryError> {
    if source.end_ms <= source.start_ms {
        return Err(SpeakerLibraryError::InvalidProfile(
            "speaker sample source must have a non-empty time range".into(),
        ));
    }
    if !source_is_recoverable(source) {
        return Err(SpeakerLibraryError::InvalidProfile(
            "speaker sample source must retain a project segment, audio path, or independent backup"
                .into(),
        ));
    }
    if source.audio_sha256.as_deref().is_some_and(|digest| {
        digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    }) {
        return Err(SpeakerLibraryError::InvalidProfile(
            "speaker sample source SHA-256 must contain 64 hexadecimal characters".into(),
        ));
    }
    Ok(())
}

fn source_is_recoverable(source: &SpeakerSampleSource) -> bool {
    let has_direct_audio_reference = source
        .audio_path
        .as_deref()
        .is_some_and(|path| !path.trim().is_empty())
        || source
            .backup_path
            .as_deref()
            .is_some_and(|path| !path.trim().is_empty());
    let has_logical_audio_reference = source
        .project_id
        .as_deref()
        .is_some_and(|id| !id.trim().is_empty())
        && source
            .source_segment_id
            .as_deref()
            .is_some_and(|id| !id.trim().is_empty());
    has_direct_audio_reference || has_logical_audio_reference
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

    fn observation(
        id: &str,
        cluster_id: u32,
        start_ms: u64,
        end_ms: u64,
        vector: [f32; 2],
    ) -> SpeakerEmbeddingObservation {
        SpeakerEmbeddingObservation {
            id: id.into(),
            cluster_id,
            start_ms,
            end_ms,
            vector: vector.to_vec(),
            quality: 0.9,
            embedding_space: embedding_space(),
        }
    }

    fn source(name: &str, start_ms: u64, end_ms: u64) -> SpeakerSampleSource {
        SpeakerSampleSource {
            project_id: Some("identity-tests".into()),
            source_segment_id: Some(name.into()),
            start_ms,
            end_ms,
            audio_path: Some(format!("{name}.wav")),
            audio_sha256: None,
            backup_path: None,
        }
    }

    fn enroll(library: &SpeakerLibraryService, name: &str, vector: [f32; 2]) -> SpeakerProfile {
        library
            .enroll(
                name,
                &observation(name, 0, 0, 1_000, vector),
                source(name, 0, 1_000),
                0.7,
            )
            .unwrap()
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
                    audio_path: Some("fixture.wav".into()),
                    audio_sha256: None,
                    backup_path: None,
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
                artifacts: Vec::new(),
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
            speaker_profile_candidates: Vec::new(),
        };

        let matches = apply_identity_matches(
            &mut result,
            &library,
            &ClusterIdentityResolver::new(IdentityResolverConfig {
                default_threshold: 0.7,
                ambiguity_margin: 0.03,
                ..IdentityResolverConfig::default()
            }),
        )
        .unwrap();

        assert_eq!(matches[0].display_name, "Alice");
        assert_eq!(result.segments[0].speaker_label.as_deref(), Some("Alice"));
    }

    #[test]
    fn accepts_non_overlapping_clusters_for_the_same_profile() {
        let library = SpeakerLibraryService::new(Arc::new(InMemorySpeakerProfileStore::default()));
        let profile = enroll(&library, "Alice", [1.0, 0.0]);
        let observations = vec![
            observation("first", 0, 0, 1_000, [1.0, 0.01]),
            observation("second", 1, 2_000, 3_000, [0.99, -0.01]),
        ];

        let resolution = ClusterIdentityResolver::new(IdentityResolverConfig::default())
            .resolve_detailed(&observations, &library.profiles().unwrap())
            .unwrap();

        assert_eq!(resolution.matches.len(), 2);
        assert!(resolution
            .matches
            .iter()
            .all(|identity| identity.profile_id == profile.id));
        assert!(resolution.candidates.is_empty());
    }

    #[test]
    fn rejects_a_second_high_confidence_cluster_that_overlaps_the_same_profile() {
        let library = SpeakerLibraryService::new(Arc::new(InMemorySpeakerProfileStore::default()));
        enroll(&library, "Alice", [1.0, 0.0]);
        let observations = vec![
            observation("first", 0, 0, 1_000, [1.0, 0.01]),
            observation("second", 1, 500, 1_500, [0.99, -0.01]),
        ];

        let resolution = ClusterIdentityResolver::new(IdentityResolverConfig::default())
            .resolve_detailed(&observations, &library.profiles().unwrap())
            .unwrap();

        assert_eq!(resolution.matches.len(), 1);
        assert_eq!(resolution.matches[0].cluster_id, 0);
    }

    #[test]
    fn rejects_a_cluster_with_a_good_centroid_but_wide_distribution() {
        let library = SpeakerLibraryService::new(Arc::new(InMemorySpeakerProfileStore::default()));
        enroll(&library, "Alice", [1.0, 0.0]);
        let observations = vec![
            observation("positive", 0, 0, 1_000, [0.7, 0.714]),
            observation("negative", 0, 1_000, 2_000, [0.7, -0.714]),
        ];

        let matches = ClusterIdentityResolver::new(IdentityResolverConfig::default())
            .resolve(&observations, &library.profiles().unwrap())
            .unwrap();

        assert!(matches.is_empty());
    }

    #[test]
    fn rejects_an_ambiguous_profile_match_without_competition_margin() {
        let library = SpeakerLibraryService::new(Arc::new(InMemorySpeakerProfileStore::default()));
        enroll(&library, "Alice", [1.0, 0.01]);
        enroll(&library, "Bob", [1.0, -0.01]);

        let matches = ClusterIdentityResolver::new(IdentityResolverConfig::default())
            .resolve(
                &[observation("ambiguous", 0, 0, 1_000, [1.0, 0.0])],
                &library.profiles().unwrap(),
            )
            .unwrap();

        assert!(matches.is_empty());
    }

    #[test]
    fn creates_a_candidate_only_from_sufficient_compact_unknown_evidence() {
        let resolver = ClusterIdentityResolver::new(IdentityResolverConfig::default());
        let observations = vec![
            observation("one", 4, 0, 1_000, [0.0, 1.0]),
            observation("two", 4, 1_000, 2_000, [0.01, 1.0]),
            observation("three", 4, 2_000, 3_000, [-0.01, 1.0]),
        ];

        let resolution = resolver.resolve_detailed(&observations, &[]).unwrap();

        assert!(resolution.matches.is_empty());
        assert_eq!(resolution.candidates.len(), 1);
        assert_eq!(resolution.candidates[0].cluster_id, 4);
        assert_eq!(
            resolution.candidates[0]
                .representative_observation_ids
                .len(),
            3
        );
    }

    #[test]
    fn merges_and_splits_profiles_while_retaining_audio_evidence() {
        let library = SpeakerLibraryService::new(Arc::new(InMemorySpeakerProfileStore::default()));
        let target = enroll(&library, "Alice", [1.0, 0.0]);
        let source_profile = enroll(&library, "Alice interview", [0.99, 0.01]);

        let merged = library
            .merge_profiles(&target.id, &source_profile.id)
            .unwrap();
        assert_eq!(library.profiles().unwrap().len(), 1);
        assert_eq!(merged.samples.len(), 2);
        assert!(merged
            .samples
            .iter()
            .all(|sample| sample.source.audio_path.is_some()));

        let moved_sample = merged.samples[1].id.clone();
        let (retained, split) = library
            .split_profile(&merged.id, &[moved_sample], "Alice alternate")
            .unwrap();
        assert_eq!(retained.samples.len(), 1);
        assert_eq!(split.samples.len(), 1);
        assert!(split.samples[0].source.audio_path.is_some());
        assert_eq!(library.profiles().unwrap().len(), 2);
    }

    #[test]
    fn refuses_enrollment_without_recoverable_audio_evidence() {
        let library = SpeakerLibraryService::new(Arc::new(InMemorySpeakerProfileStore::default()));
        let error = library
            .enroll(
                "Alice",
                &observation("missing", 0, 0, 1_000, [1.0, 0.0]),
                SpeakerSampleSource {
                    project_id: None,
                    source_segment_id: None,
                    start_ms: 0,
                    end_ms: 1_000,
                    audio_path: None,
                    audio_sha256: None,
                    backup_path: None,
                },
                0.7,
            )
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("project segment, audio path, or independent backup"));
    }

    #[test]
    fn refuses_to_pollute_a_profile_with_a_dissimilar_sample() {
        let library = SpeakerLibraryService::new(Arc::new(InMemorySpeakerProfileStore::default()));
        let profile = enroll(&library, "Alice", [1.0, 0.0]);

        let error = library
            .append_observation(
                &profile.id,
                &observation("bob", 0, 2_000, 3_000, [0.0, 1.0]),
                source("bob", 2_000, 3_000),
            )
            .unwrap_err();

        assert!(error.to_string().contains("not similar enough"));
        assert_eq!(library.profiles().unwrap()[0].samples.len(), 1);
    }

    #[test]
    fn legacy_profiles_without_new_audio_fields_remain_matchable() {
        let profile = SpeakerProfile {
            id: "legacy-profile".into(),
            display_name: "Legacy".into(),
            aliases: Vec::new(),
            embedding_space: embedding_space(),
            samples: vec![SpeakerEmbeddingSample {
                id: "legacy-sample".into(),
                vector: vec![1.0, 0.0],
                quality: 0.9,
                source: SpeakerSampleSource {
                    project_id: None,
                    source_segment_id: None,
                    start_ms: 0,
                    end_ms: 1_000,
                    audio_path: None,
                    audio_sha256: None,
                    backup_path: None,
                },
                created_at_ms: 1,
            }],
            match_threshold: 0.7,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        let store = Arc::new(InMemorySpeakerProfileStore {
            document: Mutex::new(SpeakerLibraryDocument {
                schema_version: SPEAKER_LIBRARY_SCHEMA_VERSION,
                profiles: vec![profile],
            }),
        });
        let library = SpeakerLibraryService::new(store);

        let profiles = library.profiles().unwrap();
        let matches = ClusterIdentityResolver::new(IdentityResolverConfig::default())
            .resolve(
                &[observation("legacy-observation", 0, 0, 1_000, [1.0, 0.0])],
                &profiles,
            )
            .unwrap();

        assert_eq!(matches[0].profile_id, "legacy-profile");
    }
}
