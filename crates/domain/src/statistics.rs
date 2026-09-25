//! Result facts the Detail panel shows for one document.

use crate::{DocumentState, ModelProvenance, UNASSIGNED_SPEAKER_ID};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentResultStatistics {
    /// `results.transcript` segments.
    pub transcript_segments: usize,
    /// `results.diarization` turns.
    pub diarization_segments: usize,
    /// Distinct speaker ids of diarization and alignment, without the
    /// alignment placeholder for unattributed transcript.
    pub speakers: usize,
    /// Length of the union of all diarization turns; overlapping speech is
    /// counted once.
    pub speech_duration_ms: u64,
    /// Transcript characters other than whitespace.
    pub transcript_characters: usize,
    /// Transcript words as counted by [`count_words`].
    pub transcript_words: usize,
    /// Missing processing provenance facts; empty when complete.
    pub provenance_gaps: Vec<ProvenanceGap>,
}

impl DocumentResultStatistics {
    pub fn provenance_complete(&self) -> bool {
        self.provenance_gaps.is_empty()
    }
}

/// A provenance fact `processing.latest` lacks, in record order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProvenanceGap {
    NoProcessingRecord,
    CompletionTime,
    InputAudioChecksum,
    AsrModel,
    AsrModelRevision,
    AsrArtifactChecksum,
    DiarizationBackend,
    DiarizationModelRevision,
    Clustering,
    Alignment,
    RuntimeDevice,
}

pub fn document_result_statistics(state: &DocumentState) -> DocumentResultStatistics {
    let results = &state.results;
    let speakers = state
        .speaker_ids()
        .into_iter()
        .filter(|speaker_id| *speaker_id != UNASSIGNED_SPEAKER_ID)
        .count();
    DocumentResultStatistics {
        transcript_segments: results.transcript.len(),
        diarization_segments: results.diarization.len(),
        speakers,
        speech_duration_ms: covered_duration_ms(
            results
                .diarization
                .iter()
                .map(|segment| (segment.start_ms, segment.end_ms)),
        ),
        transcript_characters: results
            .transcript
            .iter()
            .map(|segment| segment.text.chars().filter(|c| !c.is_whitespace()).count())
            .sum(),
        transcript_words: results
            .transcript
            .iter()
            .map(|segment| count_words(&segment.text))
            .sum(),
        provenance_gaps: provenance_gaps(state),
    }
}

/// Counts each Han ideograph or kana as one word, since those scripts do not
/// separate words with spaces, and every other whitespace-separated run
/// containing a letter or digit as one word.
pub fn count_words(text: &str) -> usize {
    let mut words = 0;
    let mut run_has_word = false;
    for character in text.chars() {
        if character.is_whitespace() || is_unspaced_script(character) {
            words += usize::from(run_has_word);
            run_has_word = false;
            words += usize::from(is_unspaced_script(character));
        } else if character.is_alphanumeric() {
            run_has_word = true;
        }
    }
    words + usize::from(run_has_word)
}

fn is_unspaced_script(character: char) -> bool {
    matches!(
        u32::from(character),
        0x3040..=0x30FF   // Hiragana, Katakana
            | 0x3400..=0x4DBF // CJK Extension A
            | 0x4E00..=0x9FFF // CJK Unified Ideographs
            | 0xF900..=0xFAFF // CJK Compatibility Ideographs
            | 0x20000..=0x323AF // CJK Extensions B–H
    ) && character.is_alphanumeric()
}

fn covered_duration_ms(intervals: impl Iterator<Item = (u64, u64)>) -> u64 {
    let mut intervals: Vec<_> = intervals.filter(|(start, end)| end > start).collect();
    intervals.sort_unstable();
    let mut total = 0;
    let mut current: Option<(u64, u64)> = None;
    for (start, end) in intervals {
        match current {
            Some((open_start, open_end)) if start <= open_end => {
                current = Some((open_start, open_end.max(end)));
            }
            _ => {
                total += current.map_or(0, |(open_start, open_end)| open_end - open_start);
                current = Some((start, end));
            }
        }
    }
    total + current.map_or(0, |(start, end)| end - start)
}

