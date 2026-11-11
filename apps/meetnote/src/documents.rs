mod details;
mod speakers;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use application::{
    AudioStatus, DocumentIssue, DocumentSearchResults, DocumentSnapshot,
    SearchSource as PersistedSource, TranscriptSearchTarget,
};
use contracts::{DocumentPatch, DocumentRecord};

use domain::{
    DocumentState, ProjectId, ProjectTreeError, SpeakerCorrection, SpeakerTaggedSegment,
    parse_document_tags, reorder_project, reparent_project,
};
use rune::{El, Key, KeyEvent, TextEdit, Tx, col, el, row};
use rune_kit::{
    Badge, BadgeVariant, Button, ButtonVariant, Collapsible, Divider, DropPlacement as Placement,
    DropTarget as Target, Kit, Markdown, Progress, SegmentedControl, TextArea, TextField,
    TextInput, Tree, TreeDrag, TreeItem, TreeModel, TreeStyle, glyphs,
};

use crate::MeetNote;
use crate::controls::{
    action, caption, card, dot, eyebrow, icon_action, label, scroll, select, title,
};

const MAIN: &str = "product";
const TREE_KEY: &str = "document-tree";
const WORKFLOW_LABELS: [&str; 5] = [
    "status_inbox",
    "status_in_progress",
    "status_review",
    "status_final",
    "status_archived",
];
const WORKFLOW_STATES: [&str; 5] = ["inbox", "in-progress", "review", "final", "archived"];
const SAMPLE_NOTES: &str = "## Follow-up notes\n\nShare the indexing measurements with Design before the next review.\n\n- [ ] Confirm the pilot cohort\n- [ ] Add the recovery walkthrough";
const SUMMARY: &str = "The team agreed to ship the workspace beta to the pilot cohort next week. The release will focus on reliable imports, searchable transcripts and clear recovery when a source file moves.";
const TRANSCRIPT: [(u64, &str, &str); 6] = [
    (
        12_000,
        "Alex Chen",
        "Let's start with the workspace beta. We need reliable imports and a clear path to reconnect a recording after it's moved.",
    ),
    (
        94_000,
        "Morgan Park",
        "Search is ready for the pilot. I'll send the indexing measurements to Design before Thursday.",
    ),
    (
        188_000,
        "Alex Chen",
        "Great. Let's keep the pilot cohort small so we can follow up on failed imports quickly.",
    ),
    (
        322_000,
        "Morgan Park",
        "I'll add a recovery walkthrough to the onboarding checklist and share it with the pilot leads.",
    ),
    (
        531_000,
        "Alex Chen",
        "Let's check the pilot criteria and gather feedback at next week's review.",
    ),
    (
        702_000,
        "Morgan Park",
        "I have the action items. I'll bring the draft checklist on Thursday.",
    ),
];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Summary,
    Original,
    Notes,
    Edit,
}

impl Mode {
    const ALL: [Self; 4] = [Self::Summary, Self::Original, Self::Notes, Self::Edit];
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Imported,
    Processing,
    Ready,
}

#[derive(Clone)]
struct Document {
    state: DocumentState,
    notes: String,
    draft: Draft,
    duration: String,
    note_read: NoteRead,
    revision_conflict: bool,
    speaker_drafts: BTreeMap<String, TextEdit>,
    status: Status,
}

#[derive(Clone)]
enum NoteRead {
    Pending,
    Ready,
    Failed(String),
}

#[derive(Clone)]
struct Draft {
    title: TextEdit,
    notes: TextEdit,
    tags: TextEdit,
    summary: TextEdit,
    workflow: usize,
    base_notes: String,
    base_summary: String,
}

impl Draft {
    fn new(title: &str, notes: &str, summary: &str) -> Self {
        Self {
            title: TextEdit::new(title),
            notes: TextEdit::new(notes),
            tags: TextEdit::new(""),
            summary: TextEdit::new(summary),
            workflow: 0,
            base_notes: notes.into(),
            base_summary: summary.into(),
        }
    }
}

impl Document {
    fn new(
        id: &str,
        title: &str,
        source: &'static str,
        duration: &'static str,
        summary: &'static str,
    ) -> Self {
        let mut state = DocumentState::new(ProjectId::new(id), title, source);
        state.content.summary = Some(summary.into());
        if source == "product-sync.m4a" {
            state.source.audio_asset_id = Some(ProjectId::new(MAIN));
        }
        Self {
            state,
            notes: String::new(),
            draft: Draft::new(title, "", summary),
            duration: duration.into(),
            note_read: NoteRead::Ready,
            revision_conflict: false,
            speaker_drafts: BTreeMap::new(),
            status: Status::Ready,
        }
    }

    fn from_state(state: DocumentState) -> Self {
        let summary = state.content.summary.as_deref().unwrap_or("");
        let mut draft = Draft::new(state.title(), "", summary);
        draft.tags = TextEdit::new(state.content.tags.join(", "));
        draft.workflow = WORKFLOW_STATES
            .iter()
            .position(|value| *value == state.document_status.value)
            .unwrap_or(0);
        let duration = state
            .source
            .audio_end_ms
            .map(|end| timestamp(end.saturating_sub(state.source.audio_start_ms)))
            .unwrap_or_else(|| "Unknown duration".into());
        let status = if state.processing.latest.is_some()
            || !state.results.transcript.is_empty()
            || !state.results.speaker_segments.is_empty()
        {
            Status::Ready
        } else {
            Status::Imported
        };
        let mut document = Self {
            state,
            notes: String::new(),
            draft,
            duration,
            note_read: NoteRead::Pending,
            revision_conflict: false,
            speaker_drafts: BTreeMap::new(),
            status,
        };
        document.sync_speaker_drafts();
        document
    }

    fn from_record(record: DocumentRecord) -> Self {
        let mut document = Self::from_state(record.state);
        document.update_notes(record.markdown);
        document.note_read = NoteRead::Ready;
        document
    }

    fn patch(&self) -> DocumentPatch {
        let title = self.draft.title.text.trim();
        let tags = parse_document_tags(&self.draft.tags.text);
        let workflow = WORKFLOW_STATES[self.draft.workflow];
        let summary =
            (!self.draft.summary.text.trim().is_empty()).then(|| self.draft.summary.text.clone());
        DocumentPatch {
            title: (title != self.title()).then(|| title.into()),
            tags: (tags != self.state.content.tags).then_some(tags),
            document_status: (workflow != self.state.document_status.value
                && (self.draft.workflow != 0
                    || WORKFLOW_STATES.contains(&self.state.document_status.value.as_str())))
            .then(|| workflow.into()),
            summary: (summary.as_deref().unwrap_or("") != self.summary()).then_some(summary),
            markdown: (self.draft.notes.text != self.notes).then(|| self.draft.notes.text.clone()),
        }
    }

    fn title(&self) -> &str {
        &self.state.content.title
    }

    fn summary(&self) -> &str {
        self.state.content.summary.as_deref().unwrap_or("")
    }

