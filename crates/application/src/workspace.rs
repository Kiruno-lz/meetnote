use crate::{run_pipeline, AsrBackend, BackendFuture, DiarizationBackend, PipelineResult};
use contracts::{AsrRequest, ComputeDevice, DiarizationRequest, ModelRef};
use domain::{MeetingProject, ProjectId, ProjectStatus, SpeakerTaggedSegment};
use runtime::{JobContext, JobRuntime};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ProcessingRequest {
    pub audio_path: String,
    pub asr_model: ModelRef,
    pub diarization_model: ModelRef,
    pub language: Option<String>,
    pub expected_speakers: Option<u32>,
    pub device: ComputeDevice,
}

#[derive(Clone)]
pub struct ProcessingService {
    asr_backend: Arc<dyn AsrBackend>,
    diarization_backend: Arc<dyn DiarizationBackend>,
    runtime: Arc<JobRuntime>,
}

impl ProcessingService {
    pub fn new(
        asr_backend: Arc<dyn AsrBackend>,
        diarization_backend: Arc<dyn DiarizationBackend>,
    ) -> Self {
        Self {
            asr_backend,
            diarization_backend,
            runtime: Arc::new(JobRuntime::default()),
        }
    }

    pub fn probe(&self, request: &ProcessingRequest) -> Result<(), contracts::BackendError> {
        self.asr_backend.probe(&request.asr_model)?;
        self.diarization_backend.probe(&request.diarization_model)
    }

    pub fn backend_ids(&self) -> (String, String) {
        (
            self.asr_backend.descriptor().id,
            self.diarization_backend.descriptor().id,
        )
    }

    pub fn contexts(&self) -> (JobContext, JobContext) {
        let (asr_context, _) = self.runtime.create_job();
        let (diarization_context, _) = self.runtime.create_job();
        (asr_context, diarization_context)
    }

    pub fn process(
        &self,
        request: ProcessingRequest,
        asr_context: JobContext,
        diarization_context: JobContext,
    ) -> BackendFuture<PipelineResult> {
        let asr_backend = self.asr_backend.clone();
        let diarization_backend = self.diarization_backend.clone();
        Box::pin(async move {
            run_pipeline(
                asr_backend.as_ref(),
                diarization_backend.as_ref(),
                AsrRequest {
                    audio_path: request.audio_path.clone(),
                    model: request.asr_model,
                    language: request.language,
                    device: request.device,
                },
                DiarizationRequest {
                    audio_path: request.audio_path,
                    model: request.diarization_model,
                    expected_speakers: request.expected_speakers,
                    device: request.device,
                },
                asr_context,
                diarization_context,
            )
            .await
        })
    }
}

#[derive(Clone)]
pub struct WorkspaceService {
    processing: ProcessingService,
    repository: Arc<dyn crate::ProjectRepository>,
    markdown_directory: PathBuf,
}

impl WorkspaceService {
    pub fn new(
        processing: ProcessingService,
        repository: Arc<dyn crate::ProjectRepository>,
        markdown_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            processing,
            repository,
            markdown_directory: markdown_directory.into(),
        }
    }

    pub fn list_projects(&self) -> Result<Vec<MeetingProject>, String> {
        self.repository.list()
    }

    pub fn import_audio(&self, audio_path: impl Into<String>) -> Result<MeetingProject, String> {
        let audio_path = audio_path.into();
        let path = Path::new(&audio_path);
        let title = path
            .file_stem()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("Untitled meeting")
            .to_string();
        let id = ProjectId::new(format!("project-{}", unique_suffix()));
        let project = MeetingProject::new(id, title, audio_path);
        self.repository.save(project.clone())?;
        Ok(project)
    }

    pub fn project(&self, project_id: &ProjectId) -> Result<Option<MeetingProject>, String> {
        self.repository.get(project_id)
    }

    pub fn process_project(
        &self,
        project_id: ProjectId,
        asr_model: ModelRef,
        diarization_model: ModelRef,
    ) -> BackendFuture<MeetingProject> {
        let workspace = self.clone();
        Box::pin(async move {
            let mut project = workspace
                .repository
                .get(&project_id)
                .map_err(persistence_error)?
                .ok_or_else(|| persistence_error("project does not exist"))?;
            project.status = ProjectStatus::Processing;
            workspace
                .repository
                .save(project.clone())
                .map_err(persistence_error)?;

            let request = ProcessingRequest {
                audio_path: project.audio_path.clone(),
                asr_model,
                diarization_model,
                language: None,
                expected_speakers: None,
                device: ComputeDevice::Auto,
            };
            let result = match workspace.processing.probe(&request) {
                Ok(()) => {
                    let (asr_context, diarization_context) = workspace.processing.contexts();
                    asr_context.started();
                    diarization_context.started();
                    workspace
                        .processing
                        .process(request, asr_context, diarization_context)
                        .await
                }
                Err(error) => Err(error),
            };
            let result = match result {
                Ok(result) => result,
                Err(error) => {
                    project.status = ProjectStatus::Failed;
                    let _ = workspace.repository.save(project);
                    return Err(error);
                }
            };

            project.status = ProjectStatus::Ready;
            project.transcript = result.asr.segments.clone();
            project.diarization = result.diarization.segments.clone();
            project.speaker_segments = result.aligned.clone();
            let markdown_path = workspace
                .markdown_directory
                .join(format!("{}.md", project.id.as_str()));
            fs::create_dir_all(&workspace.markdown_directory)
                .map_err(|error| persistence_error(error.to_string()))?;
            fs::write(
                &markdown_path,
                render_markdown(Path::new(&project.audio_path), &project.speaker_segments),
            )
            .map_err(|error| persistence_error(error.to_string()))?;
            project.markdown_path = Some(markdown_path.display().to_string());
            workspace
                .repository
                .save(project.clone())
                .map_err(persistence_error)?;
            Ok(project)
        })
    }
}

