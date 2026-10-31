//! Missing-audio detection and relinking of a document's source audio.
//!
//! Documents share an audio asset when they have the same asset key: the
//! root's id, which semantic children record as `audio_asset_id`. The
//! checksum an asset is verified against is the `input_audio_checksum` that
//! processing recorded on any document of the asset.

use crate::workspace::read_file_sha256;
use crate::{render_markdown, DocumentCommandError, DocumentService, WorkspaceService};
use domain::{DocumentId, DocumentState};
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioStatus {
    /// The file hashes to the checksum processing recorded.
    Present { path: String, checksum: String },
    /// No regular file exists at the recorded path.
    Missing { path: String },
    /// The file differs from the audio the results were computed from.
    ChecksumMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    /// The file exists but no processing recorded a checksum to verify it
    /// against, typically because the document was never processed.
    Unverifiable { path: String },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AudioSourceError {
    #[error("document {0} does not exist")]
    NotFound(DocumentId),
    /// The command wrote nothing.
    #[error(
        "document {document_id} changed since it was read: expected revision {expected}, found {actual}"
    )]
    RevisionConflict {
        document_id: DocumentId,
        expected: u64,
        actual: u64,
    },
    #[error("audio file {path} does not exist")]
    FileMissing { path: String },
    /// The file is not the audio the recorded results came from; the command
    /// wrote nothing.
    #[error("audio file {path} has checksum {actual}, expected {expected}")]
    ChecksumMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    #[error("cannot read audio file {path}: {message}")]
    Read { path: String, message: String },
    #[error("document service error: {0}")]
    Service(String),
}

impl From<DocumentCommandError> for AudioSourceError {
    fn from(error: DocumentCommandError) -> Self {
        match error {
            DocumentCommandError::NotFound(document_id) => Self::NotFound(document_id),
            DocumentCommandError::RevisionConflict {
                document_id,
                expected,
                actual,
            } => Self::RevisionConflict {
                document_id,
                expected,
                actual,
            },
            other => Self::Service(other.to_string()),
        }
    }
}

impl WorkspaceService {
    /// Whether the document's source audio is still the file its results
    /// were computed from. Blocking: it hashes the whole file, so call it
    /// from a worker, not the UI thread.
    pub fn audio_status(&self, document_id: &DocumentId) -> Result<AudioStatus, AudioSourceError> {
        let documents = self.list_projects().map_err(AudioSourceError::Service)?;
        let document = find(&documents, document_id)?;
        let path = document.audio_path().to_string();
        if !Path::new(&path).is_file() {
            return Ok(AudioStatus::Missing { path });
        }
        let recorded = recorded_checksums(&asset_documents(&documents, document));
        if recorded.is_empty() {
            return Ok(AudioStatus::Unverifiable { path });
        }
        let actual = checksum(&path)?;
        Ok(
            match recorded.into_iter().find(|expected| *expected != actual) {
                Some(expected) => AudioStatus::ChecksumMismatch {
                    path,
                    expected,
                    actual,
                },
                None => AudioStatus::Present {
                    path,
                    checksum: actual,
                },
            },
        )
    }

    /// Relinks the recording in the workspace current when this call starts.
    pub fn relink_audio(
        &self,
        document_id: &DocumentId,
        expected_revision: u64,
        new_path: impl Into<String>,
    ) -> Result<Vec<DocumentState>, AudioSourceError> {
        self.documents()
            .relink_audio(document_id, expected_revision, new_path)
    }
}

