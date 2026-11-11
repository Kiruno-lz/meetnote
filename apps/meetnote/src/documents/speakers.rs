use domain::{SpeakerCorrection, SpeakerTaggedSegment};
use rune::{El, TextEdit, Tx, col, row};
use rune_kit::{Button, ButtonVariant, Kit, RadioGroup, RadioItem, TextInput};

use super::Document;
use crate::controls::{caption, card};
use crate::{Confirm, MeetNote};

pub(super) fn name(segment: &SpeakerTaggedSegment) -> &str {
    segment
        .speaker_label
        .as_deref()
        .unwrap_or(&segment.speaker_id)
}

impl Document {
    pub(super) fn sync_speaker_drafts(&mut self) {
        let ids = self.state.speaker_ids();
        self.speaker_drafts
            .retain(|id, _| ids.contains(id.as_str()));
        for segment in &self.state.results.speaker_segments {
            self.speaker_drafts
                .entry(segment.speaker_id.clone())
                .or_insert_with(|| TextEdit::new(name(segment)));
        }
    }

    fn speaker_name<'a>(&'a self, id: &'a str) -> &'a str {
        self.state
            .results
            .speaker_segments
            .iter()
            .find(|segment| segment.speaker_id == id)
            .map_or(id, name)
    }

    pub(super) fn keep_speaker_drafts(
        &mut self,
        previous: &Document,
        correction: Option<&SpeakerCorrection>,
    ) {
        for (id, edit) in &previous.speaker_drafts {
            if !self.speaker_drafts.contains_key(id) || edit.text == previous.speaker_name(id) {
                continue;
            }
            if matches!(correction, Some(SpeakerCorrection::Label { speaker_id, label })
                if speaker_id == id && label.as_deref().unwrap_or("") == edit.text.trim())
            {
                continue;
            }
            self.speaker_drafts.insert(id.clone(), edit.clone());
        }
    }
}

impl MeetNote {
    fn rename_speaker(&mut self, id: &str, tx: &mut Tx) {
        let document = self.documents.current();
        let Some(draft) = document.speaker_drafts.get(id) else {
            return;
        };
        let label = draft.text.trim().to_owned();
        let correction = SpeakerCorrection::Label {
            speaker_id: id.into(),
            label: (!label.is_empty()).then_some(label),
        };
        if self.workspace_host.live.is_some() {
            self.correct_workspace_speakers(
                document.state.id().clone(),
                document.state.revision(),
                correction,
                tx,
            );
            return;
        }
        let document = self.documents.current_mut();
        if document.state.apply_speaker_correction(&correction).is_ok() {
            let saved = document.speaker_name(id).to_owned();
            let edit = document.speaker_drafts.get_mut(id).unwrap();
            if edit.text != saved {
                edit.replace_text(saved);
            }
            self.notify("Speaker name saved", tx);
        }
    }

    pub fn confirm_speaker_merge(
        &mut self,
        document: &str,
        from: String,
        into: String,
        tx: &mut Tx,
    ) {
        let Some(doc) = self.documents.tree.get(document) else {
            return;
        };
        let correction = SpeakerCorrection::Merge {
            from,
            into: into.clone(),
        };
        if self.workspace_host.live.is_some() {
            self.correct_workspace_speakers(
                doc.state.id().clone(),
                doc.state.revision(),
                correction,
                tx,
            );
            return;
        }
        let doc = self
            .documents
            .tree
            .get_mut(document)
            .expect("speaker document exists");
        if doc.state.apply_speaker_correction(&correction).is_ok() {
            doc.sync_speaker_drafts();
            tx.focus(format!("speaker-name-{into}"), tx.focus_visible());
            self.notify("Speakers merged", tx);
        }
    }

    pub fn speaker_merge_choices(&self, doc: &str, from: &str, into: Option<&str>, k: &Kit) -> El {
        let Some(document) = self.documents.tree.get(doc) else {
            return caption(k, "The document is no longer available.");
        };
        let mut choices = RadioGroup::new().label("Keep this speaker");
        for id in document
            .speaker_drafts
            .keys()
            .filter(|id| id.as_str() != from)
        {
            let target = id.clone();
            choices = choices.item(
                RadioItem::new(document.speaker_name(id), into == Some(id.as_str())).on_select(
                    move |s: &mut MeetNote| {
                        if let Some(Confirm::MergeSpeaker { into, .. }) = &mut s.confirm {
                            *into = Some(target.clone());
                        }
                    },
                ),
            );
        }
        choices.build(k).key("merge-targets")
    }
}

pub(super) fn controls(document: &Document, k: &Kit, busy: bool) -> El {
    card(k).children(document.speaker_drafts.iter().map(|(id, draft)| {
        let edit_id = id.clone();
        let rename_id = id.clone();
        let save_id = id.clone();
        let from = id.clone();
        col().w_full().gap(6.0).child(caption(k, id)).child(
            row()
                .items_center()
                .gap(6.0)
                .flex_wrap()
                .child(
                    TextInput::new(draft)
                        .label(format!("Display name for {id}"))
                        .width(None)
                        .on_edit(move |s: &mut MeetNote| {
                            s.documents
                                .current_mut()
                                .speaker_drafts
                                .get_mut(&edit_id)
                                .expect("a mounted input belongs to the current speaker list")
                        })
                        .on_submit(move |s: &mut MeetNote, tx| s.rename_speaker(&rename_id, tx))
                        .build(k)
                        .key(format!("speaker-name-{id}"))
                        .grow(1.0)
                        .min_w(100.0),
                )
                .child(
                    Button::new("Save name")
                        .disabled(busy)
                        .on_press_tx(move |s: &mut MeetNote, tx| s.rename_speaker(&save_id, tx))
                        .build(k)
                        .key(format!("speaker-save-{id}")),
                )
                .child(
                    Button::new("Merge")
                        .variant(ButtonVariant::Outline)
                        .disabled(busy || document.speaker_drafts.len() < 2)
                        .on_press(move |s: &mut MeetNote| {
                            s.confirm = Some(Confirm::MergeSpeaker {
                                document: s.documents.selection().0.into(),
                                from: from.clone(),
                                into: None,
                            });
                        })
                        .build(k)
                        .key(format!("speaker-merge-{id}")),
                ),
        )
    }))
}
