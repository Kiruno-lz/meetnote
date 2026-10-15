use crate::{
    assistant_ui::ConnectionProbeKind,
    components::{
        setting_control_row, settings_section, SETTING_CONTROL_WIDTH, SETTING_LABEL_WIDTH,
    },
    controller::{settings_error_text, RootView},
    controls::{Button, Dropdown, RadioGroup, Region, Scrollbar, TextField},
    i18n::text as tr,
    pages::meetings::marked_control,
    speakers::profile_summary,
    theme::{self, Tokens},
};
use contracts::AgentWriteMode;
use gpui::{div, prelude::*, px, Context, IntoElement, Pixels};
use settings::{AssistantAuthMode, LlmProtocol};

pub(crate) const SECTIONS: [&str; 8] = [
    "appearance",
    "workspace",
    "interface",
    "transcription",
    "speaker_separation",
    "speaker_library",
    "runtime",
    "assistant_settings",
];

pub(crate) const SECTION_ICONS: [&str; 8] = [
    "icons/palette.svg",
    "icons/folder.svg",
    "icons/globe.svg",
    "icons/file.svg",
    "icons/user.svg",
    "icons/book-open.svg",
    "icons/cpu.svg",
    "icons/bot.svg",
];

pub(crate) fn render(root: &mut RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let tokens = *theme::tokens(cx);
    let assistant = assistant_settings_section(root, &tokens, cx);
    let workspace = workspace_section(root, &tokens, cx);
    let Some(selectors) = root.settings_selectors.as_ref() else {
        return div().flex_1().into_any_element();
    };

    let scroll = root.scroll.settings.clone();
    let body = div()
        .id("settings-scroll")
        .debug_selector(|| "settings-scroll".into())
        .size_full()
        .overflow_y_scroll()
        .track_scroll(&scroll)
        .flex()
        .flex_col()
        .gap(tokens.spacing.xl)
        .max_w(px(960.))
        .mx_auto()
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
        .child(workspace)
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
                    select_control(&selectors.asr_model, "asr-model"),
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
                    select_control(&selectors.diarization_model, "diarization-model"),
                    &tokens,
                )
                .into_any_element(),
                if root.show_diarization_stage_options() {
                    setting_control_row(
                        tr(root.ui.locale, "diarization_segmentation_model"),
                        select_control(
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
                        select_control(
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
            ],
        ))
        .child(speaker_library_section(root, &tokens, cx))
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
        .min_w_0()
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
        .children(root.settings_error.as_ref().map(|error| {
            div()
                .id("settings-error")
                .debug_selector(|| "settings-error".into())
                .flex_shrink_0()
                .text_sm()
                .child(settings_error_text(root.ui.locale, error))
        }))
        // The scrollbar stays outside the scrolled form.
        .child(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .child(body)
                .child(Scrollbar::vertical("settings-scrollbar", &scroll)),
        )
        .into_any_element()
}

/// The position of the Workspace section among the form's sections.
pub(crate) const WORKSPACE_SECTION: usize = 1;
/// The position of the Speaker Library section among the form's sections.
pub(crate) const SPEAKER_LIBRARY_SECTION: usize = 5;
/// The position of the Assistant section among the form's sections.
#[cfg(test)]
pub(crate) const ASSISTANT_SECTION: usize = 7;
/// The width of a path shown beside its choose button.
const PATH_WIDTH: Pixels = px(280.);

/// The Workspace: the documents root the workspace reads and writes, and a
/// button that chooses another folder. The switch runs on a worker; while
/// it runs the button is disabled, and its outcome shows under the path.
fn workspace_section(
    root: &RootView,
    tokens: &Tokens,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    let locale = root.ui.locale;
    let switch = &root.documents_root_switch;
    let mut rows = vec![setting_control_row(
        tr(locale, "documents_root"),
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .debug_selector(|| "documents-root-path".into())
                    .w(PATH_WIDTH)
                    .min_w_0()
                    .flex_shrink(1.)
                    .text_xs()
                    .text_color(tokens.text_muted)
                    .child(root.settings_draft.documents_root.clone()),
            )
            .child(
                Button::new("choose-documents-root", tr(locale, "choose_directory"))
                    .debug_selector(|| "choose-documents-root".into())
                    .secondary()
                    .compact()
                    .disabled(switch.switching || root.documents_root_busy())
                    .on_click(cx.listener(RootView::choose_documents_root_directory)),
            )
            .into_any_element(),
        tokens,
    )
    .into_any_element()];
    let status = if switch.switching {
        Some((tr(locale, "documents_root_switching").to_string(), false))
    } else if root.documents_root_busy() {
        Some((
            tr(locale, "documents_root_operation_pending").to_string(),
            false,
        ))
    } else {
        switch.status.clone()
    };
    if let Some((message, failed)) = status {
        rows.push(
            div()
                .id("documents-root-status")
                .debug_selector(|| "documents-root-status".into())
                .mt(tokens.spacing.sm)
                .text_sm()
                .text_color(if failed {
                    tokens.warning_text
                } else {
                    tokens.text_muted
                })
                .child(message)
                .into_any_element(),
        );
    }
    div()
        .flex_shrink_0()
        .debug_selector(|| "workspace-section".into())
        .child(settings_section(tr(locale, "workspace"), rows))
        .into_any_element()
}

