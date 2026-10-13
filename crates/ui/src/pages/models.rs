use crate::{
    components::{empty_card, utc_time},
    controller::{DownloadAction, RootView},
    controls::{Button, Collapsible, ProgressBar, Scrollbar},
    i18n::{self, text as tr, Locale},
    theme::{self, Tokens},
};
use gpui::{div, prelude::*, px, Context, FontWeight, IntoElement, Pixels};
use models::{
    manager::{DownloadId, DownloadProgress, DownloadState},
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
        .gap(tokens.spacing.lg);
    if groups.is_empty() {
        content = content.child(empty_card(tr(root.ui.locale, "no_models"), &tokens));
    } else {
        for (family, entries) in groups {
            let expanded = root.ui.expanded_model_families.contains(&family);
            let family_for_click = family.clone();
            let locale = root.ui.locale;
            let states = entries
                .iter()
                .map(|entry| entry.install_state)
                .collect::<Vec<_>>();
            let (summary, complete) = family_summary(locale, &states);
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
                .rounded(tokens.radius.lg)
                .bg(tokens.surface)
                .child(
                    Collapsible::new(format!("model-family-{family}"), family.clone())
                        .debug_selector(|| format!("model-family-{family}"))
                        .open(expanded)
                        .px(tokens.spacing.lg)
                        .py(tokens.spacing.md)
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(tokens.spacing.md)
                        .on_toggle(cx.listener(move |this, _, _, cx| {
                            this.toggle_model_family(family_for_click.clone(), cx)
                        }))
                        .child(
                            div()
                                .min_w_0()
                                .flex()
                                .items_center()
                                .gap(tokens.spacing.sm)
                                .child(div().text_color(tokens.text_muted).child(if expanded {
                                    "▾"
                                } else {
                                    "▸"
                                }))
                                .child(
                                    div()
                                        .text_lg()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(family.clone()),
                                ),
                        )
                        .child({
                            let selector = format!("model-family-summary-{family}");
                            div()
                                .debug_selector(move || selector.clone())
                                .flex_none()
                                .text_sm()
                                .text_color(if complete {
                                    tokens.success
                                } else {
                                    tokens.text_muted
                                })
                                .child(summary)
                        })
                        .content(
                            div()
                                .px(tokens.spacing.lg)
                                .pb(tokens.spacing.lg)
                                .flex()
                                .flex_col()
                                .gap(tokens.spacing.md)
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
        .p(tokens.spacing.xl)
        .flex()
        .flex_col()
        .gap(tokens.spacing.md)
        // The title and its readiness line read as one block; the list
        // starts a step further down.
        .child(
            div()
                .flex_shrink_0()
                .pb(tokens.spacing.xs)
                .flex()
                .flex_col()
                .gap(tokens.spacing.xs)
                .child(
                    div()
                        .debug_selector(|| "models-title".into())
                        .text_2xl()
                        .child(tr(root.ui.locale, "models")),
                )
                .child(
                    div()
                        .debug_selector(|| "models-readiness".into())
                        .text_sm()
                        .text_color(tokens.text_muted)
                        .child(readiness_text(&root.installation, root.ui.locale)),
                ),
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

/// Whether the default models can process audio, and if not which are
/// missing and where to get them.
pub(crate) fn readiness_text(
    installation: &application::InstallationState,
    locale: Locale,
) -> &'static str {
    tr(
        locale,
        readiness_key(
            installation.default_asr_ready,
            installation.default_diarization_ready,
        ),
    )
}

fn readiness_key(asr_ready: bool, diarization_ready: bool) -> &'static str {
    match (asr_ready, diarization_ready) {
        (true, true) => "models_ready",
        (false, true) => "models_asr_not_ready",
        (true, false) => "models_diarization_not_ready",
        (false, false) => "models_not_ready",
    }
}

/// How many of a family's models are downloaded, and whether all are.
pub(crate) fn family_summary(locale: Locale, states: &[ModelInstallState]) -> (String, bool) {
    let installed = states
        .iter()
        .filter(|state| **state == ModelInstallState::Installed)
        .count();
    (
        i18n::model_family_installed(locale, installed, states.len()),
        !states.is_empty() && installed == states.len(),
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
    let starting = root.model_download_starts.contains(&model_id);
    let reprobing = root.model_reprobes.contains_key(&model_id);
    let expanded = root.ui.expanded_model_details.contains(&model_id);
    let action = card_action(&entry, progress.as_ref(), reprobing);
    let facts = model_facts(&entry, locale);
    let downloaded = matches!(entry.install_state, ModelInstallState::Installed);
    let (chip_key, chip_tone) = if starting {
        ("download_preparing", ChipTone::Busy)
    } else {
        status_key(
            downloaded,
            progress.as_ref().map(|progress| &progress.state),
            reprobing,
        )
    };
    let marked = |name: &str| {
        let selector = format!("model-{name}-{model_id}");
        move || selector.clone()
    };

    let action_button = {
        let id = model_id.clone();
        match action {
            CardAction::Download => Some(
                Button::new(format!("download-{id}"), tr(locale, "download"))
                    .debug_selector(marked(if starting { "starting" } else { "download" }))
                    .disabled(starting)
                    .primary()
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.download_model(id.clone(), cx)),
                    ),
            ),
            CardAction::Retry => Some(
                Button::new(format!("retry-{id}"), tr(locale, "retry_download"))
                    .debug_selector(marked(if starting { "starting" } else { "retry" }))
                    .disabled(starting)
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
            // One id for both, so focus stays as pause turns into resume.
            CardAction::Pause(download) => Some(
                Button::new(
                    format!("download-action-{}", download.0),
                    tr(locale, "pause"),
                )
                .debug_selector(marked("pause"))
                .secondary()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.control_download(download, DownloadAction::Pause, cx)
                })),
            ),
            CardAction::Resume(download) => Some(
                Button::new(
                    format!("download-action-{}", download.0),
                    tr(locale, "resume"),
                )
                .debug_selector(marked("resume"))
                .primary()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.control_download(download, DownloadAction::Resume, cx)
                })),
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
                .bg(match chip_tone {
                    ChipTone::Ready => tokens.status.ready,
                    ChipTone::Busy => tokens.status.processing,
                    ChipTone::Idle => tokens.status.imported,
                })
                .text_xs()
                .child(tr(locale, chip_key)),
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
            Button::new(format!("model-details-{model_id}"), details_label)
                .debug_selector(marked("details-toggle"))
                .ghost()
                .compact()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_model_details(details_id.clone(), cx)
                })),
        );

    // A running download or re-probe supersedes the last failure.
    let busy = progress.is_some() || reprobing || starting;
    let failure = facts.failure.clone().filter(|_| !busy).map(|failure| {
        notice(&tokens)
            .debug_selector(marked("failure"))
            .child(
                gpui::svg()
                    .path("icons/triangle-alert.svg")
                    .size(px(16.))
                    .flex_none()
                    .text_color(tokens.warning_text),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_wrap()
                    .gap_x(tokens.spacing.sm)
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(failure.headline),
                    )
                    .child(failure.explanation),
            )
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(tokens.text_muted)
                    .child(failure.time),
            )
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
        let failure_detail = facts.failure.as_ref().map(|failure| {
            detail_text(
                tr(locale, "model_failure_detail"),
                failure.detail.clone(),
                &tokens,
            )
            .debug_selector(marked("failure-detail"))
        });
        // A hairline sets the details apart from what is going on.
        let mut details = div()
            .pt(tokens.spacing.md)
            .border_t_1()
            .border_color(tokens.hairline)
            .flex()
            .flex_col()
            .gap(tokens.spacing.sm)
            .children(failure_detail)
            .child(
                detail_text(
                    tr(locale, "model_source"),
                    facts.source_url.clone(),
                    &tokens,
                )
                .debug_selector(marked("source-url")),
            );
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

    let progress_view = progress
        .as_ref()
        .map(|progress| download_progress_view(&model_id, progress, locale, &tokens, cx));

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
        .gap(tokens.spacing.md)
}

