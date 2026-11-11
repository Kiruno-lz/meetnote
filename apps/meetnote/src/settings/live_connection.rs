use ::settings::{AssistantAuthMode, LlmProtocol};
use application::desktop_assistant::ConnectionProbeKind;
use rune::{El, TextEdit, row};
use rune_kit::{Button, ButtonVariant, Kit, Switch, TextField, TextInput};

use super::section;
use crate::MeetNote;
use crate::controls::{caption, field, label, select};
use crate::workspace_host::assistant_connection::ConnectionDraft;

const PROTOCOLS: [LlmProtocol; 3] = [
    LlmProtocol::OpenAiChatCompletions,
    LlmProtocol::AnthropicMessages,
    LlmProtocol::OpenAiResponses,
];
const AUTH: [AssistantAuthMode; 2] = [AssistantAuthMode::Bearer, AssistantAuthMode::XApiKey];

pub(super) fn page(s: &MeetNote, k: &Kit, overlays: &mut Vec<El>) -> El {
    let host = &s.workspace_host.assistant;
    let draft = host
        .connection
        .as_ref()
        .expect("live workspace loads the Assistant connection");
    let profiles = &host.settings.connections;
    let mut labels: Vec<_> = profiles
        .iter()
        .map(|profile| format!("{} · {}", profile.name, profile.id))
        .collect();
    labels.push(s.tr("assistant_new_profile").into());
    let labels: Vec<_> = labels.iter().map(String::as_str).collect();
    let active = draft
        .profile_id
        .as_ref()
        .and_then(|id| profiles.iter().position(|profile| &profile.id == id))
        .unwrap_or(profiles.len());
    let form = section(
        k,
        s.tr("assistant_connection"),
        s.tr("assistant_connection_description"),
    )
    .child(field(
        k,
        s.tr("assistant_profiles"),
        select(
            s,
            k,
            overlays,
            ("live-connection-profile", s.tr("assistant_profiles")),
            &labels,
            active,
            |s, index| {
                let host = &mut s.workspace_host.assistant;
                host.connection = Some(ConnectionDraft::new(
                    &host.settings,
                    host.settings.connections.get(index),
                ));
            },
        ),
    ))
    .child(
        TextField::new(s.tr("assistant_connection_name"), &draft.name)
            .on_edit(|s: &mut MeetNote| {
                &mut s.workspace_host.assistant.connection.as_mut().unwrap().name
            })
            .build(k)
            .key("live-connection-name"),
    )
    .child(
        TextField::new(s.tr("assistant_endpoint"), &draft.endpoint)
            .on_edit(|s: &mut MeetNote| {
                &mut s
                    .workspace_host
                    .assistant
                    .connection
                    .as_mut()
                    .unwrap()
                    .endpoint
            })
            .build(k)
            .key("live-connection-endpoint"),
    )
    .child(
        TextField::new(s.tr("assistant_model"), &draft.model)
            .on_edit(|s: &mut MeetNote| {
                &mut s
                    .workspace_host
                    .assistant
                    .connection
                    .as_mut()
                    .unwrap()
                    .model
            })
            .build(k)
            .key("live-connection-model"),
    )
    .child(field(
        k,
        s.tr("assistant_protocol"),
        select(
            s,
            k,
            overlays,
            ("live-connection-protocol", s.tr("assistant_protocol")),
            &["Chat Completions", "Anthropic Messages", "OpenAI Responses"],
            PROTOCOLS
                .iter()
                .position(|protocol| *protocol == draft.protocol)
                .unwrap_or_default(),
            |s, index| {
                s.workspace_host
                    .assistant
                    .connection
                    .as_mut()
                    .unwrap()
                    .protocol = PROTOCOLS[index]
            },
        ),
    ))
    .child(field(
        k,
        s.tr("assistant_auth_mode"),
        select(
            s,
            k,
            overlays,
            ("live-connection-auth", s.tr("assistant_auth_mode")),
            &["Bearer token", "X-API-Key"],
            usize::from(draft.auth == AssistantAuthMode::XApiKey),
            |s, index| s.workspace_host.assistant.connection.as_mut().unwrap().auth = AUTH[index],
        ),
    ))
    .child(field(
        k,
        s.tr("assistant_credential"),
        select(
            s,
            k,
            overlays,
            ("live-connection-credential", s.tr("assistant_credential")),
            &[
                s.tr("assistant_keep_credential"),
                s.tr("assistant_enter_key"),
                s.tr("assistant_use_key_file"),
                s.tr("assistant_clear_credential"),
            ],
            draft.credential_mode,
            |s, index| {
                let draft = s.workspace_host.assistant.connection.as_mut().unwrap();
                draft.credential_mode = index;
                draft.credential.clear();
            },
        ),
    ))
    .children((draft.credential_mode == 1).then(|| {
        TextInput::new(&draft.credential)
            .label(s.tr("assistant_api_key"))
            .secure()
            .on_edit(|s: &mut MeetNote| {
                &mut s
                    .workspace_host
                    .assistant
                    .connection
                    .as_mut()
                    .unwrap()
                    .credential
            })
            .build(k)
            .key("live-connection-key")
    }))
    .children((draft.credential_mode == 2).then(|| {
        TextField::new(s.tr("assistant_key_file_path"), &draft.credential)
            .on_edit(|s: &mut MeetNote| {
                &mut s
                    .workspace_host
                    .assistant
                    .connection
                    .as_mut()
                    .unwrap()
                    .credential
            })
            .build(k)
    }))
    .child(
        Switch::new(draft.enabled)
            .label(s.tr("assistant_enabled"))
            .on_toggle(|s: &mut MeetNote| {
                let draft = s.workspace_host.assistant.connection.as_mut().unwrap();
                draft.enabled = !draft.enabled;
            })
            .build(k),
    )
    .child(
        Switch::new(draft.confirm_writes)
            .label(s.tr("assistant_confirm_each"))
            .on_toggle(|s: &mut MeetNote| {
                let draft = s.workspace_host.assistant.connection.as_mut().unwrap();
                draft.confirm_writes = !draft.confirm_writes;
            })
            .build(k),
    )
    .child(limit(
        k,
        (
            "live-connection-context-window",
            s.tr("assistant_context_window"),
        ),
        &draft.context_window,
        |draft| &mut draft.context_window,
    ))
    .child(limit(
        k,
        ("live-connection-timeout", s.tr("assistant_timeout")),
        &draft.timeout,
        |draft| &mut draft.timeout,
    ))
    .child(limit(
        k,
        ("live-connection-output-limit", s.tr("assistant_max_output")),
        &draft.output_limit,
        |draft| &mut draft.output_limit,
    ))
    .child(limit(
        k,
        ("live-connection-tool-rounds", s.tr("assistant_max_rounds")),
        &draft.tool_rounds,
        |draft| &mut draft.tool_rounds,
    ))
    .child(limit(
        k,
        (
            "live-connection-validation-retries",
            s.tr("assistant_max_retries"),
        ),
        &draft.validation_retries,
        |draft| &mut draft.validation_retries,
    ));
    let busy = s.workspace_host.busy();
    let mut controls = row()
        .gap(8.0)
        .flex_wrap()
        .child(
            Button::new(s.tr("save"))
                .disabled(busy)
                .on_press_tx(MeetNote::save_assistant_connection)
                .build(k)
                .key("save-live-connection"),
        )
        .child(
            Button::new(s.tr("assistant_reload_connection"))
                .variant(ButtonVariant::Outline)
                .disabled(busy)
                .on_press_tx(MeetNote::reload_assistant_connection)
                .build(k),
        );
    if host.controller.probe_running.is_some() {
        controls = controls.child(
            Button::new(s.tr("assistant_cancel_probe"))
                .variant(ButtonVariant::Outline)
                .on_press(|s: &mut MeetNote| {
                    s.workspace_host.assistant.controller.request_probe_cancel();
                })
                .build(k),
        );
    } else if host.settings.active_connection_id.is_some() {
        controls = controls
            .child(
                Button::new(s.tr("assistant_readiness"))
                    .variant(ButtonVariant::Outline)
                    .disabled(busy)
                    .on_press_tx(|s: &mut MeetNote, tx| {
                        s.probe_assistant_connection(ConnectionProbeKind::Readiness, tx)
                    })
                    .build(k),
            )
            .child(
                Button::new(s.tr("assistant_model_probe"))
                    .variant(ButtonVariant::Outline)
                    .disabled(busy)
                    .on_press_tx(|s: &mut MeetNote, tx| {
                        s.probe_assistant_connection(ConnectionProbeKind::Model, tx)
                    })
                    .build(k),
            );
    }
    let form = if busy { form.inert() } else { form };
    form.child(controls).children(
        host.controller
            .diagnostic
            .as_ref()
            .map(|result| match result {
                Ok(diagnostic) => caption(k, &diagnostic.message),
                Err(error) => label(k, error.to_string()),
            }),
    )
}

fn limit(
    k: &Kit,
    (key, label): (&str, &str),
    value: &TextEdit,
    edit: fn(&mut ConnectionDraft) -> &mut TextEdit,
) -> El {
    TextField::new(label, value)
        .on_edit(move |s: &mut MeetNote| {
            edit(s.workspace_host.assistant.connection.as_mut().unwrap())
        })
        .build(k)
        .key(key)
}
