//! Manual speaker corrections. They change who a segment is attributed to and
//! how that speaker is displayed; ASR text, timings, audio source and
//! processing provenance are never touched.

use crate::DocumentState;
use std::collections::BTreeSet;

/// Speaker id that alignment assigns to a transcript segment no diarization
/// turn overlaps.
pub const UNASSIGNED_SPEAKER_ID: &str = "unknown";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpeakerCorrection {
    /// Attribute every segment of `from` to `into`, which keeps its profile
    /// identity and display label.
    Merge { from: String, into: String },
    /// Set (trimmed) or clear the display label of every segment of one
    /// speaker. Profile identities stay as matched against the Speaker
    /// Library.
    Label {
        speaker_id: String,
        label: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpeakerCorrectionError {
    UnknownSpeaker(String),
    SameSpeaker(String),
    EmptyLabel,
}

impl std::fmt::Display for SpeakerCorrectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownSpeaker(id) => write!(f, "speaker {id} does not occur in this document"),
            Self::SameSpeaker(id) => write!(f, "cannot merge speaker {id} into itself"),
            Self::EmptyLabel => f.write_str("speaker label cannot be empty"),
        }
    }
}

impl std::error::Error for SpeakerCorrectionError {}

impl DocumentState {
    /// Speaker ids used by `results.diarization` or `results.speaker_segments`.
    pub fn speaker_ids(&self) -> BTreeSet<&str> {
        self.results
            .diarization
            .iter()
            .map(|segment| segment.speaker_id.as_str())
            .chain(
                self.results
                    .speaker_segments
                    .iter()
                    .map(|segment| segment.speaker_id.as_str()),
            )
            .collect()
    }

    /// Applies `correction` to the diarization turns and the aligned speaker
    /// segments and returns how many segments changed. Nothing changes on
    /// error.
    pub fn apply_speaker_correction(
        &mut self,
        correction: &SpeakerCorrection,
    ) -> Result<usize, SpeakerCorrectionError> {
        match correction {
            SpeakerCorrection::Merge { from, into } => {
                self.ensure_speaker(from)?;
                self.ensure_speaker(into)?;
                if from == into {
                    return Err(SpeakerCorrectionError::SameSpeaker(from.clone()));
                }
                let (profile_id, label) = self.speaker_identity(into);
                Ok(self.rewrite_speaker(from, |speaker_id, profile, display| {
                    *speaker_id = into.clone();
                    *profile = profile_id.clone();
                    *display = label.clone();
                }))
            }
            SpeakerCorrection::Label { speaker_id, label } => {
                let label = match label {
                    Some(label) if label.trim().is_empty() => {
                        return Err(SpeakerCorrectionError::EmptyLabel)
                    }
                    Some(label) => Some(label.trim().to_string()),
                    None => None,
                };
                self.ensure_speaker(speaker_id)?;
                Ok(self.rewrite_speaker(speaker_id, |_, _, display| {
                    *display = label.clone();
                }))
            }
        }
    }

    fn ensure_speaker(&self, speaker_id: &str) -> Result<(), SpeakerCorrectionError> {
        if self.speaker_ids().contains(speaker_id) {
            Ok(())
        } else {
            Err(SpeakerCorrectionError::UnknownSpeaker(speaker_id.into()))
        }
    }

    /// The profile id and label of the first segment of `speaker_id`,
    /// diarization turns first.
    fn speaker_identity(&self, speaker_id: &str) -> (Option<String>, Option<String>) {
        self.results
            .diarization
            .iter()
            .find(|segment| segment.speaker_id == speaker_id)
            .map(|segment| {
                (
                    segment.speaker_profile_id.clone(),
                    segment.speaker_label.clone(),
                )
            })
            .or_else(|| {
                self.results
                    .speaker_segments
                    .iter()
                    .find(|segment| segment.speaker_id == speaker_id)
                    .map(|segment| {
                        (
                            segment.speaker_profile_id.clone(),
                            segment.speaker_label.clone(),
                        )
                    })
            })
            .unwrap_or_default()
    }

