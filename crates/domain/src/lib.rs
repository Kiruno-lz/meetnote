use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectId(String);

impl ProjectId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeetingProject {
    pub id: ProjectId,
    pub title: String,
    pub audio_path: String,
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

impl MeetingProject {
    pub fn new(id: ProjectId, title: impl Into<String>, audio_path: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            audio_path: audio_path.into(),
            status: ProjectStatus::Imported,
            transcript: Vec::new(),
            diarization: Vec::new(),
            speaker_segments: Vec::new(),
            markdown_path: None,
        }
    }
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
    pub start_ms: u64,
    pub end_ms: u64,
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeakerTaggedSegment {
    pub transcript_segment_id: String,
    pub speaker_id: String,
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
            let speaker_id = diarization
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
                .map(|(_, candidate)| candidate.speaker_id.clone())
                .unwrap_or_else(|| "unknown".to_string());

            SpeakerTaggedSegment {
                transcript_segment_id: segment.id.clone(),
                speaker_id,
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
                start_ms: 0,
                end_ms: 300,
                confidence: None,
            },
            DiarizationSegment {
                id: "d2".into(),
                speaker_id: "speaker-b".into(),
                start_ms: 250,
                end_ms: 1_000,
                confidence: None,
            },
        ];

        let result = align_transcript_and_speakers(&transcript, &diarization);

        assert_eq!(result[0].speaker_id, "speaker-b");
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
            start_ms: 2_000,
            end_ms: 3_000,
            confidence: None,
        }];

        let result = align_transcript_and_speakers(&transcript, &diarization);

        assert_eq!(result[0].speaker_id, "unknown");
    }
}
