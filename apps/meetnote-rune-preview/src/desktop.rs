use crate::assistant_service;
use application::{
    AgentHost, DocumentRepository, DocumentService, DocumentsRoot, SettingsCapabilityPort,
    WorkspaceAccessPolicy, WorkspaceCapabilityPort, WorkspaceService, validate_documents_root,
};
#[cfg(feature = "host-test-fake-audio")]
use contracts::{AsrRequest, AsrResult, BackendError, DiarizationRequest, DiarizationResult};
use contracts::{CapabilityCallRequest, CapabilityCallResponse, ModelRef};
use llm::{LlmOutputBlock, LlmProvider};

use crate::audio_services::{AudioBackends, model_catalog, processing_options};
use models::ModelManager;
use persistence::FileDocumentRepository;
use runtime::JobRuntime;
use settings::{AppSettings, JsonSettingsRepository, SettingsService};
use std::env;
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Downloaded models; it can never hold the document packages.
const MODEL_DIRECTORY: &str = "models";

fn open_documents_repository(root: &Path) -> Result<Arc<dyn DocumentRepository>, String> {
    Ok(Arc::new(FileDocumentRepository::open(root)?))
}

struct Host {
    audio: AudioBackends,
    assistant: Arc<assistant_service::DesktopAssistantService>,
    documents_root: PathBuf,
    settings: SettingsService,
    snapshot: AppSettings,
    workspace: WorkspaceService,
}

impl Host {
    fn open(_is_agent: bool) -> Result<Self, String> {
        let settings = SettingsService::open(Arc::new(JsonSettingsRepository::open(
            "data/settings/settings.json",
        )))
        .map_err(|error| format!("cannot load settings: {error}"))?;
        let snapshot = settings
            .snapshot()
            .map_err(|error| format!("cannot read settings: {error}"))?;
        let audio = AudioBackends::registered(Path::new(&snapshot.speaker_library_path));
        let documents_root = PathBuf::from(&snapshot.documents_root);
        validate_documents_root(
            &documents_root,
            Some(Path::new(&snapshot.speaker_library_path)),
            Path::new(MODEL_DIRECTORY),
        )
        .map_err(|error| format!("invalid documents root: {error}"))?;
        let repository = open_documents_repository(&documents_root)
            .map_err(|error| format!("cannot open document repository: {error}"))?;
        let documents = DocumentService::new(repository);
        let jobs = Arc::new(JobRuntime::default());
        let assistant = Arc::new(
            assistant_service::DesktopAssistantService::with_job_runtime(
                settings.clone(),
                documents.clone(),
                jobs.clone(),
            ),
        );
        #[cfg(feature = "host-test-fake-audio")]
        let processing =
            if _is_agent && env::var("MEETNOTE_AGENT_HOST_FAKE_AUDIO").as_deref() == Ok("1") {
                application::ProcessingService::new(
                    Arc::new(HostFakeAsr),
                    Arc::new(HostFakeDiarization),
                )
                .with_job_runtime(jobs)
            } else {
                audio.processing(jobs)
            };
        #[cfg(not(feature = "host-test-fake-audio"))]
        let processing = audio.processing(jobs);
        let workspace =
            WorkspaceService::with_options(processing, documents, processing_options(&snapshot))
                .with_documents_root(DocumentsRoot::new(
                    &documents_root,
                    MODEL_DIRECTORY,
                    open_documents_repository,
                ));
        Ok(Self {
            audio,
            assistant,
            documents_root,
            settings,
            snapshot,
            workspace,
        })
    }

    fn load_models(&self) -> (ModelManager, ModelRef, ModelRef) {
        let asr_backend = self
            .snapshot
            .asr_backend_id
            .as_deref()
            .unwrap_or("whisper-rs");
        let diarization_backend = self
            .snapshot
            .diarization_backend_id
            .as_deref()
            .unwrap_or("diarization-native");
        let catalog =
            model_catalog().unwrap_or_else(|error| panic!("failed to open model catalog: {error}"));
        let manager = self.audio.models(catalog, Path::new(MODEL_DIRECTORY));
        let asr = manager
            .configured_model_ref("asr", asr_backend, self.snapshot.asr_model.as_ref())
            .unwrap_or_else(|_| unconfigured_model(asr_backend));
        let diarization = manager
            .configured_model_ref(
                "diarization",
                diarization_backend,
                self.snapshot.diarization_model.as_ref(),
            )
            .unwrap_or_else(|_| unconfigured_model(diarization_backend));
        (manager, asr, diarization)
    }
}

