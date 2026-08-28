use application::{BackendInventory, WorkspaceService};
use contracts::ModelRef;
use domain::{MeetingProject, ProjectId, ProjectStatus};
use futures::{FutureExt, StreamExt};
use gpui::{
    div, prelude::*, px, rgb, size, App, Bounds, ClickEvent, Context, PathPromptOptions, Window,
    WindowBounds, WindowOptions,
};
use gpui_platform::application;
use models::manager::{DownloadEvent, DownloadState};
use runtime::{JobContext, JobEvent};
use settings::SettingsService;
use std::path::Path;
use std::sync::{Arc, Mutex};

mod route;
mod state;

use route::Route;
use state::{ContentMode, Overlay, UiState};

#[derive(Clone)]
pub struct DesktopServices {
    pub workspace: WorkspaceService,
    pub settings: SettingsService,
    pub model_manager: models::manager::ModelManager,
    pub asr_model: ModelRef,
    pub diarization_model: ModelRef,
}

impl DesktopServices {
    fn model_status(&self) -> String {
        let asr = if Path::new(&self.asr_model.artifact_path).is_file() {
            "ready"
        } else {
            "missing"
        };
        let diarization = if Path::new(&self.diarization_model.artifact_path).is_dir() {
            "ready"
        } else {
            "missing"
        };
        format!("ASR {asr} / DIA {diarization}")
    }
}

struct RootView {
    inventory: BackendInventory,
    services: DesktopServices,
    projects: Vec<MeetingProject>,
    ui: UiState,
    cancel_contexts: Option<(JobContext, JobContext)>,
}

#[derive(Clone)]
struct ProjectDrag {
    project_id: ProjectId,
}

struct DragGhost {
    title: String,
}

impl Render for DragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .p_3()
            .rounded_md()
            .bg(rgb(0x263142))
            .text_sm()
            .child(self.title.clone())
    }
}

impl RootView {
    fn new(inventory: BackendInventory, services: DesktopServices) -> Self {
        let projects = services.workspace.list_projects().unwrap_or_default();
        let ui = UiState::from_projects(&projects);
        Self {
            inventory,
            services,
            projects,
            ui,
            cancel_contexts: None,
        }
    }

    fn import_audio(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose meeting audio".into()),
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
                        this.ui.selected_project = Some(project.id.clone());
                        this.projects = this.services.workspace.list_projects().unwrap_or_default();
                        this.ui.notice = format!("Imported {}", project.title);
                    }
                    Err(error) => this.ui.notice = format!("Import failed: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn select_project(&mut self, project_id: ProjectId, cx: &mut Context<Self>) {
        self.ui.selected_project = Some(project_id);
        self.ui.notice.clear();
        cx.notify();
    }

    fn navigate(&mut self, route: Route, cx: &mut Context<Self>) {
        self.ui.route = route;
        self.ui.detail_overlay = None;
        cx.notify();
    }

    fn open_detail(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.ui.selected_project.is_some() {
            self.ui.detail_overlay = Some(Overlay::ProjectDetail);
            cx.notify();
        }
    }

    fn close_detail(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.ui.detail_overlay = None;
        cx.notify();
    }

    fn set_content_mode(&mut self, mode: ContentMode, cx: &mut Context<Self>) {
        self.ui.content_mode = mode;
        cx.notify();
    }

    fn assistant_action(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.ui.notice =
            "Assistant input is ready for the LLM index service. No conversation history is stored."
                .into();
        cx.notify();
    }

    fn toggle_project(&mut self, project_id: ProjectId, cx: &mut Context<Self>) {
        if !self.ui.expanded_projects.insert(project_id.clone()) {
            self.ui.expanded_projects.remove(&project_id);
        }
        cx.notify();
    }

    fn move_project(
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
                        this.ui.notice = "Project hierarchy updated.".into();
                    }
                    Err(error) => this.ui.notice = format!("Cannot move project: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn select_asr_backend(&mut self, backend_id: String, cx: &mut Context<Self>) {
        if let Ok(mut settings) = self.services.settings.snapshot() {
            settings.asr_backend_id = Some(backend_id.clone());
            if settings
                .asr_model
                .as_ref()
                .is_some_and(|model| model.backend_id != backend_id)
            {
                settings.asr_model = None;
            }
            match self.services.settings.update(settings) {
                Ok(_) => self.ui.notice = format!("ASR backend selected: {backend_id}"),
                Err(error) => self.ui.notice = format!("Settings update failed: {error}"),
            }
        }
        cx.notify();
    }

