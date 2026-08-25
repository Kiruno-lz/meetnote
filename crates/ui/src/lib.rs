use application::{BackendInventory, WorkspaceService};
use contracts::ModelRef;
use domain::{MeetingProject, ProjectId, ProjectStatus};
use futures::{FutureExt, StreamExt};
use gpui::{
    div, prelude::*, px, rgb, size, App, Bounds, ClickEvent, Context, PathPromptOptions, Window,
    WindowBounds, WindowOptions,
};
use gpui_platform::application;
use runtime::{JobContext, JobEvent};
use std::path::Path;

#[derive(Clone)]
pub struct DesktopServices {
    pub workspace: WorkspaceService,
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
    selected_project: Option<ProjectId>,
    notice: String,
    processing: bool,
    cancel_contexts: Option<(JobContext, JobContext)>,
}

impl RootView {
    fn new(inventory: BackendInventory, services: DesktopServices) -> Self {
        let projects = services.workspace.list_projects().unwrap_or_default();
        let selected_project = projects.first().map(|project| project.id.clone());
        Self {
            inventory,
            services,
            projects,
            selected_project,
            notice: "Import an audio file to create a meeting project.".into(),
            processing: false,
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
                        this.selected_project = Some(project.id.clone());
                        this.projects = this.services.workspace.list_projects().unwrap_or_default();
                        this.notice = format!("Imported {}", project.title);
                    }
                    Err(error) => this.notice = format!("Import failed: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn select_project(&mut self, project_id: ProjectId, cx: &mut Context<Self>) {
        self.selected_project = Some(project_id);
        self.notice.clear();
        cx.notify();
    }

    fn process_or_cancel(
        &mut self,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.processing {
            self.cancel_processing(event, window, cx);
        } else {
            self.process_selected(event, window, cx);
        }
    }

    fn cancel_processing(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((asr_context, diarization_context)) = &self.cancel_contexts {
            asr_context.cancel();
            diarization_context.cancel();
            self.notice = "Cancellation requested…".into();
            cx.notify();
        }
    }

    fn process_selected(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(project_id) = self.selected_project.clone() else {
            self.notice = "Select a project first.".into();
            cx.notify();
            return;
        };
        if self.processing {
            return;
        }
        let Some(project) = self
            .projects
            .iter()
            .find(|project| project.id == project_id)
            .cloned()
        else {
            self.notice = "Selected project no longer exists.".into();
            cx.notify();
            return;
        };
        self.processing = true;
        self.notice = format!("Processing {}…", project.title);
        let workspace = self.services.workspace.clone();
        let asr_model = self.services.asr_model.clone();
        let diarization_model = self.services.diarization_model.clone();
        let job = match workspace.start_project_processing(project_id, asr_model, diarization_model)
        {
            Ok(job) => job,
            Err(error) => {
                self.processing = false;
                self.notice = format!("Processing failed: {}", error.message);
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
                            this.processing = false;
                            this.cancel_contexts = None;
                            this.projects = this.services.workspace.list_projects().unwrap_or_default();
                            match result {
                                Ok(project) => {
                                    this.selected_project = Some(project.id.clone());
                                    this.notice = format!("Finished {}", project.title);
                                }
                                Err(error) => this.notice = format!("Processing failed: {}", error.message),
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
        self.notice = match event {
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

impl Render for RootView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self
            .selected_project
            .as_ref()
            .and_then(|id| self.projects.iter().find(|project| &project.id == id))
            .cloned();
        div()
            .size_full()
            .bg(rgb(0x111318))
            .text_color(rgb(0xe9edf5))
            .flex()
            .flex_col()
            .child(header(self, cx))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .child(sidebar(self, cx))
                    .child(main_panel(
                        selected.as_ref(),
                        &self.inventory,
                        &self.notice,
                        &self.services.model_status(),
                        self.processing,
                        cx,
                    )),
            )
    }
}

fn header(_: &mut RootView, cx: &mut Context<RootView>) -> impl IntoElement {
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
                .id("import-audio")
                .cursor_pointer()
                .px_4()
                .py_2()
                .rounded_md()
                .bg(rgb(0x3b82f6))
                .child("Import audio")
                .on_click(cx.listener(RootView::import_audio)),
        )
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
        .child(div().text_sm().text_color(rgb(0x8992a3)).child("MEETINGS"));

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
        for project in root.projects.clone() {
            let selected = root.selected_project.as_ref() == Some(&project.id);
            let project_id = project.id.clone();
            content = content.child(
                div()
                    .id(project.id.as_str().to_string())
                    .cursor_pointer()
                    .p_3()
                    .rounded_md()
                    .bg(if selected {
                        rgb(0x27344a)
                    } else {
                        rgb(0x171b22)
                    })
                    .child(div().text_sm().child(project.title))
                    .child(
                        div()
                            .mt_1()
                            .text_xs()
                            .text_color(rgb(0x8992a3))
                            .child(status_label(project.status)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_project(project_id.clone(), cx)
                    })),
            );
        }
    }
    content
}

fn main_panel(
    project: Option<&MeetingProject>,
    inventory: &BackendInventory,
    notice: &str,
    model_status: &str,
    processing: bool,
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
                    ),
            );

        if project.status == ProjectStatus::Ready {
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
