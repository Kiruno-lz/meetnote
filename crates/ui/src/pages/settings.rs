use crate::{
    components::{setting_control_row, settings_section},
    controller::RootView,
    i18n::{self, text as tr, Locale},
};
use gpui::{div, prelude::*, px, rgb, Context, IntoElement};
use models::ModelInstallState;

pub(crate) fn render(root: &mut RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let snapshot = root.settings_draft.clone();
    let asr_backend = snapshot
        .asr_backend_id
        .clone()
        .unwrap_or_else(|| tr(root.ui.locale, "auto").into());
    let diarization_backend = snapshot
        .diarization_backend_id
        .clone()
        .unwrap_or_else(|| tr(root.ui.locale, "auto").into());
    let asr_model = snapshot
        .asr_model
        .as_ref()
        .map(|model| model.model_id.clone())
        .unwrap_or_else(|| tr(root.ui.locale, "not_selected").into());
    let diarization_model = snapshot
        .diarization_model
        .as_ref()
        .map(|model| model.model_id.clone())
        .unwrap_or_else(|| tr(root.ui.locale, "not_selected").into());
    let expected_speakers = snapshot
        .expected_speakers
        .map(|value| value.to_string())
        .unwrap_or_else(|| tr(root.ui.locale, "auto").into());

    let body = div()
        .id("settings-scroll")
        .flex_1()
        .overflow_scroll()
        .track_scroll(&root.settings_scroll_handle)
        .flex()
        .flex_col()
        .gap_5()
        .child(settings_section(
            tr(root.ui.locale, "interface"),
            vec![
                setting_control_row(tr(root.ui.locale, "language"), locale_selector(root, cx))
                    .into_any_element(),
            ],
        ))
        .child(settings_section(
            tr(root.ui.locale, "transcription"),
            vec![
                setting_control_row(
                    tr(root.ui.locale, "asr_backend"),
                    backend_selector(root, "asr_backend", &asr_backend, true, cx),
                )
                .into_any_element(),
                setting_control_row(
                    tr(root.ui.locale, "asr_model"),
                    model_selector(
                        root,
                        "asr_model",
                        &asr_model,
                        "asr",
                        snapshot.asr_backend_id.as_deref(),
                        cx,
                    ),
                )
                .into_any_element(),
                setting_control_row(
                    tr(root.ui.locale, "transcription_language"),
                    static_selector(
                        "transcription-language",
                        snapshot
                            .language
                            .as_deref()
                            .unwrap_or(tr(root.ui.locale, "auto")),
                    ),
                )
                .into_any_element(),
            ],
        ))
        .child(settings_section(
            tr(root.ui.locale, "speaker_separation"),
            vec![
                setting_control_row(
                    tr(root.ui.locale, "diarization_backend"),
                    backend_selector(root, "diarization_backend", &diarization_backend, false, cx),
                )
                .into_any_element(),
                setting_control_row(
                    tr(root.ui.locale, "diarization_model"),
                    model_selector(
                        root,
                        "diarization_model",
                        &diarization_model,
                        "diarization",
                        snapshot.diarization_backend_id.as_deref(),
                        cx,
                    ),
                )
                .into_any_element(),
                setting_control_row(
                    tr(root.ui.locale, "expected_speakers"),
                    static_selector("expected-speakers", &expected_speakers),
                )
                .into_any_element(),
            ],
        ))
        .child(settings_section(
            tr(root.ui.locale, "runtime"),
            vec![setting_control_row(
                tr(root.ui.locale, "compute_device"),
                static_selector(
                    "compute-device",
                    i18n::device(root.ui.locale, snapshot.compute_device),
                ),
            )
            .into_any_element()],
        ));

    div()
        .flex_1()
        .h_full()
        .p_6()
        .flex()
        .flex_col()
        .gap_4()
        .child(
            div()
                .flex_shrink_0()
                .text_2xl()
                .child(tr(root.ui.locale, "settings")),
        )
        .child(body)
        .child(
            div()
                .flex_shrink_0()
                .border_t_1()
                .border_color(rgb(0x2b303b))
                .pt_3()
                .flex()
                .justify_between()
                .child(
                    div()
                        .id("reset-settings")
                        .cursor_pointer()
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .bg(rgb(0x263142))
                        .text_sm()
                        .child(tr(root.ui.locale, "reset_settings"))
                        .on_click(cx.listener(RootView::reset_settings)),
                )
                .child(
                    div()
                        .id("save-settings")
                        .cursor_pointer()
                        .px_4()
                        .py_2()
                        .rounded_md()
                        .bg(rgb(0x2563eb))
                        .text_sm()
                        .child(tr(root.ui.locale, "save_settings"))
                        .on_click(cx.listener(RootView::save_settings)),
                ),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(rgb(0x8992a3))
                .child(root.ui.notice.clone()),
        )
}

