//! L2: the Assistant page and the Assistant section of Settings, driven
//! through the rendered page with a fake `AssistantUiService`: a run that
//! wrote twice before it failed or was cancelled, the review queue of a
//! paused run, the write mode's explanation, connection probes and the
//! credential a changed connection target needs.
//!
//! Service calls run on `runtime::spawn_blocking` workers; `wait_for`
//! drives GPUI with `run_until_parked` until their outcome is projected.
//! The fake's gates hold a call on its worker, so a test can act while it
//! is in flight.

use crate::assistant_ui::{
    AssistantMutationSummary, AssistantPendingReview, AssistantRunResult, AssistantTaskState,
    AssistantUiError, ConnectionDiagnostic, ConnectionProbeKind, PendingReviewState,
    SETTINGS_CREDENTIAL_TARGET_CONFLICT,
};
use crate::controller::RootView;
use crate::document_tests::{leak, rendered, within, View};
use crate::route::Route;
use crate::tests::{
    click_selector, empty_result, field_value, focus_field, hold_a_worker_alone, open_desktop_with,
    pending_review, start_run, stored_key_settings, wait_for, FakeAssistantUiService, Field, Gate,
    STORED_KEY,
};
use gpui::{TestAppContext, VisualTestContext};
use runtime::{JobEvent, JobId, Progress};
use std::sync::Arc;
use std::time::Duration;

fn warning(code: &str, retryable: bool) -> AssistantUiError {
    AssistantUiError {
        code: code.into(),
        message: format!("{code} after two writes"),
        retryable,
    }
}

fn write(id: &str) -> AssistantMutationSummary {
    AssistantMutationSummary {
        mutation_id: id.into(),
        affected_documents: vec![crate::assistant_ui::AssistantDocumentChange {
            document_id: "first".into(),
            previous_revision: Some(1),
            revision: Some(2),
            deleted: false,
        }],
        undo_token: format!("undo-{id}"),
        uncovered_segment_ids: Vec::new(),
    }
}

/// What a run returns when it stopped with `stopped` after two writes, as
/// the desktop service maps it: the error becomes a warning beside the
/// receipts, and the provider's notice is a second warning.
fn stopped_after_two_writes(stopped: AssistantUiError) -> AssistantRunResult {
    AssistantRunResult {
        warning: Some(stopped.clone()),
        warnings: vec![stopped, warning("PROVIDER_NOTICE", false)],
        mutation: Some(write("write-2")),
        mutations: vec![write("write-1"), write("write-2")],
        ..empty_result()
    }
}

fn shown(cx: &mut VisualTestContext, selector: String) -> bool {
    cx.run_until_parked();
    cx.debug_bounds(leak(selector)).is_some()
}

/// Both receipts with their undo entries lie in the receipts, both
/// warnings in the result card, and the task shows `task`.
#[track_caller]
fn assert_receipts_and_warnings(cx: &mut VisualTestContext, task: &'static str) {
    cx.run_until_parked();
    let card = rendered(cx, "assistant-result");
    let receipts = rendered(cx, "assistant-receipts");
    for id in ["write-1", "write-2"] {
        let receipt = rendered(cx, leak(format!("assistant-receipt-{id}")));
        assert!(within(receipt, receipts), "the receipt of {id} is listed");
        let undo = rendered(cx, leak(format!("assistant-undo-{id}")));
        assert!(within(undo, receipt), "{id} has its own undo entry");
    }
    for index in 0..2 {
        let warning = rendered(cx, leak(format!("assistant-warning-{index}")));
        assert!(within(warning, card), "warning {index} lies in the result");
    }
    assert!(cx.debug_bounds(task).is_some(), "the task shows `{task}`");
}