    fn select_diarization_backend(&mut self, backend_id: String, cx: &mut Context<Self>) {
        if let Ok(mut settings) = self.services.settings.snapshot() {
            settings.diarization_backend_id = Some(backend_id.clone());
            if settings
                .diarization_model
                .as_ref()
                .is_some_and(|model| model.backend_id != backend_id)
            {
                settings.diarization_model = None;
            }
            match self.services.settings.update(settings) {
                Ok(_) => self.ui.notice = format!("Diarization backend selected: {backend_id}"),
                Err(error) => self.ui.notice = format!("Settings update failed: {error}"),
            }
        }
        cx.notify();
    }

    fn use_model(
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
                let mut settings = self.services.settings.snapshot().unwrap_or_default();
                if task == "asr" {
                    settings.asr_backend_id = Some(backend_id);
                    settings.asr_model = Some(model);
                } else {
                    settings.diarization_backend_id = Some(backend_id);
                    settings.diarization_model = Some(model);
                }
                self.ui.notice = match self.services.settings.update(settings) {
                    Ok(_) => format!("Selected {model_id} for {task}."),
                    Err(error) => format!("Settings update failed: {error}"),
                };
            }
            Err(error) => self.ui.notice = format!("Model is not ready: {error}"),
        }
        cx.notify();
    }

    fn download_model(&mut self, model_id: String, cx: &mut Context<Self>) {
        match self.services.model_manager.start(&model_id) {
            Ok(task) => {
                self.ui.downloading = true;
                self.ui.notice = format!("Downloading {model_id}…");
                let events = Arc::new(Mutex::new(task.events));
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
                                format!("Installed {}.", progress.model_id)
                            }
                            DownloadState::Cancelled => {
                                format!("Cancelled {}.", progress.model_id)
                            }
                            DownloadState::Failed(error) => {
                                format!("Download failed: {error}")
                            }
                            _ => format!(
                                "{} artifact {}/{}: {} bytes",
                                progress.model_id,
                                progress.artifact_index,
                                progress.artifact_count,
                                progress.completed_bytes
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
                self.ui.notice = format!("Cannot download {model_id}: {error}");
                cx.notify();
            }
        }
    }

    fn control_download(
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
            Ok(()) => "Download task updated.".into(),
            Err(error) => format!("Download task failed: {error}"),
        };
        cx.notify();
    }

    fn merge_speaker(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
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
                        this.ui.selected_project = Some(project.id);
                        this.ui.notice =
                            "Merged speaker-1 into speaker-0 and updated Markdown.".into();
                    }
                    Err(error) => this.ui.notice = format!("Speaker edit failed: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn process_or_cancel(
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

    fn cancel_processing(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((asr_context, diarization_context)) = &self.cancel_contexts {
            asr_context.cancel();
            diarization_context.cancel();
            self.ui.notice = "Cancellation requested…".into();
            cx.notify();
        }
    }

    fn process_selected(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(project_id) = self.ui.selected_project.clone() else {
            self.ui.notice = "Select a project first.".into();
            cx.notify();
            return;
        };
        if self.ui.processing {
            return;
        }
        let Some(project) = self
            .projects
            .iter()
            .find(|project| project.id == project_id)
            .cloned()
        else {
            self.ui.notice = "Selected project no longer exists.".into();
            cx.notify();
            return;
        };
        self.ui.processing = true;
        self.ui.notice = format!("Processing {}…", project.title);
        let workspace = self.services.workspace.clone();
        let settings = self.services.settings.snapshot().unwrap_or_default();
        let asr_model = match (settings.asr_backend_id, settings.asr_model) {
            (Some(_), Some(model)) => model,
            (Some(backend_id), None) => {
                self.ui.notice = format!("Select an installed ASR model for {backend_id} first.");
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
                    format!("Select an installed diarization model for {backend_id} first.");
                self.ui.processing = false;
                cx.notify();
                return;
            }
            (None, None) => self.services.diarization_model.clone(),
            (None, Some(model)) => model,
        };
        let job = match workspace.start_project_processing(project_id, asr_model, diarization_model)
        {
            Ok(job) => job,
            Err(error) => {
                self.ui.processing = false;
                self.ui.notice = format!("Processing failed: {}", error.message);
                cx.notify();
                return;
            }
        };
        self.cancel_contexts = Some((job.asr_context.clone(), job.diarization_context.clone()));
        let mut asr_events = job.asr_events.fuse();
        let mut diarization_events = job.diarization_events.fuse();
        let mut processing_future = job.future.fuse();
        cx.notify();
        cx.spawn(async move |this, cx| {
            loop {
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
                                    this.ui.selected_project = Some(project.id.clone());
                                    this.ui.notice = format!("Finished {}", project.title);
                                }
                                Err(error) => this.ui.notice = format!("Processing failed: {}", error.message),
                            }
                            cx.notify();
                        }).ok();
                        break;
                    }
                }
            }
        })
        .detach();
    }

    fn apply_job_event(&mut self, source: &str, event: JobEvent, cx: &mut Context<Self>) {
        self.ui.notice = match event {
            JobEvent::Started { .. } => format!("{source} started…"),
            JobEvent::Progress { progress, .. } => match progress.total {
                Some(total) => format!("{source} progress: {}/{}", progress.completed, total),
                None => format!("{source} progress: {}", progress.completed),
            },
            JobEvent::Completed { .. } => format!("{source} completed…"),
            JobEvent::Failed { message, .. } => format!("{source} failed: {message}"),
            JobEvent::Cancelled { .. } => format!("{source} cancelled…"),
        };
        cx.notify();
    }
}

