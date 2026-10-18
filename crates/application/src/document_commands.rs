use crate::{render_markdown, DocumentService};
use contracts::{
    DocumentCommand, DocumentCommandEnvelope, DocumentCommandIssue, DocumentCommandResult,
    DocumentPatch, DocumentRecord, DOCUMENT_COMMAND_SCHEMA_VERSION,
};
use domain::{
    reorder_project, reparent_project, DocumentId, DocumentState, ProjectTreeError,
    SpeakerCorrection, SpeakerCorrectionError,
};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum DocumentCommandError {
    #[error("unsupported document command schema: {actual}, expected {expected}")]
    UnsupportedSchema { actual: u32, expected: u32 },
    #[error("document service error: {0}")]
    Service(String),
    #[error("document {0} does not exist")]
    NotFound(DocumentId),
    /// `document_id` is not at the revision the command was based on; the
    /// command wrote nothing further.
    #[error(
        "document {document_id} changed since it was read: expected revision {expected}, found {actual}"
    )]
    RevisionConflict {
        document_id: DocumentId,
        expected: u64,
        actual: u64,
    },
    #[error("cannot move document: {0}")]
    Tree(ProjectTreeError),
    #[error("cannot correct speakers: {0}")]
    Speaker(SpeakerCorrectionError),
    #[error("invalid document command: {0}")]
    Invalid(String),
}

/// Revision-checked document writes shared by the desktop UI, CLI/IPC
/// adapters and tests. Every write names the revision it was based on and
/// fails with [`DocumentCommandError::RevisionConflict`] instead of
/// overwriting a newer document.
#[derive(Clone)]
pub struct DocumentCommandService {
    documents: DocumentService,
}

struct SemanticChildRequest {
    parent_id: DocumentId,
    expected_parent_revision: u64,
    title: String,
    start_ms: u64,
    end_ms: u64,
    tags: Vec<String>,
    markdown: String,
}

impl DocumentCommandService {
    /// Follows `documents` between commands, while each command keeps one
    /// repository from its first read through its final result.
    pub fn new(documents: DocumentService) -> Self {
        Self { documents }
    }

    pub fn execute(
        &self,
        envelope: DocumentCommandEnvelope,
    ) -> Result<DocumentCommandResult, DocumentCommandError> {
        if envelope.schema_version != DOCUMENT_COMMAND_SCHEMA_VERSION {
            return Err(DocumentCommandError::UnsupportedSchema {
                actual: envelope.schema_version,
                expected: DOCUMENT_COMMAND_SCHEMA_VERSION,
            });
        }
        let commands = Self::new(self.documents.frozen());
        match envelope.command {
            DocumentCommand::List => commands.list(),
            DocumentCommand::Read { document_id } => Ok(DocumentCommandResult::Document(
                commands.read(&document_id)?,
            )),
            DocumentCommand::Create {
                title,
                audio_path,
                markdown,
                tags,
            } => commands.create(title, audio_path, markdown, tags),
            DocumentCommand::Update {
                document_id,
                expected_revision,
                patch,
            } => Ok(DocumentCommandResult::Updated(commands.update_document(
                &document_id,
                expected_revision,
                patch,
            )?)),
            DocumentCommand::CreateSemanticChild {
                parent_id,
                expected_parent_revision,
                title,
                start_ms,
                end_ms,
                tags,
                markdown,
            } => commands.create_semantic_child(SemanticChildRequest {
                parent_id,
                expected_parent_revision,
                title,
                start_ms,
                end_ms,
                tags,
                markdown,
            }),
            DocumentCommand::Reparent {
                document_id,
                expected_revision,
                new_parent_id,
                sibling_order,
            } => {
                commands.reparent_document(
                    &document_id,
                    expected_revision,
                    new_parent_id.as_ref(),
                    sibling_order,
                )?;
                commands.list()
            }
            DocumentCommand::Reorder {
                document_id,
                expected_revision,
                target_id,
                before,
            } => {
                commands.reorder_document(&document_id, expected_revision, &target_id, before)?;
                commands.list()
            }
            DocumentCommand::MergeSpeaker {
                document_id,
                expected_revision,
                from_speaker_id,
                into_speaker_id,
            } => Ok(DocumentCommandResult::Updated(commands.merge_speaker(
                &document_id,
                expected_revision,
                &from_speaker_id,
                &into_speaker_id,
            )?)),
            DocumentCommand::SetSpeakerLabel {
                document_id,
                expected_revision,
                speaker_id,
                label,
            } => Ok(DocumentCommandResult::Updated(commands.set_speaker_label(
                &document_id,
                expected_revision,
                &speaker_id,
                label,
            )?)),
        }
    }

    /// Applies `patch` to the semantic fields if the document is still at
    /// `expected_revision`. The stored Markdown is kept unless the patch
    /// replaces it; `processing` and `results` are never patched.
    pub fn update_document(
        &self,
        document_id: &DocumentId,
        expected_revision: u64,
        patch: DocumentPatch,
    ) -> Result<DocumentRecord, DocumentCommandError> {
        let commands = Self::new(self.documents.frozen());
        let record = commands.require(document_id)?;
        ensure_revision(&record.state, expected_revision)?;
        let mut state = record.state;
        apply_patch(&mut state, &patch)?;
        commands
            .documents
            .save_expecting_revision(state, expected_revision, patch.markdown)?;
        commands.require(document_id)
    }

