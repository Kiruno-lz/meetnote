use rune::{El, col, row};
use rune_kit::{Button, ButtonVariant, Kit, RadioGroup, RadioItem, TextField};

use crate::controls::{caption, label};
use crate::workspace_host::speaker_library::LibraryChange;
use crate::{Confirm, MeetNote};

pub(super) fn page(s: &MeetNote, k: &Kit) -> El {
    let library = &s.workspace_host.speaker_library;
    let busy = s.workspace_host.busy() || library.busy();
    let path = &s
        .workspace_host
        .preferences
        .draft
        .as_ref()
        .unwrap()
        .speaker_library;
    let mut section = super::section(k, s.tr("speaker_library"), s.tr("speaker_library_manage"))
        .child(caption(k, path).key("speaker-library-path"))
        .child(
            row()
                .gap(8.0)
                .flex_wrap()
                .child(
                    Button::new(s.tr("choose_directory"))
                        .variant(ButtonVariant::Outline)
                        .disabled(s.workspace_host.busy())
                        .on_press_tx(MeetNote::choose_speaker_library)
                        .build(k)
                        .key("choose-speaker-library"),
                )
                .child(
                    Button::new(s.tr("reload"))
                        .variant(ButtonVariant::Outline)
                        .disabled(busy)
                        .on_press_tx(MeetNote::reload_speaker_profiles)
                        .build(k)
                        .key("library-reload"),
                ),
        );
    if let Some(error) = &library.error {
        section = section.child(label(k, error).role("alert"));
    } else if library.rows.is_empty() {
        section = section.child(caption(
            k,
            if library.busy() {
                s.tr("speaker_library_loading")
            } else {
                s.tr("speaker_library_empty")
            },
        ));
    }
    section.children(library.rows.iter().map(|profile| {
        let edit = profile.id.clone();
        let rename = profile.id.clone();
        let merge = profile.id.clone();
        let delete = profile.id.clone();
        let field = TextField::new(
            ui_text::message(s.locale(), "speaker_name_for", &profile.display_name),
            &profile.name,
        )
        .on_edit(move |s: &mut MeetNote| s.workspace_host.speaker_library.name_draft(&edit))
        .build(k)
        .key(format!("library-name-{}", profile.id));
        let name_changed = !profile.name.text.trim().is_empty()
            && profile.name.text.trim() != profile.display_name;
        col()
            .gap(6.0)
            .child(if library.busy() { field.inert() } else { field })
            .child(caption(
                k,
                ui_text::message(
                    s.locale(),
                    "voice_sample_count",
                    &profile.sample_count.to_string(),
                ),
            ))
            .child(
                row()
                    .gap(8.0)
                    .flex_wrap()
                    .child(
                        Button::new(s.tr("rename"))
                            .variant(ButtonVariant::Outline)
                            .disabled(busy || !name_changed)
                            .on_press_tx(move |s: &mut MeetNote, tx| {
                                let name = s.workspace_host.speaker_library.name_draft(&rename);
                                let change = LibraryChange::Rename {
                                    id: rename.clone(),
                                    name: name.text.clone(),
                                };
                                s.change_speaker_profiles(change, tx);
                            })
                            .build(k)
                            .key(format!("library-rename-{}", profile.id)),
                    )
                    .children((library.rows.len() > 1).then(|| {
                        Button::new(s.tr("merge_into_placeholder"))
                            .variant(ButtonVariant::Outline)
                            .disabled(busy)
                            .on_press(move |s: &mut MeetNote| {
                                s.confirm = Some(Confirm::MergeLibraryProfile {
                                    source: merge.clone(),
                                    target: None,
                                });
                            })
                            .build(k)
                            .key(format!("library-merge-{}", profile.id))
                    }))
                    .child(
                        Button::new(s.tr("delete"))
                            .variant(ButtonVariant::Danger)
                            .disabled(busy)
                            .on_press(move |s: &mut MeetNote| {
                                s.confirm = Some(Confirm::DeleteLibraryProfile(delete.clone()));
                            })
                            .build(k)
                            .key(format!("library-delete-{}", profile.id)),
                    ),
            )
    }))
}

pub(crate) fn merge_choices(s: &MeetNote, k: &Kit, source: &str, target: Option<&str>) -> El {
    let mut choices = RadioGroup::new().label(s.tr("keep_speaker_profile"));
    for profile in &s.workspace_host.speaker_library.rows {
        if profile.id != source {
            let id = profile.id.clone();
            choices = choices.item(
                RadioItem::new(&profile.display_name, target == Some(id.as_str())).on_select(
                    move |s: &mut MeetNote| {
                        if let Some(Confirm::MergeLibraryProfile { target, .. }) = &mut s.confirm {
                            *target = Some(id.clone());
                        }
                    },
                ),
            );
        }
    }
    choices.build(k).key("library-merge-targets")
}
