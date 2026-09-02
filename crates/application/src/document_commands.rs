use crate::DocumentService;
use contracts::{
    DocumentCommand, DocumentCommandEnvelope, DocumentCommandIssue, DocumentCommandResult,
    DocumentPatch, DocumentRecord, DOCUMENT_COMMAND_SCHEMA_VERSION,
};
use domain::{reorder_project, reparent_project, DocumentId, DocumentState};

#[derive(Debug, thiserror::Error)]
pub enum DocumentCommandError {
    #[error("unsupported document command schema: {actual}, expected {expected}")]
    UnsupportedSchema { actual: u32, expected: u32 },
    #[error("document service error: {0}")]
    Service(String),
    #[error("document was not found: {0:?}")]
    NotFound(DocumentId),
    #[error("document revision conflict: expected {expected}, found {actual}")]
    RevisionConflict { expected: u64, actual: u64 },
    #[error("invalid document command: {0}")]
    Invalid(String),
}

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
        match envelope.command {
            DocumentCommand::List => self.list(),
            DocumentCommand::Read { document_id } => {
                Ok(DocumentCommandResult::Document(self.read(&document_id)?))
            }
            DocumentCommand::Create {
                title,
                audio_path,
                markdown,
                tags,
            } => self.create(title, audio_path, markdown, tags),
            DocumentCommand::Update {
                document_id,
                expected_revision,
                patch,
            } => self.update(&document_id, expected_revision, patch),
            DocumentCommand::CreateSemanticChild {
                parent_id,
                expected_parent_revision,
                title,
                start_ms,
                end_ms,
                tags,
                markdown,
            } => self.create_semantic_child(SemanticChildRequest {
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
            } => self.reparent(
                &document_id,
                expected_revision,
                new_parent_id.as_ref(),
                sibling_order,
            ),
            DocumentCommand::Reorder {
                document_id,
                expected_revision,
                target_id,
                before,
            } => self.reorder(&document_id, expected_revision, &target_id, before),
        }
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

    fn update(
        &self,
        document_id: &DocumentId,
        expected_revision: u64,
        patch: DocumentPatch,
    ) -> Result<DocumentCommandResult, DocumentCommandError> {
        let record = self
            .read(document_id)?
            .ok_or_else(|| DocumentCommandError::NotFound(document_id.clone()))?;
        ensure_revision(&record.state, expected_revision)?;
        let mut state = record.state;
        apply_patch(&mut state, &patch)?;
        let markdown = patch.markdown.unwrap_or(record.markdown);
        let updated = self
            .documents
            .save_versioned(state, expected_revision, markdown)
            .map_err(ServiceError::from)?;
        Ok(DocumentCommandResult::Updated(updated))
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

    fn reparent(
        &self,
        document_id: &DocumentId,
        expected_revision: u64,
        new_parent_id: Option<&DocumentId>,
        sibling_order: u32,
    ) -> Result<DocumentCommandResult, DocumentCommandError> {
        let mut documents = self
            .documents
            .snapshot()
            .map_err(ServiceError::from)?
            .documents;
        let current = find_document(&documents, document_id)?;
        ensure_revision(current, expected_revision)?;
        reparent_project(&mut documents, document_id, new_parent_id, sibling_order).map_err(
            |error| DocumentCommandError::Invalid(format!("cannot reparent document: {error:?}")),
        )?;
        for document in documents {
            self.documents.save(document).map_err(ServiceError::from)?;
        }
        self.list()
    }

    fn reorder(
        &self,
        document_id: &DocumentId,
        expected_revision: u64,
        target_id: &DocumentId,
        before: bool,
    ) -> Result<DocumentCommandResult, DocumentCommandError> {
        let mut documents = self
            .documents
            .snapshot()
            .map_err(ServiceError::from)?
            .documents;
        let current = find_document(&documents, document_id)?;
        ensure_revision(current, expected_revision)?;
        reorder_project(&mut documents, document_id, target_id, before).map_err(|error| {
            DocumentCommandError::Invalid(format!("cannot reorder document: {error:?}"))
        })?;
        for document in documents {
            self.documents.save(document).map_err(ServiceError::from)?;
        }
        self.list()
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

    #[derive(Default)]
    struct MemoryRepository {
        documents: Mutex<BTreeMap<String, DocumentRecord>>,
    }

    impl DocumentRepository for MemoryRepository {
        fn snapshot(&self) -> Result<DocumentSnapshot, String> {
            Ok(DocumentSnapshot {
                documents: self
                    .documents
                    .lock()
                    .unwrap()
                    .values()
                    .map(|record| record.state.clone())
                    .collect(),
                issues: Vec::<DocumentIssue>::new(),
            })
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
            let mut documents = self.documents.lock().unwrap();
            let markdown = markdown
                .or_else(|| {
                    documents
                        .get(document.id().as_str())
                        .map(|record| record.markdown.clone())
                })
                .unwrap_or_default();
            documents.insert(
                document.id().as_str().into(),
                DocumentRecord {
                    state: document,
                    markdown,
                },
            );
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
                actual: 2
            })
        ));
    }
}
