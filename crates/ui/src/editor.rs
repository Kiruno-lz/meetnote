//! The document editor of the Meetings content plane: labelled fields for
//! the semantic fields of `document.md + state.json` (title, tags, summary,
//! body and document status) and the drafts behind them.
//!
//! A draft is the values a document was read with (`DocumentCommand::Read`)
//! and the revision it was read at; the user's edits stay in the fields until
//! Save sends one revision-checked `DocumentCommand::Update` with only the
//! changed fields. Drafts are kept per document, so selecting another
//! document never drops unsaved edits; only an explicit reload replaces one.
//! Processing and results are not part of a draft and cannot be edited.
//!
//! Reading and writing run in the controller on worker threads; this module
//! holds UI state and pure helpers only.

use crate::{
    controller::RootView,
    controls::{Dropdown, SelectOption, TextArea, TextField},
    i18n::{self, Locale},
};
use contracts::{DocumentPatch, DocumentRecord};
use domain::{DocumentState, ProjectId};
use gpui::{prelude::*, App, Context, Entity, Window};
use std::collections::{BTreeMap, BTreeSet};

/// Workflow statuses the status select offers besides those the workspace
/// already uses, with the keys of their descriptions. MeetNote does not fix
/// the set: any stored value is shown and kept.
pub(crate) const SUGGESTED_STATUSES: [(&str, &str); 5] = [
    ("inbox", "status_inbox"),
    ("in-progress", "status_in_progress"),
    ("review", "status_review"),
    ("final", "status_final"),
    ("archived", "status_archived"),
];

/// The editable fields as the editor shows them; tags are the typed text.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct DraftValues {
    pub(crate) title: String,
    pub(crate) tags: String,
    pub(crate) summary: String,
    pub(crate) body: String,
    pub(crate) status: String,
}

impl DraftValues {
    pub(crate) fn from_record(record: &DocumentRecord) -> Self {
        let content = &record.state.content;
        Self {
            title: content.title.clone(),
            tags: format_tags(&content.tags),
            summary: content.summary.clone().unwrap_or_default(),
            body: record.markdown.clone(),
            status: record.state.document_status.value.clone(),
        }
    }
}

/// Why a draft cannot be saved; the value is the notice's translation key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DraftError {
    TitleRequired,
}

impl DraftError {
    pub(crate) fn notice_key(self) -> &'static str {
        match self {
            Self::TitleRequired => "title_required",
        }
    }
}

/// Tags typed as a list separated by commas (`,`, `，` or `、`): trimmed,
/// without empty or repeated entries, in typed order.
pub(crate) fn parse_tags(text: &str) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for tag in text.split([',', '，', '、']).map(str::trim) {
        if !tag.is_empty() && !tags.iter().any(|kept| kept == tag) {
            tags.push(tag.to_string());
        }
    }
    tags
}

/// The text a tag list is edited as.
pub(crate) fn format_tags(tags: &[String]) -> String {
    tags.join(", ")
}

/// The patch that turns `base` into `edits`: only the fields that differ.
/// Tags compare as parsed lists and a blank summary clears it.
pub(crate) fn draft_patch(
    base: &DraftValues,
    edits: &DraftValues,
) -> Result<DocumentPatch, DraftError> {
    let title = edits.title.trim();
    if title.is_empty() {
        return Err(DraftError::TitleRequired);
    }
    let summary = |text: &str| (!text.trim().is_empty()).then(|| text.to_string());
    let tags = parse_tags(&edits.tags);
    let summary_edit = summary(&edits.summary);
    Ok(DocumentPatch {
        title: (title != base.title).then(|| title.to_string()),
        tags: (tags != parse_tags(&base.tags)).then_some(tags),
        summary: (summary_edit != summary(&base.summary)).then_some(summary_edit),
        document_status: (edits.status != base.status).then(|| edits.status.clone()),
        markdown: (edits.body != base.body).then(|| edits.body.clone()),
    })
}