    fn update_notes(&mut self, notes: String) {
        if self.draft.notes.text == self.notes {
            self.draft.notes.replace_text(&notes);
            self.draft.base_notes.clone_from(&notes);
        }
        self.notes = notes;
    }

    fn update_summary(&mut self, summary: &str) {
        if self.draft.summary.text == self.summary() {
            self.draft.summary.replace_text(summary);
            self.draft.base_summary = summary.into();
        }
        self.state.content.summary = Some(summary.into());
    }

    fn draft_conflicts(&self) -> bool {
        self.revision_conflict
            || (self.draft.base_notes != self.notes && self.draft.notes.text != self.notes)
            || (self.draft.base_summary != self.summary()
                && self.draft.summary.text != self.summary())
    }
}

const SEARCH_LIMIT: usize = 50;

#[derive(Clone, Copy)]
enum SearchSource {
    Title,
    Tag,
    Summary,
    Transcript(Option<usize>, u64),
    Notes,
}

impl SearchSource {
    fn label(self) -> String {
        match self {
            Self::Title => "Title".into(),
            Self::Tag => "Tag".into(),
            Self::Summary => "Summary".into(),
            Self::Transcript(_, time) => format!("Transcript · {}", timestamp(time)),
            Self::Notes => "Saved notes".into(),
        }
    }
}

struct SearchHit {
    id: String,
    source: SearchSource,
    excerpt: String,
}

#[derive(Default)]
struct SearchResults {
    hits: Vec<SearchHit>,
    truncated: bool,
    unreadable: usize,
    error: Option<String>,
}

pub enum TreeMove {
    Reorder { target: String, before: bool },
    Reparent(Option<String>),
}

pub struct Workspace {
    tree: TreeModel<Document>,
    selected: String,
    drop_target: Option<Target>,
    search: TextEdit,
    search_hits: Option<SearchResults>,
    transcript_match: Option<usize>,
    pub mode: Mode,
    pub detail: bool,
    speakers: bool,
    processing: Option<(String, f64, Status)>,
    imports: usize,
    issues: Vec<DocumentIssue>,
    issues_open: bool,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            tree: TreeModel::new().key(TREE_KEY),
            selected: String::new(),
            drop_target: None,
            search: TextEdit::new(""),
            search_hits: None,
            transcript_match: None,
            mode: Mode::Summary,
            detail: false,
            speakers: false,
            processing: None,
            imports: 0,
            issues: Vec::new(),
            issues_open: false,
        }
    }
}

impl Workspace {
    pub fn sample() -> Self {
        let mut workspace = Self::default();
        let tree = &mut workspace.tree;
        let mut product = Document::new(MAIN, "Product sync", "product-sync.m4a", "12:48", SUMMARY);
        product.update_notes(SAMPLE_NOTES.into());
        product.state.results.speaker_segments = TRANSCRIPT
            .iter()
            .enumerate()
            .map(|(index, &(time, name, text))| SpeakerTaggedSegment {
                transcript_segment_id: format!("sample-segment-{index}"),
                speaker_id: format!("speaker-{}", index % 2 + 1),
                speaker_profile_id: None,
                speaker_label: Some(name.into()),
                start_ms: time,
                end_ms: time + 5_000,
                text: text.into(),
            })
            .collect();
        product.sync_speaker_drafts();
        tree.insert(None, MAIN, product);
        tree.insert(
            Some(MAIN),
            "rollout",
            Document::new(
                "rollout",
                "Workspace rollout",
                "product-sync.m4a",
                "04:20",
                "Pilot cohort selection and the workspace release checklist.",
            ),
        );
        tree.insert(
            Some("rollout"),
            "actions",
            Document::new(
                "actions",
                "Action items",
                "product-sync.m4a",
                "01:50",
                "Confirm pilot participants and share the recovery walkthrough.",
            ),
        );
        tree.insert(None, "design", Document::new("design", "Design review", "design-review.m4a", "21:04", "The design team reviewed search, transcript readability and the new workspace sidebar."));
        tree.insert(
            None,
            "research",
            Document::new(
                "research",
                "Research notes",
                "research-notes.m4a",
                "08:03",
                "Participants described how they find decisions in long recordings.",
            ),
        );
        tree.expand_all();
        tree.select(MAIN);
        workspace.selected = MAIN.into();
        workspace
    }

    pub fn open_snapshot(&mut self, mut snapshot: DocumentSnapshot) {
        snapshot
            .documents
            .sort_by_key(|state| (state.hierarchy.depth, state.hierarchy.sibling_order));
        let mut tree = TreeModel::new().key(TREE_KEY);
        for state in snapshot.documents {
            let id = state.id().as_str().to_owned();
            let parent = state
                .hierarchy
                .parent_id
                .as_ref()
                .map(ProjectId::as_str)
                .filter(|id| tree.contains(id))
                .map(str::to_owned);
            tree.insert(parent.as_deref(), id, Document::from_state(state));
        }
        self.tree = tree;
        self.selected.clear();
        if let Some(id) = self.tree.visible_id(0).map(str::to_owned) {
            self.pick(&id);
        }
        self.issues = snapshot.issues;
        self.issues_open = false;
        self.clear_search();
        self.mode = Mode::Summary;
        self.detail = false;
        self.processing = None;
        self.drop_target = None;
    }

    pub fn insert_imported(&mut self, record: DocumentRecord) {
        let id = record.state.id().as_str().to_owned();
        self.tree.insert(None, id, Document::from_record(record));
        self.clear_search();
    }

    pub fn reconcile_snapshot(&mut self, snapshot: DocumentSnapshot, reveal: Option<&str>) {
        let ids: BTreeSet<_> = snapshot
            .documents
            .iter()
            .map(|state| state.id().clone())
            .collect();
        let mut documents: Vec<_> = snapshot
            .documents
            .into_iter()
            .map(|state| {
                let Some(previous) = self.tree.get(state.id().as_str()) else {
                    return Document::from_state(state);
                };
                if previous.state == state {
                    return previous.clone();
                }
                if previous.patch() == DocumentPatch::default() {
                    let mut updated = Document::from_state(state);
                    updated.keep_speaker_drafts(previous, None);
                    return updated;
                }
                let mut draft = previous.clone();
                draft.state.hierarchy = state.hierarchy;
                draft.revision_conflict = true;
                draft
            })
            .collect();
        for previous in self
            .tree
            .children(None)
            .flat_map(|id| self.tree.subtree(id))
        {
            if !ids.contains(previous.state.id()) && previous.patch() != DocumentPatch::default() {
                let mut draft = previous.clone();
                draft.revision_conflict = true;
                documents.push(draft);
            }
        }
        self.issues = snapshot.issues;
        self.replace_tree(documents, reveal);
    }

    pub fn reveal_tree_document(&self, id: &str, tx: &mut Tx) {
        tx.scroll_into_view_in(TREE_KEY, id);
    }

    pub fn has_selection(&self) -> bool {
        self.tree.contains(&self.selected)
    }

    pub fn has_unsaved_edits(&self) -> bool {
        self.tree.children(None).any(|id| {
            self.tree
                .subtree(id)
                .any(|document| document.patch() != DocumentPatch::default())
        })
    }

