use crate::{
    components::{empty_card, timestamp},
    controller::RootView,
    controls::{Button, Region, Scrollbar},
    i18n::{text as tr, Locale},
    theme::{self, Tokens},
};
use application::{AudioSourceError, AudioStatus};
use domain::{document_result_statistics, DocumentState, ProvenanceGap};
use gpui::{
    div, prelude::*, px, relative, Context, FocusHandle, IntoElement, Pixels, ScrollHandle,
};

const DETAIL_WIDTH: Pixels = px(400.);
/// The label column of a Detail row.
const LABEL_WIDTH: Pixels = px(116.);
/// Shown for a fact the document does not record.
const MISSING: &str = "—";

/// Detail: the selected document's facts in an opaque panel at the right of
/// the content plane, beside the document rather than over it. Opening or
/// closing it changes nothing but its own visibility. The facts and the
/// transcript scroll together; every value is read from the loaded state.
/// The panel is a group that holds `focus`, so closing it from the keyboard
/// can return focus to the button that opened it.
pub(crate) fn render(
    project: Option<&DocumentState>,
    projects: &[DocumentState],
    audio: Option<&Result<AudioStatus, AudioSourceError>>,
    locale: Locale,
    scroll: &ScrollHandle,
    focus: &FocusHandle,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let tokens = *theme::tokens(cx);
    let content = project
        .map(|project| {
            let transcript = div()
                .mt(tokens.spacing.lg)
                .child(section_title(tr(locale, "detail_transcript"), &tokens))
                .children(project.results.speaker_segments.iter().map(|segment| {
                    div()
                        .pt(tokens.spacing.sm)
                        .child(div().text_xs().text_color(tokens.text_muted).child(format!(
                                    "{}  {}",
                                    timestamp(segment.start_ms),
                                    segment
                                        .speaker_label
                                        .as_deref()
                                        .unwrap_or(&segment.speaker_id)
                                )))
                        .child(div().text_sm().child(segment.text.clone()))
                }));
            let body = div()
                .id("detail-body")
                .debug_selector(|| "detail-body".into())
                .size_full()
                .overflow_y_scroll()
                .track_scroll(scroll)
                .pb(tokens.spacing.lg)
                .children(
                    detail_sections(project, projects, locale, audio)
                        .into_iter()
                        .map(|section| render_section(section, &tokens)),
                )
                .child(transcript);
            // The scrollbar stays outside the fade.
            div()
                .relative()
                .mt(tokens.spacing.md)
                .flex_1()
                .min_h_0()
                .child(theme::scroll_faded(scroll, tokens.raised, body))
                .child(Scrollbar::vertical("detail-scrollbar", scroll))
                .into_any_element()
        })
        .unwrap_or_else(|| {
            empty_card(tr(locale, "select_project_first"), &tokens).into_any_element()
        });
    // The panel's right corners follow the content plane's.
    Region::new("detail-panel", tr(locale, "project_detail"), focus)
        .debug_selector(|| "detail-panel".into())
        .w(DETAIL_WIDTH)
        .max_w(relative(0.5))
        .h_full()
        .flex_shrink_0()
        .p_5()
        .rounded_tr(tokens.radius.lg)
        .rounded_br(tokens.radius.lg)
        .border_l_1()
        .border_color(tokens.hairline)
        .bg(tokens.raised)
        .rounded_r(tokens.radius.xl)
        .flex()
        .flex_col()
        .child(
            div()
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .text_color(tokens.accent_text)
                        .child(tr(locale, "project_detail")),
                )
                .child(
                    Button::new("close-detail", tr(locale, "close_detail"))
                        .debug_selector(|| "close-detail".into())
                        .ghost()
                        .compact()
                        .glyph("×")
                        .on_click(cx.listener(RootView::close_detail)),
                ),
        )
        .child(content)
}