    /// Moves a document under `new_parent_id` (a root for `None`) at
    /// `sibling_order` and returns the documents it wrote: the moved
    /// document and descendants whose depth changed. See
    /// [`Self::reorder_document`] for the conflict rules.
    pub fn reparent_document(
        &self,
        document_id: &DocumentId,
        expected_revision: u64,
        new_parent_id: Option<&DocumentId>,
        sibling_order: u32,
    ) -> Result<Vec<DocumentState>, DocumentCommandError> {
        let commands = Self::new(self.documents.frozen());
        let read = commands.read_tree(document_id, expected_revision)?;
        let mut moved = read.clone();
        reparent_project(&mut moved, document_id, new_parent_id, sibling_order)
            .map_err(DocumentCommandError::Tree)?;
        let anchors: Vec<_> = [Some(document_id), new_parent_id]
            .into_iter()
            .flatten()
            .collect();
        commands.write_tree_change(&read, moved, &anchors)
    }

    /// Places a document before or after its sibling `target_id` and
    /// returns the siblings whose order changed.
    ///
    /// Only documents whose hierarchy changes are written, each against the
    /// revision this command read; the others keep their revision. Before
    /// the first write, every document to be written, the moved document and
    /// the target are re-read; any revision change fails the command with
    /// [`DocumentCommandError::RevisionConflict`] and writes nothing. The
    /// writes themselves are one file save per document, not a transaction.
    pub fn reorder_document(
        &self,
        document_id: &DocumentId,
        expected_revision: u64,
        target_id: &DocumentId,
        before: bool,
    ) -> Result<Vec<DocumentState>, DocumentCommandError> {
        let commands = Self::new(self.documents.frozen());
        let read = commands.read_tree(document_id, expected_revision)?;
        let mut moved = read.clone();
        reorder_project(&mut moved, document_id, target_id, before)
            .map_err(DocumentCommandError::Tree)?;
        commands.write_tree_change(&read, moved, &[document_id, target_id])
    }

    /// Manual correction: attributes every segment of `from_speaker_id` to
    /// `into_speaker_id`, which keeps its profile identity and label.
    pub fn merge_speaker(
        &self,
        document_id: &DocumentId,
        expected_revision: u64,
        from_speaker_id: &str,
        into_speaker_id: &str,
    ) -> Result<DocumentRecord, DocumentCommandError> {
        Self::new(self.documents.frozen()).correct_speakers(
            document_id,
            expected_revision,
            &SpeakerCorrection::Merge {
                from: from_speaker_id.into(),
                into: into_speaker_id.into(),
            },
        )
    }

    /// Manual correction: sets (trimmed) or clears one speaker's display
    /// label. Speaker Library profile identities are left as matched.
    pub fn set_speaker_label(
        &self,
        document_id: &DocumentId,
        expected_revision: u64,
        speaker_id: &str,
        label: Option<String>,
    ) -> Result<DocumentRecord, DocumentCommandError> {
        Self::new(self.documents.frozen()).correct_speakers(
            document_id,
            expected_revision,
            &SpeakerCorrection::Label {
                speaker_id: speaker_id.into(),
                label,
            },
        )
    }

    /// Changes only speaker fields of `results.diarization` and
    /// `results.speaker_segments`. A body that is still the generated
    /// transcript is regenerated; an edited body is kept. A correction that
    /// changes nothing writes nothing.
    fn correct_speakers(
        &self,
        document_id: &DocumentId,
        expected_revision: u64,
        correction: &SpeakerCorrection,
    ) -> Result<DocumentRecord, DocumentCommandError> {
        let record = self.require(document_id)?;
        ensure_revision(&record.state, expected_revision)?;
        let mut state = record.state.clone();
        let changed = state
            .apply_speaker_correction(correction)
            .map_err(DocumentCommandError::Speaker)?;
        if changed == 0 {
            return Ok(record);
        }
        let audio_path = Path::new(record.state.audio_path());
        let markdown = (record.markdown
            == render_markdown(audio_path, &record.state.results.speaker_segments))
        .then(|| render_markdown(audio_path, &state.results.speaker_segments));
        self.documents
            .save_expecting_revision(state, expected_revision, markdown)?;
        self.require(document_id)
    }

    fn read_tree(
        &self,
        document_id: &DocumentId,
        expected_revision: u64,
    ) -> Result<Vec<DocumentState>, DocumentCommandError> {
        let documents = self
            .documents
            .snapshot()
            .map_err(ServiceError::from)?
            .documents;
        ensure_revision(find_document(&documents, document_id)?, expected_revision)?;
        Ok(documents)
    }