fn persistence_error(message: impl Into<String>) -> contracts::BackendError {
    contracts::BackendError {
        code: contracts::BackendErrorCode::PersistenceFailed,
        message: message.into(),
    }
}

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

pub fn render_markdown(audio_path: &Path, segments: &[SpeakerTaggedSegment]) -> String {
    let mut output = format!(
        "# Meeting transcript\n\nAudio: `{}`\n\n",
        audio_path.display()
    );
    for segment in segments {
        output.push_str(&format!(
            "- **[{}–{}] {}** {}\n",
            timestamp(segment.start_ms),
            timestamp(segment.end_ms),
            segment.speaker_id,
            segment.text.replace('\n', " ")
        ));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{asr_descriptor, diarization_descriptor, ProjectRepository};
    use contracts::{AsrResult, BackendDescriptor, BackendError, DiarizationResult};
    use domain::{DiarizationSegment, TranscriptSegment};
    use std::collections::BTreeMap;
    use std::sync::RwLock;

    #[derive(Default)]
    struct MemoryRepository {
        projects: RwLock<BTreeMap<String, MeetingProject>>,
    }

    impl ProjectRepository for MemoryRepository {
        fn save(&self, project: MeetingProject) -> Result<(), String> {
            self.projects
                .write()
                .unwrap()
                .insert(project.id.as_str().into(), project);
            Ok(())
        }

        fn get(&self, id: &ProjectId) -> Result<Option<MeetingProject>, String> {
            Ok(self.projects.read().unwrap().get(id.as_str()).cloned())
        }

        fn list(&self) -> Result<Vec<MeetingProject>, String> {
            Ok(self.projects.read().unwrap().values().cloned().collect())
        }
    }

    struct FakeAsr;

    impl AsrBackend for FakeAsr {
        fn descriptor(&self) -> BackendDescriptor {
            asr_descriptor("fake-asr", &["timestamps"])
        }

        fn probe(&self, _: &ModelRef) -> Result<(), BackendError> {
            Ok(())
        }

        fn transcribe(&self, request: AsrRequest, _: JobContext) -> BackendFuture<AsrResult> {
            Box::pin(async move {
                Ok(AsrResult {
                    backend: asr_descriptor("fake-asr", &["timestamps"]),
                    model: request.model,
                    language: Some("en".into()),
                    segments: vec![TranscriptSegment {
                        id: "t1".into(),
                        start_ms: 0,
                        end_ms: 1_000,
                        text: "hello".into(),
                    }],
                })
            })
        }
    }

    struct FakeDiarization;

    impl DiarizationBackend for FakeDiarization {
        fn descriptor(&self) -> BackendDescriptor {
            diarization_descriptor("fake-diarization", &["offline"])
        }

        fn probe(&self, _: &ModelRef) -> Result<(), BackendError> {
            Ok(())
        }

        fn diarize(
            &self,
            request: DiarizationRequest,
            _: JobContext,
        ) -> BackendFuture<DiarizationResult> {
            Box::pin(async move {
                Ok(DiarizationResult {
                    backend: diarization_descriptor("fake-diarization", &["offline"]),
                    model: request.model,
                    segments: vec![DiarizationSegment {
                        id: "d1".into(),
                        speaker_id: "speaker-1".into(),
                        start_ms: 0,
                        end_ms: 1_000,
                        confidence: Some(1.0),
                    }],
                    speaker_count: Some(1),
                })
            })
        }
    }

    #[test]
    fn workspace_processes_and_persists_a_project() {
        let repository = Arc::new(MemoryRepository::default());
        let workspace = WorkspaceService::new(
            ProcessingService::new(Arc::new(FakeAsr), Arc::new(FakeDiarization)),
            repository.clone(),
            std::env::temp_dir().join(format!("meetnote-workspace-{}", unique_suffix())),
        );
        let project = workspace.import_audio("fixture.wav").unwrap();
        let model = ModelRef {
            backend_id: "fake".into(),
            model_id: "fixture".into(),
            revision: None,
            artifact_path: "fixture-model".into(),
        };

        let result = futures::executor::block_on(workspace.process_project(
            project.id.clone(),
            model.clone(),
            model,
        ))
        .unwrap();

        assert_eq!(result.status, ProjectStatus::Ready);
        assert_eq!(result.speaker_segments[0].speaker_id, "speaker-1");
        assert!(result.markdown_path.is_some());
        assert_eq!(repository.get(&project.id).unwrap(), Some(result.clone()));
        if let Some(markdown_path) = result.markdown_path {
            let _ = fs::remove_file(markdown_path);
            if let Some(parent) = Path::new(&result.audio_path).parent() {
                let _ = parent;
            }
        }
    }
}

fn timestamp(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    let total_minutes = seconds / 60;
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    let seconds = seconds % 60;
    format!(
        "{hours:02}:{minutes:02}:{seconds:02}.{:03}",
        milliseconds % 1_000
    )
}