#[derive(Clone, Copy)]
enum DownloadAction {
    Pause,
    Resume,
    Cancel,
}

impl Render for RootView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self
            .ui
            .selected_project
            .as_ref()
            .and_then(|id| self.projects.iter().find(|project| &project.id == id))
            .cloned();
        let body = match self.ui.route {
            Route::Assistant => assistant_panel(self, cx).into_any_element(),
            Route::Meetings => meetings_panel(self, selected.as_ref(), cx).into_any_element(),
            Route::Models => models_panel(self, cx).into_any_element(),
            Route::Settings => settings_panel(self, cx).into_any_element(),
        };
        let mut shell = div()
            .size_full()
            .bg(rgb(0x111318))
            .text_color(rgb(0xe9edf5))
            .flex()
            .flex_col()
            .child(header(self, cx))
            .child(body);
        if self.ui.detail_overlay.is_some() && self.ui.route == Route::Meetings {
            shell = shell.child(detail_overlay(selected.as_ref(), cx));
        }
        shell
    }
}

fn assistant_panel(root: &mut RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let assistant_placeholder = if root.ui.assistant_query.is_empty() {
        "Ask about your recordings…".to_string()
    } else {
        root.ui.assistant_query.clone()
    };
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_5()
        .p_8()
        .child(
            div()
                .text_3xl()
                .font_weight(gpui::FontWeight::BOLD)
                .child("meetnote"),
        )
        .child(
            div()
                .text_lg()
                .text_color(rgb(0x8992a3))
                .child("Ask your voice library"),
        )
        .child(
            div()
                .id("assistant-input")
                .w(px(620.))
                .min_h(px(96.))
                .p_4()
                .rounded_lg()
                .border_1()
                .border_color(rgb(0x3b82f6))
                .bg(rgb(0x171b22))
                .cursor_pointer()
                .text_color(rgb(0x8992a3))
                .child(assistant_placeholder)
                .on_click(cx.listener(RootView::assistant_action)),
        )
        .child(
            div()
                .flex()
                .gap_3()
                .child(assistant_action_button("Organize voice library"))
                .child(assistant_action_button("Find a meeting"))
                .child(assistant_action_button("Split a project")),
        )
        .child(
            div()
                .mt_4()
                .text_xs()
                .text_color(rgb(0x667085))
                .child(root.ui.notice.clone()),
        )
}

fn assistant_action_button(label: &'static str) -> impl IntoElement {
    div()
        .px_3()
        .py_2()
        .rounded_md()
        .bg(rgb(0x1f2937))
        .text_sm()
        .child(label)
}

fn meetings_panel(
    root: &mut RootView,
    selected: Option<&MeetingProject>,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    div()
        .flex_1()
        .flex()
        .child(sidebar(root, cx))
        .child(main_panel(
            selected,
            &root.inventory,
            &root.ui.notice,
            &root.services.model_status(),
            root.ui.processing,
            root.ui.content_mode,
            cx,
        ))
}