/// A titled text of a card's details, in muted type wrapped to the card's
/// width.
fn detail_text(title: &'static str, text: String, tokens: &Tokens) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(tokens.spacing.xs)
        .text_xs()
        .child(div().text_color(tokens.accent_text).child(title))
        .child(div().min_w_0().text_color(tokens.text_muted).child(text))
}

/// A titled list of a card's details, each row named `<prefix>-<index>`.
fn fact_rows(title: &'static str, rows: &[FactRow], prefix: &str, tokens: &Tokens) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(tokens.spacing.xs)
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
                .gap(tokens.spacing.md)
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

/// A running download: its bar over one compact line, and cancel under the
/// header's pause or resume.
fn download_progress_view(
    model_id: &str,
    progress: &DownloadProgress,
    locale: Locale,
    tokens: &Tokens,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    let total = progress.total_bytes.filter(|total| *total > 0);
    let percent = total.map_or(0., |total| {
        progress.completed_bytes as f32 / total as f32 * 100.
    });
    let indeterminate = match progress.state {
        DownloadState::Verifying => true,
        DownloadState::Downloading => total.is_none(),
        _ => false,
    };
    let download = progress.id;
    let selector = format!("model-download-progress-{model_id}");
    let cancel = format!("model-cancel-{model_id}");
    div()
        .flex()
        .items_center()
        .gap(tokens.spacing.md)
        .child(
            progress_block(
                ProgressBar::new(
                    format!("download-progress-{model_id}"),
                    tr(locale, "model_download_progress"),
                )
                .value(percent)
                .indeterminate(indeterminate),
                download_line(locale, progress),
                tokens,
            )
            .debug_selector(move || selector.clone()),
        )
        .child(
            Button::new(
                format!("download-cancel-{}", download.0),
                tr(locale, "cancel"),
            )
            .debug_selector(move || cancel.clone())
            .ghost()
            .compact()
            .w(ACTION_COLUMN_WIDTH)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.control_download(download, DownloadAction::Cancel, cx)
            })),
        )
        .into_any_element()
}

