use crate::{
    assistant_ui::{
        now_ms, AssistantController, AssistantScopeMode, AssistantSettingsSave, AssistantTaskState,
        AssistantUiError, AssistantUiService, ConnectionProbeKind,
    },
    audio::AudioCheck,
    components::{ProjectDrag, TreeMove, TreeShiftMove},
    controls::{
        self, ConfirmDialog, Dropdown, DropdownEvent, SelectOption, TextField, TextFieldEvent,
        TreeShift, TreeStep,
    },
    editor::DocumentEditor,
    i18n, pages,
    route::Route,
    speakers::{document_speakers, NameEntry, NameRows, SpeakerConfirmation, SpeakerProfiles},
    state::{ContentMode, NoticeChange, NoticeTone, ProcessingStage, UiState, TOAST_LIFETIME},
    theme,
};
use application::{
    AudioSourceError, BackendInventory, DocumentCommandError, DocumentIssue, DocumentSnapshot,
    DocumentsRootError, InstallationQuery, InstallationService, InstallationState,
    JsonSpeakerProfileStore, ProcessingOptions, SpeakerLibraryError, SpeakerLibraryService,
    WorkspaceService,
};
use contracts::{
    AgentScene, AgentWriteMode, ComputeDevice, DiarizationStageKind, DocumentCommand,
    DocumentCommandEnvelope, DocumentCommandResult, DocumentRecord, ModelRef,
};
use domain::{DocumentState, ProjectId, ProjectStatus};
use futures::{FutureExt, StreamExt};
use gpui::{
    prelude::*, App, Bounds, ClickEvent, Context, Entity, FocusHandle, PathPromptOptions,
    ScrollHandle, Task, Window, WindowBounds, WindowOptions,
};
use gpui_platform::application;
use models::manager::{
    DownloadEvent, DownloadId, DownloadState, DownloadTask, ModelManager, ModelManagerError,
    ReprobeTask,
};
use models::{ModelAvailability, ModelCatalog, ModelInstallState};
use runtime::{JobContext, JobEvent};
use settings::{
    AppSettings, AssistantAuthMode, AssistantConnectionPatch, AssistantConnectionProfile,
    AssistantSettings, CredentialUpdate, LlmProtocol, MaterialPreference, SettingsService,
    ThemePreference,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

type SettingsChange = Arc<dyn Fn(&mut AppSettings) + Send + Sync>;

enum SettingsWrite {
    /// An instant-save field. It is applied to the persisted settings alone,
    /// so it never carries the unsaved Assistant draft.
    Field {
        change: SettingsChange,
        saved_notice: String,
    },
    /// The explicit save and its raw form for acknowledging later edits.
    Assistant {
        save: AssistantSettingsSave,
        submitted: AssistantForm,
    },
    /// A documents-root switch and the save of `documents_root` it needs.
    DocumentsRoot { root: PathBuf },
}

/// A failed settings write and, after a revision conflict, the settings the
/// repository holds, which the next write compares against.
struct SettingsWriteFailure {
    error: AssistantUiError,
    reloaded: Option<AppSettings>,
}

/// A documents root saved in the settings; `switched` is false when the
/// workspace was already on it.
struct DocumentsRootSwitched {
    saved: AppSettings,
    switched: bool,
}

/// Why a documents-root switch failed to save its requested setting.
enum DocumentsRootSwitchFailure {
    /// The workspace refused the root; nothing changed.
    Refused(DocumentsRootError),
    /// The setting was not saved. A workspace that `switched` was switched
    /// back; `restore` is why that failed, if it did.
    NotSaved {
        failure: SettingsWriteFailure,
        switched: bool,
        restore: Option<DocumentsRootError>,
        active_root: Option<PathBuf>,
    },
}

/// Switches `workspace` to `root`, then saves it as `documents_root` in the
/// settings compared against `expected`; a failed save switches the
/// workspace back. Blocking: it runs on the settings write's worker.
fn switch_documents_root(
    workspace: &WorkspaceService,
    settings: &SettingsService,
    expected: &AppSettings,
    root: PathBuf,
) -> Result<DocumentsRootSwitched, DocumentsRootSwitchFailure> {
    let previous = workspace.documents_root();
    workspace
        .switch_documents_root(&root)
        .map_err(DocumentsRootSwitchFailure::Refused)?;
    // A switch to the root the workspace is on changes nothing.
    let switched = workspace.documents_root() != previous;
    let mut next = expected.clone();
    next.documents_root = root.display().to_string();
    let saved = settings
        .update_if_equal(expected, next)
        .map_err(|error| AssistantUiError::from_settings(&error));
    match reload_after_conflict(settings, saved) {
        Ok(saved) => Ok(DocumentsRootSwitched { saved, switched }),
        Err(failure) => {
            let restore = previous
                .filter(|_| switched)
                .and_then(|previous| workspace.switch_documents_root(previous).err());
            let active_root = restore.as_ref().and_then(|_| workspace.documents_root());
            Err(DocumentsRootSwitchFailure::NotSaved {
                active_root,
                failure,
                switched,
                restore,
            })
        }
    }
}

/// Re-reads the repository after a revision conflict, on the write's worker,
/// so a retry sees what another writer saved. A reload that fails reports
/// its own error instead.
fn reload_after_conflict(
    settings: &SettingsService,
    result: Result<AppSettings, AssistantUiError>,
) -> Result<AppSettings, SettingsWriteFailure> {
    result.map_err(|error| {
        if !error.is_settings_conflict() {
            return SettingsWriteFailure {
                error,
                reloaded: None,
            };
        }
        match settings.reload() {
            Ok(latest) => SettingsWriteFailure {
                error,
                reloaded: Some(latest),
            },
            Err(reload) => SettingsWriteFailure {
                error: AssistantUiError::from_settings(&reload),
                reloaded: None,
            },
        }
    })
}

#[derive(Debug, Clone)]
pub(crate) struct ModelOption {
    id: String,
    installed: bool,
    locale: i18n::Locale,
}

impl ModelOption {
    fn from_availability(availability: &ModelAvailability, locale: i18n::Locale) -> Self {
        Self {
            id: availability.model.id.clone(),
            installed: availability.install_state == ModelInstallState::Installed,
            locale,
        }
    }

    fn status_label(&self) -> &'static str {
        install_status_label(self.locale, self.installed)
    }
}

fn install_status_label(locale: i18n::Locale, installed: bool) -> &'static str {
    i18n::text(
        locale,
        if installed {
            "downloaded"
        } else {
            "not_downloaded"
        },
    )
}

impl ModelOption {
    /// The menu entry: the model id with its install status; a model that is
    /// not downloaded cannot be chosen.
    fn option(&self) -> SelectOption {
        SelectOption::text(self.id.clone())
            .detail(self.status_label())
            .disabled(!self.installed)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct StageModelOption {
    id: String,
    installed: bool,
    locale: i18n::Locale,
}

impl StageModelOption {
    fn status_label(&self) -> &'static str {
        install_status_label(self.locale, self.installed)
    }
}

impl StageModelOption {
    fn option(&self) -> SelectOption {
        SelectOption::text(self.id.clone())
            .detail(self.status_label())
            .disabled(!self.installed)
    }
}

pub(crate) struct SettingsSelectors {
    pub(crate) theme: Entity<Dropdown>,
    pub(crate) material: Entity<Dropdown>,
    pub(crate) language: Entity<Dropdown>,
    pub(crate) asr_backend: Entity<Dropdown>,
    pub(crate) asr_model: Entity<Dropdown>,
    pub(crate) transcription_language: Entity<Dropdown>,
    pub(crate) diarization_backend: Entity<Dropdown>,
    pub(crate) diarization_model: Entity<Dropdown>,
    pub(crate) diarization_segmentation_model: Entity<Dropdown>,
    pub(crate) diarization_embedding_model: Entity<Dropdown>,
    pub(crate) diarization_clustering: Entity<Dropdown>,
    pub(crate) expected_speakers: Entity<Dropdown>,
    pub(crate) compute_device: Entity<Dropdown>,
}

pub(crate) struct AssistantInputs {
    pub(crate) prompt: Entity<TextField>,
    pub(crate) connection_id: Entity<TextField>,
    pub(crate) connection_name: Entity<TextField>,
    pub(crate) base_url: Entity<TextField>,
    pub(crate) model: Entity<TextField>,
    pub(crate) api_key: Entity<TextField>,
    pub(crate) api_key_file: Entity<TextField>,
    pub(crate) context_window_tokens: Entity<TextField>,
    pub(crate) timeout_ms: Entity<TextField>,
    pub(crate) max_output_tokens: Entity<TextField>,
    pub(crate) max_tool_rounds: Entity<TextField>,
    pub(crate) max_validation_retries: Entity<TextField>,
}

impl AssistantInputs {
    /// The fields loaded from `settings`: the active connection's profile,
    /// or an empty profile, and the loop limits.
    fn new(
        locale: i18n::Locale,
        settings: &AssistantSettings,
        window: &mut Window,
        cx: &mut Context<RootView>,
    ) -> Self {
        let mut field = |key: &'static str, placeholder: &'static str, secret: bool| {
            let label = i18n::text(locale, key);
            cx.new(|cx| {
                if secret {
                    TextField::secret(label, placeholder, window, cx)
                } else {
                    TextField::new(label, placeholder, window, cx)
                }
            })
        };
        let prompt = field(
            "assistant_prompt",
            i18n::text(locale, "ask_placeholder"),
            false,
        );
        let this = Self {
            connection_id: field("assistant_connection_id", "local-litellm", false),
            connection_name: field("assistant_connection_name", "Local LiteLLM", false),
            base_url: field("assistant_base_url", "http://127.0.0.1:4000", false),
            model: field("assistant_model", "assistant-default", false),
            api_key: field("assistant_api_key", "API key (stored in settings)", true),
            api_key_file: field("assistant_api_key_file", "path/to/api-key.txt", false),
            context_window_tokens: field("assistant_context_window", "128000", false),
            timeout_ms: field("assistant_timeout", "120000", false),
            max_output_tokens: field("assistant_max_output", "4096", false),
            max_tool_rounds: field("assistant_max_rounds", "8", false),
            max_validation_retries: field("assistant_max_retries", "3", false),
            prompt,
        };
        this.prompt.update(cx, |prompt, cx| prompt.set_large(cx));
        this.load_profile(settings.active_connection(), cx);
        this.load_limits(settings, cx);
        this
    }

    /// Every field with the translation key of its label.
    pub(crate) fn labelled(&self) -> [(&Entity<TextField>, &'static str); 12] {
        [
            (&self.prompt, "assistant_prompt"),
            (&self.connection_id, "assistant_connection_id"),
            (&self.connection_name, "assistant_connection_name"),
            (&self.base_url, "assistant_base_url"),
            (&self.model, "assistant_model"),
            (&self.api_key, "assistant_api_key"),
            (&self.api_key_file, "assistant_api_key_file"),
            (&self.context_window_tokens, "assistant_context_window"),
            (&self.timeout_ms, "assistant_timeout"),
            (&self.max_output_tokens, "assistant_max_output"),
            (&self.max_tool_rounds, "assistant_max_rounds"),
            (&self.max_validation_retries, "assistant_max_retries"),
        ]
    }

    /// Relabels every field, and the prompt's placeholder, in `locale`.
    fn follow_locale(&self, locale: i18n::Locale, cx: &mut Context<RootView>) {
        for (field, key) in self.labelled() {
            field.update(cx, |field, cx| field.set_label(i18n::text(locale, key), cx));
        }
        self.prompt.update(cx, |field, cx| {
            field.set_placeholder(i18n::text(locale, "ask_placeholder"), cx)
        });
    }

    /// The secret field always starts empty: the stored key is never shown,
    /// and an empty field keeps it.
    fn load_profile(
        &self,
        profile: Option<&AssistantConnectionProfile>,
        cx: &mut Context<RootView>,
    ) {
        let profile = profile.cloned().unwrap_or_else(empty_connection_profile);
        for (input, value) in [
            (&self.connection_id, profile.id),
            (&self.connection_name, profile.name),
            (&self.base_url, profile.base_url),
            (&self.model, profile.model),
            (&self.api_key, String::new()),
            (&self.api_key_file, profile.api_key_file.unwrap_or_default()),
            (
                &self.context_window_tokens,
                profile
                    .context_window_tokens
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
            ),
            (&self.timeout_ms, profile.timeout_ms.to_string()),
            (
                &self.max_output_tokens,
                profile.max_output_tokens.to_string(),
            ),
        ] {
            input.update(cx, |input, cx| input.set_value(value, cx));
        }
    }

    fn load_limits(&self, settings: &AssistantSettings, cx: &mut Context<RootView>) {
        let rounds = settings.max_tool_rounds.to_string();
        let retries = settings.max_validation_retries.to_string();
        self.max_tool_rounds
            .update(cx, |input, cx| input.set_value(rounds, cx));
        self.max_validation_retries
            .update(cx, |input, cx| input.set_value(retries, cx));
    }

    fn value(field: &Entity<TextField>, cx: &App) -> String {
        field.read(cx).value(cx)
    }
}

/// The raw form at submission. Keep its text so later unfinished edits
/// can be compared without parsing or replacing them.
struct AssistantForm {
    connection_id: String,
    connection_name: String,
    base_url: String,
    model: String,
    api_key: String,
    api_key_file: String,
    context_window_tokens: String,
    timeout_ms: String,
    max_output_tokens: String,
    max_tool_rounds: String,
    max_validation_retries: String,
    protocol: LlmProtocol,
    auth_mode: AssistantAuthMode,
    enabled: bool,
    write_mode: AgentWriteMode,
    active_connection_id: Option<String>,
}

#[derive(Clone)]
pub struct DesktopServices {
    pub workspace: WorkspaceService,
    pub settings: SettingsService,
    pub model_manager: Arc<dyn ModelService>,
    /// Publishes the installation facts the pages show.
    pub installation: InstallationService,
    pub asr_model: ModelRef,
    pub diarization_model: ModelRef,
    pub assistant: Arc<dyn AssistantUiService>,
    /// The Speaker Library at the configured `speaker_library_path`. The
    /// desktop replaces it with [`speaker_library_at`] the new path whenever
    /// that setting changes.
    pub speaker_library: SpeakerLibraryService,
}

/// The Speaker Library stored at `path`. Opening it reads nothing; every
/// call of the service reads or replaces the file.
pub fn speaker_library_at(path: impl Into<std::path::PathBuf>) -> SpeakerLibraryService {
    SpeakerLibraryService::new(Arc::new(JsonSpeakerProfileStore::new(path)))
}

/// The catalog and download operations the desktop calls. `ModelManager`
/// implements it; tests substitute fakes that block or fail. Everything but
/// `catalog` and `list` may touch the file system, so the controller calls
/// those off the main thread.
pub trait ModelService: Send + Sync {
    fn catalog(&self) -> &ModelCatalog;
    fn list(&self) -> Result<Vec<models::manager::DownloadProgress>, ModelManagerError>;
    fn model_ref(&self, model_id: &str, backend_id: &str) -> Result<ModelRef, ModelManagerError>;
    fn start(&self, model_id: &str) -> Result<DownloadTask, ModelManagerError>;
    fn pause(&self, id: DownloadId) -> Result<(), ModelManagerError>;
    fn resume(&self, id: DownloadId) -> Result<(), ModelManagerError>;
    fn cancel(&self, id: DownloadId) -> Result<(), ModelManagerError>;
    fn reprobe(&self, model_id: &str) -> Result<ReprobeTask, ModelManagerError>;
}

impl ModelService for ModelManager {
    fn catalog(&self) -> &ModelCatalog {
        ModelManager::catalog(self)
    }

    fn list(&self) -> Result<Vec<models::manager::DownloadProgress>, ModelManagerError> {
        ModelManager::list(self)
    }

    fn model_ref(&self, model_id: &str, backend_id: &str) -> Result<ModelRef, ModelManagerError> {
        ModelManager::model_ref(self, model_id, backend_id)
    }

    fn start(&self, model_id: &str) -> Result<DownloadTask, ModelManagerError> {
        ModelManager::start(self, model_id)
    }

    fn pause(&self, id: DownloadId) -> Result<(), ModelManagerError> {
        ModelManager::pause(self, id)
    }

    fn resume(&self, id: DownloadId) -> Result<(), ModelManagerError> {
        ModelManager::resume(self, id)
    }

    fn cancel(&self, id: DownloadId) -> Result<(), ModelManagerError> {
        ModelManager::cancel(self, id)
    }