/// The width of a profile's name field and merge select.
const PROFILE_CONTROL_WIDTH: Pixels = px(200.);

/// The Speaker Library: its path, then every profile with a name field and
/// Rename, a select that merges it into another profile and Delete; merging
/// and deleting ask first. Library calls run on workers; their outcome
/// shows under the path.
fn speaker_library_section(
    root: &RootView,
    tokens: &Tokens,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    let locale = root.ui.locale;
    let library = &root.speaker_profiles;
    let mut rows = vec![setting_control_row(
        tr(locale, "speaker_library_path"),
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .w(PATH_WIDTH)
                    .min_w_0()
                    .flex_shrink(1.)
                    .text_xs()
                    .text_color(tokens.text_muted)
                    .child(root.settings_draft.speaker_library_path.clone()),
            )
            .child(
                Button::new("choose-speaker-library", tr(locale, "choose_directory"))
                    .debug_selector(|| "choose-speaker-library".into())
                    .secondary()
                    .compact()
                    .on_click(cx.listener(RootView::choose_speaker_library_directory)),
            )
            .into_any_element(),
        tokens,
    )
    .into_any_element()];
    let outcome = library
        .read_error
        .clone()
        .map(|error| (error, true))
        .or_else(|| library.status.clone());
    if let Some((message, failed)) = outcome {
        rows.push(
            div()
                .id("speaker-library-status")
                .debug_selector(|| "speaker-library-status".into())
                .mt(tokens.spacing.sm)
                .text_sm()
                .text_color(if failed {
                    tokens.warning_text
                } else {
                    tokens.text_muted
                })
                .child(message)
                .into_any_element(),
        );
    }
    let empty_key = if !library.loaded {
        Some("speaker_library_loading")
    } else if library.profiles.is_empty() && library.read_error.is_none() {
        Some("speaker_library_empty")
    } else {
        None
    };
    if let Some(key) = empty_key {
        rows.push(
            div()
                .mt(tokens.spacing.sm)
                .text_sm()
                .text_color(tokens.text_muted)
                .child(tr(locale, key))
                .into_any_element(),
        );
    }
    let controls = root.profile_rows.as_ref();
    for profile in &library.profiles {
        let Some(row) = controls.and_then(|rows| rows.row(&profile.id)) else {
            continue;
        };
        let id = profile.id.clone();
        let (rename_id, delete_id) = (id.clone(), id.clone());
        let marked = |name: &str| {
            let selector = format!("{name}-{id}");
            move || selector.clone()
        };
        rows.push(
            div()
                .debug_selector(marked("speaker-profile"))
                .mt(tokens.spacing.sm)
                .min_h(tokens.control_height.lg)
                .flex()
                .items_center()
                .gap(tokens.spacing.lg)
                .text_sm()
                .child(
                    div()
                        .w(SETTING_LABEL_WIDTH)
                        .flex_shrink_0()
                        .flex()
                        .flex_col()
                        .child(theme::tail_faded_label(
                            format!("speaker-profile-label-{id}"),
                            profile.display_name.clone(),
                            tokens.content,
                        ))
                        .child(
                            div()
                                .text_xs()
                                .text_color(tokens.text_muted)
                                .child(profile_summary(profile, locale)),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(marked_control(
                            format!("speaker-profile-name-{id}"),
                            PROFILE_CONTROL_WIDTH,
                            row.name.clone().into_any_element(),
                        ))
                        .child(
                            Button::new(
                                format!("rename-speaker-profile-{id}"),
                                tr(locale, "rename"),
                            )
                            .debug_selector(marked("rename-speaker-profile"))
                            .secondary()
                            .compact()
                            .disabled(library.busy)
                            .on_click(cx.listener(
                                move |root, _, _, cx| {
                                    root.rename_speaker_profile(rename_id.clone(), cx)
                                },
                            )),
                        )
                        .children(row.merge_into.as_ref().map(|select| {
                            marked_control(
                                format!("merge-speaker-profile-{id}"),
                                PROFILE_CONTROL_WIDTH,
                                select.clone().into_any_element(),
                            )
                        }))
                        .child(
                            Button::new(
                                format!("delete-speaker-profile-{id}"),
                                tr(locale, "delete"),
                            )
                            .debug_selector(marked("delete-speaker-profile"))
                            .danger()
                            .compact()
                            .disabled(library.busy)
                            .on_click(cx.listener(
                                move |root, _, window, cx| {
                                    root.ask_delete_speaker_profile(delete_id.clone(), window, cx)
                                },
                            )),
                        ),
                )
                .into_any_element(),
        );
    }
    div()
        .flex_shrink_0()
        .debug_selector(|| "speaker-library-section".into())
        .child(
            div()
                .pb_4()
                .flex_shrink_0()
                .child(
                    Region::new(
                        "speaker-library-title",
                        tr(locale, "speaker_library"),
                        &root.speaker_library_focus,
                    )
                    .debug_selector(|| "speaker-library-title".into())
                    .mb_1()
                    .text_lg()
                    .child(tr(locale, "speaker_library")),
                )
                .child(
                    div()
                        .border_1()
                        .border_color(tokens.hairline)
                        .rounded(tokens.radius.xl)
                        .px(tokens.spacing.lg)
                        .py(tokens.spacing.sm)
                        .children(rows),
                ),
        )
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
            let selector = format!("assistant-profile-{id}");
            (
                selected,
                Button::new(selector.clone(), profile.name.clone())
                    .debug_selector(move || selector.clone())
                    .compact()
                    .on_click(
                        cx.listener(move |root, _, _, cx| root.edit_assistant_profile(&id, cx)),
                    ),
            )
        })
        .collect::<Vec<_>>();
    let probe = probe_outcome(root, tokens, cx);
    let write_mode = root.settings_draft.assistant.write_mode;
    let write_mode_notice = write_mode_notice(write_mode, root.saved_write_mode(), locale, tokens);
    let credential = credential_caption(root, tokens, cx);
    let inputs = &root.assistant_inputs;

    settings_section(
        tr(locale, "assistant_settings"),
        vec![
            setting_control_row(
                tr(locale, "assistant_enabled"),
                choice_buttons(
                    tr(locale, "assistant_enabled"),
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
                    .child(
                        RadioGroup::new("assistant-profiles", tr(locale, "assistant_profiles"))
                            .debug_selector(|| "assistant-profiles-group".into())
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .options(profiles),
                    )
                    .child(
                        Button::new("assistant-new-profile", tr(locale, "assistant_new_profile"))
                            .secondary()
                            .compact()
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
            marked_text_row(
                tr(locale, "assistant_api_key"),
                &inputs.api_key,
                "assistant-api-key-field",
                tokens,
            ),
            credential,
            marked_text_row(
                tr(locale, "assistant_api_key_file"),
                &inputs.api_key_file,
                "assistant-api-key-file-field",
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
                write_mode_buttons(write_mode, locale, cx),
                tokens,
            )
            .into_any_element(),
            write_mode_notice,
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
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new("assistant-save-settings", tr(locale, "save"))
                            .debug_selector(|| "assistant-save-settings".into())
                            .primary()
                            .compact()
                            .on_click(cx.listener(RootView::save_assistant_settings)),
                    )
                    .child(
                        Button::new("assistant-readiness", tr(locale, "assistant_readiness"))
                            .debug_selector(|| "assistant-readiness".into())
                            .secondary()
                            .compact()
                            .disabled(root.assistant.probe_running.is_some())
                            .on_click(cx.listener(RootView::probe_assistant_readiness)),
                    )
                    .child(
                        Button::new("assistant-model-probe", tr(locale, "assistant_model_probe"))
                            .debug_selector(|| "assistant-model-probe".into())
                            .secondary()
                            .compact()
                            .disabled(root.assistant.probe_running.is_some())
                            .on_click(cx.listener(RootView::probe_assistant_model)),
                    )
                    .children(root.assistant.probe_running.map(|kind| {
                        let probe = match kind {
                            ConnectionProbeKind::Readiness => "assistant_probe_readiness",
                            ConnectionProbeKind::Model => "assistant_probe_model",
                        };
                        let label = format!("{} ({})", tr(locale, "cancel"), tr(locale, probe));
                        Button::new("assistant-cancel-probe", label)
                            .debug_selector(|| "assistant-cancel-probe".into())
                            .secondary()
                            .compact()
                            .on_click(cx.listener(RootView::cancel_assistant_probe))
                    }))
                    .into_any_element(),
                tokens,
            )
            .into_any_element(),
            probe,
        ],
    )
    .into_any_element()
}

