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
    pending_review, start_run, stored_key_settings, test_settings, wait_for,
    FakeAssistantUiService, Field, Gate, STORED_KEY,
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

/// The actual default empty workspace, with an optional saved connection.
fn first_use_assistant(
    cx: &mut TestAppContext,
    connected: bool,
) -> (View, &mut VisualTestContext, Arc<FakeAssistantUiService>) {
    let assistant = Arc::new(FakeAssistantUiService::default());
    let settings = if connected {
        stored_key_settings()
    } else {
        test_settings()
    };
    let (view, cx) = open_desktop_with(cx, assistant.clone(), settings);
    cx.simulate_resize(crate::pages::chrome::MIN_WINDOW_SIZE);
    cx.run_until_parked();
    assert!(view.read_with(cx, |root, _| root.projects.is_empty()));
    (view, cx, assistant)
}

fn tab_to_first_use_action(cx: &mut VisualTestContext, selector: &'static str) {
    for _ in 0..40 {
        cx.simulate_keystrokes("tab");
        cx.run_until_parked();
        if cx
            .debug_bounds("focus-ring")
            .is_some_and(|ring| within(ring, rendered(cx, selector)))
        {
            return;
        }
    }
    panic!("Tab never reached {selector}");
}

fn press_enter(cx: &mut VisualTestContext) {
    let keystroke = gpui::Keystroke::parse("enter").unwrap();
    cx.simulate_event(gpui::KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(gpui::KeyUpEvent { keystroke });
    cx.run_until_parked();
}

#[gpui::test]
fn first_use_offers_connection_settings_without_losing_a_rejected_question(
    cx: &mut TestAppContext,
) {
    let (view, cx, assistant) = first_use_assistant(cx, false);
    let action = rendered(cx, "assistant-first-settings");
    assert!(within(action, rendered(cx, "assistant-results")));
    assert!(
        !shown(cx, "assistant-first-import".into()),
        "connection comes before import"
    );
    let prompt = view.read_with(cx, |root, _| root.assistant_inputs.prompt.clone());
    retype(cx, &prompt, "keep this question");
    assert!(
        cx.update(
            |window, cx| gpui::Focusable::focus_handle(prompt.read(cx), cx)
                .contains_focused(window, cx)
        ),
        "drawing guidance does not move focus"
    );
    click_selector(cx, "assistant-submit");
    view.read_with(cx, |root, _| {
        assert_eq!(root.assistant.task, AssistantTaskState::Idle);
        assert!(root.ui.notice.contains("CONNECTION_NOT_CONFIGURED"));
    });
    rendered(cx, "assistant-first-settings");
    assert!(assistant.run_request_ids.lock().unwrap().is_empty());
    click_selector(cx, "assistant-first-settings");
    view.read_with(cx, |root, _| {
        assert_eq!(root.ui.route, Route::Settings);
        assert_eq!(
            root.ui.settings_section,
            crate::pages::settings::ASSISTANT_SECTION
        );
    });
    assert_eq!(field_value(cx, &prompt), "keep this question");
}

#[gpui::test]
fn first_use_connection_settings_is_keyboard_reachable_at_minimum_size(cx: &mut TestAppContext) {
    let (view, cx, _) = first_use_assistant(cx, false);
    assert!(within(
        rendered(cx, "assistant-first-settings"),
        rendered(cx, "assistant-results")
    ));
    tab_to_first_use_action(cx, "assistant-first-settings");
    press_enter(cx);
    view.read_with(cx, |root, _| {
        assert_eq!(root.ui.route, Route::Settings);
        assert_eq!(
            root.ui.settings_section,
            crate::pages::settings::ASSISTANT_SECTION
        );
    });
}

fn first_settings_jump_reveals_the_assistant_form(
    cx: &mut TestAppContext,
    locale: crate::i18n::Locale,
    keyboard: bool,
) {
    let (view, cx, _) = first_use_assistant(cx, false);
    view.update(cx, |root, cx| root.set_locale(locale, cx));
    wait_for(cx, &view, |root| {
        root.ui.notice == crate::i18n::text(locale, "language_changed")
    });
    if keyboard {
        tab_to_first_use_action(cx, "assistant-first-settings");
        press_enter(cx);
    } else {
        click_selector(cx, "assistant-first-settings");
    }
    assert_eq!(view.read_with(cx, |root, _| root.ui.route), Route::Settings);
    assert_assistant_form_visible(cx, locale);
}

fn assert_assistant_form_visible(cx: &mut VisualTestContext, locale: crate::i18n::Locale) {
    let viewport = rendered(cx, "settings-scroll");
    for key in [
        "assistant_enabled",
        "assistant_profiles",
        "assistant_connection_id",
    ] {
        let selector = leak(format!("setting-label-{}", crate::i18n::text(locale, key)));
        let label = rendered(cx, selector);
        assert!(
            within(label, viewport),
            "{key} must be visible after the first settings jump: {label:?} in {viewport:?}"
        );
    }
}

#[gpui::test]
fn the_first_chinese_settings_button_reveals_the_assistant_form(cx: &mut TestAppContext) {
    first_settings_jump_reveals_the_assistant_form(cx, crate::i18n::Locale::ZhCn, false);
}

#[gpui::test]
fn the_first_english_settings_button_reveals_the_assistant_form(cx: &mut TestAppContext) {
    first_settings_jump_reveals_the_assistant_form(cx, crate::i18n::Locale::EnUs, false);
}

#[gpui::test]
fn the_first_settings_keyboard_action_reveals_the_assistant_form(cx: &mut TestAppContext) {
    first_settings_jump_reveals_the_assistant_form(cx, crate::i18n::Locale::EnUs, true);
}

#[gpui::test]
fn returning_to_assistant_settings_keeps_the_question_and_reveals_the_form(
    cx: &mut TestAppContext,
) {
    let (view, cx, _) = first_use_assistant(cx, false);
    let prompt = view.read_with(cx, |root, _| root.assistant_inputs.prompt.clone());
    retype(cx, &prompt, "keep my question while I connect");
    click_selector(cx, "assistant-first-settings");
    assert_assistant_form_visible(cx, crate::i18n::Locale::ZhCn);
    click_selector(cx, "settings-section-0");
    click_selector(cx, "nav-Assistant");
    assert_eq!(field_value(cx, &prompt), "keep my question while I connect");
    click_selector(cx, "assistant-first-settings");
    assert_assistant_form_visible(cx, crate::i18n::Locale::ZhCn);
    click_selector(cx, "nav-Assistant");
    assert_eq!(field_value(cx, &prompt), "keep my question while I connect");
}

#[gpui::test]
fn leaving_settings_before_layout_discards_the_assistant_jump(cx: &mut TestAppContext) {
    let (view, cx, _) = first_use_assistant(cx, false);
    view.update(cx, |root, cx| {
        root.open_assistant_settings(cx);
        root.navigate(Route::Assistant, cx);
    });
    cx.run_until_parked();
    click_selector(cx, "nav-Settings");
    let scroll = view.read_with(cx, |root, _| root.scroll.settings.clone());
    assert_eq!(scroll.offset().y, gpui::px(0.));
    assert_eq!(view.read_with(cx, |root, _| root.ui.route), Route::Settings);
}

#[gpui::test]
fn first_use_with_a_connection_offers_the_existing_audio_picker(cx: &mut TestAppContext) {
    let (view, cx, assistant) = first_use_assistant(cx, true);
    let action = rendered(cx, "assistant-first-import");
    assert!(within(action, rendered(cx, "assistant-results")));
    assert!(action.bottom() <= rendered(cx, "assistant-input-bar").top());
    assert!(!shown(cx, "assistant-first-settings".into()));
    let prompt = view.read_with(cx, |root, _| root.assistant_inputs.prompt.clone());
    retype(cx, &prompt, "keep this question too");
    click_selector(cx, "assistant-submit");
    view.read_with(cx, |root, _| {
        assert_eq!(root.assistant.task, AssistantTaskState::Idle);
        assert!(root.ui.notice.contains("DOCUMENT_ACCESS_DENIED"));
    });
    assert!(!cx.cx.did_prompt_for_paths());
    click_selector(cx, "assistant-first-import");
    assert!(cx.cx.did_prompt_for_paths());
    assert_eq!(field_value(cx, &prompt), "keep this question too");
    assert!(assistant.run_request_ids.lock().unwrap().is_empty());
}

#[gpui::test]
fn first_use_audio_import_is_keyboard_reachable_at_minimum_size(cx: &mut TestAppContext) {
    let (_view, cx, _) = first_use_assistant(cx, true);
    assert!(within(
        rendered(cx, "assistant-first-import"),
        rendered(cx, "assistant-results")
    ));
    tab_to_first_use_action(cx, "assistant-first-import");
    press_enter(cx);
    assert!(cx.cx.did_prompt_for_paths());
}

#[gpui::test]
fn first_use_guidance_never_replaces_an_existing_result_or_review(cx: &mut TestAppContext) {
    let (view, cx, _) = first_use_assistant(cx, false);
    // A result already received remains readable if the connection is later removed.
    view.update(cx, |root, cx| {
        let document =
            domain::DocumentState::new(domain::ProjectId::new("first"), "First", "first.wav");
        root.assistant.authorize_document(Some(&document));
        let settings = settings::AssistantSettings {
            active_connection_id: Some("previous-connection".into()),
            ..Default::default()
        };
        let (request, _) = root
            .assistant
            .begin("previous request".into(), &settings)
            .unwrap();
        root.assistant.complete(
            &request.request_id,
            AssistantRunResult {
                answer: Some("existing answer".into()),
                pending_reviews: vec![pending_review("existing-review", u64::MAX)],
                ..empty_result()
            },
        );
        cx.notify();
    });
    cx.run_until_parked();
    rendered(cx, "assistant-result");
    rendered(cx, "assistant-pending-existing-review");
    assert!(!shown(cx, "assistant-first-settings".into()));
    assert!(!shown(cx, "assistant-first-import".into()));
}

/// Runs the real import use case and its asynchronous controller completion.
/// The in-memory repository avoids user files; no audio processing is run.
fn complete_first_audio_import(view: &View, cx: &mut VisualTestContext) {
    view.read_with(cx, |root, _| {
        assert!(root.projects.is_empty());
        assert!(root.ui.selected_project.is_none());
    });
    view.update(cx, |root, cx| {
        root.import_audio_path("first-recording.wav".into(), cx)
    });
    wait_for(cx, view, |root| {
        root.projects.len() == 1 && !root.audio.pending()
    });
    view.read_with(cx, |root, _| {
        assert_eq!(
            root.ui.selected_project.as_ref(),
            Some(root.projects[0].id())
        );
        assert_eq!(
            root.projects[0].processing_status(),
            domain::ProjectStatus::Imported
        );
        assert!(!root.ui.processing);
    });
}

#[gpui::test]
fn imported_audio_offers_its_project_without_interrupting_the_question(cx: &mut TestAppContext) {
    let (view, cx, assistant) = first_use_assistant(cx, true);
    let prompt = view.read_with(cx, |root, _| root.assistant_inputs.prompt.clone());
    retype(cx, &prompt, "keep the question while importing");
    complete_first_audio_import(&view, cx);
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.route),
        Route::Assistant
    );
    assert!(cx.update(
        |window, cx| gpui::Focusable::focus_handle(prompt.read(cx), cx)
            .contains_focused(window, cx)
    ));
    let action = rendered(cx, "assistant-first-project");
    assert!(within(action, rendered(cx, "assistant-results")));
    assert!(action.bottom() <= rendered(cx, "assistant-input-bar").top());
    assert!(!shown(cx, "assistant-first-import".into()));
    let selected = view.read_with(cx, |root, _| root.ui.selected_project.clone());
    click_selector(cx, "assistant-first-project");
    view.read_with(cx, |root, _| {
        assert_eq!(root.ui.route, Route::Meetings);
        assert_eq!(root.ui.selected_project, selected);
        assert!(!root.ui.processing);
    });
    rendered(cx, "process-project");
    assert_eq!(
        field_value(cx, &prompt),
        "keep the question while importing"
    );
    assert!(assistant.run_request_ids.lock().unwrap().is_empty());
}