    pub fn pending_read(&self) -> Option<ProjectId> {
        if !self.has_selection() || !matches!(self.mode, Mode::Notes | Mode::Edit) {
            return None;
        }
        let doc = self.current();
        matches!(doc.note_read, NoteRead::Pending).then(|| doc.state.id().clone())
    }

    pub fn observed_document(&self) -> Option<ProjectId> {
        let doc = self.tree.get(&self.selected)?;
        matches!(doc.note_read, NoteRead::Ready).then(|| doc.state.id().clone())
    }

    pub fn observe_record(&mut self, record: DocumentRecord) {
        let Some(document) = self.tree.get_mut(record.state.id().as_str()) else {
            return;
        };
        if document.state == record.state
            && document.notes == record.markdown
            && matches!(document.note_read, NoteRead::Ready)
        {
            return;
        }
        if document.patch() == DocumentPatch::default() {
            let mut updated = Document::from_record(record);
            updated.keep_speaker_drafts(document, None);
            *document = updated;
        } else {
            document.revision_conflict = true;
        }
    }

    pub fn finish_read(&mut self, id: &ProjectId, result: Result<DocumentRecord, String>) {
        if let Some(document) = self.tree.get_mut(id.as_str()) {
            match result {
                Ok(record) => *document = Document::from_record(record),
                Err(error) if document.patch() == DocumentPatch::default() => {
                    document.note_read = NoteRead::Failed(error)
                }
                Err(_) => {}
            }
        }
    }

    pub fn finish_save(&mut self, record: DocumentRecord, submitted: &DocumentPatch) {
        self.apply_saved_record(record, submitted, None);
    }

    pub fn finish_speaker_correction(
        &mut self,
        record: DocumentRecord,
        correction: &SpeakerCorrection,
    ) {
        if let Some(document) = self.tree.get_mut(record.state.id().as_str())
            && document.patch() != DocumentPatch::default()
            && (document.notes != record.markdown || document.state.content != record.state.content)
        {
            document.revision_conflict = true;
            return;
        }
        self.apply_saved_record(record, &DocumentPatch::default(), Some(correction));
    }

    fn apply_saved_record(
        &mut self,
        record: DocumentRecord,
        submitted: &DocumentPatch,
        correction: Option<&SpeakerCorrection>,
    ) {
        if let Some(document) = self.tree.get_mut(record.state.id().as_str()) {
            let pending = document.patch();
            let mut saved = Document::from_record(record);
            saved.keep_speaker_drafts(document, correction);
            // A completion rebases only the acknowledged edit. New input and
            // the focused editor's selection stay where the user put them.
            let draft = &mut document.draft;
            if pending.title != submitted.title || draft.title.focused {
                std::mem::swap(&mut saved.draft.title, &mut draft.title);
            }
            if pending.tags != submitted.tags || draft.tags.focused {
                std::mem::swap(&mut saved.draft.tags, &mut draft.tags);
            }
            if pending.summary != submitted.summary || draft.summary.focused {
                std::mem::swap(&mut saved.draft.summary, &mut draft.summary);
            }
            if pending.markdown != submitted.markdown || draft.notes.focused {
                std::mem::swap(&mut saved.draft.notes, &mut draft.notes);
            }
            if pending.document_status != submitted.document_status {
                saved.draft.workflow = draft.workflow;
            }
            *document = saved;
        }
    }

    pub fn mark_save_conflict(&mut self, id: &ProjectId) {
        if let Some(document) = self.tree.get_mut(id.as_str()) {
            document.revision_conflict = true;
        }
    }

    fn current(&self) -> &Document {
        self.tree
            .get(&self.selected)
            .expect("preview selection belongs to the tree")
    }

    fn current_mut(&mut self) -> &mut Document {
        self.tree
            .get_mut(&self.selected)
            .expect("preview selection belongs to the tree")
    }

    pub fn pick(&mut self, id: &str) {
        if self.selected == id || !self.tree.contains(id) {
            return;
        }
        if let Some(previous) = self.tree.get_mut(&self.selected) {
            previous.draft.title.focused = false;
            previous.draft.notes.focused = false;
            previous.draft.tags.focused = false;
            previous.draft.summary.focused = false;
            for edit in previous.speaker_drafts.values_mut() {
                edit.focused = false;
            }
        }
        self.selected = id.to_owned();
        self.tree.expand_to(id);
        self.tree.select(id);
        self.mode = Mode::Summary;
        self.transcript_match = None;
    }

    fn tree_move_for(&self, key: &Key) -> Option<TreeMove> {
        let id = self.tree.cursor()?;
        let parent = self.tree.parent(id);
        let mut siblings = self.tree.children(parent);
        let mut previous = None;
        for sibling in siblings.by_ref() {
            if sibling == id {
                break;
            }
            previous = Some(sibling);
        }
        match key {
            Key::Up => previous.map(|id| TreeMove::Reorder {
                target: id.into(),
                before: true,
            }),
            Key::Down => siblings.next().map(|id| TreeMove::Reorder {
                target: id.into(),
                before: false,
            }),
            Key::Right => previous.map(|id| TreeMove::Reparent(Some(id.into()))),
            Key::Left => {
                parent.map(|id| TreeMove::Reparent(self.tree.parent(id).map(str::to_owned)))
            }
            _ => None,
        }
    }

    fn tree_facts(&self) -> Vec<DocumentState> {
        let mut facts = Vec::new();
        let mut parents = vec![None];
        while let Some(parent) = parents.pop() {
            for (order, id) in self.tree.children(parent).enumerate() {
                let doc = self.tree.get(id).expect("child belongs to the tree");
                let mut state = doc.state.clone();
                state.hierarchy.parent_id = parent.map(ProjectId::new);
                state.hierarchy.depth = self.tree.depth(id).expect("child has a depth") as u8;
                state.hierarchy.sibling_order = order as u32;
                facts.push(state);
                parents.push(Some(id));
            }
        }
        facts
    }

    fn move_document(&mut self, id: &str, movement: TreeMove) -> Result<(), ProjectTreeError> {
        let mut facts = self.tree_facts();
        let source = ProjectId::new(id);
        match movement {
            TreeMove::Reorder { target, before } => {
                reorder_project(&mut facts, &source, &ProjectId::new(target), before)?;
            }
            TreeMove::Reparent(parent) => {
                if self.tree.parent(id) == parent.as_deref() {
                    return Ok(());
                }
                let order = self.tree.children(parent.as_deref()).count() as u32;
                reparent_project(
                    &mut facts,
                    &source,
                    parent.as_deref().map(ProjectId::new).as_ref(),
                    order,
                )?;
            }
        }
        let documents = facts
            .into_iter()
            .map(|fact| {
                let mut moved = self
                    .tree
                    .get(fact.id().as_str())
                    .expect("moved document belongs to the tree")
                    .clone();
                moved.state.hierarchy = fact.hierarchy;
                moved
            })
            .collect();
        self.replace_tree(documents, Some(id));
        self.refresh_search();
        Ok(())
    }

