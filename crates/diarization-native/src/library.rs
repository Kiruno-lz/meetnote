use crate::types::{now_ms, SPEAKER_LIBRARY_SCHEMA_VERSION};
use crate::{
    SpeakerEmbeddingObservation, SpeakerEmbeddingSample, SpeakerLibraryDocument,
    SpeakerLibraryError, SpeakerProfile, SpeakerProfileStore, SpeakerSampleSource,
};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

const MAX_SAMPLES_PER_PROFILE: usize = 32;

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
        profile.samples.truncate(MAX_SAMPLES_PER_PROFILE);
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
        let before = document.profiles.len();
        document.profiles.retain(|profile| profile.id != profile_id);
        if document.profiles.len() == before {
            return Ok(false);
        }
        self.store.replace(&document)?;
        Ok(true)
    }
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
            "unsupported schema version {}",
            document.schema_version
        )));
    }
    let mut ids = BTreeSet::new();
    for profile in &document.profiles {
        if !ids.insert(&profile.id) {
            return Err(SpeakerLibraryError::InvalidProfile(format!(
                "duplicate profile id {}",
                profile.id
            )));
        }
        if profile.id.is_empty() || profile.display_name.trim().is_empty() {
            return Err(SpeakerLibraryError::InvalidProfile(
                "profile id and display name must not be empty".into(),
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
            validate_quality(sample.quality)?;
        }
    }
    Ok(())
}

fn validate_observation(
    observation: &SpeakerEmbeddingObservation,
) -> Result<(), SpeakerLibraryError> {
    validate_vector(&observation.vector, observation.embedding_space.dimension)?;
    validate_quality(observation.quality)
}

fn validate_vector(vector: &[f32], dimension: usize) -> Result<(), SpeakerLibraryError> {
    if dimension == 0 || vector.len() != dimension {
        return Err(SpeakerLibraryError::InvalidProfile(format!(
            "embedding dimension is {}, expected {}",
            vector.len(),
            dimension
        )));
    }
    if vector.iter().any(|value| !value.is_finite()) {
        return Err(SpeakerLibraryError::InvalidProfile(
            "embedding contains a non-finite value".into(),
        ));
    }
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm <= f32::EPSILON {
        return Err(SpeakerLibraryError::InvalidProfile(
            "embedding has zero norm".into(),
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

fn validate_quality(quality: f32) -> Result<(), SpeakerLibraryError> {
    if quality.is_finite() && (0.0..=1.0).contains(&quality) {
        Ok(())
    } else {
        Err(SpeakerLibraryError::InvalidProfile(
            "sample quality must be finite and between 0 and 1".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EmbeddingNormalization, EmbeddingSpace, InMemorySpeakerProfileStore};

    fn observation(vector: Vec<f32>) -> SpeakerEmbeddingObservation {
        SpeakerEmbeddingObservation {
            id: "observation".into(),
            cluster_id: 0,
            start_ms: 0,
            end_ms: 1_000,
            quality: 0.9,
            embedding_space: EmbeddingSpace {
                backend_id: "diarization-native".into(),
                model_id: "wespeaker".into(),
                model_revision: Some("1".into()),
                dimension: vector.len(),
                sample_rate_hz: 16_000,
                feature_config: "80-bin-fbank".into(),
                normalization: EmbeddingNormalization::L2,
            },
            vector,
        }
    }

    fn source() -> SpeakerSampleSource {
        SpeakerSampleSource {
            project_id: Some("project".into()),
            source_segment_id: Some("segment".into()),
            start_ms: 0,
            end_ms: 1_000,
        }
    }

    #[test]
    fn enrollment_persists_a_versioned_embedding_profile() {
        let service = SpeakerLibraryService::new(Arc::new(InMemorySpeakerProfileStore::default()));

        let profile = service
            .enroll("Alice", &observation(vec![1.0, 0.0]), source(), 0.72)
            .unwrap();

        let profiles = service.profiles().unwrap();
        assert_eq!(profiles, vec![profile]);
        assert_eq!(
            profiles[0].embedding_space.model_revision.as_deref(),
            Some("1")
        );
    }

    #[test]
    fn append_rejects_a_different_embedding_space() {
        let service = SpeakerLibraryService::new(Arc::new(InMemorySpeakerProfileStore::default()));
        let profile = service
            .enroll("Alice", &observation(vec![1.0, 0.0]), source(), 0.72)
            .unwrap();
        let mut incompatible = observation(vec![1.0, 0.0]);
        incompatible.embedding_space.model_revision = Some("2".into());

        let result = service.append_observation(&profile.id, &incompatible, source());

        assert!(matches!(
            result,
            Err(SpeakerLibraryError::IncompatibleEmbeddingSpace)
        ));
    }

    #[test]
    fn deleting_a_profile_removes_its_sensitive_embeddings() {
        let service = SpeakerLibraryService::new(Arc::new(InMemorySpeakerProfileStore::default()));
        let profile = service
            .enroll("Alice", &observation(vec![1.0, 0.0]), source(), 0.72)
            .unwrap();

        assert!(service.delete(&profile.id).unwrap());
        assert!(service.profiles().unwrap().is_empty());
    }
}
