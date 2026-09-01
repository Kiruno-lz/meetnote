use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const DOCUMENT_STATE_SCHEMA_VERSION: u32 = 1;

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentState {
    #[serde(default = "default_document_state_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub updated_at_ms: u64,
    pub id: ProjectId,
    pub title: String,
    pub audio_path: String,
    #[serde(default)]
    pub created_at_ms: u64,
    #[serde(default)]
    pub recorded_at_ms: Option<u64>,
    #[serde(default)]
    pub audio_start_ms: u64,
    #[serde(default)]
    pub audio_end_ms: Option<u64>,
    #[serde(default)]
    pub parent_id: Option<ProjectId>,
    #[serde(default)]
    pub audio_asset_id: Option<ProjectId>,
    #[serde(default)]
    pub depth: u8,
    #[serde(default)]
    pub sibling_order: u32,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub processing_error: Option<String>,
    pub status: ProjectStatus,
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

impl DocumentState {
    pub fn new(id: ProjectId, title: impl Into<String>, audio_path: impl Into<String>) -> Self {
        let timestamp = now_ms();
        Self {
            schema_version: DOCUMENT_STATE_SCHEMA_VERSION,
            revision: 0,
            updated_at_ms: timestamp,
            id,
            title: title.into(),
            audio_path: audio_path.into(),
            created_at_ms: timestamp,
            recorded_at_ms: None,
            audio_start_ms: 0,
            audio_end_ms: None,
            parent_id: None,
            audio_asset_id: None,
            depth: 0,
            sibling_order: 0,
            tags: Vec::new(),
            summary: None,
            processing_error: None,
            status: ProjectStatus::Imported,
            transcript: Vec::new(),
            diarization: Vec::new(),
            speaker_segments: Vec::new(),
            markdown_path: None,
        }
    }

    pub fn validate_schema(&self) -> Result<(), DocumentStateError> {
        if self.schema_version != DOCUMENT_STATE_SCHEMA_VERSION {
            return Err(DocumentStateError::UnsupportedSchema {
                actual: self.schema_version,
                expected: DOCUMENT_STATE_SCHEMA_VERSION,
            });
        }
        Ok(())
    }

    pub fn is_root(&self) -> bool {
        self.parent_id.is_none()
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
            .audio_asset_id
            .clone()
            .unwrap_or_else(|| self.id.clone());
        let mut child = Self::new(id, title, self.audio_path.clone());
        child.recorded_at_ms = self.recorded_at_ms;
        child.audio_start_ms = start_ms;
        child.audio_end_ms = Some(end_ms);
        child.parent_id = Some(self.id.clone());
        child.audio_asset_id = Some(root_id);
        child.depth = self.depth.saturating_add(1);
        child.tags = tags;
        child.status = self.status;
        child.transcript = self
            .transcript
            .iter()
            .filter(|segment| overlaps(segment.start_ms, segment.end_ms, start_ms, end_ms))
            .cloned()
            .collect();
        child.diarization = self
            .diarization
            .iter()
            .filter(|segment| overlaps(segment.start_ms, segment.end_ms, start_ms, end_ms))
            .cloned()
            .collect();
        child.speaker_segments = self
            .speaker_segments
            .iter()
            .filter(|segment| overlaps(segment.start_ms, segment.end_ms, start_ms, end_ms))
            .cloned()
            .collect();
        child
    }

    pub fn merge_speaker(&mut self, from: &str, into: &str) -> usize {
        let mut changed = 0;
        for segment in &mut self.diarization {
            if segment.speaker_id == from {
                segment.speaker_id = into.to_string();
                changed += 1;
            }
        }
        for segment in &mut self.speaker_segments {
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

fn default_document_state_schema_version() -> u32 {
    DOCUMENT_STATE_SCHEMA_VERSION
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
        .position(|project| &project.id == project_id)
        .ok_or(ProjectTreeError::ProjectNotFound)?;
    let root_id = projects[index]
        .audio_asset_id
        .as_ref()
        .unwrap_or(&projects[index].id)
        .clone();
    let parent_depth = match new_parent_id {
        Some(parent_id) => {
            let parent = projects
                .iter()
                .find(|project| &project.id == parent_id)
                .ok_or(ProjectTreeError::ParentNotFound)?;
            if parent.id == *project_id {
                return Err(ProjectTreeError::Cycle);
            }
            if parent.audio_asset_id.as_ref().unwrap_or(&parent.id) != &root_id {
                return Err(ProjectTreeError::CrossAudioAsset);
            }
            if is_descendant(projects, parent_id, project_id) {
                return Err(ProjectTreeError::Cycle);
            }
            parent.depth + 1
        }
        None => 0,
    };
    let subtree_ids = collect_subtree_ids(projects, project_id);
    let subtree_max_relative_depth = subtree_ids
        .iter()
        .filter_map(|id| projects.iter().find(|project| &project.id == id))
        .map(|project| project.depth.saturating_sub(projects[index].depth))
        .max()
        .unwrap_or(0);
    if parent_depth.saturating_add(subtree_max_relative_depth) > 2 {
        return Err(ProjectTreeError::MaximumDepth);
    }
    let old_depth = projects[index].depth;
    projects[index].parent_id = new_parent_id.cloned();
    projects[index].sibling_order = sibling_order;
    for project in projects.iter_mut() {
        if subtree_ids.iter().any(|id| id == &project.id) {
            project.depth = parent_depth + project.depth.saturating_sub(old_depth);
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
        .find(|project| &project.id == project_id)
        .ok_or(ProjectTreeError::ProjectNotFound)?;
    let target = projects
        .iter()
        .find(|project| &project.id == target_id)
        .ok_or(ProjectTreeError::ParentNotFound)?;
    if project.parent_id != target.parent_id {
        return Err(ProjectTreeError::CrossAudioAsset);
    }
    let mut sibling_ids: Vec<_> = projects
        .iter()
        .filter(|candidate| candidate.parent_id == target.parent_id)
        .map(|candidate| (candidate.sibling_order, candidate.id.clone()))
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
        if let Some(project) = projects.iter_mut().find(|project| project.id == id) {
            project.sibling_order = order as u32;
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
            .find(|project| &project.id == id)
            .and_then(|project| project.parent_id.as_ref());
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
                .filter(|project| project.parent_id.as_ref() == Some(&parent))
                .map(|project| project.id.clone()),
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
    fn merges_speaker_labels_without_touching_transcript_text() {
        let mut project = DocumentState::new(ProjectId::new("p1"), "Meeting", "a.wav");
        project.diarization.push(DiarizationSegment {
            id: "d1".into(),
            speaker_id: "speaker-1".into(),
            speaker_profile_id: None,
            speaker_label: None,
            start_ms: 0,
            end_ms: 1_000,
            confidence: None,
        });
        project.speaker_segments.push(SpeakerTaggedSegment {
            transcript_segment_id: "t1".into(),
            speaker_id: "speaker-1".into(),
            speaker_profile_id: None,
            speaker_label: None,
            start_ms: 0,
            end_ms: 1_000,
            text: "keep this text".into(),
        });

        assert_eq!(project.merge_speaker("speaker-1", "speaker-0"), 1);
        assert_eq!(project.diarization[0].speaker_id, "speaker-0");
        assert_eq!(project.speaker_segments[0].text, "keep this text");
    }

    #[test]
    fn reparenting_rejects_cycles_and_keeps_tree_depth_bounded() {
        let root = DocumentState::new(ProjectId::new("root"), "Root", "a.wav");
        let mut child = root.semantic_child(ProjectId::new("child"), "Child", 0, 1_000, Vec::new());
        child.depth = 1;
        let mut grandchild = child.semantic_child(
            ProjectId::new("grandchild"),
            "Grandchild",
            0,
            500,
            Vec::new(),
        );
        grandchild.depth = 2;
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
        assert_eq!(projects[1].depth, 1);
    }

    #[test]
    fn reordering_updates_sibling_order_before_or_after_a_target() {
        let mut first = DocumentState::new(ProjectId::new("first"), "First", "a.wav");
        let mut second = DocumentState::new(ProjectId::new("second"), "Second", "b.wav");
        let mut third = DocumentState::new(ProjectId::new("third"), "Third", "c.wav");
        first.sibling_order = 0;
        second.sibling_order = 1;
        third.sibling_order = 2;
        let mut projects = vec![first, second, third];

        reorder_project(
            &mut projects,
            &ProjectId::new("third"),
            &ProjectId::new("first"),
            true,
        )
        .unwrap();

        projects.sort_by_key(|project| project.sibling_order);
        assert_eq!(projects[0].id.as_str(), "third");
        assert_eq!(projects[1].id.as_str(), "first");
        assert_eq!(projects[2].id.as_str(), "second");
    }
}