fn section_title(title: &'static str, tokens: &Tokens) -> impl IntoElement {
    div()
        .pb(tokens.spacing.xs)
        .border_b_1()
        .border_color(tokens.hairline)
        .text_xs()
        .text_color(tokens.accent_text)
        .child(title)
}

fn render_section(section: DetailSection, tokens: &Tokens) -> impl IntoElement {
    div()
        .pt(tokens.spacing.md)
        .flex()
        .flex_col()
        .child(section_title(section.title, tokens))
        .children(section.rows.into_iter().map(|row| {
            let selector = format!(
                "detail-{}",
                row.key.trim_start_matches("detail_").replace('_', "-")
            );
            div()
                .debug_selector(move || selector.clone())
                .pt(tokens.spacing.xs)
                .flex()
                .gap(tokens.spacing.sm)
                .text_sm()
                .child(
                    div()
                        .w(LABEL_WIDTH)
                        .flex_shrink_0()
                        .text_color(tokens.text_muted)
                        .child(row.label),
                )
                .child(div().flex_1().min_w_0().child(row.value))
        }))
}

/// One labelled fact of Detail; `key` is its label's translation key and
/// names its row (`detail-<key>` with dashes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DetailRow {
    pub(crate) key: &'static str,
    pub(crate) label: &'static str,
    pub(crate) value: String,
}

/// A titled group of Detail rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DetailSection {
    pub(crate) title: &'static str,
    pub(crate) rows: Vec<DetailRow>,
}

/// The read-only facts Detail shows for `project`, whose tree is `projects`:
/// identity, audio source and what the latest `audio` check of it found,
/// hierarchy, workflow state and revision, the processing record and the
/// result statistics. A pure projection of the loaded state; nothing here
/// reads files.
pub(crate) fn detail_sections(
    project: &DocumentState,
    projects: &[DocumentState],
    locale: Locale,
    audio: Option<&Result<AudioStatus, AudioSourceError>>,
) -> Vec<DetailSection> {
    let text = |key| tr(locale, key);
    let row = |key: &'static str, value: String| DetailRow {
        key,
        label: tr(locale, key),
        value,
    };
    let known = |value: Option<&str>| {
        value
            .filter(|value| !value.is_empty())
            .unwrap_or(MISSING)
            .to_string()
    };
    let joined = |parts: &[Option<&str>]| {
        let parts = parts
            .iter()
            .flatten()
            .filter(|part| !part.is_empty())
            .copied()
            .collect::<Vec<_>>();
        if parts.is_empty() {
            MISSING.to_string()
        } else {
            parts.join(" · ")
        }
    };

    let mut source = vec![
        row("detail_audio_path", known(Some(project.audio_path()))),
        row(
            "detail_audio_status",
            match audio {
                None => text("audio_checking").to_string(),
                Some(Ok(status)) => crate::i18n::audio_status(locale, status),
                Some(Err(error)) => crate::i18n::audio_source_error(locale, error),
            },
        ),
    ];
    if let Some(end_ms) = project.source.audio_end_ms {
        source.push(row(
            "detail_audio_range",
            format!(
                "{} – {}",
                timestamp(project.source.audio_start_ms),
                timestamp(end_ms)
            ),
        ));
    }
    let parent = match &project.hierarchy.parent_id {
        None => text("detail_root").to_string(),
        Some(parent_id) => projects
            .iter()
            .find(|candidate| candidate.id() == parent_id)
            .map_or_else(
                || parent_id.to_string(),
                |parent| parent.title().to_string(),
            ),
    };
    let children = projects
        .iter()
        .filter(|candidate| candidate.hierarchy.parent_id.as_ref() == Some(project.id()))
        .count();

    let mut processing = match &project.processing.latest {
        None => vec![row(
            "detail_processing_record",
            text("detail_not_processed").into(),
        )],
        Some(record) => vec![
            row(
                "detail_asr",
                joined(&[
                    Some(&record.asr.backend_id),
                    Some(&record.asr.model_id),
                    record.asr.revision.as_deref(),
                ]),
            ),
            row(
                "detail_diarization",
                joined(&[
                    Some(&record.diarization.backend_id),
                    record.diarization.model_revision.as_deref(),
                ]),
            ),
            row("detail_device", known(Some(&record.runtime.device))),
            row("detail_language", known(record.runtime.language.as_deref())),
            row(
                "detail_input_checksum",
                known(record.input_audio_checksum.as_deref()),
            ),
        ],
    };
    if let Some(error) = &project.processing.last_error {
        processing.push(row("detail_last_error", error.clone()));
    }

    let statistics = document_result_statistics(project);
    let gaps = if statistics.provenance_complete() {
        text("detail_none").to_string()
    } else {
        statistics
            .provenance_gaps
            .iter()
            .map(|gap| provenance_gap(locale, *gap))
            .collect::<Vec<_>>()
            .join(match locale {
                Locale::ZhCn => "、",
                Locale::EnUs => ", ",
            })
    };

    vec![
        DetailSection {
            title: text("detail_identity"),
            rows: vec![
                row("detail_title", project.title().to_string()),
                row("detail_id", project.id().to_string()),
            ],
        },
        DetailSection {
            title: text("detail_source"),
            rows: source,
        },
        DetailSection {
            title: text("detail_hierarchy"),
            rows: vec![
                row("detail_parent", parent),
                row("detail_depth", project.hierarchy.depth.to_string()),
                row("detail_children", children.to_string()),
            ],
        },
        DetailSection {
            title: text("detail_state"),
            rows: vec![
                row(
                    "detail_document_status",
                    project.document_status.value.clone(),
                ),
                row("detail_revision", project.revision().to_string()),
            ],
        },
        DetailSection {
            title: text("detail_processing"),
            rows: processing,
        },
        DetailSection {
            title: text("detail_statistics"),
            rows: vec![
                row(
                    "detail_transcript_segments",
                    statistics.transcript_segments.to_string(),
                ),
                row(
                    "detail_diarization_segments",
                    statistics.diarization_segments.to_string(),
                ),
                row("detail_speakers", statistics.speakers.to_string()),
                row(
                    "detail_speech_duration",
                    timestamp(statistics.speech_duration_ms),
                ),
                row(
                    "detail_characters",
                    statistics.transcript_characters.to_string(),
                ),
                row("detail_words", statistics.transcript_words.to_string()),
                row("detail_provenance_gaps", gaps),
            ],
        },
    ]
}

