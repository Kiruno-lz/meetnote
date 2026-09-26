use crate::{
    components::empty_card,
    controller::{DownloadAction, RootView},
    controls::ProgressBar,
    i18n::{self, text as tr, Locale},
    theme::{self, Tokens},
};
use gpui::{div, prelude::*, px, Context, IntoElement, Pixels};
use gpui_component::{
    button::{Button, ButtonVariants},
    collapsible::Collapsible,
    scroll::ScrollableElement as _,
};
use models::{
    manager::{DownloadProgress, DownloadState},
    ArtifactChecksum, ModelAvailability, ModelCatalogEntry, ModelInstallState,
};
use std::collections::BTreeMap;

/// The install status chip column; fixed, so the chips of every model row
/// line up whatever the model id's length.
const STATUS_COLUMN_WIDTH: Pixels = px(96.);
/// The action column after it, empty when a model offers no download.
const ACTION_COLUMN_WIDTH: Pixels = px(88.);
/// The widest a license gets before its tail fades; the source takes the
/// rest of the row.
const LICENSE_WIDTH: Pixels = px(200.);
/// The file or backend column of a card's details.
const DETAIL_LABEL_WIDTH: Pixels = px(240.);
/// The re-probe progress bar beside the probe summary.
const REPROBE_PROGRESS_WIDTH: Pixels = px(120.);

/// The catalog's task names and the status of a model it can install.
const ASR_TASK: &str = "asr";
const DIARIZATION_TASK: &str = "diarization";
const READY_STATUS: &str = "ready";
const CANDIDATE_SOURCE_STATUS: &str = "candidate-source";
const NATIVE_ADAPTER_STATUS: &str = "requires-native-adapter";

pub(crate) fn render(root: &mut RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let tokens = *theme::tokens(cx);
    let entries = root.installation.catalog.clone();
    let downloads = root.services.model_manager.list().unwrap_or_default();
    let mut groups: BTreeMap<String, Vec<ModelAvailability>> = BTreeMap::new();
    for entry in entries {
        groups
            .entry(entry.model.model_family.clone())
            .or_default()
            .push(entry);
    }

    let scroll = root.scroll.models.clone();
    let mut content = div()
        .id("models-scroll")
        .debug_selector(|| "models-scroll".into())
        .size_full()
        .overflow_y_scroll()
        .track_scroll(&scroll)
        .flex()
        .flex_col()
        .gap_4();
    if groups.is_empty() {
        content = content.child(empty_card(tr(root.ui.locale, "no_models"), &tokens));
    } else {
        for (family, entries) in groups {
            let expanded = root.ui.expanded_model_families.contains(&family);
            let family_for_click = family.clone();
            let model_count = entries.len();
            let locale = root.ui.locale;
            let downloads = &downloads;
            let cards = entries
                .into_iter()
                .map(|entry| {
                    let progress = downloads
                        .iter()
                        .find(|progress| progress.model_id == entry.model.id)
                        .cloned();
                    model_card(root, entry, progress, cx)
                })
                .collect::<Vec<_>>();
            let group = div()
                .flex_shrink_0()
                .rounded(tokens.radius.md)
                .bg(tokens.surface)
                .child(
                    Collapsible::new()
                        .open(expanded)
                        .child(
                            div()
                                .id(format!("model-family-{family}"))
                                .debug_selector(|| format!("model-family-{family}"))
                                .cursor_pointer()
                                .p_4()
                                .flex()
                                .items_center()
                                .justify_between()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.toggle_model_family(family_for_click.clone(), cx)
                                }))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(if expanded { "▾" } else { "▸" })
                                        .child(div().text_lg().child(family.clone())),
                                )
                                .child(div().text_sm().text_color(tokens.text_muted).child(
                                    format!("{} {}", model_count, tr(locale, "models_count")),
                                )),
                        )
                        .content(
                            div()
                                .px_4()
                                .pb_4()
                                .flex()
                                .flex_col()
                                .gap_3()
                                .children(cards),
                        ),
                );
            content = content.child(group);
        }
    }

    div()
        .debug_selector(|| "models-page".into())
        .flex_1()
        .min_w_0()
        .h_full()
        .min_h_0()
        .p_6()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex_shrink_0()
                .text_2xl()
                .child(tr(root.ui.locale, "models")),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_sm()
                .text_color(tokens.text_muted)
                .child(tr(root.ui.locale, "catalog_description")),
        )
        // The scrollbar stays outside the fade.
        .child(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .child(theme::scroll_faded(&scroll, tokens.content, content))
                .vertical_scrollbar(&scroll),
        )
        .child(
            div()
                .flex_shrink_0()
                .pt_2()
                .text_xs()
                .text_color(tokens.text_muted)
                .child(root.ui.notice.clone()),
        )
}