fn models_panel(root: &mut RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let entries = root.services.model_manager.browse(None, None);
    let mut panel = div()
        .flex_1()
        .h_full()
        .p_6()
        .flex()
        .flex_col()
        .gap_4()
        .child(div().text_2xl().child("Models"))
        .child(
            div()
                .text_sm()
                .text_color(rgb(0x8992a3))
                .child("Catalog, installation state, and download tasks"),
        );
    if entries.is_empty() {
        panel = panel.child(empty_card("No models are available in the catalog."));
    } else {
        let cards = entries.into_iter().map(|entry| {
            let model_id = entry.model.id.clone();
            let action =
                if matches!(
                    entry.install_state,
                    models::ModelInstallState::Missing | models::ModelInstallState::Partial
                ) {
                    div()
                        .id(format!("download-{}", model_id))
                        .cursor_pointer()
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(rgb(0x2563eb))
                        .text_sm()
                        .child("Download")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.download_model(model_id.clone(), cx)
                        }))
                        .into_any_element()
                } else if let Some(backend_id) = entry.model.candidate_backends.first().cloned() {
                    let task = entry.model.task.clone();
                    div()
                        .id(format!("use-{}", model_id))
                        .cursor_pointer()
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(rgb(0x263142))
                        .text_sm()
                        .child("Use in settings")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.use_model(model_id.clone(), task.clone(), backend_id.clone(), cx)
                        }))
                        .into_any_element()
                } else {
                    div().into_any_element()
                };
            div()
                .p_4()
                .rounded_lg()
                .bg(rgb(0x171b22))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .child(div().text_lg().child(entry.model.id))
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(0x60a5fa))
                                .child(format!("{:?}", entry.install_state)),
                        )
                        .child(action),
                )
                .child(
                    div()
                        .mt_2()
                        .text_sm()
                        .text_color(rgb(0x8992a3))
                        .child(format!(
                            "{} · {} · {}",
                            entry.model.task, entry.model.source_format, entry.model.license
                        )),
                )
        });
        panel = panel.child(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .children(cards.collect::<Vec<_>>()),
        );
    }
    let downloads = root.services.model_manager.list().unwrap_or_default();
    for progress in downloads {
        let id = progress.id;
        let action = match progress.state {
            DownloadState::Paused => DownloadAction::Resume,
            DownloadState::Queued | DownloadState::Downloading => DownloadAction::Pause,
            _ => DownloadAction::Cancel,
        };
        let label = match progress.state {
            DownloadState::Paused => "Resume",
            DownloadState::Queued | DownloadState::Downloading => "Pause",
            _ => "Cancel",
        };
        panel =
            panel.child(
                div()
                    .p_3()
                    .rounded_md()
                    .bg(rgb(0x1b2029))
                    .child(format!(
                        "{} · artifact {}/{} · {} bytes",
                        progress.model_id,
                        progress.artifact_index,
                        progress.artifact_count,
                        progress.completed_bytes
                    ))
                    .child(
                        div()
                            .id(format!("download-action-{}", id.0))
                            .cursor_pointer()
                            .mt_2()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(rgb(0x263142))
                            .text_xs()
                            .child(label)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.control_download(id, action, cx)
                            })),
                    ),
            );
    }
    panel.child(
        div()
            .mt_auto()
            .text_xs()
            .text_color(rgb(0x8992a3))
            .child(format!("{}", root.ui.notice)),
    )
}