/// Handles the desktop application's headless commands before a UI starts.
pub fn run_command_line() -> bool {
    let args: Vec<String> = env::args().collect();
    let launch_dir = prepare_data_directory(&args).unwrap_or_else(|error| {
        let message = format!("cannot open MeetNote's data directory: {error}");
        match args.get(1).map(String::as_str) {
            Some("agent") => fail_agent_adapter(&message),
            Some("process") => fail_process(message),
            _ => panic!("{message}"),
        }
    });
    match args.get(1).map(String::as_str) {
        Some("agent") => {
            let host = Host::open(true).unwrap_or_else(|error| fail_agent_adapter(&error));
            run_agent_cli(
                &args,
                &host.assistant,
                host.settings,
                host.snapshot.assistant.write_mode,
                host.workspace,
            );
        }
        Some("process") => {
            if args.len() != 3 {
                eprintln!("usage: meetnote process <audio>");
                std::process::exit(2);
            }
            let host = Host::open(false).unwrap_or_else(|error| fail_process(error));
            let (_, asr, diarization) = host.load_models();
            let audio_path = if let Some(launch_dir) = launch_dir {
                launch_dir
                    .join(&args[2])
                    .into_os_string()
                    .into_string()
                    .unwrap_or_else(|_| fail_process("the audio path is not valid Unicode".into()))
            } else {
                args[2].clone()
            };
            let project = host
                .workspace
                .import_audio(audio_path)
                .unwrap_or_else(|error| fail_process(error));
            let result = futures::executor::block_on(host.workspace.process_project(
                project.id().clone(),
                asr,
                diarization,
            ))
            .unwrap_or_else(|error| fail_process(error.message));
            println!("processed document {}", result.id().as_str());
            println!(
                "document package: {}",
                host.documents_root.join(result.id().as_str()).display()
            );
            true
        }
        _ => false,
    }
}

fn prepare_data_directory(args: &[String]) -> std::io::Result<Option<PathBuf>> {
    let requested = env::var_os("MEETNOTE_DATA_DIR").map(PathBuf::from);
    let directory = match requested {
        Some(directory) => directory,
        None if cfg!(target_os = "macos")
            && !args
                .iter()
                .any(|arg| matches!(arg.as_str(), "--sample" | "--scene" | "--help")) =>
        {
            let executable = env::current_exe()?;
            let Some(macos) = executable
                .parent()
                .filter(|path| path.ends_with("Contents/MacOS"))
            else {
                return Ok(None);
            };
            if !macos
                .parent()
                .and_then(Path::parent)
                .and_then(Path::extension)
                .is_some_and(|extension| extension == "app")
            {
                return Ok(None);
            }
            let home = env::var_os("HOME")
                .ok_or_else(|| std::io::Error::other("the home directory is unavailable"))?;
            PathBuf::from(home).join("Library/Application Support/MeetNote")
        }
        None => return Ok(None),
    };
    let launch_directory = env::current_dir()?;
    std::fs::create_dir_all(&directory)?;
    env::set_current_dir(directory)?;
    Ok(Some(launch_directory))
}

#[cfg(feature = "host-test-fake-audio")]
struct HostFakeAsr;

#[cfg(feature = "host-test-fake-audio")]
impl application::AsrBackend for HostFakeAsr {
    fn descriptor(&self) -> contracts::BackendDescriptor {
        application::asr_descriptor("fake-asr", &["timestamps", "fixture"])
    }

    fn probe(&self, _model: &ModelRef) -> Result<(), BackendError> {
        Ok(())
    }