/// A row whose control is `field`, which carries the row's label.
fn assistant_text_row(
    label: &'static str,
    field: &gpui::Entity<TextField>,
    tokens: &Tokens,
) -> gpui::AnyElement {
    setting_control_row(
        label,
        div()
            .w_full()
            .max_w(SETTING_CONTROL_WIDTH)
            .child(field.clone())
            .into_any_element(),
        tokens,
    )
    .into_any_element()
}

/// A text row whose control is marked `selector` for tests.
fn marked_text_row(
    label: &'static str,
    field: &gpui::Entity<TextField>,
    selector: &'static str,
    tokens: &Tokens,
) -> gpui::AnyElement {
    setting_control_row(
        label,
        div()
            .debug_selector(move || selector.into())
            .w_full()
            .max_w(SETTING_CONTROL_WIDTH)
            .child(field.clone())
            .into_any_element(),
        tokens,
    )
    .into_any_element()
}

/// A line under a settings row, in the control column: muted, or in the
/// warning color when it reports a failure.
fn caption(
    selector: &'static str,
    message: impl Into<gpui::SharedString>,
    failed: bool,
    tokens: &Tokens,
) -> gpui::Div {
    div()
        .debug_selector(move || selector.into())
        .mt(tokens.spacing.xs)
        .ml(SETTING_LABEL_WIDTH + tokens.spacing.lg)
        .max_w(SETTING_CONTROL_WIDTH + px(160.))
        .text_xs()
        .text_color(if failed {
            tokens.warning_text
        } else {
            tokens.text_muted
        })
        .child(message.into())
}

