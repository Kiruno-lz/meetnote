use domain::{DocumentState, ModelProvenance, ProvenanceGap, document_result_statistics};
use rune::{El, col, row};
use rune_kit::{Divider, Kit, glyphs};
use ui_text::Locale;

use crate::MeetNote;
use crate::controls::{caption, icon_action, label, scroll, title};

use super::{audio_source_status, timestamp, transcript_lines, workflow_label};

pub(super) fn panel(s: &MeetNote, k: &Kit) -> El {
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
    let body = col()
        .w_full()
        .padding(16.0)
        .gap(16.0)
        .child(
            row()
                .items_center()
                .justify_between()
                .child(title(k, s.tr("document_details"), 18.0))
                .child(icon_action(
                    k,
                    "detail-close",
                    &glyphs::CLOSE,
                    s.tr("hide_detail"),
                    |s, tx| {
                        s.documents.detail = false;
                        tx.focus("detail-toggle", tx.focus_visible());
                    },
                )),
        )
        .child(Divider::horizontal().build(k))
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
        .child(transcript(s, k));
    col()
        .key("detail-panel")
        .role("region")
        .label(s.tr("document_details"))
        .w(235.0)
        .min_w(235.0)
        .h_full()
        .fill(k.c.layer_01)
        .child(scroll(k, "detail-scroll", body))
}

fn transcript(s: &MeetNote, k: &Kit) -> El {
    let document = s.documents.current();
    let mut lines = transcript_lines(document).peekable();
    let body = col()
        .gap(8.0)
        .child(title(k, s.tr("detail_transcript"), 14.0));
    if lines.peek().is_none() {
        return body.child(caption(k, s.tr("transcript_empty")));
    }
    body.children(lines.enumerate().map(|(index, (start, speaker, text))| {
        col()
            .gap(3.0)
            .key(format!("detail-transcript-{index}"))
            .child(
                row()
                    .gap(8.0)
                    .flex_wrap()
                    .child(caption(k, timestamp(start)))
                    .child(caption(
                        k,
                        if speaker.is_empty() {
                            s.tr("unattributed_speech")
                        } else {
                            speaker
                        },
                    )),
            )
            .child(label(k, text).selectable(true))
    }))
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
    col()
        .gap(9.0)
        .child(title(k, s.tr(heading), 14.0))
        .children(rows.into_iter().map(|(key, value)| fact(s, k, key, value)))
}

fn fact(s: &MeetNote, k: &Kit, key: &str, value: String) -> El {
    col()
        .gap(2.0)
        .child(caption(k, s.tr(key)))
        .child(label(k, value).selectable(true).key(key))
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