/// Undoes both writes by their own entries; while an undo is in flight no
/// other run starts.
fn undo_both(
    cx: &mut VisualTestContext,
    view: &View,
    assistant: &FakeAssistantUiService,
    gate: &Gate,
) {
    gate.arm();
    click_selector(cx, "assistant-undo-write-1");
    wait_for(cx, view, |_| gate.counts().0 == 1);
    assert!(view.read_with(cx, |root, _| root
        .assistant
        .is_mutation_undo_in_flight("write-1")));
    let task = view.read_with(cx, |root, _| root.assistant.task.clone());
    click_selector(cx, "assistant-submit");
    assert_no_new_run(cx, view, assistant, 1, &task);
    assert!(view.read_with(cx, |root, _| root
        .assistant
        .is_mutation_undo_in_flight("write-1")));
    gate.release();
    wait_for(cx, view, |root| {
        root.assistant.is_mutation_undone("write-1")
    });
    assert!(shown(cx, "assistant-undone-write-1".into()));
    assert!(!shown(cx, "assistant-undone-write-2".into()));

    click_selector(cx, "assistant-undo-write-2");
    wait_for(cx, view, |root| {
        root.assistant.is_mutation_undone("write-2")
    });
    assert_eq!(
        assistant.undo_calls.lock().unwrap().as_slice(),
        ["undo-write-1", "undo-write-2"]
    );
}

/// A run changes controller state before it queues a worker. Check that
/// state as well as calls already received, so a queued worker cannot make
/// a negative assertion pass simply because it has not run yet.
#[track_caller]
fn assert_no_new_run(
    cx: &mut VisualTestContext,
    view: &View,
    assistant: &FakeAssistantUiService,
    runs: usize,
    task: &AssistantTaskState,
) {
    cx.run_until_parked();
    view.read_with(cx, |root, _| {
        assert_eq!(&root.assistant.task, task, "no replacement run is queued");
        assert!(
            root.assistant_busy(),
            "the existing operation stays in flight"
        );
    });
    assert_eq!(
        assistant.run_request_ids.lock().unwrap().len(),
        runs,
        "no replacement run reaches the service"
    );
}

/// Everything the page shows of the Assistant, to compare before and after
/// late events.
fn page_state(cx: &mut VisualTestContext, view: &View) -> String {
    cx.run_until_parked();
    view.read_with(cx, |root, _| {
        format!("{:?} | {}", root.assistant, root.ui.notice)
    })
}

/// Job events of `run` that arrive after its result change nothing.
#[track_caller]
fn late_events_change_nothing(
    cx: &mut VisualTestContext,
    view: &View,
    assistant: &FakeAssistantUiService,
    run: &str,
    task: &'static str,
) {
    let before = page_state(cx, view);
    let job_id = JobId(41);
    for event in [
        JobEvent::Progress {
            job_id,
            progress: Progress {
                completed: 9,
                total: Some(9),
            },
        },
        JobEvent::Failed {
            job_id,
            message: "late failure".into(),
        },
        JobEvent::Cancelled { job_id },
    ] {
        assistant.publish_job(Some(run), event);
    }
    for _ in 0..20 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(page_state(cx, view), before, "late events change nothing");
    assert_receipts_and_warnings(cx, task);
}

fn open_assistant(
    cx: &mut TestAppContext,
    assistant: FakeAssistantUiService,
) -> (View, &mut VisualTestContext, Arc<FakeAssistantUiService>) {
    let assistant = Arc::new(assistant);
    let (view, cx) = open_desktop_with(cx, assistant.clone(), stored_key_settings());
    (view, cx, assistant)
}

