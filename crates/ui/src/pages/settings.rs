use crate::{
    assistant_ui::ConnectionProbeKind,
    components::{setting_control_row, settings_section},
    controller::{settings_error_text, ModelOption, RootView},
    i18n::text as tr,
    theme::{self, Tokens},
};
use contracts::AgentWriteMode;
use gpui::{div, prelude::*, px, Context, IntoElement};
use gpui_component::{
    button::{Button, ButtonVariants},
    scroll::ScrollableElement as _,
    select::Select,
    Disableable,
};
use settings::{AssistantAuthMode, LlmProtocol};

pub(crate) fn render(root: &mut RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let tokens = *theme::tokens(cx);
    let assistant = assistant_settings_section(root, &tokens, cx);
    let Some(selectors) = root.settings_selectors.as_ref() else {
        return div().flex_1().bg(tokens.content).into_any_element();
    };

    let body = div()
        .id("settings-scroll")
        .debug_selector(|| "settings-scroll".into())
        .flex_1()
        .min_h_0()
        .overflow_y_scrollbar()
        .flex()
        .flex_col()
        .gap_5()
        .child(settings_section(
            tr(root.ui.locale, "appearance"),
            vec![
                setting_control_row(
                    tr(root.ui.locale, "theme"),
                    select_control(&selectors.theme, "theme"),
                    &tokens,
                )
                .into_any_element(),
                setting_control_row(
                    tr(root.ui.locale, "material"),
                    select_control(&selectors.material, "material"),
                    &tokens,
                )
                .into_any_element(),
            ],
        ))
        .child(settings_section(
            tr(root.ui.locale, "interface"),
            vec![setting_control_row(
                tr(root.ui.locale, "language"),
                select_control(&selectors.language, "language"),
                &tokens,
            )
            .into_any_element()],
        ))
        .child(settings_section(
            tr(root.ui.locale, "transcription"),
            vec![
                setting_control_row(
                    tr(root.ui.locale, "asr_backend"),
                    select_control(&selectors.asr_backend, "asr-backend"),
                    &tokens,
                )
                .into_any_element(),
                setting_control_row(
                    tr(root.ui.locale, "asr_model"),
                    model_select_control(&selectors.asr_model, "asr-model"),
                    &tokens,
                )
                .into_any_element(),
                setting_control_row(
                    tr(root.ui.locale, "transcription_language"),
                    select_control(&selectors.transcription_language, "transcription-language"),
                    &tokens,
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
                    &tokens,
                )
                .into_any_element(),
                setting_control_row(
                    tr(root.ui.locale, "diarization_model"),
                    model_select_control(&selectors.diarization_model, "diarization-model"),
                    &tokens,
                )
                .into_any_element(),
                if root.show_diarization_stage_options() {
                    setting_control_row(
                        tr(root.ui.locale, "diarization_segmentation_model"),
                        stage_model_select_control(
                            &selectors.diarization_segmentation_model,
                            "diarization-segmentation-model",
                        ),
                        &tokens,
                    )
                    .into_any_element()
                } else {
                    div().into_any_element()
                },
                if root.show_diarization_stage_options() {
                    setting_control_row(
                        tr(root.ui.locale, "diarization_embedding_model"),
                        stage_model_select_control(
                            &selectors.diarization_embedding_model,
                            "diarization-embedding-model",
                        ),
                        &tokens,
                    )
                    .into_any_element()
                } else {
                    div().into_any_element()
                },
                if root.show_diarization_stage_options() {
                    setting_control_row(
                        tr(root.ui.locale, "diarization_clustering"),
                        select_control(&selectors.diarization_clustering, "diarization-clustering"),
                        &tokens,
                    )
                    .into_any_element()
                } else {
                    div().into_any_element()
                },
                setting_control_row(
                    tr(root.ui.locale, "expected_speakers"),
                    select_control(&selectors.expected_speakers, "expected-speakers"),
                    &tokens,
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
                                .text_color(tokens.text_muted)
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
                    &tokens,
                )
                .into_any_element(),
            ],
        ))
        .child(settings_section(
            tr(root.ui.locale, "runtime"),
            vec![setting_control_row(
                tr(root.ui.locale, "compute_device"),
                select_control(&selectors.compute_device, "compute-device"),
                &tokens,
            )
            .into_any_element()],
        ))
        .child(assistant);

    div()
        .debug_selector(|| "settings-page".into())
        .flex_1()
        .h_full()
        .bg(tokens.content)
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
        .children(root.settings_error.as_ref().map(|error| {
            div()
                .id("settings-error")
                .debug_selector(|| "settings-error".into())
                .flex_shrink_0()
                .text_sm()
                .child(settings_error_text(root.ui.locale, error))
        }))
        .child(body)
        .into_any_element()
}