fn settings_panel(root: &mut RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let snapshot = root.services.settings.snapshot().unwrap_or_default();
    let mut asr_choices = div().flex().gap_2().flex_wrap();
    for descriptor in root.inventory.asr.clone() {
        let backend_id = descriptor.id.clone();
        let selected = snapshot.asr_backend_id.as_deref() == Some(descriptor.id.as_str());
        asr_choices = asr_choices.child(
            div()
                .id(format!("asr-backend-{}", descriptor.id))
                .cursor_pointer()
                .px_3()
                .py_2()
                .rounded_md()
                .bg(if selected {
                    rgb(0x2563eb)
                } else {
                    rgb(0x263142)
                })
                .child(descriptor.id)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select_asr_backend(backend_id.clone(), cx)
                })),
        );
    }
    let mut diarization_choices = div().flex().gap_2().flex_wrap();
    for descriptor in root.inventory.diarization.clone() {
        let backend_id = descriptor.id.clone();
        let selected = snapshot.diarization_backend_id.as_deref() == Some(descriptor.id.as_str());
        diarization_choices = diarization_choices.child(
            div()
                .id(format!("diarization-backend-{}", descriptor.id))
                .cursor_pointer()
                .px_3()
                .py_2()
                .rounded_md()
                .bg(if selected {
                    rgb(0x2563eb)
                } else {
                    rgb(0x263142)
                })
                .child(descriptor.id)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select_diarization_backend(backend_id.clone(), cx)
                })),
        );
    }
    div()
        .flex_1()
        .h_full()
        .p_6()
        .flex()
        .flex_col()
        .gap_4()
        .child(div().text_2xl().child("Settings"))
        .child(settings_section(
            "Interface",
            vec![
                setting_row("Language / Interface locale", &snapshot.ui_locale).into_any_element(),
            ],
        ))
        .child(settings_section(
            "Transcription",
            vec![
                setting_row(
                    "ASR backend",
                    snapshot.asr_backend_id.as_deref().unwrap_or("Auto"),
                )
                .into_any_element(),
                asr_choices.into_any_element(),
                setting_row(
                    "ASR model",
                    snapshot
                        .asr_model
                        .as_ref()
                        .map(|model| model.model_id.as_str())
                        .unwrap_or("Not selected"),
                )
                .into_any_element(),
                setting_row(
                    "Transcription language",
                    snapshot.language.as_deref().unwrap_or("Auto"),
                )
                .into_any_element(),
            ],
        ))
        .child(settings_section(
            "Speaker separation",
            vec![
                setting_row(
                    "Diarization backend",
                    snapshot.diarization_backend_id.as_deref().unwrap_or("Auto"),
                )
                .into_any_element(),
                diarization_choices.into_any_element(),
                setting_row(
                    "Diarization model",
                    snapshot
                        .diarization_model
                        .as_ref()
                        .map(|model| model.model_id.as_str())
                        .unwrap_or("Not selected"),
                )
                .into_any_element(),
                setting_row(
                    "Expected speakers",
                    &snapshot
                        .expected_speakers
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "Auto".into()),
                )
                .into_any_element(),
            ],
        ))
        .child(settings_section(
            "Runtime",
            vec![
                setting_row("Compute device", &format!("{:?}", snapshot.compute_device))
                    .into_any_element(),
            ],
        ))
}

fn settings_section(title: &'static str, rows: Vec<gpui::AnyElement>) -> impl IntoElement {
    div()
        .p_4()
        .rounded_lg()
        .bg(rgb(0x171b22))
        .child(div().text_lg().child(title))
        .children(rows)
}

fn setting_row(label: &'static str, value: &str) -> impl IntoElement {
    div()
        .mt_3()
        .flex()
        .justify_between()
        .text_sm()
        .child(div().text_color(rgb(0x8992a3)).child(label))
        .child(div().child(value.to_string()))
}

fn empty_card(message: &'static str) -> impl IntoElement {
    div()
        .flex_1()
        .rounded_lg()
        .bg(rgb(0x171b22))
        .flex()
        .items_center()
        .justify_center()
        .text_color(rgb(0x8992a3))
        .child(message)
}

fn detail_overlay(
    project: Option<&MeetingProject>,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let content = project
        .map(|project| {
            div()
                .child(div().text_xl().child(project.title.clone()))
                .child(
                    div()
                        .mt_2()
                        .text_sm()
                        .text_color(rgb(0x8992a3))
                        .child(format!("UUID: {}", project.id.as_str())),
                )
                .child(
                    div()
                        .mt_2()
                        .text_sm()
                        .text_color(rgb(0x8992a3))
                        .child(project.audio_path.clone()),
                )
                .child(
                    div()
                        .mt_5()
                        .flex_1()
                        .id("detail-transcript")
                        .overflow_scroll()
                        .children(
                            project
                                .speaker_segments
                                .iter()
                                .map(|segment| {
                                    div()
                                        .pb_3()
                                        .child(format!(
                                            "{}  {}",
                                            timestamp(segment.start_ms),
                                            segment.speaker_id
                                        ))
                                        .child(div().text_sm().child(segment.text.clone()))
                                })
                                .collect::<Vec<_>>(),
                        ),
                )
                .into_any_element()
        })
        .unwrap_or_else(|| empty_card("Select a project first.").into_any_element());
    div()
        .absolute()
        .top(px(76.))
        .right(px(24.))
        .bottom(px(24.))
        .w(px(440.))
        .p_5()
        .rounded_lg()
        .border_1()
        .border_color(rgb(0x3b82f6))
        .bg(rgb(0x202733))
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(0x60a5fa))
                        .child("PROJECT DETAIL"),
                )
                .child(
                    div()
                        .id("close-detail")
                        .cursor_pointer()
                        .child("×")
                        .on_click(cx.listener(RootView::close_detail)),
                ),
        )
        .child(content)
}

