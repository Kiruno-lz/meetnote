use crate::{
    components::empty_card,
    controller::{DownloadAction, RootView},
    controls::ProgressBar,
    i18n::{self, text as tr},
    theme,
};
use gpui::{div, prelude::*, px, Context, IntoElement, Pixels};
use gpui_component::{
    button::{Button, ButtonVariants},
    collapsible::Collapsible,
    scroll::ScrollableElement as _,
};
use models::{manager::DownloadProgress, ModelAvailability, ModelInstallState};
use std::collections::BTreeMap;

/// The install status chip column; fixed, so the chips of every model row
/// line up whatever the model id's length.
const STATUS_COLUMN_WIDTH: Pixels = px(96.);
/// The action column after it, empty when a model offers no download.
const ACTION_COLUMN_WIDTH: Pixels = px(88.);

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

fn model_card(
    root: &mut RootView,
    entry: ModelAvailability,
    progress: Option<DownloadProgress>,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    let tokens = *theme::tokens(cx);
    let model_id = entry.model.id.clone();
    let downloading = progress.as_ref().is_some_and(|progress| {
        matches!(
            progress.state,
            models::manager::DownloadState::Queued
                | models::manager::DownloadState::Downloading
                | models::manager::DownloadState::Paused
                | models::manager::DownloadState::Verifying
        )
    });
    let action = if entry.model.status == "ready"
        && matches!(
            entry.install_state,
            ModelInstallState::Missing | ModelInstallState::Partial
        )
        && !downloading
    {
        let id = model_id.clone();
        Button::new(format!("download-{id}"))
            .primary()
            .compact()
            .label(tr(root.ui.locale, "download"))
            .on_click(cx.listener(move |this, _, _, cx| this.download_model(id.clone(), cx)))
            .into_any_element()
    } else {
        div().into_any_element()
    };
    let downloaded = matches!(entry.install_state, ModelInstallState::Installed);
    let progress_view = progress
        .filter(|progress| {
            matches!(
                progress.state,
                models::manager::DownloadState::Queued
                    | models::manager::DownloadState::Downloading
                    | models::manager::DownloadState::Paused
                    | models::manager::DownloadState::Verifying
            )
        })
        .map(|progress| download_progress_view(root, &model_id, progress, cx));

    div()
        .p_4()
        .rounded(tokens.radius.sm)
        .bg(tokens.raised)
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(div().flex_1().min_w_0().text_base().child(model_id.clone()))
                .child({
                    let chip_id = model_id.clone();
                    div()
                        .debug_selector(move || format!("model-status-{chip_id}"))
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
                            tr(root.ui.locale, "downloaded")
                        } else {
                            tr(root.ui.locale, "not_downloaded")
                        })
                })
                .child(
                    div()
                        .w(ACTION_COLUMN_WIDTH)
                        .flex_shrink_0()
                        .flex()
                        .justify_end()
                        .child(action),
                ),
        )
        .child(
            div()
                .mt_2()
                .flex()
                .gap_6()
                .text_sm()
                .text_color(tokens.text_muted)
                .child(format!(
                    "{}: {}",
                    tr(root.ui.locale, "model_size"),
                    format_bytes(entry.model.size_bytes)
                ))
                .child(format!(
                    "{}: {}",
                    tr(root.ui.locale, "vram_estimate"),
                    format_bytes(entry.model.estimated_vram_bytes)
                ))
                .child(format!(
                    "{}: {}",
                    tr(root.ui.locale, "applicable_backends"),
                    if entry.model.candidate_backends.is_empty() {
                        "—".into()
                    } else {
                        entry.model.candidate_backends.join(", ")
                    }
                )),
        )
        .children(progress_view)
        .into_any_element()
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
        models::manager::DownloadState::Paused => DownloadAction::Resume,
        models::manager::DownloadState::Queued
        | models::manager::DownloadState::Downloading
        | models::manager::DownloadState::Verifying => DownloadAction::Pause,
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
