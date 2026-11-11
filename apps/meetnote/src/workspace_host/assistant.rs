use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use application::desktop_assistant::{
    AssistantController, AssistantMutationSummary, AssistantProgress, AssistantRunResult,
    AssistantScopeMode, AssistantTaskState, AssistantUiError, AssistantUiService,
    ConnectionDiagnostic, ConnectionProbeId, ConnectionProbeKind, PendingReviewState,
};
use futures::Stream;
use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use meetnote::assistant_service::DesktopAssistantService;
use rune::{TextEdit, Tx};
use settings::{AssistantSettings, SettingsService};

use super::assistant_connection::{ConnectionDraft, public_assistant};
use super::{LiveWorkspace, Task, WorkspaceHost, processing::ProcessingConfig};
use crate::MeetNote;

#[derive(Default)]
pub(crate) struct AssistantHost {
    pub controller: AssistantController,
    pub prompt: TextEdit,
    pub settings: AssistantSettings,
    pub connection: Option<ConnectionDraft>,
    task: Option<Task<Reply>>,
    progress: Option<UnboundedReceiver<AssistantProgress>>,
}

enum Reply {
    Run(String, Result<AssistantRunResult, AssistantUiError>),
    Confirm(String, Result<AssistantRunResult, AssistantUiError>),
    Reject(String, Result<(), AssistantUiError>),
    Undo(
        String,
        String,
        Result<AssistantMutationSummary, AssistantUiError>,
    ),
    SavedConnection(Result<AssistantSettings, AssistantUiError>),
    Probe(
        ConnectionProbeId,
        Result<ConnectionDiagnostic, AssistantUiError>,
    ),
}

#[derive(Clone, Copy)]
pub(crate) enum ReviewDecision {
    Confirm,
    Reject,
}

impl AssistantHost {
    pub fn busy(&self) -> bool {
        self.task.is_some()
    }

    pub fn awaiting_review(&self) -> bool {
        matches!(
            self.controller.pending,
            Some(PendingReviewState::Awaiting(_))
        )
    }

    pub(super) fn reset_workspace(&mut self, settings: AssistantSettings) {
        self.controller.forget_workspace();
        self.controller.forget_probe();
        self.connection = Some(ConnectionDraft::active(&settings));
        self.settings = settings;
        self.prompt.clear();
    }

    fn start(&mut self, tx: &mut Tx, work: impl FnOnce() -> Reply + Send + 'static) {
        self.task = Some(Box::pin(runtime::spawn_blocking(move || Ok(work()))));
        tx.wake_after(Duration::ZERO);
    }

    fn poll_progress(&mut self, waker: Option<&Waker>, tx: &mut Tx) -> bool {
        let Some(progress) = &mut self.progress else {
            return false;
        };
        let mut cx = Context::from_waker(waker.unwrap_or_else(|| Waker::noop()));
        let mut changed = false;
        for _ in 0..32 {
            match Pin::new(&mut *progress).poll_next(&mut cx) {
                Poll::Ready(Some(event)) => {
                    self.controller.record_progress(event);
                    changed = true;
                }
                Poll::Ready(None) | Poll::Pending => return changed,
            }
        }
        tx.wake_after(Duration::ZERO);
        changed
    }
}

impl Drop for AssistantHost {
    fn drop(&mut self) {
        self.controller.forget_workspace();
        self.controller.forget_probe();
    }
}

