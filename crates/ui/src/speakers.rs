//! Speaker names: the speakers of a document in the Original view and the
//! profiles of the Speaker Library in Settings.
//!
//! A document speaker is renamed with `DocumentCommand::SetSpeakerLabel` and
//! merged into another with `DocumentCommand::MergeSpeaker`, each checked
//! against the revision its row was built from; ASR text, timings and
//! processing provenance never change. A profile is renamed, merged or
//! deleted through `application::SpeakerLibraryService`; documents keep the
//! speaker labels they were given. Merges and deletions wait for the user's
//! confirmation.
//!
//! The commands and library calls run in the controller on worker threads;
//! this module holds UI state and pure helpers only.

use crate::{
    controller::RootView,
    controls::{Dropdown, DropdownEvent, SelectOption, TextField, TextFieldEvent},
    i18n::{self, Locale},
};
use contracts::SpeakerProfile;
use domain::{DocumentState, ProjectId};
use gpui::{prelude::*, App, Context, Entity, Subscription, Window};

/// One speaker of a document's results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocumentSpeaker {
    pub(crate) id: String,
    pub(crate) label: Option<String>,
    /// Segments of the speaker-labelled transcript attributed to it.
    pub(crate) segments: usize,
}

impl DocumentSpeaker {
    /// The label, or the id of a speaker without one.
    pub(crate) fn name(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.id)
    }
}

/// The speakers of `state` in the order they first speak, then those only
/// the diarization turns name. A speaker's label is the first one its
/// segments carry; corrections label every segment of a speaker alike.
pub(crate) fn document_speakers(state: &DocumentState) -> Vec<DocumentSpeaker> {
    let mut speakers: Vec<DocumentSpeaker> = Vec::new();
    let mut note = |id: &str, label: Option<&String>, segment: bool| {
        let index = match speakers.iter().position(|speaker| speaker.id == id) {
            Some(index) => index,
            None => {
                speakers.push(DocumentSpeaker {
                    id: id.into(),
                    label: None,
                    segments: 0,
                });
                speakers.len() - 1
            }
        };
        let speaker = &mut speakers[index];
        speaker.segments += usize::from(segment);
        if speaker.label.is_none() {
            speaker.label = label.cloned();
        }
    };
    for segment in &state.results.speaker_segments {
        note(&segment.speaker_id, segment.speaker_label.as_ref(), true);
    }
    for turn in &state.results.diarization {
        note(&turn.speaker_id, turn.speaker_label.as_ref(), false);
    }
    speakers
}

/// The quiet line under a profile's name: its sample count and aliases.
pub(crate) fn profile_summary(profile: &SpeakerProfile, locale: Locale) -> String {
    let samples = i18n::message(
        locale,
        "speaker_profile_samples",
        &profile.samples.len().to_string(),
    );
    if profile.aliases.is_empty() {
        return samples;
    }
    let aliases = i18n::message(
        locale,
        "speaker_profile_aliases",
        &profile.aliases.join(", "),
    );
    format!("{samples} · {aliases}")
}

/// The Speaker Library profiles as the last read returned them.
#[derive(Debug, Default)]
pub(crate) struct SpeakerProfiles {
    pub(crate) profiles: Vec<SpeakerProfile>,
    /// Whether a read has landed since the window opened.
    pub(crate) loaded: bool,
    /// Why the last read failed; the list is empty then.
    pub(crate) read_error: Option<String>,
    /// The outcome of the last library call: its message and whether it
    /// failed.
    pub(crate) status: Option<(String, bool)>,
    /// Whether a library call is in flight.
    pub(crate) busy: bool,
    /// Bumped whenever `profiles` changes, so the rows are rebuilt.
    pub(crate) version: u64,
    /// The latest read; an older one that lands later is dropped.
    generation: u64,
}

impl SpeakerProfiles {
    pub(crate) fn begin_read(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }

    /// Shows the profiles read by read `generation` unless a newer read has
    /// started.
    pub(crate) fn apply_read(
        &mut self,
        generation: u64,
        read: Result<Vec<SpeakerProfile>, String>,
        locale: Locale,
    ) {
        if generation != self.generation {
            return;
        }
        let profiles = match read {
            Ok(profiles) => {
                self.read_error = None;
                profiles
            }
            Err(error) => {
                self.read_error =
                    Some(i18n::message(locale, "speaker_library_read_failed", &error));
                Vec::new()
            }
        };
        if !self.loaded || profiles != self.profiles {
            self.profiles = profiles;
            self.version += 1;
        }
        self.loaded = true;
    }

    pub(crate) fn find(&self, id: &str) -> Option<&SpeakerProfile> {
        self.profiles.iter().find(|profile| profile.id == id)
    }
}

/// A change that waits for the user's confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SpeakerConfirmation {
    /// Attribute every segment of document speaker `from` to `into`, checked
    /// against `revision`.
    MergeDocumentSpeaker {
        document_id: ProjectId,
        revision: u64,
        from: String,
        from_name: String,
        into: String,
        into_name: String,
    },
    /// Remove a profile from the Speaker Library.
    DeleteProfile { profile_id: String, name: String },
    /// Merge profile `source` into `target`, which keeps its id and name.
    MergeProfiles {
        source_id: String,
        source_name: String,
        target_id: String,
        target_name: String,
    },
}

impl SpeakerConfirmation {
    pub(crate) fn title(&self, locale: Locale) -> String {
        match self {
            Self::MergeDocumentSpeaker {
                from_name,
                into_name,
                ..
            } => i18n::message_pair(locale, "merge_document_speaker_title", from_name, into_name),
            Self::DeleteProfile { name, .. } => {
                i18n::message(locale, "delete_speaker_profile_title", name)
            }
            Self::MergeProfiles {
                source_name,
                target_name,
                ..
            } => i18n::message_pair(
                locale,
                "merge_speaker_profiles_title",
                source_name,
                target_name,
            ),
        }
    }

    pub(crate) fn description(&self, locale: Locale) -> &'static str {
        i18n::text(
            locale,
            match self {
                Self::MergeDocumentSpeaker { .. } => "merge_document_speaker_description",
                Self::DeleteProfile { .. } => "delete_speaker_profile_description",
                Self::MergeProfiles { .. } => "merge_speaker_profiles_description",
            },
        )
    }

    pub(crate) fn confirm_label(&self, locale: Locale) -> &'static str {
        i18n::text(
            locale,
            match self {
                Self::DeleteProfile { .. } => "delete",
                Self::MergeDocumentSpeaker { .. } | Self::MergeProfiles { .. } => "merge",
            },
        )
    }
}

/// An entry a row lets the user rename and merge into another entry: a
/// document speaker or a Speaker Library profile.
pub(crate) struct NameEntry {
    pub(crate) id: String,
    /// The text the name field starts with.
    pub(crate) name: String,
    /// Shown while the name field is empty.
    pub(crate) placeholder: String,
    /// How the other rows' merge selects offer this entry.
    pub(crate) option: SelectOption,
    /// The name field's label for assistive technology.
    pub(crate) field_label: String,
    /// The merge select's label for assistive technology.
    pub(crate) merge_label: String,
}

impl NameEntry {
    pub(crate) fn speaker(speaker: &DocumentSpeaker, locale: Locale) -> Self {
        let name = speaker.name();
        let option = SelectOption::labelled(speaker.id.clone(), name);
        Self {
            id: speaker.id.clone(),
            name: speaker.label.clone().unwrap_or_default(),
            placeholder: speaker.id.clone(),
            option: if speaker.label.is_some() {
                option.detail(speaker.id.clone())
            } else {
                option
            },
            field_label: i18n::message(locale, "speaker_name_for", name),
            merge_label: i18n::message(locale, "merge_speaker_for", name),
        }
    }

    pub(crate) fn profile(profile: &SpeakerProfile, locale: Locale) -> Self {
        let name = &profile.display_name;
        Self {
            id: profile.id.clone(),
            name: name.clone(),
            placeholder: i18n::text(locale, "speaker_profile_name_placeholder").into(),
            option: SelectOption::labelled(profile.id.clone(), name.clone()),
            field_label: i18n::message(locale, "speaker_profile_name_for", name),
            merge_label: i18n::message(locale, "merge_speaker_profile_for", name),
        }
    }
}