    fn transcribe(
        &self,
        request: AsrRequest,
        context: runtime::JobContext,
    ) -> application::BackendFuture<AsrResult> {
        Box::pin(async move {
            for _ in 0..20 {
                if context.is_cancelled() {
                    return Err(BackendError {
                        code: contracts::BackendErrorCode::Cancelled,
                        message: "fixture ASR cancelled".into(),
                    });
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Ok(AsrResult {
                backend: application::asr_descriptor("fake-asr", &["timestamps", "fixture"]),
                model: request.model,
                language: request.language,
                segments: vec![
                    domain::TranscriptSegment {
                        id: "segment-1".into(),
                        start_ms: 0,
                        end_ms: 1_000,
                        text: "fixture fact one".into(),
                    },
                    domain::TranscriptSegment {
                        id: "segment-2".into(),
                        start_ms: 1_000,
                        end_ms: 2_000,
                        text: "fixture fact two".into(),
                    },
                ],
            })
        })
    }
}

#[cfg(feature = "host-test-fake-audio")]
struct HostFakeDiarization;

#[cfg(feature = "host-test-fake-audio")]
impl application::DiarizationBackend for HostFakeDiarization {
    fn descriptor(&self) -> contracts::BackendDescriptor {
        application::diarization_descriptor("fake-diarization", &["offline", "fixture"])
    }

    fn probe(&self, _model: &ModelRef) -> Result<(), BackendError> {
        Ok(())
    }

    fn stage_provenance(&self, model: &ModelRef) -> application::DiarizationStageProvenance {
        application::DiarizationStageProvenance {
            segmentation: Some(model.clone()),
            embedding: Some(model.clone()),
            clustering: Some(domain::AlgorithmProvenance {
                id: "fake-clustering".into(),
                revision: Some("1".into()),
                parameters: None,
            }),
        }
    }

    fn diarize(
        &self,
        request: DiarizationRequest,
        context: runtime::JobContext,
    ) -> application::BackendFuture<DiarizationResult> {
        Box::pin(async move {
            for _ in 0..20 {
                if context.is_cancelled() {
                    return Err(BackendError {
                        code: contracts::BackendErrorCode::Cancelled,
                        message: "fixture diarization cancelled".into(),
                    });
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Ok(DiarizationResult {
                backend: application::diarization_descriptor(
                    "fake-diarization",
                    &["offline", "fixture"],
                ),
                model: request.model,
                segments: vec![domain::DiarizationSegment {
                    id: "speaker-segment-1".into(),
                    speaker_id: "speaker-1".into(),
                    speaker_profile_id: None,
                    speaker_label: None,
                    start_ms: 0,
                    end_ms: 2_000,
                    confidence: Some(1.0),
                }],
                speaker_count: Some(1),
                embedding_observations: Vec::new(),
                speaker_profile_candidates: Vec::new(),
            })
        })
    }
}

fn run_agent_cli(
    args: &[String],
    assistant: &assistant_service::DesktopAssistantService,
    settings: settings::SettingsService,
    write_mode: contracts::AgentWriteMode,
    workspace: WorkspaceService,
) -> ! {
    let documents = workspace.documents();
    match args.get(2).map(String::as_str) {
        Some("host") if args.get(3).map(String::as_str) == Some("probe") => {
            run_host_probe(assistant, settings, workspace);
        }
        Some("host") if args.get(3).map(String::as_str) == Some("discover") => {
            run_host_discover(&args[4..], settings, workspace);
        }
        Some("host") if args.get(3).map(String::as_str) == Some("call") => {
            run_host_call(&args[4..], settings, write_mode, workspace);
        }
        Some("host") if args.get(3).map(String::as_str) == Some("serve") => {
            run_host_serve(&args[4..], settings, write_mode, workspace);
        }
        Some("call") if args.get(3).map(String::as_str) == Some("workspace") => {
            run_workspace_call(&args[4..], settings, write_mode, workspace);
        }
        Some("describe") if args.get(3).map(String::as_str) == Some("--json") => {
            let access = parse_agent_access(args, 4);
            let gateway = WorkspaceCapabilityPort::new(documents, access)
                .with_write_mode(write_mode)
                .with_actor(contracts::AgentActor::ExternalAgent);
            write_agent_json(&gateway.describe());
            std::process::exit(0);
        }
        Some("call") if args.get(4).map(String::as_str) == Some("--json-stdin") => {
            let access = parse_agent_access(args, 5);
            let gateway = WorkspaceCapabilityPort::new(documents.clone(), access.clone())
                .with_write_mode(write_mode)
                .with_actor(contracts::AgentActor::ExternalAgent);
            let tool_name = args
                .get(3)
                .unwrap_or_else(|| fail_agent_adapter("missing agent tool name"));
            let mut json = String::new();
            std::io::stdin()
                .read_to_string(&mut json)
                .unwrap_or_else(|error| fail_agent_adapter(&format!("cannot read stdin: {error}")));
            let request: contracts::CapabilityRequest =
                serde_json::from_str(&json).unwrap_or_else(|error| {
                    fail_agent_adapter(&format!("invalid request JSON: {error}"))
                });
            let mut call = CapabilityCallRequest::new(
                request.request_id.clone(),
                request.tool_call_id.clone(),
                contracts::WORKSPACE_CAPABILITY_ID,
                contracts::WORKSPACE_CAPABILITY_VERSION,
                tool_name,
                contracts::CapabilityAuthorization {
                    all_documents: request.scope.all_documents,
                    document_ids: request.scope.document_ids.clone(),
                    ..contracts::CapabilityAuthorization::default()
                },
                request.arguments,
            );
            call.schema_version = request.schema_version;
            let host = AgentHost::new(
                gateway,
                application::AudioCapabilityPort::new(
                    workspace,
                    std::iter::empty::<domain::ProjectId>(),
                ),
                SettingsCapabilityPort::new(settings, Vec::<String>::new()),
                std::iter::empty::<String>(),
            );
            let response = host.dispatch(call);
            let legacy = host_response_as_workspace(response);
            let exit_code = if legacy.ok { 0 } else { 2 };
            write_workspace_response(legacy);
            std::process::exit(exit_code);
        }
        Some(operation @ ("confirm" | "reject" | "undo")) => {
            let token = args
                .get(3)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| fail_agent_adapter("agent control operation requires a token"));
            let request_id = args
                .get(4)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| {
                    fail_agent_adapter("agent control operation requires a request ID")
                });
            let access = parse_agent_access(args, 5);
            let gateway = WorkspaceCapabilityPort::new(documents, access)
                .with_write_mode(write_mode)
                .with_actor(contracts::AgentActor::ExternalAgent);
            let (operation_name, arguments) = match operation {
                "confirm" => (
                    "confirm_mutation",
                    serde_json::json!({"confirmationId": token}),
                ),
                "reject" => (
                    "reject_mutation",
                    serde_json::json!({"confirmationId": token}),
                ),
                "undo" => ("undo_mutation", serde_json::json!({"undoToken": token})),
                _ => unreachable!(),
            };
            let call = CapabilityCallRequest::new(
                request_id,
                format!("control-{operation}"),
                contracts::WORKSPACE_CAPABILITY_ID,
                contracts::WORKSPACE_CAPABILITY_VERSION,
                operation_name,
                contracts::CapabilityAuthorization::default(),
                arguments,
            );
            let host = AgentHost::new(
                gateway,
                application::AudioCapabilityPort::new(
                    workspace,
                    std::iter::empty::<domain::ProjectId>(),
                ),
                SettingsCapabilityPort::new(settings, Vec::<String>::new()),
                std::iter::empty::<String>(),
            );
            let response = host_response_as_workspace(host.dispatch(call));
            let exit_code = if response.ok { 0 } else { 2 };
            write_workspace_response(response);
            std::process::exit(exit_code);
        }
        _ => fail_agent_adapter(
            "usage: meetnote agent host <probe|discover|call|serve> ... | meetnote agent describe --json [scope] | meetnote agent call [workspace] <tool-name> --json-stdin [scope] | meetnote agent <confirm|reject|undo> <token> <request-id> [scope]",
        ),
    }
}

fn run_host_probe(
    assistant: &assistant_service::DesktopAssistantService,
    settings: settings::SettingsService,
    workspace: WorkspaceService,
) -> ! {
    // Settings and credential errors are reported before any provider exists.
    let (provider, secret) = match assistant.active_provider() {
        Ok(Some(provider)) => provider,
        Ok(None) => fail_agent_adapter("no active Assistant connection is configured"),
        Err(error) => {
            write_agent_json(&serde_json::json!({
                "hostCallId": "host-probe",
                "ok": false,
                "error": {
                    "code": error.code,
                    "message": error.message,
                    "retryable": error.retryable,
                },
            }));
            std::process::exit(2);
        }
    };
    let host = AgentHost::new(
        WorkspaceCapabilityPort::new(
            workspace.clone().documents(),
            WorkspaceAccessPolicy::AllDocuments,
        ),
        application::AudioCapabilityPort::new(workspace, std::iter::empty()),
        SettingsCapabilityPort::new(settings, std::iter::empty::<String>()),
        std::iter::empty::<String>(),
    );
    match host.provider_probe(&provider, &llm::LlmCancellation::new()) {
        Ok(output) => {
            let text = output
                .blocks
                .iter()
                .filter_map(|block| match block {
                    LlmOutputBlock::Text { text } => Some(text.as_str()),
                    LlmOutputBlock::ToolCall { .. } => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            write_agent_json(&serde_json::json!({
                "hostCallId": "host-probe",
                "ok": true,
                "providerId": provider.descriptor().id,
                "requestedModel": output.requested_model,
                "resolvedModel": output.resolved_model,
                "stopReason": format!("{:?}", output.stop_reason),
                "text": text,
            }));
            std::process::exit(0);
        }
        Err(error) => {
            let mut error = application::agent_error_from_llm(error);
            assistant_service::redact_secret(&mut error.message, secret.as_deref());
            write_agent_json(&serde_json::json!({
                "hostCallId": "host-probe",
                "ok": false,
                "providerId": provider.descriptor().id,
                "error": error,
            }));
            std::process::exit(2);
        }
    }
}

fn run_host_discover(
    args: &[String],
    settings: settings::SettingsService,
    workspace: WorkspaceService,
) -> ! {
    let grants = parse_host_grants(args);
    let workspace_port =
        WorkspaceCapabilityPort::new(workspace.clone().documents(), grants.workspace_access())
            .with_write_mode(settings.snapshot().unwrap().assistant.write_mode)
            .with_actor(contracts::AgentActor::ExternalAgent);
    let host = AgentHost::new(
        workspace_port,
        application::AudioCapabilityPort::new(workspace, grants.projects.clone()),
        SettingsCapabilityPort::new(settings, grants.fields.clone()),
        grants.fields,
    );
    write_agent_json(&serde_json::json!({
        "schemaVersion": contracts::CAPABILITY_CALL_SCHEMA_VERSION,
        "host": {"id": application::AGENT_HOST_CAPABILITY_ID, "version": application::AGENT_HOST_CAPABILITY_VERSION},
        "capabilities": host.discover(),
        "envelope": {
            "request": contracts::schema_for_arguments::<CapabilityCallRequest>(),
            "response": contracts::schema_for_arguments::<CapabilityCallResponse>(),
        },
    }));
    std::process::exit(0);
}

fn run_host_call(
    args: &[String],
    settings: settings::SettingsService,
    write_mode: contracts::AgentWriteMode,
    workspace: WorkspaceService,
) -> ! {
    let grants = parse_host_grants(args);
    if grants.rest.iter().any(|value| value != "--json-stdin") {
        fail_agent_adapter("usage: meetnote agent host call --json-stdin [trusted grants]");
    }
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .unwrap_or_else(|error| fail_agent_adapter(&format!("cannot read Host request: {error}")));
    let request: CapabilityCallRequest = serde_json::from_str(&input)
        .unwrap_or_else(|error| fail_agent_adapter(&format!("invalid Host request JSON: {error}")));
    let access = grants.workspace_access();
    let fields = grants.fields.clone();
    let host = AgentHost::new(
        WorkspaceCapabilityPort::new(workspace.clone().documents(), access)
            .with_write_mode(write_mode)
            .with_actor(contracts::AgentActor::ExternalAgent),
        application::AudioCapabilityPort::new(workspace, grants.projects),
        SettingsCapabilityPort::new(settings, fields.clone()),
        fields,
    );
    let response = host.dispatch(request);
    let exit_code = if response.ok { 0 } else { 2 };
    write_agent_json(&response);
    std::process::exit(exit_code);
}

fn run_host_serve(
    args: &[String],
    settings: settings::SettingsService,
    write_mode: contracts::AgentWriteMode,
    workspace: WorkspaceService,
) -> ! {
    let grants = parse_host_grants(args);
    if !grants.rest.is_empty() {
        fail_agent_adapter("usage: meetnote agent host serve [trusted grants]");
    }
    let fields = grants.fields.clone();
    let host = AgentHost::new(
        WorkspaceCapabilityPort::new(workspace.clone().documents(), grants.workspace_access())
            .with_write_mode(write_mode)
            .with_actor(contracts::AgentActor::ExternalAgent),
        application::AudioCapabilityPort::new(workspace, grants.projects),
        SettingsCapabilityPort::new(settings, fields.clone()),
        fields,
    );
    let stdin = std::io::stdin();
    let mut stdout = std::io::BufWriter::new(std::io::stdout());
    for line in stdin.lock().lines() {
        let line = line
            .unwrap_or_else(|error| fail_agent_adapter(&format!("cannot read Host line: {error}")));
        if line.trim().is_empty() {
            continue;
        }
        let request: CapabilityCallRequest = serde_json::from_str(&line).unwrap_or_else(|error| {
            fail_agent_adapter(&format!("invalid Host request JSON: {error}"))
        });
        let response = host.dispatch(request);
        serde_json::to_writer(&mut stdout, &response).unwrap_or_else(|error| {
            fail_agent_adapter(&format!("cannot write Host response: {error}"))
        });
        use std::io::Write;
        stdout
            .write_all(b"\n")
            .and_then(|_| stdout.flush())
            .unwrap_or_else(|error| {
                fail_agent_adapter(&format!("cannot flush Host response: {error}"))
            });
    }
    std::process::exit(0);
}

fn run_workspace_call(
    args: &[String],
    settings: settings::SettingsService,
    write_mode: contracts::AgentWriteMode,
    workspace: WorkspaceService,
) -> ! {
    let tool_name = args
        .first()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| fail_agent_adapter("workspace call requires a tool name"));
    if args.get(1).map(String::as_str) != Some("--json-stdin") {
        fail_agent_adapter("workspace call requires --json-stdin");
    }
    let access = parse_agent_access(args, 2);
    let gateway = WorkspaceCapabilityPort::new(workspace.clone().documents(), access.clone())
        .with_write_mode(write_mode)
        .with_actor(contracts::AgentActor::ExternalAgent);
    let request: contracts::CapabilityRequest = read_service_request_as_workspace();
    let mut call = CapabilityCallRequest::new(
        request.request_id.clone(),
        request.tool_call_id.clone(),
        contracts::WORKSPACE_CAPABILITY_ID,
        contracts::WORKSPACE_CAPABILITY_VERSION,
        tool_name,
        contracts::CapabilityAuthorization {
            all_documents: request.scope.all_documents,
            document_ids: request.scope.document_ids.clone(),
            ..contracts::CapabilityAuthorization::default()
        },
        request.arguments,
    );
    call.schema_version = request.schema_version;
    let host = AgentHost::new(
        gateway,
        application::AudioCapabilityPort::new(workspace, std::iter::empty::<domain::ProjectId>()),
        SettingsCapabilityPort::new(settings, Vec::<String>::new()),
        std::iter::empty::<String>(),
    );
    let response = host_response_as_workspace(host.dispatch(call));
    let exit_code = if response.ok { 0 } else { 2 };
    write_agent_json(&response);
    std::process::exit(exit_code);
}

fn read_service_request_as_workspace() -> contracts::CapabilityRequest {
    let mut json = String::new();
    std::io::stdin()
        .read_to_string(&mut json)
        .unwrap_or_else(|error| fail_agent_adapter(&format!("cannot read stdin: {error}")));
    serde_json::from_str(&json).unwrap_or_else(|error| {
        fail_agent_adapter(&format!("invalid capability request JSON: {error}"))
    })
}

fn parse_agent_access(args: &[String], mut index: usize) -> WorkspaceAccessPolicy {
    let mut document_ids = Vec::new();
    let mut all_documents = false;
    while index < args.len() {
        match args[index].as_str() {
            "--document" => {
                let document_id = args
                    .get(index + 1)
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| fail_agent_adapter("--document requires a document id"));
                if !is_safe_document_id(document_id) {
                    fail_agent_adapter("--document requires a safe document package ID");
                }
                document_ids.push(domain::DocumentId::new(document_id.clone()));
                index += 2;
            }
            "--all-documents" => {
                all_documents = true;
                index += 1;
            }
            option => fail_agent_adapter(&format!("unknown agent authorization option: {option}")),
        }
    }
    if all_documents && !document_ids.is_empty() {
        fail_agent_adapter("--all-documents cannot be combined with --document");
    }
    if all_documents {
        WorkspaceAccessPolicy::AllDocuments
    } else {
        WorkspaceAccessPolicy::documents(document_ids)
    }
}

struct HostGrants {
    documents: Vec<domain::DocumentId>,
    all_documents: bool,
    projects: Vec<domain::ProjectId>,
    fields: Vec<String>,
    rest: Vec<String>,
}

impl HostGrants {
    fn workspace_access(&self) -> WorkspaceAccessPolicy {
        if self.all_documents {
            WorkspaceAccessPolicy::AllDocuments
        } else {
            WorkspaceAccessPolicy::documents(self.documents.clone())
        }
    }
}

fn parse_host_grants(args: &[String]) -> HostGrants {
    let mut grants = HostGrants {
        documents: Vec::new(),
        all_documents: false,
        projects: Vec::new(),
        fields: Vec::new(),
        rest: Vec::new(),
    };
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json-stdin" => grants.rest.push(args[index].clone()),
            "--all-documents" => grants.all_documents = true,
            "--document" => {
                let value = args
                    .get(index + 1)
                    .filter(|value| is_safe_document_id(value))
                    .unwrap_or_else(|| {
                        fail_agent_adapter("--document requires a safe document package ID")
                    });
                grants
                    .documents
                    .push(domain::DocumentId::new(value.clone()));
                index += 1;
            }
            "--project" => {
                let value = args
                    .get(index + 1)
                    .filter(|value| is_safe_document_id(value))
                    .unwrap_or_else(|| fail_agent_adapter("--project requires a safe project ID"));
                grants.projects.push(domain::ProjectId::new(value.clone()));
                index += 1;
            }
            "--field" => {
                let value = args
                    .get(index + 1)
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| fail_agent_adapter("--field requires a settings field"));
                grants.fields.push(value.clone());
                index += 1;
            }
            option => grants.rest.push(option.to_owned()),
        }
        index += 1;
    }
    if grants.all_documents && !grants.documents.is_empty() {
        fail_agent_adapter("--all-documents cannot be combined with --document");
    }
    grants
}

