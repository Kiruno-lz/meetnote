use crate::{
    components::empty_card,
    controller::{DownloadAction, RootView},
    i18n::{self, text as tr},
};
use gpui::{div, prelude::*, rgb, Context, IntoElement};
use gpui_component::{
    button::{Button, ButtonVariants},
    collapsible::Collapsible,
    progress::Progress,
    scroll::ScrollableElement as _,
};
use models::{manager::DownloadProgress, ModelAvailability, ModelInstallState};
use std::collections::BTreeMap;

pub(crate) fn render(root: &mut RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let entries = root.services.model_manager.browse(None, None);
    let downloads = root.services.model_manager.list().unwrap_or_default();
    let mut groups: BTreeMap<String, Vec<ModelAvailability>> = BTreeMap::new();
    for entry in entries {
        groups
            .entry(entry.model.model_family.clone())
            .or_default()
            .push(entry);
    }

    let mut content = div()
        .id("models-scroll")
        .flex_1()
        .min_h_0()
        .overflow_y_scrollbar()
        .flex()
        .flex_col()
        .gap_4();
    if groups.is_empty() {
        content = content.child(empty_card(tr(root.ui.locale, "no_models")));
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
            let group = div().flex_shrink_0().rounded_lg().bg(rgb(0x171b22)).child(
                Collapsible::new()
                    .open(expanded)
                    .child(
                        div()
                            .id(format!("model-family-{family}"))
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
                            .child(div().text_sm().text_color(rgb(0x8992a3)).child(format!(
                                "{} {}",
                                model_count,
                                tr(locale, "models_count")
                            ))),
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
        .flex_1()
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
                .text_color(rgb(0x8992a3))
                .child(tr(root.ui.locale, "catalog_description")),
        )
        .child(content)
        .child(
            div()
                .flex_shrink_0()
                .pt_2()
                .text_xs()
                .text_color(rgb(0x8992a3))
                .child(root.ui.notice.clone()),
        )
}

fn model_card(
    root: &mut RootView,
    entry: ModelAvailability,
    progress: Option<DownloadProgress>,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    let model_id = entry.model.id.clone();
    let downloading = progress.as_ref().is_some_and(|progress| {
        matches!(
            progress.state,
            models::manager::DownloadState::Queued
                | models::manager::DownloadState::Downloading
                | models::manager::DownloadState::Paused
        )
    });
    let action = if matches!(
        entry.install_state,
        ModelInstallState::Missing | ModelInstallState::Partial
    ) && !downloading
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
            )
        })
        .map(|progress| download_progress_view(root, &model_id, progress, cx));

    div()
        .p_4()
        .rounded_md()
        .bg(rgb(0x202733))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(div().text_base().child(model_id.clone()))
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(0x60a5fa))
                        .child(if downloaded {
                            tr(root.ui.locale, "downloaded")
                        } else {
                            tr(root.ui.locale, "not_downloaded")
                        }),
                )
                .child(action),
        )
        .child(
            div()
                .mt_2()
                .flex()
                .gap_6()
                .text_sm()
                .text_color(rgb(0x8992a3))
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
    let percent = progress
        .total_bytes
        .filter(|total| *total > 0)
        .map(|total| progress.completed_bytes as f32 / total as f32 * 100.)
        .unwrap_or(0.);
    let loading = progress.total_bytes.is_none();
    let action = match progress.state {
        models::manager::DownloadState::Paused => DownloadAction::Resume,
        models::manager::DownloadState::Queued | models::manager::DownloadState::Downloading => {
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
                    Progress::new(format!("download-progress-{model_id}"))
                        .value(percent)
                        .loading(loading)
                        .accessibility_label("model download progress"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(0x8992a3))
                        .child(i18n::download_progress(
                            root.ui.locale,
                            model_id,
                            progress.artifact_index,
                            progress.artifact_count,
                            progress.completed_bytes,
                            progress.speed_bytes_per_second,
                        )),
                ),
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