    /// Runs `rewrite` on the speaker fields of every segment of `speaker_id`
    /// and counts the segments whose fields differ afterwards.
    fn rewrite_speaker(
        &mut self,
        speaker_id: &str,
        rewrite: impl Fn(&mut String, &mut Option<String>, &mut Option<String>),
    ) -> usize {
        let mut changed = 0;
        let mut apply =
            |id: &mut String, profile: &mut Option<String>, label: &mut Option<String>| {
                if id.as_str() != speaker_id {
                    return;
                }
                let before = (id.clone(), profile.clone(), label.clone());
                rewrite(id, profile, label);
                if before != (id.clone(), profile.clone(), label.clone()) {
                    changed += 1;
                }
            };
        for segment in &mut self.results.diarization {
            apply(
                &mut segment.speaker_id,
                &mut segment.speaker_profile_id,
                &mut segment.speaker_label,
            );
        }
        for segment in &mut self.results.speaker_segments {
            apply(
                &mut segment.speaker_id,
                &mut segment.speaker_profile_id,
                &mut segment.speaker_label,
            );
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        align_transcript_and_speakers, DiarizationSegment, DocumentState, ProjectId,
        TranscriptSegment,
    };

    fn transcript(id: &str, start_ms: u64, end_ms: u64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            id: id.into(),
            start_ms,
            end_ms,
            text: text.into(),
        }
    }

    fn turn(
        id: &str,
        speaker_id: &str,
        identity: Option<(&str, &str)>,
        start_ms: u64,
        end_ms: u64,
    ) -> DiarizationSegment {
        DiarizationSegment {
            id: id.into(),
            speaker_id: speaker_id.into(),
            speaker_profile_id: identity.map(|(profile, _)| profile.into()),
            speaker_label: identity.map(|(_, label)| label.into()),
            start_ms,
            end_ms,
            confidence: Some(0.9),
        }
    }

    /// `speaker-0` is matched to the Alice profile, `speaker-1` is anonymous
    /// and the last transcript segment has no speaker turn (`unknown`).
    fn fixture() -> DocumentState {
        let mut state = DocumentState::new(ProjectId::new("doc"), "Meeting", "meeting.wav");
        state.results.transcript = vec![
            transcript("t1", 0, 1_000, "你好"),
            transcript("t2", 1_000, 2_500, "hello there"),
            transcript("t3", 6_000, 7_000, "nobody"),
        ];
        state.results.diarization = vec![
            turn("d1", "speaker-0", Some(("p-alice", "Alice")), 0, 1_000),
            turn("d2", "speaker-1", None, 1_000, 2_500),
            turn("d3", "speaker-1", None, 3_000, 4_000),
        ];
        state.results.speaker_segments =
            align_transcript_and_speakers(&state.results.transcript, &state.results.diarization);
        state
    }

    fn timings(state: &DocumentState) -> Vec<(u64, u64)> {
        state
            .results
            .diarization
            .iter()
            .map(|segment| (segment.start_ms, segment.end_ms))
            .chain(
                state
                    .results
                    .speaker_segments
                    .iter()
                    .map(|segment| (segment.start_ms, segment.end_ms)),
            )
            .collect()
    }

    fn texts(state: &DocumentState) -> Vec<String> {
        state
            .results
            .speaker_segments
            .iter()
            .map(|segment| segment.text.clone())
            .collect()
    }

    fn assert_facts_unchanged(before: &DocumentState, after: &DocumentState) {
        assert_eq!(after.results.transcript, before.results.transcript);
        assert_eq!(timings(after), timings(before));
        assert_eq!(texts(after), texts(before));
        assert_eq!(after.source, before.source);
        assert_eq!(after.processing, before.processing);
        assert_eq!(after.schema, before.schema);
    }

    #[test]
    fn lists_the_speakers_of_diarization_and_alignment() {
        let state = fixture();

        assert_eq!(
            state.speaker_ids().into_iter().collect::<Vec<_>>(),
            vec!["speaker-0", "speaker-1", UNASSIGNED_SPEAKER_ID]
        );
    }