fn header(_root: &mut RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    div()
        .h(px(64.))
        .px_6()
        .flex()
        .items_center()
        .justify_between()
        .border_b_1()
        .border_color(rgb(0x2b303b))
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .text_xl()
                        .font_weight(gpui::FontWeight::BOLD)
                        .child("meetnote"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(0x8992a3))
                        .child("local meeting intelligence"),
                ),
        )
        .child(
            div()
                .flex()
                .gap_2()
                .child(nav_button("Assistant", Route::Assistant, cx))
                .child(nav_button("Meetings", Route::Meetings, cx))
                .child(nav_button("Models", Route::Models, cx))
                .child(nav_button("Settings", Route::Settings, cx))
                .child(
                    div()
                        .id("import-audio")
                        .cursor_pointer()
                        .px_4()
                        .py_2()
                        .rounded_md()
                        .bg(rgb(0x3b82f6))
                        .child("Import audio")
                        .on_click(cx.listener(RootView::import_audio)),
                ),
        )
}

fn nav_button(label: &'static str, route: Route, cx: &mut Context<RootView>) -> impl IntoElement {
    div()
        .id(label)
        .cursor_pointer()
        .px_2()
        .py_2()
        .rounded_md()
        .text_sm()
        .child(label)
        .on_click(cx.listener(move |this, _, _, cx| this.navigate(route, cx)))
}

fn sidebar(root: &mut RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let mut content = div()
        .w(px(280.))
        .h_full()
        .p_4()
        .gap_3()
        .flex()
        .flex_col()
        .border_r_1()
        .border_color(rgb(0x2b303b))
        .child(
            div()
                .text_sm()
                .text_color(rgb(0x8992a3))
                .child("AUDIO PROJECTS"),
        );

    if root.projects.is_empty() {
        content = content.child(
            div()
                .p_3()
                .rounded_md()
                .bg(rgb(0x1b2029))
                .text_sm()
                .text_color(rgb(0x8992a3))
                .child("No meetings yet. Import an audio file to begin."),
        );
    } else {
        for (project, depth) in ordered_projects(&root.projects, &root.ui.expanded_projects) {
            let selected = root.ui.selected_project.as_ref() == Some(&project.id);
            let project_id = project.id.clone();
            let drag_id = project.id.clone();
            let drag_title = project.title.clone();
            let target_id = project.id.clone();
            let has_children = root
                .projects
                .iter()
                .any(|child| child.parent_id.as_ref() == Some(&project.id));
            let drag_handle = div()
                .id(format!("drag-{}", project.id.as_str()))
                .mr_2()
                .cursor_pointer()
                .text_color(rgb(0x667085))
                .child("⋮⋮")
                .on_drag(
                    ProjectDrag {
                        project_id: drag_id,
                    },
                    move |_, _, _, cx| {
                        cx.new(|_| DragGhost {
                            title: drag_title.clone(),
                        })
                    },
                );
            let expand_handle =
                if has_children {
                    let expand_id = project.id.clone();
                    div()
                        .id(format!("expand-{}", project.id.as_str()))
                        .cursor_pointer()
                        .mr_1()
                        .child(if root.ui.expanded_projects.contains(&project.id) {
                            "▾"
                        } else {
                            "▸"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_project(expand_id.clone(), cx)
                        }))
                        .into_any_element()
                } else {
                    div().w(px(10.)).into_any_element()
                };
            content = content.child(
                div()
                    .id(project.id.as_str().to_string())
                    .cursor_pointer()
                    .p_3()
                    .ml(px((depth as f32) * 16.))
                    .rounded_md()
                    .bg(if selected {
                        rgb(0x27344a)
                    } else {
                        rgb(0x171b22)
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(drag_handle)
                            .child(expand_handle)
                            .child(div().text_sm().child(format!(
                                "{} {}",
                                if depth > 0 { "↳" } else { "◈" },
                                project.title
                            ))),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_xs()
                            .text_color(rgb(0x8992a3))
                            .child(status_label(project.status)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_project(project_id.clone(), cx)
                    }))
                    .on_drop(cx.listener(move |this, dragged: &ProjectDrag, _, cx| {
                        this.move_project(dragged.project_id.clone(), target_id.clone(), cx)
                    })),
            );
        }
    }
    content
}

