use crate::{
    components::empty_card,
    controller::{DownloadAction, RootView},
    controls::{Button, Collapsible, ProgressBar, Scrollbar},
    i18n::{self, text as tr, Locale},
    theme::{self, Tokens},
};
use gpui::{div, prelude::*, px, Context, FontWeight, IntoElement, Pixels};
use models::{
    manager::{DownloadProgress, DownloadState},
    ArtifactChecksum, ModelAvailability, ModelCatalogEntry, ModelInstallState,
};
use std::collections::BTreeMap;

/// The install status chip column; fixed, so the chips of every model row
/// line up whatever the model id's length.
const STATUS_COLUMN_WIDTH: Pixels = px(96.);
/// The action column after it, empty when a model offers no action; every
/// action button is this wide, so each takes the same place on its card.
const ACTION_COLUMN_WIDTH: Pixels = px(88.);
/// The widest a license gets before its tail fades.
const LICENSE_WIDTH: Pixels = px(200.);
/// The file or backend column of a card's details.
const DETAIL_LABEL_WIDTH: Pixels = px(240.);
/// Between the facts of a card's fact line.
const FACT_SEPARATOR: &str = " · ";

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
                    Collapsible::new(format!("model-family-{family}"), family.clone())
                        .debug_selector(|| format!("model-family-{family}"))
                        .open(expanded)
                        .p_4()
                        .flex()
                        .items_center()
                        .justify_between()
                        .on_toggle(cx.listener(move |this, _, _, cx| {
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
                        .child(div().text_sm().text_color(tokens.text_muted).child(format!(
                            "{} {}",
                            model_count,
                            tr(locale, "models_count")
                        )))
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
                .child(Scrollbar::vertical("models-scrollbar", &scroll)),
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

/// A catalog model's card: its name with the install status and its one
/// action; one quiet line of task, size, VRAM, backends and license; the
/// source, checksum and details toggle; then what is going on with it — a
/// download, a re-probe, the last failure, an unreadable record, the last
/// probe or why it cannot be downloaded — and the details when expanded.
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

    let action_button = {
        let id = model_id.clone();
        match action {
            CardAction::Download => Some(
                Button::new(format!("download-{id}"), tr(locale, "download"))
                    .debug_selector(marked("download"))
                    .primary()
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.download_model(id.clone(), cx)),
                    ),
            ),
            CardAction::Retry => Some(
                Button::new(format!("retry-{id}"), tr(locale, "retry_download"))
                    .debug_selector(marked("retry"))
                    .primary()
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.download_model(id.clone(), cx)),
                    ),
            ),
            CardAction::Reprobe => Some(
                Button::new(format!("reprobe-{id}"), tr(locale, "reprobe"))
                    .debug_selector(marked("reprobe"))
                    .secondary()
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.reprobe_model(id.clone(), cx)),
                    ),
            ),
            CardAction::None => None,
        }
        .map(|button| button.compact().w(ACTION_COLUMN_WIDTH))
    };
    let header = div()
        .debug_selector(marked("header"))
        .min_h(tokens.control_height.md)
        .flex()
        .items_center()
        .gap(tokens.spacing.md)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_base()
                .font_weight(FontWeight::SEMIBOLD)
                .child(theme::tail_faded_label(
                    format!("model-name-{model_id}"),
                    model_id.clone(),
                    tokens.raised,
                )),
        )
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
                .children(action_button),
        );

    let summary = div()
        .debug_selector(marked("facts"))
        .mt(tokens.spacing.xs)
        .flex()
        .items_center()
        .text_sm()
        .text_color(tokens.text_muted)
        .child(div().min_w_0().child(facts.summary.clone()))
        .child(div().flex_none().whitespace_nowrap().child(FACT_SEPARATOR))
        .child(
            div()
                .debug_selector(marked("license"))
                .max_w(LICENSE_WIDTH)
                .min_w_0()
                .flex()
                .child(div().min_w_0().child(theme::tail_faded_label(
                    format!("model-license-label-{model_id}"),
                    facts.license.clone(),
                    tokens.raised,
                ))),
        );

    let details_label = tr(
        locale,
        if expanded {
            "hide_model_details"
        } else {
            "show_model_details"
        },
    );
    let details_label = format!("{details_label} {}", if expanded { "▾" } else { "▸" });
    let details_id = model_id.clone();
    let provenance = div()
        .flex()
        .items_center()
        .gap(tokens.spacing.lg)
        .text_sm()
        .text_color(tokens.text_muted)
        .child(
            div()
                .debug_selector(marked("source"))
                .flex_1()
                .min_w_0()
                .child(theme::tail_faded_label(
                    format!("model-source-label-{model_id}"),
                    facts.source.clone(),
                    tokens.raised,
                )),
        )
        .child(
            div()
                .debug_selector(marked("checksum"))
                .flex_shrink_0()
                .child(format!(
                    "{} {}",
                    tr(locale, "model_checksum"),
                    facts.checksum
                )),
        )
        .child(
            Button::new(format!("model-details-{model_id}"), details_label)
                .debug_selector(marked("details-toggle"))
                .ghost()
                .compact()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_model_details(details_id.clone(), cx)
                })),
        );

    // A running download or re-probe supersedes the last failure.
    let busy = progress.is_some() || reprobing;
    let failure = facts.failure.clone().filter(|_| !busy).map(|failure| {
        notice(&tokens)
            .debug_selector(marked("failure"))
            .child(div().flex_1().min_w_0().child(failure))
    });
    let record_error = facts.record_error.clone().map(|error| {
        notice(&tokens)
            .debug_selector(marked("record-error"))
            .child(div().flex_1().min_w_0().child(error))
    });

    let reprobe_progress = reprobing.then(|| {
        progress_block(
            ProgressBar::new(
                format!("reprobe-progress-{model_id}"),
                tr(locale, "model_reprobe_progress"),
            )
            .indeterminate(true),
            tr(locale, "reprobing").into(),
            &tokens,
        )
        .debug_selector(marked("reprobe-progress"))
    });
    // The failure stands for the probe that failed with it.
    let probe = (!busy && failure.is_none() && (downloaded || facts.probe.is_some())).then(|| {
        let (summary, passed) = match (&facts.probe, entry.probe.as_ref()) {
            (Some(probe), Some(record)) => (probe.summary.clone(), record.passed()),
            _ => (tr(locale, "no_probe_record").to_string(), true),
        };
        div()
            .debug_selector(marked("probe"))
            .text_sm()
            .text_color(if passed {
                tokens.text_muted
            } else {
                tokens.warning_text
            })
            .child(summary)
    });
    let unavailable = facts.unavailable.clone().map(|reason| {
        div()
            .debug_selector(marked("unavailable"))
            .text_sm()
            .text_color(tokens.text_muted)
            .child(reason)
    });

    let details = expanded.then(|| {
        let mut details = div().flex().flex_col().gap(tokens.spacing.sm);
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

    // What is going on with the model, under its identity.
    let status = [
        progress_view,
        reprobe_progress.map(IntoElement::into_any_element),
        failure.map(IntoElement::into_any_element),
        record_error.map(IntoElement::into_any_element),
        probe.map(IntoElement::into_any_element),
        unavailable.map(IntoElement::into_any_element),
        details.map(IntoElement::into_any_element),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();

    div()
        .debug_selector(marked("card"))
        .p(tokens.spacing.lg)
        .rounded(tokens.radius.md)
        .bg(tokens.raised)
        .flex()
        .flex_col()
        .child(header)
        .child(summary)
        .child(provenance)
        .when(!status.is_empty(), |card| {
            card.child(
                div()
                    .mt(tokens.spacing.md)
                    .flex()
                    .flex_col()
                    .gap(tokens.spacing.md)
                    .children(status),
            )
        })
        .into_any_element()
}

/// A progress bar over the line that says what it measures.
fn progress_block(bar: ProgressBar, label: String, tokens: &Tokens) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(tokens.spacing.xs)
        .child(bar)
        .child(div().text_xs().text_color(tokens.text_muted).child(label))
}

/// A warning on a card: a failure or an unreadable record.
fn notice(tokens: &Tokens) -> gpui::Div {
    div()
        .px(tokens.spacing.md)
        .py(tokens.spacing.sm)
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
            Button::new(format!("download-action-{}", progress.id.0), label)
                .secondary()
                .compact()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.control_download(progress.id, action, cx)
                })),
        )
        .into_any_element()
}