    /// Writes the documents of `moved` whose hierarchy differs from `read`
    /// (same order, as the domain tree functions edit in place). `anchors`
    /// are the documents the move was computed from (the moved document and
    /// its new parent or sibling target); they are re-checked with the
    /// written documents even when they are not written themselves.
    fn write_tree_change(
        &self,
        read: &[DocumentState],
        moved: Vec<DocumentState>,
        anchors: &[&DocumentId],
    ) -> Result<Vec<DocumentState>, DocumentCommandError> {
        let changed: Vec<(DocumentState, u64)> = read
            .iter()
            .zip(moved)
            .filter(|(before, after)| before.hierarchy != after.hierarchy)
            .map(|(before, after)| {
                debug_assert_eq!(before.id(), after.id());
                (after, before.revision())
            })
            .collect();
        if changed.is_empty() {
            return Ok(Vec::new());
        }
        let mut read_set: BTreeMap<&DocumentId, u64> = changed
            .iter()
            .map(|(document, revision)| (document.id(), *revision))
            .collect();
        for anchor in anchors {
            let anchor = find_document(read, anchor)?;
            read_set.insert(anchor.id(), anchor.revision());
        }
        for (document_id, revision) in read_set {
            ensure_revision(&self.require(document_id)?.state, revision)?;
        }
        changed
            .into_iter()
            .map(|(document, revision)| {
                self.documents
                    .save_expecting_revision(document, revision, None)
            })
            .collect()
    }

    fn require(&self, document_id: &DocumentId) -> Result<DocumentRecord, DocumentCommandError> {
        self.read(document_id)?
            .ok_or_else(|| DocumentCommandError::NotFound(document_id.clone()))
    }

    fn list(&self) -> Result<DocumentCommandResult, DocumentCommandError> {
        let snapshot = self.documents.snapshot().map_err(ServiceError::from)?;
        Ok(DocumentCommandResult::Snapshot {
            documents: snapshot.documents,
            issues: snapshot
                .issues
                .into_iter()
                .map(|issue| DocumentCommandIssue {
                    path: issue.path,
                    message: issue.message,
                })
                .collect(),
        })
    }

    fn read(
        &self,
        document_id: &DocumentId,
    ) -> Result<Option<DocumentRecord>, DocumentCommandError> {
        Ok(self
            .documents
            .read(document_id)
            .map_err(ServiceError::from)?)
    }

    fn create(
        &self,
        title: String,
        audio_path: String,
        markdown: String,
        tags: Vec<String>,
    ) -> Result<DocumentCommandResult, DocumentCommandError> {
        if title.trim().is_empty() {
            return Err(DocumentCommandError::Invalid(
                "document title cannot be empty".into(),
            ));
        }
        let mut state = DocumentState::new(DocumentId::new_uuid(), title, audio_path);
        state.content.tags = tags;
        let document_id = state.id().clone();
        self.documents
            .save_with_markdown(state, markdown)
            .map_err(ServiceError::from)?;
        Ok(DocumentCommandResult::Document(self.read(&document_id)?))
    }

    fn create_semantic_child(
        &self,
        request: SemanticChildRequest,
    ) -> Result<DocumentCommandResult, DocumentCommandError> {
        let parent = self
            .read(&request.parent_id)?
            .ok_or_else(|| DocumentCommandError::NotFound(request.parent_id.clone()))?;
        ensure_revision(&parent.state, request.expected_parent_revision)?;
        if parent.state.hierarchy.depth >= 2 {
            return Err(DocumentCommandError::Invalid(
                "document tree cannot exceed three levels".into(),
            ));
        }
        if request.start_ms >= request.end_ms {
            return Err(DocumentCommandError::Invalid(
                "document range must be non-empty".into(),
            ));
        }
        let child = parent.state.semantic_child(
            DocumentId::new_uuid(),
            request.title,
            request.start_ms,
            request.end_ms,
            request.tags,
        );
        let child_id = child.id().clone();
        self.documents
            .save_with_markdown(child, request.markdown)
            .map_err(ServiceError::from)?;
        Ok(DocumentCommandResult::Document(self.read(&child_id)?))
    }
}

impl DocumentService {
    /// Saves `document` as revision `expected_revision + 1` if the stored
    /// document is still at `expected_revision`; `markdown: None` keeps the
    /// stored body. The re-read and the write are two repository calls, not
    /// one locked step: a writer that lands between them is overwritten.
    pub(crate) fn save_expecting_revision(
        &self,
        mut document: DocumentState,
        expected_revision: u64,
        markdown: Option<String>,
    ) -> Result<DocumentState, DocumentCommandError> {
        let repository = self.current();
        let current = repository
            .read(document.id())
            .map_err(ServiceError::from)?
            .ok_or_else(|| DocumentCommandError::NotFound(document.id().clone()))?;
        ensure_revision(&current.state, expected_revision)?;
        document.schema.revision = expected_revision.saturating_add(1);
        document.schema.updated_at_ms = crate::now_ms();
        repository
            .save(document.clone(), markdown)
            .map_err(ServiceError::from)?;
        Ok(document)
    }
}

