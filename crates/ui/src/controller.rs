use crate::{
    i18n, pages,
    route::Route,
    state::{ContentMode, Overlay, UiState},
    text_input::TextInput,
};
use application::{BackendInventory, DocumentCommandService, ProcessingOptions, WorkspaceService};
use contracts::{
    DiarizationStageKind, DiarizationStageOption, DocumentCommand, DocumentCommandEnvelope,
    DocumentCommandResult, ModelRef,
};
use domain::{DocumentState, ProjectId};
use futures::{FutureExt, StreamExt};
use gpui::{
    prelude::*, App, Bounds, ClickEvent, Context, Entity, KeyBinding, PathPromptOptions,
    SharedString, Window, WindowBounds, WindowOptions,
};
use gpui_platform::application;
use models::manager::{DownloadEvent, DownloadState};
use models::{ModelAvailability, ModelInstallState};
use runtime::{JobContext, JobEvent};
use settings::{AppSettings, SettingsService};
use std::path::Path;
use std::sync::{Arc, Mutex};

use gpui_component::{
    dialog::DialogButtonProps,
    select::{SelectEvent, SelectItem, SelectState},
    IndexPath, WindowExt,
};

type StringSelect = SelectState<Vec<String>>;
type ModelSelect = SelectState<Vec<ModelOption>>;
type StageModelSelect = SelectState<Vec<StageModelOption>>;

#[derive(Debug, Clone)]
pub(crate) struct ModelOption {
    id: String,
    installed: bool,
}

impl ModelOption {
    fn from_availability(availability: &ModelAvailability) -> Self {
        Self {
            id: availability.model.id.clone(),
            installed: availability.install_state == ModelInstallState::Installed,
        }
    }
}

impl SelectItem for ModelOption {
    type Value = String;

    fn title(&self) -> SharedString {
        self.id.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        gpui::div()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(self.id.clone())
            .child(if self.installed {
                "已下载"
            } else {
                "未下载"
            })
    }

    fn disabled(&self) -> bool {
        !self.installed
    }
}

#[derive(Debug, Clone)]
pub(crate) struct StageModelOption {
    id: String,
    installed: bool,
}

impl SelectItem for StageModelOption {
    type Value = String;

    fn title(&self) -> SharedString {
        self.id.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        gpui::div()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(self.id.clone())
            .child(if self.installed {
                "已下载"
            } else {
                "未下载"
            })
    }

    fn disabled(&self) -> bool {
        !self.installed
    }
}

pub(crate) struct SettingsSelectors {
    pub(crate) language: Entity<StringSelect>,
    pub(crate) asr_backend: Entity<StringSelect>,
    pub(crate) asr_model: Entity<ModelSelect>,
    pub(crate) transcription_language: Entity<StringSelect>,
    pub(crate) diarization_backend: Entity<StringSelect>,
    pub(crate) diarization_model: Entity<ModelSelect>,
    pub(crate) diarization_segmentation_model: Entity<StageModelSelect>,
    pub(crate) diarization_embedding_model: Entity<StageModelSelect>,
    pub(crate) diarization_clustering: Entity<StringSelect>,
    pub(crate) expected_speakers: Entity<StringSelect>,
    pub(crate) compute_device: Entity<StringSelect>,
}

#[derive(Clone)]
pub struct DesktopServices {
    pub workspace: WorkspaceService,
    pub document_commands: DocumentCommandService,
    pub settings: SettingsService,
    pub model_manager: models::manager::ModelManager,
    pub asr_model: ModelRef,
    pub diarization_model: ModelRef,
}

impl DesktopServices {
    pub(crate) fn model_status(&self, locale: i18n::Locale) -> String {
        let asr_ready = Path::new(&self.asr_model.artifact_path).is_file();
        let diarization_ready = Path::new(&self.diarization_model.artifact_path).is_dir();
        let ready = i18n::text(locale, "ready");
        let missing = i18n::text(locale, "missing");
        format!(
            "ASR {} / DIA {}",
            if asr_ready { ready } else { missing },
            if diarization_ready { ready } else { missing }
        )
    }
}