/// The status select's options: the suggested statuses, then the other
/// statuses the documents use, then `current` if it is neither.
pub(crate) fn status_options(
    documents: &[DocumentState],
    current: &str,
    locale: Locale,
) -> Vec<SelectOption> {
    let mut values: Vec<&str> = SUGGESTED_STATUSES.iter().map(|(value, _)| *value).collect();
    let mut used: Vec<&str> = documents
        .iter()
        .map(|document| document.document_status.value.as_str())
        .collect();
    used.sort_unstable();
    used.push(current);
    for value in used {
        if !value.is_empty() && !values.contains(&value) {
            values.push(value);
        }
    }
    values
        .into_iter()
        .map(|value| {
            let option = SelectOption::text(value);
            match SUGGESTED_STATUSES.iter().find(|(known, _)| *known == value) {
                Some((_, key)) => option.detail(i18n::text(locale, key)),
                None => option,
            }
        })
        .collect()
}

/// A loaded draft: the revision and values it was read with, and the edits
/// kept while another document is shown.
struct Draft {
    revision: u64,
    base: DraftValues,
    edits: DraftValues,
}

/// The shown draft, as Save reads it.
pub(crate) struct ShownDraft {
    pub(crate) document_id: ProjectId,
    pub(crate) revision: u64,
    pub(crate) base: DraftValues,
    pub(crate) edits: DraftValues,
}

pub(crate) struct DocumentEditor {
    pub(crate) title: Entity<TextField>,
    pub(crate) tags: Entity<TextField>,
    pub(crate) summary: Entity<TextArea>,
    pub(crate) body: Entity<TextArea>,
    pub(crate) status: Entity<Dropdown>,
    /// The document whose draft the fields hold.
    shown: Option<ProjectId>,
    /// The shown draft's status, as its select last confirmed it.
    status_value: String,
    drafts: BTreeMap<ProjectId, Draft>,
    /// Documents whose draft is being read.
    pub(crate) loading: BTreeSet<ProjectId>,
    /// The document whose save is in flight.
    pub(crate) saving: Option<ProjectId>,
}

impl DocumentEditor {
    pub(crate) fn new(locale: Locale, window: &mut Window, cx: &mut Context<RootView>) -> Self {
        let text = |key| i18n::text(locale, key);
        let title = cx.new(|cx| TextField::new(text("document_title"), "", window, cx));
        let tags = cx.new(|cx| {
            TextField::new(
                text("document_tags"),
                text("document_tags_placeholder"),
                window,
                cx,
            )
        });
        let summary = cx.new(|cx| TextArea::new(text("document_summary"), "", 4, window, cx));
        let body = cx.new(|cx| TextArea::new(text("document_body"), "", 12, window, cx));
        let status = cx.new(|cx| {
            Dropdown::new(
                text("document_status"),
                text("select_placeholder"),
                Vec::new(),
                None,
                cx,
            )
        });
        Self {
            title,
            tags,
            summary,
            body,
            status,
            shown: None,
            status_value: String::new(),
            drafts: BTreeMap::new(),
            loading: BTreeSet::new(),
            saving: None,
        }
    }

    /// Relabels every field in `locale` and describes the suggested
    /// statuses in it.
    pub(crate) fn follow_locale(
        &self,
        locale: Locale,
        documents: &[DocumentState],
        cx: &mut Context<RootView>,
    ) {
        let text = |key| i18n::text(locale, key);
        let options = status_options(documents, &self.status_value, locale);
        let selected = self.status_value.clone();
        self.status.update(cx, |select, cx| {
            select.set_label(text("document_status"), text("select_placeholder"), cx);
            select.set_options(options, Some(&selected), cx);
        });
        self.title
            .update(cx, |field, cx| field.set_label(text("document_title"), cx));
        self.tags.update(cx, |field, cx| {
            field.set_label(text("document_tags"), cx);
            field.set_placeholder(text("document_tags_placeholder"), cx);
        });
        self.summary
            .update(cx, |area, cx| area.set_label(text("document_summary"), cx));
        self.body
            .update(cx, |area, cx| area.set_label(text("document_body"), cx));
    }

    /// The document whose draft the fields show.
    pub(crate) fn shown(&self) -> Option<&ProjectId> {
        self.shown.as_ref()
    }

    pub(crate) fn has_draft(&self, document_id: &ProjectId) -> bool {
        self.drafts.contains_key(document_id)
    }

    /// The revision `document_id`'s draft was read at.
    #[cfg(test)]
    pub(crate) fn draft_revision(&self, document_id: &ProjectId) -> Option<u64> {
        self.drafts.get(document_id).map(|draft| draft.revision)
    }