fn assistant_settings_section(
    root: &RootView,
    tokens: &Tokens,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    let locale = root.ui.locale;
    let active = root
        .settings_draft
        .assistant
        .active_connection_id
        .as_deref();
    let profiles = root
        .settings_draft
        .assistant
        .connections
        .iter()
        .map(|profile| {
            let id = profile.id.clone();
            let selected = active == Some(id.as_str());
            let mut button = Button::new(format!("assistant-profile-{id}"))
                .compact()
                .label(profile.name.clone());
            button = if selected {
                button.primary()
            } else {
                button.secondary()
            };
            button
                .on_click(cx.listener(move |root, _, _, cx| root.edit_assistant_profile(&id, cx)))
                .into_any_element()
        })
        .collect::<Vec<_>>();
    let diagnostic = root
        .assistant
        .diagnostic
        .as_ref()
        .map(|result| match result {
            Ok(diagnostic) => format!(
                "{} · {} · text={} · tools={} · {} → {} · context={} · models={} · {}",
                if diagnostic.local {
                    tr(locale, "assistant_local_connection")
                } else {
                    tr(locale, "assistant_remote_connection")
                },
                if diagnostic.ready {
                    tr(locale, "ready")
                } else {
                    tr(locale, "failed")
                },
                capability(diagnostic.text),
                capability(diagnostic.client_tools),
                diagnostic.requested_model,
                diagnostic.resolved_model.as_deref().unwrap_or("—"),
                diagnostic
                    .context_window_tokens
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "—".into()),
                diagnostic
                    .models
                    .as_ref()
                    .map(|models| models.join(", "))
                    .unwrap_or_else(|| "—".into()),
                diagnostic.message
            ),
            Err(error) => error.to_string(),
        });
    let inputs = &root.assistant_inputs;

    settings_section(
        tr(locale, "assistant_settings"),
        vec![
            setting_control_row(
                tr(locale, "assistant_enabled"),
                choice_buttons(
                    [
                        ("assistant-enabled", tr(locale, "enabled"), true),
                        ("assistant-disabled", tr(locale, "disabled"), false),
                    ],
                    root.settings_draft.assistant.enabled,
                    cx,
                    |root, value, cx| root.set_assistant_enabled(value, cx),
                ),
                tokens,
            )
            .into_any_element(),
            setting_control_row(
                tr(locale, "assistant_profiles"),
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .children(profiles)
                    .child(
                        Button::new("assistant-new-profile")
                            .secondary()
                            .compact()
                            .label(tr(locale, "assistant_new_profile"))
                            .on_click(cx.listener(RootView::new_assistant_profile)),
                    )
                    .into_any_element(),
                tokens,
            )
            .into_any_element(),
            assistant_text_row(
                tr(locale, "assistant_connection_id"),
                &inputs.connection_id,
                tokens,
            ),
            assistant_text_row(
                tr(locale, "assistant_connection_name"),
                &inputs.connection_name,
                tokens,
            ),
            setting_control_row(
                tr(locale, "assistant_protocol"),
                protocol_buttons(root.assistant_protocol, locale, cx),
                tokens,
            )
            .into_any_element(),
            assistant_text_row(tr(locale, "assistant_base_url"), &inputs.base_url, tokens),
            assistant_text_row(tr(locale, "assistant_model"), &inputs.model, tokens),
            setting_control_row(
                tr(locale, "assistant_auth_mode"),
                auth_buttons(root.assistant_auth_mode, locale, cx),
                tokens,
            )
            .into_any_element(),
            assistant_text_row(tr(locale, "assistant_api_key"), &inputs.api_key, tokens),
            assistant_text_row(
                tr(locale, "assistant_api_key_file"),
                &inputs.api_key_file,
                tokens,
            ),
            assistant_text_row(
                tr(locale, "assistant_context_window"),
                &inputs.context_window_tokens,
                tokens,
            ),
            assistant_text_row(tr(locale, "assistant_timeout"), &inputs.timeout_ms, tokens),
            assistant_text_row(
                tr(locale, "assistant_max_output"),
                &inputs.max_output_tokens,
                tokens,
            ),
            setting_control_row(
                tr(locale, "assistant_write_mode"),
                write_mode_buttons(root.settings_draft.assistant.write_mode, locale, cx),
                tokens,
            )
            .into_any_element(),
            assistant_text_row(
                tr(locale, "assistant_max_rounds"),
                &inputs.max_tool_rounds,
                tokens,
            ),
            assistant_text_row(
                tr(locale, "assistant_max_retries"),
                &inputs.max_validation_retries,
                tokens,
            ),
            setting_control_row(
                tr(locale, "assistant_connection_test"),
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("assistant-save-settings")
                            .primary()
                            .compact()
                            .label(tr(locale, "save"))
                            .on_click(cx.listener(RootView::save_assistant_settings)),
                    )
                    .child(
                        Button::new("assistant-readiness")
                            .secondary()
                            .compact()
                            .label(tr(locale, "assistant_readiness"))
                            .disabled(root.assistant.probe_running.is_some())
                            .on_click(cx.listener(RootView::probe_assistant_readiness)),
                    )
                    .child(
                        Button::new("assistant-model-probe")
                            .secondary()
                            .compact()
                            .label(tr(locale, "assistant_model_probe"))
                            .disabled(root.assistant.probe_running.is_some())
                            .on_click(cx.listener(RootView::probe_assistant_model)),
                    )
                    .children(root.assistant.probe_running.map(|kind| {
                        Button::new("assistant-cancel-probe")
                            .secondary()
                            .compact()
                            .label(format!(
                                "{} ({})",
                                tr(locale, "cancel"),
                                match kind {
                                    ConnectionProbeKind::Readiness => "readiness",
                                    ConnectionProbeKind::Model => "model",
                                }
                            ))
                            .on_click(cx.listener(RootView::cancel_assistant_probe))
                    }))
                    .into_any_element(),
                tokens,
            )
            .into_any_element(),
            diagnostic
                .map(|message| {
                    div()
                        .text_xs()
                        .text_color(tokens.text_muted)
                        .child(message)
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element()),
        ],
    )
    .into_any_element()
}