pub(crate) struct RootView {
    pub(crate) inventory: BackendInventory,
    pub(crate) services: DesktopServices,
    pub(crate) settings_draft: AppSettings,
    pub(crate) projects: Vec<DocumentState>,
    pub(crate) ui: UiState,
    pub(crate) assistant_input: gpui::Entity<TextInput>,
    pub(crate) cancel_contexts: Option<(JobContext, JobContext)>,
    pub(crate) settings_selectors: Option<SettingsSelectors>,
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
        assistant_input: gpui::Entity<TextInput>,
    ) -> Self {
        let projects = services.workspace.list_projects().unwrap_or_default();
        let locale = services
            .settings
            .snapshot()
            .map(|settings| i18n::Locale::from_code(&settings.ui_locale))
            .unwrap_or(i18n::Locale::ZhCn);
        let settings_draft = services.settings.snapshot().unwrap_or_default();
        let mut ui = UiState::from_projects(&projects, locale);
        ui.expanded_model_families.extend(
            services
                .model_manager
                .catalog()
                .models
                .iter()
                .map(|model| model.model_family.clone()),
        );
        Self {
            inventory,
            services,
            settings_draft,
            projects,
            ui,
            assistant_input,
            cancel_contexts: None,
            settings_selectors: None,
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
        let transcription_language_items = vec!["自动".into(), "English".into(), "中文".into()];
        let expected_speakers_items = vec![
            "自动".into(),
            "1".into(),
            "2".into(),
            "3".into(),
            "4".into(),
        ];
        let compute_device_items = vec![
            "自动".into(),
            "CPU".into(),
            "Metal".into(),
            "Core ML".into(),
        ];
        let expected_speakers_value = settings.expected_speakers.map(|value| value.to_string());

        let language = cx.new(|cx| {
            SelectState::new(
                language_items.clone(),
                selected_index(
                    &language_items,
                    Some(if self.ui.locale == i18n::Locale::EnUs {
                        "English"
                    } else {
                        "简体中文"
                    }),
                ),
                window,
                cx,
            )
        });
        let asr_backend = cx.new(|cx| {
            SelectState::new(
                asr_backend_items.clone(),
                selected_index(&asr_backend_items, settings.asr_backend_id.as_deref()),
                window,
                cx,
            )
        });
        let asr_model = cx.new(|cx| {
            SelectState::new(
                asr_model_items.clone(),
                selected_model_index(
                    &asr_model_items,
                    settings
                        .asr_model
                        .as_ref()
                        .map(|model| model.model_id.as_str()),
                ),
                window,
                cx,
            )
        });
        let transcription_language = cx.new(|cx| {
            SelectState::new(
                transcription_language_items.clone(),
                selected_index(
                    &transcription_language_items,
                    Some(match settings.language.as_deref() {
                        Some("en") | Some("en-US") => "English",
                        Some("zh") | Some("zh-CN") => "中文",
                        _ => "自动",
                    }),
                ),
                window,
                cx,
            )
        });
        let diarization_backend = cx.new(|cx| {
            SelectState::new(
                diarization_backend_items.clone(),
                selected_index(
                    &diarization_backend_items,
                    settings.diarization_backend_id.as_deref(),
                ),
                window,
                cx,
            )
        });
        let diarization_model = cx.new(|cx| {
            SelectState::new(
                diarization_model_items.clone(),
                selected_model_index(
                    &diarization_model_items,
                    settings
                        .diarization_model
                        .as_ref()
                        .map(|model| model.model_id.as_str()),
                ),
                window,
                cx,
            )
        });
        let diarization_segmentation_model = cx.new(|cx| {
            SelectState::new(
                segmentation_items.clone(),
                selected_stage_model_index(
                    &segmentation_items,
                    settings
                        .diarization_stages
                        .segmentation_model
                        .as_ref()
                        .map(|model| model.model_id.as_str()),
                ),
                window,
                cx,
            )
        });
        let diarization_embedding_model = cx.new(|cx| {
            SelectState::new(
                embedding_items.clone(),
                selected_stage_model_index(
                    &embedding_items,
                    settings
                        .diarization_stages
                        .embedding_model
                        .as_ref()
                        .map(|model| model.model_id.as_str()),
                ),
                window,
                cx,
            )
        });
        let diarization_clustering = cx.new(|cx| {
            SelectState::new(
                clustering_items.clone(),
                selected_index(
                    &clustering_items,
                    settings
                        .diarization_stages
                        .clustering_algorithm
                        .as_ref()
                        .map(|algorithm| algorithm.id.as_str())
                        .or_else(|| clustering_items.first().map(String::as_str)),
                ),
                window,
                cx,
            )
        });
        let expected_speakers = cx.new(|cx| {
            SelectState::new(
                expected_speakers_items.clone(),
                selected_index(
                    &expected_speakers_items,
                    Some(expected_speakers_value.as_deref().unwrap_or("自动")),
                ),
                window,
                cx,
            )
        });
        let compute_device = cx.new(|cx| {
            SelectState::new(
                compute_device_items.clone(),
                selected_index(
                    &compute_device_items,
                    Some(i18n::device(self.ui.locale, settings.compute_device)),
                ),
                window,
                cx,
            )
        });

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

    fn model_options(&self, task: &str, backend_id: Option<&str>) -> Vec<ModelOption> {
        self.services
            .model_manager
            .browse(Some(task), backend_id)
            .into_iter()
            .map(|entry| ModelOption::from_availability(&entry))
            .collect()
    }

    fn stage_model_options(
        &self,
        stage: DiarizationStageKind,
        backend_id: Option<&str>,
        model: Option<&ModelRef>,
    ) -> Vec<StageModelOption> {
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
                installed: model.is_some_and(|model| self.stage_option_is_installed(model, option)),
            })
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

    fn stage_option_is_installed(&self, model: &ModelRef, option: &DiarizationStageOption) -> bool {
        let root = Path::new(&model.artifact_path);
        option
            .artifact_files
            .iter()
            .all(|filename| root.join(filename).is_file())
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

    fn persist_settings(&mut self, cx: &mut Context<Self>) {
        match self.services.settings.update(self.settings_draft.clone()) {
            Ok(settings) => {
                self.settings_draft = settings;
                self.ui.notice = i18n::text(self.ui.locale, "settings_saved").into();
            }
            Err(error) => {
                self.ui.notice =
                    i18n::message(self.ui.locale, "settings_failed", &error.to_string());
            }
        }
        cx.notify();
    }

    fn refresh_model_selector(
        &mut self,
        task: &'static str,
        backend_id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = self.model_options(task, backend_id);
        let selected_model_id = match task {
            "asr" => self
                .settings_draft
                .asr_model
                .as_ref()
                .map(|model| model.model_id.as_str()),
            "diarization" => self
                .settings_draft
                .diarization_model
                .as_ref()
                .map(|model| model.model_id.as_str()),
            _ => None,
        };
        let selected_index = selected_model_index(&items, selected_model_id);
        let selector = match (task, self.settings_selectors.as_ref()) {
            ("asr", Some(selectors)) => selectors.asr_model.clone(),
            ("diarization", Some(selectors)) => selectors.diarization_model.clone(),
            _ => return,
        };
        selector.update(cx, |state, cx| {
            state.set_items(items, window, cx);
            state.set_selected_index(selected_index, window, cx);
        });
    }

    fn refresh_stage_selectors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let backend_id = self.settings_draft.diarization_backend_id.clone();
        let model = self.settings_draft.diarization_model.clone();
        let segmentation_items = self.stage_model_options(
            DiarizationStageKind::Segmentation,
            backend_id.as_deref(),
            model.as_ref(),
        );
        let embedding_items = self.stage_model_options(
            DiarizationStageKind::Embedding,
            backend_id.as_deref(),
            model.as_ref(),
        );
        let clustering_items = self.clustering_options(backend_id.as_deref());
        let Some(selectors) = self.settings_selectors.as_ref() else {
            return;
        };
        let segmentation_selected = selected_stage_model_index(
            &segmentation_items,
            self.settings_draft
                .diarization_stages
                .segmentation_model
                .as_ref()
                .map(|model| model.model_id.as_str()),
        );
        selectors
            .diarization_segmentation_model
            .update(cx, |state, cx| {
                state.set_items(segmentation_items, window, cx);
                state.set_selected_index(segmentation_selected, window, cx);
            });
        let embedding_selected = selected_stage_model_index(
            &embedding_items,
            self.settings_draft
                .diarization_stages
                .embedding_model
                .as_ref()
                .map(|model| model.model_id.as_str()),
        );
        selectors
            .diarization_embedding_model
            .update(cx, |state, cx| {
                state.set_items(embedding_items, window, cx);
                state.set_selected_index(embedding_selected, window, cx);
            });
        let clustering_selected = selected_index(
            &clustering_items,
            self.settings_draft
                .diarization_stages
                .clustering_algorithm
                .as_ref()
                .map(|algorithm| algorithm.id.as_str())
                .or_else(|| clustering_items.first().map(String::as_str)),
        );
        selectors.diarization_clustering.update(cx, |state, cx| {
            state.set_items(clustering_items, window, cx);
            state.set_selected_index(clustering_selected, window, cx);
        });
    }

    fn on_language_selected(
        &mut self,
        _: &Entity<StringSelect>,
        event: &SelectEvent<Vec<String>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let SelectEvent::Confirm(Some(value)) = event {
            self.set_locale(
                if value == "English" {
                    i18n::Locale::EnUs
                } else {
                    i18n::Locale::ZhCn
                },
                cx,
            );
        }
    }

    fn on_asr_backend_selected(
        &mut self,
        _: &Entity<StringSelect>,
        event: &SelectEvent<Vec<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let SelectEvent::Confirm(Some(value)) = event {
            self.select_asr_backend(value.clone(), cx);
            self.refresh_model_selector("asr", Some(value), window, cx);
        }
    }

    fn on_asr_model_selected(
        &mut self,
        _: &Entity<ModelSelect>,
        event: &SelectEvent<Vec<ModelOption>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let SelectEvent::Confirm(Some(value)) = event {
            if let Some(backend_id) = self.settings_draft.asr_backend_id.clone() {
                self.use_model(value.clone(), "asr".into(), backend_id, cx);
            }
        }
    }

    fn on_transcription_language_selected(
        &mut self,
        _: &Entity<StringSelect>,
        event: &SelectEvent<Vec<String>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let SelectEvent::Confirm(Some(value)) = event {
            self.settings_draft.language = match value.as_str() {
                "English" => Some("en".into()),
                "中文" => Some("zh".into()),
                _ => None,
            };
            self.persist_settings(cx);
        }
    }

    fn on_diarization_backend_selected(
        &mut self,
        _: &Entity<StringSelect>,
        event: &SelectEvent<Vec<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let SelectEvent::Confirm(Some(value)) = event {
            self.select_diarization_backend(value.clone(), cx);
            self.refresh_model_selector("diarization", Some(value), window, cx);
            self.refresh_stage_selectors(window, cx);
        }
    }

    fn on_diarization_model_selected(
        &mut self,
        _: &Entity<ModelSelect>,
        event: &SelectEvent<Vec<ModelOption>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let SelectEvent::Confirm(Some(value)) = event {
            if let Some(backend_id) = self.settings_draft.diarization_backend_id.clone() {
                self.use_model(value.clone(), "diarization".into(), backend_id, cx);
                self.refresh_stage_selectors(window, cx);
            }
        }
    }

    fn on_diarization_segmentation_model_selected(
        &mut self,
        _: &Entity<StageModelSelect>,
        event: &SelectEvent<Vec<StageModelOption>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let SelectEvent::Confirm(Some(value)) = event {
            self.settings_draft.diarization_stages.segmentation_model =
                self.stage_model_ref(DiarizationStageKind::Segmentation, value);
            self.persist_settings(cx);
        }
    }

    fn on_diarization_embedding_model_selected(
        &mut self,
        _: &Entity<StageModelSelect>,
        event: &SelectEvent<Vec<StageModelOption>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let SelectEvent::Confirm(Some(value)) = event {
            self.settings_draft.diarization_stages.embedding_model =
                self.stage_model_ref(DiarizationStageKind::Embedding, value);
            self.persist_settings(cx);
        }
    }

    fn on_diarization_clustering_selected(
        &mut self,
        _: &Entity<StringSelect>,
        event: &SelectEvent<Vec<String>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let SelectEvent::Confirm(Some(value)) = event {
            let option = self
                .selected_diarization_descriptor()
                .and_then(|descriptor| {
                    descriptor.diarization_stage_options.iter().find(|option| {
                        option.stage == DiarizationStageKind::Clustering && option.id == *value
                    })
                });
            self.settings_draft.diarization_stages.clustering_algorithm =
                option.map(|option| contracts::AlgorithmRef {
                    id: option.id.clone(),
                    revision: option.revision.clone(),
                    parameters: None,
                });
            self.persist_settings(cx);
        }
    }

    fn on_expected_speakers_selected(
        &mut self,
        _: &Entity<StringSelect>,
        event: &SelectEvent<Vec<String>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let SelectEvent::Confirm(Some(value)) = event {
            self.settings_draft.expected_speakers = value.parse().ok();
            self.persist_settings(cx);
        }
    }

    fn on_compute_device_selected(
        &mut self,
        _: &Entity<StringSelect>,
        event: &SelectEvent<Vec<String>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let SelectEvent::Confirm(Some(value)) = event {
            self.settings_draft.compute_device = match value.as_str() {
                "CPU" => contracts::ComputeDevice::Cpu,
                "Metal" => contracts::ComputeDevice::Metal,
                "Core ML" => contracts::ComputeDevice::CoreMl,
                _ => contracts::ComputeDevice::Auto,
            };
            self.persist_settings(cx);
        }
    }

    pub(crate) fn import_audio(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
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
            this.update(cx, |this, cx| {
                match this
                    .services
                    .workspace
                    .import_audio(path.display().to_string())
                {
                    Ok(project) => {
                        this.ui.selected_project = Some(project.id().clone());
                        this.projects = this.services.workspace.list_projects().unwrap_or_default();
                        this.ui.notice = i18n::message(this.ui.locale, "imported", project.title());
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
        let current_path = self.settings_draft.speaker_library_path.clone();
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            let Some(directory) = paths.into_iter().next() else {
                return;
            };
            let destination = directory.join("library.json");
            this.update_in(cx, move |this, window, cx| {
                this.handle_speaker_library_path_change(
                    std::path::PathBuf::from(current_path),
                    destination,
                    window,
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }

    fn handle_speaker_library_path_change(
        &mut self,
        source: std::path::PathBuf,
        destination: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if source == destination {
            return;
        }
        if destination.is_file() || !source.is_file() {
            self.commit_speaker_library_path(destination, cx);
            return;
        }

        let source_for_migration = source.clone();
        let destination_for_migration = destination.clone();
        let source_for_description = source.display().to_string();
        let destination_for_description = destination.display().to_string();
        let locale = self.ui.locale;
        let root = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let root_for_migration = root.clone();
            let root_for_switch = root.clone();
            let source_for_migration = source_for_migration.clone();
            let destination_for_migration = destination_for_migration.clone();
            let destination_for_switch = destination.clone();
            dialog
                .confirm()
                .title(i18n::text(locale, "speaker_library_migration_title"))
                .description(format!(
                    "{}\n{}\n{}",
                    i18n::text(locale, "speaker_library_migration_description"),
                    source_for_description,
                    destination_for_description
                ))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(i18n::text(locale, "migrate"))
                        .cancel_text(i18n::text(locale, "use_new_path"))
                        .show_cancel(true),
                )
                .on_ok(move |_, _, cx| {
                    let result = application::migrate_file(
                        &source_for_migration,
                        &destination_for_migration,
                    );
                    root_for_migration
                        .update(cx, |this, cx| match result {
                            Ok(()) => this
                                .commit_speaker_library_path(destination_for_migration.clone(), cx),
                            Err(error) => {
                                this.ui.notice = i18n::message(
                                    this.ui.locale,
                                    "speaker_library_migration_failed",
                                    &error,
                                );
                                cx.notify();
                            }
                        })
                        .ok();
                    true
                })
                .on_cancel(move |_, _, cx| {
                    root_for_switch
                        .update(cx, |this, cx| {
                            this.commit_speaker_library_path(destination_for_switch.clone(), cx)
                        })
                        .ok();
                    true
                })
        });
    }

    fn commit_speaker_library_path(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        self.settings_draft.speaker_library_path = path.display().to_string();
        self.persist_settings(cx);
        self.ui.notice = i18n::message(
            self.ui.locale,
            "speaker_library_updated",
            &self.settings_draft.speaker_library_path,
        );
        cx.notify();
    }

    pub(crate) fn select_project(&mut self, project_id: ProjectId, cx: &mut Context<Self>) {
        self.ui.selected_project = Some(project_id);
        self.ui.notice.clear();
        cx.notify();
    }

    pub(crate) fn navigate(&mut self, route: Route, cx: &mut Context<Self>) {
        self.ui.route = route;
        self.ui.detail_overlay = None;
        cx.notify();
    }

    pub(crate) fn set_locale(&mut self, locale: i18n::Locale, cx: &mut Context<Self>) {
        self.settings_draft.ui_locale = locale.code().into();
        self.ui.locale = locale;
        self.assistant_input.update(cx, |input, _| {
            input.set_placeholder(i18n::text(locale, "ask_placeholder"));
        });
        match self.services.settings.update(self.settings_draft.clone()) {
            Ok(settings) => {
                self.settings_draft = settings;
                self.ui.notice = i18n::text(locale, "language_changed").into();
            }
            Err(error) => {
                self.ui.notice = i18n::message(locale, "settings_failed", &error.to_string());
            }
        }
        cx.notify();
    }

    pub(crate) fn open_detail(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.ui.selected_project.is_some() {
            self.ui.detail_overlay = Some(Overlay::ProjectDetail);
            cx.notify();
        }
    }

    pub(crate) fn close_detail(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.ui.detail_overlay = None;
        cx.notify();
    }

    pub(crate) fn set_content_mode(&mut self, mode: ContentMode, cx: &mut Context<Self>) {
        self.ui.content_mode = mode;
        cx.notify();
    }

    pub(crate) fn assistant_action(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let query = self.assistant_input.read(cx).value();
        self.ui.assistant_query = query.clone();
        self.ui.notice = if query.trim().is_empty() {
            i18n::text(self.ui.locale, "empty_request").into()
        } else {
            match self
                .services
                .document_commands
                .execute(DocumentCommandEnvelope::new(
                    format!("assistant-{}", self.ui.assistant_query.len()),
                    DocumentCommand::List,
                )) {
                Ok(DocumentCommandResult::Snapshot { documents, issues }) => {
                    let suffix = if issues.is_empty() {
                        format!(" ({} documents available)", documents.len())
                    } else {
                        format!(" ({} document issues)", issues.len())
                    };
                    i18n::message(
                        self.ui.locale,
                        "request_received",
                        &format!("{query}{suffix}"),
                    )
                }
                Ok(_) => i18n::message(self.ui.locale, "request_received", &query),
                Err(error) => error.to_string(),
            }
        };
        cx.notify();
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

    pub(crate) fn move_project(
        &mut self,
        project_id: ProjectId,
        new_parent_id: ProjectId,
        cx: &mut Context<Self>,
    ) {
        let workspace = self.services.workspace.clone();
        cx.spawn(async move |this, cx| {
            let result = runtime::spawn_blocking(move || {
                workspace.reparent_project(&project_id, Some(&new_parent_id), 0)
            })
            .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(projects) => {
                        this.projects = projects;
                        this.ui.notice = i18n::text(this.ui.locale, "hierarchy_updated").into();
                    }
                    Err(error) => {
                        this.ui.notice =
                            i18n::message(this.ui.locale, "move_failed", &error.to_string())
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn reorder_project(
        &mut self,
        project_id: ProjectId,
        target_id: ProjectId,
        before: bool,
        cx: &mut Context<Self>,
    ) {
        let workspace = self.services.workspace.clone();
        cx.spawn(async move |this, cx| {
            let result = runtime::spawn_blocking(move || {
                workspace.reorder_project(&project_id, &target_id, before)
            })
            .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(projects) => {
                        this.projects = projects;
                        this.ui.notice = i18n::text(this.ui.locale, "order_updated").into();
                    }
                    Err(error) => {
                        this.ui.notice =
                            i18n::message(this.ui.locale, "reorder_failed", &error.to_string())
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn select_asr_backend(&mut self, backend_id: String, cx: &mut Context<Self>) {
        self.settings_draft.asr_backend_id = Some(backend_id.clone());
        if self
            .settings_draft
            .asr_model
            .as_ref()
            .is_some_and(|model| model.backend_id != backend_id)
        {
            self.settings_draft.asr_model = None;
        }
        self.ui.notice = i18n::message(
            self.ui.locale,
            "backend_selected",
            &format!("ASR {backend_id}"),
        );
        self.persist_settings(cx);
        cx.notify();
    }

    pub(crate) fn select_diarization_backend(
        &mut self,
        backend_id: String,
        cx: &mut Context<Self>,
    ) {
        self.settings_draft.diarization_backend_id = Some(backend_id.clone());
        if self
            .settings_draft
            .diarization_model
            .as_ref()
            .is_some_and(|model| model.backend_id != backend_id)
        {
            self.settings_draft.diarization_model = None;
        }
        self.settings_draft.diarization_stages = Default::default();
        self.ui.notice = i18n::message(
            self.ui.locale,
            "backend_selected",
            &format!("Diarization {backend_id}"),
        );
        self.persist_settings(cx);
        cx.notify();
    }

    pub(crate) fn use_model(
        &mut self,
        model_id: String,
        task: String,
        backend_id: String,
        cx: &mut Context<Self>,
    ) {
        let model = self
            .services
            .model_manager
            .model_ref(&model_id, &backend_id);
        match model {
            Ok(model) => {
                let mut settings = self.settings_draft.clone();
                if task == "asr" {
                    settings.asr_backend_id = Some(backend_id);
                    settings.asr_model = Some(model);
                } else {
                    settings.diarization_backend_id = Some(backend_id);
                    settings.diarization_model = Some(model);
                    settings.diarization_stages = Default::default();
                }
                self.settings_draft = settings;
                self.ui.notice = i18n::message(self.ui.locale, "model_selected", &model_id);
                self.persist_settings(cx);
            }
            Err(error) => {
                self.ui.notice =
                    i18n::message(self.ui.locale, "model_not_ready", &error.to_string())
            }
        }
        cx.notify();
    }

    pub(crate) fn download_model(&mut self, model_id: String, cx: &mut Context<Self>) {
        match self.services.model_manager.start(&model_id) {
            Ok(task) => {
                self.ui.downloading = true;
                self.ui.notice = i18n::message(self.ui.locale, "download_started", &model_id);
                let events = Arc::new(Mutex::new(task.events));
                cx.notify();
                cx.spawn(async move |this, cx| loop {
                    let events = events.clone();
                    let event = runtime::spawn_blocking(move || {
                        events.lock().ok().and_then(|receiver| receiver.recv().ok())
                    })
                    .await;
                    let Some(DownloadEvent::Updated(progress)) = event else {
                        break;
                    };
                    let terminal = matches!(
                        progress.state,
                        DownloadState::Completed
                            | DownloadState::Cancelled
                            | DownloadState::Failed(_)
                    );
                    this.update(cx, |this, cx| {
                        this.ui.notice = match &progress.state {
                            DownloadState::Completed => {
                                i18n::message(this.ui.locale, "installed", &progress.model_id)
                            }
                            DownloadState::Cancelled => {
                                i18n::message(this.ui.locale, "cancelled", &progress.model_id)
                            }
                            DownloadState::Failed(error) => {
                                i18n::message(this.ui.locale, "download_failed", error)
                            }
                            _ => i18n::download_progress(
                                this.ui.locale,
                                &progress.model_id,
                                progress.artifact_index,
                                progress.artifact_count,
                                progress.completed_bytes,
                                progress.speed_bytes_per_second,
                            ),
                        };
                        if terminal {
                            this.ui.downloading = false;
                        }
                        cx.notify();
                    })
                    .ok();
                    if terminal {
                        break;
                    }
                })
                .detach();
            }
            Err(error) => {
                self.ui.notice = i18n::message(
                    self.ui.locale,
                    "download_failed",
                    &format!("{model_id}: {error}"),
                );
                cx.notify();
            }
        }
    }

    pub(crate) fn control_download(
        &mut self,
        id: models::manager::DownloadId,
        action: DownloadAction,
        cx: &mut Context<Self>,
    ) {
        let result = match action {
            DownloadAction::Pause => self.services.model_manager.pause(id),
            DownloadAction::Resume => self.services.model_manager.resume(id),
            DownloadAction::Cancel => self.services.model_manager.cancel(id),
        };
        self.ui.notice = match result {
            Ok(()) => i18n::text(self.ui.locale, "download_updated").into(),
            Err(error) => i18n::message(self.ui.locale, "download_failed", &error.to_string()),
        };
        cx.notify();
    }

    pub(crate) fn merge_speaker(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(project_id) = self.ui.selected_project.clone() else {
            return;
        };
        let workspace = self.services.workspace.clone();
        cx.spawn(async move |this, cx| {
            let result = runtime::spawn_blocking(move || {
                workspace.merge_speaker(&project_id, "speaker-1", "speaker-0")
            })
            .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(project) => {
                        this.projects = this.services.workspace.list_projects().unwrap_or_default();
                        this.ui.selected_project = Some(project.id().clone());
                        this.ui.notice = i18n::text(this.ui.locale, "speaker_merged").into();
                    }
                    Err(error) => {
                        this.ui.notice =
                            i18n::message(this.ui.locale, "speaker_edit_failed", &error.to_string())
                    }
                }
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
            self.cancel_processing(event, window, cx);
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
            self.ui.notice = i18n::text(self.ui.locale, "cancellation_requested").into();
            cx.notify();
        }
    }

    pub(crate) fn process_selected(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project_id) = self.ui.selected_project.clone() else {
            self.ui.notice = i18n::text(self.ui.locale, "select_project_first").into();
            cx.notify();
            return;
        };
        if self.ui.processing {
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
        self.ui.processing = true;
        self.ui.notice = i18n::message(self.ui.locale, "processing", project.title());
        let workspace = self.services.workspace.clone();
        let settings = self.settings_draft.clone();
        let asr_model = match (settings.asr_backend_id, settings.asr_model) {
            (Some(_), Some(model)) => model,
            (Some(backend_id), None) => {
                self.ui.notice = i18n::message(self.ui.locale, "asr_model_required", &backend_id);
                self.ui.processing = false;
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
                self.ui.processing = false;
                cx.notify();
                return;
            }
            (None, None) => self.services.diarization_model.clone(),
            (None, Some(model)) => model,
        };
        if let Err(error) = workspace.set_processing_options(ProcessingOptions {
            language: settings.language.clone(),
            expected_speakers: settings.expected_speakers,
            device: settings.compute_device,
            speaker_library_path: Some(settings.speaker_library_path.clone()),
            diarization_stages: settings.diarization_stages.clone(),
        }) {
            self.ui.processing = false;
            self.ui.notice = i18n::message(self.ui.locale, "processing_failed", &error);
            cx.notify();
            return;
        }
        let job = match workspace.start_project_processing_with_speaker_library(
            project_id,
            asr_model,
            diarization_model,
            Some(settings.speaker_library_path),
        ) {
            Ok(job) => job,
            Err(error) => {
                self.ui.processing = false;
                self.ui.notice = i18n::message(self.ui.locale, "processing_failed", &error.message);
                cx.notify();
                return;
            }
        };
        self.cancel_contexts = Some((job.asr_context.clone(), job.diarization_context.clone()));
        let mut asr_events = job.asr_events.fuse();
        let mut diarization_events = job.diarization_events.fuse();
        let mut processing_future = job.future.fuse();
        cx.notify();
        cx.spawn(async move |this, cx| loop {
            let mut asr_next = asr_events.next().fuse();
            let mut diarization_next = diarization_events.next().fuse();
            futures::select! {
                event = asr_next => {
                    if let Some(event) = event {
                        this.update(cx, |this, cx| this.apply_job_event("ASR", event, cx)).ok();
                    }
                }
                event = diarization_next => {
                    if let Some(event) = event {
                        this.update(cx, |this, cx| this.apply_job_event("DIA", event, cx)).ok();
                    }
                }
                result = processing_future => {
                    this.update(cx, |this, cx| {
                        this.ui.processing = false;
                        this.cancel_contexts = None;
                        this.projects = this.services.workspace.list_projects().unwrap_or_default();
                        match result {
                            Ok(project) => {
                                this.ui.selected_project = Some(project.id().clone());
                                this.ui.notice = i18n::message(this.ui.locale, "finished", project.title());
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
        })
        .detach();
    }

    fn apply_job_event(&mut self, source: &str, event: JobEvent, cx: &mut Context<Self>) {
        self.ui.notice = match event {
            JobEvent::Started { .. } => i18n::message(self.ui.locale, "job_started", source),
            JobEvent::Progress { progress, .. } => match progress.total {
                Some(total) => i18n::message(
                    self.ui.locale,
                    "job_progress_total",
                    &format!("{source}|{}|{total}", progress.completed),
                ),
                None => i18n::message(
                    self.ui.locale,
                    "job_progress",
                    &format!("{source}|{}", progress.completed),
                ),
            },
            JobEvent::Completed { .. } => i18n::message(self.ui.locale, "job_completed", source),
            JobEvent::Failed { message, .. } => {
                i18n::message(self.ui.locale, "job_failed", &format!("{source}|{message}"))
            }
            JobEvent::Cancelled { .. } => i18n::message(self.ui.locale, "job_cancelled", source),
        };
        cx.notify();
    }
}

fn selected_index(items: &[String], value: Option<&str>) -> Option<IndexPath> {
    value
        .and_then(|value| items.iter().position(|item| item == value))
        .map(|row| IndexPath::default().row(row))
}

fn selected_model_index(items: &[ModelOption], value: Option<&str>) -> Option<IndexPath> {
    value
        .and_then(|value| items.iter().position(|item| item.id == value))
        .map(|row| IndexPath::default().row(row))
}

fn selected_stage_model_index(
    items: &[StageModelOption],
    value: Option<&str>,
) -> Option<IndexPath> {
    value
        .and_then(|value| items.iter().position(|item| item.id == value))
        .or_else(|| (!items.is_empty()).then_some(0))
        .map(|row| IndexPath::default().row(row))
}

impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_settings_selectors(window, cx);
        let selected = self
            .ui
            .selected_project
            .as_ref()
            .and_then(|id| self.projects.iter().find(|project| project.id() == id))
            .cloned();
        let body = match self.ui.route {
            Route::Assistant => pages::assistant::render(self, cx).into_any_element(),
            Route::Meetings => {
                pages::meetings::render(self, selected.as_ref(), cx).into_any_element()
            }
            Route::Models => pages::models::render(self, cx).into_any_element(),
            Route::Settings => pages::settings::render(self, cx).into_any_element(),
        };
        let mut shell = gpui::div()
            .size_full()
            .bg(gpui::rgb(0x111318))
            .text_color(gpui::rgb(0xe9edf5))
            .flex()
            .flex_col()
            .child(pages::chrome::header(self, cx))
            .child(body);
        if self.ui.detail_overlay.is_some() && self.ui.route == Route::Meetings {
            shell = shell.child(pages::detail::render(selected.as_ref(), self.ui.locale, cx));
        }
        shell
    }
}

pub fn run(inventory: BackendInventory, services: DesktopServices) {
    let initial_locale = services
        .settings
        .snapshot()
        .map(|settings| i18n::Locale::from_code(&settings.ui_locale))
        .unwrap_or(i18n::Locale::ZhCn);
    application()
        .with_assets(gpui_component_assets::Assets)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);
            gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
            cx.bind_keys([
                KeyBinding::new("backspace", crate::text_input::Backspace, None),
                KeyBinding::new("delete", crate::text_input::Delete, None),
                KeyBinding::new("left", crate::text_input::Left, None),
                KeyBinding::new("right", crate::text_input::Right, None),
                KeyBinding::new("cmd-a", crate::text_input::SelectAll, None),
            ]);
            let bounds = Bounds::centered(None, gpui::size(gpui::px(1_180.), gpui::px(760.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |_, cx| {
                    let assistant_input = cx.new(|cx| {
                        TextInput::new(cx, i18n::text(initial_locale, "ask_placeholder"))
                    });
                    let workspace = services.workspace.clone();
                    let view = cx.new(|_| {
                        RootView::new(inventory.clone(), services.clone(), assistant_input)
                    });
                    if let Ok(receiver) = workspace.watch_documents() {
                        let receiver = Arc::new(Mutex::new(receiver));
                        let watched_view = view.clone();
                        cx.spawn(async move |cx| loop {
                            let receiver = receiver.clone();
                            let changed = runtime::spawn_blocking(move || {
                                receiver
                                    .lock()
                                    .ok()
                                    .and_then(|receiver| receiver.recv().ok())
                            })
                            .await;
                            if changed.is_none() {
                                break;
                            }
                            let workspace = workspace.clone();
                            watched_view.update(cx, |view, cx| {
                                view.projects = workspace.list_projects().unwrap_or_default();
                                cx.notify();
                            });
                        })
                        .detach();
                    }
                    view
                },
            )
            .expect("failed to open meetnote window");
            cx.activate(true);
        });
}