fn ordered_projects(
    projects: &[MeetingProject],
    expanded: &std::collections::BTreeSet<ProjectId>,
) -> Vec<(MeetingProject, u8)> {
    fn append(
        projects: &[MeetingProject],
        parent: Option<&ProjectId>,
        depth: u8,
        expanded: &std::collections::BTreeSet<ProjectId>,
        output: &mut Vec<(MeetingProject, u8)>,
    ) {
        let mut children: Vec<_> = projects
            .iter()
            .filter(|project| project.parent_id.as_ref() == parent)
            .cloned()
            .collect();
        children.sort_by_key(|project| project.sibling_order);
        for child in children {
            let id = child.id.clone();
            output.push((child, depth));
            if expanded.contains(&id) {
                append(
                    projects,
                    Some(&id),
                    depth.saturating_add(1),
                    expanded,
                    output,
                );
            }
        }
    }
    let mut output = Vec::new();
    append(projects, None, 0, expanded, &mut output);
    output
}

fn main_panel(
    project: Option<&MeetingProject>,
    inventory: &BackendInventory,
    notice: &str,
    model_status: &str,
    processing: bool,
    content_mode: ContentMode,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let mut panel = div().flex_1().h_full().p_6().flex().flex_col().gap_4();
    if let Some(project) = project {
        panel = panel
            .child(div().text_2xl().child(project.title.clone()))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x8992a3))
                    .child(project.audio_path.clone()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(status_color(project.status))
                            .text_sm()
                            .child(status_label(project.status)),
                    )
                    .child(
                        div()
                            .id("process-project")
                            .cursor_pointer()
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .when(
                                project.status == ProjectStatus::Imported
                                    || project.status == ProjectStatus::Failed,
                                |button| button.bg(rgb(0x2563eb)),
                            )
                            .child(if processing {
                                "Cancel"
                            } else {
                                "Process meeting"
                            })
                            .on_click(cx.listener(RootView::process_or_cancel)),
                    )
                    .child(
                        div()
                            .id("open-project-detail")
                            .cursor_pointer()
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(rgb(0x263142))
                            .text_sm()
                            .child("Open detail")
                            .on_click(cx.listener(RootView::open_detail)),
                    ),
            );

        if project.status == ProjectStatus::Ready {
            if project
                .speaker_segments
                .iter()
                .any(|segment| segment.speaker_id == "speaker-1")
            {
                panel = panel.child(
                    div()
                        .id("merge-speaker")
                        .cursor_pointer()
                        .w(px(260.))
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .bg(rgb(0x263142))
                        .text_sm()
                        .child("Merge speaker-1 into speaker-0")
                        .on_click(cx.listener(RootView::merge_speaker)),
                );
            }
            panel = panel.child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .id("summary-mode")
                            .cursor_pointer()
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(if content_mode == ContentMode::Summary {
                                rgb(0x2563eb)
                            } else {
                                rgb(0x263142)
                            })
                            .child("Summary")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_content_mode(ContentMode::Summary, cx)
                            })),
                    )
                    .child(
                        div()
                            .id("original-mode")
                            .cursor_pointer()
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(if content_mode == ContentMode::Original {
                                rgb(0x2563eb)
                            } else {
                                rgb(0x263142)
                            })
                            .child("Original")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_content_mode(ContentMode::Original, cx)
                            })),
                    ),
            );
            if content_mode == ContentMode::Summary {
                panel = panel.child(
                    div()
                        .flex_1()
                        .p_5()
                        .rounded_lg()
                        .bg(rgb(0x171b22))
                        .text_sm()
                        .child(project.summary.clone().unwrap_or_else(|| {
                            "No summary yet. Use the Assistant to organize this project.".into()
                        })),
                );
            } else {
                panel = panel.child(
                    div()
                        .flex_1()
                        .id("transcript-list")
                        .overflow_scroll()
                        .p_4()
                        .rounded_lg()
                        .bg(rgb(0x171b22))
                        .children(
                            project
                                .speaker_segments
                                .iter()
                                .map(|segment| {
                                    div()
                                        .pb_3()
                                        .child(div().text_xs().text_color(rgb(0x60a5fa)).child(
                                            format!(
                                                "{}  {}",
                                                timestamp(segment.start_ms),
                                                segment.speaker_id
                                            ),
                                        ))
                                        .child(div().text_sm().child(segment.text.clone()))
                                })
                                .collect::<Vec<_>>(),
                        ),
                );
            }
        } else {
            panel = panel.child(
                div()
                    .flex_1()
                    .rounded_lg()
                    .bg(rgb(0x171b22))
                    .p_5()
                    .text_color(rgb(0x8992a3))
                    .child("Run processing to generate a speaker-labelled transcript."),
            );
        }
    } else {
        panel = panel.child(div().text_2xl().child("Your meetings")).child(
            div()
                .flex_1()
                .rounded_lg()
                .bg(rgb(0x171b22))
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(0x8992a3))
                .child("Import an audio file to start a project."),
        );
    }
    panel.child(
        div()
            .border_t_1()
            .border_color(rgb(0x2b303b))
            .pt_3()
            .text_xs()
            .text_color(rgb(0x8992a3))
            .child(format!(
                "{}   •   ASR: {}   •   DIA: {}",
                notice,
                backend_ids(&inventory.asr),
                backend_ids(&inventory.diarization)
            ))
            .child(format!("Models: {model_status}")),
    )
}