/// A catalog model's card: identity, install status and its one action;
/// task, catalog status, size, VRAM and backends; license, source and
/// checksum; the last failure and an unreadable record; the last probe; the
/// details when expanded; a running download.
fn model_card(
    root: &mut RootView,
    entry: ModelAvailability,
    progress: Option<DownloadProgress>,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    let tokens = *theme::tokens(cx);
    let locale = root.ui.locale;
    let model_id = entry.model.id.clone();
    let progress = progress.filter(|progress| {
        matches!(
            progress.state,
            DownloadState::Queued
                | DownloadState::Downloading
                | DownloadState::Paused
                | DownloadState::Verifying
        )
    });
    let reprobing = root.model_reprobes.contains_key(&model_id);
    let expanded = root.ui.expanded_model_details.contains(&model_id);
    let action = card_action(&entry, progress.is_some(), reprobing);
    let facts = model_facts(&entry, locale);
    let downloaded = matches!(entry.install_state, ModelInstallState::Installed);
    let marked = |name: &str| {
        let selector = format!("model-{name}-{model_id}");
        move || selector.clone()
    };

    let download = (action == CardAction::Download).then(|| {
        let id = model_id.clone();
        Button::new(format!("download-{id}"))
            .debug_selector(marked("download"))
            .primary()
            .compact()
            .label(tr(locale, "download"))
            .on_click(cx.listener(move |this, _, _, cx| this.download_model(id.clone(), cx)))
    });
    let header = div()
        .flex()
        .items_center()
        .gap_3()
        .child(div().flex_1().min_w_0().text_base().child(model_id.clone()))
        .child(
            div()
                .debug_selector(marked("status"))
                .w(STATUS_COLUMN_WIDTH)
                .flex_shrink_0()
                .flex()
                .justify_center()
                .py_0p5()
                .rounded(tokens.radius.sm)
                .bg(if downloaded {
                    tokens.status.ready
                } else {
                    tokens.status.imported
                })
                .text_xs()
                .child(if downloaded {
                    tr(locale, "downloaded")
                } else {
                    tr(locale, "not_downloaded")
                }),
        )
        .child(
            div()
                .w(ACTION_COLUMN_WIDTH)
                .flex_shrink_0()
                .flex()
                .justify_end()
                .children(download),
        );

    let backends = if entry.model.candidate_backends.is_empty() {
        "—".into()
    } else {
        entry.model.candidate_backends.join(", ")
    };
    let labelled = |label: &str, value: String| format!("{label}: {value}");
    let summary = div()
        .mt_2()
        .flex()
        .flex_wrap()
        .gap_x_6()
        .gap_y_1()
        .text_sm()
        .text_color(tokens.text_muted)
        .child(
            div()
                .debug_selector(marked("task"))
                .child(labelled(tr(locale, "model_task"), facts.task.clone())),
        )
        .child(labelled(
            tr(locale, "catalog_status"),
            facts.catalog_status.clone(),
        ))
        .child(labelled(
            tr(locale, "model_size"),
            format_bytes(entry.model.size_bytes),
        ))
        .child(labelled(
            tr(locale, "vram_estimate"),
            format_bytes(entry.model.estimated_vram_bytes),
        ))
        .child(labelled(tr(locale, "applicable_backends"), backends));

    let details_label = tr(
        locale,
        if expanded {
            "hide_model_details"
        } else {
            "show_model_details"
        },
    );
    let details_id = model_id.clone();
    let provenance = div()
        .mt_1()
        .flex()
        .items_center()
        .gap_6()
        .text_sm()
        .text_color(tokens.text_muted)
        .child(
            div()
                .debug_selector(marked("license"))
                .max_w(LICENSE_WIDTH)
                .min_w_0()
                .flex()
                .gap_1()
                .child(
                    div()
                        .flex_shrink_0()
                        .child(format!("{}:", tr(locale, "model_license"))),
                )
                .child(div().min_w_0().child(theme::tail_faded_label(
                    format!("model-license-label-{model_id}"),
                    facts.license.clone(),
                    tokens.raised,
                ))),
        )
        .child(
            div()
                .debug_selector(marked("source"))
                .flex_1()
                .min_w_0()
                .flex()
                .gap_1()
                .child(
                    div()
                        .flex_shrink_0()
                        .child(format!("{}:", tr(locale, "model_source"))),
                )
                .child(div().flex_1().min_w_0().child(theme::tail_faded_label(
                    format!("model-source-label-{model_id}"),
                    facts.source.clone(),
                    tokens.raised,
                ))),
        )
        .child(
            div()
                .debug_selector(marked("checksum"))
                .flex_shrink_0()
                .child(labelled(
                    tr(locale, "model_checksum"),
                    facts.checksum.clone(),
                )),
        )
        .child(
            Button::new(format!("model-details-{model_id}"))
                .debug_selector(marked("details-toggle"))
                .ghost()
                .compact()
                .label(format!(
                    "{details_label} {}",
                    if expanded { "▾" } else { "▸" }
                ))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_model_details(details_id.clone(), cx)
                })),
        );

    let failure = facts.failure.clone().map(|failure| {
        let retry = (action == CardAction::Retry).then(|| {
            let id = model_id.clone();
            Button::new(format!("retry-{id}"))
                .debug_selector(marked("retry"))
                .primary()
                .compact()
                .label(tr(locale, "retry_download"))
                .on_click(cx.listener(move |this, _, _, cx| this.download_model(id.clone(), cx)))
        });
        notice(&tokens)
            .debug_selector(marked("failure"))
            .child(div().flex_1().min_w_0().child(failure))
            .children(retry)
    });
    let record_error = facts.record_error.clone().map(|error| {
        notice(&tokens)
            .debug_selector(marked("record-error"))
            .child(div().flex_1().min_w_0().child(error))
    });

    let probe = (downloaded || reprobing || facts.probe.is_some()).then(|| {
        let (summary, passed) = match (&facts.probe, entry.probe.as_ref()) {
            (Some(probe), Some(record)) => (probe.summary.clone(), record.passed()),
            _ => (tr(locale, "no_probe_record").to_string(), true),
        };
        let progress = reprobing.then(|| {
            div()
                .debug_selector(marked("reprobe-progress"))
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div().w(REPROBE_PROGRESS_WIDTH).child(
                        ProgressBar::new(
                            format!("reprobe-progress-{model_id}"),
                            tr(locale, "model_reprobe_progress"),
                        )
                        .indeterminate(true),
                    ),
                )
                .child(tr(locale, "reprobing"))
        });
        let reprobe = (action == CardAction::Reprobe).then(|| {
            let id = model_id.clone();
            Button::new(format!("reprobe-{id}"))
                .debug_selector(marked("reprobe"))
                .secondary()
                .compact()
                .label(tr(locale, "reprobe"))
                .on_click(cx.listener(move |this, _, _, cx| this.reprobe_model(id.clone(), cx)))
        });
        div()
            .debug_selector(marked("probe"))
            .mt_3()
            .pt_3()
            .border_t_1()
            .border_color(tokens.hairline)
            .flex()
            .items_center()
            .gap_3()
            .text_sm()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(if passed {
                        tokens.text_muted
                    } else {
                        tokens.warning_text
                    })
                    .child(summary),
            )
            .children(progress)
            .children(reprobe)
    });

    let details = expanded.then(|| {
        let mut details = div().mt_2().flex().flex_col().gap_2();
        let artifacts = match &facts.probe {
            Some(probe) => {
                details = details.child(fact_rows(
                    tr(locale, "probe_backends"),
                    &probe.backends,
                    &format!("model-backend-{model_id}"),
                    &tokens,
                ));
                (!probe.artifacts.is_empty())
                    .then(|| (tr(locale, "probe_artifacts"), &probe.artifacts, "artifact"))
            }
            None => None,
        };
        let (title, rows, name) = artifacts.unwrap_or((
            tr(locale, "declared_checksums"),
            &facts.declared,
            "declared",
        ));
        details.child(fact_rows(
            title,
            rows,
            &format!("model-{name}-{model_id}"),
            &tokens,
        ))
    });

    let progress_view =
        progress.map(|progress| download_progress_view(root, &model_id, progress, cx));

    div()
        .debug_selector(marked("card"))
        .p_4()
        .rounded(tokens.radius.sm)
        .bg(tokens.raised)
        .child(header)
        .child(summary)
        .child(provenance)
        .children(failure)
        .children(record_error)
        .children(probe)
        .children(details)
        .children(progress_view)
        .into_any_element()
}