    /// Keeps the shown draft's edits and shows `document_id`'s draft. Returns
    /// false, showing nothing, when that document has no draft yet.
    pub(crate) fn show(
        &mut self,
        document_id: &ProjectId,
        documents: &[DocumentState],
        locale: Locale,
        cx: &mut Context<RootView>,
    ) -> bool {
        if self.shown.as_ref() == Some(document_id) {
            return true;
        }
        self.stash(cx);
        let Some(edits) = self
            .drafts
            .get(document_id)
            .map(|draft| draft.edits.clone())
        else {
            return false;
        };
        self.fill(&edits, documents, locale, cx);
        self.shown = Some(document_id.clone());
        true
    }

    /// Replaces the draft of `record`'s document with the values it holds,
    /// discarding its edits, and shows it when `show`.
    pub(crate) fn load(
        &mut self,
        record: &DocumentRecord,
        show: bool,
        documents: &[DocumentState],
        locale: Locale,
        cx: &mut Context<RootView>,
    ) {
        let document_id = record.state.id().clone();
        let values = DraftValues::from_record(record);
        self.loading.remove(&document_id);
        if show && self.shown.as_ref() != Some(&document_id) {
            self.stash(cx);
        }
        self.drafts.insert(
            document_id.clone(),
            Draft {
                revision: record.state.revision(),
                base: values.clone(),
                edits: values.clone(),
            },
        );
        if show || self.shown.as_ref() == Some(&document_id) {
            self.fill(&values, documents, locale, cx);
            self.shown = Some(document_id);
        }
    }

    /// Rebases `record`'s draft on the revision a save just wrote. The
    /// edits stay, including any typed while the save was in flight.
    pub(crate) fn saved(&mut self, record: &DocumentRecord) {
        if let Some(draft) = self.drafts.get_mut(record.state.id()) {
            draft.revision = record.state.revision();
            draft.base = DraftValues::from_record(record);
        }
    }

    /// Drops `document_id`'s draft; the fields hide it if they show it.
    pub(crate) fn discard(&mut self, document_id: &ProjectId) {
        self.drafts.remove(document_id);
        if self.shown.as_ref() == Some(document_id) {
            self.shown = None;
        }
    }

    /// The status the select last confirmed for the shown draft.
    pub(crate) fn set_status(&mut self, value: String) {
        self.status_value = value;
    }

    /// The shown draft with the fields' current values.
    pub(crate) fn current(&self, cx: &App) -> Option<ShownDraft> {
        let document_id = self.shown.clone()?;
        let draft = self.drafts.get(&document_id)?;
        Some(ShownDraft {
            revision: draft.revision,
            base: draft.base.clone(),
            edits: self.field_values(cx),
            document_id,
        })
    }

    fn field_values(&self, cx: &App) -> DraftValues {
        DraftValues {
            title: self.title.read(cx).value(cx),
            tags: self.tags.read(cx).value(cx),
            summary: self.summary.read(cx).value(cx),
            body: self.body.read(cx).value(cx),
            status: self.status_value.clone(),
        }
    }

    /// Keeps the fields' values as the shown draft's edits.
    fn stash(&mut self, cx: &App) {
        let Some(shown) = self.shown.take() else {
            return;
        };
        let edits = self.field_values(cx);
        if let Some(draft) = self.drafts.get_mut(&shown) {
            draft.edits = edits;
        }
    }