    fn replace_tree(&mut self, mut documents: Vec<Document>, reveal: Option<&str>) {
        documents.sort_by_key(|doc| (doc.state.hierarchy.depth, doc.state.hierarchy.sibling_order));
        let mut tree = TreeModel::new().key(TREE_KEY);
        let mut expanded = Vec::new();
        for doc in documents {
            let id = doc.state.id().as_str().to_owned();
            if self.tree.is_expanded(&id) {
                expanded.push(id.clone());
            }
            let parent = doc
                .state
                .hierarchy
                .parent_id
                .as_ref()
                .map(ProjectId::as_str)
                .filter(|id| tree.contains(id))
                .map(str::to_owned);
            tree.insert(parent.as_deref(), id, doc);
        }
        for id in expanded {
            tree.set_expanded(&id, true);
        }
        if let Some(id) = reveal {
            tree.expand_to(id);
        }
        if tree.contains(&self.selected) {
            tree.expand_to(&self.selected);
            tree.select(&self.selected);
            self.tree = tree;
        } else {
            self.selected.clear();
            self.tree = tree;
            if let Some(id) = self.tree.visible_id(0).map(str::to_owned) {
                self.pick(&id);
            }
        }
    }

    fn reset_search_results(&mut self) {
        self.search_hits = None;
        self.transcript_match = None;
    }

    fn clear_search(&mut self) {
        self.search.clear();
        self.reset_search_results();
        if self.has_selection() {
            self.tree.expand_to(&self.selected);
            self.tree.select(&self.selected);
        }
    }

    fn submit_search(&mut self) {
        let query = self.search.text.trim().to_lowercase();
        if query.is_empty() {
            self.clear_search();
            return;
        }
        self.transcript_match = None;
        let hits = self.collect_hits(&query);
        self.search_hits = Some(SearchResults {
            truncated: hits.len() > SEARCH_LIMIT,
            hits,
            ..Default::default()
        });
    }

    fn refresh_search(&mut self) {
        if self.search_hits.is_some() {
            self.submit_search();
        }
    }

    pub fn finish_search(&mut self, result: Result<DocumentSearchResults, String>) {
        let results = match result {
            Ok(results) => SearchResults {
                truncated: results.truncated,
                unreadable: results.unreadable_count,
                hits: results
                    .hits
                    .into_iter()
                    .filter_map(|hit| {
                        let id = hit.document_id.as_str();
                        let document = self.tree.get(id)?;
                        let source = match hit.source {
                            PersistedSource::Title => SearchSource::Title,
                            PersistedSource::Tag => SearchSource::Tag,
                            PersistedSource::Summary => SearchSource::Summary,
                            PersistedSource::Markdown => SearchSource::Notes,
                            PersistedSource::Transcript => {
                                let index = hit.transcript_target.as_ref().and_then(|target| {
                                    (hit.document_revision == document.state.revision())
                                        .then(|| transcript_index(document, target))
                                        .flatten()
                                });
                                SearchSource::Transcript(index, hit.start_ms.unwrap_or_default())
                            }
                        };
                        Some(SearchHit {
                            id: id.into(),
                            source,
                            excerpt: hit.snippet,
                        })
                    })
                    .collect(),
                ..Default::default()
            },
            Err(error) => SearchResults {
                error: Some(error),
                ..Default::default()
            },
        };
        self.search_hits = Some(results);
    }

    fn collect_hits(&self, query: &str) -> Vec<SearchHit> {
        let matches = |text: &str| text.to_lowercase().contains(query);
        let mut stack: Vec<_> = self.tree.children(None).collect();
        stack.reverse();
        let mut hits = Vec::new();
        while let Some(id) = stack.pop() {
            let document = self.tree.get(id).expect("child belongs to the tree");
            let transcript = document
                .state
                .results
                .speaker_segments
                .iter()
                .enumerate()
                .find(|(_, segment)| matches(&segment.text));
            let hit = if matches(document.title()) {
                Some((SearchSource::Title, document.title()))
            } else if let Some(tag) = document.state.content.tags.iter().find(|tag| matches(tag)) {
                Some((SearchSource::Tag, tag.as_str()))
            } else if matches(document.summary()) {
                Some((SearchSource::Summary, document.summary()))
            } else if let Some((index, segment)) = transcript {
                Some((
                    SearchSource::Transcript(Some(index), segment.start_ms),
                    segment.text.as_str(),
                ))
            } else {
                document
                    .notes
                    .lines()
                    .find(|text| matches(text))
                    .map(|text| (SearchSource::Notes, text))
                    .or_else(|| {
                        matches(&document.notes)
                            .then_some((SearchSource::Notes, document.notes.as_str()))
                    })
            };
            if let Some((source, excerpt)) = hit {
                hits.push(SearchHit {
                    id: id.into(),
                    source,
                    excerpt: excerpt.into(),
                });
                if hits.len() > SEARCH_LIMIT {
                    break;
                }
            }
            let start = stack.len();
            stack.extend(self.tree.children(Some(id)));
            stack[start..].reverse();
        }
        hits
    }

    pub fn import(&mut self) -> String {
        self.imports += 1;
        let id = format!("imported-{}", self.imports);
        let mut document = Document::new(
            &id,
            &format!("Sample recording {}", self.imports),
            "sample-recording.m4a",
            "03:24",
            "This sample is waiting to be processed.",
        );
        document.status = Status::Imported;
        self.clear_search();
        self.tree.insert(None, id.as_str(), document);
        id
    }

    fn toggle_processing(&mut self, now: f64, tx: &mut Tx) {
        let selected = self.selected.clone();
        let document = self.tree.get_mut(&selected).unwrap();
        if let Some((_, _, previous)) = self.processing.take() {
            document.status = previous;
        } else {
            self.processing = Some((selected, now, document.status));
            document.status = Status::Processing;
            tx.wake_after(Duration::from_millis(120));
        }
    }

    pub fn poll(&mut self, now: f64, tx: &mut Tx) -> bool {
        if let Some((id, start, _)) = &self.processing {
            if now - start >= 3.0 {
                let document = self.tree.get_mut(id).unwrap();
                document.status = Status::Ready;
                if document.state.source.audio_path == "sample-recording.m4a" {
                    document.update_summary("Sample transcription finished. This recording contains a short workspace onboarding walkthrough.");
                }
                self.processing = None;
                self.refresh_search();
            } else {
                tx.wake_after(Duration::from_millis(120));
            }
            true
        } else {
            false
        }
    }

    fn save(&mut self) -> bool {
        let document = self.current_mut();
        let title = document.draft.title.text.trim();
        if title.is_empty() || document.draft_conflicts() {
            return false;
        }
        document.state.content.title = title.into();
        document.state.content.tags = parse_document_tags(&document.draft.tags.text);
        document.state.document_status.value = WORKFLOW_STATES[document.draft.workflow].into();
        document.state.content.summary = (!document.draft.summary.text.trim().is_empty())
            .then(|| document.draft.summary.text.clone());
        document.notes.clone_from(&document.draft.notes.text);
        document.draft.base_summary = document.summary().into();
        document.draft.base_notes.clone_from(&document.notes);
        self.refresh_search();
        self.mode = Mode::Notes;
        true
    }