/// A warning on a card: a failure or an unreadable record.
fn notice(tokens: &Tokens) -> gpui::Div {
    div()
        .mt_3()
        .px_3()
        .py_2()
        .rounded(tokens.radius.sm)
        .bg(tokens.warning_surface)
        .text_color(tokens.warning_text)
        .text_sm()
        .flex()
        .items_center()
        .gap_3()
}

/// A titled list of a card's details, each row named `<prefix>-<index>`.
fn fact_rows(title: &'static str, rows: &[FactRow], prefix: &str, tokens: &Tokens) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .text_xs()
        .child(div().text_color(tokens.accent_text).child(title))
        .children(rows.iter().enumerate().map(|(index, row)| {
            let selector = format!("{prefix}-{index}");
            div()
                .debug_selector({
                    let selector = selector.clone();
                    move || selector.clone()
                })
                .flex()
                .gap_3()
                .child(
                    div()
                        .w(DETAIL_LABEL_WIDTH)
                        .flex_shrink_0()
                        .text_color(tokens.text_muted)
                        .child(theme::tail_faded_label(
                            format!("{selector}-label"),
                            row.label.clone(),
                            tokens.raised,
                        )),
                )
                .child(div().flex_1().min_w_0().child(row.value.clone()))
        }))
}

fn download_progress_view(
    root: &mut RootView,
    model_id: &str,
    progress: DownloadProgress,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    let tokens = *theme::tokens(cx);
    let percent = progress
        .total_bytes
        .filter(|total| *total > 0)
        .map(|total| progress.completed_bytes as f32 / total as f32 * 100.)
        .unwrap_or(0.);
    let loading = progress.total_bytes.is_none();
    let action = match progress.state {
        DownloadState::Paused => DownloadAction::Resume,
        DownloadState::Queued | DownloadState::Downloading | DownloadState::Verifying => {
            DownloadAction::Pause
        }
        _ => DownloadAction::Cancel,
    };
    let label = match action {
        DownloadAction::Pause => tr(root.ui.locale, "pause"),
        DownloadAction::Resume => tr(root.ui.locale, "resume"),
        DownloadAction::Cancel => tr(root.ui.locale, "cancel"),
    };
    div()
        .mt_3()
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    ProgressBar::new(
                        format!("download-progress-{model_id}"),
                        tr(root.ui.locale, "model_download_progress"),
                    )
                    .value(percent)
                    .indeterminate(loading),
                )
                .child(div().text_xs().text_color(tokens.text_muted).child(
                    i18n::download_progress(
                        root.ui.locale,
                        model_id,
                        progress.artifact_index,
                        progress.artifact_count,
                        progress.completed_bytes,
                        progress.speed_bytes_per_second,
                    ),
                )),
        )
        .child(
            Button::new(format!("download-action-{}", progress.id.0))
                .secondary()
                .compact()
                .label(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.control_download(progress.id, action, cx)
                })),
        )
        .into_any_element()
}

