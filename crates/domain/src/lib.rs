use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

pub const DOCUMENT_STATE_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct ProjectId(String);

impl ProjectId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn new_uuid() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub type DocumentId = ProjectId;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DocumentState {
    pub schema: DocumentSchema,
    pub identity: DocumentIdentity,
    pub source: AudioSource,
    pub hierarchy: DocumentHierarchy,
    pub document_status: DocumentStatus,
    pub content: DocumentContent,
    pub processing: ProcessingMetadata,
    pub results: ProcessingResults,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentSchema {
    pub schema_version: u32,
    pub revision: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentIdentity {
    pub id: ProjectId,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioSource {
    pub audio_path: String,
    pub recorded_at_ms: Option<u64>,
    pub audio_start_ms: u64,
    pub audio_end_ms: Option<u64>,
    pub audio_asset_id: Option<ProjectId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentHierarchy {
    pub parent_id: Option<ProjectId>,
    pub depth: u8,
    pub sibling_order: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentStatus {
    pub value: String,
}

impl Default for DocumentStatus {
    fn default() -> Self {
        Self {
            value: "inbox".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentContent {
    pub title: String,
    pub tags: Vec<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ProcessingMetadata {
    pub latest: Option<ProcessingRecord>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessingRecord {
    pub processing_revision: u64,
    pub completed_at_ms: u64,
    pub input_audio_checksum: Option<String>,
    pub asr: ModelProvenance,
    pub diarization: DiarizationProvenance,
    pub alignment: AlgorithmProvenance,
    pub runtime: RuntimeProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelProvenance {
    pub backend_id: String,
    pub model_id: String,
    pub revision: Option<String>,
    pub artifact_sha256: Option<String>,
    pub artifact_path: String,
    #[serde(default)]
    pub artifacts: Vec<ModelArtifactProvenance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelArtifactProvenance {
    pub filename: String,
    pub expected_sha256: Option<String>,
    pub observed_sha256: Option<String>,
    pub archive_member: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DiarizationProvenance {
    pub backend_id: String,
    pub model_revision: Option<String>,
    pub segmentation: Option<ModelProvenance>,
    pub embedding: Option<ModelProvenance>,
    pub clustering: AlgorithmProvenance,
    pub identity_matching: Option<IdentityMatchingProvenance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityMatchingProvenance {
    pub embedding_space: Option<String>,
    pub profile_library_schema_version: Option<u32>,
    pub threshold_milli: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AlgorithmProvenance {
    pub id: String,
    pub revision: Option<String>,
    pub parameters: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RuntimeProvenance {
    pub device: String,
    pub language: Option<String>,
    pub sample_rate_hz: Option<u32>,
    pub threads: Option<u32>,
    pub expected_speakers: Option<u32>,
    #[serde(default)]
    pub windowing: Option<AudioWindowingProvenance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioWindowingProvenance {
    pub sample_rate_hz: u32,
    pub core_samples: u64,
    pub context_samples: u64,
    pub window_count: u64,
    pub source_samples: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ProcessingResults {
    pub transcript: Vec<TranscriptSegment>,
    pub diarization: Vec<DiarizationSegment>,
    pub speaker_segments: Vec<SpeakerTaggedSegment>,
    pub markdown_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectStatus {
    Imported,
    Processing,
    Ready,
    Failed,
}

impl<'de> Deserialize<'de> for DocumentState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.get("schema").is_some() {
            serde_json::from_value::<CurrentDocumentState>(value)
                .map(CurrentDocumentState::into_document_state)
                .map_err(serde::de::Error::custom)
        } else {
            let legacy: LegacyDocumentState =
                serde_json::from_value(value).map_err(serde::de::Error::custom)?;
            if legacy.schema_version != 1 {
                return Err(serde::de::Error::custom(format!(
                    "unsupported legacy document schema: {}",
                    legacy.schema_version
                )));
            }
            Ok(legacy.into_document_state())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct CurrentDocumentState {
    schema: DocumentSchema,
    identity: DocumentIdentity,
    source: AudioSource,
    hierarchy: DocumentHierarchy,
    document_status: DocumentStatus,
    content: DocumentContent,
    processing: ProcessingMetadata,
    results: ProcessingResults,
}

impl CurrentDocumentState {
    fn into_document_state(self) -> DocumentState {
        DocumentState {
            schema: self.schema,
            identity: self.identity,
            source: self.source,
            hierarchy: self.hierarchy,
            document_status: self.document_status,
            content: self.content,
            processing: self.processing,
            results: self.results,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct LegacyDocumentState {
    #[serde(default = "default_legacy_schema_version")]
    schema_version: u32,
    #[serde(default)]
    revision: u64,
    #[serde(default)]
    updated_at_ms: u64,
    id: ProjectId,
    title: String,
    audio_path: String,
    #[serde(default)]
    created_at_ms: u64,
    #[serde(default)]
    recorded_at_ms: Option<u64>,
    #[serde(default)]
    audio_start_ms: u64,
    #[serde(default)]
    audio_end_ms: Option<u64>,
    #[serde(default)]
    parent_id: Option<ProjectId>,
    #[serde(default)]
    audio_asset_id: Option<ProjectId>,
    #[serde(default)]
    depth: u8,
    #[serde(default)]
    sibling_order: u32,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    processing_error: Option<String>,
    #[serde(default = "default_legacy_status")]
    status: ProjectStatus,
    #[serde(default)]
    transcript: Vec<TranscriptSegment>,
    #[serde(default)]
    diarization: Vec<DiarizationSegment>,
    #[serde(default)]
    speaker_segments: Vec<SpeakerTaggedSegment>,
    #[serde(default)]
    markdown_path: Option<String>,
}

impl LegacyDocumentState {
    fn into_document_state(self) -> DocumentState {
        let latest = match self.status {
            ProjectStatus::Ready => Some(ProcessingRecord::placeholder(self.revision)),
            _ => None,
        };
        let last_error = self.processing_error.or_else(|| match self.status {
            ProjectStatus::Processing => Some("processing interrupted before completion".into()),
            ProjectStatus::Failed => Some("processing failed".into()),
            _ => None,
        });
        DocumentState {
            schema: DocumentSchema {
                schema_version: DOCUMENT_STATE_SCHEMA_VERSION,
                revision: self.revision,
                updated_at_ms: self.updated_at_ms,
            },
            identity: DocumentIdentity {
                id: self.id.clone(),
                created_at_ms: self.created_at_ms,
            },
            source: AudioSource {
                audio_path: self.audio_path,
                recorded_at_ms: self.recorded_at_ms,
                audio_start_ms: self.audio_start_ms,
                audio_end_ms: self.audio_end_ms,
                audio_asset_id: self.audio_asset_id,
            },
            hierarchy: DocumentHierarchy {
                parent_id: self.parent_id,
                depth: self.depth,
                sibling_order: self.sibling_order,
            },
            document_status: DocumentStatus::default(),
            content: DocumentContent {
                title: self.title,
                tags: self.tags,
                summary: self.summary,
            },
            processing: ProcessingMetadata { latest, last_error },
            results: ProcessingResults {
                transcript: self.transcript,
                diarization: self.diarization,
                speaker_segments: self.speaker_segments,
                markdown_path: self.markdown_path,
            },
        }
    }
}

impl ProcessingRecord {
    fn placeholder(processing_revision: u64) -> Self {
        Self {
            processing_revision,
            completed_at_ms: 0,
            input_audio_checksum: None,
            asr: ModelProvenance::default(),
            diarization: DiarizationProvenance::default(),
            alignment: AlgorithmProvenance::default(),
            runtime: RuntimeProvenance::default(),
        }
    }
}

fn default_legacy_schema_version() -> u32 {
    1
}

fn default_legacy_status() -> ProjectStatus {
    ProjectStatus::Imported
}

impl DocumentState {
    pub fn new(id: ProjectId, title: impl Into<String>, audio_path: impl Into<String>) -> Self {
        let timestamp = now_ms();
        Self {
            schema: DocumentSchema {
                schema_version: DOCUMENT_STATE_SCHEMA_VERSION,
                revision: 0,
                updated_at_ms: timestamp,
            },
            identity: DocumentIdentity {
                id,
                created_at_ms: timestamp,
            },
            source: AudioSource {
                audio_path: audio_path.into(),
                recorded_at_ms: None,
                audio_start_ms: 0,
                audio_end_ms: None,
                audio_asset_id: None,
            },
            hierarchy: DocumentHierarchy {
                parent_id: None,
                depth: 0,
                sibling_order: 0,
            },
            document_status: DocumentStatus::default(),
            content: DocumentContent {
                title: title.into(),
                tags: Vec::new(),
                summary: None,
            },
            processing: ProcessingMetadata::default(),
            results: ProcessingResults::default(),
        }
    }

    pub fn validate_schema(&self) -> Result<(), DocumentStateError> {
        if self.schema.schema_version != DOCUMENT_STATE_SCHEMA_VERSION {
            return Err(DocumentStateError::UnsupportedSchema {
                actual: self.schema.schema_version,
                expected: DOCUMENT_STATE_SCHEMA_VERSION,
            });
        }
        Ok(())
    }

    pub fn id(&self) -> &ProjectId {
        &self.identity.id
    }

    pub fn title(&self) -> &str {
        &self.content.title
    }

    pub fn audio_path(&self) -> &str {
        &self.source.audio_path
    }

    pub fn revision(&self) -> u64 {
        self.schema.revision
    }

    pub fn updated_at_ms(&self) -> u64 {
        self.schema.updated_at_ms
    }

    pub fn processing_status(&self) -> ProjectStatus {
        if self.processing.last_error.is_some() {
            ProjectStatus::Failed
        } else if self.processing.latest.is_some() {
            ProjectStatus::Ready
        } else {
            ProjectStatus::Imported
        }
    }

    pub fn set_processing_error(&mut self, error: impl Into<String>) {
        self.processing.last_error = Some(error.into());
    }

    pub fn clear_processing_error(&mut self) {
        self.processing.last_error = None;
    }

    pub fn record_processing(&mut self, record: ProcessingRecord) {
        self.processing.latest = Some(record);
        self.processing.last_error = None;
    }

    pub fn is_root(&self) -> bool {
        self.hierarchy.parent_id.is_none()
    }

    pub fn semantic_child(
        &self,
        id: ProjectId,
        title: impl Into<String>,
        start_ms: u64,
        end_ms: u64,
        tags: Vec<String>,
    ) -> Self {
        let root_id = self
            .source
            .audio_asset_id
            .clone()
            .unwrap_or_else(|| self.id().clone());
        let mut child = Self::new(id, title, self.audio_path().to_string());
        child.source.recorded_at_ms = self.source.recorded_at_ms;
        child.source.audio_start_ms = start_ms;
        child.source.audio_end_ms = Some(end_ms);
        child.hierarchy.parent_id = Some(self.id().clone());
        child.source.audio_asset_id = Some(root_id);
        child.hierarchy.depth = self.hierarchy.depth.saturating_add(1);
        child.content.tags = tags;
        child.document_status = self.document_status.clone();
        child.results.transcript = self
            .results
            .transcript
            .iter()
            .filter(|segment| overlaps(segment.start_ms, segment.end_ms, start_ms, end_ms))
            .cloned()
            .collect();
        child.results.diarization = self
            .results
            .diarization
            .iter()
            .filter(|segment| overlaps(segment.start_ms, segment.end_ms, start_ms, end_ms))
            .cloned()
            .collect();
        child.results.speaker_segments = self
            .results
            .speaker_segments
            .iter()
            .filter(|segment| overlaps(segment.start_ms, segment.end_ms, start_ms, end_ms))
            .cloned()
            .collect();
        child
    }

    pub fn merge_speaker(&mut self, from: &str, into: &str) -> usize {
        let mut changed = 0;
        for segment in &mut self.results.diarization {
            if segment.speaker_id == from {
                segment.speaker_id = into.to_string();
                changed += 1;
            }
        }
        for segment in &mut self.results.speaker_segments {
            if segment.speaker_id == from {
                segment.speaker_id = into.to_string();
            }
        }
        changed
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentStateError {
    UnsupportedSchema { actual: u32, expected: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectTreeError {
    ProjectNotFound,
    ParentNotFound,
    Cycle,
    MaximumDepth,
    CrossAudioAsset,
}

pub fn reparent_project(
    projects: &mut [DocumentState],
    project_id: &ProjectId,
    new_parent_id: Option<&ProjectId>,
    sibling_order: u32,
) -> Result<(), ProjectTreeError> {
    let index = projects
        .iter()
        .position(|project| project.id() == project_id)
        .ok_or(ProjectTreeError::ProjectNotFound)?;
    let root_id = projects[index]
        .source
        .audio_asset_id
        .as_ref()
        .unwrap_or(projects[index].id())
        .clone();
    let parent_depth = match new_parent_id {
        Some(parent_id) => {
            let parent = projects
                .iter()
                .find(|project| project.id() == parent_id)
                .ok_or(ProjectTreeError::ParentNotFound)?;
            if parent.id() == project_id {
                return Err(ProjectTreeError::Cycle);
            }
            if parent.source.audio_asset_id.as_ref().unwrap_or(parent.id()) != &root_id {
                return Err(ProjectTreeError::CrossAudioAsset);
            }
            if is_descendant(projects, parent_id, project_id) {
                return Err(ProjectTreeError::Cycle);
            }
            parent.hierarchy.depth + 1
        }
        None => 0,
    };
    let subtree_ids = collect_subtree_ids(projects, project_id);
    let subtree_max_relative_depth = subtree_ids
        .iter()
        .filter_map(|id| projects.iter().find(|project| project.id() == id))
        .map(|project| {
            project
                .hierarchy
                .depth
                .saturating_sub(projects[index].hierarchy.depth)
        })
        .max()
        .unwrap_or(0);
    if parent_depth.saturating_add(subtree_max_relative_depth) > 2 {
        return Err(ProjectTreeError::MaximumDepth);
    }
    let old_depth = projects[index].hierarchy.depth;
    projects[index].hierarchy.parent_id = new_parent_id.cloned();
    projects[index].hierarchy.sibling_order = sibling_order;
    for project in projects.iter_mut() {
        if subtree_ids.iter().any(|id| id == project.id()) {
            project.hierarchy.depth =
                parent_depth + project.hierarchy.depth.saturating_sub(old_depth);
        }
    }
    Ok(())
}

pub fn reorder_project(
    projects: &mut [DocumentState],
    project_id: &ProjectId,
    target_id: &ProjectId,
    before: bool,
) -> Result<(), ProjectTreeError> {
    if project_id == target_id {
        return Err(ProjectTreeError::Cycle);
    }
    let project = projects
        .iter()
        .find(|project| project.id() == project_id)
        .ok_or(ProjectTreeError::ProjectNotFound)?;
    let target = projects
        .iter()
        .find(|project| project.id() == target_id)
        .ok_or(ProjectTreeError::ParentNotFound)?;
    if project.hierarchy.parent_id != target.hierarchy.parent_id {
        return Err(ProjectTreeError::CrossAudioAsset);
    }
    let mut sibling_ids: Vec<_> = projects
        .iter()
        .filter(|candidate| candidate.hierarchy.parent_id == target.hierarchy.parent_id)
        .map(|candidate| (candidate.hierarchy.sibling_order, candidate.id().clone()))
        .collect();
    sibling_ids.sort_by_key(|(order, _)| *order);
    sibling_ids.retain(|(_, id)| id != project_id);
    let target_index = sibling_ids
        .iter()
        .position(|(_, id)| id == target_id)
        .ok_or(ProjectTreeError::ParentNotFound)?;
    let insert_at = if before {
        target_index
    } else {
        target_index + 1
    };
    sibling_ids.insert(insert_at, (0, project_id.clone()));
    for (order, (_, id)) in sibling_ids.into_iter().enumerate() {
        if let Some(project) = projects.iter_mut().find(|project| project.id() == &id) {
            project.hierarchy.sibling_order = order as u32;
        }
    }
    Ok(())
}

fn overlaps(left_start: u64, left_end: u64, right_start: u64, right_end: u64) -> bool {
    left_start < right_end && right_start < left_end
}

fn is_descendant(projects: &[DocumentState], candidate: &ProjectId, ancestor: &ProjectId) -> bool {
    let mut current = Some(candidate);
    while let Some(id) = current {
        if id == ancestor {
            return true;
        }
        current = projects
            .iter()
            .find(|project| project.id() == id)
            .and_then(|project| project.hierarchy.parent_id.as_ref());
    }
    false
}

fn collect_subtree_ids(projects: &[DocumentState], root: &ProjectId) -> Vec<ProjectId> {
    let mut ids = vec![root.clone()];
    let mut cursor = 0;
    while cursor < ids.len() {
        let parent = ids[cursor].clone();
        ids.extend(
            projects
                .iter()
                .filter(|project| project.hierarchy.parent_id.as_ref() == Some(&parent))
                .map(|project| project.id().clone()),
        );
        cursor += 1;
    }
    ids
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiarizationSegment {
    pub id: String,
    pub speaker_id: String,
    #[serde(default)]
    pub speaker_profile_id: Option<String>,
    #[serde(default)]
    pub speaker_label: Option<String>,
    pub start_ms: u64,
    pub end_ms: u64,
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeakerTaggedSegment {
    pub transcript_segment_id: String,
    pub speaker_id: String,
    #[serde(default)]
    pub speaker_profile_id: Option<String>,
    #[serde(default)]
    pub speaker_label: Option<String>,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

pub fn align_transcript_and_speakers(
    transcript: &[TranscriptSegment],
    diarization: &[DiarizationSegment],
) -> Vec<SpeakerTaggedSegment> {
    transcript
        .iter()
        .map(|segment| {
            let speaker = diarization
                .iter()
                .filter_map(|candidate| {
                    let overlap_start = segment.start_ms.max(candidate.start_ms);
                    let overlap_end = segment.end_ms.min(candidate.end_ms);
                    if overlap_end > overlap_start {
                        Some((overlap_end - overlap_start, candidate))
                    } else {
                        None
                    }
                })
                .max_by_key(|(overlap, candidate)| {
                    (*overlap, std::cmp::Reverse(candidate.start_ms))
                })
                .map(|(_, candidate)| candidate);

            SpeakerTaggedSegment {
                transcript_segment_id: segment.id.clone(),
                speaker_id: speaker
                    .map(|candidate| candidate.speaker_id.clone())
                    .unwrap_or_else(|| "unknown".to_string()),
                speaker_profile_id: speaker
                    .and_then(|candidate| candidate.speaker_profile_id.clone()),
                speaker_label: speaker.and_then(|candidate| candidate.speaker_label.clone()),
                start_ms: segment.start_ms,
                end_ms: segment.end_ms,
                text: segment.text.clone(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligns_each_transcript_segment_with_the_largest_overlap() {
        let transcript = vec![TranscriptSegment {
            id: "t1".into(),
            start_ms: 0,
            end_ms: 1_000,
            text: "hello".into(),
        }];
        let diarization = vec![
            DiarizationSegment {
                id: "d1".into(),
                speaker_id: "speaker-a".into(),
                speaker_profile_id: None,
                speaker_label: None,
                start_ms: 0,
                end_ms: 300,
                confidence: None,
            },
            DiarizationSegment {
                id: "d2".into(),
                speaker_id: "speaker-b".into(),
                speaker_profile_id: Some("profile-b".into()),
                speaker_label: Some("Bob".into()),
                start_ms: 250,
                end_ms: 1_000,
                confidence: None,
            },
        ];

        let result = align_transcript_and_speakers(&transcript, &diarization);

        assert_eq!(result[0].speaker_id, "speaker-b");
        assert_eq!(result[0].speaker_profile_id.as_deref(), Some("profile-b"));
        assert_eq!(result[0].speaker_label.as_deref(), Some("Bob"));
    }

    #[test]
    fn marks_a_transcript_without_overlap_as_unknown() {
        let transcript = vec![TranscriptSegment {
            id: "t1".into(),
            start_ms: 0,
            end_ms: 1_000,
            text: "hello".into(),
        }];
        let diarization = vec![DiarizationSegment {
            id: "d1".into(),
            speaker_id: "speaker-a".into(),
            speaker_profile_id: None,
            speaker_label: None,
            start_ms: 2_000,
            end_ms: 3_000,
            confidence: None,
        }];

        let result = align_transcript_and_speakers(&transcript, &diarization);

        assert_eq!(result[0].speaker_id, "unknown");
    }

    #[test]
    fn old_diarization_json_defaults_new_identity_fields() {
        let segment: DiarizationSegment = serde_json::from_str(
            r#"{"id":"d1","speaker_id":"speaker-0","start_ms":0,"end_ms":1000,"confidence":null}"#,
        )
        .unwrap();

        assert_eq!(segment.speaker_id, "speaker-0");
        assert!(segment.speaker_profile_id.is_none());
        assert!(segment.speaker_label.is_none());
    }

    #[test]
    fn migrates_legacy_flat_document_state_into_layered_state() {
        let legacy = serde_json::json!({
            "schema_version": 1,
            "revision": 4,
            "updated_at_ms": 20,
            "id": "legacy",
            "title": "Legacy note",
            "audio_path": "recording.m4a",
            "created_at_ms": 10,
            "recorded_at_ms": 5,
            "audio_start_ms": 100,
            "audio_end_ms": 200,
            "parent_id": null,
            "audio_asset_id": null,
            "depth": 0,
            "sibling_order": 2,
            "tags": ["legacy"],
            "summary": "old summary",
            "processing_error": "old error",
            "status": "Failed",
            "transcript": [],
            "diarization": [],
            "speaker_segments": [],
            "markdown_path": null
        });

        let state: DocumentState = serde_json::from_value(legacy).unwrap();

        assert_eq!(state.id().as_str(), "legacy");
        assert_eq!(state.title(), "Legacy note");
        assert_eq!(state.source.audio_path, "recording.m4a");
        assert_eq!(state.hierarchy.sibling_order, 2);
        assert_eq!(state.content.tags, vec!["legacy"]);
        assert_eq!(state.processing_status(), ProjectStatus::Failed);
        assert_eq!(state.processing.last_error.as_deref(), Some("old error"));
        assert_eq!(state.schema.schema_version, DOCUMENT_STATE_SCHEMA_VERSION);

        let serialized = serde_json::to_value(state).unwrap();
        assert!(serialized.get("identity").is_some());
        assert!(serialized.get("title").is_none());
    }

    #[test]
    fn merges_speaker_labels_without_touching_transcript_text() {
        let mut project = DocumentState::new(ProjectId::new("p1"), "Meeting", "a.wav");
        project.results.diarization.push(DiarizationSegment {
            id: "d1".into(),
            speaker_id: "speaker-1".into(),
            speaker_profile_id: None,
            speaker_label: None,
            start_ms: 0,
            end_ms: 1_000,
            confidence: None,
        });
        project.results.speaker_segments.push(SpeakerTaggedSegment {
            transcript_segment_id: "t1".into(),
            speaker_id: "speaker-1".into(),
            speaker_profile_id: None,
            speaker_label: None,
            start_ms: 0,
            end_ms: 1_000,
            text: "keep this text".into(),
        });

        assert_eq!(project.merge_speaker("speaker-1", "speaker-0"), 1);
        assert_eq!(project.results.diarization[0].speaker_id, "speaker-0");
        assert_eq!(project.results.speaker_segments[0].text, "keep this text");
    }

    #[test]
    fn reparenting_rejects_cycles_and_keeps_tree_depth_bounded() {
        let root = DocumentState::new(ProjectId::new("root"), "Root", "a.wav");
        let mut child = root.semantic_child(ProjectId::new("child"), "Child", 0, 1_000, Vec::new());
        child.hierarchy.depth = 1;
        let mut grandchild = child.semantic_child(
            ProjectId::new("grandchild"),
            "Grandchild",
            0,
            500,
            Vec::new(),
        );
        grandchild.hierarchy.depth = 2;
        let mut projects = vec![root, child, grandchild];

        assert_eq!(
            reparent_project(
                &mut projects,
                &ProjectId::new("child"),
                Some(&ProjectId::new("grandchild")),
                0,
            ),
            Err(ProjectTreeError::Cycle)
        );
        assert_eq!(projects[1].hierarchy.depth, 1);
    }

    #[test]
    fn reordering_updates_sibling_order_before_or_after_a_target() {
        let mut first = DocumentState::new(ProjectId::new("first"), "First", "a.wav");
        let mut second = DocumentState::new(ProjectId::new("second"), "Second", "b.wav");
        let mut third = DocumentState::new(ProjectId::new("third"), "Third", "c.wav");
        first.hierarchy.sibling_order = 0;
        second.hierarchy.sibling_order = 1;
        third.hierarchy.sibling_order = 2;
        let mut projects = vec![first, second, third];

        reorder_project(
            &mut projects,
            &ProjectId::new("third"),
            &ProjectId::new("first"),
            true,
        )
        .unwrap();

        projects.sort_by_key(|project| project.hierarchy.sibling_order);
        assert_eq!(projects[0].id().as_str(), "third");
        assert_eq!(projects[1].id().as_str(), "first");
        assert_eq!(projects[2].id().as_str(), "second");
    }
}