#[gpui::test]
fn a_run_that_fails_after_two_writes_keeps_every_receipt_warning_and_undo(cx: &mut TestAppContext) {
    let _alone = hold_a_worker_alone();
    let gate = Gate::default();
    let (view, cx, assistant) = open_assistant(
        cx,
        FakeAssistantUiService {
            decision_gate: Some(gate.clone()),
            ..Default::default()
        },
    );
    *assistant.run_result.lock().unwrap() =
        Some(stopped_after_two_writes(warning("PROVIDER_TIMEOUT", true)));
    start_run(cx, &view);
    wait_for(cx, &view, |root| root.assistant.result.is_some());
    assert_receipts_and_warnings(cx, "assistant-task-partial");
    assert!(
        !shown(cx, "assistant-retry".into()),
        "a run that wrote is undone, not retried"
    );

    undo_both(cx, &view, &assistant, &gate);
    assert_receipts_and_warnings(cx, "assistant-task-partial");
    let run = assistant.run_request_ids.lock().unwrap()[0].clone();
    late_events_change_nothing(cx, &view, &assistant, &run, "assistant-task-partial");
}

#[gpui::test]
fn a_cancelled_run_keeps_the_receipts_of_the_writes_it_applied(cx: &mut TestAppContext) {
    let _alone = hold_a_worker_alone();
    let (run_gate, decision_gate) = (Gate::default(), Gate::default());
    let (view, cx, assistant) = open_assistant(
        cx,
        FakeAssistantUiService {
            run_gate: Some(run_gate.clone()),
            decision_gate: Some(decision_gate.clone()),
            ..Default::default()
        },
    );
    *assistant.run_result.lock().unwrap() =
        Some(stopped_after_two_writes(warning("TASK_CANCELLED", false)));
    run_gate.arm();
    start_run(cx, &view);
    wait_for(cx, &view, |_| run_gate.counts().0 == 1);
    let run = assistant.run_request_ids.lock().unwrap()[0].clone();

    click_selector(cx, "assistant-cancel");
    wait_for(cx, &view, |root| {
        matches!(root.assistant.task, AssistantTaskState::Cancelling { .. })
    });
    assert!(shown(cx, "assistant-task-cancelling".into()));
    // The job ends before the run's result, with its receipts, arrives.
    assistant.publish_job(Some(&run), JobEvent::Cancelled { job_id: JobId(3) });
    wait_for(cx, &view, |root| {
        root.assistant.task == AssistantTaskState::Cancelled
    });
    let task = view.read_with(cx, |root, _| root.assistant.task.clone());
    click_selector(cx, "assistant-submit");
    assert_no_new_run(cx, &view, &assistant, 1, &task);

    run_gate.release();
    wait_for(cx, &view, |root| root.assistant.result.is_some());
    assert_receipts_and_warnings(cx, "assistant-task-partial");
    undo_both(cx, &view, &assistant, &decision_gate);
    late_events_change_nothing(cx, &view, &assistant, &run, "assistant-task-partial");
}

#[gpui::test]
fn a_retryable_run_failure_offers_retry_and_the_retry_succeeds(cx: &mut TestAppContext) {
    let (view, cx, assistant) = open_assistant(cx, FakeAssistantUiService::default());
    assistant.run_errors.lock().unwrap().extend([
        warning("PROVIDER_AUTHENTICATION_FAILED", false),
        warning("PROVIDER_TIMEOUT", true),
    ]);
    *assistant.run_result.lock().unwrap() = Some(AssistantRunResult {
        answer: Some("done".into()),
        ..empty_result()
    });
    start_run(cx, &view);
    wait_for(cx, &view, |root| {
        matches!(root.assistant.task, AssistantTaskState::Failed(_))
    });
    assert!(
        !shown(cx, "assistant-retry".into()),
        "an error that is not retryable offers no retry"
    );

    click_selector(cx, "assistant-submit");
    wait_for(
        cx,
        &view,
        |root| matches!(&root.assistant.task, AssistantTaskState::Failed(error) if error.retryable),
    );
    assert!(shown(cx, "assistant-task-failed".into()));
    click_selector(cx, "assistant-retry");
    wait_for(cx, &view, |root| {
        root.assistant.task == AssistantTaskState::Completed
    });
    assert_eq!(
        assistant.run_prompts.lock().unwrap().as_slice(),
        ["organize", "organize", "organize"],
        "the retry sends the failed prompt again"
    );
    assert!(!shown(cx, "assistant-retry".into()));
}