fn format_bytes(bytes: Option<u64>) -> String {
    let Some(bytes) = bytes else {
        return "—".into();
    };
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let bytes = bytes as f64;
    if bytes >= GIB {
        format!("{:.1} GiB", bytes / GIB)
    } else if bytes >= MIB {
        format!("{:.0} MiB", bytes / MIB)
    } else {
        format!("{:.0} KiB", bytes / KIB)
    }
}

/// A labelled fact of a card's details: a file or backend and what its
/// check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FactRow {
    pub(crate) label: String,
    pub(crate) value: String,
}

/// A model's last probe as its card shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProbeFacts {
    /// Outcome, kind and time.
    pub(crate) summary: String,
    pub(crate) backends: Vec<FactRow>,
    /// Each artifact's checksum check with the full expected checksum.
    pub(crate) artifacts: Vec<FactRow>,
}

/// What a model card shows beyond its id, sizes and backends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelFacts {
    pub(crate) task: String,
    pub(crate) catalog_status: String,
    pub(crate) license: String,
    /// The full source; the card fades its tail.
    pub(crate) source: String,
    /// The declared checksums in short form.
    pub(crate) checksum: String,
    /// Each artifact's full declared checksum.
    pub(crate) declared: Vec<FactRow>,
    pub(crate) probe: Option<ProbeFacts>,
    /// The last failed download or re-probe and why.
    pub(crate) failure: Option<String>,
    /// Why the probe record cannot be read, and where it is.
    pub(crate) record_error: Option<String>,
}