fn backend_ids(descriptors: &[application::BackendDescriptor]) -> String {
    descriptors
        .iter()
        .map(|descriptor| descriptor.id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn status_label(status: ProjectStatus) -> &'static str {
    match status {
        ProjectStatus::Imported => "Ready to process",
        ProjectStatus::Processing => "Processing",
        ProjectStatus::Ready => "Ready",
        ProjectStatus::Failed => "Failed",
    }
}

fn status_color(status: ProjectStatus) -> gpui::Rgba {
    match status {
        ProjectStatus::Imported => rgb(0x374151),
        ProjectStatus::Processing => rgb(0x92400e),
        ProjectStatus::Ready => rgb(0x166534),
        ProjectStatus::Failed => rgb(0x991b1b),
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

pub fn run(inventory: BackendInventory, services: DesktopServices) {
    application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1_180.), px(760.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| RootView::new(inventory.clone(), services.clone())),
        )
        .expect("failed to open meetnote window");
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use application::{ProcessingService, WorkspaceService};
    use asr_whisper_rs::WhisperRsBackend;
    use diarization_sherpa_onnx::SherpaOnnxDiarizationBackend;
    use gpui::TestAppContext;
    use models::{ModelCatalog, ModelManager};
    use persistence::InMemoryProjectRepository;
    use settings::JsonSettingsRepository;
    use std::sync::Arc;

    #[gpui::test]
    fn boots_an_empty_desktop_workspace(cx: &mut TestAppContext) {
        let repository = Arc::new(InMemoryProjectRepository::default());
        let workspace = WorkspaceService::new(
            ProcessingService::new(
                Arc::new(WhisperRsBackend::default()),
                Arc::new(SherpaOnnxDiarizationBackend::default()),
            ),
            repository,
            std::env::temp_dir().join("meetnote-ui-test-markdown"),
        );
        let services = DesktopServices {
            workspace,
            settings: SettingsService::open(Arc::new(JsonSettingsRepository::open(
                std::env::temp_dir().join("meetnote-ui-test-settings.json"),
            )))
            .unwrap(),
            model_manager: ModelManager::new(
                ModelCatalog::from_path(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../../resources/models/catalog.json"),
                )
                .unwrap(),
                "models",
            ),
            asr_model: ModelRef {
                backend_id: "whisper-rs".into(),
                model_id: "fixture".into(),
                revision: None,
                artifact_path: "missing-model".into(),
            },
            diarization_model: ModelRef {
                backend_id: "sherpa-onnx".into(),
                model_id: "fixture".into(),
                revision: None,
                artifact_path: "missing-bundle".into(),
            },
        };
        let view = cx.new(|_| RootView::new(BackendInventory::new(vec![], vec![]), services));

        assert_eq!(view.read_with(cx, |view, _| view.projects.len()), 0);
    }
}