fn assistant_text_row(
    label: &'static str,
    input: &gpui::Entity<crate::text_input::TextInput>,
    tokens: &Tokens,
) -> gpui::AnyElement {
    setting_control_row(
        label,
        div()
            .w(px(360.))
            .min_h(px(34.))
            .px_3()
            .py_2()
            .rounded(tokens.radius.sm)
            .border_1()
            .border_color(tokens.border)
            .bg(tokens.surface)
            .child(input.clone())
            .into_any_element(),
        tokens,
    )
    .into_any_element()
}

fn protocol_buttons(
    selected: LlmProtocol,
    locale: crate::i18n::Locale,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    div()
        .flex()
        .flex_wrap()
        .gap_2()
        .children([
            protocol_button(
                LlmProtocol::OpenAiChatCompletions,
                "Chat Completions",
                selected,
                cx,
            ),
            protocol_button(
                LlmProtocol::AnthropicMessages,
                "Anthropic Messages",
                selected,
                cx,
            ),
            protocol_button(
                LlmProtocol::OpenAiResponses,
                "OpenAI Responses",
                selected,
                cx,
            ),
        ])
        .child(
            div()
                .text_xs()
                .text_color(theme::tokens(cx).text_muted)
                .child(tr(locale, "assistant_protocol_notice")),
        )
        .into_any_element()
}