fn is_safe_document_id(document_id: &str) -> bool {
    !document_id.is_empty()
        && document_id != "."
        && document_id != ".."
        && !document_id.starts_with('.')
        && !document_id.contains('/')
        && !document_id.contains('\\')
}

fn write_agent_json(value: &impl serde::Serialize) {
    let json = serde_json::to_string(value)
        .unwrap_or_else(|error| fail_agent_adapter(&format!("cannot serialize response: {error}")));
    println!("{json}");
}

fn write_workspace_response(mut response: contracts::CapabilityResponse) {
    if let Some(result) = response.result.as_mut() {
        match result {
            contracts::AgentToolResult::Document(document) => {
                ensure_markdown_trailing_newline(&mut document.markdown);
            }
            contracts::AgentToolResult::Pending(pending) => {
                for diff in &mut pending.field_diffs {
                    if diff.field == "markdown" {
                        ensure_markdown_trailing_newline(&mut diff.before);
                        ensure_markdown_trailing_newline(&mut diff.after);
                    }
                }
            }
            contracts::AgentToolResult::Documents { .. }
            | contracts::AgentToolResult::Mutation(_) => {}
        }
    }
    write_agent_json(&response);
}

fn host_response_as_workspace(
    response: contracts::CapabilityCallResponse,
) -> contracts::CapabilityResponse {
    if let Some(result) = response.result
        && let Ok(result) = serde_json::from_value(result)
    {
        return contracts::CapabilityResponse::success(response.request_id, result);
    }
    let error = response.error.map(|error| {
        let code = match error.code.as_str() {
            "CAPABILITY_SCHEMA_UNSUPPORTED" | "CAPABILITY_REQUEST_INVALID" => {
                contracts::AgentErrorCode::ToolArgumentsInvalid
            }
            value => serde_json::from_value(serde_json::Value::String(value.into()))
                .unwrap_or(contracts::AgentErrorCode::ProviderResponseInvalid),
        };
        let mut mapped = contracts::AgentToolError::new(code, error.message, error.retryable);
        if let Some(context) = error.context {
            mapped.document_id = context
                .resource_id
                .clone()
                .filter(|_| context.resource_type.as_deref() == Some("document"));
            mapped.field = context.field.clone();
            mapped.expected_revision = context
                .expected_revision
                .as_deref()
                .and_then(|value| value.parse().ok());
            mapped.actual_revision = context
                .actual_revision
                .as_deref()
                .and_then(|value| value.parse().ok());
            mapped.context = context;
        }
        mapped
    });
    contracts::CapabilityResponse::failure(
        response.request_id,
        error.unwrap_or_else(|| {
            contracts::AgentToolError::new(
                contracts::AgentErrorCode::ProviderResponseInvalid,
                "Host returned neither a result nor an error",
                false,
            )
        }),
    )
}