fn confirmed(id: &str) -> AssistantRunResult {
    AssistantRunResult {
        mutation: Some(write(id)),
        mutations: vec![write(id)],
        ..empty_result()
    }
}

fn review_focused(cx: &mut VisualTestContext, view: &View) -> bool {
    cx.run_until_parked();
    let focus = view.read_with(cx, |root, _| root.review_focus.clone());
    cx.update(|window, _| focus.is_focused(window))
}

#[gpui::test]
fn the_review_queue_advances_past_expiry_and_focuses_the_next_review(cx: &mut TestAppContext) {
    let _alone = hold_a_worker_alone();
    let gate = Gate::default();
    let (view, cx, assistant) = open_assistant(
        cx,
        FakeAssistantUiService {
            decision_gate: Some(gate.clone()),
            ..Default::default()
        },
    );
    let expiring = pending_review("review-1", crate::assistant_ui::now_ms() + 60_000);
    *assistant.run_result.lock().unwrap() = Some(AssistantRunResult {
        pending_reviews: vec![
            expiring,
            pending_review("review-2", u64::MAX),
            pending_review("review-3", u64::MAX),
        ],
        ..empty_result()
    });
    *assistant.confirm_result.lock().unwrap() = Some(confirmed("accepted-2"));
    start_run(cx, &view);
    wait_for(cx, &view, |root| root.assistant.pending.is_some());
    // The run is paused at the first of three reviews.
    for selector in [
        "assistant-task-awaiting",
        "assistant-pending-review-1",
        "assistant-pending-position-1-of-3",
        "assistant-pending-paused",
        "assistant-pending-expires",
    ] {
        assert!(shown(cx, selector.into()), "`{selector}` is shown");
    }

    // The user types the next request while the first review expires.
    click_selector(cx, "assistant-input");
    cx.executor().advance_clock(Duration::from_secs(61));
    assert!(shown(cx, "assistant-pending-review-2".into()));
    assert!(shown(cx, "assistant-pending-position-2-of-3".into()));
    assert!(!shown(cx, "assistant-pending-review-1".into()));
    assert!(
        !review_focused(cx, &view),
        "an expiry does not take the focus from the prompt"
    );

    // Accepting: both decisions and a new run wait until it is written.
    gate.arm();
    click_selector(cx, "assistant-confirm");
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    assert!(shown(cx, "assistant-review-accepting".into()));
    let task = view.read_with(cx, |root, _| root.assistant.task.clone());
    click_selector(cx, "assistant-confirm");
    click_selector(cx, "assistant-reject");
    click_selector(cx, "assistant-submit");
    assert_no_new_run(cx, &view, &assistant, 1, &task);
    assert_eq!(assistant.grants_for("confirm").len(), 1);
    assert!(assistant.grants_for("reject").is_empty());
    gate.release();
    wait_for(cx, &view, |root| {
        matches!(
            &root.assistant.pending,
            Some(PendingReviewState::Awaiting(review)) if review.confirmation_id == "review-3"
        )
    });
    assert!(shown(cx, "assistant-pending-position-3-of-3".into()));
    assert!(review_focused(cx, &view), "focus lands on the next review");
    let card = rendered(cx, "assistant-pending-review-3");
    assert!(within(rendered(cx, "assistant-confirm"), card));
    assert!(shown(cx, "assistant-receipt-accepted-2".into()));
    assert!(shown(cx, "assistant-undo-accepted-2".into()));

    // Rejecting the last one.
    gate.arm();
    click_selector(cx, "assistant-reject");
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    assert!(shown(cx, "assistant-review-rejecting".into()));
    click_selector(cx, "assistant-confirm");
    assert_eq!(assistant.grants_for("confirm").len(), 1);
    gate.release();
    wait_for(cx, &view, |root| {
        root.assistant.pending == Some(PendingReviewState::Rejected)
    });
    assert!(shown(cx, "assistant-pending-rejected".into()));
    assert!(shown(cx, "assistant-task-completed".into()));
}

