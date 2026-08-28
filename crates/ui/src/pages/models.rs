use crate::{
    components::empty_card,
    controller::{DownloadAction, RootView},
    i18n::{self, text as tr},
};
use gpui::{div, prelude::*, rgb, Context, IntoElement};
use models::{ModelAvailability, ModelInstallState};
use std::collections::BTreeMap;

pub(crate) fn render(root: &mut RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let entries = root.services.model_manager.browse(None, None);
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
        .overflow_scroll()
        .track_scroll(&root.models_scroll_handle)
        .flex()
        .flex_col()
        .gap_4();
    if groups.is_empty() {
        content = content.child(empty_card(tr(root.ui.locale, "no_models")));
    } else {
        for (family, entries) in groups {
            let expanded = root.ui.expanded_model_families.contains(&family);
            let family_for_click = family.clone();
            let mut group = div().rounded_lg().bg(rgb(0x171b22)).child(
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
                        entries.len(),
                        tr(root.ui.locale, "models_count")
                    ))),
            );
            if expanded {
                let cards = entries.into_iter().map(|entry| model_card(root, entry, cx));
                group = group.child(
                    div()
                        .px_4()
                        .pb_4()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .children(cards),
                );
            }
            content = content.child(group);
        }
    }

    let downloads = root.services.model_manager.list().unwrap_or_default();
    if !downloads.is_empty() {
        let mut queue = div()
            .p_4()
            .rounded_lg()
            .bg(rgb(0x1b2029))
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_lg().child(tr(root.ui.locale, "download_queue")));
        for progress in downloads {
            let id = progress.id;
            let action = match &progress.state {
                models::manager::DownloadState::Paused => DownloadAction::Resume,
                models::manager::DownloadState::Queued
                | models::manager::DownloadState::Downloading => DownloadAction::Pause,
                _ => DownloadAction::Cancel,
            };
            let label = match action {
                DownloadAction::Pause => tr(root.ui.locale, "pause"),
                DownloadAction::Resume => tr(root.ui.locale, "resume"),
                DownloadAction::Cancel => tr(root.ui.locale, "cancel"),
            };
            queue = queue.child(
                div()
                    .p_3()
                    .rounded_md()
                    .bg(rgb(0x202733))
                    .child(i18n::download_progress(
                        root.ui.locale,
                        &progress.model_id,
                        progress.artifact_index,
                        progress.artifact_count,
                        progress.completed_bytes,
                    ))
                    .child(
                        div()
                            .id(format!("download-action-{}", id.0))
                            .cursor_pointer()
                            .mt_2()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(rgb(0x263142))
                            .text_xs()
                            .child(label)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.control_download(id, action, cx)
                            })),
                    ),
            );
        }
        content = content.child(queue);
    }

    div()
        .flex_1()
        .h_full()
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
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    let model_id = entry.model.id.clone();
    let task = entry.model.task.clone();
    let action = if matches!(
        entry.install_state,
        ModelInstallState::Missing | ModelInstallState::Partial
    ) {
        let id = model_id.clone();
        div()
            .id(format!("download-{id}"))
            .cursor_pointer()
            .px_3()
            .py_1()
            .rounded_md()
            .bg(rgb(0x2563eb))
            .text_sm()
            .child(tr(root.ui.locale, "download"))
            .on_click(cx.listener(move |this, _, _, cx| this.download_model(id.clone(), cx)))
            .into_any_element()
    } else if let Some(backend_id) = entry.model.candidate_backends.first().cloned() {
        let id = model_id.clone();
        let backend = backend_id.clone();
        div()
            .id(format!("use-{id}"))
            .cursor_pointer()
            .px_3()
            .py_1()
            .rounded_md()
            .bg(rgb(0x263142))
            .text_sm()
            .child(tr(root.ui.locale, "use_settings"))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.use_model(id.clone(), task.clone(), backend.clone(), cx)
            }))
            .into_any_element()
    } else {
        div().into_any_element()
    };
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
                        .child(i18n::install_state(root.ui.locale, entry.install_state)),
                )
                .child(action),
        )
        .child(
            div()
                .mt_2()
                .text_sm()
                .text_color(rgb(0x8992a3))
                .child(format!(
                    "{} · {} · {}",
                    entry.model.task, entry.model.source_format, entry.model.license
                )),
        )
        .into_any_element()
}