fn provenance_gaps(state: &DocumentState) -> Vec<ProvenanceGap> {
    let Some(record) = &state.processing.latest else {
        return vec![ProvenanceGap::NoProcessingRecord];
    };
    let missing_model = |model: &ModelProvenance| {
        model.backend_id.trim().is_empty() || model.model_id.trim().is_empty()
    };
    [
        (record.completed_at_ms == 0, ProvenanceGap::CompletionTime),
        (
            record.input_audio_checksum.is_none(),
            ProvenanceGap::InputAudioChecksum,
        ),
        (missing_model(&record.asr), ProvenanceGap::AsrModel),
        (
            record.asr.revision.is_none(),
            ProvenanceGap::AsrModelRevision,
        ),
        (
            record.asr.artifact_sha256.is_none(),
            ProvenanceGap::AsrArtifactChecksum,
        ),
        (
            record.diarization.backend_id.trim().is_empty(),
            ProvenanceGap::DiarizationBackend,
        ),
        (
            record.diarization.model_revision.is_none(),
            ProvenanceGap::DiarizationModelRevision,
        ),
        (
            record.diarization.clustering.id.trim().is_empty(),
            ProvenanceGap::Clustering,
        ),
        (
            record.alignment.id.trim().is_empty(),
            ProvenanceGap::Alignment,
        ),
        (
            record.runtime.device.trim().is_empty(),
            ProvenanceGap::RuntimeDevice,
        ),
    ]
    .into_iter()
    .filter_map(|(missing, gap)| missing.then_some(gap))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        align_transcript_and_speakers, AlgorithmProvenance, DiarizationProvenance,
        DiarizationSegment, DocumentState, ModelProvenance, ProcessingRecord, ProjectId,
        RuntimeProvenance, TranscriptSegment,
    };

    fn transcript(id: &str, start_ms: u64, end_ms: u64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            id: id.into(),
            start_ms,
            end_ms,
            text: text.into(),
        }
    }

    fn turn(id: &str, speaker_id: &str, start_ms: u64, end_ms: u64) -> DiarizationSegment {
        DiarizationSegment {
            id: id.into(),
            speaker_id: speaker_id.into(),
            speaker_profile_id: None,
            speaker_label: None,
            start_ms,
            end_ms,
            confidence: None,
        }
    }

    fn model(backend_id: &str) -> ModelProvenance {
        ModelProvenance {
            backend_id: backend_id.into(),
            model_id: format!("{backend_id}-model"),
            revision: Some("r1".into()),
            artifact_sha256: Some("ab".repeat(32)),
            artifact_path: format!("models/{backend_id}"),
            artifacts: Vec::new(),
        }
    }

    fn complete_record() -> ProcessingRecord {
        ProcessingRecord {
            processing_revision: 1,
            completed_at_ms: 1_700_000_000_000,
            input_audio_checksum: Some("cd".repeat(32)),
            asr: model("asr"),
            diarization: DiarizationProvenance {
                backend_id: "diarization".into(),
                model_revision: Some("r2".into()),
                segmentation: Some(model("segmentation")),
                embedding: Some(model("embedding")),
                clustering: AlgorithmProvenance {
                    id: "clustering".into(),
                    revision: Some("1".into()),
                    parameters: None,
                },
                identity_matching: None,
            },
            alignment: AlgorithmProvenance {
                id: "max-overlap-speaker-alignment".into(),
                revision: Some("1".into()),
                parameters: None,
            },
            runtime: RuntimeProvenance {
                device: "Cpu".into(),
                language: Some("zh".into()),
                sample_rate_hz: Some(16_000),
                threads: None,
                expected_speakers: None,
                windowing: None,
            },
        }
    }

    /// Two overlapping turns and a separate one: 0–2500 ms and 4000–5000 ms
    /// of speech. The last transcript segment has no turn (`unknown`).
    fn fixture() -> DocumentState {
        let mut state = DocumentState::new(ProjectId::new("doc"), "Meeting", "meeting.wav");
        state.results.transcript = vec![
            transcript("t1", 0, 1_000, "你好，世界"),
            transcript("t2", 1_000, 2_500, "hello world, don't stop"),
            transcript("t3", 6_000, 7_000, " 3.5 km — ok\n"),
        ];
        state.results.diarization = vec![
            turn("d1", "speaker-0", 0, 1_200),
            turn("d2", "speaker-1", 1_000, 2_500),
            turn("d3", "speaker-0", 4_000, 5_000),
        ];
        state.results.speaker_segments =
            align_transcript_and_speakers(&state.results.transcript, &state.results.diarization);
        state
    }

    #[test]
    fn counts_the_results_of_a_processed_document() {
        let mut state = fixture();
        state.record_processing(complete_record());

        let statistics = document_result_statistics(&state);

        assert_eq!(
            statistics,
            DocumentResultStatistics {
                transcript_segments: 3,
                diarization_segments: 3,
                speakers: 2,
                speech_duration_ms: 3_500,
                transcript_characters: 33,
                transcript_words: 11,
                provenance_gaps: Vec::new(),
            }
        );
        assert!(statistics.provenance_complete());
    }

    #[test]
    fn an_unprocessed_document_has_empty_results_and_no_provenance() {
        let state = DocumentState::new(ProjectId::new("new"), "New", "new.wav");

        let statistics = document_result_statistics(&state);

        assert_eq!(statistics.transcript_segments, 0);
        assert_eq!(statistics.speakers, 0);
        assert_eq!(statistics.speech_duration_ms, 0);
        assert_eq!(statistics.transcript_words, 0);
        assert_eq!(
            statistics.provenance_gaps,
            vec![ProvenanceGap::NoProcessingRecord]
        );
        assert!(!statistics.provenance_complete());
    }

    #[test]
    fn reports_each_missing_provenance_fact() {
        let mut record = complete_record();
        record.input_audio_checksum = None;
        record.asr.revision = None;
        record.asr.artifact_sha256 = None;
        record.diarization.model_revision = None;
        let mut state = fixture();
        state.record_processing(record);

        assert_eq!(
            document_result_statistics(&state).provenance_gaps,
            vec![
                ProvenanceGap::InputAudioChecksum,
                ProvenanceGap::AsrModelRevision,
                ProvenanceGap::AsrArtifactChecksum,
                ProvenanceGap::DiarizationModelRevision,
            ]
        );

        let mut legacy = fixture();
        legacy.processing.latest = Some(ProcessingRecord::placeholder(4));
        let gaps = document_result_statistics(&legacy).provenance_gaps;
        assert!(gaps.contains(&ProvenanceGap::CompletionTime));
        assert!(gaps.contains(&ProvenanceGap::AsrModel));
        assert!(gaps.contains(&ProvenanceGap::RuntimeDevice));
    }

    #[test]
    fn counts_words_across_scripts() {
        assert_eq!(count_words("你好world"), 3);
        assert_eq!(count_words("こんにちは、カタカナ"), 9);
        assert_eq!(count_words("안녕 하세요"), 2);
        assert_eq!(count_words("state-of-the-art — ok"), 2);
        assert_eq!(count_words("  \n\t"), 0);
    }
}