#[gpui::test]
fn reviews_over_the_queue_limit_are_noticed_on_the_page(cx: &mut TestAppContext) {
    let (view, cx, assistant) = open_assistant(cx, FakeAssistantUiService::default());
    *assistant.run_result.lock().unwrap() = Some(AssistantRunResult {
        pending_reviews: (0..10)
            .map(|index| pending_review(&format!("review-{index}"), u64::MAX))
            .collect::<Vec<AssistantPendingReview>>(),
        ..empty_result()
    });
    start_run(cx, &view);
    wait_for(cx, &view, |root| root.assistant.pending.is_some());
    let results = rendered(cx, "assistant-results");
    let notice = rendered(cx, "assistant-pending-overflow");
    assert!(within(notice, results));
    assert!(shown(cx, "assistant-pending-overflow-2".into()));
    assert!(shown(cx, "assistant-pending-position-1-of-8".into()));
}

/// Scrolls the results to their end and draws the page.
fn scroll_results_to_end(cx: &mut VisualTestContext, view: &View) {
    let scroll = view.read_with(cx, |root, _| root.scroll.assistant_results.clone());
    assert!(
        scroll.max_offset().y > gpui::px(0.),
        "the results overflow the page"
    );
    scroll.scroll_to_bottom();
    cx.update(|window, cx| window.draw(cx).clear());
    cx.run_until_parked();
}

#[gpui::test]
fn the_last_card_scrolls_clear_of_the_input_bar_and_nothing_shows_below_it(
    cx: &mut TestAppContext,
) {
    let (view, cx, assistant) = open_assistant(cx, FakeAssistantUiService::default());
    *assistant.run_result.lock().unwrap() = Some(AssistantRunResult {
        answer: Some("done".into()),
        mutations: (0..12)
            .map(|index| write(&format!("write-{index}")))
            .collect(),
        pending_reviews: (0..10)
            .map(|index| pending_review(&format!("review-{index}"), u64::MAX))
            .collect(),
        ..empty_result()
    });
    start_run(cx, &view);
    wait_for(cx, &view, |root| root.assistant.pending.is_some());
    scroll_results_to_end(cx, &view);

    let bar = rendered(cx, "assistant-input-bar");
    let results = rendered(cx, "assistant-results");
    let last = rendered(cx, "assistant-pending-overflow");
    assert!(
        last.bottom() <= bar.top(),
        "the last card ends above the bar: {last:?} and {bar:?}"
    );
    assert!(
        results.top() < bar.top() && results.bottom() <= bar.bottom(),
        "the results run under the bar but end with it: {results:?} and {bar:?}"
    );
}

#[gpui::test]
fn the_sent_request_is_quoted_above_its_result(cx: &mut TestAppContext) {
    let _alone = hold_a_worker_alone();
    let gate = Gate::default();
    let (view, cx, assistant) = open_assistant(
        cx,
        FakeAssistantUiService {
            run_gate: Some(gate.clone()),
            ..Default::default()
        },
    );
    assert!(
        !shown(cx, "assistant-request".into()),
        "nothing is quoted before a run"
    );
    *assistant.run_result.lock().unwrap() = Some(AssistantRunResult {
        answer: Some("done".into()),
        ..empty_result()
    });
    gate.arm();
    start_run(cx, &view);
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    let results = rendered(cx, "assistant-results");
    let request = rendered(cx, "assistant-request");
    assert!(
        within(request, results),
        "the request is quoted while the run goes on"
    );
    gate.release();
    wait_for(cx, &view, |root| root.assistant.result.is_some());
    let request = rendered(cx, "assistant-request");
    let result = rendered(cx, "assistant-result");
    assert!(
        request.bottom() <= result.top() && within(request, rendered(cx, "assistant-results")),
        "the request leads the results: {request:?} above {result:?}"
    );
}