fn backend_selector(
    root: &mut RootView,
    key: &'static str,
    current: &str,
    asr: bool,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    let descriptors = if asr {
        root.inventory.asr.clone()
    } else {
        root.inventory.diarization.clone()
    };
    let options = descriptors
        .into_iter()
        .map(|descriptor| {
            let backend_id = descriptor.id.clone();
            let option = div()
                .id(format!("setting-option-{key}-{backend_id}"))
                .cursor_pointer()
                .px_3()
                .py_2()
                .rounded_md()
                .hover(|style| style.bg(rgb(0x263142)))
                .child(descriptor.id);
            if asr {
                option
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_asr_backend(backend_id.clone(), cx)
                    }))
                    .into_any_element()
            } else {
                option
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_diarization_backend(backend_id.clone(), cx)
                    }))
                    .into_any_element()
            }
        })
        .collect();
    dropdown(root, key, current, options, cx)
}

fn model_selector(
    root: &mut RootView,
    key: &'static str,
    current: &str,
    task: &'static str,
    backend_id: Option<&str>,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    let options = root
        .services
        .model_manager
        .browse(Some(task), backend_id)
        .into_iter()
        .filter(|entry| entry.install_state == ModelInstallState::Installed)
        .filter_map(|entry| {
            let model_id = entry.model.id.clone();
            let backend_id = entry.model.candidate_backends.first()?.clone();
            Some(
                div()
                    .id(format!("setting-option-{key}-{model_id}"))
                    .cursor_pointer()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .hover(|style| style.bg(rgb(0x263142)))
                    .child(model_id.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.use_model(model_id.clone(), task.into(), backend_id.clone(), cx)
                    }))
                    .into_any_element(),
            )
        })
        .collect();
    dropdown(root, key, current, options, cx)
}

fn locale_selector(root: &mut RootView, cx: &mut Context<RootView>) -> gpui::AnyElement {
    let options = [(Locale::ZhCn, "简体中文"), (Locale::EnUs, "English")]
        .into_iter()
        .map(|(locale, label)| {
            div()
                .id(format!("setting-option-language-{}", locale.code()))
                .cursor_pointer()
                .px_3()
                .py_2()
                .rounded_md()
                .hover(|style| style.bg(rgb(0x263142)))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| this.set_locale(locale, cx)))
                .into_any_element()
        })
        .collect();
    dropdown(
        root,
        "language",
        locale_display(root.ui.locale),
        options,
        cx,
    )
}

fn dropdown(
    root: &mut RootView,
    key: &'static str,
    current: &str,
    options: Vec<gpui::AnyElement>,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    let open = root.ui.open_setting.as_deref() == Some(key);
    let key_for_click = key.to_string();
    let control = div()
        .id(format!("setting-select-{key}"))
        .w(px(280.))
        .px_3()
        .py_2()
        .rounded_md()
        .bg(rgb(0x202733))
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_between()
        .child(current.to_string())
        .child(div().text_color(rgb(0x8992a3)).child("⌄"))
        .on_click(
            cx.listener(move |this, _, _, cx| this.toggle_setting(key_for_click.clone(), cx)),
        );
    let mut selector = div().flex().flex_col().items_end().child(control);
    if open && !options.is_empty() {
        selector = selector.child(
            div()
                .mt_1()
                .w(px(280.))
                .p_1()
                .rounded_md()
                .bg(rgb(0x171b22))
                .flex()
                .flex_col()
                .children(options),
        );
    }
    selector.into_any_element()
}

fn static_selector(key: &str, current: &str) -> gpui::AnyElement {
    div()
        .id(format!("setting-value-{key}"))
        .w(px(280.))
        .px_3()
        .py_2()
        .rounded_md()
        .bg(rgb(0x202733))
        .flex()
        .items_center()
        .justify_between()
        .child(current.to_string())
        .child(div().text_color(rgb(0x8992a3)).child("⌄"))
        .into_any_element()
}

fn locale_display(locale: Locale) -> &'static str {
    match locale {
        Locale::ZhCn => "简体中文",
        Locale::EnUs => "English",
    }
}