    fn reprobe(&self, model_id: &str) -> Result<ReprobeTask, ModelManagerError> {
        ModelManager::reprobe(self, model_id)
    }
}

/// A re-probe the Models page shows as running: while `reprobe` runs, then
/// until the installation service publishes the state that carries the
/// record it wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReprobeWait {
    Running,
    /// The record probed at `probed_at_ms` is written.
    Written {
        probed_at_ms: u64,
    },
}

impl ReprobeWait {
    /// Whether `entry`, the model's entry in a published state, ends the
    /// wait: it carries the written record or a later one, or its record
    /// cannot be read. A model no longer in the catalog ends it too.
    pub(crate) fn settled_by(&self, entry: Option<&ModelAvailability>) -> bool {
        let Self::Written { probed_at_ms } = *self else {
            return false;
        };
        entry.is_none_or(|entry| {
            entry.record_error.is_some()
                || entry
                    .probe
                    .as_ref()
                    .is_some_and(|probe| probe.probed_at_ms >= probed_at_ms)
        })
    }
}

fn empty_connection_profile() -> AssistantConnectionProfile {
    AssistantConnectionProfile {
        id: String::new(),
        name: String::new(),
        protocol: LlmProtocol::OpenAiChatCompletions,
        base_url: String::new(),
        model: String::new(),
        auth_mode: AssistantAuthMode::Bearer,
        api_key: None,
        api_key_file: None,
        context_window_tokens: None,
        timeout_ms: 120_000,
        max_output_tokens: 4_096,
    }
}

pub(crate) struct RootView {
    pub(crate) inventory: BackendInventory,
    pub(crate) services: DesktopServices,
    pub(crate) settings_draft: AppSettings,
    /// The persisted settings as the UI last loaded them. Every settings write
    /// is a compare-and-swap against them.
    settings_base: AppSettings,
    /// The Assistant section the settings form was loaded from; saving sends
    /// only the changes relative to it, so a retry keeps external changes.
    assistant_origin: AssistantSettings,
    /// The last queued settings write; writes run one at a time.
    settings_write: Option<Task<()>>,
    /// Keep editing available while a save awaits its acknowledgment,
    /// but require that result before accepting another explicit save.
    pub(crate) assistant_save_pending: bool,
    pub(crate) projects: Vec<DocumentState>,
    pub(crate) ui: UiState,
    pub(crate) assistant: AssistantController,
    pub(crate) assistant_inputs: AssistantInputs,
    pub(crate) assistant_protocol: LlmProtocol,
    pub(crate) assistant_auth_mode: AssistantAuthMode,
    pub(crate) cancel_contexts: Option<(JobContext, JobContext)>,
    /// Cancel was clicked while processing was still starting; the job is
    /// cancelled as soon as the start returns.
    processing_cancel_requested: bool,
    pub(crate) settings_selectors: Option<SettingsSelectors>,
    /// A requested section is revealed after its settings viewport is laid out.
    pending_settings_section: Option<usize>,
    /// The last failed settings write, shown on the settings page.
    pub(crate) settings_error: Option<AssistantUiError>,
    pending_expiry: Option<Task<()>>,
    /// Installation facts for rendering, as `services.installation` last
    /// published them; see `refresh_installation`.
    pub(crate) installation: InstallationState,
    /// The latest project-list reload; an older one that finishes later is
    /// dropped.
    projects_generation: u64,
    /// The repository subscription lifecycle, including a rolled-back
    /// switch; ordinary list reloads do not invalidate document reads.
    documents_repository_generation: u64,
    /// A different root discards drafts and their pending saves. A switch
    /// rolled back to the same root still accepts its completed save.
    pub(crate) documents_root_generation: u64,
    /// Whether the initial project-list request has completed.
    pub(crate) projects_loaded: bool,
    /// The latest list read failure; the last successful snapshot stays shown.
    pub(crate) projects_error: Option<String>,
    /// The locale in which the last list failure's notice was opened.
    projects_error_locale: i18n::Locale,
    /// The open Speaker Library migration question, if any.
    pub(crate) speaker_library_prompt: Option<SpeakerLibraryPrompt>,
    pub(crate) scroll: ScrollAreas,
    /// Holds focus while a dialog is open, so Enter and Esc reach it.
    pub(crate) dialog_focus: FocusHandle,
    /// What held focus before the open dialog took it; closing the dialog
    /// returns focus there.
    dialog_return: Option<FocusHandle>,
    /// The window root's, which holds focus whenever no control does.
    pub(crate) window_focus: FocusHandle,
    /// The sidebar entries, in `pages::chrome::ENTRIES` order.
    pub(crate) nav_focus: [FocusHandle; 4],
    pub(crate) sidebar_sizing: Entity<controls::sidebar::SidebarSizing>,
    /// The project tree, one tab stop whose arrow keys move the selection.
    pub(crate) tree_focus: FocusHandle,
    /// Scrolls the tree's selection into view on the next render, after a
    /// keyboard move has put the selected row somewhere else.
    pub(crate) reveal_tree_selection: bool,
    /// The Detail panel, and the button that opens it, which focus returns
    /// to when Detail closes from inside.
    pub(crate) detail_focus: FocusHandle,
    pub(crate) detail_trigger: FocusHandle,
    /// The awaiting review's card on the Assistant page.
    pub(crate) review_focus: FocusHandle,
    /// The Assistant page's scene tabs, in `pages::assistant::SCENES` order.
    pub(crate) assistant_scene_focus: [FocusHandle; 5],
    /// Moves focus to the awaiting review on the next render.
    review_focus_request: Option<ReviewFocus>,
    /// Scanner issues of the documents directory, as the last project-list
    /// reload read them.
    pub(crate) document_issues: Vec<DocumentIssue>,
    /// The editor of the selected document's semantic fields.
    pub(crate) editor: DocumentEditor,
    /// The last document write refused for a newer revision, until the user
    /// reloads.
    pub(crate) document_conflict: Option<DocumentConflict>,
    /// The Speaker Library section's profiles and last outcome.
    pub(crate) speaker_profiles: SpeakerProfiles,
    /// The path `services.speaker_library` was opened on.
    speaker_library_path: String,
    /// The rename and merge controls of the speakers of the document shown
    /// in the Original view, and the revision they were read at.
    pub(crate) speaker_rows: Option<NameRows<(ProjectId, u64, i18n::Locale)>>,
    /// The rename and merge controls of the Speaker Library profiles.
    pub(crate) profile_rows: Option<NameRows<(u64, i18n::Locale)>>,
    /// Focus destination when the last profile is deleted.
    pub(crate) speaker_library_focus: FocusHandle,
    /// A confirmed deletion returns focus after the refreshed rows exist.
    profile_removal_focus: Option<ProfileRemovalFocus>,
    /// The document whose speaker correction is in flight.
    pub(crate) speaker_command: Option<ProjectId>,
    /// The speaker merge or profile change waiting for confirmation.
    pub(crate) speaker_confirmation: Option<SpeakerConfirmation>,
    /// The selected document's source audio, as the last check found it.
    pub(crate) audio: AudioCheck,
    /// The Workspace section's documents-root switch.
    pub(crate) documents_root_switch: DocumentsRootSwitch,
    /// The models the Models page shows re-probing.
    pub(crate) model_reprobes: BTreeMap<String, ReprobeWait>,
}

/// Why the next review awaits, which decides whether it takes the focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReviewFocus {
    /// The user accepted or rejected the one before: continue the keyboard
    /// queue only while focus remains in its review region.
    Decision,
    /// The one before expired: the next review takes the focus only from
    /// the review that had it, never from a field the user is typing in.
    Expiry,
}

/// A documents-root switch in flight, and the outcome of the last one: its
/// message and whether it failed.
#[derive(Debug, Clone, Default)]
pub(crate) struct DocumentsRootSwitch {
    pub(crate) switching: bool,
    pub(crate) status: Option<(String, bool)>,
}

/// Where a tree move came from, which decides what follows it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TreeMoveFocus {
    /// A drop: the tree stays as it is.
    Pointer,
    /// An Option-arrow move: `open`, the new parent, opens so the moved row
    /// shows, and the tree reveals it.
    Keyboard { open: Option<ProjectId> },
}

/// A document write refused because the document changed after the UI read
/// it; nothing was written. Reloading reads the document again and discards
/// its draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocumentConflict {
    pub(crate) document_id: ProjectId,
    pub(crate) title: String,
    pub(crate) expected: u64,
    pub(crate) actual: u64,
}

/// The scroll positions of the lists that fade at their edges; each fade
/// reads its list's handle at paint time.
#[derive(Default)]
pub(crate) struct ScrollAreas {
    pub(crate) assistant_results: ScrollHandle,
    pub(crate) project_tree: ScrollHandle,
    pub(crate) meeting_content: ScrollHandle,
    pub(crate) document_issues: ScrollHandle,
    pub(crate) detail: ScrollHandle,
    pub(crate) models: ScrollHandle,
    pub(crate) settings: ScrollHandle,
}

/// Returns focus after a profile deletion unless the user has moved it.
struct ProfileRemovalFocus {
    removed_id: String,
    next_id: Option<String>,
    previous: Option<FocusHandle>,
}

/// Asked when the Speaker Library moves to a path without a library while
/// the current path has one: migrate it, or only switch to the new path.
pub(crate) struct SpeakerLibraryPrompt {
    pub(crate) source: std::path::PathBuf,
    pub(crate) destination: std::path::PathBuf,
}

#[derive(Clone, Copy)]
pub(crate) enum DownloadAction {
    Pause,
    Resume,
    Cancel,
}

impl RootView {
    pub(crate) fn new(
        inventory: BackendInventory,
        services: DesktopServices,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let locale = services
            .settings
            .snapshot()
            .map(|settings| i18n::Locale::from_code(&settings.ui_locale))
            .unwrap_or(i18n::Locale::ZhCn);
        let settings_draft = services.settings.snapshot().unwrap_or_default();
        let active_profile = settings_draft.assistant.active_connection().cloned();
        let assistant_inputs = AssistantInputs::new(locale, &settings_draft.assistant, window, cx);
        cx.subscribe_in(
            &assistant_inputs.prompt,
            window,
            |this, _, event, _, cx| match event {
                TextFieldEvent::Submit => this.submit_assistant_prompt(cx),
                TextFieldEvent::Change => {}
            },
        )
        .detach();
        // Feedback belongs to the exact connection currently held by the form.
        for field in [
            &assistant_inputs.connection_id,
            &assistant_inputs.connection_name,
            &assistant_inputs.base_url,
            &assistant_inputs.model,
            &assistant_inputs.api_key,
            &assistant_inputs.api_key_file,
            &assistant_inputs.context_window_tokens,
            &assistant_inputs.timeout_ms,
            &assistant_inputs.max_output_tokens,
        ] {
            cx.subscribe(field, |this, _, event, cx| {
                if matches!(event, TextFieldEvent::Change) {
                    this.assistant.forget_probe();
                    cx.notify();
                }
            })
            .detach();
        }
        let editor = DocumentEditor::new(locale, window, cx);
        cx.subscribe_in(&editor.status, window, |this, _, event, _, cx| {
            let DropdownEvent::Confirm(value) = event;
            this.editor.set_status(value.clone());
            cx.notify();
        })
        .detach();
        let mut ui = UiState::new(locale);
        ui.expanded_model_families.extend(
            services
                .model_manager
                .catalog()
                .models
                .iter()
                .map(|model| model.model_family.clone()),
        );
        // `main` opens the service on the configured path.
        let speaker_library_path = settings_draft.speaker_library_path.clone();
        let mut this = Self {
            inventory,
            services,
            settings_base: settings_draft.clone(),
            assistant_origin: settings_draft.assistant.clone(),
            settings_write: None,
            assistant_save_pending: false,
            settings_draft,
            projects: Vec::new(),
            ui,
            assistant: AssistantController::default(),
            assistant_inputs,
            assistant_protocol: active_profile
                .as_ref()
                .map(|profile| profile.protocol)
                .unwrap_or(LlmProtocol::OpenAiChatCompletions),
            assistant_auth_mode: active_profile
                .as_ref()
                .map(|profile| profile.auth_mode)
                .unwrap_or(AssistantAuthMode::Bearer),
            cancel_contexts: None,
            processing_cancel_requested: false,
            settings_selectors: None,
            pending_settings_section: None,
            settings_error: None,
            pending_expiry: None,
            installation: InstallationState::default(),
            projects_generation: 0,
            documents_repository_generation: 0,
            documents_root_generation: 0,
            projects_loaded: false,
            projects_error: None,
            projects_error_locale: locale,
            speaker_library_prompt: None,
            scroll: ScrollAreas::default(),
            dialog_focus: cx.focus_handle(),
            dialog_return: None,
            window_focus: cx.focus_handle(),
            nav_focus: std::array::from_fn(|_| cx.focus_handle()),
            sidebar_sizing: cx.new(|_| controls::sidebar::SidebarSizing::default()),
            tree_focus: cx.focus_handle(),
            reveal_tree_selection: false,
            detail_focus: cx.focus_handle(),
            detail_trigger: cx.focus_handle(),
            review_focus: cx.focus_handle(),
            assistant_scene_focus: std::array::from_fn(|_| cx.focus_handle()),
            review_focus_request: None,
            document_issues: Vec::new(),
            editor,
            document_conflict: None,
            speaker_profiles: SpeakerProfiles::default(),
            speaker_library_path,
            speaker_rows: None,
            profile_rows: None,
            speaker_library_focus: cx.focus_handle(),
            profile_removal_focus: None,
            speaker_command: None,
            speaker_confirmation: None,
            audio: AudioCheck::default(),
            documents_root_switch: DocumentsRootSwitch::default(),
            model_reprobes: Default::default(),
        };
        // Every update that sets a notice opens its toast.
        cx.observe_self(Self::take_up_notice).detach();
        // The tree and the Speaker Library stay empty until they load on
        // workers.
        this.reload_projects(cx);
        this.reload_speaker_profiles(cx);
        this.follow_installation(cx);
        this.follow_assistant_jobs(cx);
        this.refresh_installation();
        this
    }