    fn reload_changed_content(&mut self) {
        let doc = self.current_mut();
        if doc.draft.base_notes != doc.notes {
            doc.draft.notes.replace_text(&doc.notes);
            doc.draft.base_notes.clone_from(&doc.notes);
        }
        if doc.draft.base_summary != doc.summary() {
            let summary = doc.summary().to_owned();
            doc.draft.summary.replace_text(&summary);
            doc.draft.base_summary = summary;
        }
    }

    pub fn selection(&self) -> (&str, &str) {
        (&self.selected, self.current().title())
    }

    pub fn document_states(&self) -> impl Iterator<Item = &DocumentState> {
        self.tree
            .children(None)
            .flat_map(|id| self.tree.subtree(id))
            .map(|document| &document.state)
    }

    pub fn selected_state(&self) -> Option<&DocumentState> {
        self.tree
            .get(&self.selected)
            .map(|document| &document.state)
    }

    pub fn append_action(&mut self, id: &str) -> Option<(String, String)> {
        let document = self.tree.get_mut(id)?;
        let before = document.notes.clone();
        let after = format!("{before}\n\n- [ ] Share the workspace recovery checklist.");
        document.update_notes(after.clone());
        self.refresh_search();
        Some((before, after))
    }

    pub fn restore_notes(&mut self, id: &str, before: &str, after: &str) -> bool {
        let Some(document) = self.tree.get_mut(id) else {
            return false;
        };
        if document.notes != after {
            return false;
        }
        document.update_notes(before.into());
        self.refresh_search();
        true
    }
}

impl MeetNote {
    fn submit_document_search(&mut self, tx: &mut Tx) {
        let query = self.documents.search.text.trim();
        if query.is_empty() {
            self.workspace_host.cancel_search();
            self.documents.clear_search();
        } else if self.workspace_host.live.is_some() {
            self.workspace_host.search(query.to_owned(), SEARCH_LIMIT);
            self.documents.reset_search_results();
            tx.wake_after(Duration::ZERO);
        } else {
            self.documents.submit_search();
        }
        tx.scroll_to_y("workspace-search-results", 0.0);
    }

    pub fn refresh_workspace_search(&mut self) {
        if self.documents.search_hits.is_some() || self.workspace_host.searching() {
            self.workspace_host
                .search(self.documents.search.text.trim().to_owned(), SEARCH_LIMIT);
        }
    }

    pub fn document_tree_shortcut(&mut self, event: &KeyEvent, tx: &mut Tx) -> bool {
        if tx.focused() != Some(TREE_KEY)
            || !event.mods.alt
            || event.mods.ctrl
            || event.mods.cmd
            || event.mods.shift
            || !matches!(event.key, Key::Up | Key::Down | Key::Left | Key::Right)
        {
            return false;
        }
        if let (Some(movement), Some(source)) = (
            self.documents.tree_move_for(&event.key),
            self.documents.tree.cursor().map(str::to_owned),
        ) {
            self.move_document(&source, movement, tx);
        }
        true
    }

    fn move_document(&mut self, source: &str, movement: TreeMove, tx: &mut Tx) {
        if self.workspace_host.live.is_some() {
            if self.workspace_host.busy() {
                self.notify("The workspace is busy", tx);
                return;
            }
            if let TreeMove::Reparent(parent) = &movement
                && self.documents.tree.parent(source) == parent.as_deref()
            {
                return;
            }
            if let Some(document) = self.documents.tree.get(source) {
                self.move_workspace_document(
                    document.state.id().clone(),
                    document.state.revision(),
                    movement,
                );
                tx.wake_after(Duration::ZERO);
            }
            return;
        }
        match self.documents.move_document(source, movement) {
            Ok(()) => tx.scroll_into_view_in(TREE_KEY, source),
            Err(error) => self.notify(
                match error {
                    ProjectTreeError::CrossAudioAsset => {
                        "Move requires the same audio source or sibling group"
                    }
                    ProjectTreeError::Cycle => "Cannot move a document into its own branch",
                    ProjectTreeError::MaximumDepth => "Documents can be nested up to three levels",
                    _ => "The document or drop target is unavailable",
                },
                tx,
            ),
        }
    }

    pub fn save_document(&mut self, tx: &mut Tx) {
        if !self.documents.has_selection() {
            return;
        }
        if self.workspace_host.live.is_some() {
            let document = self.documents.current();
            if document.draft_conflicts()
                || document.draft.title.text.trim().is_empty()
                || !matches!(document.note_read, NoteRead::Ready)
            {
                return;
            }
            let patch = document.patch();
            if patch != DocumentPatch::default() {
                tx.wake_after(Duration::ZERO);
                self.save_workspace_document(
                    document.state.id().clone(),
                    document.state.revision(),
                    patch
                        .markdown
                        .as_ref()
                        .map(|_| document.draft.base_notes.clone()),
                    patch,
                );
            }
        } else if self.documents.save() {
            self.notify("Document saved in preview", tx);
        }
    }

    fn drop_document(&mut self, source: &str, target: Target, tx: &mut Tx) {
        let movement = match target.placement {
            Placement::Inside => TreeMove::Reparent(Some(target.id)),
            Placement::Before | Placement::After => TreeMove::Reorder {
                target: target.id,
                before: target.placement == Placement::Before,
            },
        };
        self.move_document(source, movement, tx);
    }
}

pub fn sidebar(s: &MeetNote, k: &Kit) -> El {
    let status = (k.c.success.icon, k.c.info.icon, k.c.text_muted);
    let style = TreeStyle::from_kit(k).fade_long_labels(12.0);
    let tree = Tree::new(
        |s: &MeetNote| &s.documents.tree,
        |s: &mut MeetNote| &mut s.documents.tree,
        Document::title,
    )
    .name(s.tr("audio_projects"))
    .style(style)
    .row(move |document, row| {
        TreeItem::new(document.title())
            .icon(if row.expanded.is_some() {
                &glyphs::FOLDER
            } else {
                &glyphs::FILE_TEXT
            })
            .dot(match document.status {
                Status::Ready => status.0,
                Status::Processing => status.1,
                Status::Imported => status.2,
            })
    })
    .drag(TreeDrag::new(
        |s: &MeetNote| &s.documents.drop_target,
        |s| &mut s.documents.drop_target,
        MeetNote::drop_document,
    ))
    .on_select(|s, id, _| s.select_document(id))
    .build(s, k)
    .grow(1.0)
    .min_h(0.0);

    col()
        .grow(1.0)
        .min_h(0.0)
        .gap(11.0)
        .child(eyebrow(k, s.tr("audio_projects")))
        .child(
            TextInput::new(&s.documents.search)
                .label(s.tr("workspace_search"))
                .placeholder(s.tr("search_placeholder"))
                .leading_icon(&glyphs::SEARCH)
                .width(None)
                .clear_button()
                .on_action(|s: &mut MeetNote, _| {
                    s.workspace_host.cancel_search();
                    s.documents.clear_search();
                })
                .on_edit(|s: &mut MeetNote| &mut s.documents.search)
                .on_change(|s: &mut MeetNote, _| {
                    s.workspace_host.cancel_search();
                    s.documents.reset_search_results();
                })
                .on_submit(MeetNote::submit_document_search)
                .build(k)
                .key("workspace-search"),
        )
        .child(if s.workspace_host.searching() {
            caption(k, s.tr("search_pending"))
        } else if let Some(results) = &s.documents.search_hits {
            search_results(s, k, results)
        } else {
            tree
        })
}