#[gpui::test]
fn a_scene_tab_selects_and_explains_its_scene_and_leaves_focus_in_the_prompt(
    cx: &mut TestAppContext,
) {
    let (view, cx, _) = open_assistant(cx, FakeAssistantUiService::default());
    assert!(shown(cx, "assistant-scene-selected-ask".into()));
    assert!(shown(cx, "assistant-scene-description-ask".into()));
    let tabs = rendered(cx, "assistant-scene-group");
    let description = rendered(cx, "assistant-scene-description-ask");
    assert!(
        description.top() >= tabs.bottom() && description.left() == tabs.left(),
        "the explanation sits under the tabs"
    );

    let prompt = view.read_with(cx, |root, _| root.assistant_inputs.prompt.clone());
    focus_field(cx, &prompt);
    click_selector(cx, "assistant-scene-organize");
    assert_eq!(
        view.read_with(cx, |root, _| root.assistant.scene),
        contracts::AgentScene::Organize
    );
    assert!(shown(cx, "assistant-scene-selected-organize".into()));
    assert!(!shown(cx, "assistant-scene-selected-ask".into()));
    assert!(shown(cx, "assistant-scene-description-organize".into()));
    assert!(!shown(cx, "assistant-scene-description-ask".into()));
    assert!(
        cx.update(|window, cx| {
            gpui::Focusable::focus_handle(prompt.read(cx), cx).contains_focused(window, cx)
        }),
        "a press on a tab leaves focus in the prompt"
    );
}

#[gpui::test]
fn the_scope_and_its_context_share_one_line_above_the_privacy_note(cx: &mut TestAppContext) {
    let (_view, cx, _) = open_assistant(cx, FakeAssistantUiService::default());
    cx.run_until_parked();
    let scope = rendered(cx, "assistant-scope-group");
    let context = rendered(cx, "assistant-context");
    let mode = rendered(cx, "assistant-write-mode-automatic");
    let privacy = rendered(cx, "assistant-privacy");
    assert!(
        context.left() >= scope.right()
            && context.top() < scope.bottom()
            && scope.top() < context.bottom(),
        "the context sits beside the scope: {context:?} and {scope:?}"
    );
    assert!(
        within(mode, context),
        "the write mode is part of the context"
    );
    assert!(
        privacy.top() >= scope.bottom() && privacy.top() >= context.bottom(),
        "the privacy note sits under the line"
    );
    assert!(
        privacy.bottom() <= rendered(cx, "assistant-body").top(),
        "the results start below the context"
    );
}

/// Opens Settings at its Assistant section.
fn open_assistant_settings(cx: &mut VisualTestContext, view: &View) {
    view.update(cx, |root, cx| {
        root.navigate(Route::Settings, cx);
        root.edit_assistant_profile("local", cx);
    });
    cx.run_until_parked();
    view.update(cx, |root, cx| {
        root.scroll
            .settings
            .scroll_to_top_of_item(crate::pages::settings::ASSISTANT_SECTION);
        cx.notify();
    });
    cx.run_until_parked();
}

/// The taller grouped form requires scrolling a control into the viewport
/// before the simulated pointer can press it.
fn click_assistant_setting(cx: &mut VisualTestContext, view: &View, selector: &'static str) {
    for _ in 0..3 {
        let bounds = rendered(cx, selector);
        let scroll = view.read_with(cx, |root, _| root.scroll.settings.clone());
        let viewport = scroll.bounds();
        let delta = if bounds.bottom() > viewport.bottom() {
            viewport.bottom() - bounds.bottom() - gpui::px(8.)
        } else if bounds.top() < viewport.top() {
            viewport.top() - bounds.top() + gpui::px(8.)
        } else {
            break;
        };
        scroll.set_offset(gpui::point(scroll.offset().x, scroll.offset().y + delta));
        cx.update(|window, cx| window.draw(cx).clear());
    }
    click_selector(cx, selector);
}

