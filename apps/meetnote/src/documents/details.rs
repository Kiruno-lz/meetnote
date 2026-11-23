use domain::{DocumentState, ModelProvenance, ProvenanceGap, document_result_statistics};
use rune::{El, col, row};
use rune_kit::Kit;
use ui_text::Locale;

use crate::MeetNote;
use crate::controls::{caption, card, label, title};

use super::{audio_source_status, timestamp, workflow_label};

/// The details reading mode: one card per fact group, laid out like the summary.
pub(super) fn page(s: &MeetNote, k: &Kit) -> El {
    let state = &s.documents.current().state;
    let parent = state.hierarchy.parent_id.as_ref().map_or_else(
        || s.tr("detail_root").to_owned(),
        |id| {
            s.documents
                .tree
                .get(id.as_str())
                .map_or_else(|| id.to_string(), |document| document.title().to_owned())
        },
    );
    let child_count = s
        .documents
        .document_states()
        .filter(|candidate| candidate.hierarchy.parent_id.as_ref() == Some(state.id()))
        .count();
    let mut source = section(
        s,
        k,
        "detail_source",
        [
            ("detail_audio_path", known(Some(state.audio_path()))),
            ("detail_audio_status", audio_source_status(s).1.to_owned()),
        ],
    );
    if let Some(end) = state.source.audio_end_ms {
        source = source.child(fact(
            s,
            k,
            "detail_audio_range",
            format!(
                "{} – {}",
                timestamp(state.source.audio_start_ms),
                timestamp(end)
            ),
        ));
    }
    col()
        .key("document-details")
        .role("region")
        .label(s.tr("document_details"))
        .w_full()
        .gap(16.0)
        .padding(24.0)
        .child(section(
            s,
            k,
            "detail_identity",
            [
                ("detail_title", state.title().to_owned()),
                ("detail_id", state.id().to_string()),
            ],
        ))
        .child(source)
        .child(section(
            s,
            k,
            "detail_hierarchy",
            [
                ("detail_parent", parent),
                ("detail_depth", state.hierarchy.depth.to_string()),
                ("detail_children", child_count.to_string()),
            ],
        ))
        .child(section(
            s,
            k,
            "detail_state",
            [
                (
                    "detail_document_status",
                    workflow_label(s, &state.document_status.value).into(),
                ),
                ("detail_revision", state.revision().to_string()),
            ],
        ))
        .child(processing(s, k, state))
        .child(statistics(s, k, state))
}

fn processing(s: &MeetNote, k: &Kit, state: &DocumentState) -> El {
    let mut facts = match &state.processing.latest {
        Some(record) => vec![
            ("detail_asr", model_label(&record.asr)),
            (
                "detail_diarization",
                record.diarization.base_model.as_ref().map_or_else(
                    || {
                        joined([
                            Some(record.diarization.backend_id.as_str()),
                            record.diarization.model_revision.as_deref(),
                        ])
                    },
                    model_label,
                ),
            ),
            (
                "diarization_segmentation_model",
                record
                    .diarization
                    .segmentation
                    .as_ref()
                    .map_or_else(|| "—".into(), model_label),
            ),
            (
                "diarization_embedding_model",
                record
                    .diarization
                    .embedding
                    .as_ref()
                    .map_or_else(|| "—".into(), model_label),
            ),
            (
                "diarization_clustering",
                joined([
                    Some(record.diarization.clustering.id.as_str()),
                    record.diarization.clustering.revision.as_deref(),
                ]),
            ),
            ("detail_device", known(Some(&record.runtime.device))),
            ("detail_language", known(record.runtime.language.as_deref())),
            (
                "detail_input_checksum",
                known(record.input_audio_checksum.as_deref()),
            ),
        ],
        None => vec![(
            "detail_processing_record",
            s.tr("detail_not_processed").into(),
        )],
    };
    if let Some(error) = &state.processing.last_error {
        facts.push(("detail_last_error", error.clone()));
    }
    section(s, k, "detail_processing", facts)
}

fn statistics(s: &MeetNote, k: &Kit, state: &DocumentState) -> El {
    let values = document_result_statistics(state);
    if values.transcript_segments == 0 && values.diarization_segments == 0 {
        return section(s, k, "detail_statistics", []).child(caption(k, s.tr("detail_no_results")));
    }
    let gaps = if values.provenance_complete() {
        s.tr("detail_none").into()
    } else {
        let delimiter = if s.locale() == Locale::ZhCn {
            "、"
        } else {
            ", "
        };
        values
            .provenance_gaps
            .into_iter()
            .map(|gap| {
                s.tr(match gap {
                    ProvenanceGap::NoProcessingRecord => "gap_no_processing_record",
                    ProvenanceGap::CompletionTime => "gap_completion_time",
                    ProvenanceGap::InputAudioChecksum => "gap_input_audio_checksum",
                    ProvenanceGap::AsrModel => "gap_asr_model",
                    ProvenanceGap::AsrModelRevision => "gap_asr_model_revision",
                    ProvenanceGap::AsrArtifactChecksum => "gap_asr_artifact_checksum",
                    ProvenanceGap::DiarizationBackend => "gap_diarization_backend",
                    ProvenanceGap::DiarizationModelRevision => "gap_diarization_model_revision",
                    ProvenanceGap::Clustering => "gap_clustering",
                    ProvenanceGap::Alignment => "gap_alignment",
                    ProvenanceGap::RuntimeDevice => "gap_runtime_device",
                })
            })
            .collect::<Vec<_>>()
            .join(delimiter)
    };
    section(
        s,
        k,
        "detail_statistics",
        [
            (
                "detail_transcript_segments",
                values.transcript_segments.to_string(),
            ),
            (
                "detail_diarization_segments",
                values.diarization_segments.to_string(),
            ),
            ("detail_speakers", values.speakers.to_string()),
            (
                "detail_speech_duration",
                timestamp(values.speech_duration_ms),
            ),
            (
                "detail_characters",
                values.transcript_characters.to_string(),
            ),
            ("detail_words", values.transcript_words.to_string()),
            ("detail_provenance_gaps", gaps),
        ],
    )
}

fn section(
    s: &MeetNote,
    k: &Kit,
    heading: &str,
    rows: impl IntoIterator<Item = (&'static str, String)>,
) -> El {
    card(k)
        .child(title(k, s.tr(heading), 18.0))
        .children(rows.into_iter().map(|(key, value)| fact(s, k, key, value)))
}

fn fact(s: &MeetNote, k: &Kit, key: &str, value: String) -> El {
    row()
        .w_full()
        .items_center()
        .gap(16.0)
        .child(caption(k, s.tr(key)).w(168.0).min_w(168.0))
        .child(
            label(k, value)
                .selectable(true)
                .grow(1.0)
                .min_w(0.0)
                .key(key),
        )
}

fn model_label(model: &ModelProvenance) -> String {
    joined([
        Some(model.backend_id.as_str()),
        Some(model.model_id.as_str()),
        model.revision.as_deref(),
    ])
}

fn joined<'a>(parts: impl IntoIterator<Item = Option<&'a str>>) -> String {
    let text = parts
        .into_iter()
        .flatten()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    known(Some(&text))
}

fn known(value: Option<&str>) -> String {
    value
        .filter(|value| !value.is_empty())
        .unwrap_or("—")
        .into()
}
