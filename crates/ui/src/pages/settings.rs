use crate::{
    components::{setting_control_row, settings_section},
    controller::{ModelOption, RootView},
    i18n::text as tr,
};
use gpui::{div, prelude::*, px, Context, IntoElement};
use gpui_component::{
    button::{Button, ButtonVariants},
    scroll::ScrollableElement as _,
    select::Select,
};

pub(crate) fn render(root: &mut RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let Some(selectors) = root.settings_selectors.as_ref() else {
        return div().flex_1().into_any_element();
    };

    let body = div()
        .id("settings-scroll")
        .flex_1()
        .min_h_0()
        .overflow_y_scrollbar()
        .flex()
        .flex_col()
        .gap_5()
        .child(settings_section(
            tr(root.ui.locale, "interface"),
            vec![setting_control_row(
                tr(root.ui.locale, "language"),
                select_control(&selectors.language, "language"),
            )
            .into_any_element()],
        ))
        .child(settings_section(
            tr(root.ui.locale, "transcription"),
            vec![
                setting_control_row(
                    tr(root.ui.locale, "asr_backend"),
                    select_control(&selectors.asr_backend, "asr-backend"),
                )
                .into_any_element(),
                setting_control_row(
                    tr(root.ui.locale, "asr_model"),
                    model_select_control(&selectors.asr_model, "asr-model"),
                )
                .into_any_element(),
                setting_control_row(
                    tr(root.ui.locale, "transcription_language"),
                    select_control(&selectors.transcription_language, "transcription-language"),
                )
                .into_any_element(),
            ],
        ))
        .child(settings_section(
            tr(root.ui.locale, "speaker_separation"),
            vec![
                setting_control_row(
                    tr(root.ui.locale, "diarization_backend"),
                    select_control(&selectors.diarization_backend, "diarization-backend"),
                )
                .into_any_element(),
                setting_control_row(
                    tr(root.ui.locale, "diarization_model"),
                    model_select_control(&selectors.diarization_model, "diarization-model"),
                )
                .into_any_element(),
                setting_control_row(
                    tr(root.ui.locale, "expected_speakers"),
                    select_control(&selectors.expected_speakers, "expected-speakers"),
                )
                .into_any_element(),
                setting_control_row(
                    tr(root.ui.locale, "speaker_library_path"),
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .w(px(280.))
                                .text_xs()
                                .text_color(gpui::rgb(0x8992a3))
                                .child(root.settings_draft.speaker_library_path.clone()),
                        )
                        .child(
                            Button::new("choose-speaker-library")
                                .secondary()
                                .compact()
                                .label(tr(root.ui.locale, "choose_directory"))
                                .on_click(cx.listener(RootView::choose_speaker_library_directory)),
                        )
                        .into_any_element(),
                )
                .into_any_element(),
            ],
        ))
        .child(settings_section(
            tr(root.ui.locale, "runtime"),
            vec![setting_control_row(
                tr(root.ui.locale, "compute_device"),
                select_control(&selectors.compute_device, "compute-device"),
            )
            .into_any_element()],
        ));

    div()
        .flex_1()
        .h_full()
        .min_h_0()
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
        .into_any_element()
}

fn select_control(
    state: &gpui::Entity<gpui_component::select::SelectState<Vec<String>>>,
    id: &'static str,
) -> gpui::AnyElement {
    Select::new(state)
        .w(px(280.))
        .menu_width(px(280.))
        .accessibility_label(id)
        .into_any_element()
}

fn model_select_control(
    state: &gpui::Entity<gpui_component::select::SelectState<Vec<ModelOption>>>,
    id: &'static str,
) -> gpui::AnyElement {
    Select::new(state)
        .w(px(280.))
        .menu_width(px(280.))
        .accessibility_label(id)
        .into_any_element()
}
