use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use application::desktop_assistant::{
    AssistantController, AssistantMutationSummary, AssistantProgress, AssistantRunResult,
    AssistantScopeMode, AssistantTaskState, AssistantUiError, AssistantUiService,
    PendingReviewState,
};
use contracts::AgentWriteMode;
use futures::Stream;
use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use meetnote::assistant_service::DesktopAssistantService;
use rune::{TextEdit, Tx};
use settings::{AssistantSettings, SettingsError, SettingsService};

use super::{LiveWorkspace, Task, WorkspaceHost, processing::ProcessingConfig};
use crate::MeetNote;

#[derive(Default)]
pub(crate) struct AssistantHost {
    pub controller: AssistantController,
    pub prompt: TextEdit,
    pub settings: AssistantSettings,
    pub behaviour: BehaviourDraft,
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
    Saved(Result<AssistantSettings, AssistantUiError>),
}

/// The Settings page's Assistant behaviour form. Providers and the model
/// choice have their own page.
#[derive(Default)]
pub(crate) struct BehaviourDraft {
    pub enabled: bool,
    pub confirm_writes: bool,
    pub tool_rounds: TextEdit,
    pub validation_retries: TextEdit,
}

impl BehaviourDraft {
    fn new(settings: &AssistantSettings) -> Self {
        Self {
            enabled: settings.enabled,
            confirm_writes: settings.write_mode == AgentWriteMode::ConfirmEach,
            tool_rounds: TextEdit::new(settings.max_tool_rounds.to_string()),
            validation_retries: TextEdit::new(settings.max_validation_retries.to_string()),
        }
    }

    /// `settings` with the form's values, or which field is not a number.
    fn apply(&self, settings: &AssistantSettings) -> Result<AssistantSettings, String> {
        Ok(AssistantSettings {
            enabled: self.enabled,
            write_mode: if self.confirm_writes {
                AgentWriteMode::ConfirmEach
            } else {
                AgentWriteMode::Automatic
            },
            max_tool_rounds: number(&self.tool_rounds, "maximum tool rounds")?,
            max_validation_retries: number(&self.validation_retries, "maximum validation retries")?,
            ..settings.clone()
        })
    }
}

fn number<T: FromStr>(value: &TextEdit, field: &str) -> Result<T, String> {
    value
        .text
        .trim()
        .parse()
        .map_err(|_| format!("Enter a valid number for {field}"))
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
        self.behaviour = BehaviourDraft::new(&settings);
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
    }
}

fn service(
    config: &ProcessingConfig,
    live: LiveWorkspace,
) -> Result<DesktopAssistantService, AssistantUiError> {
    let providers = config.providers().map_err(AssistantUiError::unavailable)?;
    Ok(DesktopAssistantService::with_job_runtime(
        providers,
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
    /// Saves the behaviour form unless the Assistant settings changed since
    /// they were loaded.
    pub(crate) fn save_assistant_behaviour(&mut self, tx: &mut Tx) {
        if self.workspace_host.busy() {
            return;
        }
        let config = self.workspace_host.processing_config.clone();
        let host = &mut self.workspace_host.assistant;
        let expected = host.settings.clone();
        let updated = match host.behaviour.apply(&expected) {
            Ok(updated) => updated,
            Err(message) => {
                self.notify(message, tx);
                return;
            }
        };
        host.start(tx, move || {
            Reply::Saved((|| {
                let settings = config
                    .settings_service()
                    .map_err(AssistantUiError::unavailable)?;
                let snapshot = settings.snapshot()?;
                if snapshot.assistant != expected {
                    return Err(SettingsError::RevisionConflict.into());
                }
                let saved = settings.edit(&SettingsService::revision_of(&snapshot), |all| {
                    all.assistant = updated;
                    Ok(())
                })?;
                Ok(saved.assistant)
            })())
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
                Ok(Reply::Saved(Ok(settings))) => {
                    host.behaviour = BehaviourDraft::new(&settings);
                    host.settings = settings;
                    notice = Some("Assistant settings saved".to_owned());
                }
                Ok(Reply::Saved(Err(error))) => notice = Some(error.to_string()),
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