    /// Projects the Assistant's jobs on the shared runtime into the page.
    fn follow_assistant_jobs(&mut self, cx: &mut Context<Self>) {
        let mut events = self.services.assistant.job_events();
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                let applied = this.update(cx, |this, cx| {
                    if this.assistant.apply_job_event(event) {
                        cx.notify();
                    }
                });
                if applied.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Applies every installation state the service publishes, including
    /// the ones it probes after downloads end.
    fn follow_installation(&mut self, cx: &mut Context<Self>) {
        let mut states = self.services.installation.subscribe();
        cx.spawn(async move |this, cx| {
            while let Some(installation) = states.next().await {
                let applied = this.update(cx, |this, cx| {
                    if installation != this.installation {
                        this.installation = installation;
                        this.settle_reprobes();
                        // Refresh model availability without replacing a menu
                        // the user is currently browsing.
                        let asr = this.settings_draft.asr_backend_id.clone();
                        let diarization = this.settings_draft.diarization_backend_id.clone();
                        this.refresh_model_selector("asr", asr.as_deref(), cx);
                        this.refresh_model_selector("diarization", diarization.as_deref(), cx);
                        this.refresh_stage_selectors(cx);
                        cx.notify();
                    }
                });
                if applied.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Ends the re-probe waits the current installation state settles.
    fn settle_reprobes(&mut self) {
        let catalog = &self.installation.catalog;
        self.model_reprobes.retain(|model_id, wait| {
            !wait.settled_by(catalog.iter().find(|entry| entry.model.id == *model_id))
        });
    }

    /// Asks the installation service to probe the current query on its
    /// worker. Call it whenever the facts may have changed; the published
    /// state arrives through `follow_installation`.
    pub(crate) fn refresh_installation(&self) {
        self.services
            .installation
            .refresh(self.installation_query());
    }

    /// Re-reads the project list and the scanner issues off the main thread.
    pub(crate) fn reload_projects(&mut self, cx: &mut Context<Self>) {
        if self.documents_root_switch.switching {
            return;
        }
        let generation = self.begin_projects_reload();
        let workspace = self.services.workspace.clone();
        cx.spawn(async move |this, cx| {
            let snapshot = runtime::spawn_blocking(move || workspace.document_snapshot()).await;
            this.update(cx, |this, cx| {
                if generation != this.projects_generation {
                    cx.notify();
                    return;
                }
                this.apply_projects(generation, snapshot, cx);
            })
            .ok();
        })
        .detach();
    }

    fn begin_projects_reload(&mut self) -> u64 {
        self.projects_generation += 1;
        self.projects_generation
    }

    /// Shows a reloaded project list and its scanner issues unless a newer
    /// reload has started; a successful list initializes an absent selection.
    fn apply_projects(
        &mut self,
        generation: u64,
        snapshot: Result<DocumentSnapshot, String>,
        cx: &mut Context<Self>,
    ) {
        if generation == self.projects_generation {
            match snapshot {
                Ok(snapshot) => {
                    self.projects = snapshot.documents;
                    self.document_issues = snapshot.issues;
                    if let Some(error) = self.projects_error.take() {
                        let old_notice = i18n::message(
                            self.projects_error_locale,
                            "documents_load_failed",
                            &error,
                        );
                        if self.ui.notice == old_notice {
                            self.ui.notice.clear();
                        }
                        if let Some(generation) = self
                            .ui
                            .toast()
                            .filter(|toast| toast.notice == old_notice)
                            .map(|toast| toast.generation)
                        {
                            self.dismiss_toast(generation, cx);
                        }
                    }
                    if !self.projects_loaded || self.ui.selected_project.is_none() {
                        self.ui.open_loaded_projects(&self.projects);
                    }
                    self.check_selected_audio(cx);
                }
                Err(error) => {
                    self.projects_error_locale = self.ui.locale;
                    self.ui.notice = i18n::message(self.ui.locale, "documents_load_failed", &error);
                    self.projects_error = Some(error);
                }
            }
            self.projects_loaded = true;
            cx.notify();
        }
    }

    /// Checks the selected document's source audio on a worker (it hashes
    /// the file); the tree row, the Meetings header and Detail show what it
    /// finds.
    fn check_selected_audio(&mut self, cx: &mut Context<Self>) {
        if self.documents_root_switch.switching {
            return;
        }
        let Some(document_id) = self
            .ui
            .selected_project
            .clone()
            .filter(|id| self.projects.iter().any(|project| project.id() == id))
        else {
            return;
        };
        let generation = self.audio.begin(document_id.clone());
        let workspace = self.services.workspace.clone();
        cx.spawn(async move |this, cx| {
            let result =
                runtime::spawn_blocking(move || workspace.audio_status(&document_id)).await;
            this.update(cx, |this, cx| {
                this.audio.apply(generation, result);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The persisted Assistant settings; the settings form's unsaved draft
    /// never drives the Assistant page or its runs.
    pub(crate) fn saved_assistant(&self) -> &AssistantSettings {
        &self.settings_base.assistant
    }

    /// Changes only the in-memory connection projection for visual fixtures.
    /// The Settings service and its persisted configuration stay untouched.
    pub(crate) fn set_fixture_assistant_connection(&mut self, id: Option<String>) {
        self.settings_base.assistant.active_connection_id = id;
    }

    fn installation_query(&self) -> InstallationQuery {
        InstallationQuery {
            asr_model: self.services.asr_model.clone(),
            diarization_model: self.services.diarization_model.clone(),
            stage_root: self
                .settings_draft
                .diarization_model
                .as_ref()
                .map(|model| model.artifact_path.clone()),
            stage_files: self
                .inventory
                .diarization
                .iter()
                .flat_map(|descriptor| &descriptor.diarization_stage_options)
                .flat_map(|option| option.artifact_files.iter().cloned())
                .collect(),
        }
    }

    pub(crate) fn ensure_settings_selectors(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_selectors.is_some() {
            return;
        }

        let settings = self.settings_draft.clone();
        let language_items = vec!["简体中文".into(), "English".into()];
        let asr_backend_items = self
            .inventory
            .asr
            .iter()
            .map(|descriptor| descriptor.id.clone())
            .collect::<Vec<_>>();
        let diarization_backend_items = self
            .inventory
            .diarization
            .iter()
            .map(|descriptor| descriptor.id.clone())
            .collect::<Vec<_>>();
        let asr_model_items = self.model_options("asr", settings.asr_backend_id.as_deref());
        let diarization_model_items =
            self.model_options("diarization", settings.diarization_backend_id.as_deref());
        let segmentation_items = self.stage_model_options(
            DiarizationStageKind::Segmentation,
            settings.diarization_backend_id.as_deref(),
            settings.diarization_model.as_ref(),
        );
        let embedding_items = self.stage_model_options(
            DiarizationStageKind::Embedding,
            settings.diarization_backend_id.as_deref(),
            settings.diarization_model.as_ref(),
        );
        let clustering_items = self.clustering_options(settings.diarization_backend_id.as_deref());
        let locale = self.ui.locale;
        let transcription_language_items = transcription_language_items(locale);
        let expected_speakers_items = expected_speaker_items(locale);
        let compute_device_items = compute_device_items(locale);
        let expected_speakers_value = settings.expected_speakers.map(|value| value.to_string());
        let theme_items = theme_preference_items(locale);
        let material_items = material_preference_items(locale);

        let placeholder = i18n::text(locale, "select_placeholder");
        let mut dropdown =
            |label_key: &'static str, options: Vec<SelectOption>, selected: Option<&str>| {
                let label = i18n::text(locale, label_key);
                cx.new(|cx| Dropdown::new(label, placeholder, options, selected, cx))
            };
        let theme = dropdown(
            "theme",
            text_options(theme_items),
            Some(i18n::text(
                locale,
                theme_preference_key(settings.appearance.theme),
            )),
        );
        let material = dropdown(
            "material",
            text_options(material_items),
            Some(i18n::text(
                locale,
                material_preference_key(settings.appearance.material),
            )),
        );
        let language = dropdown(
            "language",
            text_options(language_items),
            Some(if locale == i18n::Locale::EnUs {
                "English"
            } else {
                "简体中文"
            }),
        );
        let asr_backend = dropdown(
            "asr_backend",
            text_options(asr_backend_items),
            settings.asr_backend_id.as_deref(),
        );
        let asr_model = dropdown(
            "asr_model",
            asr_model_items,
            settings
                .asr_model
                .as_ref()
                .map(|model| model.model_id.as_str()),
        );
        let transcription_language = dropdown(
            "transcription_language",
            text_options(transcription_language_items),
            Some(transcription_language_label(
                locale,
                settings.language.as_deref(),
            )),
        );
        let diarization_backend = dropdown(
            "diarization_backend",
            text_options(diarization_backend_items),
            settings.diarization_backend_id.as_deref(),
        );
        let diarization_model = dropdown(
            "diarization_model",
            diarization_model_items,
            settings
                .diarization_model
                .as_ref()
                .map(|model| model.model_id.as_str()),
        );
        let segmentation_selected = stage_selection(
            &segmentation_items,
            settings
                .diarization_stages
                .segmentation_model
                .as_ref()
                .map(|model| model.model_id.as_str()),
        )
        .map(str::to_owned);
        let diarization_segmentation_model = dropdown(
            "diarization_segmentation_model",
            segmentation_items,
            segmentation_selected.as_deref(),
        );
        let embedding_selected = stage_selection(
            &embedding_items,
            settings
                .diarization_stages
                .embedding_model
                .as_ref()
                .map(|model| model.model_id.as_str()),
        )
        .map(str::to_owned);
        let diarization_embedding_model = dropdown(
            "diarization_embedding_model",
            embedding_items,
            embedding_selected.as_deref(),
        );
        let diarization_clustering = dropdown(
            "diarization_clustering",
            text_options(clustering_items.clone()),
            settings
                .diarization_stages
                .clustering_algorithm
                .as_ref()
                .map(|algorithm| algorithm.id.as_str())
                .or_else(|| clustering_items.first().map(String::as_str)),
        );
        let expected_speakers = dropdown(
            "expected_speakers",
            text_options(expected_speakers_items),
            Some(
                expected_speakers_value
                    .as_deref()
                    .unwrap_or(auto_label(locale)),
            ),
        );
        let compute_device = dropdown(
            "compute_device",
            text_options(compute_device_items),
            Some(i18n::device(locale, settings.compute_device)),
        );

        cx.subscribe_in(&theme, window, Self::on_theme_selected)
            .detach();
        cx.subscribe_in(&material, window, Self::on_material_selected)
            .detach();
        cx.subscribe_in(&language, window, Self::on_language_selected)
            .detach();
        cx.subscribe_in(&asr_backend, window, Self::on_asr_backend_selected)
            .detach();
        cx.subscribe_in(&asr_model, window, Self::on_asr_model_selected)
            .detach();
        cx.subscribe_in(
            &transcription_language,
            window,
            Self::on_transcription_language_selected,
        )
        .detach();
        cx.subscribe_in(
            &diarization_backend,
            window,
            Self::on_diarization_backend_selected,
        )
        .detach();
        cx.subscribe_in(
            &diarization_model,
            window,
            Self::on_diarization_model_selected,
        )
        .detach();
        cx.subscribe_in(
            &diarization_segmentation_model,
            window,
            Self::on_diarization_segmentation_model_selected,
        )
        .detach();
        cx.subscribe_in(
            &diarization_embedding_model,
            window,
            Self::on_diarization_embedding_model_selected,
        )
        .detach();
        cx.subscribe_in(
            &diarization_clustering,
            window,
            Self::on_diarization_clustering_selected,
        )
        .detach();
        cx.subscribe_in(
            &expected_speakers,
            window,
            Self::on_expected_speakers_selected,
        )
        .detach();
        cx.subscribe_in(&compute_device, window, Self::on_compute_device_selected)
            .detach();

        self.settings_selectors = Some(SettingsSelectors {
            theme,
            material,
            language,
            asr_backend,
            asr_model,
            transcription_language,
            diarization_backend,
            diarization_model,
            diarization_segmentation_model,
            diarization_embedding_model,
            diarization_clustering,
            expected_speakers,
            compute_device,
        });
    }

    fn model_options(&self, task: &str, backend_id: Option<&str>) -> Vec<SelectOption> {
        self.installation
            .models(task, backend_id)
            .map(|entry| ModelOption::from_availability(entry, self.ui.locale).option())
            .collect()
    }

    fn stage_model_options(
        &self,
        stage: DiarizationStageKind,
        backend_id: Option<&str>,
        model: Option<&ModelRef>,
    ) -> Vec<SelectOption> {
        let Some(descriptor) = backend_id.and_then(|id| {
            self.inventory
                .diarization
                .iter()
                .find(|descriptor| descriptor.id == id)
        }) else {
            return Vec::new();
        };
        if !descriptor.configurable_diarization_stages.contains(&stage) {
            return Vec::new();
        }
        descriptor
            .diarization_stage_options
            .iter()
            .filter(|option| option.stage == stage)
            .map(|option| StageModelOption {
                id: option.id.clone(),
                installed: model
                    .is_some_and(|model| self.installation.stage_installed(model, option)),
                locale: self.ui.locale,
            })
            .map(|option| option.option())
            .collect()
    }

    fn clustering_options(&self, backend_id: Option<&str>) -> Vec<String> {
        let Some(descriptor) = backend_id.and_then(|id| {
            self.inventory
                .diarization
                .iter()
                .find(|descriptor| descriptor.id == id)
        }) else {
            return Vec::new();
        };
        descriptor
            .diarization_stage_options
            .iter()
            .filter(|option| option.stage == DiarizationStageKind::Clustering)
            .map(|option| option.id.clone())
            .collect()
    }

    fn selected_diarization_descriptor(&self) -> Option<&contracts::BackendDescriptor> {
        self.settings_draft
            .diarization_backend_id
            .as_deref()
            .and_then(|id| self.inventory.diarization.iter().find(|item| item.id == id))
    }

    pub(crate) fn show_diarization_stage_options(&self) -> bool {
        self.selected_diarization_descriptor()
            .is_some_and(|descriptor| !descriptor.configurable_diarization_stages.is_empty())
    }

    fn stage_model_ref(&self, stage: DiarizationStageKind, id: &str) -> Option<ModelRef> {
        let descriptor = self.selected_diarization_descriptor()?;
        let option = descriptor
            .diarization_stage_options
            .iter()
            .find(|option| option.stage == stage && option.id == id)?;
        let parent = self.settings_draft.diarization_model.as_ref()?;
        let filename = option.artifact_files.first()?.clone();
        Some(ModelRef {
            backend_id: parent.backend_id.clone(),
            model_id: option.model_id.clone().unwrap_or_else(|| option.id.clone()),
            revision: option.revision.clone().or_else(|| parent.revision.clone()),
            artifact_path: Path::new(&parent.artifact_path)
                .join(filename)
                .display()
                .to_string(),
            artifacts: parent
                .artifacts
                .iter()
                .filter(|artifact| option.artifact_files.contains(&artifact.filename))
                .cloned()
                .collect(),
        })
    }

    /// Shows `change` at once and persists only that change.
    fn persist_field(
        &mut self,
        change: impl Fn(&mut AppSettings) + Send + Sync + 'static,
        cx: &mut Context<Self>,
    ) {
        let saved_notice = i18n::text(self.ui.locale, "settings_saved").into();
        self.persist_field_with_notice(change, saved_notice, cx);
    }

    fn persist_field_with_notice(
        &mut self,
        change: impl Fn(&mut AppSettings) + Send + Sync + 'static,
        saved_notice: String,
        cx: &mut Context<Self>,
    ) {
        change(&mut self.settings_draft);
        self.sync_appearance(cx);
        self.sync_speaker_library(cx);
        self.queue_settings_write(
            SettingsWrite::Field {
                change: Arc::new(change),
                saved_notice,
            },
            cx,
        );
        self.refresh_installation();
        cx.notify();
    }

    /// Settings writes run one at a time off the main thread. Each compares
    /// against the settings loaded when it starts, so the UI's own writes do
    /// not conflict with each other while external changes still do.
    fn queue_settings_write(&mut self, write: SettingsWrite, cx: &mut Context<Self>) {
        let previous = self.settings_write.take();
        self.settings_write = Some(cx.spawn(async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let Ok((settings, assistant, workspace, expected)) = this.update(cx, |this, _| {
                (
                    this.services.settings.clone(),
                    this.services.assistant.clone(),
                    this.services.workspace.clone(),
                    this.settings_base.clone(),
                )
            }) else {
                return;
            };
            match write {
                SettingsWrite::Field {
                    change,
                    saved_notice,
                } => {
                    let apply = change.clone();
                    let result = runtime::spawn_blocking(move || {
                        let mut next = expected.clone();
                        apply(&mut next);
                        let result = settings
                            .update_if_equal(&expected, next)
                            .map_err(|error| AssistantUiError::from_settings(&error));
                        reload_after_conflict(&settings, result)
                    })
                    .await;
                    this.update(cx, |this, cx| {
                        this.finish_field_write(change.as_ref(), saved_notice, result, cx)
                    })
                    .ok();
                }
                SettingsWrite::Assistant { save, submitted } => {
                    let result = runtime::spawn_blocking(move || {
                        let result = assistant
                            .save_assistant_settings(SettingsService::revision_of(&expected), save);
                        reload_after_conflict(&settings, result)
                    })
                    .await;
                    this.update(cx, |this, cx| {
                        this.finish_assistant_save(&submitted, result, cx);
                    })
                    .ok();
                }
                SettingsWrite::DocumentsRoot { root } => {
                    let result = runtime::spawn_blocking(move || {
                        switch_documents_root(&workspace, &settings, &expected, root)
                    })
                    .await;
                    this.update(cx, |this, cx| this.finish_documents_root_switch(result, cx))
                        .ok();
                }
            }
        }));
    }

    /// Shows how a documents-root switch ended. A switch saved in the
    /// settings shows the new root's tree; a refused one changes nothing.
    /// A failed save adopts reloaded settings, but if restoring the old root
    /// also fails, the path and tree still follow the active workspace.
    fn finish_documents_root_switch(
        &mut self,
        result: Result<DocumentsRootSwitched, DocumentsRootSwitchFailure>,
        cx: &mut Context<Self>,
    ) {
        self.documents_root_switch.switching = false;
        let locale = self.ui.locale;
        let (message, failed, root_changed) = match result {
            Ok(DocumentsRootSwitched { saved, switched }) => {
                self.settings_draft.documents_root = saved.documents_root.clone();
                self.settings_base = saved;
                self.settings_error = None;
                (
                    i18n::message(
                        locale,
                        "documents_root_switched",
                        &self.settings_draft.documents_root,
                    ),
                    false,
                    switched.then_some(true),
                )
            }
            Err(DocumentsRootSwitchFailure::Refused(error)) => {
                (i18n::documents_root_error(locale, &error), true, None)
            }
            Err(DocumentsRootSwitchFailure::NotSaved {
                failure,
                switched,
                restore,
                active_root,
            }) => {
                let reason = settings_error_text(locale, &failure.error);
                self.settings_write_failed(failure, cx);
                if let Some(active_root) = active_root {
                    // The saved setting could not be restored. Show the
                    // directory the workspace actually reads and writes.
                    self.settings_draft.documents_root = active_root.display().to_string();
                }
                let root_changed = switched.then_some(restore.is_some());
                let message = match restore {
                    None => i18n::message(locale, "documents_root_not_saved", &reason),
                    Some(error) => i18n::message(
                        locale,
                        "documents_root_restore_failed",
                        &format!("{reason} {}", i18n::documents_root_error(locale, &error)),
                    ),
                };
                (message, true, root_changed)
            }
        };
        match root_changed {
            // Switching there and back also ended the old change feed.
            Some(changed) => self.follow_documents_root(changed, cx),
            // Refusal and choosing the same root still resume the reads
            // invalidated when the switch began.
            None => self.reload_projects(cx),
        }
        let tone = if failed {
            NoticeTone::Error
        } else {
            NoticeTone::Success
        };
        self.ui.set_notice(tone, message.clone());
        self.documents_root_switch.status = Some((message, failed));
        cx.notify();
    }

    /// After a switch, follows the changes of the workspace's documents
    /// root again (every switch ends the feeds of the root it left) and
    /// reloads the tree. When the root changed, everything read from the
    /// old one is dropped first: the selection, drafts, conflict notice,
    /// audio check, document speaker controls and the Assistant's results,
    /// reviews and undo entries.
    fn follow_documents_root(&mut self, root_changed: bool, cx: &mut Context<Self>) {
        self.documents_repository_generation += 1;
        // A successful rollback also replaced the repository. Retain its
        // drafts, but let reads from the restored root start again.
        self.editor.loading.clear();
        if root_changed {
            self.documents_root_generation += 1;
            self.editor.saving = None;
            self.projects.clear();
            self.document_issues.clear();
            self.projects_error = None;
            self.projects_loaded = false;
            self.ui.selected_project = None;
            self.ui.expanded_projects.clear();
            self.ui.content_mode = ContentMode::Summary;
            self.editor.discard_all();
            self.document_conflict = None;
            self.audio.clear();
            self.speaker_rows = None;
            self.speaker_command = None;
            if matches!(
                self.speaker_confirmation,
                Some(SpeakerConfirmation::MergeDocumentSpeaker { .. })
            ) {
                self.speaker_confirmation = None;
            }
            self.assistant.forget_workspace();
            self.pending_expiry = None;
        }
        self.watch_documents(cx);
        self.reload_projects(cx);
    }

    fn finish_field_write(
        &mut self,
        change: &(dyn Fn(&mut AppSettings) + Send + Sync),
        saved_notice: String,
        result: Result<AppSettings, SettingsWriteFailure>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(saved) => {
                self.settings_base = saved;
                // An earlier failed write resets the draft; keep this change.
                change(&mut self.settings_draft);
                self.sync_locale(cx);
                self.sync_appearance(cx);
                self.sync_speaker_library(cx);
                self.settings_error = None;
                self.ui.notice = saved_notice;
            }
            Err(error) => self.settings_write_failed(error, cx),
        }
        cx.notify();
    }

    fn finish_assistant_save(
        &mut self,
        submitted: &AssistantForm,
        result: Result<AppSettings, SettingsWriteFailure>,
        cx: &mut Context<Self>,
    ) {
        self.assistant_save_pending = false;
        match result {
            Ok(saved) => {
                let draft_kept = self.acknowledge_assistant_form(submitted, &saved.assistant, cx);
                self.assistant_origin = saved.assistant.clone();
                self.settings_base = saved;
                self.settings_error = None;
                let notice = if draft_kept {
                    "assistant_settings_saved_draft_kept"
                } else {
                    "assistant_settings_saved"
                };
                self.ui.notice = i18n::text(self.ui.locale, notice).into();
            }
            Err(error) => self.settings_write_failed(error, cx),
        }
        cx.notify();
    }

    /// The persisted settings stay authoritative after a failed write: a
    /// conflict adopts the settings reloaded from the repository, and the
    /// instant-save fields show them again. Keep the active documents path
    /// until an explicit root switch, and keep the unsaved Assistant form
    /// so saving it again is the retry.
    fn settings_write_failed(&mut self, failure: SettingsWriteFailure, cx: &mut Context<Self>) {
        let SettingsWriteFailure { error, reloaded } = failure;
        if let Some(latest) = reloaded {
            self.settings_base = latest;
        }
        self.settings_draft = AppSettings {
            documents_root: self.settings_draft.documents_root.clone(),
            assistant: self.settings_draft.assistant.clone(),
            ..self.settings_base.clone()
        };
        self.settings_selectors = None;
        self.sync_locale(cx);
        self.sync_appearance(cx);
        self.sync_speaker_library(cx);
        self.refresh_installation();
        self.ui.notice = settings_error_text(self.ui.locale, &error);
        self.settings_error = Some(error);
    }

    /// Follows the draft's UI locale; the localized selects are rebuilt on
    /// the next render.
    fn sync_locale(&mut self, cx: &mut Context<Self>) {
        let locale = i18n::Locale::from_code(&self.settings_draft.ui_locale);
        if locale == self.ui.locale {
            return;
        }
        self.ui.locale = locale;
        self.settings_selectors = None;
        self.assistant_inputs.follow_locale(locale, cx);
        self.editor.follow_locale(locale, &self.projects, cx);
    }

    /// Follows the draft's appearance; the theme re-applies it to the window.
    fn sync_appearance(&self, cx: &mut Context<Self>) {
        theme::set_preference(self.settings_draft.appearance, cx);
    }

    /// Adopt the saved baseline, acknowledging only values the user has
    /// not changed since submission. Leave other text, selection and focus
    /// with the current form, including a newly selected connection.
    fn acknowledge_assistant_form(
        &mut self,
        submitted: &AssistantForm,
        saved: &AssistantSettings,
        cx: &mut Context<Self>,
    ) -> bool {
        let current_id = AssistantInputs::value(&self.assistant_inputs.connection_id, cx);
        let same_profile = current_id == submitted.connection_id;
        let current = &self.settings_draft.assistant;
        let mut draft = saved.clone();
        let mut draft_kept = false;
        if current.enabled != submitted.enabled {
            draft.enabled = current.enabled;
            draft_kept |= current.enabled != saved.enabled;
        }
        if current.write_mode != submitted.write_mode {
            draft.write_mode = current.write_mode;
            draft_kept |= current.write_mode != saved.write_mode;
        }
        if !same_profile || current.active_connection_id != submitted.active_connection_id {
            draft.active_connection_id = current.active_connection_id.clone();
            draft_kept |=
                !same_profile || current.active_connection_id != saved.active_connection_id;
        }
        let mut inputs = vec![
            (
                &self.assistant_inputs.max_tool_rounds,
                &submitted.max_tool_rounds,
                saved.max_tool_rounds.to_string(),
            ),
            (
                &self.assistant_inputs.max_validation_retries,
                &submitted.max_validation_retries,
                saved.max_validation_retries.to_string(),
            ),
        ];
        if same_profile {
            let profile = saved
                .active_connection()
                .cloned()
                .unwrap_or_else(empty_connection_profile);
            if self.assistant_protocol == submitted.protocol {
                self.assistant_protocol = profile.protocol;
            } else {
                draft_kept |= self.assistant_protocol != profile.protocol;
            }
            if self.assistant_auth_mode == submitted.auth_mode {
                self.assistant_auth_mode = profile.auth_mode;
            } else {
                draft_kept |= self.assistant_auth_mode != profile.auth_mode;
            }
            inputs.extend([
                (
                    &self.assistant_inputs.connection_id,
                    &submitted.connection_id,
                    profile.id,
                ),
                (
                    &self.assistant_inputs.connection_name,
                    &submitted.connection_name,
                    profile.name,
                ),
                (
                    &self.assistant_inputs.base_url,
                    &submitted.base_url,
                    profile.base_url,
                ),
                (
                    &self.assistant_inputs.model,
                    &submitted.model,
                    profile.model,
                ),
                (
                    &self.assistant_inputs.api_key,
                    &submitted.api_key,
                    String::new(),
                ),
                (
                    &self.assistant_inputs.api_key_file,
                    &submitted.api_key_file,
                    profile.api_key_file.unwrap_or_default(),
                ),
                (
                    &self.assistant_inputs.context_window_tokens,
                    &submitted.context_window_tokens,
                    profile
                        .context_window_tokens
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                ),
                (
                    &self.assistant_inputs.timeout_ms,
                    &submitted.timeout_ms,
                    profile.timeout_ms.to_string(),
                ),
                (
                    &self.assistant_inputs.max_output_tokens,
                    &submitted.max_output_tokens,
                    profile.max_output_tokens.to_string(),
                ),
            ]);
        }
        for (input, submitted, saved) in inputs {
            let value = AssistantInputs::value(input, cx);
            if &value != submitted {
                draft_kept |= value != saved;
            } else if value != saved {
                input.update(cx, |input, cx| input.set_value(saved, cx));
            }
        }
        self.settings_draft.assistant = draft;
        draft_kept
    }

    fn refresh_model_selector(
        &mut self,
        task: &'static str,
        backend_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let options = self.model_options(task, backend_id);
        let selected = match task {
            "asr" => self.settings_draft.asr_model.as_ref(),
            "diarization" => self.settings_draft.diarization_model.as_ref(),
            _ => None,
        }
        .map(|model| model.model_id.clone());
        let selector = match (task, self.settings_selectors.as_ref()) {
            ("asr", Some(selectors)) => selectors.asr_model.clone(),
            ("diarization", Some(selectors)) => selectors.diarization_model.clone(),
            _ => return,
        };
        selector.update(cx, |dropdown, cx| {
            dropdown.set_options(options, selected.as_deref(), cx)
        });
    }

    fn refresh_stage_selectors(&mut self, cx: &mut Context<Self>) {
        let backend_id = self.settings_draft.diarization_backend_id.clone();
        let model = self.settings_draft.diarization_model.clone();
        let segmentation = self.stage_model_options(
            DiarizationStageKind::Segmentation,
            backend_id.as_deref(),
            model.as_ref(),
        );
        let embedding = self.stage_model_options(
            DiarizationStageKind::Embedding,
            backend_id.as_deref(),
            model.as_ref(),
        );
        let clustering = self.clustering_options(backend_id.as_deref());
        let Some(selectors) = self.settings_selectors.as_ref() else {
            return;
        };
        let stages = &self.settings_draft.diarization_stages;
        let segmentation_selected = stage_selection(
            &segmentation,
            stages
                .segmentation_model
                .as_ref()
                .map(|model| model.model_id.as_str()),
        )
        .map(str::to_owned);
        let embedding_selected = stage_selection(
            &embedding,
            stages
                .embedding_model
                .as_ref()
                .map(|model| model.model_id.as_str()),
        )
        .map(str::to_owned);
        let clustering_selected = stages
            .clustering_algorithm
            .as_ref()
            .map(|algorithm| algorithm.id.clone())
            .or_else(|| clustering.first().cloned());
        selectors
            .diarization_segmentation_model
            .update(cx, |dropdown, cx| {
                dropdown.set_options(segmentation, segmentation_selected.as_deref(), cx)
            });
        selectors
            .diarization_embedding_model
            .update(cx, |dropdown, cx| {
                dropdown.set_options(embedding, embedding_selected.as_deref(), cx)
            });
        selectors.diarization_clustering.update(cx, |dropdown, cx| {
            dropdown.set_options(text_options(clustering), clustering_selected.as_deref(), cx)
        });
    }

    fn on_theme_selected(
        &mut self,
        _: &Entity<Dropdown>,
        event: &DropdownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let DropdownEvent::Confirm(value) = event;
        if let Some(theme) = theme_preference_from_label(self.ui.locale, value) {
            self.persist_field(move |settings| settings.appearance.theme = theme, cx);
        }
    }

    fn on_material_selected(
        &mut self,
        _: &Entity<Dropdown>,
        event: &DropdownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let DropdownEvent::Confirm(value) = event;
        if let Some(material) = material_preference_from_label(self.ui.locale, value) {
            self.persist_field(move |settings| settings.appearance.material = material, cx);
        }
    }

    fn on_language_selected(
        &mut self,
        _: &Entity<Dropdown>,
        event: &DropdownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let DropdownEvent::Confirm(value) = event;
        self.set_locale(
            if value == "English" {
                i18n::Locale::EnUs
            } else {
                i18n::Locale::ZhCn
            },
            cx,
        );
    }

    fn on_asr_backend_selected(
        &mut self,
        _: &Entity<Dropdown>,
        event: &DropdownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let DropdownEvent::Confirm(value) = event;
        self.select_asr_backend(value.clone(), cx);
        self.refresh_model_selector("asr", Some(value), cx);
    }

    fn on_asr_model_selected(
        &mut self,
        _: &Entity<Dropdown>,
        event: &DropdownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let DropdownEvent::Confirm(value) = event;
        if let Some(backend_id) = self.settings_draft.asr_backend_id.clone() {
            self.use_model(value.clone(), "asr".into(), backend_id, cx);
        }
    }

    fn on_transcription_language_selected(
        &mut self,
        _: &Entity<Dropdown>,
        event: &DropdownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let DropdownEvent::Confirm(value) = event;
        let language: Option<String> = match value.as_str() {
            "English" => Some("en".into()),
            "中文" => Some("zh".into()),
            _ => None,
        };
        self.persist_field(move |settings| settings.language = language.clone(), cx);
    }

    fn on_diarization_backend_selected(
        &mut self,
        _: &Entity<Dropdown>,
        event: &DropdownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let DropdownEvent::Confirm(value) = event;
        self.select_diarization_backend(value.clone(), cx);
        self.refresh_model_selector("diarization", Some(value), cx);
        self.refresh_stage_selectors(cx);
    }

    fn on_diarization_model_selected(
        &mut self,
        _: &Entity<Dropdown>,
        event: &DropdownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let DropdownEvent::Confirm(value) = event;
        if let Some(backend_id) = self.settings_draft.diarization_backend_id.clone() {
            self.use_model(value.clone(), "diarization".into(), backend_id, cx);
            self.refresh_stage_selectors(cx);
        }
    }

    fn on_diarization_segmentation_model_selected(
        &mut self,
        _: &Entity<Dropdown>,
        event: &DropdownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let DropdownEvent::Confirm(value) = event;
        let model = self.stage_model_ref(DiarizationStageKind::Segmentation, value);
        self.persist_field(
            move |settings| settings.diarization_stages.segmentation_model = model.clone(),
            cx,
        );
    }

    fn on_diarization_embedding_model_selected(
        &mut self,
        _: &Entity<Dropdown>,
        event: &DropdownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let DropdownEvent::Confirm(value) = event;
        let model = self.stage_model_ref(DiarizationStageKind::Embedding, value);
        self.persist_field(
            move |settings| settings.diarization_stages.embedding_model = model.clone(),
            cx,
        );
    }

    fn on_diarization_clustering_selected(
        &mut self,
        _: &Entity<Dropdown>,
        event: &DropdownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let DropdownEvent::Confirm(value) = event;
        let option = self
            .selected_diarization_descriptor()
            .and_then(|descriptor| {
                descriptor.diarization_stage_options.iter().find(|option| {
                    option.stage == DiarizationStageKind::Clustering && option.id == *value
                })
            });
        let algorithm = option.map(|option| contracts::AlgorithmRef {
            id: option.id.clone(),
            revision: option.revision.clone(),
            parameters: None,
        });
        self.persist_field(
            move |settings| settings.diarization_stages.clustering_algorithm = algorithm.clone(),
            cx,
        );
    }

    fn on_expected_speakers_selected(
        &mut self,
        _: &Entity<Dropdown>,
        event: &DropdownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let DropdownEvent::Confirm(value) = event;
        let expected_speakers = value.parse().ok();
        self.persist_field(
            move |settings| settings.expected_speakers = expected_speakers,
            cx,
        );
    }

    fn on_compute_device_selected(
        &mut self,
        _: &Entity<Dropdown>,
        event: &DropdownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let DropdownEvent::Confirm(value) = event;
        let device = compute_device_from_label(self.ui.locale, value);
        self.persist_field(move |settings| settings.compute_device = device, cx);
    }

    pub(crate) fn import_audio(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.documents_root_switch.switching {
            return;
        }
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(i18n::text(self.ui.locale, "choose_audio").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            this.update(cx, |this, cx| this.import_audio_path(path, cx))
                .ok();
        })
        .detach();
    }

    /// Imports `path` and reloads the project list off the main thread.
    pub(crate) fn import_audio_path(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        if self.documents_root_switch.switching {
            return;
        }
        let generation = self.begin_projects_reload();
        let root_generation = self.documents_root_generation;
        let documents = self.services.workspace.documents().frozen();
        cx.spawn(async move |this, cx| {
            let imported = runtime::spawn_blocking(move || {
                let project = documents.import_audio(path.display().to_string())?;
                Ok::<_, String>((project, documents.snapshot()))
            })
            .await;
            this.update(cx, |this, cx| {
                if root_generation != this.documents_root_generation {
                    cx.notify();
                    return;
                }
                match imported {
                    Ok((project, projects)) => {
                        let refreshed = projects.is_ok();
                        this.apply_projects(generation, projects, cx);
                        if refreshed {
                            this.ui.notice =
                                i18n::message(this.ui.locale, "imported", project.title());
                        }
                    }
                    Err(error) => {
                        this.ui.notice = i18n::message(this.ui.locale, "import_failed", &error)
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn choose_speaker_library_directory(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(i18n::text(self.ui.locale, "choose_directory").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            let Some(directory) = paths.into_iter().next() else {
                return;
            };
            let destination = directory.join("library.json");
            this.update(cx, move |this, cx| {
                this.change_speaker_library_path(destination, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Switches the Speaker Library to `destination`, offering to migrate the
    /// current library when only the current path has one. Both paths are
    /// probed off the main thread.
    pub(crate) fn change_speaker_library_path(
        &mut self,
        destination: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        let source = std::path::PathBuf::from(&self.settings_draft.speaker_library_path);
        if source == destination {
            return;
        }
        cx.spawn(async move |this, cx| {
            let paths = (source.clone(), destination.clone());
            let presence = runtime::spawn_blocking(move || {
                application::speaker_library_presence(&paths.0, &paths.1)
            })
            .await;
            this.update_in(cx, move |this, window, cx| {
                if presence.destination_exists || !presence.source_exists {
                    this.commit_speaker_library_path(destination, cx);
                } else {
                    this.ask_speaker_library_migration(source, destination, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn ask_speaker_library_migration(
        &mut self,
        source: std::path::PathBuf,
        destination: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.speaker_library_prompt = Some(SpeakerLibraryPrompt {
            source,
            destination,
        });
        self.focus_dialog(window, cx);
        cx.notify();
    }

    /// Moves focus to the dialog that just opened once the event that
    /// opened it is done, and remembers what held focus, which closing the
    /// dialog returns it to. A select that opened the dialog hands focus
    /// back to its trigger as its own event ends; the dialog takes it after.
    fn focus_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = self.dialog_focus.clone();
        let this = cx.entity().downgrade();
        window.defer(cx, move |window, cx| {
            let previous = window.focused(cx).filter(|focused| *focused != dialog);
            this.update(cx, |this, _| this.dialog_return = previous)
                .ok();
            dialog.focus(window, cx);
        });
    }

    /// Returns focus to what held it before the dialog that just closed.
    fn return_focus_from_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(previous) = self.dialog_return.take() {
            previous.focus(window, cx);
        }
    }

    /// Closes the migration question: `migrate` moves the library to the new
    /// path, otherwise only the path changes.
    pub(crate) fn answer_speaker_library_prompt(&mut self, migrate: bool, cx: &mut Context<Self>) {
        let Some(SpeakerLibraryPrompt {
            source,
            destination,
        }) = self.speaker_library_prompt.take()
        else {
            return;
        };
        if migrate {
            self.migrate_speaker_library(source, destination, cx);
        } else {
            self.commit_speaker_library_path(destination, cx);
        }
        cx.notify();
    }

    fn speaker_library_dialog(&self, cx: &mut Context<Self>) -> Option<ConfirmDialog> {
        let prompt = self.speaker_library_prompt.as_ref()?;
        let locale = self.ui.locale;
        let migrate = cx.entity().downgrade();
        let switch = migrate.clone();
        Some(
            ConfirmDialog::new(
                i18n::text(locale, "speaker_library_migration_title"),
                &self.dialog_focus,
            )
            .description(format!(
                "{}\n{}\n{}",
                i18n::text(locale, "speaker_library_migration_description"),
                prompt.source.display(),
                prompt.destination.display()
            ))
            .on_confirm(i18n::text(locale, "migrate"), move |window, cx| {
                migrate
                    .update(cx, |this, cx| {
                        this.answer_speaker_library_prompt(true, cx);
                        this.return_focus_from_dialog(window, cx);
                    })
                    .ok();
            })
            .on_cancel(i18n::text(locale, "use_new_path"), move |window, cx| {
                switch
                    .update(cx, |this, cx| {
                        this.answer_speaker_library_prompt(false, cx);
                        this.return_focus_from_dialog(window, cx);
                    })
                    .ok();
            }),
        )
    }

    fn migrate_speaker_library(
        &mut self,
        source: std::path::PathBuf,
        destination: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let target = destination.clone();
            let result =
                runtime::spawn_blocking(move || application::migrate_file(&source, &target)).await;
            this.update(cx, |this, cx| match result {
                Ok(()) => this.commit_speaker_library_path(destination, cx),
                Err(error) => {
                    this.ui.notice =
                        i18n::message(this.ui.locale, "speaker_library_migration_failed", &error);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn commit_speaker_library_path(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        let path = path.display().to_string();
        let saved_notice = i18n::message(self.ui.locale, "speaker_library_updated", &path);
        self.persist_field_with_notice(
            move |settings| settings.speaker_library_path = path.clone(),
            saved_notice,
            cx,
        );
    }

    /// Follows the draft's Speaker Library path: a new path opens the
    /// library there, gives processing and the workspace's documents-root
    /// check the new path, and reads the profiles again.
    fn sync_speaker_library(&mut self, cx: &mut Context<Self>) {
        if self.speaker_library_path == self.settings_draft.speaker_library_path {
            return;
        }
        self.speaker_library_path = self.settings_draft.speaker_library_path.clone();
        self.services.speaker_library = speaker_library_at(&self.speaker_library_path);
        if let Err(error) = self
            .services
            .workspace
            .set_processing_options(processing_options(&self.settings_draft))
        {
            self.ui.notice = i18n::message(self.ui.locale, "processing_failed", &error);
        }
        self.speaker_profiles.status = None;
        self.reload_speaker_profiles(cx);
    }

    /// Reads the Speaker Library profiles on a worker.
    pub(crate) fn reload_speaker_profiles(&mut self, cx: &mut Context<Self>) {
        let generation = self.speaker_profiles.begin_read();
        let library = self.services.speaker_library.clone();
        cx.spawn(async move |this, cx| {
            let read = runtime::spawn_blocking(move || {
                library.profiles().map_err(|error| error.to_string())
            })
            .await;
            this.update(cx, |this, cx| {
                let locale = this.ui.locale;
                this.speaker_profiles.apply_read(generation, read, locale);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Builds the rename and merge controls of the speakers of `document`
    /// when the Original view shows it; they are rebuilt once its revision
    /// or the locale changes.
    fn ensure_speaker_rows(
        &mut self,
        document: Option<&DocumentState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ui.route != Route::Meetings || self.ui.content_mode != ContentMode::Original {
            return;
        }
        let Some(document) =
            document.filter(|document| document.processing_status() == ProjectStatus::Ready)
        else {
            return;
        };
        let locale = self.ui.locale;
        let key = (document.id().clone(), document.revision(), locale);
        if self
            .speaker_rows
            .as_ref()
            .is_some_and(|rows| rows.is_for(&key))
        {
            return;
        }
        let entries = document_speakers(document)
            .iter()
            .map(|speaker| NameEntry::speaker(speaker, locale))
            .collect::<Vec<_>>();
        self.speaker_rows = Some(NameRows::build(
            key,
            &entries,
            i18n::text(locale, "merge_into_placeholder"),
            (
                Self::rename_document_speaker,
                Self::ask_merge_document_speaker,
            ),
            window,
            cx,
        ));
    }

    /// Builds the rename and merge controls of the Speaker Library profiles
    /// on the Settings route; they are rebuilt once the list or the locale
    /// changes.
    fn ensure_profile_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui.route != Route::Settings {
            self.profile_removal_focus = None;
            return;
        }
        let locale = self.ui.locale;
        let key = (self.speaker_profiles.version, locale);
        if self
            .profile_rows
            .as_ref()
            .is_some_and(|rows| rows.is_for(&key))
        {
            return;
        }
        let entries = self
            .speaker_profiles
            .profiles
            .iter()
            .map(|profile| NameEntry::profile(profile, locale))
            .collect::<Vec<_>>();
        self.profile_rows = Some(NameRows::build(
            key,
            &entries,
            i18n::text(locale, "merge_into_placeholder"),
            (
                Self::rename_speaker_profile,
                Self::ask_merge_speaker_profiles,
            ),
            window,
            cx,
        ));
        let removed = self.profile_removal_focus.as_ref().is_some_and(|request| {
            self.speaker_profiles.loaded
                && self.speaker_profiles.read_error.is_none()
                && self.speaker_profiles.find(&request.removed_id).is_none()
        });
        if removed {
            let request = self.profile_removal_focus.take().unwrap();
            // A user may have moved on while the library worker ran.
            if window.focused(cx) == request.previous {
                let destination = request
                    .next_id
                    .as_deref()
                    .and_then(|id| self.profile_rows.as_ref()?.row(id))
                    .map(|row| gpui::Focusable::focus_handle(row.name.read(cx), cx));
                let destination = destination.unwrap_or_else(|| {
                    self.scroll
                        .settings
                        .scroll_to_top_of_item(pages::settings::SPEAKER_LIBRARY_SECTION);
                    self.speaker_library_focus.clone()
                });
                window.defer(cx, move |window, cx| {
                    if window.focused(cx) == request.previous {
                        destination.focus(window, cx);
                    }
                });
            }
        }
    }

    /// Sets the typed name as `speaker_id`'s label, or clears it when the
    /// field is blank: one `SetSpeakerLabel` against the revision the list
    /// was read at.
    pub(crate) fn rename_document_speaker(&mut self, speaker_id: String, cx: &mut Context<Self>) {
        let Some(rows) = &self.speaker_rows else {
            return;
        };
        let Some((name, changed)) = rows.edited_name(&speaker_id, cx) else {
            return;
        };
        let (document_id, revision, _) = rows.key().clone();
        if !changed {
            self.ui.notice = i18n::text(self.ui.locale, "speaker_name_unchanged").into();
            cx.notify();
            return;
        }
        let shown = if name.is_empty() { &speaker_id } else { &name };
        let done = i18n::message(self.ui.locale, "speaker_renamed", shown);
        let label = (!name.is_empty()).then_some(name);
        self.run_speaker_command(
            DocumentCommand::SetSpeakerLabel {
                document_id,
                expected_revision: revision,
                speaker_id,
                label,
            },
            revision,
            done,
            cx,
        );
    }

    /// Asks before `from`'s segments are given to `into`.
    pub(crate) fn ask_merge_document_speaker(
        &mut self,
        from: String,
        into: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.documents_root_switch.switching {
            return;
        }
        let Some(rows) = &self.speaker_rows else {
            return;
        };
        let (document_id, revision, _) = rows.key().clone();
        let Some(document) = self
            .projects
            .iter()
            .find(|document| document.id() == &document_id)
        else {
            return;
        };
        let speakers = document_speakers(document);
        let name = |id: &str| {
            speakers
                .iter()
                .find(|speaker| speaker.id == id)
                .map_or_else(|| id.to_string(), |speaker| speaker.name().to_string())
        };
        self.speaker_confirmation = Some(SpeakerConfirmation::MergeDocumentSpeaker {
            document_id,
            revision,
            from_name: name(&from),
            into_name: name(&into),
            from,
            into,
        });
        self.focus_dialog(window, cx);
        cx.notify();
    }

    /// Runs a speaker correction on a worker and reloads the tree, whose
    /// new revision rebuilds the speaker rows. A conflict opens the
    /// conflict notice; nothing was written then.
    fn run_speaker_command(
        &mut self,
        command: DocumentCommand,
        revision: u64,
        done: String,
        cx: &mut Context<Self>,
    ) {
        if self.documents_root_switch.switching {
            return;
        }
        let (DocumentCommand::SetSpeakerLabel { document_id, .. }
        | DocumentCommand::MergeSpeaker { document_id, .. }) = &command
        else {
            return;
        };
        if self.speaker_command.is_some() {
            return;
        }
        let envelope = DocumentCommandEnvelope::new(
            format!("ui-speaker-{document_id}-{revision}"),
            command.clone(),
        );
        self.speaker_command = Some(document_id.clone());
        cx.notify();
        let commands = self.services.workspace.document_commands();
        let root_generation = self.documents_root_generation;
        cx.spawn(async move |this, cx| {
            let result = runtime::spawn_blocking(move || commands.execute(envelope)).await;
            this.update(cx, |this, cx| {
                if root_generation != this.documents_root_generation {
                    cx.notify();
                    return;
                }
                this.speaker_command = None;
                match result {
                    Ok(_) => {
                        this.ui.notice = done;
                        this.reload_projects(cx);
                    }
                    Err(error) => this.document_write_failed(error, "speaker_edit_failed", cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Renames profile `profile_id` to the typed name.
    pub(crate) fn rename_speaker_profile(&mut self, profile_id: String, cx: &mut Context<Self>) {
        let Some((name, changed)) = self
            .profile_rows
            .as_ref()
            .and_then(|rows| rows.edited_name(&profile_id, cx))
        else {
            return;
        };
        if !changed {
            let unchanged = i18n::text(self.ui.locale, "speaker_name_unchanged");
            self.speaker_profiles.status = Some((unchanged.into(), false));
            cx.notify();
            return;
        }
        let done = i18n::message(self.ui.locale, "speaker_profile_renamed", &name);
        self.run_speaker_library_call(
            move |library| library.rename(&profile_id, name).map(drop),
            done,
            cx,
        );
    }

    /// Asks before profile `profile_id` is deleted.
    pub(crate) fn ask_delete_speaker_profile(
        &mut self,
        profile_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(profile) = self.speaker_profiles.find(&profile_id) else {
            return;
        };
        self.speaker_confirmation = Some(SpeakerConfirmation::DeleteProfile {
            name: profile.display_name.clone(),
            profile_id,
        });
        self.focus_dialog(window, cx);
        cx.notify();
    }

    /// Asks before profile `source_id` is merged into `target_id`.
    pub(crate) fn ask_merge_speaker_profiles(
        &mut self,
        source_id: String,
        target_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = |id: &str| {
            self.speaker_profiles
                .find(id)
                .map(|profile| profile.display_name.clone())
        };
        let (Some(source_name), Some(target_name)) = (name(&source_id), name(&target_id)) else {
            return;
        };
        self.speaker_confirmation = Some(SpeakerConfirmation::MergeProfiles {
            source_id,
            source_name,
            target_id,
            target_name,
        });
        self.focus_dialog(window, cx);
        cx.notify();
    }

    /// Closes the open speaker confirmation; `confirmed` runs its change.
    pub(crate) fn answer_speaker_confirmation(&mut self, confirmed: bool, cx: &mut Context<Self>) {
        let Some(confirmation) = self.speaker_confirmation.take() else {
            return;
        };
        let locale = self.ui.locale;
        match confirmation {
            SpeakerConfirmation::MergeDocumentSpeaker {
                document_id,
                revision,
                from,
                from_name,
                into,
                into_name,
            } => {
                if let Some(rows) = &self.speaker_rows {
                    rows.clear_merge_choice(&from, cx);
                }
                if confirmed {
                    let done =
                        i18n::message_pair(locale, "speakers_merged", &from_name, &into_name);
                    self.run_speaker_command(
                        DocumentCommand::MergeSpeaker {
                            document_id,
                            expected_revision: revision,
                            from_speaker_id: from,
                            into_speaker_id: into,
                        },
                        revision,
                        done,
                        cx,
                    );
                }
            }
            SpeakerConfirmation::DeleteProfile { profile_id, name } => {
                if confirmed {
                    let next_id = self
                        .speaker_profiles
                        .profiles
                        .iter()
                        .position(|profile| profile.id == profile_id)
                        .and_then(|index| self.speaker_profiles.profiles.get(index + 1))
                        .map(|profile| profile.id.clone());
                    self.profile_removal_focus = Some(ProfileRemovalFocus {
                        removed_id: profile_id.clone(),
                        next_id,
                        previous: self.dialog_return.clone(),
                    });
                    let done = i18n::message(locale, "speaker_profile_deleted", &name);
                    self.run_speaker_library_call(
                        move |library| match library.delete(&profile_id)? {
                            true => Ok(()),
                            false => Err(SpeakerLibraryError::ProfileNotFound(profile_id)),
                        },
                        done,
                        cx,
                    );
                }
            }
            SpeakerConfirmation::MergeProfiles {
                source_id,
                source_name,
                target_id,
                target_name,
            } => {
                if let Some(rows) = &self.profile_rows {
                    rows.clear_merge_choice(&source_id, cx);
                }
                if confirmed {
                    let done = i18n::message_pair(
                        locale,
                        "speaker_profiles_merged",
                        &source_name,
                        &target_name,
                    );
                    self.run_speaker_library_call(
                        move |library| library.merge_profiles(&target_id, &source_id).map(drop),
                        done,
                        cx,
                    );
                }
            }
        }
        cx.notify();
    }

    fn speaker_confirmation_dialog(&self, cx: &mut Context<Self>) -> Option<ConfirmDialog> {
        let confirmation = self.speaker_confirmation.as_ref()?;
        let locale = self.ui.locale;
        let confirm = cx.entity().downgrade();
        let cancel = confirm.clone();
        Some(
            ConfirmDialog::new(confirmation.title(locale), &self.dialog_focus)
                .description(confirmation.description(locale))
                .on_confirm(confirmation.confirm_label(locale), move |window, cx| {
                    confirm
                        .update(cx, |this, cx| {
                            this.answer_speaker_confirmation(true, cx);
                            this.return_focus_from_dialog(window, cx);
                        })
                        .ok();
                })
                .on_cancel(i18n::text(locale, "cancel"), move |window, cx| {
                    cancel
                        .update(cx, |this, cx| {
                            this.answer_speaker_confirmation(false, cx);
                            this.return_focus_from_dialog(window, cx);
                        })
                        .ok();
                }),
        )
    }

    /// Runs one Speaker Library call on a worker, shows its outcome and
    /// reads the profiles again; calls run one at a time.
    fn run_speaker_library_call(
        &mut self,
        call: impl FnOnce(&SpeakerLibraryService) -> Result<(), SpeakerLibraryError> + Send + 'static,
        done: String,
        cx: &mut Context<Self>,
    ) {
        if self.speaker_profiles.busy {
            return;
        }
        self.speaker_profiles.busy = true;
        cx.notify();
        let library = self.services.speaker_library.clone();
        cx.spawn(async move |this, cx| {
            let result =
                runtime::spawn_blocking(move || call(&library).map_err(|error| error.to_string()))
                    .await;
            this.update(cx, |this, cx| {
                this.speaker_profiles.busy = false;
                if result.is_err() {
                    this.profile_removal_focus = None;
                }
                this.speaker_profiles.status = Some(match result {
                    Ok(()) => (done, false),
                    Err(error) => (
                        i18n::message(this.ui.locale, "speaker_library_failed", &error),
                        true,
                    ),
                });
                this.reload_speaker_profiles(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn select_project(&mut self, project_id: ProjectId, cx: &mut Context<Self>) {
        self.ui.selected_project = Some(project_id);
        self.ui.remember_navigation();
        self.ui.notice.clear();
        self.check_selected_audio(cx);
        cx.notify();
    }

    pub(crate) fn navigate(&mut self, route: Route, cx: &mut Context<Self>) {
        self.sidebar_sizing
            .update(cx, |state, _| state.reset_interaction());
        self.ui.route = route;
        self.ui.remember_navigation();
        self.ui.detail_open = false;
        // Processing or another writer may have changed the library.
        if route == Route::Settings {
            self.reload_speaker_profiles(cx);
        }
        cx.notify();
    }

    pub(crate) fn travel(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.ui.travel(forward) {
            self.sidebar_sizing
                .update(cx, |state, _| state.reset_interaction());
            if self.ui.route == Route::Settings {
                self.reload_speaker_profiles(cx);
            }
            self.check_selected_audio(cx);
            cx.notify();
        }
    }

    /// Folds the Meetings or Settings sidebar closed or open; the other
    /// routes have none. Focus in the folding project tree moves to the
    /// Meetings entry.
    pub(crate) fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.ui.route, Route::Meetings | Route::Settings) {
            return;
        }
        if self.ui.sidebar_open && self.tree_focus.contains_focused(window, cx) {
            self.nav_focus[1].focus(window, cx);
        }
        self.ui.sidebar_open = !self.ui.sidebar_open;
        let (open, duration) = (self.ui.sidebar_open, theme::tokens(cx).motion.layout);
        let now = cx.background_executor().now();
        self.sidebar_sizing
            .update(cx, |state, _| state.set_open(open, duration, now));
        cx.notify();
    }

    /// Runs a window shortcut as its control in the shell would.
    fn run_shortcut(
        &mut self,
        shortcut: &controls::window::Shortcut,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use controls::window::Shortcut;
        match *shortcut {
            Shortcut::Entry(index) => self.navigate(pages::chrome::ENTRIES[index].0, cx),
            Shortcut::Settings => self.navigate(Route::Settings, cx),
            Shortcut::Back => self.travel(false, cx),
            Shortcut::Forward => self.travel(true, cx),
            Shortcut::ToggleSidebar => self.toggle_sidebar(window, cx),
            Shortcut::ImportAudio => self.import_audio(&ClickEvent::default(), window, cx),
            Shortcut::SaveDocument => {
                if self.ui.route == Route::Meetings
                    && self.ui.content_mode == ContentMode::Edit
                    && self.ui.selected_project.as_ref().is_some_and(|id| {
                        self.editor.shown() == Some(id) && !self.editor.read_errors.contains_key(id)
                    })
                {
                    self.save_document_draft(&ClickEvent::default(), window, cx);
                }
            }
        }
    }

    pub(crate) fn set_locale(&mut self, locale: i18n::Locale, cx: &mut Context<Self>) {
        self.persist_field_with_notice(
            move |settings| settings.ui_locale = locale.code().into(),
            i18n::text(locale, "language_changed").into(),
            cx,
        );
        self.sync_locale(cx);
    }

    pub(crate) fn open_detail(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.ui.selected_project.is_some() {
            self.ui.detail_open = true;
            cx.notify();
        }
    }

    /// Hides Detail; processing and downloads carry on. Focus inside Detail
    /// returns to the button that opens it.
    pub(crate) fn close_detail(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.detail_focus.contains_focused(window, cx) {
            self.detail_trigger.focus(window, cx);
        }
        self.ui.detail_open = false;
        cx.notify();
    }

    pub(crate) fn set_content_mode(&mut self, mode: ContentMode, cx: &mut Context<Self>) {
        self.ui.content_mode = mode;
        cx.notify();
    }

    /// Shows the selected document's draft in the editor, first reading it
    /// on a worker when it has none. Edit mode calls it on every render; it
    /// does nothing once the draft is shown or being read.
    fn ensure_document_editor(&mut self, cx: &mut Context<Self>) {
        if self.ui.route != Route::Meetings || self.ui.content_mode != ContentMode::Edit {
            return;
        }
        let Some(document_id) = self.ui.selected_project.clone() else {
            return;
        };
        if self.editor.loading.contains(&document_id)
            || self.editor.read_errors.contains_key(&document_id)
            || self
                .editor
                .show(&document_id, &self.projects, self.ui.locale, cx)
        {
            return;
        }
        self.load_document_draft(document_id, cx);
    }

    /// Reads `document_id` (state and Markdown body) on a worker and makes it
    /// the document's draft, replacing any edits.
    fn load_document_draft(&mut self, document_id: ProjectId, cx: &mut Context<Self>) {
        if self.documents_root_switch.switching {
            return;
        }
        let previous_error = self.editor.read_errors.remove(&document_id);
        let generation = self.documents_repository_generation;
        self.editor.loading.insert(document_id.clone());
        let commands = self.services.workspace.document_commands();
        let read_id = document_id.clone();
        cx.spawn(async move |this, cx| {
            let result = runtime::spawn_blocking(move || {
                commands.execute(DocumentCommandEnvelope::new(
                    format!("ui-read-{read_id}"),
                    DocumentCommand::Read {
                        document_id: read_id,
                    },
                ))
            })
            .await;
            this.update(cx, |this, cx| {
                if generation != this.documents_repository_generation {
                    cx.notify();
                    return;
                }
                this.editor.loading.remove(&document_id);
                match result {
                    Ok(DocumentCommandResult::Document(Some(record))) => {
                        if previous_error.as_ref() == Some(&this.ui.notice) {
                            this.ui.notice.clear();
                        }
                        if let Some(generation) = this
                            .ui
                            .toast()
                            .filter(|toast| previous_error.as_ref() == Some(&toast.notice))
                            .map(|toast| toast.generation)
                        {
                            this.dismiss_toast(generation, cx);
                        }
                        this.show_loaded_draft(&record, cx)
                    }
                    Ok(_) => {
                        let message = i18n::text(this.ui.locale, "project_missing").to_string();
                        this.editor
                            .read_errors
                            .insert(document_id.clone(), message.clone());
                        this.ui.notice = message;
                    }
                    Err(error) => {
                        let message = i18n::message(
                            this.ui.locale,
                            "document_read_failed",
                            &error.to_string(),
                        );
                        this.editor
                            .read_errors
                            .insert(document_id.clone(), message.clone());
                        this.ui.notice = message;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Retries the selected document's failed read, leaving other drafts intact.
    pub(crate) fn retry_document_read(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.ui.selected_project.clone() {
            if self.editor.read_errors.contains_key(&id) {
                self.load_document_draft(id, cx);
                cx.notify();
            }
        }
    }

    /// Makes `record` its document's draft, shown when it is selected.
    pub(crate) fn show_loaded_draft(&mut self, record: &DocumentRecord, cx: &mut Context<Self>) {
        let show = self.ui.selected_project.as_ref() == Some(record.state.id());
        let locale = self.ui.locale;
        self.editor.load(record, show, &self.projects, locale, cx);
    }

    /// Saves the shown draft: one `Update` with only the changed fields,
    /// checked against the revision the draft was read at, on a worker.
    pub(crate) fn save_document_draft(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.documents_root_switch.switching {
            return;
        }
        if self.editor.saving.is_some() {
            return;
        }
        let Some(draft) = self.editor.current(cx) else {
            return;
        };
        let patch = match crate::editor::draft_patch(&draft.base, &draft.edits) {
            Ok(patch) if patch == contracts::DocumentPatch::default() => {
                self.ui.notice = i18n::text(self.ui.locale, "no_document_changes").into();
                cx.notify();
                return;
            }
            Ok(patch) => patch,
            Err(error) => {
                self.ui.notice = i18n::text(self.ui.locale, error.notice_key()).into();
                cx.notify();
                return;
            }
        };
        let generation = self.documents_root_generation;
        let document_id = draft.document_id;
        self.editor.saving = Some(document_id.clone());
        cx.notify();
        let commands = self.services.workspace.document_commands();
        let envelope = DocumentCommandEnvelope::new(
            format!("ui-update-{document_id}-{}", draft.revision),
            DocumentCommand::Update {
                document_id,
                expected_revision: draft.revision,
                patch,
            },
        );
        cx.spawn(async move |this, cx| {
            let result = runtime::spawn_blocking(move || commands.execute(envelope)).await;
            this.update(cx, |this, cx| {
                if generation != this.documents_root_generation {
                    cx.notify();
                    return;
                }
                this.editor.saving = None;
                match result {
                    Ok(DocumentCommandResult::Updated(record)) => {
                        this.ui.notice =
                            i18n::message(this.ui.locale, "document_saved", record.state.title());
                        this.editor.saved(&record);
                        this.reload_projects(cx);
                    }
                    Ok(_) => {}
                    Err(error) => this.document_write_failed(error, "document_save_failed", cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A refused document write: a revision conflict opens the conflict
    /// notice and reloads the tree; any other error is reported with
    /// `failure_key`. The draft is kept either way.
    fn document_write_failed(
        &mut self,
        error: DocumentCommandError,
        failure_key: &str,
        cx: &mut Context<Self>,
    ) {
        match error {
            DocumentCommandError::RevisionConflict {
                document_id,
                expected,
                actual,
            } => {
                let title = self
                    .projects
                    .iter()
                    .find(|project| project.id() == &document_id)
                    .map_or_else(|| document_id.to_string(), |project| project.title().into());
                self.ui.notice = i18n::message(self.ui.locale, "document_conflict", &title);
                self.document_conflict = Some(DocumentConflict {
                    document_id,
                    title,
                    expected,
                    actual,
                });
                self.reload_projects(cx);
            }
            error => {
                self.ui.notice = i18n::message(self.ui.locale, failure_key, &error.to_string())
            }
        }
    }

    /// The conflict notice's reload: reads the tree and the conflicting
    /// document again, discarding that document's draft, and closes the
    /// notice.
    pub(crate) fn reload_conflicted_document(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.documents_root_switch.switching {
            return;
        }
        let Some(conflict) = self.document_conflict.take() else {
            return;
        };
        let had_draft = self.editor.has_draft(&conflict.document_id);
        self.editor.discard(&conflict.document_id);
        if had_draft {
            self.load_document_draft(conflict.document_id, cx);
        }
        self.reload_projects(cx);
        self.ui.notice = i18n::message(self.ui.locale, "document_reloaded", &conflict.title);
        cx.notify();
    }

    /// Drops the dragged row onto `new_parent_id`: a `Reparent` checked
    /// against the revision the row showed when the drag started. The row
    /// becomes the new parent's last child.
    pub(crate) fn reparent_document(
        &mut self,
        drag: ProjectDrag,
        new_parent_id: ProjectId,
        cx: &mut Context<Self>,
    ) {
        if drag.root_generation != self.documents_root_generation {
            cx.notify();
            return;
        }
        self.run_tree_command(
            self.reparent_command(drag, Some(new_parent_id)),
            "hierarchy_updated",
            "move_failed",
            TreeMoveFocus::Pointer,
            cx,
        );
    }

    /// A `Reparent` of `drag` that makes it the last child of
    /// `new_parent_id` (a root for `None`).
    fn reparent_command(
        &self,
        drag: ProjectDrag,
        new_parent_id: Option<ProjectId>,
    ) -> DocumentCommand {
        let sibling_order = self
            .projects
            .iter()
            .filter(|project| {
                project.hierarchy.parent_id == new_parent_id && project.id() != &drag.project_id
            })
            .count() as u32;
        DocumentCommand::Reparent {
            document_id: drag.project_id,
            expected_revision: drag.revision,
            new_parent_id,
            sibling_order,
        }
    }

    /// Drops the dragged row before or after its sibling `target_id`: a
    /// `Reorder` checked against the revision read at drag start.
    pub(crate) fn reorder_document(
        &mut self,
        drag: ProjectDrag,
        target_id: ProjectId,
        before: bool,
        cx: &mut Context<Self>,
    ) {
        if drag.root_generation != self.documents_root_generation {
            cx.notify();
            return;
        }
        self.run_tree_command(
            Self::reorder_command(drag, target_id, before),
            "order_updated",
            "reorder_failed",
            TreeMoveFocus::Pointer,
            cx,
        );
    }

    fn reorder_command(drag: ProjectDrag, target_id: ProjectId, before: bool) -> DocumentCommand {
        DocumentCommand::Reorder {
            document_id: drag.project_id,
            expected_revision: drag.revision,
            target_id,
            before,
        }
    }

    /// An Option-arrow move of the selected row: the same revision-checked
    /// `Reorder` or `Reparent` a drop runs, against the revision the tree
    /// shows. A row moved into its sibling opens that sibling, and the tree
    /// scrolls the row into view where it lands.
    pub(crate) fn shift_project(&mut self, shift: TreeShift, cx: &mut Context<Self>) {
        let Some(document) = self
            .ui
            .selected_project
            .as_ref()
            .and_then(|id| self.projects.iter().find(|project| project.id() == id))
        else {
            return;
        };
        let drag = ProjectDrag {
            project_id: document.id().clone(),
            revision: document.revision(),
            root_generation: self.documents_root_generation,
        };
        match crate::components::tree_shift(&self.projects, &drag.project_id, shift) {
            Some(TreeShiftMove::Reorder { target, before }) => self.run_tree_command(
                Self::reorder_command(drag, target, before),
                "order_updated",
                "reorder_failed",
                TreeMoveFocus::Keyboard { open: None },
                cx,
            ),
            Some(TreeShiftMove::Reparent { parent }) => self.run_tree_command(
                self.reparent_command(drag, parent.clone()),
                "hierarchy_updated",
                "move_failed",
                TreeMoveFocus::Keyboard { open: parent },
                cx,
            ),
            None => {}
        }
    }

    /// Runs a tree command on a worker and shows the tree it returns; after
    /// a keyboard move, `focus` opens the new parent and reveals the row.
    fn run_tree_command(
        &mut self,
        command: DocumentCommand,
        success_key: &'static str,
        failure_key: &'static str,
        focus: TreeMoveFocus,
        cx: &mut Context<Self>,
    ) {
        if self.documents_root_switch.switching {
            return;
        }
        let root_generation = self.documents_root_generation;
        let generation = self.begin_projects_reload();
        let commands = self.services.workspace.document_commands();
        let envelope = DocumentCommandEnvelope::new("ui-tree-move", command);
        cx.spawn(async move |this, cx| {
            let result = runtime::spawn_blocking(move || commands.execute(envelope)).await;
            this.update(cx, |this, cx| {
                if root_generation != this.documents_root_generation {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(DocumentCommandResult::Snapshot { documents, issues }) => {
                        // The listing's issues carry no kind; each keeps
                        // the kind the last scan read for it.
                        let issues = issues
                            .into_iter()
                            .map(|issue| {
                                let kind = this
                                    .document_issues
                                    .iter()
                                    .find(|known| {
                                        known.path == issue.path && known.message == issue.message
                                    })
                                    .and_then(|known| known.kind.clone());
                                DocumentIssue {
                                    path: issue.path,
                                    message: issue.message,
                                    kind,
                                }
                            })
                            .collect();
                        this.apply_projects(
                            generation,
                            Ok(DocumentSnapshot { documents, issues }),
                            cx,
                        );
                        this.ui.notice = i18n::text(this.ui.locale, success_key).into();
                        if let TreeMoveFocus::Keyboard { open } = focus {
                            this.ui.expanded_projects.extend(open);
                            this.reveal_tree_selection = true;
                        }
                    }
                    Ok(_) => this.reload_projects(cx),
                    Err(error) => this.document_write_failed(error, failure_key, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn edit_assistant_profile(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(profile) = self
            .settings_draft
            .assistant
            .connections
            .iter()
            .find(|profile| profile.id == id)
            .cloned()
        else {
            return;
        };
        self.assistant.forget_probe();
        self.settings_draft.assistant.active_connection_id = Some(profile.id.clone());
        self.assistant_protocol = profile.protocol;
        self.assistant_auth_mode = profile.auth_mode;
        self.assistant_inputs.load_profile(Some(&profile), cx);
        self.forget_credential_target_conflict();
        cx.notify();
    }

    /// A refused credential belongs to the profile the form held.
    fn forget_credential_target_conflict(&mut self) {
        if self
            .settings_error
            .as_ref()
            .is_some_and(AssistantUiError::is_credential_target_conflict)
        {
            self.settings_error = None;
        }
    }

    /// Whether the profile in the form keeps a stored credential while its
    /// key and key file fields are left as loaded.
    pub(crate) fn form_keeps_stored_credential(&self, cx: &App) -> bool {
        let id = AssistantInputs::value(&self.assistant_inputs.connection_id, cx);
        let key = AssistantInputs::value(&self.assistant_inputs.api_key, cx);
        let key_file = AssistantInputs::value(&self.assistant_inputs.api_key_file, cx);
        self.assistant_origin
            .connections
            .iter()
            .find(|profile| profile.id == id)
            .is_some_and(|stored| {
                (stored.api_key.is_some() || stored.api_key_file.is_some())
                    && credential_update(&key, &key_file, Some(stored)).is_none()
            })
    }

    /// The write mode the settings hold; the form's is a draft until saved.
    pub(crate) fn saved_write_mode(&self) -> AgentWriteMode {
        self.assistant_origin.write_mode
    }

    pub(crate) fn new_assistant_profile(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.assistant.forget_probe();
        self.assistant_protocol = LlmProtocol::OpenAiChatCompletions;
        self.assistant_auth_mode = AssistantAuthMode::Bearer;
        self.assistant_inputs.load_profile(None, cx);
        self.forget_credential_target_conflict();
        cx.notify();
    }

    pub(crate) fn set_assistant_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings_draft.assistant.enabled = enabled;
        cx.notify();
    }

    pub(crate) fn set_assistant_write_mode(
        &mut self,
        write_mode: AgentWriteMode,
        cx: &mut Context<Self>,
    ) {
        self.settings_draft.assistant.write_mode = write_mode;
        cx.notify();
    }

    pub(crate) fn set_assistant_protocol(&mut self, protocol: LlmProtocol, cx: &mut Context<Self>) {
        if self.assistant_protocol != protocol {
            self.assistant.forget_probe();
            self.assistant_protocol = protocol;
        }
        cx.notify();
    }

    pub(crate) fn set_assistant_auth_mode(
        &mut self,
        auth_mode: AssistantAuthMode,
        cx: &mut Context<Self>,
    ) {
        if self.assistant_auth_mode != auth_mode {
            self.assistant.forget_probe();
            self.assistant_auth_mode = auth_mode;
        }
        cx.notify();
    }

    /// The profile in the form and its credential decision. The returned
    /// profile carries the credential it would use: the typed key, the key
    /// file, or the stored credential when the decision is `None`.
    fn assistant_profile_from_inputs(
        &self,
        cx: &Context<Self>,
    ) -> Result<Option<(AssistantConnectionProfile, Option<CredentialUpdate>)>, AssistantUiError>
    {
        let id = AssistantInputs::value(&self.assistant_inputs.connection_id, cx);
        let name = AssistantInputs::value(&self.assistant_inputs.connection_name, cx);
        let base_url = AssistantInputs::value(&self.assistant_inputs.base_url, cx);
        let model = AssistantInputs::value(&self.assistant_inputs.model, cx);
        let api_key = AssistantInputs::value(&self.assistant_inputs.api_key, cx);
        let api_key_file = AssistantInputs::value(&self.assistant_inputs.api_key_file, cx);
        if [&id, &name, &base_url, &model, &api_key, &api_key_file]
            .iter()
            .all(|value| value.trim().is_empty())
        {
            return Ok(None);
        }
        let stored = self
            .assistant_origin
            .connections
            .iter()
            .find(|profile| profile.id == id);
        let credential = credential_update(&api_key, &api_key_file, stored);
        let (api_key, api_key_file) = match &credential {
            Some(CredentialUpdate::Direct(key)) => (Some(key.clone()), None),
            Some(CredentialUpdate::File(file)) => (None, Some(file.clone())),
            Some(CredentialUpdate::Clear) => (None, None),
            None => stored.map_or((None, None), |stored| {
                (stored.api_key.clone(), stored.api_key_file.clone())
            }),
        };
        let profile = AssistantConnectionProfile {
            id,
            name,
            protocol: self.assistant_protocol,
            base_url,
            model,
            auth_mode: self.assistant_auth_mode,
            api_key,
            api_key_file,
            context_window_tokens: parse_optional_u32(
                &AssistantInputs::value(&self.assistant_inputs.context_window_tokens, cx),
                "contextWindowTokens",
            )?,
            timeout_ms: parse_u64(
                &AssistantInputs::value(&self.assistant_inputs.timeout_ms, cx),
                "timeoutMs",
            )?,
            max_output_tokens: parse_u32(
                &AssistantInputs::value(&self.assistant_inputs.max_output_tokens, cx),
                "maxOutputTokens",
            )?,
        };
        Ok(Some((profile, credential)))
    }

    /// Only the fields that differ from `assistant_origin` are saved, so
    /// fields changed elsewhere since the form loaded are kept.
    fn assistant_save_from_inputs(
        &self,
        cx: &Context<Self>,
    ) -> Result<AssistantSettingsSave, AssistantUiError> {
        let origin = &self.assistant_origin;
        let draft = &self.settings_draft.assistant;
        let max_tool_rounds = parse_u32(
            &AssistantInputs::value(&self.assistant_inputs.max_tool_rounds, cx),
            "maxToolRounds",
        )?;
        let max_validation_retries = parse_u32(
            &AssistantInputs::value(&self.assistant_inputs.max_validation_retries, cx),
            "maxValidationRetries",
        )?;
        let mut save = AssistantSettingsSave::default();
        let mut active_connection_id = draft.active_connection_id.clone();
        if let Some((profile, credential)) = self.assistant_profile_from_inputs(cx)? {
            active_connection_id = Some(profile.id.clone());
            let unchanged = AssistantConnectionPatch {
                id: profile.id.clone(),
                ..Default::default()
            };
            let connection = match origin
                .connections
                .iter()
                .find(|stored| stored.id == profile.id)
            {
                Some(stored) => connection_patch(stored, &profile, credential),
                None => {
                    save.new_connection = Some(AssistantConnectionProfile {
                        api_key: None,
                        api_key_file: None,
                        ..profile
                    });
                    AssistantConnectionPatch {
                        credential,
                        ..unchanged.clone()
                    }
                }
            };
            save.patch.connection = (connection != unchanged).then_some(connection);
        }
        save.patch.assistant_enabled = changed(&origin.enabled, &draft.enabled);
        save.patch.write_mode = changed(&origin.write_mode, &draft.write_mode);
        save.patch.active_connection_id =
            changed(&origin.active_connection_id, &active_connection_id);
        save.patch.max_tool_rounds = changed(&origin.max_tool_rounds, &max_tool_rounds);
        save.patch.max_validation_retries =
            changed(&origin.max_validation_retries, &max_validation_retries);
        Ok(save)
    }

    fn assistant_form(&self, cx: &App) -> AssistantForm {
        AssistantForm {
            connection_id: AssistantInputs::value(&self.assistant_inputs.connection_id, cx),
            connection_name: AssistantInputs::value(&self.assistant_inputs.connection_name, cx),
            base_url: AssistantInputs::value(&self.assistant_inputs.base_url, cx),
            model: AssistantInputs::value(&self.assistant_inputs.model, cx),
            api_key: AssistantInputs::value(&self.assistant_inputs.api_key, cx),
            api_key_file: AssistantInputs::value(&self.assistant_inputs.api_key_file, cx),
            context_window_tokens: AssistantInputs::value(
                &self.assistant_inputs.context_window_tokens,
                cx,
            ),
            timeout_ms: AssistantInputs::value(&self.assistant_inputs.timeout_ms, cx),
            max_output_tokens: AssistantInputs::value(&self.assistant_inputs.max_output_tokens, cx),
            max_tool_rounds: AssistantInputs::value(&self.assistant_inputs.max_tool_rounds, cx),
            max_validation_retries: AssistantInputs::value(
                &self.assistant_inputs.max_validation_retries,
                cx,
            ),
            protocol: self.assistant_protocol,
            auth_mode: self.assistant_auth_mode,
            enabled: self.settings_draft.assistant.enabled,
            write_mode: self.settings_draft.assistant.write_mode,
            active_connection_id: self.settings_draft.assistant.active_connection_id.clone(),
        }
    }

    pub(crate) fn save_assistant_settings(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.assistant_save_pending {
            return;
        }
        match self.assistant_save_from_inputs(cx) {
            Ok(save) => {
                let submitted = self.assistant_form(cx);
                self.assistant_save_pending = true;
                self.queue_settings_write(SettingsWrite::Assistant { save, submitted }, cx);
                cx.notify();
            }
            Err(error) => {
                self.ui.notice = error.to_string();
                cx.notify();
            }
        }
    }

    fn probe_assistant_connection(&mut self, kind: ConnectionProbeKind, cx: &mut Context<Self>) {
        let profile = match self.assistant_profile_from_inputs(cx) {
            Ok(Some((profile, credential))) => {
                let validation = self
                    .assistant_origin
                    .connections
                    .iter()
                    .find(|stored| stored.id == profile.id)
                    .map_or(Ok(()), |stored| {
                        connection_patch(stored, &profile, credential)
                            .validate_credential_target(stored)
                    });
                if let Err(error) = validation {
                    let error = AssistantUiError::from_settings(&error);
                    self.ui.notice = settings_error_text(self.ui.locale, &error);
                    self.settings_error = Some(error);
                    cx.notify();
                    return;
                }
                self.forget_credential_target_conflict();
                profile
            }
            Ok(None) => {
                self.ui.notice = i18n::text(self.ui.locale, "assistant_connection_required").into();
                cx.notify();
                return;
            }
            Err(error) => {
                self.ui.notice = error.to_string();
                cx.notify();
                return;
            }
        };
        let (probe_id, cancellation) = self.assistant.begin_probe(kind);
        let service = self.services.assistant.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime::spawn_blocking(move || {
                service.probe_connection(profile, kind, cancellation)
            })
            .await;
            this.update(cx, |this, cx| {
                this.assistant.finish_probe(probe_id, result);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn probe_assistant_readiness(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.probe_assistant_connection(ConnectionProbeKind::Readiness, cx);
    }

    pub(crate) fn probe_assistant_model(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.probe_assistant_connection(ConnectionProbeKind::Model, cx);
    }

    /// Repeats the probe that failed with a retryable error, on the form's
    /// current connection.
    pub(crate) fn retry_assistant_probe(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(kind) = self.assistant.probe_to_retry() {
            self.probe_assistant_connection(kind, cx);
        }
    }

    pub(crate) fn cancel_assistant_probe(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.assistant.request_probe_cancel() {
            self.ui.notice = i18n::text(self.ui.locale, "cancellation_requested").into();
            cx.notify();
        }
    }

    /// Whether a run's result, a decision on a review or an undo is still
    /// to come; the send button and Enter start no run then, so nothing the
    /// page shows is replaced before its outcome arrives.
    pub(crate) fn assistant_busy(&self) -> bool {
        self.assistant.awaiting_result() || self.assistant.decision_in_flight()
    }

    /// Whether the run can still be cancelled: it runs and no cancel was
    /// requested or reported.
    pub(crate) fn assistant_cancellable(&self) -> bool {
        matches!(self.assistant.task, AssistantTaskState::Running { .. })
    }

    /// Enter in the prompt sends it, as the send button does.
    fn submit_assistant_prompt(&mut self, cx: &mut Context<Self>) {
        let query = AssistantInputs::value(&self.assistant_inputs.prompt, cx);
        self.start_assistant_run(query, cx);
    }

    pub(crate) fn assistant_action(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let query = AssistantInputs::value(&self.assistant_inputs.prompt, cx);
        self.start_assistant_run(query, cx);
    }

    /// Sends the prompt of a run that failed with a retryable error again.
    pub(crate) fn retry_assistant_run(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(prompt) = self.assistant.retry_prompt() {
            self.start_assistant_run(prompt, cx);
        }
    }

    fn start_assistant_run(&mut self, query: String, cx: &mut Context<Self>) {
        if self.documents_root_switch.switching || self.assistant_busy() {
            return;
        }
        match self.assistant.scope_mode {
            AssistantScopeMode::CurrentDocument => {
                let selected = self
                    .ui
                    .selected_project
                    .as_ref()
                    .and_then(|id| self.projects.iter().find(|project| project.id() == id));
                self.assistant.authorize_document(selected);
            }
            AssistantScopeMode::AllVisibleDocuments => {
                self.assistant.authorize_documents(&self.projects);
            }
        }
        let (request, cancellation) =
            match self.assistant.begin(query, &self.settings_base.assistant) {
                Ok(started) => started,
                Err(error) => {
                    self.ui.notice = error.to_string();
                    cx.notify();
                    return;
                }
            };
        self.schedule_pending_expiry(cx);
        let request_id = request.request_id.clone();
        let service = self.services.assistant.clone();
        let (progress_sender, mut progress_receiver) =
            futures::channel::mpsc::unbounded::<crate::assistant_ui::AssistantProgress>();
        let progress_handler = Arc::new(move |progress| {
            let _ = progress_sender.unbounded_send(progress);
        });
        self.ui.notice = i18n::text(self.ui.locale, "assistant_running").into();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let run_future = runtime::spawn_blocking(move || {
                service.run_with_progress(request, cancellation, progress_handler)
            })
            .fuse();
            futures::pin_mut!(run_future);
            let result = loop {
                futures::select! {
                    result = run_future => break result,
                    progress = progress_receiver.next().fuse() => {
                        if let Some(progress) = progress {
                            let notice = format!(
                                "assistant {}{}",
                                progress.stage,
                                progress.tool_name.as_deref().map(|tool| format!(" · {tool}")).unwrap_or_default()
                            );
                            this.update(cx, |this, cx| {
                                this.assistant.record_progress(progress);
                                this.ui.notice = notice;
                                cx.notify();
                            }).ok();
                        }
                    }
                }
            };
            this.update(cx, |this, cx| {
                match result {
                    Ok(result) => {
                        let notice = result
                            .warning
                            .as_ref()
                            .map(ToString::to_string)
                            .unwrap_or_else(|| {
                                i18n::text(this.ui.locale, "assistant_completed").into()
                            });
                        this.assistant.complete(&request_id, result);
                        this.ui.notice = notice;
                    }
                    Err(error) => {
                        this.ui.notice = error.to_string();
                        this.assistant.fail(&request_id, error);
                    }
                }
                this.schedule_pending_expiry(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn cancel_assistant(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.assistant.request_cancel() {
            self.ui.notice = i18n::text(self.ui.locale, "cancellation_requested").into();
            cx.notify();
        }
    }

    /// Opens Settings at its Assistant section, where a connection, key or
    /// limit an Assistant error names is fixed.
    pub(crate) fn open_assistant_settings(&mut self, cx: &mut Context<Self>) {
        self.navigate(Route::Settings, cx);
        if let Some(section) = pages::settings::SECTIONS
            .iter()
            .position(|key| *key == "assistant_settings")
        {
            self.ui.settings_section = section;
            self.pending_settings_section = Some(section);
        }
        cx.notify();
    }

    pub(crate) fn select_assistant_scene(&mut self, scene: AgentScene, cx: &mut Context<Self>) {
        self.assistant.select_scene(scene);
        cx.notify();
    }

    pub(crate) fn select_assistant_scope(
        &mut self,
        scope: AssistantScopeMode,
        cx: &mut Context<Self>,
    ) {
        self.assistant.scope_mode = scope;
        match scope {
            AssistantScopeMode::CurrentDocument => {
                let selected = self
                    .ui
                    .selected_project
                    .as_ref()
                    .and_then(|id| self.projects.iter().find(|project| project.id() == id));
                self.assistant.authorize_document(selected);
            }
            AssistantScopeMode::AllVisibleDocuments => {
                self.assistant.authorize_documents(&self.projects);
            }
        }
        cx.notify();
    }

    pub(crate) fn accept_assistant_pending(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.documents_root_switch.switching {
            return;
        }
        let (Some(run), Some(pending)) = (
            self.assistant.result_run(),
            self.assistant.pending_for_accept(now_ms()),
        ) else {
            cx.notify();
            return;
        };
        // The decision buttons become disabled; keep keyboard focus in
        // their stable region until the user chooses somewhere else.
        if self.review_focus.contains_focused(window, cx) {
            self.review_focus.focus(window, cx);
        }
        let authorized_document_ids = self.assistant.result_authorized_document_ids();
        let service = self.services.assistant.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime::spawn_blocking(move || {
                service.confirm(pending.confirmation_id, authorized_document_ids)
            })
            .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(result) => this.assistant.confirmation_complete(&run, result),
                    Err(error) => this.assistant.confirmation_failed(&run, error),
                }
                this.review_focus_request = Some(ReviewFocus::Decision);
                this.schedule_pending_expiry(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn reject_assistant_pending(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(run), Some(pending)) = (
            self.assistant.result_run(),
            self.assistant.pending_for_reject(now_ms()),
        ) else {
            cx.notify();
            return;
        };
        if self.review_focus.contains_focused(window, cx) {
            self.review_focus.focus(window, cx);
        }
        let authorized_document_ids = self.assistant.result_authorized_document_ids();
        let service = self.services.assistant.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime::spawn_blocking(move || {
                service.reject(pending.confirmation_id, authorized_document_ids)
            })
            .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(()) => this.assistant.rejection_complete(&run),
                    Err(error) => this.assistant.confirmation_failed(&run, error),
                }
                this.review_focus_request = Some(ReviewFocus::Decision);
                this.schedule_pending_expiry(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Replaces the expiry timer with one for the awaiting review, if any, so
    /// expiry does not depend on a re-render.
    fn schedule_pending_expiry(&mut self, cx: &mut Context<Self>) {
        self.schedule_pending_expiry_at(now_ms(), cx);
    }

    pub(crate) fn schedule_pending_expiry_at(&mut self, now: u64, cx: &mut Context<Self>) {
        self.pending_expiry =
            self.assistant
                .pending_expiry_delay(now)
                .map(|(confirmation_id, delay)| {
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(delay).await;
                        this.update(cx, |this, cx| {
                            if this.assistant.expire_pending(&confirmation_id) {
                                this.ui.notice =
                                    i18n::text(this.ui.locale, "assistant_expired").into();
                                this.review_focus_request.get_or_insert(ReviewFocus::Expiry);
                                // The next queued review, if any, awaits now.
                                this.schedule_pending_expiry(cx);
                                cx.notify();
                            }
                        })
                        .ok();
                    })
                });
    }

    pub(crate) fn undo_assistant_mutation_for(
        &mut self,
        mutation_id: String,
        cx: &mut Context<Self>,
    ) {
        if self.documents_root_switch.switching {
            return;
        }
        let Some(run) = self.assistant.result_run() else {
            return;
        };
        let Some((token, mutation_id)) = self.assistant.begin_mutation_undo(&mutation_id) else {
            return;
        };
        let authorized_document_ids = self.assistant.result_authorized_document_ids();
        let service = self.services.assistant.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result =
                runtime::spawn_blocking(move || service.undo(token, authorized_document_ids)).await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(mutation) => {
                        this.assistant
                            .mutation_undo_complete(&run, &mutation_id, mutation)
                    }
                    Err(error) => this
                        .assistant
                        .mutation_undo_failed(&run, &mutation_id, error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// An accepted action owns the current workspace before its worker
    /// registers a shared job, and until its result has reached the UI.
    pub(crate) fn documents_root_busy(&self) -> bool {
        self.ui.processing || self.assistant_busy()
    }

    /// The Workspace section's button: asks for a folder and switches the
    /// documents root to it.
    pub(crate) fn choose_documents_root_directory(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.documents_root_switch.switching {
            return;
        }
        if self.documents_root_busy() {
            self.ui.notice = i18n::text(self.ui.locale, "documents_root_operation_pending").into();
            cx.notify();
            return;
        }
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(i18n::text(self.ui.locale, "choose_directory").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            let Some(directory) = paths.into_iter().next() else {
                return;
            };
            this.update(cx, |this, cx| this.choose_documents_root(directory, cx))
                .ok();
        })
        .detach();
    }

    /// Switches the workspace to the documents root `root` on a worker and
    /// saves it with the settings' revision check, queued behind the other
    /// settings writes. A refused root changes nothing; a save that fails
    /// after the switch switches the workspace back.
    pub(crate) fn choose_documents_root(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        if self.documents_root_switch.switching {
            return;
        }
        if self.documents_root_busy() {
            self.ui.notice = i18n::text(self.ui.locale, "documents_root_operation_pending").into();
            cx.notify();
            return;
        }
        self.documents_root_switch.switching = true;
        // The repository changes before the settings worker returns.
        // Keep the current projection until the switch settles, and let
        // the selected document start its read again afterwards.
        self.documents_repository_generation += 1;
        self.editor.loading.clear();
        self.begin_projects_reload();
        self.audio.invalidate_pending();
        self.documents_root_switch.status = None;
        self.ui.notice = i18n::text(self.ui.locale, "documents_root_switching").into();
        self.queue_settings_write(SettingsWrite::DocumentsRoot { root }, cx);
        cx.notify();
    }

    /// The Meetings header's relink button: asks for the selected
    /// document's audio file and relinks it.
    pub(crate) fn relink_audio(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.documents_root_switch.switching {
            return;
        }
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(i18n::text(self.ui.locale, "choose_audio").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            this.update(cx, |this, cx| this.relink_audio_path(path, cx))
                .ok();
        })
        .detach();
    }

    /// Points the selected document's recording (it and every document
    /// sharing its audio) at `path` on a worker, checked against the
    /// revision the tree read and the checksum processing recorded. A
    /// refused relink writes nothing; a revision conflict opens the
    /// conflict notice.
    pub(crate) fn relink_audio_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.documents_root_switch.switching {
            return;
        }
        if self.audio.relinking.is_some() {
            return;
        }
        let Some(document) = self
            .ui
            .selected_project
            .as_ref()
            .and_then(|id| self.projects.iter().find(|project| project.id() == id))
        else {
            self.ui.notice = i18n::text(self.ui.locale, "select_project_first").into();
            cx.notify();
            return;
        };
        let (document_id, revision) = (document.id().clone(), document.revision());
        let title = document.title().to_string();
        self.audio.relinking = Some(document_id.clone());
        self.ui.notice = i18n::text(self.ui.locale, "relinking_audio").into();
        cx.notify();
        let root_generation = self.documents_root_generation;
        let documents = self.services.workspace.documents().frozen();
        let path = path.display().to_string();
        cx.spawn(async move |this, cx| {
            let result = runtime::spawn_blocking(move || {
                documents.relink_audio(&document_id, revision, path)
            })
            .await;
            this.update(cx, |this, cx| {
                if root_generation != this.documents_root_generation {
                    cx.notify();
                    return;
                }
                this.audio.relinking = None;
                let locale = this.ui.locale;
                match result {
                    Ok(_) => {
                        this.ui.notice = i18n::message(locale, "audio_relinked", &title);
                        this.reload_projects(cx);
                    }
                    Err(error) => {
                        this.ui.set_notice(
                            NoticeTone::Error,
                            i18n::audio_source_error(locale, &error),
                        );
                        if let AudioSourceError::RevisionConflict {
                            document_id,
                            expected,
                            actual,
                        } = error
                        {
                            let title = this
                                .projects
                                .iter()
                                .find(|project| project.id() == &document_id)
                                .map_or_else(
                                    || document_id.to_string(),
                                    |project| project.title().into(),
                                );
                            this.document_conflict = Some(DocumentConflict {
                                document_id,
                                title,
                                expected,
                                actual,
                            });
                            this.reload_projects(cx);
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A key step in the project tree: selects a row, or expands or
    /// collapses the selected one.
    pub(crate) fn step_project_tree(
        &mut self,
        step: TreeStep,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected = self.ui.selected_project.as_ref();
        match crate::components::tree_move(
            &self.projects,
            &self.ui.expanded_projects,
            selected,
            step,
        ) {
            Some(TreeMove::Select(id)) => self.select_project(id, cx),
            Some(TreeMove::Expand(id)) | Some(TreeMove::Collapse(id)) => {
                self.toggle_project(id, cx)
            }
            None => {}
        }
    }

    pub(crate) fn toggle_project(&mut self, project_id: ProjectId, cx: &mut Context<Self>) {
        if !self.ui.expanded_projects.insert(project_id.clone()) {
            self.ui.expanded_projects.remove(&project_id);
        }
        cx.notify();
    }

    pub(crate) fn toggle_model_family(&mut self, family: String, cx: &mut Context<Self>) {
        if !self.ui.expanded_model_families.insert(family.clone()) {
            self.ui.expanded_model_families.remove(&family);
        }
        cx.notify();
    }

    /// Shows or hides a model card's checksums and probe results.
    pub(crate) fn toggle_model_details(&mut self, model_id: String, cx: &mut Context<Self>) {
        if !self.ui.expanded_model_details.insert(model_id.clone()) {
            self.ui.expanded_model_details.remove(&model_id);
        }
        cx.notify();
    }

    pub(crate) fn select_asr_backend(&mut self, backend_id: String, cx: &mut Context<Self>) {
        self.ui.notice = i18n::message(
            self.ui.locale,
            "backend_selected",
            &format!("ASR {backend_id}"),
        );
        self.persist_field(
            move |settings| {
                if settings
                    .asr_model
                    .as_ref()
                    .is_some_and(|model| model.backend_id != backend_id)
                {
                    settings.asr_model = None;
                }
                settings.asr_backend_id = Some(backend_id.clone());
            },
            cx,
        );
    }

    pub(crate) fn select_diarization_backend(
        &mut self,
        backend_id: String,
        cx: &mut Context<Self>,
    ) {
        self.ui.notice = i18n::message(
            self.ui.locale,
            "backend_selected",
            &format!("Diarization {backend_id}"),
        );
        self.persist_field(
            move |settings| {
                if settings
                    .diarization_model
                    .as_ref()
                    .is_some_and(|model| model.backend_id != backend_id)
                {
                    settings.diarization_model = None;
                }
                settings.diarization_backend_id = Some(backend_id.clone());
                settings.diarization_stages = Default::default();
            },
            cx,
        );
    }

    pub(crate) fn use_model(
        &mut self,
        model_id: String,
        task: String,
        backend_id: String,
        cx: &mut Context<Self>,
    ) {
        let models = self.services.model_manager.clone();
        cx.spawn(async move |this, cx| {
            let (model_id_for_ref, backend_id_for_ref) = (model_id.clone(), backend_id.clone());
            let model = runtime::spawn_blocking(move || {
                models.model_ref(&model_id_for_ref, &backend_id_for_ref)
            })
            .await;
            this.update(cx, |this, cx| {
                match model {
                    Ok(model) => {
                        this.ui.notice = i18n::message(this.ui.locale, "model_selected", &model_id);
                        this.persist_field(
                            move |settings| {
                                if task == "asr" {
                                    settings.asr_backend_id = Some(backend_id.clone());
                                    settings.asr_model = Some(model.clone());
                                } else {
                                    settings.diarization_backend_id = Some(backend_id.clone());
                                    settings.diarization_model = Some(model.clone());
                                    settings.diarization_stages = Default::default();
                                }
                            },
                            cx,
                        );
                    }
                    Err(error) => {
                        this.ui.notice =
                            i18n::message(this.ui.locale, "model_not_ready", &error.to_string())
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Starts the download off the main thread and follows its progress.
    pub(crate) fn download_model(&mut self, model_id: String, cx: &mut Context<Self>) {
        let models = self.services.model_manager.clone();
        cx.spawn(async move |this, cx| {
            let id = model_id.clone();
            let started = runtime::spawn_blocking(move || models.start(&id)).await;
            this.update(cx, |this, cx| {
                match started {
                    Ok(task) => {
                        this.ui.notice =
                            i18n::message(this.ui.locale, "download_started", &model_id);
                        this.follow_download(task.updates, cx);
                    }
                    Err(error) => {
                        this.ui.notice = i18n::message(
                            this.ui.locale,
                            "download_failed",
                            &format!("{model_id}: {error}"),
                        );
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Re-probes an installed model: `reprobe` runs off the main thread and
    /// the manager probes on a thread of its own. The page shows it running
    /// until the installation service, which republishes when a re-probe
    /// ends, publishes the state that carries the written record.
    pub(crate) fn reprobe_model(&mut self, model_id: String, cx: &mut Context<Self>) {
        if self.model_reprobes.contains_key(&model_id) {
            return;
        }
        self.model_reprobes
            .insert(model_id.clone(), ReprobeWait::Running);
        self.ui.notice = i18n::message(self.ui.locale, "reprobing_model", &model_id);
        cx.notify();
        let models = self.services.model_manager.clone();
        cx.spawn(async move |this, cx| {
            let id = model_id.clone();
            let finished = match runtime::spawn_blocking(move || models.reprobe(&id)).await {
                Ok(task) => match task.finished.await {
                    Ok(written) => written.map_err(|error| error.to_string()),
                    Err(canceled) => Err(canceled.to_string()),
                },
                Err(error) => Err(error.to_string()),
            };
            this.update(cx, |this, cx| {
                match finished {
                    Ok(record) => {
                        this.ui.notice = i18n::message(this.ui.locale, "reprobed", &model_id);
                        this.model_reprobes.insert(
                            model_id,
                            ReprobeWait::Written {
                                probed_at_ms: record.probed_at_ms,
                            },
                        );
                        // The state may have arrived first.
                        this.settle_reprobes();
                    }
                    Err(error) => {
                        this.model_reprobes.remove(&model_id);
                        this.ui.notice = i18n::message(
                            this.ui.locale,
                            "reprobe_failed",
                            &format!("{model_id}: {error}"),
                        );
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Shows a download's progress, pushed by the download thread, until it
    /// ends.
    pub(crate) fn follow_download(
        &mut self,
        mut updates: futures::channel::mpsc::UnboundedReceiver<DownloadEvent>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            while let Some(DownloadEvent::Updated(progress)) = updates.next().await {
                let terminal = progress.state.is_terminal();
                let applied =
                    this.update(cx, |this, cx| this.apply_download_progress(progress, cx));
                if applied.is_err() || terminal {
                    break;
                }
            }
        })
        .detach();
    }

    fn apply_download_progress(
        &mut self,
        progress: models::manager::DownloadProgress,
        cx: &mut Context<Self>,
    ) {
        self.ui.notice = match &progress.state {
            DownloadState::Completed => {
                i18n::message(self.ui.locale, "installed", &progress.model_id)
            }
            DownloadState::Cancelled => {
                i18n::message(self.ui.locale, "cancelled", &progress.model_id)
            }
            DownloadState::Failed(error) => i18n::message(self.ui.locale, "download_failed", error),
            _ => i18n::download_progress(
                self.ui.locale,
                &progress.model_id,
                progress.artifact_index,
                progress.artifact_count,
                progress.completed_bytes,
                progress.speed_bytes_per_second,
            ),
        };
        // The installation service re-probes when a download ends.
        cx.notify();
    }

    pub(crate) fn control_download(
        &mut self,
        id: models::manager::DownloadId,
        action: DownloadAction,
        cx: &mut Context<Self>,
    ) {
        let models = self.services.model_manager.clone();
        cx.spawn(async move |this, cx| {
            // Each control persists the download tasks.
            let result = runtime::spawn_blocking(move || match action {
                DownloadAction::Pause => models.pause(id),
                DownloadAction::Resume => models.resume(id),
                DownloadAction::Cancel => models.cancel(id),
            })
            .await;
            this.update(cx, |this, cx| {
                this.ui.notice = match result {
                    Ok(()) => i18n::text(this.ui.locale, "download_updated").into(),
                    Err(error) => {
                        i18n::message(this.ui.locale, "download_failed", &error.to_string())
                    }
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn process_or_cancel(
        &mut self,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ui.processing {
            if self
                .ui
                .selected_project
                .as_ref()
                .is_some_and(|id| self.ui.is_processing_document(id))
            {
                self.cancel_processing(event, window, cx);
            }
        } else {
            self.process_selected(event, window, cx);
        }
    }

    pub(crate) fn cancel_processing(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((asr_context, diarization_context)) = &self.cancel_contexts {
            asr_context.cancel();
            diarization_context.cancel();
        } else if self.ui.processing {
            // Still starting: the job is cancelled once the start returns.
            self.processing_cancel_requested = true;
        } else {
            return;
        }
        self.ui.notice = i18n::text(self.ui.locale, "cancellation_requested").into();
        cx.notify();
    }

    pub(crate) fn process_selected(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.documents_root_switch.switching {
            return;
        }
        let Some(project_id) = self.ui.selected_project.clone() else {
            self.ui.notice = i18n::text(self.ui.locale, "select_project_first").into();
            cx.notify();
            return;
        };
        if self.ui.processing {
            return;
        }
        if self.audio.missing(&project_id) {
            self.ui.notice = i18n::text(self.ui.locale, "audio_missing_cannot_process").into();
            cx.notify();
            return;
        }
        let Some(project) = self
            .projects
            .iter()
            .find(|project| project.id() == &project_id)
            .cloned()
        else {
            self.ui.notice = i18n::text(self.ui.locale, "project_missing").into();
            cx.notify();
            return;
        };
        if crate::state::missing_models(
            &self.installation,
            &self.settings_draft,
            (&self.services.asr_model, &self.services.diarization_model),
        )
        .any()
        {
            return;
        }
        self.ui.begin_processing(project_id.clone());
        self.processing_cancel_requested = false;
        self.ui.notice = i18n::message(self.ui.locale, "processing", project.title());
        let workspace = self.services.workspace.clone();
        let settings = self.settings_draft.clone();
        let options = processing_options(&settings);
        let asr_model = match (settings.asr_backend_id, settings.asr_model) {
            (Some(_), Some(model)) => model,
            (Some(backend_id), None) => {
                self.ui.notice = i18n::message(self.ui.locale, "asr_model_required", &backend_id);
                self.ui.finish_processing();
                cx.notify();
                return;
            }
            (None, None) => self.services.asr_model.clone(),
            (None, Some(model)) => model,
        };
        let diarization_model = match (settings.diarization_backend_id, settings.diarization_model)
        {
            (Some(_), Some(model)) => model,
            (Some(backend_id), None) => {
                self.ui.notice =
                    i18n::message(self.ui.locale, "diarization_model_required", &backend_id);
                self.ui.finish_processing();
                cx.notify();
                return;
            }
            (None, None) => self.services.diarization_model.clone(),
            (None, Some(model)) => model,
        };
        let speaker_library_path = settings.speaker_library_path;
        let title = project.title().to_string();
        cx.notify();
        cx.spawn(async move |this, cx| {
            // Starting reads the document and probes the models.
            let started = runtime::spawn_blocking(move || {
                workspace.set_processing_options(options)?;
                workspace
                    .start_project_processing_with_speaker_library(
                        project_id,
                        asr_model,
                        diarization_model,
                        Some(speaker_library_path),
                    )
                    .map_err(|error| error.message)
            })
            .await;
            let job = match started {
                Ok(job) => job,
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.ui.finish_processing();
                        this.processing_cancel_requested = false;
                        this.ui.notice = i18n::message(this.ui.locale, "processing_failed", &error);
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            let contexts = (job.asr_context.clone(), job.diarization_context.clone());
            let follow = this.update(cx, |this, cx| {
                cx.notify();
                if std::mem::take(&mut this.processing_cancel_requested) {
                    this.ui.finish_processing();
                    this.ui.notice = i18n::message(this.ui.locale, "cancelled", &title);
                    return false;
                }
                this.cancel_contexts = Some(contexts);
                true
            });
            if !matches!(follow, Ok(true)) {
                // Cancelled while it started, or the window closed: both
                // jobs end cancelled and the unpolled job, dropped here,
                // writes nothing.
                job.cancel();
                return;
            }
            Self::follow_processing(this, job, cx).await;
        })
        .detach();
    }

    /// Shows a started job's events until it finishes, then reloads the
    /// project list.
    pub(crate) async fn follow_processing(
        this: gpui::WeakEntity<Self>,
        job: application::ProjectProcessingJob,
        cx: &mut gpui::AsyncApp,
    ) {
        let mut asr_events = job.asr_events.fuse();
        let mut diarization_events = job.diarization_events.fuse();
        let mut processing_future = job.future.fuse();
        loop {
            let mut asr_next = asr_events.next().fuse();
            let mut diarization_next = diarization_events.next().fuse();
            futures::select! {
                event = asr_next => {
                    if let Some(event) = event {
                        this.update(cx, |this, cx| this.apply_job_event(ProcessingStage::Transcription, event, cx)).ok();
                    }
                }
                event = diarization_next => {
                    if let Some(event) = event {
                        this.update(cx, |this, cx| this.apply_job_event(ProcessingStage::Diarization, event, cx)).ok();
                    }
                }
                result = processing_future => {
                    this.update(cx, |this, cx| {
                        let title = this.ui.processing_project.as_ref().map(|id| {
                            this.projects.iter().find(|project| project.id() == id)
                                .map(|project| project.title()).unwrap_or(id.as_str())
                        }).unwrap_or_default().to_string();
                        this.ui.finish_processing();
                        this.cancel_contexts = None;
                        this.reload_projects(cx);
                        match result {
                            Ok(project) => {
                                this.ui.notice = i18n::message(this.ui.locale, "finished", project.title());
                            }
                            Err(error) if error.code == contracts::BackendErrorCode::Cancelled => {
                                this.ui.set_notice(NoticeTone::Info, i18n::message(this.ui.locale, "cancelled", &title));
                            }
                            Err(error) => {
                                this.ui.notice = i18n::message(this.ui.locale, "processing_failed", &error.message)
                            }
                        }
                        cx.notify();
                    }).ok();
                    break;
                }
            }
        }
    }

    /// Reloads the project list whenever the document directory changes.
    pub(crate) fn watch_documents(&mut self, cx: &mut Context<Self>) {
        if let Ok(changes) = self.services.workspace.subscribe_document_changes() {
            self.follow_document_changes(changes, cx);
        }
    }

    /// Follows the watcher's change stream; a burst of changes that is
    /// already queued triggers one reload.
    pub(crate) fn follow_document_changes(
        &mut self,
        mut changes: impl futures::Stream<Item = application::DocumentChange> + Unpin + 'static,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            while changes.next().await.is_some() {
                while let Some(Some(_)) = changes.next().now_or_never() {}
                let refreshed = this.update(cx, |view, cx| {
                    view.reload_projects(cx);
                    view.refresh_installation();
                });
                if refreshed.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn apply_job_event(&mut self, stage: ProcessingStage, event: JobEvent, cx: &mut Context<Self>) {
        if self.ui.processing {
            self.ui.processing_feedback.apply(stage, event);
            cx.notify();
        }
    }
}

/// What processing reads from the settings; the workspace also checks a
/// documents root against its Speaker Library path.
fn processing_options(settings: &AppSettings) -> ProcessingOptions {
    ProcessingOptions {
        language: settings.language.clone(),
        expected_speakers: settings.expected_speakers,
        device: settings.compute_device,
        speaker_library_path: Some(settings.speaker_library_path.clone()),
        diarization_stages: settings.diarization_stages.clone(),
    }
}

fn auto_label(locale: i18n::Locale) -> &'static str {
    i18n::text(locale, "auto")
}

fn transcription_language_items(locale: i18n::Locale) -> Vec<String> {
    vec![auto_label(locale).into(), "English".into(), "中文".into()]
}

fn transcription_language_label(locale: i18n::Locale, language: Option<&str>) -> &'static str {
    match language {
        Some("en") | Some("en-US") => "English",
        Some("zh") | Some("zh-CN") => "中文",
        _ => auto_label(locale),
    }
}

fn expected_speaker_items(locale: i18n::Locale) -> Vec<String> {
    std::iter::once(auto_label(locale).to_string())
        .chain((1..=4).map(|count: u32| count.to_string()))
        .collect()
}

const COMPUTE_DEVICES: [ComputeDevice; 4] = [
    ComputeDevice::Auto,
    ComputeDevice::Cpu,
    ComputeDevice::Metal,
    ComputeDevice::CoreMl,
];

/// Labels come from `i18n::device`, the same function the selection uses.
fn compute_device_items(locale: i18n::Locale) -> Vec<String> {
    COMPUTE_DEVICES
        .iter()
        .map(|device| i18n::device(locale, *device).to_string())
        .collect()
}

fn compute_device_from_label(locale: i18n::Locale, label: &str) -> ComputeDevice {
    COMPUTE_DEVICES
        .into_iter()
        .find(|device| i18n::device(locale, *device) == label)
        .unwrap_or(ComputeDevice::Auto)
}

const THEME_PREFERENCES: [ThemePreference; 3] = [
    ThemePreference::System,
    ThemePreference::Light,
    ThemePreference::Dark,
];

const MATERIAL_PREFERENCES: [MaterialPreference; 2] =
    [MaterialPreference::Glass, MaterialPreference::Opaque];

fn theme_preference_key(preference: ThemePreference) -> &'static str {
    match preference {
        ThemePreference::System => "theme_system",
        ThemePreference::Light => "theme_light",
        ThemePreference::Dark => "theme_dark",
    }
}

fn material_preference_key(preference: MaterialPreference) -> &'static str {
    match preference {
        MaterialPreference::Glass => "material_glass",
        MaterialPreference::Opaque => "material_opaque",
    }
}

fn theme_preference_items(locale: i18n::Locale) -> Vec<String> {
    THEME_PREFERENCES
        .iter()
        .map(|preference| i18n::text(locale, theme_preference_key(*preference)).to_string())
        .collect()
}

fn material_preference_items(locale: i18n::Locale) -> Vec<String> {
    MATERIAL_PREFERENCES
        .iter()
        .map(|preference| i18n::text(locale, material_preference_key(*preference)).to_string())
        .collect()
}

fn theme_preference_from_label(locale: i18n::Locale, label: &str) -> Option<ThemePreference> {
    THEME_PREFERENCES
        .into_iter()
        .find(|preference| i18n::text(locale, theme_preference_key(*preference)) == label)
}

fn material_preference_from_label(locale: i18n::Locale, label: &str) -> Option<MaterialPreference> {
    MATERIAL_PREFERENCES
        .into_iter()
        .find(|preference| i18n::text(locale, material_preference_key(*preference)) == label)
}

fn changed<T: PartialEq + Clone>(before: &T, after: &T) -> Option<T> {
    (before != after).then(|| after.clone())
}

/// A typed key replaces the stored credential and an edited key file
/// replaces it; clearing the key file clears it. Anything else keeps it.
fn credential_update(
    api_key: &str,
    api_key_file: &str,
    stored: Option<&AssistantConnectionProfile>,
) -> Option<CredentialUpdate> {
    let (api_key, api_key_file) = (api_key.trim(), api_key_file.trim());
    let stored_file = stored
        .and_then(|profile| profile.api_key_file.as_deref())
        .unwrap_or("");
    if !api_key.is_empty() {
        Some(CredentialUpdate::Direct(api_key.into()))
    } else if api_key_file == stored_file {
        None
    } else if api_key_file.is_empty() {
        Some(CredentialUpdate::Clear)
    } else {
        Some(CredentialUpdate::File(api_key_file.into()))
    }
}

fn connection_patch(
    stored: &AssistantConnectionProfile,
    profile: &AssistantConnectionProfile,
    credential: Option<CredentialUpdate>,
) -> AssistantConnectionPatch {
    AssistantConnectionPatch {
        id: profile.id.clone(),
        name: changed(&stored.name, &profile.name),
        protocol: changed(&stored.protocol, &profile.protocol),
        base_url: changed(&stored.base_url, &profile.base_url),
        model: changed(&stored.model, &profile.model),
        auth_mode: changed(&stored.auth_mode, &profile.auth_mode),
        context_window_tokens: changed(
            &stored.context_window_tokens,
            &profile.context_window_tokens,
        ),
        timeout_ms: changed(&stored.timeout_ms, &profile.timeout_ms),
        max_output_tokens: changed(&stored.max_output_tokens, &profile.max_output_tokens),
        credential,
    }
}

pub(crate) fn settings_error_text(locale: i18n::Locale, error: &AssistantUiError) -> String {
    if error.is_settings_conflict() {
        i18n::text(locale, "settings_conflict").into()
    } else if error.is_credential_target_conflict() {
        i18n::text(locale, "assistant_credential_target_conflict").into()
    } else {
        i18n::message(locale, "settings_failed", &error.to_string())
    }
}

/// Options shown as their values.
fn text_options(items: Vec<String>) -> Vec<SelectOption> {
    items.into_iter().map(SelectOption::text).collect()
}

fn parse_u32(value: &str, field: &str) -> Result<u32, AssistantUiError> {
    value.trim().parse().map_err(|_| AssistantUiError {
        code: "ASSISTANT_SETTINGS_INVALID".into(),
        message: format!("{field} must be an unsigned integer"),
        retryable: false,
    })
}

fn parse_optional_u32(value: &str, field: &str) -> Result<Option<u32>, AssistantUiError> {
    let value = value.trim();
    if value.is_empty() {
        Ok(None)
    } else {
        parse_u32(value, field).map(Some)
    }
}

fn parse_u64(value: &str, field: &str) -> Result<u64, AssistantUiError> {
    value.trim().parse().map_err(|_| AssistantUiError {
        code: "ASSISTANT_SETTINGS_INVALID".into(),
        message: format!("{field} must be an unsigned integer"),
        retryable: false,
    })
}

/// A stage select shows its saved model, or the first option until one is
/// saved.
fn stage_selection<'a>(options: &'a [SelectOption], saved: Option<&'a str>) -> Option<&'a str> {
    saved
        .filter(|saved| SelectOption::position(options, Some(saved)).is_some())
        .or_else(|| options.first().map(SelectOption::value))
}

/// The toast of the latest notice. Its timers end only the toast they
/// were started for, so a newer notice needs none of them cancelled.
impl RootView {
    /// Takes up a notice set since the last update: a toast that opens
    /// fades after `TOAST_LIFETIME` unless its tone keeps it, and a cleared
    /// notice closes its toast.
    fn take_up_notice(&mut self, cx: &mut Context<Self>) {
        match self.ui.take_up_notice() {
            Some(NoticeChange::Opened { generation, tone }) if tone.fades() => {
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(TOAST_LIFETIME).await;
                    this.update(cx, |this, cx| this.dismiss_toast(generation, cx))
                        .ok();
                })
                .detach();
            }
            Some(NoticeChange::Cleared(generation)) => self.dismiss_toast(generation, cx),
            _ => {}
        }
    }

    /// Fades out the toast of `generation`, then removes it and clears its
    /// notice.
    pub(crate) fn dismiss_toast(&mut self, generation: u64, cx: &mut Context<Self>) {
        if !self.ui.toast_leave(generation) {
            return;
        }
        let fade = theme::tokens(cx).motion.menu;
        if fade.is_zero() {
            self.ui.toast_remove(generation);
        } else {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(fade).await;
                this.update(cx, |this, cx| {
                    this.ui.toast_remove(generation);
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        cx.notify();
    }
}

impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(section) = self.pending_settings_section.take() {
            if self.ui.route == Route::Settings && self.ui.settings_section == section {
                // Defer from render so this Settings layout publishes its
                // viewport before the section scroll is requested.
                cx.defer_in(window, move |this, _, cx| {
                    if this.ui.route == Route::Settings && this.ui.settings_section == section {
                        this.scroll.settings.scroll_to_top_of_item(section);
                        cx.notify();
                    }
                });
            }
        }
        self.focus_awaiting_review(window, cx);
        self.ensure_settings_selectors(window, cx);
        self.ensure_document_editor(cx);
        let selected = self
            .ui
            .selected_project
            .as_ref()
            .and_then(|id| self.projects.iter().find(|project| project.id() == id))
            .cloned();
        self.ensure_speaker_rows(selected.as_ref(), window, cx);
        self.ensure_profile_rows(window, cx);
        let body = match self.ui.route {
            Route::Assistant => pages::assistant::render(self, cx).into_any_element(),
            Route::Meetings => {
                pages::meetings::render(self, selected.as_ref(), cx).into_any_element()
            }
            Route::Models => pages::models::render(self, cx).into_any_element(),
            Route::Settings => pages::settings::render(self, cx).into_any_element(),
        };
        let detail = (self.ui.detail_open && self.ui.route == Route::Meetings).then(|| {
            pages::detail::render(
                selected.as_ref(),
                &self.projects,
                selected
                    .as_ref()
                    .and_then(|project| self.audio.result_for(project.id())),
                self.ui.locale,
                &self.scroll.detail,
                &self.detail_focus,
                cx,
            )
            .into_any_element()
        });
        let shell = pages::chrome::shell(self, body, detail, window, cx)
            .children(self.speaker_library_dialog(cx))
            .children(self.speaker_confirmation_dialog(cx));
        let focus = self.window_focus.clone();
        controls::window::root(shell, &focus, cx.listener(Self::run_shortcut), window, cx)
    }
}

impl RootView {
    /// Moves focus to the awaiting review when a decision or an expiry
    /// brought it up, after this frame, on the Assistant page only.
    fn focus_awaiting_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(request) = self.review_focus_request.take() else {
            return;
        };
        let awaiting = matches!(
            self.assistant.pending,
            Some(crate::assistant_ui::PendingReviewState::Awaiting(_))
        );
        if !awaiting || self.ui.route != Route::Assistant {
            return;
        }
        let take = match request {
            ReviewFocus::Decision => self.review_focus.contains_focused(window, cx),
            ReviewFocus::Expiry => {
                self.window_focus.is_focused(window)
                    || window.focused(cx).is_none()
                    || self.review_focus.contains_focused(window, cx)
            }
        };
        if take {
            let focus = self.review_focus.clone();
            let previous = window.focused(cx);
            window.defer(cx, move |window, cx| {
                if window.focused(cx) == previous {
                    focus.focus(window, cx);
                }
            });
        }
    }
}

pub fn run(inventory: BackendInventory, services: DesktopServices) {
    application()
        .with_assets(gpui_component_assets::Assets)
        .run(move |cx: &mut App| {
            init_app(theme::system_accessibility(), cx);
            let bounds = Bounds::centered(None, gpui::size(gpui::px(1_180.), gpui::px(760.)), cx);
            open_main_window(
                inventory.clone(),
                services.clone(),
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                cx,
            )
            .expect("failed to open meetnote window");
            cx.activate(true);
        });
}

/// The app-wide setup `run` performs before it opens the window; the route
/// tests under `crates/ui/tests` perform the same setup with fixed
/// accessibility flags, where `run` reads the system's.
pub(crate) fn init_app(accessibility: theme::Accessibility, cx: &mut App) {
    controls::window::init(cx);
    theme::init(accessibility, cx);
}

/// Opens the desktop window in the saved appearance and starts following
/// document changes; `run` and the GPUI tests build the window the same way.
/// `options` supplies placement and visibility; this function owns the
/// titlebar (the title, a transparent titlebar with content under it, and
/// dragging through the shell's own strips) and the least size the shell
/// fits. The first layer is the root view itself, rendered through
/// `controls::window::root`: it paints the window's base fill and cycles Tab
/// inside gpui-base focus traps such as the confirm dialog's.
pub(crate) fn open_main_window(
    inventory: BackendInventory,
    services: DesktopServices,
    options: WindowOptions,
    cx: &mut App,
) -> gpui::Result<(gpui::AnyWindowHandle, Entity<RootView>)> {
    let saved = services.settings.snapshot().unwrap_or_default();
    let options = WindowOptions {
        titlebar: Some(pages::chrome::titlebar_options()),
        app_owns_titlebar_drag: true,
        window_min_size: Some(pages::chrome::MIN_WINDOW_SIZE),
        ..options
    };
    let mut root_view = None;
    let window = cx.open_window(options, |window, cx| {
        // The titlebar title reaches AppKit at creation; setting it here
        // also labels the accessibility root and is what gpui's test
        // window records.
        window.set_window_title(pages::chrome::WINDOW_TITLE);
        theme::attach(saved.appearance, window, cx);
        controls::window::attach(window, cx);
        let view = cx.new(|cx| RootView::new(inventory, services, window, cx));
        view.update(cx, |view, cx| view.watch_documents(cx));
        root_view = Some(view.clone());
        view
    })?;
    let view = root_view.expect("the window builds its root view");
    Ok((window.into(), view))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_options_are_selected_and_labelled_in_both_locales() {
        let first = Some(0);
        let selected_index =
            |items: &[String], value| SelectOption::position(&text_options(items.to_vec()), value);
        for (locale, auto) in [(i18n::Locale::ZhCn, "自动"), (i18n::Locale::EnUs, "Auto")] {
            let devices = compute_device_items(locale);
            assert_eq!(
                selected_index(&devices, Some(i18n::device(locale, ComputeDevice::Auto))),
                first,
                "{locale:?} device select must mark Auto"
            );
            assert_eq!(devices[0], auto, "{locale:?} device label");
            let languages = transcription_language_items(locale);
            assert_eq!(
                selected_index(&languages, Some(transcription_language_label(locale, None))),
                first,
                "{locale:?} transcription language select must mark Auto"
            );
            assert_eq!(
                languages[0], auto,
                "{locale:?} transcription language label"
            );
            let speakers = expected_speaker_items(locale);
            assert_eq!(
                selected_index(&speakers, Some(auto_label(locale))),
                first,
                "{locale:?} expected speakers select must mark Auto"
            );
            assert_eq!(speakers[0], auto, "{locale:?} expected speakers label");
        }
    }

    #[test]
    fn appearance_labels_map_back_to_their_preferences_in_both_locales() {
        for locale in [i18n::Locale::ZhCn, i18n::Locale::EnUs] {
            let themes = theme_preference_items(locale);
            assert_eq!(themes.len(), THEME_PREFERENCES.len());
            for (label, preference) in themes.iter().zip(THEME_PREFERENCES) {
                assert_eq!(theme_preference_from_label(locale, label), Some(preference));
            }
            let materials = material_preference_items(locale);
            assert_eq!(materials.len(), MATERIAL_PREFERENCES.len());
            for (label, preference) in materials.iter().zip(MATERIAL_PREFERENCES) {
                assert_eq!(
                    material_preference_from_label(locale, label),
                    Some(preference)
                );
            }
        }
        assert_eq!(
            theme_preference_items(i18n::Locale::ZhCn),
            ["跟随系统", "浅色", "深色"]
        );
        assert_eq!(
            material_preference_items(i18n::Locale::EnUs),
            ["Glass", "Opaque"]
        );
        assert_eq!(
            theme_preference_from_label(i18n::Locale::ZhCn, "Dark"),
            None
        );
    }

    #[test]
    fn install_status_labels_follow_the_locale() {
        let option = |installed, locale| ModelOption {
            id: "model".into(),
            installed,
            locale,
        };
        assert_eq!(option(true, i18n::Locale::ZhCn).status_label(), "已下载");
        assert_eq!(option(false, i18n::Locale::ZhCn).status_label(), "未下载");
        assert_eq!(
            option(true, i18n::Locale::EnUs).status_label(),
            "Downloaded"
        );
        assert_eq!(
            StageModelOption {
                id: "stage".into(),
                installed: false,
                locale: i18n::Locale::EnUs,
            }
            .status_label(),
            "Not downloaded"
        );
    }
}
