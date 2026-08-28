use crate::{
    components::{setting_row, settings_section},
    controller::RootView,
    i18n::{self, text as tr, Locale},
};
use gpui::{div, prelude::*, rgb, Context, IntoElement};

pub(crate) fn render(root: &mut RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let snapshot = root.services.settings.snapshot().unwrap_or_default();
    let asr_choices = backend_choices(
        root.inventory.asr.clone(),
        snapshot.asr_backend_id.as_deref(),
        true,
        cx,
    );
    let diarization_choices = backend_choices(
        root.inventory.diarization.clone(),
        snapshot.diarization_backend_id.as_deref(),
        false,
        cx,
    );
    let locale_choices = locale_choices(root, cx);
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
        .gap_4()
        .child(settings_section(
            tr(root.ui.locale, "interface"),
            tr(root.ui.locale, "interface_description"),
            vec![
                setting_row(
                    tr(root.ui.locale, "language"),
                    locale_display(root.ui.locale),
                )
                .into_any_element(),
                locale_choices.into_any_element(),
            ],
        ))
        .child(settings_section(
            tr(root.ui.locale, "transcription"),
            tr(root.ui.locale, "transcription_description"),
            vec![
                setting_row(
                    tr(root.ui.locale, "asr_backend"),
                    snapshot
                        .asr_backend_id
                        .as_deref()
                        .unwrap_or(tr(root.ui.locale, "auto")),
                )
                .into_any_element(),
                asr_choices.into_any_element(),
                setting_row(tr(root.ui.locale, "asr_model"), asr_model).into_any_element(),
                setting_row(
                    tr(root.ui.locale, "transcription_language"),
                    snapshot
                        .language
                        .as_deref()
                        .unwrap_or(tr(root.ui.locale, "auto")),
                )
                .into_any_element(),
            ],
        ))
        .child(settings_section(
            tr(root.ui.locale, "speaker_separation"),
            tr(root.ui.locale, "speaker_description"),
            vec![
                setting_row(
                    tr(root.ui.locale, "diarization_backend"),
                    snapshot
                        .diarization_backend_id
                        .as_deref()
                        .unwrap_or(tr(root.ui.locale, "auto")),
                )
                .into_any_element(),
                diarization_choices.into_any_element(),
                setting_row(tr(root.ui.locale, "diarization_model"), diarization_model)
                    .into_any_element(),
                setting_row(tr(root.ui.locale, "expected_speakers"), expected_speakers)
                    .into_any_element(),
            ],
        ))
        .child(settings_section(
            tr(root.ui.locale, "runtime"),
            tr(root.ui.locale, "runtime_description"),
            vec![
                setting_row(
                    tr(root.ui.locale, "compute_device"),
                    i18n::device(root.ui.locale, snapshot.compute_device),
                )
                .into_any_element(),
                div()
                    .mt_4()
                    .text_sm()
                    .text_color(rgb(0x60a5fa))
                    .child(tr(root.ui.locale, "saved_automatically"))
                    .into_any_element(),
            ],
        ));

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
                .child(tr(root.ui.locale, "settings")),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_sm()
                .text_color(rgb(0x8992a3))
                .child(tr(root.ui.locale, "settings_description")),
        )
        .child(body)
        .child(
            div()
                .flex_shrink_0()
                .pt_2()
                .text_xs()
                .text_color(rgb(0x8992a3))
                .child(root.ui.notice.clone()),
        )
}

fn backend_choices(
    descriptors: Vec<application::BackendDescriptor>,
    selected: Option<&str>,
    asr: bool,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let mut choices = div().flex().gap_2().flex_wrap();
    for descriptor in descriptors {
        let backend_id = descriptor.id.clone();
        let selected = selected == Some(descriptor.id.as_str());
        let button = div()
            .id(if asr {
                format!("asr-backend-{}", descriptor.id)
            } else {
                format!("diarization-backend-{}", descriptor.id)
            })
            .cursor_pointer()
            .px_3()
            .py_2()
            .rounded_md()
            .bg(if selected {
                rgb(0x2563eb)
            } else {
                rgb(0x263142)
            })
            .child(descriptor.id);
        choices = if asr {
            choices.child(button.on_click(
                cx.listener(move |this, _, _, cx| this.select_asr_backend(backend_id.clone(), cx)),
            ))
        } else {
            choices.child(button.on_click(cx.listener(move |this, _, _, cx| {
                this.select_diarization_backend(backend_id.clone(), cx)
            })))
        };
    }
    choices
}

fn locale_choices(root: &mut RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let mut choices = div().flex().gap_2();
    for (locale, label) in [(Locale::ZhCn, "简体中文"), (Locale::EnUs, "English")] {
        let selected = root.ui.locale == locale;
        choices = choices.child(
            div()
                .id(format!("locale-{}", locale.code()))
                .cursor_pointer()
                .px_3()
                .py_2()
                .rounded_md()
                .bg(if selected {
                    rgb(0x2563eb)
                } else {
                    rgb(0x263142)
                })
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| this.set_locale(locale, cx))),
        );
    }
    choices
}

fn locale_display(locale: Locale) -> &'static str {
    match locale {
        Locale::ZhCn => "简体中文 · zh-CN",
        Locale::EnUs => "English · en-US",
    }
}