#[gpui::test]
fn turning_automatic_write_back_off_explains_confirm_each(cx: &mut TestAppContext) {
    let settings = stored_key_settings();
    let assistant = Arc::new(FakeAssistantUiService {
        settings: Some(settings.clone()),
        ..Default::default()
    });
    let (view, cx) = open_desktop_with(cx, assistant, settings.clone());
    open_assistant_settings(cx, &view);
    assert!(shown(cx, "settings-write-mode-notice-automatic".into()));
    assert!(!shown(cx, "settings-write-mode-unsaved".into()));

    click_assistant_setting(cx, &view, "settings-write-mode-confirm-each");
    assert!(shown(cx, "settings-write-mode-notice-confirm-each".into()));
    assert!(!shown(cx, "settings-write-mode-notice-automatic".into()));
    assert!(
        shown(cx, "settings-write-mode-unsaved".into()),
        "the change waits for Save"
    );
    click_assistant_setting(cx, &view, "assistant-save-settings");
    wait_for(cx, &view, |root| {
        settings.snapshot().unwrap().assistant.write_mode == contracts::AgentWriteMode::ConfirmEach
            && root.saved_write_mode() == contracts::AgentWriteMode::ConfirmEach
    });
    assert!(shown(cx, "settings-write-mode-notice-confirm-each".into()));
    assert!(!shown(cx, "settings-write-mode-unsaved".into()));

    view.update(cx, |root, cx| root.navigate(Route::Assistant, cx));
    assert!(shown(cx, "assistant-write-mode-confirm-each".into()));
}

fn diagnostic(kind: ConnectionProbeKind) -> ConnectionDiagnostic {
    ConnectionDiagnostic {
        kind,
        ready: true,
        local: true,
        models: None,
        text: Some(true),
        client_tools: Some(true),
        requested_model: "assistant-default".into(),
        resolved_model: Some("assistant-default".into()),
        context_window_tokens: None,
        message: "ready".into(),
    }
}

fn probing(view: &View, cx: &mut VisualTestContext) -> Option<ConnectionProbeKind> {
    view.read_with(cx, |root, _| root.assistant.probe_running)
}

#[gpui::test]
fn connection_probes_succeed_fail_retry_and_cancel_off_the_ui_thread(cx: &mut TestAppContext) {
    let _alone = hold_a_worker_alone();
    let gate = Gate::default();
    let (view, cx, assistant) = open_assistant(
        cx,
        FakeAssistantUiService {
            probe_gate: Some(gate.clone()),
            ..Default::default()
        },
    );
    assistant.probe_results.lock().unwrap().extend([
        Ok(diagnostic(ConnectionProbeKind::Readiness)),
        Err(warning("PROVIDER_TIMEOUT", true)),
        Ok(diagnostic(ConnectionProbeKind::Model)),
        Ok(diagnostic(ConnectionProbeKind::Readiness)),
        Err(warning("PROVIDER_AUTHENTICATION_FAILED", false)),
    ]);
    open_assistant_settings(cx, &view);

    click_assistant_setting(cx, &view, "assistant-readiness");
    wait_for(cx, &view, |root| {
        matches!(root.assistant.diagnostic, Some(Ok(_)))
    });
    assert!(shown(cx, "assistant-probe-result".into()));

    click_assistant_setting(cx, &view, "assistant-model-probe");
    wait_for(cx, &view, |root| {
        matches!(root.assistant.diagnostic, Some(Err(_)))
    });
    assert!(shown(cx, "assistant-probe-error".into()));
    click_assistant_setting(cx, &view, "assistant-probe-retry");
    wait_for(cx, &view, |root| {
        matches!(root.assistant.diagnostic, Some(Ok(_)))
    });
    assert!(shown(cx, "assistant-probe-result".into()));
    assert!(!shown(cx, "assistant-probe-retry".into()));
    assert_eq!(
        assistant.probe_calls.lock().unwrap().as_slice(),
        [
            ConnectionProbeKind::Readiness,
            ConnectionProbeKind::Model,
            ConnectionProbeKind::Model
        ],
        "the retry repeats the failed probe"
    );

    // A probe in flight can be cancelled; its late answer is dropped.
    gate.arm();
    click_assistant_setting(cx, &view, "assistant-readiness");
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    assert!(shown(cx, "assistant-probe-running".into()));
    click_assistant_setting(cx, &view, "assistant-model-probe");
    assert_eq!(
        assistant.probe_calls.lock().unwrap().len(),
        4,
        "no second probe starts while one runs"
    );
    click_assistant_setting(cx, &view, "assistant-cancel-probe");
    assert_eq!(probing(&view, cx), None);
    gate.release();
    for _ in 0..20 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(shown(cx, "assistant-probe-cancelled".into()));
    assert!(!shown(cx, "assistant-probe-result".into()));

    click_assistant_setting(cx, &view, "assistant-readiness");
    wait_for(cx, &view, |root| {
        matches!(root.assistant.diagnostic, Some(Err(_)))
    });
    assert!(shown(cx, "assistant-probe-error".into()));
    assert!(
        !shown(cx, "assistant-probe-retry".into()),
        "an error that is not retryable offers no retry"
    );
}