/// A running download in one line: the file when there are several, the
/// share and amount done, then the rate and time left while it runs. A
/// queued or verifying download says what it waits for.
pub(crate) fn download_line(locale: Locale, progress: &DownloadProgress) -> String {
    match progress.state {
        DownloadState::Queued if progress.retry_count > 0 => {
            return i18n::download_retrying(locale, progress.retry_count)
        }
        DownloadState::Queued => return tr(locale, "download_waiting").into(),
        DownloadState::Verifying => return tr(locale, "download_verifying").into(),
        _ => {}
    }
    let mut parts = Vec::new();
    if progress.artifact_count > 1 {
        parts.push(i18n::download_artifact(
            locale,
            progress.artifact_index.max(1),
            progress.artifact_count,
        ));
    }
    match progress.total_bytes.filter(|total| *total > 0) {
        Some(total) => {
            let done = progress.completed_bytes.min(total);
            parts.push(format!("{}%", done * 100 / total));
            let (divisor, unit, decimals) = binary_unit(total);
            parts.push(format!(
                "{:.1} / {:.*} {unit}",
                progress.completed_bytes as f64 / divisor,
                decimals,
                total as f64 / divisor
            ));
        }
        None => parts.push(format_size(progress.completed_bytes)),
    }
    if progress.state == DownloadState::Downloading {
        parts.extend(
            progress
                .speed_bytes_per_second
                .filter(|speed| *speed > 0)
                .map(format_speed),
        );
        parts.extend(
            progress
                .eta_seconds
                .filter(|seconds| *seconds > 0)
                .map(|seconds| i18n::download_eta(locale, seconds)),
        );
    }
    parts.join(FACT_SEPARATOR)
}

/// The divisor, name and decimals of the binary unit `bytes` reads in.
fn binary_unit(bytes: u64) -> (f64, &'static str, usize) {
    const KIB: u64 = 1 << 10;
    const MIB: u64 = 1 << 20;
    const GIB: u64 = 1 << 30;
    match bytes {
        GIB.. => (GIB as f64, "GiB", 1),
        MIB.. => (MIB as f64, "MiB", 0),
        KIB.. => (KIB as f64, "KiB", 0),
        _ => (1., "B", 0),
    }
}

/// A transfer rate: tenths from MiB/s up, whole KiB/s and B/s below.
fn format_speed(bytes_per_second: u64) -> String {
    let (divisor, unit, _) = binary_unit(bytes_per_second);
    let decimals = usize::from(divisor >= (1 << 20) as f64);
    format!(
        "{:.*} {unit}/s",
        decimals,
        bytes_per_second as f64 / divisor
    )
}

/// A size in binary units: `78 MiB`, `1.0 GiB`.
pub(crate) fn format_size(bytes: u64) -> String {
    let (divisor, unit, decimals) = binary_unit(bytes);
    format!("{:.*} {unit}", decimals, bytes as f64 / divisor)
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

/// The last failed download or re-probe as its card shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FailureFacts {
    /// What failed, in a few words.
    pub(crate) headline: String,
    /// Why, in plain words.
    pub(crate) explanation: String,
    pub(crate) time: String,
    /// The reason with its paths and the backend's own message, in the
    /// details.
    pub(crate) detail: String,
}