fn protocol_button(
    protocol: LlmProtocol,
    label: &'static str,
    selected: LlmProtocol,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    let mut button = Button::new(format!("assistant-protocol-{label}"))
        .compact()
        .label(label);
    button = if protocol == selected {
        button.primary()
    } else {
        button.secondary()
    };
    button
        .on_click(cx.listener(move |root, _, _, cx| root.set_assistant_protocol(protocol, cx)))
        .into_any_element()
}

fn auth_buttons(
    selected: AssistantAuthMode,
    _locale: crate::i18n::Locale,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    div()
        .flex()
        .gap_2()
        .children([
            auth_button(AssistantAuthMode::Bearer, "Bearer", selected, cx),
            auth_button(AssistantAuthMode::XApiKey, "x-api-key", selected, cx),
        ])
        .into_any_element()
}

fn auth_button(
    auth: AssistantAuthMode,
    label: &'static str,
    selected: AssistantAuthMode,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    let mut button = Button::new(format!("assistant-auth-{label}"))
        .compact()
        .label(label);
    button = if auth == selected {
        button.primary()
    } else {
        button.secondary()
    };
    button
        .on_click(cx.listener(move |root, _, _, cx| root.set_assistant_auth_mode(auth, cx)))
        .into_any_element()
}

fn write_mode_buttons(
    selected: AgentWriteMode,
    locale: crate::i18n::Locale,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    div()
        .flex()
        .gap_2()
        .children([
            write_mode_button(
                AgentWriteMode::Automatic,
                tr(locale, "assistant_automatic"),
                selected,
                cx,
            ),
            write_mode_button(
                AgentWriteMode::ConfirmEach,
                tr(locale, "assistant_confirm_each"),
                selected,
                cx,
            ),
        ])
        .into_any_element()
}

fn write_mode_button(
    mode: AgentWriteMode,
    label: &'static str,
    selected: AgentWriteMode,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    let mut button = Button::new(format!("assistant-write-{label}"))
        .compact()
        .label(label);
    button = if mode == selected {
        button.primary()
    } else {
        button.secondary()
    };
    button
        .on_click(cx.listener(move |root, _, _, cx| root.set_assistant_write_mode(mode, cx)))
        .into_any_element()
}

fn choice_buttons<F>(
    choices: [(&'static str, &'static str, bool); 2],
    selected: bool,
    cx: &mut Context<RootView>,
    action: F,
) -> gpui::AnyElement
where
    F: Fn(&mut RootView, bool, &mut Context<RootView>) + Clone + 'static,
{
    div()
        .flex()
        .gap_2()
        .children(choices.map(|(id, label, value)| {
            let action = action.clone();
            let mut button = Button::new(id).compact().label(label);
            button = if value == selected {
                button.primary()
            } else {
                button.secondary()
            };
            button
                .on_click(cx.listener(move |root, _, _, cx| action(root, value, cx)))
                .into_any_element()
        }))
        .into_any_element()
}

fn capability(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown",
    }
}

fn select_control(
    state: &gpui::Entity<gpui_component::select::SelectState<Vec<String>>>,
    id: &'static str,
) -> gpui::AnyElement {
    // The wrapper takes the full size, as `Select` does, so it only adds the
    // test selector and leaves the row layout unchanged.
    div()
        .size_full()
        .debug_selector(|| format!("select-{id}"))
        .child(Select::new(state).w(px(280.)).menu_width(px(280.)))
        .into_any_element()
}

fn model_select_control(
    state: &gpui::Entity<gpui_component::select::SelectState<Vec<ModelOption>>>,
    _id: &'static str,
) -> gpui::AnyElement {
    Select::new(state)
        .w(px(280.))
        .menu_width(px(280.))
        .into_any_element()
}

fn stage_model_select_control(
    state: &gpui::Entity<
        gpui_component::select::SelectState<Vec<crate::controller::StageModelOption>>,
    >,
    _id: &'static str,
) -> gpui::AnyElement {
    Select::new(state)
        .w(px(280.))
        .menu_width(px(280.))
        .into_any_element()
}