fn service(
    config: &ProcessingConfig,
    live: LiveWorkspace,
) -> Result<DesktopAssistantService, AssistantUiError> {
    let settings = config
        .settings_service()
        .map_err(AssistantUiError::unavailable)?;
    Ok(DesktopAssistantService::with_job_runtime(
        settings,
        live.documents,
        config.jobs(),
    ))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl MeetNote {
    pub(crate) fn save_assistant_connection(&mut self, tx: &mut Tx) {
        if self.workspace_host.busy() {
            return;
        }
        let Some(live) = self.workspace_host.live.clone() else {
            return;
        };
        let config = self.workspace_host.processing_config.clone();
        let host = &mut self.workspace_host.assistant;
        let Some(draft) = &host.connection else {
            return;
        };
        let patch = match draft.changes(&host.settings) {
            Ok(patch) => patch,
            Err(message) => {
                self.notify(message, tx);
                return;
            }
        };
        let expected = host.settings.clone();
        host.start(tx, move || {
            Reply::SavedConnection((|| {
                let settings = config
                    .settings_service()
                    .map_err(AssistantUiError::unavailable)?;
                let snapshot = settings
                    .snapshot()
                    .map_err(|error| AssistantUiError::from_settings(&error))?;
                if public_assistant(snapshot.assistant.clone()) != expected {
                    return Err(AssistantUiError::from_settings(
                        &settings::SettingsError::RevisionConflict,
                    ));
                }
                let revision = SettingsService::revision_of(&snapshot);
                let service = DesktopAssistantService::with_job_runtime(
                    settings,
                    live.documents,
                    config.jobs(),
                );
                service
                    .save_assistant_settings(revision, patch)
                    .map(|saved| public_assistant(saved.assistant))
            })())
        });
    }

    pub(crate) fn probe_assistant_connection(&mut self, kind: ConnectionProbeKind, tx: &mut Tx) {
        if self.workspace_host.busy() {
            return;
        }
        let Some(live) = self.workspace_host.live.clone() else {
            return;
        };
        let config = self.workspace_host.processing_config.clone();
        let host = &mut self.workspace_host.assistant;
        let Some(profile_id) = host.settings.active_connection_id.clone() else {
            return;
        };
        let (id, cancellation) = host.controller.begin_probe(kind);
        host.start(tx, move || {
            Reply::Probe(
                id,
                (|| {
                    let settings = config
                        .settings_service()
                        .map_err(AssistantUiError::unavailable)?;
                    let snapshot = settings
                        .snapshot()
                        .map_err(|error| AssistantUiError::from_settings(&error))?;
                    let profile = snapshot
                        .assistant
                        .active_connection()
                        .filter(|profile| profile.id == profile_id)
                        .cloned()
                        .ok_or_else(|| {
                            AssistantUiError::unavailable("The active connection changed")
                        })?;
                    DesktopAssistantService::with_job_runtime(
                        settings,
                        live.documents,
                        config.jobs(),
                    )
                    .probe_connection(profile, kind, cancellation)
                })(),
            )
        });
    }

    pub(crate) fn reload_assistant_connection(&mut self, tx: &mut Tx) {
        if self.workspace_host.busy() {
            return;
        }
        let config = self.workspace_host.processing_config.clone();
        self.workspace_host.assistant.start(tx, move || {
            Reply::SavedConnection(
                config
                    .settings()
                    .map(|saved| public_assistant(saved.assistant))
                    .map_err(AssistantUiError::unavailable),
            )
        });
    }

    pub(crate) fn run_live_assistant(&mut self, tx: &mut Tx) {
        if self.workspace_host.busy() || self.workspace_host.assistant.awaiting_review() {
            return;
        }
        let Some(live) = self.workspace_host.live.clone() else {
            return;
        };
        let host = &mut self.workspace_host.assistant;
        let prompt = host.prompt.text.trim().to_owned();
        if prompt.is_empty() {
            return;
        }
        match host.controller.scope_mode {
            AssistantScopeMode::CurrentDocument => host
                .controller
                .authorize_document(self.documents.selected_state()),
            AssistantScopeMode::AllVisibleDocuments => host
                .controller
                .authorize_documents(self.documents.document_states()),
        }
        let (request, cancellation) = match host.controller.begin(prompt, &host.settings) {
            Ok(started) => started,
            Err(error) => {
                self.notify(error.to_string(), tx);
                return;
            }
        };
        host.prompt.clear();
        let (updates, progress) = unbounded();
        host.progress = Some(progress);
        let config = self.workspace_host.processing_config.clone();
        host.start(tx, move || {
            let id = request.request_id.clone();
            let result = service(&config, live).and_then(|service| {
                service.run_with_progress(
                    request,
                    cancellation,
                    Arc::new(move |event| {
                        let _ = updates.unbounded_send(event);
                    }),
                )
            });
            Reply::Run(id, result)
        });
    }

    pub(crate) fn cancel_live_assistant(&mut self, _: &mut Tx) {
        self.workspace_host.assistant.controller.request_cancel();
    }

    pub(crate) fn decide_live_review(&mut self, decision: ReviewDecision, tx: &mut Tx) {
        if self.workspace_host.busy() {
            return;
        }
        let Some(live) = self.workspace_host.live.clone() else {
            return;
        };
        let host = &mut self.workspace_host.assistant;
        let Some(run) = host.controller.result_run() else {
            return;
        };
        let pending = match decision {
            ReviewDecision::Confirm => host.controller.pending_for_accept(now_ms()),
            ReviewDecision::Reject => host.controller.pending_for_reject(now_ms()),
        };
        let Some(pending) = pending else { return };
        let grant = host.controller.result_authorized_document_ids();
        let config = self.workspace_host.processing_config.clone();
        host.start(tx, move || {
            let service = service(&config, live);
            match decision {
                ReviewDecision::Confirm => Reply::Confirm(
                    run,
                    service.and_then(|service| service.confirm(pending.confirmation_id, grant)),
                ),
                ReviewDecision::Reject => Reply::Reject(
                    run,
                    service.and_then(|service| service.reject(pending.confirmation_id, grant)),
                ),
            }
        });
    }

    pub(crate) fn undo_live_assistant(&mut self, mutation_id: &str, tx: &mut Tx) {
        if self.workspace_host.busy() {
            return;
        }
        let Some(live) = self.workspace_host.live.clone() else {
            return;
        };
        let host = &mut self.workspace_host.assistant;
        let Some(run) = host.controller.result_run() else {
            return;
        };
        let Some((token, mutation_id)) = host.controller.begin_mutation_undo(mutation_id) else {
            return;
        };
        let grant = host.controller.result_authorized_document_ids();
        let config = self.workspace_host.processing_config.clone();
        host.start(tx, move || {
            Reply::Undo(
                run,
                mutation_id,
                service(&config, live).and_then(|service| service.undo(token, grant)),
            )
        });
    }

    pub(super) fn poll_live_assistant(&mut self, tx: &mut Tx) -> bool {
        let host = &mut self.workspace_host.assistant;
        let waker = self.workspace_host.waker.as_ref();
        let mut changed = host.poll_progress(waker, tx);
        let mut notice = None;
        if let Some(reply) = host
            .task
            .as_mut()
            .and_then(|task| WorkspaceHost::poll(task, waker, tx))
        {
            host.task = None;
            host.progress = None;
            let controller = &mut host.controller;
            match reply {
                Ok(Reply::Run(id, Ok(result))) => controller.complete(&id, result),
                Ok(Reply::Run(id, Err(error))) => controller.fail(&id, error),
                Ok(Reply::Confirm(id, Ok(result))) => controller.confirmation_complete(&id, result),
                Ok(Reply::Confirm(id, Err(error)) | Reply::Reject(id, Err(error))) => {
                    controller.confirmation_failed(&id, error);
                }
                Ok(Reply::Reject(id, Ok(()))) => controller.rejection_complete(&id),
                Ok(Reply::Undo(id, mutation, Ok(receipt))) => {
                    controller.mutation_undo_complete(&id, &mutation, receipt)
                }
                Ok(Reply::Undo(id, mutation, Err(error))) => {
                    controller.mutation_undo_failed(&id, &mutation, error)
                }
                Ok(Reply::SavedConnection(Ok(settings))) => {
                    controller.forget_probe();
                    host.connection = Some(ConnectionDraft::active(&settings));
                    host.settings = settings;
                    notice = Some("Assistant connection updated".to_owned());
                }
                Ok(Reply::SavedConnection(Err(error))) => notice = Some(error.to_string()),
                Ok(Reply::Probe(id, result)) => controller.finish_probe(id, result),
                Err(message) => {
                    controller.task =
                        AssistantTaskState::Failed(AssistantUiError::unavailable(message))
                }
            }
            changed = true;
        }
        if let Some((id, delay)) = host.controller.pending_expiry_delay(now_ms()) {
            if delay.is_zero() {
                changed |= host.controller.expire_pending(&id);
                if changed {
                    tx.wake_after(Duration::ZERO);
                }
            } else {
                tx.wake_after(delay);
            }
        }
        if let Some(notice) = notice {
            self.notify(notice, tx);
        }
        changed
    }
}