/// One entry's controls.
pub(crate) struct NameRow {
    pub(crate) id: String,
    /// The name the row was built with.
    pub(crate) initial: String,
    pub(crate) name: Entity<TextField>,
    /// The entries this one can merge into; none when it is the only entry.
    pub(crate) merge_into: Option<Entity<Dropdown>>,
    options: Vec<SelectOption>,
    _subscriptions: Vec<Subscription>,
}

/// Called with a row's id when Enter is pressed in its name field.
pub(crate) type OnRename = fn(&mut RootView, String, &mut Context<RootView>);
/// Called with a row's id and the id chosen in its merge select.
pub(crate) type OnMerge = fn(&mut RootView, String, String, &mut Window, &mut Context<RootView>);

/// The rows and subscriptions for a source key. The owner can advance that
/// key after verifying that the controls' source data did not change.
pub(crate) struct NameRows<K> {
    key: K,
    pub(crate) rows: Vec<NameRow>,
}

impl<K: PartialEq> NameRows<K> {
    pub(crate) fn build(
        key: K,
        entries: &[NameEntry],
        merge_placeholder: &str,
        (on_rename, on_merge): (OnRename, OnMerge),
        window: &mut Window,
        cx: &mut Context<RootView>,
    ) -> Self {
        let mut rows = Vec::with_capacity(entries.len());
        for entry in entries {
            let mut subscriptions = Vec::new();
            let name = cx.new(|cx| {
                let mut field = TextField::new(
                    entry.field_label.clone(),
                    entry.placeholder.clone(),
                    window,
                    cx,
                );
                field.set_value(entry.name.clone(), cx);
                field
            });
            let id = entry.id.clone();
            subscriptions.push(cx.subscribe_in(
                &name,
                window,
                move |root, _, event: &TextFieldEvent, _, cx| match event {
                    TextFieldEvent::Submit => on_rename(root, id.clone(), cx),
                    TextFieldEvent::Change => {}
                },
            ));
            let options = entries
                .iter()
                .filter(|other| other.id != entry.id)
                .map(|other| other.option.clone())
                .collect::<Vec<_>>();
            let merge_into = (!options.is_empty()).then(|| {
                let select = cx.new(|cx| {
                    Dropdown::new(
                        entry.merge_label.clone(),
                        merge_placeholder.to_string(),
                        options.clone(),
                        None,
                        cx,
                    )
                });
                let from = entry.id.clone();
                subscriptions.push(cx.subscribe_in(
                    &select,
                    window,
                    move |root, _, event: &DropdownEvent, window, cx| {
                        let DropdownEvent::Confirm(into) = event;
                        on_merge(root, from.clone(), into.clone(), window, cx);
                    },
                ));
                select
            });
            rows.push(NameRow {
                id: entry.id.clone(),
                initial: entry.name.clone(),
                name,
                merge_into,
                options,
                _subscriptions: subscriptions,
            });
        }
        Self { key, rows }
    }

    pub(crate) fn is_for(&self, key: &K) -> bool {
        &self.key == key
    }

    pub(crate) fn key(&self) -> &K {
        &self.key
    }

    /// Advance a checked source key without replacing controls or their state.
    pub(crate) fn set_key(&mut self, key: K) {
        self.key = key;
    }