#[gpui::test]
fn imported_audio_project_is_keyboard_reachable_at_minimum_size(cx: &mut TestAppContext) {
    let (view, cx, _) = first_use_assistant(cx, true);
    complete_first_audio_import(&view, cx);
    tab_to_first_use_action(cx, "assistant-first-project");
    press_enter(cx);
    assert_eq!(view.read_with(cx, |root, _| root.ui.route), Route::Meetings);
    rendered(cx, "process-project");
}

#[gpui::test]
fn imported_audio_guidance_yields_to_processing_and_existing_results(cx: &mut TestAppContext) {
    let (view, cx, _) = first_use_assistant(cx, true);
    complete_first_audio_import(&view, cx);
    rendered(cx, "assistant-first-project");
    view.update(cx, |root, cx| {
        root.ui
            .begin_processing(root.ui.selected_project.clone().unwrap());
        cx.notify();
    });
    assert!(!shown(cx, "assistant-first-project".into()));
    view.update(cx, |root, cx| {
        root.ui.finish_processing();
        let document = root.projects[0].clone();
        root.assistant.authorize_document(Some(&document));
        let settings = root.saved_assistant().clone();
        let (request, _) = root
            .assistant
            .begin("previous question".into(), &settings)
            .unwrap();
        root.assistant.complete(
            &request.request_id,
            AssistantRunResult {
                answer: Some("existing answer".into()),
                pending_reviews: vec![pending_review("existing-review", u64::MAX)],
                ..empty_result()
            },
        );
        cx.notify();
    });
    cx.run_until_parked();
    rendered(cx, "assistant-result");
    rendered(cx, "assistant-pending-existing-review");
    assert!(!shown(cx, "assistant-first-project".into()));
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
    let error = rendered(cx, "assistant-error");
    assert!(
        within(rendered(cx, "assistant-retry"), error),
        "the retry sits on the error it repeats the request after"
    );
    assert!(within(rendered(cx, "assistant-error-details"), error));
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
    assert!(
        !shown(cx, "assistant-error".into()),
        "a run that succeeds shows no error"
    );
}