/// What a model card shows beyond its id and install status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelFacts {
    /// Task, size, VRAM and backends, each only when the catalog names it.
    pub(crate) summary: String,
    pub(crate) license: String,
    /// The source's host and, for a code or model hub, its project.
    pub(crate) source: String,
    /// The full source, in the details.
    pub(crate) source_url: String,
    /// Each artifact's full declared checksum.
    pub(crate) declared: Vec<FactRow>,
    pub(crate) probe: Option<ProbeFacts>,
    /// The last failed download or re-probe and why.
    pub(crate) failure: Option<FailureFacts>,
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
    ModelFacts {
        summary: fact_summary(model, locale),
        license: model.license.clone(),
        source: short_source(&model.source),
        source_url: model.source.clone(),
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
        failure: entry.failure.as_ref().map(|record| FailureFacts {
            headline: i18n::model_failure_headline(locale, record.operation, &record.reason).into(),
            explanation: i18n::model_failure_explanation(locale, &record.reason),
            time: utc_time(record.failed_at_ms),
            detail: i18n::model_failure_reason(locale, &record.reason),
        }),
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

/// Where a model comes from, as a reader names it: the host, and on GitHub
/// and Hugging Face the owner and project too. A source that is not an
/// HTTP URL is shown as it is.
pub(crate) fn short_source(source: &str) -> String {
    let address = ["https://", "http://"].into_iter().find_map(|scheme| {
        source
            .get(..scheme.len())
            .filter(|prefix| prefix.eq_ignore_ascii_case(scheme))
            .map(|_| &source[scheme.len()..])
    });
    let Some(address) = address else {
        return source.into();
    };
    let address = address.split(['?', '#']).next().unwrap_or_default();
    let mut segments = address.split('/');
    let authority = segments.next().unwrap_or_default();
    let host = authority
        .rsplit('@')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    if host.is_empty() {
        return source.into();
    }
    let project = match host {
        "github.com" | "huggingface.co" => 2,
        _ => 0,
    };
    std::iter::once(host)
        .chain(
            segments
                .filter(|segment| !segment.is_empty())
                .take(project)
                .map(|segment| segment.trim_end_matches(".git")),
        )
        .collect::<Vec<_>>()
        .join("/")
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

/// The one action in a card's header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CardAction {
    /// A ready model that is missing or partial.
    Download,
    /// The same download, after the last one failed.
    Retry,
    /// An installed model.
    Reprobe,
    /// A running download.
    Pause(DownloadId),
    /// A paused download.
    Resume(DownloadId),
    None,
}

/// A running download pauses or resumes; otherwise only a ready model that
/// is missing or partial downloads, and an installed one re-probes, which
/// is also how a failed re-probe is retried. Nothing is offered while the
/// model re-probes.
pub(crate) fn card_action(
    entry: &ModelAvailability,
    download: Option<&DownloadProgress>,
    reprobing: bool,
) -> CardAction {
    if let Some(download) = download {
        return match download.state {
            DownloadState::Paused => CardAction::Resume(download.id),
            _ => CardAction::Pause(download.id),
        };
    }
    if reprobing {
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

/// How a card's status chip is filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChipTone {
    Ready,
    Busy,
    Idle,
}

/// A card's status chip: a running download or check before whether the
/// model is downloaded.
pub(crate) fn status_key(
    installed: bool,
    download: Option<&DownloadState>,
    reprobing: bool,
) -> (&'static str, ChipTone) {
    match download {
        Some(DownloadState::Queued) => ("download_queued", ChipTone::Busy),
        Some(DownloadState::Downloading) => ("downloading", ChipTone::Busy),
        Some(DownloadState::Paused) => ("download_paused", ChipTone::Idle),
        Some(DownloadState::Verifying) => ("verifying", ChipTone::Busy),
        _ if reprobing => ("verifying", ChipTone::Busy),
        _ if installed => ("downloaded", ChipTone::Ready),
        _ => ("not_downloaded", ChipTone::Idle),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Halfway through the first of two files: 32.1 of 78 MiB at 5.2 MiB/s.
    fn progress(state: DownloadState) -> DownloadProgress {
        DownloadProgress {
            id: DownloadId(1),
            model_id: "paraformer-zh".into(),
            state,
            completed_bytes: 33_659_290,
            speed_bytes_per_second: Some(5_452_595),
            total_bytes: Some(81_904_027),
            artifact_index: 1,
            artifact_count: 2,
            eta_seconds: Some(9),
            retry_count: 0,
        }
    }

    #[test]
    fn the_readiness_line_names_the_default_models_that_are_not_ready() {
        let mut sentences = std::collections::BTreeSet::new();
        for (asr, diarization, chinese) in [
            (true, true, "语音识别和说话人分离模型均已就绪"),
            (
                false,
                true,
                "语音识别模型尚未就绪，可在下方下载或到设置中选择",
            ),
            (
                true,
                false,
                "说话人分离模型尚未就绪，可在下方下载或到设置中选择",
            ),
            (
                false,
                false,
                "语音识别和说话人分离模型尚未就绪，可在下方下载或到设置中选择",
            ),
        ] {
            let installation = application::InstallationState {
                default_asr_ready: asr,
                default_diarization_ready: diarization,
                ..Default::default()
            };
            assert_eq!(readiness_text(&installation, Locale::ZhCn), chinese);
            let english = readiness_text(&installation, Locale::EnUs);
            assert!(english.is_ascii() && sentences.insert(english), "{english}");
        }
    }

    #[test]
    fn a_family_counts_its_downloaded_models_and_is_complete_once_all_are() {
        use ModelInstallState::{Installed, Missing, Partial};
        for (states, chinese, english, complete) in [
            (&[Installed][..], "已下载 1 / 1", "1 of 1 downloaded", true),
            (
                &[Installed, Partial],
                "已下载 1 / 2",
                "1 of 2 downloaded",
                false,
            ),
            (
                &[Missing, Missing],
                "已下载 0 / 2",
                "0 of 2 downloaded",
                false,
            ),
            (&[], "已下载 0 / 0", "0 of 0 downloaded", false),
        ] {
            assert_eq!(
                family_summary(Locale::ZhCn, states),
                (chinese.to_string(), complete)
            );
            assert_eq!(
                family_summary(Locale::EnUs, states),
                (english.to_string(), complete)
            );
        }
    }

    #[test]
    fn sizes_read_in_binary_units() {
        for (bytes, size) in [
            (512, "512 B"),
            (2_048, "2 KiB"),
            (81_904_027, "78 MiB"),
            (1 << 30, "1.0 GiB"),
            (3_095_033_483, "2.9 GiB"),
        ] {
            assert_eq!(format_size(bytes), size, "{bytes}");
        }
    }

    #[test]
    fn a_running_download_reads_as_one_compact_line() {
        let running = progress(DownloadState::Downloading);
        assert_eq!(
            download_line(Locale::ZhCn, &running),
            "文件 1/2 · 41% · 32.1 / 78 MiB · 5.2 MiB/s · 约 9 秒"
        );
        assert_eq!(
            download_line(Locale::EnUs, &running),
            "File 1 of 2 · 41% · 32.1 / 78 MiB · 5.2 MiB/s · about 9 s left"
        );

        // One file names no file; a large one counts in GiB, a slow one in
        // KiB/s, and a long wait in minutes or hours.
        let large = DownloadProgress {
            completed_bytes: 1_288_490_189,
            total_bytes: Some(3_095_033_483),
            speed_bytes_per_second: Some(900_000),
            artifact_count: 1,
            eta_seconds: Some(1_996),
            ..running.clone()
        };
        assert_eq!(
            download_line(Locale::ZhCn, &large),
            "41% · 1.2 / 2.9 GiB · 879 KiB/s · 约 33 分钟"
        );
        assert_eq!(
            download_line(Locale::EnUs, &large),
            "41% · 1.2 / 2.9 GiB · 879 KiB/s · about 33 min left"
        );
        for (seconds, chinese, english) in [
            (59, "约 59 秒", "about 59 s left"),
            (89, "约 1 分钟", "about 1 min left"),
            (3_599, "约 1 小时", "about 1 h left"),
            (5_000, "约 1 小时 23 分钟", "about 1 h 23 min left"),
            (7_200, "约 2 小时", "about 2 h left"),
        ] {
            assert_eq!(i18n::download_eta(Locale::ZhCn, seconds), chinese);
            assert_eq!(i18n::download_eta(Locale::EnUs, seconds), english);
        }

        // An unknown total, speed or wait is left out, not shown as zero.
        let unknown = DownloadProgress {
            total_bytes: None,
            speed_bytes_per_second: Some(0),
            eta_seconds: Some(0),
            artifact_count: 1,
            ..running.clone()
        };
        assert_eq!(download_line(Locale::ZhCn, &unknown), "32 MiB");
        let overshoot = DownloadProgress {
            completed_bytes: 90_000_000,
            speed_bytes_per_second: None,
            eta_seconds: None,
            ..running
        };
        assert_eq!(
            download_line(Locale::EnUs, &overshoot),
            "File 1 of 2 · 100% · 85.8 / 78 MiB"
        );
    }

    #[test]
    fn a_paused_queued_or_verifying_download_says_so_without_a_rate() {
        assert_eq!(
            download_line(Locale::ZhCn, &progress(DownloadState::Paused)),
            "文件 1/2 · 41% · 32.1 / 78 MiB"
        );
        let queued = progress(DownloadState::Queued);
        assert_eq!(download_line(Locale::ZhCn, &queued), "等待开始");
        assert_eq!(download_line(Locale::EnUs, &queued), "Waiting to start");
        let retrying = DownloadProgress {
            retry_count: 2,
            ..queued
        };
        assert_eq!(
            download_line(Locale::ZhCn, &retrying),
            "连接中断，正在第 2 次重试"
        );
        assert_eq!(
            download_line(Locale::EnUs, &retrying),
            "Connection lost, retrying (attempt 2)"
        );
        let verifying = progress(DownloadState::Verifying);
        assert_eq!(
            download_line(Locale::ZhCn, &verifying),
            "正在校验下载的文件"
        );
        assert_eq!(
            download_line(Locale::EnUs, &verifying),
            "Checking the downloaded files"
        );
    }

    #[test]
    fn the_status_chip_names_a_running_download_or_check_before_the_install_state() {
        use DownloadState as State;
        for (installed, download, reprobing, chip) in [
            (false, None, false, ("not_downloaded", ChipTone::Idle)),
            (true, None, false, ("downloaded", ChipTone::Ready)),
            (true, None, true, ("verifying", ChipTone::Busy)),
            (
                false,
                Some(State::Queued),
                false,
                ("download_queued", ChipTone::Busy),
            ),
            (
                false,
                Some(State::Downloading),
                false,
                ("downloading", ChipTone::Busy),
            ),
            (
                true,
                Some(State::Paused),
                false,
                ("download_paused", ChipTone::Idle),
            ),
            (
                false,
                Some(State::Verifying),
                false,
                ("verifying", ChipTone::Busy),
            ),
        ] {
            assert_eq!(
                status_key(installed, download.as_ref(), reprobing),
                chip,
                "{installed} {download:?} {reprobing}"
            );
        }
    }

    #[test]
    fn a_source_reads_as_its_host_and_for_hubs_its_owner_and_project() {
        for (source, short) in [
            (
                "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/\
                 sherpa-onnx-paraformer-zh-small-2024-03-09.tar.bz2",
                "github.com/k2-fsa/sherpa-onnx",
            ),
            (
                "https://github.com/k2-fsa/sherpa-onnx.git",
                "github.com/k2-fsa/sherpa-onnx",
            ),
            ("https://github.com/k2-fsa", "github.com/k2-fsa"),
            (
                "https://huggingface.co/avencera/speakrs-models/tree/5d24ffee75f13fb061fa6d10944a64e2dc1d5e6f",
                "huggingface.co/avencera/speakrs-models",
            ),
            (
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin",
                "huggingface.co/ggerganov/whisper.cpp",
            ),
            (
                "HTTPS://HuggingFace.co/pyannote/speaker-diarization-3.1?download=1#top",
                "huggingface.co/pyannote/speaker-diarization-3.1",
            ),
            // Any other host stands alone, without credentials or `www.`.
            (
                "https://www.models.example/bundle-asr/model.bin?token=1",
                "models.example",
            ),
            (
                "http://reader@mirror.example:8080/models/a.bin",
                "mirror.example:8080",
            ),
        ] {
            assert_eq!(short_source(source), short, "{source}");
        }
        // What is not an HTTP URL is shown as it is.
        for source in [
            "local/fixture.bin",
            "ftp://mirror.example/a.bin",
            "https://",
            "",
        ] {
            assert_eq!(short_source(source), source);
        }
    }
}
