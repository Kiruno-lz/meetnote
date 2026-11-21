use rune::{El, TextEdit};
use rune_kit::{Button, Kit, Switch, TextField};

use super::section;
use crate::MeetNote;
use crate::workspace_host::assistant::BehaviourDraft;

/// How the built-in Assistant behaves; providers and its model are chosen
/// on the Providers page.
pub(super) fn page(s: &MeetNote, k: &Kit) -> El {
    let draft = &s.workspace_host.assistant.behaviour;
    let busy = s.workspace_host.busy();
    let form = section(
        k,
        s.tr("assistant_settings"),
        s.tr("assistant_behaviour_description"),
    )
    .child(
        Switch::new(draft.enabled)
            .label(s.tr("assistant_enabled"))
            .on_toggle(|s: &mut MeetNote| {
                let draft = &mut s.workspace_host.assistant.behaviour;
                draft.enabled = !draft.enabled;
            })
            .build(k)
            .key("assistant-enabled"),
    )
    .child(
        Switch::new(draft.confirm_writes)
            .label(s.tr("assistant_confirm_each"))
            .on_toggle(|s: &mut MeetNote| {
                let draft = &mut s.workspace_host.assistant.behaviour;
                draft.confirm_writes = !draft.confirm_writes;
            })
            .build(k)
            .key("assistant-confirm-each"),
    )
    .child(limit(
        k,
        ("assistant-tool-rounds", s.tr("assistant_max_rounds")),
        &draft.tool_rounds,
        |draft| &mut draft.tool_rounds,
    ))
    .child(limit(
        k,
        (
            "assistant-validation-retries",
            s.tr("assistant_max_retries"),
        ),
        &draft.validation_retries,
        |draft| &mut draft.validation_retries,
    ))
    .child(
        Button::new(s.tr("save_settings"))
            .disabled(busy)
            .on_press_tx(MeetNote::save_assistant_behaviour)
            .build(k)
            .key("save-assistant-settings"),
    );
    if busy { form.inert() } else { form }
}

fn limit(
    k: &Kit,
    (key, label): (&str, &str),
    value: &TextEdit,
    edit: fn(&mut BehaviourDraft) -> &mut TextEdit,
) -> El {
    TextField::new(label, value)
        .on_edit(move |s: &mut MeetNote| edit(&mut s.workspace_host.assistant.behaviour))
        .build(k)
        .key(key)
}