fn apply_patch(
    state: &mut DocumentState,
    patch: &DocumentPatch,
) -> Result<(), DocumentCommandError> {
    if let Some(title) = &patch.title {
        state.content.title = title.clone();
    }
    if let Some(tags) = &patch.tags {
        state.content.tags = tags.clone();
    }
    if let Some(summary) = &patch.summary {
        state.content.summary = summary.clone();
    }
    if let Some(status) = &patch.document_status {
        let status = status.trim();
        if status.is_empty() {
            return Err(DocumentCommandError::Invalid(
                "document status cannot be empty".into(),
            ));
        }
        state.document_status.value = status.to_string();
    }
    Ok(())
}

fn ensure_revision(
    state: &DocumentState,
    expected_revision: u64,
) -> Result<(), DocumentCommandError> {
    if state.revision() != expected_revision {
        return Err(DocumentCommandError::RevisionConflict {
            document_id: state.id().clone(),
            expected: expected_revision,
            actual: state.revision(),
        });
    }
    Ok(())
}

fn find_document<'a>(
    documents: &'a [DocumentState],
    document_id: &DocumentId,
) -> Result<&'a DocumentState, DocumentCommandError> {
    documents
        .iter()
        .find(|document| document.id() == document_id)
        .ok_or_else(|| DocumentCommandError::NotFound(document_id.clone()))
}

struct ServiceError(String);