/// Under the key field: why a save that changed the connection target was
/// refused, or that an empty field keeps the stored key.
fn credential_caption(
    root: &RootView,
    tokens: &Tokens,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    let locale = root.ui.locale;
    if root
        .settings_error
        .as_ref()
        .is_some_and(|error| error.is_credential_target_conflict())
    {
        caption(
            "assistant-credential-target-conflict",
            tr(locale, "assistant_credential_target_conflict"),
            true,
            tokens,
        )
        .into_any_element()
    } else if root.form_keeps_stored_credential(cx) {
        caption(
            "assistant-api-key-kept",
            tr(locale, "assistant_api_key_kept"),
            false,
            tokens,
        )
        .into_any_element()
    } else {
        div().into_any_element()
    }
}

/// What the chosen write mode means, and that it waits for Save while it
/// differs from the saved one.
fn write_mode_notice(
    draft: AgentWriteMode,
    saved: AgentWriteMode,
    locale: crate::i18n::Locale,
    tokens: &Tokens,
) -> gpui::AnyElement {
    let (selector, key) = match draft {
        AgentWriteMode::Automatic => (
            "settings-write-mode-notice-automatic",
            "assistant_write_automatic_notice",
        ),
        AgentWriteMode::ConfirmEach => (
            "settings-write-mode-notice-confirm-each",
            "assistant_write_confirm_each_notice",
        ),
    };
    div()
        .child(caption(selector, tr(locale, key), false, tokens))
        .children((draft != saved).then(|| {
            caption(
                "settings-write-mode-unsaved",
                tr(locale, "assistant_write_mode_unsaved"),
                true,
                tokens,
            )
        }))
        .into_any_element()
}