impl DocumentService {
    /// Points every document sharing the document's audio asset (the root
    /// and its semantic children) at `new_path` and returns the documents it
    /// wrote.
    ///
    /// `new_path` must be an existing file; when processing recorded a
    /// checksum for the asset, the file must match it, otherwise the command
    /// fails with [`AudioSourceError::ChecksumMismatch`]. An unprocessed asset
    /// relinks without a checksum check. Only `source.audio_path` changes (and
    /// a body that is still the generated transcript is regenerated with the
    /// new path); transcript, diarization, alignment and processing records
    /// are kept. Each document is saved against the revision this command
    /// read, and every revision is re-checked before the first write, so a
    /// conflict writes nothing. Capture a frozen service before queueing the
    /// relink to retain its chosen workspace. Blocking: it hashes the whole file.
    pub fn relink_audio(
        &self,
        document_id: &DocumentId,
        expected_revision: u64,
        new_path: impl Into<String>,
    ) -> Result<Vec<DocumentState>, AudioSourceError> {
        let new_path = new_path.into();
        let service = self.frozen();
        let documents = service.list().map_err(AudioSourceError::Service)?;
        let document = find(&documents, document_id)?;
        ensure_revision(document, expected_revision)?;
        if !Path::new(&new_path).is_file() {
            return Err(AudioSourceError::FileMissing { path: new_path });
        }
        let asset = asset_documents(&documents, document);
        let recorded = recorded_checksums(&asset);
        if !recorded.is_empty() {
            let actual = checksum(&new_path)?;
            if let Some(expected) = recorded.into_iter().find(|expected| *expected != actual) {
                return Err(AudioSourceError::ChecksumMismatch {
                    path: new_path,
                    expected,
                    actual,
                });
            }
        }

        let mut writes = Vec::new();
        for read in asset {
            let current = service
                .read(read.id())
                .map_err(AudioSourceError::Service)?
                .ok_or_else(|| AudioSourceError::NotFound(read.id().clone()))?;
            ensure_revision(&current.state, read.revision())?;
            if current.state.audio_path() != new_path {
                writes.push(current);
            }
        }
        writes
            .into_iter()
            .map(|record| {
                let revision = record.state.revision();
                let segments = &record.state.results.speaker_segments;
                let markdown = (record.markdown
                    == render_markdown(Path::new(record.state.audio_path()), segments))
                .then(|| render_markdown(Path::new(&new_path), segments));
                let mut state = record.state;
                state.source.audio_path = new_path.clone();
                service
                    .save_expecting_revision(state, revision, markdown)
                    .map_err(AudioSourceError::from)
            })
            .collect()
    }
}

fn asset_key(document: &DocumentState) -> &DocumentId {
    document
        .source
        .audio_asset_id
        .as_ref()
        .unwrap_or(document.id())
}

fn asset_documents<'a>(
    documents: &'a [DocumentState],
    document: &DocumentState,
) -> Vec<&'a DocumentState> {
    let key = asset_key(document);
    documents
        .iter()
        .filter(|candidate| asset_key(candidate) == key)
        .collect()
}

fn recorded_checksums(documents: &[&DocumentState]) -> BTreeSet<String> {
    documents
        .iter()
        .filter_map(|document| document.processing.latest.as_ref())
        .filter_map(|record| record.input_audio_checksum.clone())
        .collect()
}

fn checksum(path: &str) -> Result<String, AudioSourceError> {
    read_file_sha256(Path::new(path)).map_err(|error| AudioSourceError::Read {
        path: path.into(),
        message: error.to_string(),
    })
}

fn find<'a>(
    documents: &'a [DocumentState],
    document_id: &DocumentId,
) -> Result<&'a DocumentState, AudioSourceError> {
    documents
        .iter()
        .find(|document| document.id() == document_id)
        .ok_or_else(|| AudioSourceError::NotFound(document_id.clone()))
}