impl From<String> for ServiceError {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<ServiceError> for DocumentCommandError {
    fn from(value: ServiceError) -> Self {
        Self::Service(value.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DocumentIssue, DocumentRepository, DocumentSnapshot};
    use contracts::DocumentCommand;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    type ConcurrentWrite = Box<dyn FnOnce(&mut BTreeMap<String, DocumentRecord>) + Send>;

    #[derive(Default)]
    struct MemoryRepository {
        documents: Mutex<BTreeMap<String, DocumentRecord>>,
        /// Ids passed to `save`, in call order.
        saved: Mutex<Vec<String>>,
        /// Runs once right after the next snapshot is taken, standing in
        /// for another writer that changes files after the command read them.
        after_snapshot: Mutex<Option<ConcurrentWrite>>,
        /// Runs once right after the document with this id is saved.
        after_save: Mutex<Option<(String, ConcurrentWrite)>>,
    }

    impl MemoryRepository {
        fn seeded(documents: Vec<DocumentState>) -> Arc<Self> {
            let repository = Self::default();
            for state in documents {
                let markdown = format!("# {}\n", state.title());
                repository.documents.lock().unwrap().insert(
                    state.id().as_str().into(),
                    DocumentRecord { state, markdown },
                );
            }
            Arc::new(repository)
        }

        fn state(&self, id: &str) -> DocumentState {
            self.documents.lock().unwrap()[id].state.clone()
        }

        fn saved(&self) -> Vec<String> {
            self.saved.lock().unwrap().clone()
        }

        fn change_after_next_snapshot(
            &self,
            write: impl FnOnce(&mut BTreeMap<String, DocumentRecord>) + Send + 'static,
        ) {
            *self.after_snapshot.lock().unwrap() = Some(Box::new(write));
        }

        fn change_after_saving(
            &self,
            document_id: &str,
            write: impl FnOnce(&mut BTreeMap<String, DocumentRecord>) + Send + 'static,
        ) {
            *self.after_save.lock().unwrap() = Some((document_id.into(), Box::new(write)));
        }
    }

    impl DocumentRepository for MemoryRepository {
        fn snapshot(&self) -> Result<DocumentSnapshot, String> {
            let mut documents = self.documents.lock().unwrap();
            let snapshot = DocumentSnapshot {
                documents: documents
                    .values()
                    .map(|record| record.state.clone())
                    .collect(),
                issues: Vec::<DocumentIssue>::new(),
            };
            if let Some(write) = self.after_snapshot.lock().unwrap().take() {
                write(&mut documents);
            }
            Ok(snapshot)
        }

        fn get(&self, id: &DocumentId) -> Result<Option<DocumentState>, String> {
            Ok(self
                .documents
                .lock()
                .unwrap()
                .get(id.as_str())
                .map(|record| record.state.clone()))
        }

        fn read(&self, id: &DocumentId) -> Result<Option<DocumentRecord>, String> {
            Ok(self.documents.lock().unwrap().get(id.as_str()).cloned())
        }

        fn watch(&self) -> Result<std::sync::mpsc::Receiver<crate::DocumentChange>, String> {
            let (_sender, receiver) = std::sync::mpsc::channel();
            Ok(receiver)
        }

        fn save(&self, document: DocumentState, markdown: Option<String>) -> Result<(), String> {
            self.saved
                .lock()
                .unwrap()
                .push(document.id().as_str().into());
            let mut documents = self.documents.lock().unwrap();
            let document_id = document.id().as_str().to_string();
            let markdown = markdown
                .or_else(|| {
                    documents
                        .get(&document_id)
                        .map(|record| record.markdown.clone())
                })
                .unwrap_or_default();
            documents.insert(
                document_id.clone(),
                DocumentRecord {
                    state: document,
                    markdown,
                },
            );
            let mut after_save = self.after_save.lock().unwrap();
            if after_save
                .as_ref()
                .is_some_and(|(id, _)| *id == document_id)
            {
                let (_, write) = after_save.take().unwrap();
                write(&mut documents);
            }
            Ok(())
        }
    }

    #[test]
    fn command_service_supports_create_update_and_revision_conflicts() {
        let documents = DocumentService::new(Arc::new(MemoryRepository::default()));
        let service = DocumentCommandService::new(documents);
        let created = service
            .execute(contracts::DocumentCommandEnvelope::new(
                "create-1",
                DocumentCommand::Create {
                    title: "Daily note".into(),
                    audio_path: "daily.m4a".into(),
                    markdown: "# Daily note\n".into(),
                    tags: vec!["daily".into()],
                },
            ))
            .unwrap();
        let record = match created {
            contracts::DocumentCommandResult::Document(Some(record)) => record,
            other => panic!("unexpected create result: {other:?}"),
        };

        let updated = service
            .execute(contracts::DocumentCommandEnvelope::new(
                "update-1",
                DocumentCommand::Update {
                    document_id: record.state.id().clone(),
                    expected_revision: record.state.revision(),
                    patch: DocumentPatch {
                        title: Some("Updated note".into()),
                        summary: Some(Some("A short summary".into())),
                        ..DocumentPatch::default()
                    },
                },
            ))
            .unwrap();
        let updated_record = match updated {
            contracts::DocumentCommandResult::Updated(record) => record,
            other => panic!("unexpected update result: {other:?}"),
        };
        assert_eq!(updated_record.state.title(), "Updated note");
        assert_eq!(updated_record.state.revision(), 1);
        assert_eq!(updated_record.markdown, "# Daily note\n");

        let status_updated = service
            .execute(contracts::DocumentCommandEnvelope::new(
                "status-1",
                DocumentCommand::Update {
                    document_id: updated_record.state.id().clone(),
                    expected_revision: updated_record.state.revision(),
                    patch: DocumentPatch {
                        document_status: Some("review".into()),
                        ..DocumentPatch::default()
                    },
                },
            ))
            .unwrap();
        let status_record = match status_updated {
            contracts::DocumentCommandResult::Updated(record) => record,
            other => panic!("unexpected status result: {other:?}"),
        };
        assert_eq!(status_record.state.document_status.value, "review");

        let stale = service.execute(contracts::DocumentCommandEnvelope::new(
            "update-stale",
            DocumentCommand::Update {
                document_id: updated_record.state.id().clone(),
                expected_revision: 0,
                patch: DocumentPatch::default(),
            },
        ));
        assert!(matches!(
            stale,
            Err(DocumentCommandError::RevisionConflict {
                expected: 0,
                actual: 2,
                ..
            })
        ));
    }

    const SEEDED_REVISION: u64 = 3;

    fn root(id: &str, sibling_order: u32) -> DocumentState {
        let mut state = DocumentState::new(DocumentId::new(id), id, format!("{id}.wav"));
        state.hierarchy.sibling_order = sibling_order;
        state.schema.revision = SEEDED_REVISION;
        state
    }

    fn child(parent: &DocumentState, id: &str, sibling_order: u32) -> DocumentState {
        let mut state = parent.semantic_child(DocumentId::new(id), id, 0, 1_000, Vec::new());
        state.hierarchy.sibling_order = sibling_order;
        state.schema.revision = SEEDED_REVISION;
        state
    }

    /// `r` and `x` are roots of different recordings; `a`, `b`, `c` are the
    /// children of `r` in that order and `a1` is the only grandchild.
    fn tree() -> Arc<MemoryRepository> {
        let r = root("r", 0);
        let x = root("x", 1);
        let a = child(&r, "a", 0);
        let b = child(&r, "b", 1);
        let c = child(&r, "c", 2);
        let a1 = child(&a, "a1", 0);
        MemoryRepository::seeded(vec![r, x, a, b, c, a1])
    }

    fn reorder(document_id: &str, target_id: &str, before: bool) -> DocumentCommandEnvelope {
        DocumentCommandEnvelope::new(
            "reorder",
            DocumentCommand::Reorder {
                document_id: DocumentId::new(document_id),
                expected_revision: SEEDED_REVISION,
                target_id: DocumentId::new(target_id),
                before,
            },
        )
    }

    fn reparent(
        document_id: &str,
        new_parent_id: Option<&str>,
        sibling_order: u32,
    ) -> DocumentCommandEnvelope {
        DocumentCommandEnvelope::new(
            "reparent",
            DocumentCommand::Reparent {
                document_id: DocumentId::new(document_id),
                expected_revision: SEEDED_REVISION,
                new_parent_id: new_parent_id.map(DocumentId::new),
                sibling_order,
            },
        )
    }

    fn revisions(repository: &MemoryRepository) -> BTreeMap<String, u64> {
        repository
            .documents
            .lock()
            .unwrap()
            .iter()
            .map(|(id, record)| (id.clone(), record.state.revision()))
            .collect()
    }

    fn expected_revisions(bumped: &[&str]) -> BTreeMap<String, u64> {
        ["r", "x", "a", "b", "c", "a1"]
            .into_iter()
            .map(|id| {
                let revision = if bumped.contains(&id) {
                    SEEDED_REVISION + 1
                } else {
                    SEEDED_REVISION
                };
                (id.to_string(), revision)
            })
            .collect()
    }

    fn sorted(mut ids: Vec<String>) -> Vec<String> {
        ids.sort();
        ids
    }

    #[test]
    fn reorder_writes_only_the_siblings_whose_order_changes() {
        let repository = tree();
        let service = DocumentCommandService::new(DocumentService::new(repository.clone()));

        service.execute(reorder("c", "b", true)).unwrap();

        assert_eq!(sorted(repository.saved()), vec!["b", "c"]);
        assert_eq!(revisions(&repository), expected_revisions(&["b", "c"]));
        assert_eq!(repository.state("a").hierarchy.sibling_order, 0);
        assert_eq!(repository.state("c").hierarchy.sibling_order, 1);
        assert_eq!(repository.state("b").hierarchy.sibling_order, 2);
    }

    #[test]
    fn reparent_writes_the_moved_document_and_its_subtree_only() {
        let repository = tree();
        let service = DocumentCommandService::new(DocumentService::new(repository.clone()));

        service.execute(reparent("a", None, 2)).unwrap();

        assert_eq!(sorted(repository.saved()), vec!["a", "a1"]);
        assert_eq!(revisions(&repository), expected_revisions(&["a", "a1"]));
        let moved = repository.state("a");
        assert_eq!(moved.hierarchy.parent_id, None);
        assert_eq!(moved.hierarchy.depth, 0);
        assert_eq!(moved.hierarchy.sibling_order, 2);
        assert_eq!(repository.state("a1").hierarchy.depth, 1);
    }

    #[test]
    fn reorder_conflicts_without_writing_when_a_changed_sibling_moved_after_the_read() {
        let repository = tree();
        repository.change_after_next_snapshot(|documents| {
            let sibling = &mut documents.get_mut("b").unwrap().state;
            sibling.hierarchy.sibling_order = 7;
            sibling.schema.revision += 1;
        });
        let service = DocumentCommandService::new(DocumentService::new(repository.clone()));

        let result = service.execute(reorder("c", "b", true));

        assert!(
            matches!(
                &result,
                Err(DocumentCommandError::RevisionConflict {
                    document_id,
                    expected: SEEDED_REVISION,
                    actual,
                }) if document_id.as_str() == "b" && *actual == SEEDED_REVISION + 1
            ),
            "unexpected result: {result:?}"
        );
        assert!(repository.saved().is_empty());
        assert_eq!(repository.state("c").revision(), SEEDED_REVISION);
        assert_eq!(repository.state("b").hierarchy.sibling_order, 7);
    }

    #[test]
    fn reparent_conflicts_without_writing_when_the_new_parent_changed_after_the_read() {
        let repository = tree();
        repository.change_after_next_snapshot(|documents| {
            documents.get_mut("b").unwrap().state.schema.revision += 1;
        });
        let service = DocumentCommandService::new(DocumentService::new(repository.clone()));

        let result = service.execute(reparent("a1", Some("b"), 0));

        assert!(
            matches!(result, Err(DocumentCommandError::RevisionConflict { .. })),
            "unexpected result: {result:?}"
        );
        assert!(repository.saved().is_empty());
        assert_eq!(
            repository.state("a1").hierarchy.parent_id,
            Some(DocumentId::new("a"))
        );
    }

    /// Pins the window that remains without a multi-document transaction: a
    /// writer landing after the pre-write check but before a later document
    /// is saved leaves the earlier documents written.
    #[test]
    fn a_change_between_two_tree_writes_stops_the_move_after_the_first_write() {
        let repository = tree();
        repository.change_after_saving("b", |documents| {
            documents.get_mut("c").unwrap().state.schema.revision += 1;
        });
        let service = DocumentCommandService::new(DocumentService::new(repository.clone()));

        let result = service.execute(reorder("c", "b", true));

        assert!(
            matches!(
                &result,
                Err(DocumentCommandError::RevisionConflict { document_id, .. })
                    if document_id.as_str() == "c"
            ),
            "unexpected result: {result:?}"
        );
        assert_eq!(repository.saved(), vec!["b"]);
        assert_eq!(repository.state("b").hierarchy.sibling_order, 2);
        assert_eq!(repository.state("c").hierarchy.sibling_order, 2);
    }

    #[test]
    fn tree_moves_keep_domain_validation() {
        let repository = tree();
        let service = DocumentCommandService::new(DocumentService::new(repository.clone()));

        for envelope in [
            reparent("a", Some("a1"), 0),
            reparent("a", Some("x"), 0),
            reparent("b", Some("a1"), 0),
            reorder("a", "x", true),
        ] {
            assert!(service.execute(envelope).is_err());
        }
        assert!(repository.saved().is_empty());
        assert_eq!(revisions(&repository), expected_revisions(&[]));
    }

    #[test]
    fn update_with_a_stale_revision_writes_nothing() {
        let repository = tree();
        let service = DocumentCommandService::new(DocumentService::new(repository.clone()));
        let before = repository.documents.lock().unwrap()["a"].clone();

        let result = service.execute(DocumentCommandEnvelope::new(
            "stale-update",
            DocumentCommand::Update {
                document_id: DocumentId::new("a"),
                expected_revision: SEEDED_REVISION - 1,
                patch: DocumentPatch {
                    title: Some("Overwritten".into()),
                    markdown: Some("# Overwritten\n".into()),
                    ..DocumentPatch::default()
                },
            },
        ));

        assert!(matches!(
            result,
            Err(DocumentCommandError::RevisionConflict { .. })
        ));
        assert!(repository.saved().is_empty());
        assert_eq!(repository.documents.lock().unwrap()["a"], before);
    }

    #[test]
    fn typed_tree_moves_return_the_written_documents() {
        let repository = tree();
        let service = DocumentCommandService::new(DocumentService::new(repository.clone()));

        let written = service
            .reorder_document(
                &DocumentId::new("c"),
                SEEDED_REVISION,
                &DocumentId::new("b"),
                true,
            )
            .unwrap();

        let written: Vec<_> = written
            .iter()
            .map(|document| (document.id().as_str(), document.revision()))
            .collect();
        assert_eq!(
            written,
            vec![("b", SEEDED_REVISION + 1), ("c", SEEDED_REVISION + 1)]
        );

        let unchanged = service
            .reparent_document(&DocumentId::new("x"), SEEDED_REVISION, None, 1)
            .unwrap();
        assert!(unchanged.is_empty());
        assert!(matches!(
            service.reparent_document(
                &DocumentId::new("a1"),
                SEEDED_REVISION,
                Some(&DocumentId::new("x")),
                0
            ),
            Err(DocumentCommandError::Tree(
                domain::ProjectTreeError::CrossAudioAsset
            ))
        ));
    }

    fn processed(id: &str) -> DocumentState {
        let mut state = root(id, 0);
        state.results.transcript = vec![
            domain::TranscriptSegment {
                id: "t1".into(),
                start_ms: 0,
                end_ms: 1_000,
                text: "你好".into(),
            },
            domain::TranscriptSegment {
                id: "t2".into(),
                start_ms: 1_000,
                end_ms: 2_500,
                text: "hello there".into(),
            },
        ];
        state.results.diarization = vec![
            domain::DiarizationSegment {
                id: "d1".into(),
                speaker_id: "speaker-0".into(),
                speaker_profile_id: Some("p-alice".into()),
                speaker_label: Some("Alice".into()),
                start_ms: 0,
                end_ms: 1_000,
                confidence: Some(0.9),
            },
            domain::DiarizationSegment {
                id: "d2".into(),
                speaker_id: "speaker-1".into(),
                speaker_profile_id: None,
                speaker_label: None,
                start_ms: 1_000,
                end_ms: 2_500,
                confidence: Some(0.8),
            },
        ];
        state.results.speaker_segments = domain::align_transcript_and_speakers(
            &state.results.transcript,
            &state.results.diarization,
        );
        state.processing.latest = Some(domain::ProcessingRecord {
            processing_revision: 1,
            completed_at_ms: 42,
            input_audio_checksum: Some("abc".into()),
            asr: domain::ModelProvenance::default(),
            diarization: domain::DiarizationProvenance::default(),
            alignment: domain::AlgorithmProvenance::default(),
            runtime: domain::RuntimeProvenance::default(),
        });
        state
    }

    /// A processed document whose body is still the generated transcript.
    fn processed_repository() -> Arc<MemoryRepository> {
        let state = processed("doc");
        let markdown = render_markdown(
            std::path::Path::new(state.audio_path()),
            &state.results.speaker_segments,
        );
        let repository = MemoryRepository::seeded(Vec::new());
        repository
            .documents
            .lock()
            .unwrap()
            .insert("doc".into(), DocumentRecord { state, markdown });
        repository
    }

    fn assert_audio_facts_unchanged(before: &DocumentState, after: &DocumentState) {
        assert_eq!(after.results.transcript, before.results.transcript);
        assert_eq!(after.source, before.source);
        assert_eq!(after.processing, before.processing);
        let timings = |state: &DocumentState| {
            state
                .results
                .diarization
                .iter()
                .map(|segment| (segment.id.clone(), segment.start_ms, segment.end_ms))
                .chain(state.results.speaker_segments.iter().map(|segment| {
                    (
                        segment.transcript_segment_id.clone(),
                        segment.start_ms,
                        segment.end_ms,
                    )
                }))
                .collect::<Vec<_>>()
        };
        assert_eq!(timings(after), timings(before));
        let texts = |state: &DocumentState| {
            state
                .results
                .speaker_segments
                .iter()
                .map(|segment| segment.text.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(texts(after), texts(before));
    }

    #[test]
    fn merge_speaker_command_is_revision_checked_and_keeps_audio_facts() {
        let repository = processed_repository();
        let before = repository.state("doc");
        let service = DocumentCommandService::new(DocumentService::new(repository.clone()));

        let result = service
            .execute(DocumentCommandEnvelope::new(
                "merge",
                DocumentCommand::MergeSpeaker {
                    document_id: DocumentId::new("doc"),
                    expected_revision: SEEDED_REVISION,
                    from_speaker_id: "speaker-1".into(),
                    into_speaker_id: "speaker-0".into(),
                },
            ))
            .unwrap();

        let DocumentCommandResult::Updated(record) = result else {
            panic!("unexpected merge result: {result:?}");
        };
        assert_eq!(record.state.revision(), SEEDED_REVISION + 1);
        assert_eq!(record.state.results.diarization[1].speaker_id, "speaker-0");
        assert_eq!(
            record.state.results.speaker_segments[1]
                .speaker_label
                .as_deref(),
            Some("Alice")
        );
        assert_audio_facts_unchanged(&before, &record.state);
        assert_eq!(
            record.markdown,
            render_markdown(
                std::path::Path::new(before.audio_path()),
                &record.state.results.speaker_segments
            )
        );
        assert_eq!(record.markdown.matches("Alice").count(), 2);
        assert_eq!(repository.saved(), vec!["doc"]);
        assert_eq!(repository.state("doc"), record.state);
    }

    #[test]
    fn set_speaker_label_is_revision_checked_and_keeps_an_edited_body() {
        let repository = processed_repository();
        repository
            .documents
            .lock()
            .unwrap()
            .get_mut("doc")
            .unwrap()
            .markdown = "# My notes\n".into();
        let before = repository.state("doc");
        let service = DocumentCommandService::new(DocumentService::new(repository.clone()));

        let record = service
            .set_speaker_label(
                &DocumentId::new("doc"),
                SEEDED_REVISION,
                "speaker-1",
                Some("Bob".into()),
            )
            .unwrap();

        assert_eq!(record.state.revision(), SEEDED_REVISION + 1);
        assert_eq!(
            record.state.results.diarization[1].speaker_label.as_deref(),
            Some("Bob")
        );
        assert_eq!(
            record.state.results.speaker_segments[1]
                .speaker_label
                .as_deref(),
            Some("Bob")
        );
        assert_eq!(
            record.state.results.diarization[0].speaker_label.as_deref(),
            Some("Alice")
        );
        assert_audio_facts_unchanged(&before, &record.state);
        assert_eq!(record.markdown, "# My notes\n");

        let unchanged = service
            .set_speaker_label(
                &DocumentId::new("doc"),
                SEEDED_REVISION + 1,
                "speaker-1",
                Some("Bob".into()),
            )
            .unwrap();
        assert_eq!(unchanged.state.revision(), SEEDED_REVISION + 1);
        assert_eq!(repository.saved(), vec!["doc"]);
    }

    #[test]
    fn speaker_commands_reject_unknown_ids_and_stale_revisions_without_writing() {
        let repository = processed_repository();
        let before = repository.documents.lock().unwrap()["doc"].clone();
        let service = DocumentCommandService::new(DocumentService::new(repository.clone()));
        let id = DocumentId::new("doc");

        assert!(matches!(
            service.merge_speaker(&id, SEEDED_REVISION, "speaker-9", "speaker-0"),
            Err(DocumentCommandError::Speaker(
                domain::SpeakerCorrectionError::UnknownSpeaker(ref speaker)
            )) if speaker == "speaker-9"
        ));
        assert!(matches!(
            service.merge_speaker(&id, SEEDED_REVISION, "speaker-1", "speaker-9"),
            Err(DocumentCommandError::Speaker(
                domain::SpeakerCorrectionError::UnknownSpeaker(_)
            ))
        ));
        assert!(matches!(
            service.set_speaker_label(&id, SEEDED_REVISION, "speaker-9", Some("Carol".into())),
            Err(DocumentCommandError::Speaker(
                domain::SpeakerCorrectionError::UnknownSpeaker(_)
            ))
        ));
        let stale = service.merge_speaker(&id, SEEDED_REVISION - 1, "speaker-1", "speaker-0");
        assert!(
            matches!(
                &stale,
                Err(DocumentCommandError::RevisionConflict {
                    document_id,
                    expected,
                    actual: SEEDED_REVISION,
                }) if document_id == &id && *expected == SEEDED_REVISION - 1
            ),
            "unexpected result: {stale:?}"
        );
        assert!(matches!(
            service.merge_speaker(&DocumentId::new("missing"), 0, "speaker-1", "speaker-0"),
            Err(DocumentCommandError::NotFound(_))
        ));

        assert!(repository.saved().is_empty());
        assert_eq!(repository.documents.lock().unwrap()["doc"], before);
    }

    #[test]
    fn command_errors_name_documents_without_debug_text() {
        let messages = [
            DocumentCommandError::NotFound(DocumentId::new("doc")).to_string(),
            DocumentCommandError::RevisionConflict {
                document_id: DocumentId::new("doc"),
                expected: 1,
                actual: 2,
            }
            .to_string(),
            DocumentCommandError::Tree(domain::ProjectTreeError::MaximumDepth).to_string(),
            DocumentCommandError::Speaker(domain::SpeakerCorrectionError::EmptyLabel).to_string(),
        ];

        for message in &messages {
            assert!(!message.contains(['{', '}', '(', '"']), "{message}");
        }
        assert_eq!(
            messages[1],
            "document doc changed since it was read: expected revision 1, found 2"
        );
    }
}