fn ensure_markdown_trailing_newline(markdown: &mut Option<String>) {
    if let Some(markdown) = markdown
        && !markdown.is_empty()
        && !markdown.ends_with('\n')
    {
        markdown.push('\n');
    }
}

fn fail_agent_adapter(error: &str) -> ! {
    eprintln!("agent adapter failed: {error}");
    std::process::exit(3);
}

fn unconfigured_model(backend_id: &str) -> ModelRef {
    ModelRef {
        backend_id: backend_id.into(),
        model_id: "unconfigured".into(),
        revision: None,
        artifact_path: String::new(),
        artifacts: Vec::new(),
    }
}

fn fail_process(error: String) -> ! {
    eprintln!("processing failed: {error}");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use application::ProcessingService;
    use asr::{AsrBackendManager, AsrBackendRegistry};
    use diarization::{DiarizationBackendManager, DiarizationBackendRegistry};

    #[test]
    fn a_documents_root_switch_moves_the_file_repository_to_the_new_directory() {
        let base = std::env::temp_dir().join(format!(
            "meetnote-documents-root-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let old_root = base.join("documents");
        let new_root = base.join("elsewhere/documents");
        let processing = ProcessingService::new_with_providers(
            Arc::new(AsrBackendManager::new(AsrBackendRegistry::default())),
            Arc::new(DiarizationBackendManager::new(
                DiarizationBackendRegistry::default(),
            )),
        );
        let workspace = WorkspaceService::new(
            processing,
            DocumentService::new(open_documents_repository(&old_root).unwrap()),
        )
        .with_documents_root(DocumentsRoot::new(
            &old_root,
            base.join(MODEL_DIRECTORY),
            open_documents_repository,
        ));
        let first = workspace.import_audio("first.wav").unwrap();

        workspace.switch_documents_root(&new_root).unwrap();
        let second = workspace.import_audio("second.wav").unwrap();

        let state = |root: &Path, document: &domain::DocumentState| {
            root.join(document.id().as_str())
                .join(persistence::DOCUMENT_STATE_FILENAME)
                .is_file()
        };
        assert!(state(&old_root, &first));
        assert!(!state(&old_root, &second));
        assert!(state(&new_root, &second));
        let listed: Vec<_> = workspace
            .list_projects()
            .unwrap()
            .into_iter()
            .map(|document| document.id().clone())
            .collect();
        assert_eq!(listed, vec![second.id().clone()]);
        assert_eq!(
            workspace.switch_documents_root(base.join(MODEL_DIRECTORY)),
            Err(application::DocumentsRootError::Reserved {
                path: base.join(MODEL_DIRECTORY),
                location: application::ReservedLocation::ModelDirectory,
                reserved: base.join(MODEL_DIRECTORY),
            })
        );
        std::fs::remove_dir_all(base).unwrap();
    }
}