/// A size in binary units: `78 MiB`, `1.0 GiB`.
pub(crate) fn format_size(bytes: u64) -> String {
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

/// What a model card shows beyond its id and install status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelFacts {
    /// Task, size, VRAM and backends, each only when the catalog names it.
    pub(crate) summary: String,
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
    /// Why a model that is not installed cannot be downloaded.
    pub(crate) unavailable: Option<String>,
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
        summary: fact_summary(model, locale),
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
        unavailable: (model.status != READY_STATUS
            && entry.install_state != ModelInstallState::Installed)
            .then(|| tr(locale, unavailable_key(&model.status)).into()),
    }
}

/// A card's fact line: task, size, VRAM and backends, without labels; what
/// the catalog leaves out is left out.
fn fact_summary(model: &ModelCatalogEntry, locale: Locale) -> String {
    let task =
        task_key(&model.task).map_or_else(|| model.task.clone(), |key| tr(locale, key).into());
    let mut facts = vec![task];
    facts.extend(model.size_bytes.map(format_size));
    facts.extend(
        model
            .estimated_vram_bytes
            .map(|bytes| i18n::model_vram(locale, &format_size(bytes))),
    );
    if !model.candidate_backends.is_empty() {
        facts.push(model.candidate_backends.join(", "));
    }
    facts.join(FACT_SEPARATOR)
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

/// Why the catalog does not offer a model that is not `ready`.
fn unavailable_key(status: &str) -> &'static str {
    match status {
        CANDIDATE_SOURCE_STATUS => "catalog_candidate_source",
        NATIVE_ADAPTER_STATUS => "catalog_requires_native_adapter",
        _ => "catalog_unavailable",
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