fn ensure_revision(document: &DocumentState, expected: u64) -> Result<(), AudioSourceError> {
    if document.revision() != expected {
        return Err(AudioSourceError::RevisionConflict {
            document_id: document.id().clone(),
            expected,
            actual: document.revision(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DocumentRepository, DocumentService, DocumentSnapshot, ProcessingService};
    use contracts::{
        AsrRequest, AsrResult, BackendDescriptor, BackendError, DiarizationRequest,
        DiarizationResult, DocumentRecord, ModelRef,
    };
    use domain::{ProcessingRecord, SpeakerTaggedSegment};
    use runtime::JobContext;
    use sha2::{Digest, Sha256};
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    type ReadHook = Box<dyn FnOnce(&mut BTreeMap<String, DocumentRecord>) + Send>;

    /// Stores whole records; `on_read` runs once, on the next `read`, to
    /// simulate a writer landing while the command runs.
    #[derive(Default)]
    struct Records {
        records: Mutex<BTreeMap<String, DocumentRecord>>,
        on_read: Mutex<Option<ReadHook>>,
    }

    impl Records {
        fn record(&self, id: &DocumentId) -> DocumentRecord {
            self.records.lock().unwrap()[id.as_str()].clone()
        }
    }

    impl DocumentRepository for Records {
        fn snapshot(&self) -> Result<DocumentSnapshot, String> {
            Ok(DocumentSnapshot {
                documents: self
                    .records
                    .lock()
                    .unwrap()
                    .values()
                    .map(|record| record.state.clone())
                    .collect(),
                issues: Vec::new(),
            })
        }

        fn get(&self, id: &DocumentId) -> Result<Option<DocumentState>, String> {
            Ok(self.read(id)?.map(|record| record.state))
        }

        fn read(&self, id: &DocumentId) -> Result<Option<DocumentRecord>, String> {
            let mut records = self.records.lock().unwrap();
            if let Some(hook) = self.on_read.lock().unwrap().take() {
                hook(&mut records);
            }
            Ok(records.get(id.as_str()).cloned())
        }

        fn watch(&self) -> Result<std::sync::mpsc::Receiver<crate::DocumentChange>, String> {
            Err("not used".into())
        }

        fn save(&self, state: DocumentState, markdown: Option<String>) -> Result<(), String> {
            let mut records = self.records.lock().unwrap();
            let markdown = markdown
                .or_else(|| {
                    records
                        .get(state.id().as_str())
                        .map(|record| record.markdown.clone())
                })
                .unwrap_or_default();
            records.insert(
                state.id().as_str().into(),
                DocumentRecord { state, markdown },
            );
            Ok(())
        }
    }

    struct NoopAsr;

    impl crate::AsrBackend for NoopAsr {
        fn descriptor(&self) -> BackendDescriptor {
            crate::asr_descriptor("noop", &[])
        }

        fn probe(&self, _: &ModelRef) -> Result<(), BackendError> {
            Ok(())
        }

        fn transcribe(&self, _: AsrRequest, _: JobContext) -> crate::BackendFuture<AsrResult> {
            Box::pin(async { Err(BackendError::not_implemented("noop")) })
        }
    }

    struct NoopDiarization;

    impl crate::DiarizationBackend for NoopDiarization {
        fn descriptor(&self) -> BackendDescriptor {
            crate::diarization_descriptor("noop", &[])
        }

        fn probe(&self, _: &ModelRef) -> Result<(), BackendError> {
            Ok(())
        }

        fn diarize(
            &self,
            _: DiarizationRequest,
            _: JobContext,
        ) -> crate::BackendFuture<DiarizationResult> {
            Box::pin(async { Err(BackendError::not_implemented("noop")) })
        }
    }

    struct Fixture {
        directory: PathBuf,
        records: Arc<Records>,
        workspace: WorkspaceService,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let directory = std::env::temp_dir().join(format!(
                "meetnote-audio-source-{label}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&directory).unwrap();
            let records = Arc::new(Records::default());
            let workspace = WorkspaceService::new(
                ProcessingService::new(Arc::new(NoopAsr), Arc::new(NoopDiarization)),
                DocumentService::new(records.clone()),
            );
            Self {
                directory,
                records,
                workspace,
            }
        }

        fn audio(&self, name: &str, bytes: &[u8]) -> String {
            let path = self.directory.join(name);
            std::fs::write(&path, bytes).unwrap();
            path.display().to_string()
        }

        fn missing(&self, name: &str) -> String {
            self.directory.join(name).display().to_string()
        }

        /// Stores `state` at revision 3 with `markdown` as its body.
        fn store(&self, mut state: DocumentState, markdown: &str) -> DocumentState {
            state.schema.revision = 3;
            self.records.records.lock().unwrap().insert(
                state.id().as_str().into(),
                DocumentRecord {
                    state: state.clone(),
                    markdown: markdown.into(),
                },
            );
            state
        }

        /// A root at `path` processed from audio with the checksum of
        /// `bytes`, a semantic child with a generated body and a grandchild
        /// with an edited body.
        fn processed_asset(
            &self,
            path: &str,
            bytes: &[u8],
        ) -> (DocumentState, DocumentState, DocumentState) {
            let mut root = DocumentState::new(DocumentId::new("root"), "Root", path);
            root.results.speaker_segments = vec![SpeakerTaggedSegment {
                transcript_segment_id: "t1".into(),
                start_ms: 0,
                end_ms: 5_000,
                speaker_id: "speaker-0".into(),
                speaker_profile_id: None,
                speaker_label: None,
                text: "hello".into(),
            }];
            root.record_processing(record(Some(sha256(bytes))));
            let first = root.semantic_child(DocumentId::new("first"), "First", 0, 2_000, vec![]);
            let second =
                first.semantic_child(DocumentId::new("second"), "Second", 0, 1_000, vec![]);
            let generated = |state: &DocumentState| {
                render_markdown(
                    Path::new(state.audio_path()),
                    &state.results.speaker_segments,
                )
            };
            (
                self.store(root.clone(), &generated(&root)),
                self.store(first.clone(), &generated(&first)),
                self.store(second, "# Edited by hand\n"),
            )
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    fn sha256(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    }

    fn record(input_audio_checksum: Option<String>) -> ProcessingRecord {
        ProcessingRecord {
            processing_revision: 1,
            completed_at_ms: 1,
            input_audio_checksum,
            asr: Default::default(),
            diarization: Default::default(),
            alignment: Default::default(),
            runtime: Default::default(),
        }
    }

    #[test]
    fn audio_status_reports_present_missing_mismatched_and_unverifiable_audio() {
        let fixture = Fixture::new("status");
        let path = fixture.audio("meeting.wav", b"original audio");
        let (root, child, _) = fixture.processed_asset(&path, b"original audio");
        let never_processed = fixture.store(
            DocumentState::new(DocumentId::new("imported"), "Imported", &path),
            "",
        );
        let without_checksum = {
            let mut state = DocumentState::new(DocumentId::new("unhashed"), "Unhashed", &path);
            state.record_processing(record(None));
            fixture.store(state, "")
        };
        let gone = fixture.store(
            DocumentState::new(DocumentId::new("gone"), "Gone", fixture.missing("gone.wav")),
            "",
        );

        let present = AudioStatus::Present {
            path: path.clone(),
            checksum: sha256(b"original audio"),
        };
        assert_eq!(
            fixture.workspace.audio_status(root.id()),
            Ok(present.clone())
        );
        // A child is verified against the checksum its root recorded.
        assert_eq!(fixture.workspace.audio_status(child.id()), Ok(present));
        assert_eq!(
            fixture.workspace.audio_status(never_processed.id()),
            Ok(AudioStatus::Unverifiable { path: path.clone() })
        );
        assert_eq!(
            fixture.workspace.audio_status(without_checksum.id()),
            Ok(AudioStatus::Unverifiable { path: path.clone() })
        );
        assert_eq!(
            fixture.workspace.audio_status(gone.id()),
            Ok(AudioStatus::Missing {
                path: gone.audio_path().into()
            })
        );

        std::fs::write(&path, b"re-encoded audio").unwrap();
        assert_eq!(
            fixture.workspace.audio_status(child.id()),
            Ok(AudioStatus::ChecksumMismatch {
                path: path.clone(),
                expected: sha256(b"original audio"),
                actual: sha256(b"re-encoded audio"),
            })
        );
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            fixture.workspace.audio_status(root.id()),
            Ok(AudioStatus::Missing { path })
        );
        let unknown = DocumentId::new("does-not-exist");
        assert_eq!(
            fixture.workspace.audio_status(&unknown),
            Err(AudioSourceError::NotFound(unknown))
        );
    }

    #[test]
    fn relinking_the_matching_file_updates_every_document_of_the_asset_only() {
        let fixture = Fixture::new("relink");
        let old_path = fixture.missing("moved-away.wav");
        let (root, first, second) = fixture.processed_asset(&old_path, b"original audio");
        let other = fixture.store(
            DocumentState::new(DocumentId::new("other"), "Other", &old_path),
            "# Other\n",
        );
        let new_path = fixture.audio("found-again.wav", b"original audio");
        let before: Vec<_> = [&root, &first, &second]
            .iter()
            .map(|state| fixture.records.record(state.id()))
            .collect();

        // Through a child: the root and the other descendant follow.
        let written = fixture
            .workspace
            .relink_audio(first.id(), first.revision(), new_path.clone())
            .unwrap();

        let mut written_ids: Vec<_> = written.iter().map(|state| state.id().clone()).collect();
        written_ids.sort();
        let mut expected_ids = vec![root.id().clone(), first.id().clone(), second.id().clone()];
        expected_ids.sort();
        assert_eq!(written_ids, expected_ids);
        for previous in &before {
            let current = fixture.records.record(previous.state.id());
            assert_eq!(current.state.audio_path(), new_path);
            assert_eq!(current.state.revision(), previous.state.revision() + 1);
            assert_eq!(current.state.results, previous.state.results);
            assert_eq!(current.state.processing, previous.state.processing);
            assert_eq!(current.state.hierarchy, previous.state.hierarchy);
            assert_eq!(current.state.content, previous.state.content);
            let mut source = previous.state.source.clone();
            source.audio_path = new_path.clone();
            assert_eq!(current.state.source, source);
        }
        // A generated body follows the new path; an edited one is kept.
        assert_eq!(
            fixture.records.record(root.id()).markdown,
            render_markdown(Path::new(&new_path), &root.results.speaker_segments)
        );
        assert_eq!(
            fixture.records.record(second.id()).markdown,
            "# Edited by hand\n"
        );
        assert_eq!(
            fixture.records.record(other.id()),
            DocumentRecord {
                state: other,
                markdown: "# Other\n".into(),
            }
        );
        assert!(matches!(
            fixture.workspace.audio_status(second.id()),
            Ok(AudioStatus::Present { .. })
        ));
    }

    #[test]
    fn relinking_writes_nothing_for_another_file_a_missing_file_or_a_stale_revision() {
        let fixture = Fixture::new("relink-refused");
        let old_path = fixture.missing("moved-away.wav");
        let (root, first, second) = fixture.processed_asset(&old_path, b"original audio");
        let different = fixture.audio("different.wav", b"another recording");
        let matching = fixture.audio("matching.wav", b"original audio");
        let stored = || fixture.records.records.lock().unwrap().clone();
        let before = stored();

        assert_eq!(
            fixture
                .workspace
                .relink_audio(root.id(), root.revision(), different.clone()),
            Err(AudioSourceError::ChecksumMismatch {
                path: different,
                expected: sha256(b"original audio"),
                actual: sha256(b"another recording"),
            })
        );
        let nowhere = fixture.missing("nowhere.wav");
        assert_eq!(
            fixture
                .workspace
                .relink_audio(root.id(), root.revision(), nowhere.clone()),
            Err(AudioSourceError::FileMissing { path: nowhere })
        );
        assert_eq!(
            fixture
                .workspace
                .relink_audio(root.id(), root.revision() - 1, matching.clone()),
            Err(AudioSourceError::RevisionConflict {
                document_id: root.id().clone(),
                expected: root.revision() - 1,
                actual: root.revision(),
            })
        );
        assert_eq!(stored(), before);

        // A document of the asset written while the command runs fails it
        // before its first write.
        let second_id = second.id().as_str().to_string();
        *fixture.records.on_read.lock().unwrap() = Some(Box::new(move |records| {
            records.get_mut(&second_id).unwrap().state.schema.revision += 1;
        }));
        assert_eq!(
            fixture
                .workspace
                .relink_audio(first.id(), first.revision(), matching),
            Err(AudioSourceError::RevisionConflict {
                document_id: second.id().clone(),
                expected: second.revision(),
                actual: second.revision() + 1,
            })
        );
        for id in [root.id(), first.id(), second.id()] {
            assert_eq!(fixture.records.record(id).state.audio_path(), old_path);
        }
        assert_eq!(fixture.records.record(root.id()), before["root"]);
        assert_eq!(fixture.records.record(first.id()), before["first"]);
    }

    #[test]
    fn unprocessed_documents_relink_without_a_checksum_check() {
        let fixture = Fixture::new("relink-unprocessed");
        let imported = fixture.store(
            DocumentState::new(
                DocumentId::new("imported"),
                "Imported",
                fixture.missing("lost.wav"),
            ),
            "# Imported\n",
        );
        let any_file = fixture.audio("any.wav", b"whatever was found");

        let written = fixture
            .workspace
            .relink_audio(imported.id(), imported.revision(), any_file.clone())
            .unwrap();

        assert_eq!(written.len(), 1);
        assert_eq!(written[0].audio_path(), any_file);
        assert_eq!(written[0].revision(), imported.revision() + 1);
        assert_eq!(
            fixture.workspace.audio_status(imported.id()),
            Ok(AudioStatus::Unverifiable { path: any_file })
        );
        // Relinking to where it already points writes nothing.
        let unchanged = fixture
            .workspace
            .relink_audio(
                imported.id(),
                written[0].revision(),
                written[0].audio_path(),
            )
            .unwrap();
        assert!(unchanged.is_empty());
    }
}