    #[test]
    fn merge_moves_every_segment_to_the_target_identity() {
        let before = fixture();
        let mut state = before.clone();

        let changed = state
            .apply_speaker_correction(&SpeakerCorrection::Merge {
                from: "speaker-1".into(),
                into: "speaker-0".into(),
            })
            .unwrap();

        assert_eq!(changed, 3);
        for segment in &state.results.diarization {
            assert_eq!(segment.speaker_id, "speaker-0");
            assert_eq!(segment.speaker_profile_id.as_deref(), Some("p-alice"));
            assert_eq!(segment.speaker_label.as_deref(), Some("Alice"));
        }
        let second = &state.results.speaker_segments[1];
        assert_eq!(second.speaker_id, "speaker-0");
        assert_eq!(second.speaker_label.as_deref(), Some("Alice"));
        assert_eq!(
            state.results.speaker_segments[2].speaker_id,
            UNASSIGNED_SPEAKER_ID
        );
        assert_eq!(
            state.speaker_ids().into_iter().collect::<Vec<_>>(),
            vec!["speaker-0", UNASSIGNED_SPEAKER_ID]
        );
        assert_facts_unchanged(&before, &state);
    }

    #[test]
    fn label_names_one_speaker_and_keeps_profile_identities() {
        let before = fixture();
        let mut state = before.clone();

        let changed = state
            .apply_speaker_correction(&SpeakerCorrection::Label {
                speaker_id: "speaker-1".into(),
                label: Some("  Bob ".into()),
            })
            .unwrap();

        assert_eq!(changed, 3);
        for segment in &state.results.diarization {
            let expected = if segment.speaker_id == "speaker-1" {
                "Bob"
            } else {
                "Alice"
            };
            assert_eq!(segment.speaker_label.as_deref(), Some(expected));
        }
        assert_eq!(
            state.results.speaker_segments[1].speaker_label.as_deref(),
            Some("Bob")
        );
        assert_eq!(
            state.results.diarization[0].speaker_profile_id.as_deref(),
            Some("p-alice")
        );
        assert_eq!(state.results.diarization[1].speaker_profile_id, None);
        assert_facts_unchanged(&before, &state);

        let changed = state
            .apply_speaker_correction(&SpeakerCorrection::Label {
                speaker_id: "speaker-0".into(),
                label: None,
            })
            .unwrap();

        assert_eq!(changed, 2);
        assert_eq!(state.results.diarization[0].speaker_label, None);
        assert_eq!(
            state.results.diarization[0].speaker_profile_id.as_deref(),
            Some("p-alice")
        );
        assert_eq!(
            state
                .clone()
                .apply_speaker_correction(&SpeakerCorrection::Label {
                    speaker_id: "speaker-0".into(),
                    label: None,
                })
                .unwrap(),
            0
        );
    }

    #[test]
    fn rejects_unknown_speakers_and_invalid_requests_without_changes() {
        let before = fixture();
        let cases = [
            (
                SpeakerCorrection::Merge {
                    from: "speaker-9".into(),
                    into: "speaker-0".into(),
                },
                SpeakerCorrectionError::UnknownSpeaker("speaker-9".into()),
            ),
            (
                SpeakerCorrection::Merge {
                    from: "speaker-1".into(),
                    into: "speaker-9".into(),
                },
                SpeakerCorrectionError::UnknownSpeaker("speaker-9".into()),
            ),
            (
                SpeakerCorrection::Merge {
                    from: "speaker-1".into(),
                    into: "speaker-1".into(),
                },
                SpeakerCorrectionError::SameSpeaker("speaker-1".into()),
            ),
            (
                SpeakerCorrection::Label {
                    speaker_id: "speaker-9".into(),
                    label: Some("Carol".into()),
                },
                SpeakerCorrectionError::UnknownSpeaker("speaker-9".into()),
            ),
            (
                SpeakerCorrection::Label {
                    speaker_id: "speaker-1".into(),
                    label: Some("   ".into()),
                },
                SpeakerCorrectionError::EmptyLabel,
            ),
        ];

        for (correction, expected) in cases {
            let mut state = before.clone();
            assert_eq!(state.apply_speaker_correction(&correction), Err(expected));
            assert_eq!(state, before);
        }
    }

    #[test]
    fn correction_errors_read_as_sentences() {
        for error in [
            SpeakerCorrectionError::UnknownSpeaker("speaker-9".into()),
            SpeakerCorrectionError::SameSpeaker("speaker-1".into()),
            SpeakerCorrectionError::EmptyLabel,
        ] {
            let message = error.to_string();
            assert!(!message.contains(['{', '}', '"']), "{message}");
        }
        assert_eq!(
            SpeakerCorrectionError::UnknownSpeaker("speaker-9".into()).to_string(),
            "speaker speaker-9 does not occur in this document"
        );
    }
}