fn search_results(s: &MeetNote, k: &Kit, search: &SearchResults) -> El {
    if let Some(error) = &search.error {
        return caption(k, error);
    }
    let hits = &search.hits;
    let count = if search.truncated {
        format!("{SEARCH_LIMIT}+ matches")
    } else if hits.len() == 1 {
        "1 match".into()
    } else {
        format!("{} matches", hits.len())
    };
    let mut results = col().w_full().gap(10.0).pb(10.0).child(caption(k, count));
    if hits.is_empty() {
        results = results.child(caption(k, s.tr("search_empty")));
    }
    if search.unreadable > 0 {
        results = results.child(caption(
            k,
            format!("{} documents could not be read.", search.unreadable),
        ));
    }
    for hit in hits.iter().take(SEARCH_LIMIT) {
        let id = hit.id.clone();
        let source = hit.source;
        let document = s
            .documents
            .tree
            .get(&id)
            .expect("search hit belongs to the tree");
        results = results.child(
            col()
                .w_full()
                .gap(3.0)
                .child(
                    rune_kit::SidebarRow::new(document.title())
                        .selected(s.documents.selected == id)
                        .on_select({
                            let id = id.clone();
                            move |s: &mut MeetNote, tx| {
                                s.select_document(&id);
                                let (mode, paragraph) = match source {
                                    SearchSource::Transcript(index, _) => {
                                        if index.is_none() {
                                            s.notify(s.tr("search_transcript_changed"), tx);
                                        }
                                        (Mode::Original, index)
                                    }
                                    SearchSource::Notes => (Mode::Notes, None),
                                    _ => (Mode::Summary, None),
                                };
                                s.documents.mode = mode;
                                s.documents.transcript_match = paragraph;
                                if let Some(index) = paragraph {
                                    tx.scroll_center_in_instant(
                                        "document-scroll",
                                        format!("transcript-{index}"),
                                    );
                                } else {
                                    tx.scroll_to_y("document-scroll", 0.0);
                                }
                            }
                        })
                        .build(k)
                        .key(format!("search-result-{id}")),
                )
                .child(eyebrow(k, source.label()).px(8.0))
                .child(caption(k, &hit.excerpt).max_lines(3).px(8.0)),
        );
    }
    scroll(k, "workspace-search-results", results)
}