/// The last connection probe: running, cancelled, its diagnostic, or its
/// error with a retry when the error is retryable.
fn probe_outcome(root: &RootView, tokens: &Tokens, cx: &mut Context<RootView>) -> gpui::AnyElement {
    let locale = root.ui.locale;
    let assistant = &root.assistant;
    if assistant.probe_running.is_some() {
        return caption(
            "assistant-probe-running",
            tr(locale, "assistant_probe_running"),
            false,
            tokens,
        )
        .into_any_element();
    }
    if assistant.probe_cancelled {
        return caption(
            "assistant-probe-cancelled",
            tr(locale, "assistant_probe_cancelled"),
            false,
            tokens,
        )
        .into_any_element();
    }
    match &assistant.diagnostic {
        Some(Ok(diagnostic)) => caption(
            "assistant-probe-result",
            format!(
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
                tr(locale, capability_key(diagnostic.text)),
                tr(locale, capability_key(diagnostic.client_tools)),
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
            false,
            tokens,
        )
        .into_any_element(),
        Some(Err(error)) => div()
            .flex()
            .items_center()
            .gap_2()
            .child(caption(
                "assistant-probe-error",
                crate::i18n::message(locale, "assistant_probe_failed", &error.to_string()),
                true,
                tokens,
            ))
            .children(assistant.probe_to_retry().map(|_| {
                Button::new("assistant-probe-retry", tr(locale, "assistant_retry"))
                    .debug_selector(|| "assistant-probe-retry".into())
                    .secondary()
                    .compact()
                    .on_click(cx.listener(RootView::retry_assistant_probe))
            }))
            .into_any_element(),
        None => div().into_any_element(),
    }
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
        .child(
            RadioGroup::new("assistant-protocol", tr(locale, "assistant_protocol"))
                .debug_selector(|| "assistant-protocol-group".into())
                .flex()
                .flex_wrap()
                .gap_2()
                .options(
                    [
                        (LlmProtocol::OpenAiChatCompletions, "Chat Completions"),
                        (LlmProtocol::AnthropicMessages, "Anthropic Messages"),
                        (LlmProtocol::OpenAiResponses, "OpenAI Responses"),
                    ]
                    .map(|(protocol, label)| {
                        (protocol == selected, protocol_button(protocol, label, cx))
                    }),
                ),
        )
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
    cx: &mut Context<RootView>,
) -> Button {
    Button::new(format!("assistant-protocol-{label}"), label)
        .compact()
        .on_click(cx.listener(move |root, _, _, cx| root.set_assistant_protocol(protocol, cx)))
}

fn auth_buttons(
    selected: AssistantAuthMode,
    locale: crate::i18n::Locale,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    RadioGroup::new("assistant-auth", tr(locale, "assistant_auth_mode"))
        .debug_selector(|| "assistant-auth-group".into())
        .flex()
        .gap_2()
        .options(
            [
                (AssistantAuthMode::Bearer, "Bearer"),
                (AssistantAuthMode::XApiKey, "x-api-key"),
            ]
            .map(|(auth, label)| (auth == selected, auth_button(auth, label, cx))),
        )
        .into_any_element()
}

fn auth_button(auth: AssistantAuthMode, label: &'static str, cx: &mut Context<RootView>) -> Button {
    Button::new(format!("assistant-auth-{label}"), label)
        .compact()
        .on_click(cx.listener(move |root, _, _, cx| root.set_assistant_auth_mode(auth, cx)))
}

fn write_mode_buttons(
    selected: AgentWriteMode,
    locale: crate::i18n::Locale,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    RadioGroup::new("assistant-write-mode", tr(locale, "assistant_write_mode"))
        .debug_selector(|| "assistant-write-mode-group".into())
        .flex()
        .gap_2()
        .options(
            [
                (
                    AgentWriteMode::Automatic,
                    tr(locale, "assistant_automatic"),
                    "settings-write-mode-automatic",
                ),
                (
                    AgentWriteMode::ConfirmEach,
                    tr(locale, "assistant_confirm_each"),
                    "settings-write-mode-confirm-each",
                ),
            ]
            .map(|(mode, label, selector)| {
                (
                    mode == selected,
                    write_mode_button(mode, label, selector, cx),
                )
            }),
        )
        .into_any_element()
}

fn write_mode_button(
    mode: AgentWriteMode,
    label: &'static str,
    selector: &'static str,
    cx: &mut Context<RootView>,
) -> Button {
    Button::new(format!("assistant-write-{label}"), label)
        .debug_selector(move || selector.into())
        .compact()
        .on_click(cx.listener(move |root, _, _, cx| root.set_assistant_write_mode(mode, cx)))
}

fn choice_buttons<F>(
    label: &'static str,
    choices: [(&'static str, &'static str, bool); 2],
    selected: bool,
    cx: &mut Context<RootView>,
    action: F,
) -> gpui::AnyElement
where
    F: Fn(&mut RootView, bool, &mut Context<RootView>) + Clone + 'static,
{
    RadioGroup::new("assistant-enabled-group", label)
        .flex()
        .gap_2()
        .options(choices.map(|(id, label, value)| {
            let action = action.clone();
            (
                value == selected,
                Button::new(id, label)
                    .compact()
                    .on_click(cx.listener(move |root, _, _, cx| action(root, value, cx))),
            )
        }))
        .into_any_element()
}

fn capability_key(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "capability_yes",
        Some(false) => "capability_no",
        None => "capability_unknown",
    }
}

/// The select column of a settings row: `select-<id>` marks it for tests.
fn select_control(dropdown: &gpui::Entity<Dropdown>, id: &'static str) -> gpui::AnyElement {
    div()
        .w_full()
        .max_w(SETTING_CONTROL_WIDTH)
        .debug_selector(move || format!("select-{id}"))
        .child(dropdown.clone())
        .into_any_element()
}