/// A card's facts: the catalog entry, the last probe record and failure,
/// and a record that cannot be read. A pure projection of the published
/// installation state; nothing here reads files.
pub(crate) fn model_facts(entry: &ModelAvailability, locale: Locale) -> ModelFacts {
    let model = &entry.model;
    let declared = declared_checksums(model);
    let distinct = declared
        .iter()
        .fold(Vec::new(), |mut distinct, (_, checksum)| {
            if let Some(checksum) = checksum {
                if !distinct.contains(&checksum.as_str()) {
                    distinct.push(checksum.as_str());
                }
            }
            distinct
        });
    ModelFacts {
        task: task_key(&model.task)
            .map_or_else(|| model.task.clone(), |key| tr(locale, key).into()),
        catalog_status: catalog_status_key(&model.status)
            .map_or_else(|| model.status.clone(), |key| tr(locale, key).into()),
        license: model.license.clone(),
        source: model.source.clone(),
        checksum: i18n::declared_checksums(locale, &distinct),
        declared: declared
            .iter()
            .map(|(filename, checksum)| FactRow {
                label: filename.clone(),
                value: checksum
                    .clone()
                    .unwrap_or_else(|| tr(locale, "checksum_not_declared").into()),
            })
            .collect(),
        probe: entry.probe.as_ref().map(|record| ProbeFacts {
            summary: i18n::probe_summary(locale, record),
            backends: record
                .backends
                .iter()
                .map(|backend| FactRow {
                    label: backend.backend_id.clone(),
                    value: i18n::backend_check(locale, &backend.result),
                })
                .collect(),
            artifacts: record
                .checksums
                .iter()
                .map(|checksum| FactRow {
                    label: checksum.filename.clone(),
                    value: artifact_check(locale, checksum),
                })
                .collect(),
        }),
        failure: entry
            .failure
            .as_ref()
            .map(|failure| i18n::model_failure(locale, failure)),
        record_error: entry
            .record_error
            .as_ref()
            .map(|error| i18n::probe_record_error(locale, error)),
    }
}

/// Each artifact file and the SHA-256 the catalog declares for it, as the
/// manager checks them: a bundle's artifacts, or the one file of its source
/// with the model's checksum.
fn declared_checksums(model: &ModelCatalogEntry) -> Vec<(String, Option<String>)> {
    if model.artifacts.is_empty() {
        return model
            .artifact_filenames()
            .into_iter()
            .take(1)
            .map(|filename| (filename, model.sha256.clone()))
            .collect();
    }
    model
        .artifacts
        .iter()
        .map(|artifact| (artifact.filename.clone(), artifact.sha256.clone()))
        .collect()
}

/// An artifact's checksum check and the checksum it was checked against.
fn artifact_check(locale: Locale, checksum: &ArtifactChecksum) -> String {
    let result = i18n::checksum_result(locale, &checksum.result);
    match &checksum.expected_sha256 {
        Some(expected) => format!("{result} · {} {expected}", tr(locale, "expected_checksum")),
        None => result,
    }
}

fn task_key(task: &str) -> Option<&'static str> {
    match task {
        ASR_TASK => Some("task_asr"),
        DIARIZATION_TASK => Some("task_diarization"),
        _ => None,
    }
}

fn catalog_status_key(status: &str) -> Option<&'static str> {
    match status {
        READY_STATUS => Some("catalog_ready"),
        CANDIDATE_SOURCE_STATUS => Some("catalog_candidate_source"),
        NATIVE_ADAPTER_STATUS => Some("catalog_requires_native_adapter"),
        _ => None,
    }
}

/// The one action a card offers besides its details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CardAction {
    /// A ready model that is missing or partial.
    Download,
    /// The same download, after the last one failed.
    Retry,
    /// An installed model.
    Reprobe,
    None,
}

/// Only a ready model that is missing or partial downloads; an installed
/// one re-probes, which is also how a failed re-probe is retried. Nothing
/// is offered while the model downloads or re-probes.
pub(crate) fn card_action(
    entry: &ModelAvailability,
    downloading: bool,
    reprobing: bool,
) -> CardAction {
    if downloading || reprobing {
        return CardAction::None;
    }
    match entry.install_state {
        ModelInstallState::Installed => CardAction::Reprobe,
        ModelInstallState::Missing | ModelInstallState::Partial
            if entry.model.status == READY_STATUS =>
        {
            if entry.failure.is_some() {
                CardAction::Retry
            } else {
                CardAction::Download
            }
        }
        ModelInstallState::Missing | ModelInstallState::Partial => CardAction::None,
    }
}