pub fn page(s: &MeetNote, k: &Kit, overlays: &mut Vec<El>) -> El {
    let w = &s.documents;
    if !w.has_selection() {
        return col()
            .key("documents-page")
            .w_full()
            .h_full()
            .padding(24.0)
            .gap(14.0)
            .child(workspace_issues(w, k))
            .child(title(k, s.tr("recordings_empty_title"), 30.0))
            .child(caption(k, s.tr("recordings_empty_hint")))
            .child(
                Button::new(s.tr("import_audio"))
                    .on_press_tx(MeetNote::import_audio)
                    .disabled(s.workspace_host.busy())
                    .build(k)
                    .key("empty-import-audio"),
            );
    }
    let document = w.current();
    let status = match (s.processing_here(), document.status) {
        (true, _) | (_, Status::Processing) => s.tr("processing"),
        (_, Status::Imported) => s.tr("recording_imported"),
        (_, Status::Ready) => s.tr("ready"),
    };
    let toolbar = row()
        .items_center()
        .gap(8.0)
        .flex_wrap()
        .child(
            SegmentedControl::new(
                &[
                    s.tr("summary"),
                    s.tr("original"),
                    s.tr("saved_markdown"),
                    s.tr("edit"),
                ],
                Mode::ALL.iter().position(|mode| *mode == w.mode),
            )
            .label(s.tr("content_mode"))
            .fit()
            .on_pick(|s: &mut MeetNote, index, tx| {
                s.documents.mode = Mode::ALL[index];
                s.documents.transcript_match = None;
                tx.scroll_to_y("document-scroll", 0.0);
            })
            .build(k)
            .key("document-modes"),
        )
        .child(el().grow(1.0))
        .child(action(
            k,
            "detail-toggle",
            if w.detail {
                s.tr("hide_detail")
            } else {
                s.tr("document_details")
            },
            ButtonVariant::Outline,
            |s| s.documents.detail = !s.documents.detail,
        ))
        .child(if s.workspace_host.live.is_some() {
            let running = s.processing_here();
            Button::new(if running {
                s.tr("cancel_processing")
            } else if document.state.processing.latest.is_some() {
                s.tr("reprocess")
            } else {
                s.tr("process")
            })
            .variant(ButtonVariant::Submit)
            .disabled(!running && !s.can_process_audio())
            .on_press_tx(MeetNote::process_document)
            .build(k)
            .key("process-document")
        } else {
            Button::new(if document.status == Status::Processing {
                "Cancel"
            } else {
                "Process sample"
            })
            .variant(ButtonVariant::Submit)
            .disabled(
                w.processing
                    .as_ref()
                    .is_some_and(|(id, _, _)| id != &w.selected),
            )
            .on_press_tx(|s: &mut MeetNote, tx| s.documents.toggle_processing(s.now, tx))
            .build(k)
            .key("process-sample")
        });
    let reading = match w.mode {
        Mode::Summary => summary(
            s,
            k,
            document,
            w.selected == MAIN && s.workspace_host.live.is_none(),
        ),
        Mode::Original => original(s, k),
        Mode::Notes | Mode::Edit => match &document.note_read {
            NoteRead::Pending => col()
                .padding(24.0)
                .child(caption(k, s.tr("document_loading"))),
            NoteRead::Failed(error) => col()
                .padding(24.0)
                .gap(12.0)
                .child(caption(k, error))
                .child(action(
                    k,
                    "retry-document",
                    s.tr("retry_document_read"),
                    ButtonVariant::Outline,
                    |s| {
                        s.documents.current_mut().note_read = NoteRead::Pending;
                    },
                )),
            NoteRead::Ready if w.mode == Mode::Notes => notes(s, k),
            NoteRead::Ready => edit(s, k, overlays),
        },
    };
    let body = row().grow(1.0).min_h(0.0).w_full().child(
        col()
            .grow(1.0)
            .min_w(0.0)
            .min_h(0.0)
            .child(scroll(k, "document-scroll", reading)),
    );
    let body = if w.detail {
        body.child(Divider::vertical().build(k))
            .child(details::panel(s, k))
    } else {
        body
    };
    let header = col()
        .w_full()
        .gap(13.0)
        .px(24.0)
        .pt(21.0)
        .pb(14.0)
        .child(eyebrow(k, s.tr("documents_workspace")))
        .child(title(k, document.title(), 31.0).key("document-title"))
        .child(
            row()
                .items_center()
                .gap(10.0)
                .flex_wrap()
                .child(Badge::new(status).variant(BadgeVariant::Accent).build(k))
                .child(
                    Badge::new(workflow_label(s, &document.state.document_status.value))
                        .variant(BadgeVariant::Subtle)
                        .build(k),
                )
                .children(
                    document
                        .state
                        .content
                        .tags
                        .iter()
                        .map(|tag| Badge::new(tag).build(k)),
                )
                .child(caption(
                    k,
                    format!(
                        "{}   ·   {}",
                        document.state.audio_path(),
                        document.duration
                    ),
                )),
        )
        .child(toolbar)
        .children(s.workspace_host.live.is_some().then(|| audio_source(s, k)));
    let header = if let Some((label, [asr, diarization])) = s.processing_progress() {
        header.child(
            col()
                .w_full()
                .gap(4.0)
                .key("audio-processing-progress")
                .child(caption(k, label))
                .child(
                    row().w_full().gap(12.0).children(
                        [("Transcription", asr), ("Speaker diarization", diarization)]
                            .into_iter()
                            .map(|(stage, fraction)| {
                                col()
                                    .grow(1.0)
                                    .min_w(0.0)
                                    .gap(3.0)
                                    .child(caption(k, stage))
                                    .child(
                                        Progress::new(fraction).label(stage).show_value().build(k),
                                    )
                            }),
                    ),
                ),
        )
    } else if let Some((id, started, _)) = &w.processing {
        if id == &w.selected {
            header.child(
                Progress::new(Some(((s.now - started) / 3.0) as f32))
                    .label("Sample audio processing")
                    .show_value()
                    .build(k)
                    .key("audio-progress"),
            )
        } else {
            header
        }
    } else {
        header
    };
    col()
        .key("documents-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(workspace_issues(w, k))
        .child(header)
        .child(Divider::horizontal().build(k))
        .child(body)
}

fn workspace_issues(w: &Workspace, k: &Kit) -> El {
    if w.issues.is_empty() {
        return el();
    }
    col().px(24.0).pt(8.0).child(
        Collapsible::new(
            format!("{} workspace scan issues", w.issues.len()),
            w.issues_open,
        )
        .on_toggle(|s: &mut MeetNote| s.documents.issues_open = !s.documents.issues_open)
        .child(
            col().gap(4.0).children(
                w.issues
                    .iter()
                    .map(|issue| caption(k, format!("{} — {}", issue.path, issue.message))),
            ),
        )
        .build(k),
    )
}

fn audio_source_status(s: &MeetNote) -> (&'static str, &str, bool) {
    match s.audio_status() {
        None => ("audio-source-checking", s.tr("audio_checking"), false),
        Some(Ok(AudioStatus::Missing { .. })) => {
            ("audio-source-missing", s.tr("audio_missing"), true)
        }
        Some(Ok(AudioStatus::ChecksumMismatch { .. })) => {
            ("audio-source-mismatch", s.tr("audio_source_mismatch"), true)
        }
        Some(Ok(AudioStatus::Present { .. })) => (
            "audio-source-verified",
            s.tr("audio_source_verified"),
            false,
        ),
        Some(Ok(AudioStatus::Unverifiable { .. })) => (
            "audio-source-available",
            s.tr("audio_source_available"),
            false,
        ),
        Some(Err(error)) => ("audio-source-error", error.as_str(), true),
    }
}

fn audio_source(s: &MeetNote, k: &Kit) -> El {
    let (key, message, relink) = audio_source_status(s);
    row()
        .items_center()
        .gap(8.0)
        .flex_wrap()
        .child(caption(k, message).key(key))
        .children(relink.then(|| {
            Button::new(s.tr("relink_audio"))
                .variant(ButtonVariant::Outline)
                .disabled(s.workspace_host.busy())
                .on_press_tx(MeetNote::relink_audio)
                .build(k)
                .key("relink-audio")
        }))
        .children(s.audio_status().is_some().then(|| {
            Button::new(s.tr("recheck_audio_source"))
                .variant(ButtonVariant::Outline)
                .on_press_tx(MeetNote::recheck_audio)
                .build(k)
                .key("check-audio")
        }))
}

fn summary(s: &MeetNote, k: &Kit, document: &Document, show_actions: bool) -> El {
    let content = col().w_full().gap(16.0).padding(24.0).child(
        card(k)
            .child(eyebrow(k, "OVERVIEW"))
            .child(title(k, s.tr("summary"), 20.0))
            .child(label(k, document.summary())),
    );
    if !show_actions {
        return content;
    }
    content
        .child(
            card(k)
                .child(title(k, "Decisions", 18.0))
                .child(bullet(k, "Start with a small pilot cohort."))
                .child(bullet(
                    k,
                    "Prioritize import recovery and transcript search.",
                )),
        )
        .child(
            card(k)
                .child(title(k, "Action items", 18.0))
                .child(action_item(
                    k,
                    "MP",
                    "Share indexing measurements",
                    "Morgan Park  ·  Thursday",
                ))
                .child(Divider::horizontal().build(k))
                .child(action_item(
                    k,
                    "AC",
                    "Confirm pilot participants",
                    "Alex Chen  ·  Friday",
                )),
        )
}

fn bullet(k: &Kit, value: &str) -> El {
    row()
        .items_center()
        .gap(8.0)
        .child(dot(k.c.text_accent))
        .child(label(k, value))
}

fn action_item(k: &Kit, initials: &str, name: &str, metadata: &str) -> El {
    row()
        .items_center()
        .gap(11.0)
        .child(
            col()
                .size(32.0, 32.0)
                .radius(8.0)
                .fill(k.c.bg_deep)
                .center()
                .child(eyebrow(k, initials)),
        )
        .child(
            col()
                .gap(2.0)
                .child(label(k, name))
                .child(caption(k, metadata)),
        )
}

fn timestamp(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    if seconds >= 3_600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3_600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{:02}:{:02}", seconds / 60, seconds % 60)
    }
}

fn workflow_label<'a>(s: &MeetNote, status: &'a str) -> &'a str {
    WORKFLOW_STATES
        .iter()
        .position(|state| *state == status)
        .map_or(status, |index| s.tr(WORKFLOW_LABELS[index]))
}

fn transcript_index(document: &Document, target: &TranscriptSearchTarget) -> Option<usize> {
    let results = &document.state.results;
    if results.speaker_segments.is_empty() {
        results
            .transcript
            .iter()
            .position(|segment| TranscriptSearchTarget::from(segment) == *target)
    } else {
        results
            .speaker_segments
            .iter()
            .position(|segment| TranscriptSearchTarget::from(segment) == *target)
    }
}

fn transcript_lines(document: &Document) -> impl Iterator<Item = (u64, &str, &str)> {
    let results = &document.state.results;
    results
        .speaker_segments
        .iter()
        .map(|segment| {
            (
                segment.start_ms,
                speakers::name(segment),
                segment.text.as_str(),
            )
        })
        .chain(
            results
                .transcript
                .iter()
                .filter(|_| results.speaker_segments.is_empty())
                .map(|segment| (segment.start_ms, "", segment.text.as_str())),
        )
}