/// The name of a provenance fact `processing.latest` lacks.
pub(crate) fn provenance_gap(locale: Locale, gap: ProvenanceGap) -> &'static str {
    tr(locale, provenance_gap_key(gap))
}

fn provenance_gap_key(gap: ProvenanceGap) -> &'static str {
    match gap {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{
        AlgorithmProvenance, DiarizationProvenance, DiarizationSegment, ModelProvenance,
        ProcessingRecord, ProjectId, RuntimeProvenance, TranscriptSegment,
    };

    fn value<'a>(sections: &'a [DetailSection], key: &str) -> Option<&'a str> {
        sections
            .iter()
            .flat_map(|section| &section.rows)
            .find(|row| row.key == key)
            .map(|row| row.value.as_str())
    }

    fn processed() -> DocumentState {
        let mut state = DocumentState::new(ProjectId::new("weekly"), "产品周会", "weekly.wav");
        state.document_status.value = "review".into();
        state.schema.revision = 7;
        state.results.transcript = vec![
            TranscriptSegment {
                id: "t-1".into(),
                start_ms: 0,
                end_ms: 42_000,
                text: "先过一下页面".into(),
            },
            TranscriptSegment {
                id: "t-2".into(),
                start_ms: 42_000,
                end_ms: 95_000,
                text: "baseline and tests".into(),
            },
        ];
        state.results.diarization = vec![
            DiarizationSegment {
                id: "d-1".into(),
                speaker_id: "speaker-0".into(),
                speaker_profile_id: None,
                speaker_label: None,
                start_ms: 0,
                end_ms: 42_000,
                confidence: None,
            },
            DiarizationSegment {
                id: "d-2".into(),
                speaker_id: "speaker-1".into(),
                speaker_profile_id: None,
                speaker_label: None,
                start_ms: 42_000,
                end_ms: 95_000,
                confidence: None,
            },
        ];
        let model = |backend: &str, model: &str| ModelProvenance {
            backend_id: backend.into(),
            model_id: model.into(),
            revision: Some("r1".into()),
            artifact_sha256: Some("ab".repeat(32)),
            artifact_path: format!("models/{model}"),
            artifacts: Vec::new(),
        };
        state.record_processing(ProcessingRecord {
            processing_revision: 1,
            completed_at_ms: 1_788_000_000_000,
            input_audio_checksum: Some("cd".repeat(32)),
            asr: model("whisper-rs", "large-v3-turbo"),
            diarization: DiarizationProvenance {
                backend_id: "sherpa-onnx".into(),
                model_revision: Some("r2".into()),
                segmentation: Some(model("sherpa-onnx", "segmentation")),
                embedding: Some(model("sherpa-onnx", "embedding")),
                clustering: AlgorithmProvenance {
                    id: "fast-clustering".into(),
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
                device: "Metal".into(),
                language: Some("zh".into()),
                sample_rate_hz: Some(16_000),
                threads: None,
                expected_speakers: None,
                windowing: None,
            },
        });
        state
    }

    #[test]
    fn a_processed_document_shows_state_provenance_and_statistics() {
        let parent = processed();
        let child = parent.semantic_child(
            ProjectId::new("decisions"),
            "决策",
            42_000,
            95_000,
            Vec::new(),
        );
        let projects = [parent.clone(), child.clone()];
        let sections = detail_sections(&parent, &projects, Locale::ZhCn, None);
        for (key, expected) in [
            ("detail_title", "产品周会"),
            ("detail_id", "weekly"),
            ("detail_audio_path", "weekly.wav"),
            ("detail_parent", "无（根文档）"),
            ("detail_depth", "0"),
            ("detail_children", "1"),
            ("detail_document_status", "review"),
            ("detail_revision", "7"),
            ("detail_asr", "whisper-rs · large-v3-turbo · r1"),
            ("detail_diarization", "sherpa-onnx · r2"),
            ("detail_device", "Metal"),
            ("detail_language", "zh"),
            ("detail_input_checksum", &"cd".repeat(32)),
            ("detail_transcript_segments", "2"),
            ("detail_diarization_segments", "2"),
            ("detail_speakers", "2"),
            ("detail_speech_duration", "00:01:35.000"),
            ("detail_characters", "22"),
            ("detail_provenance_gaps", "无"),
        ] {
            assert_eq!(value(&sections, key), Some(expected), "{key}");
        }
        assert_eq!(value(&sections, "detail_processing_record"), None);
        assert_eq!(value(&sections, "detail_last_error"), None);

        let child_sections = detail_sections(&child, &projects, Locale::EnUs, None);
        assert_eq!(value(&child_sections, "detail_parent"), Some("产品周会"));
        assert_eq!(value(&child_sections, "detail_depth"), Some("1"));
        assert_eq!(
            value(&child_sections, "detail_audio_range"),
            Some("00:00:42.000 – 00:01:35.000")
        );
    }

    #[test]
    fn an_unprocessed_document_says_so_and_shows_its_last_error() {
        let mut state = DocumentState::new(ProjectId::new("new"), "New", "new.wav");
        let sections = detail_sections(&state, &[state.clone()], Locale::EnUs, None);
        assert_eq!(
            value(&sections, "detail_processing_record"),
            Some("Not processed")
        );
        assert_eq!(value(&sections, "detail_asr"), None);
        assert_eq!(value(&sections, "detail_revision"), Some("0"));
        assert_eq!(value(&sections, "detail_document_status"), Some("inbox"));
        assert_eq!(
            value(&sections, "detail_provenance_gaps"),
            Some("No processing record")
        );

        state.set_processing_error("model missing");
        let sections = detail_sections(&state, &[state.clone()], Locale::EnUs, None);
        assert_eq!(value(&sections, "detail_last_error"), Some("model missing"));
    }

    #[test]
    fn missing_provenance_facts_are_named() {
        let mut state = processed();
        if let Some(record) = state.processing.latest.as_mut() {
            record.input_audio_checksum = None;
            record.asr.revision = None;
        }
        let sections = detail_sections(&state, &[state.clone()], Locale::ZhCn, None);
        assert_eq!(value(&sections, "detail_input_checksum"), Some("—"));
        assert_eq!(
            value(&sections, "detail_asr"),
            Some("whisper-rs · large-v3-turbo")
        );
        assert_eq!(
            value(&sections, "detail_provenance_gaps"),
            Some("输入音频校验和、ASR 模型版本")
        );
    }

    fn audio_row(
        state: &DocumentState,
        audio: Option<&Result<AudioStatus, AudioSourceError>>,
    ) -> Option<String> {
        let sections = detail_sections(state, &[state.clone()], Locale::ZhCn, audio);
        value(&sections, "detail_audio_status").map(str::to_string)
    }

    #[test]
    fn the_audio_row_reads_the_latest_check() {
        let state = processed();
        assert_eq!(
            audio_row(&state, None).as_deref(),
            Some(tr(Locale::ZhCn, "audio_checking"))
        );
        let missing = AudioStatus::Missing {
            path: "weekly.wav".into(),
        };
        assert_eq!(
            audio_row(&state, Some(&Ok(missing.clone()))),
            Some(crate::i18n::audio_status(Locale::ZhCn, &missing))
        );
        let failed = AudioSourceError::Service("offline".into());
        assert_eq!(
            audio_row(&state, Some(&Err(failed.clone()))),
            Some(crate::i18n::audio_source_error(Locale::ZhCn, &failed))
        );
    }

    /// Every label and section title is translated: the keys pass through
    /// variables, which the source scan in `i18n.rs` cannot follow.
    #[test]
    fn every_detail_label_is_translated_in_both_locales() {
        let mut failed = processed();
        failed.set_processing_error("model missing");
        let child = failed.semantic_child(ProjectId::new("child"), "Child", 1, 2, Vec::new());
        let unprocessed = DocumentState::new(ProjectId::new("new"), "New", "new.wav");
        let projects = [failed.clone(), child.clone(), unprocessed.clone()];
        for locale in [Locale::ZhCn, Locale::EnUs] {
            let mut keys = std::collections::BTreeSet::new();
            for project in &projects {
                for section in detail_sections(project, &projects, locale, None) {
                    assert!(!section.title.is_empty(), "{locale:?} section title");
                    for row in section.rows {
                        assert!(!row.label.is_empty(), "{} ({locale:?})", row.key);
                        keys.insert(row.key);
                    }
                }
            }
            assert!(keys.len() >= 20, "{locale:?}: {keys:?}");
        }
        for gap in [
            domain::ProvenanceGap::NoProcessingRecord,
            domain::ProvenanceGap::CompletionTime,
            domain::ProvenanceGap::InputAudioChecksum,
            domain::ProvenanceGap::AsrModel,
            domain::ProvenanceGap::AsrModelRevision,
            domain::ProvenanceGap::AsrArtifactChecksum,
            domain::ProvenanceGap::DiarizationBackend,
            domain::ProvenanceGap::DiarizationModelRevision,
            domain::ProvenanceGap::Clustering,
            domain::ProvenanceGap::Alignment,
            domain::ProvenanceGap::RuntimeDevice,
        ] {
            for locale in [Locale::ZhCn, Locale::EnUs] {
                assert!(!provenance_gap(locale, gap).is_empty(), "{gap:?}");
            }
        }
    }
}