    /// Accept a committed correction without replacing its inputs. A rename
    /// keeps the submitted input even if later typing restored its old value;
    /// other untouched inputs follow the saved names. Merges may remove rows.
    pub(crate) fn saved(
        &mut self,
        key: K,
        entries: &[NameEntry],
        submitted_field: Option<gpui::EntityId>,
        merge_placeholder: &str,
        cx: &mut Context<RootView>,
    ) -> bool {
        if entries.iter().any(|entry| self.row(&entry.id).is_none()) {
            return false;
        }
        self.rows
            .retain(|row| entries.iter().any(|entry| entry.id == row.id));
        for row in &mut self.rows {
            let entry = entries.iter().find(|entry| entry.id == row.id).unwrap();
            let current = row.name.read(cx).value(cx);
            let follow_saved = Some(row.name.entity_id()) != submitted_field
                && current == row.initial
                && current != entry.name;
            row.name.update(cx, |field, cx| {
                if follow_saved {
                    field.set_value(entry.name.clone(), cx);
                }
                field.set_label(entry.field_label.clone(), cx);
                field.set_placeholder(entry.placeholder.clone(), cx);
            });
            row.initial = entry.name.clone();
            let options = entries
                .iter()
                .filter(|other| other.id != entry.id)
                .map(|other| other.option.clone())
                .collect::<Vec<_>>();
            if options.is_empty() {
                row.merge_into = None;
            } else if let Some(select) = &row.merge_into {
                let options = options.clone();
                select.update(cx, |select, cx| {
                    select.set_label(entry.merge_label.clone(), merge_placeholder.to_string(), cx);
                    select.set_options(options, None, cx);
                });
            } else {
                return false;
            }
            row.options = options;
        }
        self.key = key;
        true
    }

    pub(crate) fn row(&self, id: &str) -> Option<&NameRow> {
        self.rows.iter().find(|row| row.id == id)
    }

    /// The trimmed text of row `id`'s name field and whether it differs
    /// from the name the row was built with.
    pub(crate) fn edited_name(&self, id: &str, cx: &App) -> Option<(String, bool)> {
        let row = self.row(id)?;
        let name = row.name.read(cx).value(cx).trim().to_string();
        let changed = name != row.initial;
        Some((name, changed))
    }