fn original(s: &MeetNote, k: &Kit) -> El {
    let mut body =
        col()
            .w_full()
            .gap(14.0)
            .padding(24.0)
            .child(title(k, s.tr("transcript_heading"), 20.0));
    let document = s.documents.current();
    if transcript_lines(document).next().is_none() {
        return body.child(card(k).child(caption(k, s.tr("transcript_empty"))));
    }
    body = body
        .child(icon_action(
            k,
            "copy-transcript",
            &glyphs::COPY,
            s.tr("copy_transcript"),
            |s, tx| {
                let text = transcript_lines(s.documents.current())
                    .map(|(start, speaker, text)| {
                        let time = timestamp(start);
                        if speaker.is_empty() {
                            format!("[{time}]\n{text}")
                        } else {
                            format!("[{time}] {speaker}\n{text}")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");
                tx.copy(text);
                s.notify(s.tr("transcript_copied"), tx);
            },
        ))
        .children((!document.speaker_drafts.is_empty()).then(|| {
            Collapsible::new(
                format!("Speakers · {}", document.speaker_drafts.len()),
                s.documents.speakers,
            )
            .on_toggle(|s: &mut MeetNote| s.documents.speakers = !s.documents.speakers)
            .child(speakers::controls(document, k, s.workspace_host.busy()))
            .build(k)
            .key("speakers")
        }));
    for (index, (start, speaker, text)) in transcript_lines(document).enumerate() {
        let paragraph = card(k).key(format!("transcript-{index}"));
        let header = row()
            .items_center()
            .gap(12.0)
            .child(eyebrow(k, timestamp(start)))
            .children((!speaker.is_empty()).then(|| title(k, speaker, 14.0)));
        let (paragraph, header) = if s.documents.transcript_match == Some(index) {
            (
                paragraph.border(1.0, k.c.text_accent),
                header.child(
                    Badge::new(s.tr("search_match"))
                        .variant(BadgeVariant::Accent)
                        .build(k),
                ),
            )
        } else {
            (paragraph, header)
        };
        body = body.child(
            paragraph
                .child(header)
                .child(label(k, text).selectable(true)),
        );
    }
    body
}

fn notes(s: &MeetNote, k: &Kit) -> El {
    let saved = &s.documents.current().notes;
    let heading = row()
        .items_center()
        .justify_between()
        .child(title(k, s.tr("saved_notes_heading"), 20.0))
        .child(if saved.is_empty() {
            el()
        } else {
            icon_action(
                k,
                "copy-notes",
                &glyphs::COPY,
                s.tr("copy_saved_markdown"),
                |s, tx| {
                    tx.copy(s.documents.current().notes.clone());
                    s.notify(s.tr("saved_markdown_copied"), tx);
                },
            )
        });
    let body = if saved.is_empty() {
        caption(k, s.tr("saved_notes_empty"))
    } else {
        Markdown::new(saved.clone()).selectable(true).build(k)
    };
    col()
        .w_full()
        .gap(14.0)
        .padding(24.0)
        .child(heading)
        .child(card(k).child(body))
}

fn edit(s: &MeetNote, k: &Kit, overlays: &mut Vec<El>) -> El {
    let document = s.documents.current();
    let draft = &document.draft;
    col()
        .w_full()
        .gap(14.0)
        .padding(24.0)
        .child(title(k, s.tr("edit_document_heading"), 20.0))
        .child(
            card(k)
                .child(
                    TextField::new(s.tr("document_title"), &draft.title)
                        .on_edit(|s: &mut MeetNote| &mut s.documents.current_mut().draft.title)
                        .build(k)
                        .key("document-title-edit"),
                )
                .child(TextField::new(s.tr("document_tags"), &draft.tags)
                    .placeholder(s.tr("document_tags_placeholder"))
                    .on_edit(|s: &mut MeetNote| &mut s.documents.current_mut().draft.tags)
                    .build(k).key("document-tags-edit"))
                .child(row().items_center().gap(12.0)
                    .child(label(k, s.tr("document_status")))
                    .child(select(s, k, overlays, ("document-status", s.tr("document_status")), &WORKFLOW_LABELS.map(|key| s.tr(key)),
                        draft.workflow, |s, choice| s.documents.current_mut().draft.workflow = choice)))
                .child(label(k, s.tr("summary")))
                .child(TextArea::new(&draft.summary).label(s.tr("summary")).rows(3)
                    .on_edit(|s: &mut MeetNote| &mut s.documents.current_mut().draft.summary)
                    .build(k).key("document-summary-edit"))
                .child(label(k, s.tr("document_body")))
                .child(
                    TextArea::new(&draft.notes)
                        .label(s.tr("document_body"))
                        .rows(8)
                        .on_edit(|s: &mut MeetNote| &mut s.documents.current_mut().draft.notes)
                        .build(k)
                        .key("document-notes-edit"),
                )
                .children(document.draft_conflicts().then(|| {
                    col().gap(8.0)
                        .child(caption(k, if s.workspace_host.live.is_some() {
                            s.tr("document_conflict_hint")
                        } else {
                            "Saved content changed while you were editing. Reload the updated fields to continue."
                        }))
                        .child(Button::new(if s.workspace_host.live.is_some() {
                            s.tr("reload")
                        } else {
                            "Reload changed fields"
                        }).variant(ButtonVariant::Outline)
                        .disabled(s.workspace_host.busy())
                        .on_press_tx(|s: &mut MeetNote, tx| {
                            if s.workspace_host.live.is_some() {
                                s.load_document(s.documents.current().state.id().clone());
                                tx.wake_after(Duration::ZERO);
                            } else {
                                s.documents.reload_changed_content();
                            }
                        }).build(k).key("reload-draft-notes"))
                }))
                .child(
                    Button::new(if s.workspace_host.live.is_some() { s.tr("save_document") } else { "Save preview changes" })
                        .variant(ButtonVariant::Submit)
                        .disabled(s.workspace_host.busy() || draft.title.text.trim().is_empty() || document.draft_conflicts())
                        .on_press_tx(|s: &mut MeetNote, tx| {
                            s.save_document(tx);
                        })
                        .build(k)
                        .key("save-document"),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_saved_notes_block_a_stale_draft_until_explicit_reload() {
        let mut workspace = Workspace::sample();
        let draft = &mut workspace.current_mut().draft;
        draft.title.replace_text("New title");
        draft.notes.replace_text("Unsaved notes");
        workspace.pick("design");
        workspace.append_action(MAIN).unwrap();
        workspace.pick(MAIN);

        assert!(!workspace.save());
        assert_eq!(workspace.current().draft.notes.text, "Unsaved notes");
        assert_eq!(workspace.current().title(), "Product sync");
        assert!(
            workspace
                .current()
                .notes
                .contains("workspace recovery checklist")
        );

        workspace.reload_changed_content();
        assert!(workspace.save());
        assert_eq!(workspace.current().title(), "New title");
        assert!(
            workspace
                .current()
                .notes
                .contains("workspace recovery checklist")
        );
    }
}