    fn fill(
        &mut self,
        values: &DraftValues,
        documents: &[DocumentState],
        locale: Locale,
        cx: &mut Context<RootView>,
    ) {
        self.title
            .update(cx, |field, cx| field.set_value(values.title.clone(), cx));
        self.tags
            .update(cx, |field, cx| field.set_value(values.tags.clone(), cx));
        self.summary
            .update(cx, |area, cx| area.set_value(values.summary.clone(), cx));
        self.body
            .update(cx, |area, cx| area.set_value(values.body.clone(), cx));
        let options = status_options(documents, &values.status, locale);
        let status = values.status.clone();
        self.status.update(cx, |select, cx| {
            select.set_options(options, Some(&status), cx)
        });
        self.status_value = values.status.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(title: &str, tags: &str, summary: &str, body: &str, status: &str) -> DraftValues {
        DraftValues {
            title: title.into(),
            tags: tags.into(),
            summary: summary.into(),
            body: body.into(),
            status: status.into(),
        }
    }

    #[test]
    fn tags_parse_from_any_comma_and_format_back() {
        assert_eq!(
            parse_tags(" 周会, 决策，行动项、 ,周会 "),
            ["周会", "决策", "行动项"],
            "trimmed, without empty or repeated tags, in typed order"
        );
        assert!(parse_tags(" , ，").is_empty());
        let tags = vec!["周会".to_string(), "决策".to_string()];
        assert_eq!(format_tags(&tags), "周会, 决策");
        assert_eq!(parse_tags(&format_tags(&tags)), tags);
    }

    #[test]
    fn a_draft_reads_the_semantic_fields_of_a_record() {
        let mut state = DocumentState::new(ProjectId::new("doc"), "Weekly", "weekly.wav");
        state.content.tags = vec!["周会".into(), "决策".into()];
        state.content.summary = Some("结论".into());
        state.document_status.value = "review".into();
        let record = DocumentRecord {
            state,
            markdown: "# Weekly\n\nbody".into(),
        };
        assert_eq!(
            DraftValues::from_record(&record),
            values("Weekly", "周会, 决策", "结论", "# Weekly\n\nbody", "review")
        );
        let mut empty = record.clone();
        empty.state.content.summary = None;
        assert_eq!(DraftValues::from_record(&empty).summary, "");
    }

    #[test]
    fn the_patch_carries_only_the_changed_fields() {
        let base = values("Weekly", "周会", "结论", "# Weekly\n", "inbox");
        assert_eq!(
            draft_patch(&base, &base.clone()),
            Ok(DocumentPatch::default()),
            "no edits, no fields"
        );
        let edits = values(
            "  Weekly v2 ",
            "周会, 决策",
            "新结论",
            "# Weekly\n\nmore",
            "review",
        );
        assert_eq!(
            draft_patch(&base, &edits),
            Ok(DocumentPatch {
                title: Some("Weekly v2".into()),
                tags: Some(vec!["周会".into(), "决策".into()]),
                summary: Some(Some("新结论".into())),
                document_status: Some("review".into()),
                markdown: Some("# Weekly\n\nmore".into()),
            })
        );
        // Retyped tags and an untouched title are no change.
        let retyped = values("Weekly", " 周会 ,", "结论", "# Weekly\n", "inbox");
        assert_eq!(draft_patch(&base, &retyped), Ok(DocumentPatch::default()));
        // A blank summary clears it.
        let cleared = values("Weekly", "周会", "  \n", "# Weekly\n", "inbox");
        assert_eq!(
            draft_patch(&base, &cleared).map(|patch| patch.summary),
            Ok(Some(None))
        );
        let untitled = values(" ", "周会", "结论", "# Weekly\n", "inbox");
        assert_eq!(
            draft_patch(&base, &untitled),
            Err(DraftError::TitleRequired)
        );
    }

    #[test]
    fn the_status_select_offers_suggested_used_and_current_statuses() {
        let document = |id: &str, status: &str| {
            let mut state = DocumentState::new(ProjectId::new(id), id, "a.wav");
            state.document_status.value = status.into();
            state
        };
        let documents = [
            document("a", "review"),
            document("b", "客户跟进"),
            document("c", "客户跟进"),
        ];
        let options = status_options(&documents, "待办", Locale::ZhCn);
        let values = options.iter().map(SelectOption::value).collect::<Vec<_>>();
        assert_eq!(
            values,
            [
                "inbox",
                "in-progress",
                "review",
                "final",
                "archived",
                "客户跟进",
                "待办"
            ]
        );
        assert_eq!(
            status_options(&[], "inbox", Locale::EnUs).len(),
            SUGGESTED_STATUSES.len()
        );
    }

    #[test]
    fn every_suggested_status_is_described_in_both_locales() {
        for (value, key) in SUGGESTED_STATUSES {
            for locale in [Locale::ZhCn, Locale::EnUs] {
                assert!(
                    !i18n::text(locale, key).is_empty(),
                    "{value}: {key} ({locale:?})"
                );
            }
        }
        for key in [
            "document_title",
            "document_tags",
            "document_tags_placeholder",
            "document_summary",
            "document_body",
            "document_status",
        ] {
            for locale in [Locale::ZhCn, Locale::EnUs] {
                assert!(!i18n::text(locale, key).is_empty(), "{key} ({locale:?})");
            }
        }
    }
}