    /// Shows row `id`'s merge select without a choice again.
    pub(crate) fn clear_merge_choice(&self, id: &str, cx: &mut Context<RootView>) {
        let Some(row) = self.row(id) else {
            return;
        };
        if let Some(select) = &row.merge_into {
            let options = row.options.clone();
            select.update(cx, |select, cx| select.set_options(options, None, cx));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{DiarizationSegment, SpeakerTaggedSegment};

    fn segment(id: &str, speaker: &str, label: Option<&str>) -> SpeakerTaggedSegment {
        SpeakerTaggedSegment {
            transcript_segment_id: id.into(),
            speaker_id: speaker.into(),
            speaker_profile_id: None,
            speaker_label: label.map(str::to_string),
            start_ms: 0,
            end_ms: 1_000,
            text: id.into(),
        }
    }

    #[test]
    fn speakers_are_listed_in_speaking_order_with_label_and_segment_count() {
        let mut state = DocumentState::new(ProjectId::new("doc"), "Doc", "doc.wav");
        state.results.speaker_segments = vec![
            segment("t-0", "speaker-3", None),
            segment("t-1", "speaker-0", Some("王经理")),
            segment("t-2", "speaker-3", None),
            segment("t-3", "speaker-0", Some("王经理")),
            segment("t-4", "speaker-3", None),
        ];
        // A turn no transcript segment overlaps still names a speaker.
        state.results.diarization = vec![DiarizationSegment {
            id: "d-9".into(),
            speaker_id: "speaker-9".into(),
            speaker_profile_id: None,
            speaker_label: Some("访客".into()),
            start_ms: 5_000,
            end_ms: 6_000,
            confidence: None,
        }];
        let speaker = |id: &str, label: Option<&str>, segments| DocumentSpeaker {
            id: id.into(),
            label: label.map(str::to_string),
            segments,
        };
        assert_eq!(
            document_speakers(&state),
            [
                speaker("speaker-3", None, 3),
                speaker("speaker-0", Some("王经理"), 2),
                speaker("speaker-9", Some("访客"), 0),
            ]
        );
        assert_eq!(document_speakers(&state)[0].name(), "speaker-3");
        assert_eq!(document_speakers(&state)[1].name(), "王经理");
        assert!(
            document_speakers(&DocumentState::new(ProjectId::new("new"), "New", "new.wav"))
                .is_empty()
        );
    }

    fn profile(id: &str) -> SpeakerProfile {
        SpeakerProfile {
            id: id.into(),
            display_name: id.into(),
            aliases: Vec::new(),
            embedding_space: contracts::EmbeddingSpace {
                backend_id: "b".into(),
                model_id: "m".into(),
                model_revision: None,
                dimension: 1,
                sample_rate_hz: 16_000,
                frontend: contracts::EmbeddingFrontend::default(),
                legacy_feature_config: None,
                normalization: contracts::EmbeddingNormalization::L2,
            },
            samples: Vec::new(),
            match_threshold: 0.7,
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    #[test]
    fn a_read_changes_the_list_only_when_it_is_the_latest_and_differs() {
        let mut library = SpeakerProfiles::default();
        let first = library.begin_read();
        library.apply_read(first, Ok(Vec::new()), Locale::EnUs);
        assert!(library.loaded);
        assert_eq!(library.version, 1, "the first read shows the list");

        let stale = library.begin_read();
        let latest = library.begin_read();
        library.apply_read(latest, Ok(vec![profile("a")]), Locale::EnUs);
        library.apply_read(stale, Ok(Vec::new()), Locale::EnUs);
        assert_eq!(library.profiles, [profile("a")], "a stale read is dropped");
        assert_eq!(library.version, 2);

        let same = library.begin_read();
        library.apply_read(same, Ok(vec![profile("a")]), Locale::EnUs);
        assert_eq!(library.version, 2, "an equal list keeps the rows");

        let failed = library.begin_read();
        library.apply_read(failed, Err("bad json".into()), Locale::EnUs);
        assert!(library.profiles.is_empty());
        assert_eq!(
            library.read_error.as_deref(),
            Some(i18n::message(Locale::EnUs, "speaker_library_read_failed", "bad json").as_str())
        );
        let repaired = library.begin_read();
        library.apply_read(repaired, Ok(Vec::new()), Locale::EnUs);
        assert!(library.read_error.is_none());
    }

    #[test]
    fn a_profile_summary_counts_samples_and_lists_aliases() {
        let mut named = profile("a");
        named.samples = vec![contracts::SpeakerEmbeddingSample {
            id: "s".into(),
            vector: vec![1.0],
            quality: 1.0,
            source: contracts::SpeakerSampleSource {
                project_id: None,
                source_segment_id: None,
                start_ms: 0,
                end_ms: 1,
                audio_path: Some("a.wav".into()),
                audio_sha256: None,
                backup_path: None,
            },
            created_at_ms: 0,
        }];
        assert_eq!(profile_summary(&named, Locale::EnUs), "Samples: 1");
        named.aliases = vec!["Wang".into(), "W".into()];
        assert_eq!(
            profile_summary(&named, Locale::ZhCn),
            "1 个样本 · 别名：Wang, W"
        );
    }

    #[test]
    fn every_confirmation_names_its_entries_in_both_locales() {
        let confirmations = [
            SpeakerConfirmation::MergeDocumentSpeaker {
                document_id: ProjectId::new("doc"),
                revision: 1,
                from: "speaker-2".into(),
                from_name: "访客".into(),
                into: "speaker-0".into(),
                into_name: "王经理".into(),
            },
            SpeakerConfirmation::DeleteProfile {
                profile_id: "p".into(),
                name: "王经理".into(),
            },
            SpeakerConfirmation::MergeProfiles {
                source_id: "q".into(),
                source_name: "访客".into(),
                target_id: "p".into(),
                target_name: "王经理".into(),
            },
        ];
        for confirmation in &confirmations {
            for locale in [Locale::ZhCn, Locale::EnUs] {
                let title = confirmation.title(locale);
                assert!(title.contains("王经理"), "{title}");
                assert!(!confirmation.description(locale).is_empty());
                assert!(!confirmation.confirm_label(locale).is_empty());
            }
        }
        assert!(confirmations[0].title(Locale::EnUs).contains("访客"));
        assert!(confirmations[2].title(Locale::ZhCn).contains("访客"));
    }
}