#[gpui::test]
fn a_rejected_api_key_explains_itself_and_opens_assistant_settings(cx: &mut TestAppContext) {
    let (view, cx, assistant) = open_assistant(cx, FakeAssistantUiService::default());
    assistant
        .run_errors
        .lock()
        .unwrap()
        .push_back(warning("PROVIDER_AUTHENTICATION_FAILED", false));
    start_run(cx, &view);
    wait_for(cx, &view, |root| {
        matches!(root.assistant.task, AssistantTaskState::Failed(_))
    });
    let results = rendered(cx, "assistant-results");
    let error = rendered(cx, "assistant-error");
    assert!(within(error, results), "the error is part of the results");
    assert!(
        rendered(cx, "assistant-request").bottom() <= error.top(),
        "the error follows the request it answers"
    );
    assert!(within(rendered(cx, "assistant-open-settings"), error));
    assert!(!shown(cx, "assistant-retry".into()));

    click_selector(cx, "assistant-open-settings");
    let locale = view.read_with(cx, |root, _| root.ui.locale);
    assert_assistant_form_visible(cx, locale);
    let assistant_section = crate::pages::settings::SECTIONS
        .iter()
        .position(|key| *key == "assistant_settings");
    view.read_with(cx, |root, _| {
        assert_eq!(root.ui.route, Route::Settings);
        assert_eq!(Some(root.ui.settings_section), assistant_section);
    });
    assert_eq!(
        assistant_section,
        Some(crate::pages::settings::ASSISTANT_SECTION)
    );
    assert!(
        shown(cx, "assistant-api-key-field".into()),
        "the key field is in reach"
    );
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

fn decision_keeps_the_question_focus(
    cx: &mut TestAppContext,
    accept: bool,
    move_during_decision: bool,
) {
    let _alone = hold_a_worker_alone();
    let gate = Gate::default();
    let (view, cx, assistant) = open_assistant(
        cx,
        FakeAssistantUiService {
            decision_gate: Some(gate.clone()),
            ..Default::default()
        },
    );
    cx.simulate_resize(crate::pages::chrome::MIN_WINDOW_SIZE);
    *assistant.run_result.lock().unwrap() = Some(AssistantRunResult {
        pending_reviews: vec![
            pending_review("review-1", u64::MAX),
            pending_review("review-2", u64::MAX),
        ],
        ..empty_result()
    });
    *assistant.confirm_result.lock().unwrap() = Some(confirmed("accepted-1"));
    start_run(cx, &view);
    wait_for(cx, &view, |root| root.assistant.pending.is_some());
    let prompt = view.read_with(cx, |root, _| root.assistant_inputs.prompt.clone());
    retype(cx, &prompt, "keep this next question");
    if move_during_decision {
        let focus = view.read_with(cx, |root, _| root.review_focus.clone());
        cx.update(|window, cx| focus.focus(window, cx));
    }
    // The short viewport can put the decisions below the initial view.
    let action = if accept {
        "assistant-confirm"
    } else {
        "assistant-reject"
    };
    let scroll = view.read_with(cx, |root, _| root.scroll.assistant_results.clone());
    let delta = scroll.bounds().bottom() - rendered(cx, action).bottom() - gpui::px(8.);
    if delta < gpui::px(0.) {
        scroll.set_offset(gpui::point(scroll.offset().x, scroll.offset().y + delta));
        cx.update(|window, cx| window.draw(cx).clear());
    }
    assert!(within(rendered(cx, action), scroll.bounds()));
    gate.arm();
    click_selector(cx, action);
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    if move_during_decision {
        retype(cx, &prompt, "keep this next question");
    }
    assert!(cx.update(
        |window, cx| gpui::Focusable::focus_handle(prompt.read(cx), cx)
            .contains_focused(window, cx)
    ));
    gate.release();
    wait_for(cx, &view, |root| {
        matches!(
            &root.assistant.pending,
            Some(PendingReviewState::Awaiting(review)) if review.confirmation_id == "review-2"
        )
    });
    assert!(
        cx.update(
            |window, cx| gpui::Focusable::focus_handle(prompt.read(cx), cx)
                .contains_focused(window, cx)
        ),
        "the decision must not move focus from the question"
    );
    assert_eq!(field_value(cx, &prompt), "keep this next question");
    assert_eq!(
        view.read_with(cx, |root, _| root.ui.route),
        Route::Assistant
    );
    rendered(cx, "assistant-pending-review-2");
}

#[gpui::test]
fn accepting_a_review_keeps_the_question_focus_after_the_user_moves_on(cx: &mut TestAppContext) {
    decision_keeps_the_question_focus(cx, true, true);
}

#[gpui::test]
fn rejecting_a_review_keeps_the_question_focus_after_the_user_moves_on(cx: &mut TestAppContext) {
    decision_keeps_the_question_focus(cx, false, true);
}

#[gpui::test]
fn a_pointer_review_decision_keeps_the_question_focus_without_further_input(
    cx: &mut TestAppContext,
) {
    decision_keeps_the_question_focus(cx, true, false);
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
        // A review without a diff names the documents it changes.
        "assistant-pending-affects",
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
    // Continue from the review with the keyboard; pointer presses retain
    // whichever field was already focused instead.
    let focus = view.read_with(cx, |root, _| root.review_focus.clone());
    cx.update(|window, cx| focus.focus(window, cx));
    cx.simulate_keystrokes("tab tab");
    assert!(within(
        rendered(cx, "focus-ring"),
        rendered(cx, "assistant-confirm")
    ));
    gate.arm();
    press_enter(cx);
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    assert!(
        cx.update(|window, cx| focus.contains_focused(window, cx)),
        "the in-flight keyboard decision stays in the review region"
    );
    assert!(shown(cx, "assistant-review-accepting".into()));
    let task = view.read_with(cx, |root, _| root.assistant.task.clone());
    click_selector(cx, "assistant-confirm");
    click_selector(cx, "assistant-reject");
    click_selector(cx, "assistant-submit");
    assert_no_new_run(cx, &view, &assistant, 1, &task);
    assert_eq!(assistant.grants_for("confirm").len(), 1);
    assert!(assistant.grants_for("reject").is_empty());
    assert!(
        cx.update(|window, cx| focus.contains_focused(window, cx)),
        "disabled presses leave the review focus unchanged"
    );
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
fn a_review_shows_its_diff_and_lines_its_decisions_up_at_the_end(cx: &mut TestAppContext) {
    let (view, cx, assistant) = open_assistant(cx, FakeAssistantUiService::default());
    let diff =
        |field: &str, before: Option<&str>, after: &str| crate::assistant_ui::AssistantFieldDiff {
            document_id: "first".into(),
            field: field.into(),
            before: before.map(Into::into),
            after: Some(after.into()),
        };
    let mut review = pending_review("review-1", u64::MAX);
    review.field_diffs = vec![
        diff("title", Some("Old title"), "New title"),
        diff("summary", None, "A summary"),
    ];
    *assistant.run_result.lock().unwrap() = Some(AssistantRunResult {
        pending_reviews: vec![review, pending_review("review-2", u64::MAX)],
        ..empty_result()
    });
    start_run(cx, &view);
    wait_for(cx, &view, |root| root.assistant.pending.is_some());

    let card = rendered(cx, "assistant-pending-review-1");
    let title = rendered(cx, "assistant-pending-title");
    for selector in [
        "assistant-pending-position-1-of-2",
        "assistant-pending-expires",
    ] {
        let meta = rendered(cx, selector);
        assert!(
            meta.left() >= title.right()
                && meta.top() < title.bottom()
                && title.top() < meta.bottom(),
            "`{selector}` trails the title on its line"
        );
        assert!(within(meta, card));
    }
    let (before, after) = (
        rendered(cx, "assistant-diff-0-before"),
        rendered(cx, "assistant-diff-0-after"),
    );
    assert!(before.bottom() <= after.top(), "the old value reads first");
    assert!(
        !shown(cx, "assistant-diff-1-before".into()),
        "nothing was there before"
    );
    assert!(shown(cx, "assistant-diff-1-after".into()));
    assert!(
        !shown(cx, "assistant-pending-affects".into()),
        "the diff already names the document"
    );

    let (reject, confirm) = (
        rendered(cx, "assistant-reject"),
        rendered(cx, "assistant-confirm"),
    );
    assert!(
        reject.right() <= confirm.left() && reject.top() == confirm.top(),
        "reject sits left of accept on one line"
    );
    assert!(
        confirm.right() > card.right() - gpui::px(24.) && after.bottom() <= confirm.top(),
        "the decisions close the card at its right edge"
    );
}

#[gpui::test]
fn long_review_text_can_be_read_in_full_before_a_write(cx: &mut TestAppContext) {
    let (view, cx, assistant) = open_assistant(cx, FakeAssistantUiService::default());
    let mut review = pending_review("full-diff-review", u64::MAX);
    review.field_diffs = vec![crate::assistant_ui::AssistantFieldDiff {
        document_id: "first".into(),
        field: "markdown".into(),
        before: Some(
            "The original meeting notes describe the decision and its owner in detail. "
                .repeat(120),
        ),
        after: Some(
            "The replacement meeting notes describe the revised decision and a new owner. "
                .repeat(144),
        ),
    }];
    *assistant.run_result.lock().unwrap() = Some(AssistantRunResult {
        pending_reviews: vec![review],
        ..empty_result()
    });
    *assistant.confirm_result.lock().unwrap() = Some(confirmed("full-diff-write"));
    start_run(cx, &view);
    wait_for(cx, &view, |root| root.assistant.pending.is_some());
    for size in [
        gpui::size(gpui::px(1180.), gpui::px(760.)),
        crate::pages::chrome::MIN_WINDOW_SIZE,
    ] {
        cx.simulate_resize(size);
        let scroll = view.read_with(cx, |root, _| root.scroll.assistant_results.clone());
        scroll.set_offset(gpui::Point::default());
        cx.update(|window, cx| window.draw(cx).clear());
        cx.run_until_parked();
        let before = rendered(cx, "assistant-diff-0-before");
        let after = rendered(cx, "assistant-diff-0-after");
        assert!(
            before.size.height >= gpui::px(20.) * 20. && after.size.height >= gpui::px(20.) * 24.,
            "the complete old and new prose can be read before approving: {before:?}, {after:?}"
        );
        assert!(
            before.bottom() <= after.top(),
            "old and new values read in order"
        );
        let results = rendered(cx, "assistant-results");
        for value in [before, after] {
            assert!(
                value.left() >= results.left() && value.right() <= results.right(),
                "the full text wraps inside the results viewport: {value:?} in {results:?}"
            );
        }
        assert!(
            rendered(cx, "assistant-confirm").top() > results.bottom(),
            "approval follows the complete diff beyond the initial viewport"
        );
        scroll_results_to_end(cx, &view);
        let confirm = rendered(cx, "assistant-confirm");
        assert!(within(confirm, rendered(cx, "assistant-results")));
        assert!(
            confirm.bottom() <= rendered(cx, "assistant-input-bar").top(),
            "scrolling through the full review reveals the decision above the prompt"
        );
        assert!(within(
            rendered(cx, "assistant-input-bar"),
            rendered(cx, "assistant-page")
        ));
        assert!(
            assistant.grants_for("confirm").is_empty(),
            "reading never approves a write"
        );
    }
    click_selector(cx, "assistant-confirm");
    wait_for(cx, &view, |root| {
        root.assistant.task == AssistantTaskState::Completed
    });
    assert_eq!(assistant.grants_for("confirm").len(), 1);
    assert!(shown(cx, "assistant-receipt-full-diff-write".into()));
}

#[gpui::test]
fn tab_reveals_review_controls_after_long_results_above_the_prompt(cx: &mut TestAppContext) {
    let (view, cx, assistant) = open_assistant(cx, FakeAssistantUiService::default());
    let mut review = pending_review("keyboard-review", u64::MAX);
    review.field_diffs = vec![crate::assistant_ui::AssistantFieldDiff {
        document_id: "first".into(),
        field: "summary".into(),
        before: Some("The previous summary.".into()),
        after: Some("The revised summary names the decision and its owner.".into()),
    }];
    *assistant.run_result.lock().unwrap() = Some(AssistantRunResult {
        answer: Some(
            "The meeting notes describe the decision and its owner in detail. ".repeat(144),
        ),
        pending_reviews: vec![review],
        ..empty_result()
    });
    start_run(cx, &view);
    wait_for(cx, &view, |root| root.assistant.pending.is_some());
    for size in [
        gpui::size(gpui::px(1920.), gpui::px(1080.)),
        gpui::size(gpui::px(1180.), gpui::px(760.)),
        crate::pages::chrome::MIN_WINDOW_SIZE,
    ] {
        cx.simulate_resize(size);
        let scroll = view.read_with(cx, |root, _| root.scroll.assistant_results.clone());
        scroll.set_offset(gpui::Point::default());
        cx.update(|window, cx| window.draw(cx).clear());
        cx.run_until_parked();
        assert!(
            rendered(cx, "assistant-confirm").top() > rendered(cx, "assistant-results").bottom(),
            "the long review puts its decisions below the initial viewport: {:?} in {:?}",
            rendered(cx, "assistant-confirm"),
            rendered(cx, "assistant-results")
        );
        let focus = view.read_with(cx, |root, _| root.review_focus.clone());
        cx.update(|window, cx| focus.focus(window, cx));
        cx.simulate_keystrokes("tab tab");
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear());
        cx.run_until_parked();
        let confirm = rendered(cx, "assistant-confirm");
        let ring = rendered(cx, "focus-ring");
        assert!(
            within(ring, confirm),
            "Tab reaches the confirmation button: {ring:?} in {confirm:?}"
        );
        let bar = rendered(cx, "assistant-input-bar");
        assert!(
            confirm.bottom() <= bar.top(),
            "keyboard reveal leaves the decision clear of the floating prompt: {confirm:?} above {bar:?}"
        );
        assert!(within(confirm, rendered(cx, "assistant-results")));
    }
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

    assert!(
        rendered(cx, "assistant-request").size.height >= gpui::px(16.),
        "results that overflow keep the quoted request whole"
    );
    let bar = rendered(cx, "assistant-input-bar");
    let results = rendered(cx, "assistant-results");
    let last = rendered(cx, "assistant-pending-overflow");
    assert!(
        last.bottom() <= bar.top(),
        "the last card ends above the bar: {last:?} and {bar:?}"
    );
    assert!(
        results.top() < bar.top() && results.bottom() <= bar.top(),
        "the results viewport ends above the bar: {results:?} and {bar:?}"
    );
}

#[gpui::test]
fn the_input_bar_shows_a_status_only_while_a_run_goes_on_or_has_an_outcome(
    cx: &mut TestAppContext,
) {
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
        !shown(cx, "assistant-status".into()),
        "idle shows no status"
    );
    assert!(!shown(cx, "assistant-task-idle".into()));
    let idle_bar = rendered(cx, "assistant-input-bar");

    *assistant.run_result.lock().unwrap() = Some(empty_result());
    gate.arm();
    start_run(cx, &view);
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    let bar = rendered(cx, "assistant-input-bar");
    let status = rendered(cx, "assistant-status");
    assert!(within(status, bar), "the status sits in the bar");
    assert!(within(rendered(cx, "assistant-task-running"), status));
    assert!(
        rendered(cx, "assistant-submit").bottom() <= status.top(),
        "the status sits under the prompt row"
    );
    assert_eq!(
        bar.bottom(),
        idle_bar.bottom(),
        "the bar grows upward from its place"
    );

    gate.release();
    wait_for(cx, &view, |root| {
        root.assistant.task == AssistantTaskState::Completed
    });
    assert!(shown(cx, "assistant-task-completed".into()));
}