/// Replaces the text of `field` as a user would.
fn retype(cx: &mut VisualTestContext, field: &Field, text: &str) {
    focus_field(cx, field);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input(text);
    assert_eq!(field_value(cx, field), text);
}

#[gpui::test]
fn an_endpoint_changed_without_the_key_is_refused_next_to_the_key_field(cx: &mut TestAppContext) {
    let settings = stored_key_settings();
    let assistant = Arc::new(FakeAssistantUiService {
        settings: Some(settings.clone()),
        ..Default::default()
    });
    let (view, cx) = open_desktop_with(cx, assistant, settings.clone());
    open_assistant_settings(cx, &view);
    assert!(
        shown(cx, "assistant-api-key-kept".into()),
        "the stored key is kept while the field is empty"
    );
    let (base_url, api_key) = view.read_with(cx, |root, _| {
        (
            root.assistant_inputs.base_url.clone(),
            root.assistant_inputs.api_key.clone(),
        )
    });

    retype(cx, &base_url, "http://127.0.0.1:4010");
    click_assistant_setting(cx, &view, "assistant-save-settings");
    wait_for(cx, &view, |root: &RootView| root.settings_error.is_some());
    let error = view.read_with(cx, |root, _| root.settings_error.clone().unwrap());
    assert_eq!(error.code, SETTINGS_CREDENTIAL_TARGET_CONFLICT);
    let message = rendered(cx, "assistant-credential-target-conflict");
    let key = rendered(cx, "assistant-api-key-field");
    let key_file = rendered(cx, "assistant-api-key-file-field");
    assert!(
        message.top() >= key.bottom() && message.bottom() <= key_file.top(),
        "the message sits under the key field"
    );
    let stored = settings.snapshot().unwrap().assistant.connections[0].clone();
    assert_eq!(stored.base_url, "http://127.0.0.1:4000", "nothing is saved");
    assert_eq!(stored.api_key.as_deref(), Some(STORED_KEY));
    assert_eq!(field_value(cx, &base_url), "http://127.0.0.1:4010");

    retype(cx, &api_key, "sk-new-target");
    click_assistant_setting(cx, &view, "assistant-save-settings");
    wait_for(cx, &view, |_| {
        settings.snapshot().unwrap().assistant.connections[0].base_url == "http://127.0.0.1:4010"
    });
    let stored = settings.snapshot().unwrap().assistant.connections[0].clone();
    assert_eq!(stored.api_key.as_deref(), Some("sk-new-target"));
    wait_for(cx, &view, |root| root.settings_error.is_none());
    assert!(!shown(cx, "assistant-credential-target-conflict".into()));
}