#[gpui::test]
fn a_receipt_shows_left_out_segments_only_when_there_are_some(cx: &mut TestAppContext) {
    let (view, cx, assistant) = open_assistant(cx, FakeAssistantUiService::default());
    let mut gaps = write("with-gaps");
    gaps.uncovered_segment_ids = vec!["seg-9".into()];
    *assistant.run_result.lock().unwrap() = Some(AssistantRunResult {
        mutations: vec![write("plain"), gaps],
        ..empty_result()
    });
    start_run(cx, &view);
    wait_for(cx, &view, |root| root.assistant.result.is_some());
    assert!(!shown(cx, "assistant-receipt-gaps-plain".into()));
    let receipt = rendered(cx, "assistant-receipt-with-gaps");
    assert!(within(
        rendered(cx, "assistant-receipt-gaps-with-gaps"),
        receipt
    ));
    assert!(within(rendered(cx, "assistant-undo-with-gaps"), receipt));
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

/// Real settings files whose successful CAS can wait before its result
/// reaches the form. Readers use another service, so the saving service's
/// held write lock never blocks an assertion on the UI thread.
struct AssistantSaveFiles {
    path: std::path::PathBuf,
    settings: settings::SettingsService,
    gate: Gate,
    writes: Arc<std::sync::atomic::AtomicUsize>,
}

struct HeldAssistantSave {
    inner: settings::JsonSettingsRepository,
    gate: Gate,
    writes: Arc<std::sync::atomic::AtomicUsize>,
}

impl settings::SettingsRepository for HeldAssistantSave {
    fn load(&self) -> Result<settings::AppSettings, settings::SettingsError> {
        settings::SettingsRepository::load(&self.inner)
    }

    fn save(&self, value: &settings::AppSettings) -> Result<(), settings::SettingsError> {
        settings::SettingsRepository::save(&self.inner, value)
    }

    fn save_if_equal(
        &self,
        expected: &settings::AppSettings,
        value: &settings::AppSettings,
    ) -> Result<(), settings::SettingsError> {
        let result = settings::SettingsRepository::save_if_equal(&self.inner, expected, value);
        if result.is_ok() {
            self.writes
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.gate.pass();
        }
        result
    }
}

impl AssistantSaveFiles {
    fn new() -> Self {
        let path = crate::tests::temp_directory().join("settings.json");
        let inner = settings::JsonSettingsRepository::open(&path);
        let mut saved = stored_key_settings().snapshot().unwrap();
        let mut second = saved.assistant.connections[0].clone();
        second.id = "second".into();
        second.name = "Second".into();
        second.api_key = Some("second-stored-key".into());
        saved.assistant.connections.push(second);
        settings::SettingsRepository::save(&inner, &saved).unwrap();
        let gate = Gate::default();
        let writes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let settings = settings::SettingsService::open(Arc::new(HeldAssistantSave {
            inner,
            gate: gate.clone(),
            writes: writes.clone(),
        }))
        .unwrap();
        Self {
            path,
            settings,
            gate,
            writes,
        }
    }

    fn read(&self) -> settings::AssistantSettings {
        settings::SettingsService::open(Arc::new(settings::JsonSettingsRepository::open(
            &self.path,
        )))
        .unwrap()
        .snapshot()
        .unwrap()
        .assistant
    }

    fn write_count(&self) -> usize {
        self.writes.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Drop for AssistantSaveFiles {
    fn drop(&mut self) {
        std::fs::remove_dir_all(self.path.parent().unwrap()).ok();
    }
}

fn open_saving_form<'a>(
    cx: &'a mut TestAppContext,
    files: &AssistantSaveFiles,
) -> (View, &'a mut VisualTestContext) {
    let assistant = Arc::new(FakeAssistantUiService {
        settings: Some(files.settings.clone()),
        ..Default::default()
    });
    let (view, cx) = open_desktop_with(cx, assistant, files.settings.clone());
    open_assistant_settings(cx, &view);
    (view, cx)
}

#[gpui::test]
fn a_saved_assistant_form_keeps_new_typing_and_credentials(cx: &mut TestAppContext) {
    let _alone = hold_a_worker_alone();
    let files = AssistantSaveFiles::new();
    let (view, cx) = open_saving_form(cx, &files);
    let (name, key, rounds) = view.read_with(cx, |root, _| {
        (
            root.assistant_inputs.connection_name.clone(),
            root.assistant_inputs.api_key.clone(),
            root.assistant_inputs.max_tool_rounds.clone(),
        )
    });
    retype(cx, &name, "Saved name");
    files.gate.arm();
    click_assistant_setting(cx, &view, "assistant-save-settings");
    wait_for(cx, &view, |_| files.gate.counts().1 == 1);
    assert_eq!(files.read().connections[0].name, "Saved name");
    click_assistant_setting(cx, &view, "settings-write-mode-confirm-each");
    retype(cx, &key, "later-key-draft");
    retype(cx, &rounds, "1e"); // A partial numeric edit is a draft too.
    retype(cx, &name, "Keep this draft");
    cx.simulate_keystrokes("left");
    files.gate.release();
    wait_for(cx, &view, |root| {
        root.saved_assistant().connections[0].name == "Saved name"
    });
    assert_eq!(
        field_value(cx, &name),
        "Keep this draft",
        "a save result must not replace later typing"
    );
    assert!(
        field_value(cx, &key) == "later-key-draft",
        "a save result must keep a newly typed key"
    );
    assert_eq!(field_value(cx, &rounds), "1e");
    view.read_with(cx, |root, _| {
        assert_eq!(
            root.settings_draft.assistant.write_mode,
            contracts::AgentWriteMode::ConfirmEach
        );
        assert_eq!(
            root.saved_write_mode(),
            contracts::AgentWriteMode::Automatic
        );
        assert_eq!(
            root.ui.notice,
            crate::i18n::text(root.ui.locale, "assistant_settings_saved_draft_kept")
        );
    });
    assert!(cx.update(
        |window, cx| gpui::Focusable::focus_handle(name.read(cx), cx).contains_focused(window, cx)
    ));
    cx.simulate_input("!");
    assert_eq!(field_value(cx, &name), "Keep this draf!t");
    assert!(files.read().connections[0].api_key.as_deref() == Some(STORED_KEY));
    retype(cx, &rounds, "12");
    click_assistant_setting(cx, &view, "assistant-save-settings");
    wait_for(cx, &view, |root| {
        root.saved_assistant().connections[0].name == "Keep this draf!t"
    });
    let saved = files.read();
    assert_eq!(saved.connections[0].name, "Keep this draf!t");
    assert_eq!(saved.max_tool_rounds, 12);
    assert_eq!(saved.write_mode, contracts::AgentWriteMode::ConfirmEach);
    view.read_with(cx, |root, _| {
        assert_eq!(
            root.ui.notice,
            crate::i18n::text(root.ui.locale, "assistant_settings_saved")
        )
    });
    assert!(saved.connections[0].api_key.as_deref() == Some("later-key-draft"));
    assert!(
        field_value(cx, &key).is_empty(),
        "only the explicitly saved key is cleared"
    );
}

#[gpui::test]
fn a_saved_assistant_form_keeps_the_newly_selected_connection(cx: &mut TestAppContext) {
    let _alone = hold_a_worker_alone();
    let files = AssistantSaveFiles::new();
    let (view, cx) = open_saving_form(cx, &files);
    let (id, name, key) = view.read_with(cx, |root, _| {
        (
            root.assistant_inputs.connection_id.clone(),
            root.assistant_inputs.connection_name.clone(),
            root.assistant_inputs.api_key.clone(),
        )
    });
    retype(cx, &name, "Saved first connection");
    files.gate.arm();
    click_assistant_setting(cx, &view, "assistant-save-settings");
    wait_for(cx, &view, |_| files.gate.counts().1 == 1);
    click_assistant_setting(cx, &view, "assistant-profile-second");
    retype(cx, &name, "Second draft");
    files.gate.release();
    wait_for(cx, &view, |root| {
        root.saved_assistant().connections[0].name == "Saved first connection"
    });
    assert_eq!(
        field_value(cx, &id),
        "second",
        "an earlier save must not switch the form back"
    );
    assert_eq!(field_value(cx, &name), "Second draft");
    assert!(field_value(cx, &key).is_empty());
    view.read_with(cx, |root, cx| {
        assert_eq!(
            root.settings_draft
                .assistant
                .active_connection_id
                .as_deref(),
            Some("second")
        );
        assert!(root.form_keeps_stored_credential(cx));
        assert_eq!(
            root.ui.notice,
            crate::i18n::text(root.ui.locale, "assistant_settings_saved_draft_kept")
        );
    });
    assert!(cx.update(
        |window, cx| gpui::Focusable::focus_handle(name.read(cx), cx).contains_focused(window, cx)
    ));
    let saved_first = files.read();
    assert_eq!(saved_first.active_connection_id.as_deref(), Some("local"));
    assert_eq!(saved_first.connections[1].name, "Second");
    click_assistant_setting(cx, &view, "assistant-save-settings");
    wait_for(cx, &view, |root| {
        root.saved_assistant().active_connection_id.as_deref() == Some("second")
    });
    let saved_second = files.read();
    assert_eq!(saved_second.connections[0].name, "Saved first connection");
    assert_eq!(saved_second.connections[1].name, "Second draft");
    assert!(saved_second.connections[1].api_key.as_deref() == Some("second-stored-key"));
}

#[gpui::test]
fn a_pending_assistant_save_requires_a_later_explicit_save(cx: &mut TestAppContext) {
    let _alone = hold_a_worker_alone();
    let files = AssistantSaveFiles::new();
    let (view, cx) = open_saving_form(cx, &files);
    let name = view.read_with(cx, |root, _| root.assistant_inputs.connection_name.clone());
    retype(cx, &name, "First saved edit");
    files.gate.arm();
    click_assistant_setting(cx, &view, "assistant-save-settings");
    wait_for(cx, &view, |_| files.gate.counts().1 == 1);
    assert_eq!(files.read().connections[0].name, "First saved edit");
    retype(cx, &name, "Local");
    click_assistant_setting(cx, &view, "assistant-save-settings");
    view.update_in(cx, |root, window, cx| {
        root.save_assistant_settings(&gpui::ClickEvent::default(), window, cx);
        // An ordinary settings write provides an actual queue barrier after
        // both the button and a stale callback tried to save again.
        root.set_locale(crate::i18n::Locale::EnUs, cx);
    });
    files.gate.release();
    wait_for(cx, &view, |root| {
        root.ui.notice == crate::i18n::text(crate::i18n::Locale::EnUs, "language_changed")
    });
    assert_eq!(
        files.write_count(),
        2,
        "only A and the locale write may commit"
    );
    assert_eq!(files.read().connections[0].name, "First saved edit");
    assert_eq!(field_value(cx, &name), "Local");
    click_assistant_setting(cx, &view, "assistant-save-settings");
    wait_for(cx, &view, |root| {
        root.saved_assistant().connections[0].name == "Local"
    });
    assert_eq!(files.write_count(), 3);
    assert_eq!(files.read().connections[0].name, "Local");
}

#[gpui::test]
fn a_later_assistant_save_preserves_unedited_external_settings(cx: &mut TestAppContext) {
    let _alone = hold_a_worker_alone();
    let files = AssistantSaveFiles::new();
    let (view, cx) = open_saving_form(cx, &files);
    let external = settings::SettingsService::open(Arc::new(
        settings::JsonSettingsRepository::open(&files.path),
    ))
    .unwrap();
    external
        .patch_if_equal(
            &external.revision().unwrap(),
            settings::SettingsPatch {
                write_mode: Some(contracts::AgentWriteMode::ConfirmEach),
                connection: Some(settings::AssistantConnectionPatch {
                    id: "local".into(),
                    max_output_tokens: Some(3072),
                    ..Default::default()
                }),
                ..Default::default()
            },
            settings::SettingsGrant::full(),
        )
        .unwrap();
    let name = view.read_with(cx, |root, _| root.assistant_inputs.connection_name.clone());
    retype(cx, &name, "First retry");
    click_assistant_setting(cx, &view, "assistant-save-settings");
    wait_for(cx, &view, |root| root.settings_error.is_some());
    assert_eq!(files.write_count(), 0);
    files.gate.arm();
    click_assistant_setting(cx, &view, "assistant-save-settings");
    wait_for(cx, &view, |_| files.gate.counts().1 == 1);
    assert_eq!(
        files.read().write_mode,
        contracts::AgentWriteMode::ConfirmEach
    );
    assert_eq!(files.read().connections[0].max_output_tokens, 3072);
    retype(cx, &name, "Second explicit edit");
    click_assistant_setting(cx, &view, "assistant-save-settings");
    files.gate.release();
    wait_for(cx, &view, |root| {
        root.saved_assistant().connections[0].name == "First retry"
    });
    assert_eq!(field_value(cx, &name), "Second explicit edit");
    click_assistant_setting(cx, &view, "assistant-save-settings");
    wait_for(cx, &view, |root| {
        root.saved_assistant().connections[0].name == "Second explicit edit"
    });
    assert_eq!(files.write_count(), 2);
    let saved = files.read();
    assert_eq!(
        saved.write_mode,
        contracts::AgentWriteMode::ConfirmEach,
        "a later name edit must not restore the old external write mode"
    );
    assert_eq!(saved.connections[0].max_output_tokens, 3072);
    assert_eq!(saved.connections[0].name, "Second explicit edit");
    wait_for(cx, &view, |root| {
        root.saved_assistant().connections[0].name == "Second explicit edit"
    });
}

#[gpui::test]
fn a_later_assistant_save_keeps_the_acknowledged_credential(cx: &mut TestAppContext) {
    let _alone = hold_a_worker_alone();
    let files = AssistantSaveFiles::new();
    let key_file = files.path.parent().unwrap().join("credentials/key.txt");
    std::fs::create_dir_all(key_file.parent().unwrap()).unwrap();
    std::fs::write(&key_file, "fixture-file-key").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_file, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    files
        .settings
        .patch_if_equal(
            &files.settings.revision().unwrap(),
            settings::SettingsPatch {
                connection: Some(settings::AssistantConnectionPatch {
                    id: "local".into(),
                    credential: Some(settings::CredentialUpdate::File(
                        "credentials/key.txt".into(),
                    )),
                    ..Default::default()
                }),
                ..Default::default()
            },
            settings::SettingsGrant::full(),
        )
        .unwrap();
    let before = files.write_count();
    let (view, cx) = open_saving_form(cx, &files);
    let (name, key, file) = view.read_with(cx, |root, _| {
        (
            root.assistant_inputs.connection_name.clone(),
            root.assistant_inputs.api_key.clone(),
            root.assistant_inputs.api_key_file.clone(),
        )
    });
    retype(cx, &key, "first-rotated-key");
    retype(cx, &name, "First key save");
    files.gate.arm();
    click_assistant_setting(cx, &view, "assistant-save-settings");
    wait_for(cx, &view, |_| files.gate.counts().1 == 1);
    assert!(files.read().connections[0].api_key_file.is_none());
    focus_field(cx, &key);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_keystrokes("backspace");
    assert!(field_value(cx, &key).is_empty());
    retype(cx, &name, "Keep the saved credential");
    click_assistant_setting(cx, &view, "assistant-save-settings");
    files.gate.release();
    wait_for(cx, &view, |root| {
        root.saved_assistant().connections[0].name == "First key save"
    });
    assert!(field_value(cx, &key).is_empty());
    assert!(field_value(cx, &file).is_empty());
    assert_eq!(field_value(cx, &name), "Keep the saved credential");
    click_assistant_setting(cx, &view, "assistant-save-settings");
    wait_for(cx, &view, |root| {
        root.saved_assistant().connections[0].name == "Keep the saved credential"
    });
    assert_eq!(files.write_count(), before + 2);
    let saved = files.read();
    assert!(
        saved.connections[0].api_key.as_deref() == Some("first-rotated-key"),
        "an untouched old file field must not replace the just-saved key"
    );
    assert!(saved.connections[0].api_key_file.is_none());
    wait_for(cx, &view, |root| {
        root.saved_assistant().connections[0].name == "Keep the saved credential"
    });
    assert!(field_value(cx, &key).is_empty());
    assert!(field_value(cx, &file).is_empty());
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
fn editing_a_connection_clears_its_old_probe_without_losing_the_draft(cx: &mut TestAppContext) {
    let (view, cx, _) = open_assistant(cx, FakeAssistantUiService::default());
    open_assistant_settings(cx, &view);
    click_assistant_setting(cx, &view, "assistant-model-probe");
    wait_for(cx, &view, |root| root.assistant.diagnostic.is_some());
    let model = view.read_with(cx, |root, _| root.assistant_inputs.model.clone());
    retype(cx, &model, "changed-model");
    assert!(
        !shown(cx, "assistant-probe-result".into()),
        "the old model result no longer describes this draft"
    );
    assert_eq!(field_value(cx, &model), "changed-model");
    assert!(cx.update(
        |window, cx| gpui::Focusable::focus_handle(model.read(cx), cx).contains_focused(window, cx)
    ));
    assert_eq!(
        view.read_with(cx, |root, _| root
            .saved_assistant()
            .active_connection()
            .unwrap()
            .model
            .clone()),
        "assistant-default"
    );
}

#[gpui::test]
fn switching_profiles_clears_probe_errors_and_cancels_the_previous_probe(cx: &mut TestAppContext) {
    let _alone = hold_a_worker_alone();
    let settings = stored_key_settings();
    let mut value = settings.snapshot().unwrap();
    let mut second = value.assistant.connections[0].clone();
    second.id = "second".into();
    second.name = "Second".into();
    second.model = "second-model".into();
    value.assistant.connections.push(second);
    settings.update(value).unwrap();
    let gate = Gate::default();
    let assistant = Arc::new(FakeAssistantUiService {
        settings: Some(settings.clone()),
        probe_gate: Some(gate.clone()),
        ..Default::default()
    });
    assistant
        .probe_results
        .lock()
        .unwrap()
        .push_back(Err(warning("PROVIDER_TIMEOUT", true)));
    let (view, cx) = open_desktop_with(cx, assistant.clone(), settings);
    open_assistant_settings(cx, &view);
    click_assistant_setting(cx, &view, "assistant-model-probe");
    wait_for(cx, &view, |root| root.assistant.diagnostic.is_some());
    assert!(shown(cx, "assistant-probe-retry".into()));
    click_assistant_setting(cx, &view, "assistant-profile-second");
    assert!(
        !shown(cx, "assistant-probe-error".into()),
        "the second profile does not inherit the first error"
    );
    assert!(!shown(cx, "assistant-probe-retry".into()));
    gate.arm();
    click_assistant_setting(cx, &view, "assistant-model-probe");
    wait_for(cx, &view, |_| gate.counts().0 == 1);
    click_assistant_setting(cx, &view, "assistant-profile-local");
    let cancellation = assistant.probe_cancellations.lock().unwrap()[1].clone();
    let cancelled = cancellation.is_cancelled();
    gate.release();
    assert!(
        cancelled,
        "switching cancels the old profile's actual service token"
    );
    assert_eq!(probing(&view, cx), None);
    assert!(!shown(cx, "assistant-probe-running".into()));
    assert!(
        !shown(cx, "assistant-probe-cancelled".into()),
        "a profile change is not a user cancellation outcome"
    );
    click_assistant_setting(cx, &view, "assistant-model-probe");
    wait_for(cx, &view, |root| root.assistant.diagnostic.is_some());
    view.read_with(cx, |root, _| {
        assert_eq!(
            root.assistant
                .diagnostic
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .requested_model,
            "assistant-default"
        )
    });
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
fn probing_an_edited_target_requires_the_same_explicit_key_as_saving(cx: &mut TestAppContext) {
    let settings = stored_key_settings();
    let before = settings.snapshot().unwrap();
    let assistant = Arc::new(FakeAssistantUiService {
        settings: Some(settings.clone()),
        ..Default::default()
    });
    let (view, cx) = open_desktop_with(cx, assistant.clone(), settings.clone());
    open_assistant_settings(cx, &view);
    let (base_url, key) = view.read_with(cx, |root, _| {
        (
            root.assistant_inputs.base_url.clone(),
            root.assistant_inputs.api_key.clone(),
        )
    });
    retype(cx, &base_url, "http://127.0.0.1:4011");
    click_assistant_setting(cx, &view, "assistant-model-probe");
    wait_for(cx, &view, |root| {
        root.assistant.diagnostic.is_some() || root.settings_error.is_some()
    });
    assert!(
        assistant.probe_calls.lock().unwrap().is_empty(),
        "a draft target cannot receive a stored key before it is explicitly entered"
    );
    view.read_with(cx, |root, _| {
        assert_eq!(
            root.settings_error.as_ref().unwrap().code,
            SETTINGS_CREDENTIAL_TARGET_CONFLICT
        )
    });
    assert!(shown(cx, "assistant-credential-target-conflict".into()));
    assert_eq!(
        settings.snapshot().unwrap(),
        before,
        "a refused probe never saves the form"
    );

    // Entering the same key again is an explicit decision, just as it is for Save.
    retype(cx, &key, STORED_KEY);
    click_assistant_setting(cx, &view, "assistant-model-probe");
    wait_for(cx, &view, |root| root.assistant.diagnostic.is_some());
    assert!(!shown(cx, "assistant-credential-target-conflict".into()));
    let profiles = assistant.probe_profiles.lock().unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].base_url, "http://127.0.0.1:4011");
    assert_eq!(profiles[0].api_key.as_deref(), Some(STORED_KEY));
    assert_eq!(
        settings.snapshot().unwrap(),
        before,
        "testing the explicitly entered draft still does not save it"
    );
}

#[gpui::test]
fn probing_a_budget_edit_keeps_the_existing_credential(cx: &mut TestAppContext) {
    let settings = stored_key_settings();
    let assistant = Arc::new(FakeAssistantUiService {
        settings: Some(settings.clone()),
        ..Default::default()
    });
    let (view, cx) = open_desktop_with(cx, assistant.clone(), settings);
    open_assistant_settings(cx, &view);
    let timeout = view.read_with(cx, |root, _| root.assistant_inputs.timeout_ms.clone());
    retype(cx, &timeout, "60000");
    click_assistant_setting(cx, &view, "assistant-model-probe");
    wait_for(cx, &view, |root| root.assistant.diagnostic.is_some());
    let profiles = assistant.probe_profiles.lock().unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].timeout_ms, 60000);
    assert_eq!(profiles[0].api_key.as_deref(), Some(STORED_KEY));
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
