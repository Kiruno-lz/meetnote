mod assistant;
mod controls;
mod documents;
mod settings;
mod shell;
mod workspace_host;

use std::time::Duration;

use rune::{App, AppSpec, Chrome, Cx, El, Tx, col, el, row};
use rune_kit::{ButtonVariant, Dialog, Toast, ToastRegion};
use ui_text::Locale;

use controls::action;
use workspace_host::speaker_library::{LibraryChange, LibraryPathChange};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Route {
    Assistant,
    #[default]
    Documents,
    Models,
    Settings,
}

impl Route {
    const ALL: [Self; 4] = [
        Self::Assistant,
        Self::Documents,
        Self::Models,
        Self::Settings,
    ];

    fn name(self, locale: Locale) -> &'static str {
        ui_text::text(
            locale,
            match self {
                Self::Assistant => "assistant",
                Self::Documents => "meetings",
                Self::Models => "models",
                Self::Settings => "settings",
            },
        )
    }

    fn key(self) -> &'static str {
        match self {
            Self::Assistant => "nav-assistant",
            Self::Documents => "nav-documents",
            Self::Models => "nav-models",
            Self::Settings => "nav-settings",
        }
    }
}

enum Confirm {
    Import,
    ApplyAssistant,
    RemoveSpeaker,
    MoveSpeakerLibrary(LibraryPathChange),
    DeleteLibraryProfile(String),
    MergeLibraryProfile {
        source: String,
        target: Option<String>,
    },
    MergeSpeaker {
        document: String,
        from: String,
        into: Option<String>,
    },
    ResetSettings,
}

impl Confirm {
    fn copy(&self, s: &MeetNote) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Import => (
                "Import a recording",
                "Add a sample audio document to the preview workspace.",
                "Import sample",
            ),
            Self::ApplyAssistant => (
                "Apply the proposed change?",
                "Append the action item to the saved notes of the document in scope.",
                "Apply change",
            ),
            Self::MergeSpeaker { .. } => (
                s.tr("merge_document_speaker"),
                s.tr("merge_document_speaker_description"),
                s.tr("merge"),
            ),
            Self::RemoveSpeaker => (
                "Remove this speaker?",
                "Remove this profile from the preview Speaker Library.",
                "Remove speaker",
            ),
            Self::DeleteLibraryProfile(_) => (
                s.tr("delete_speaker_profile"),
                s.tr("delete_speaker_profile_description"),
                s.tr("delete"),
            ),
            Self::MoveSpeakerLibrary(_) => (
                s.tr("speaker_library_migration_title"),
                s.tr("speaker_library_migration_description"),
                s.tr("migrate"),
            ),
            Self::MergeLibraryProfile { .. } => (
                s.tr("merge_speaker_profiles"),
                s.tr("merge_speaker_profiles_description"),
                s.tr("merge"),
            ),
            Self::ResetSettings => (
                "Reset preview settings?",
                "Restore the sample appearance, processing and Assistant preferences.",
                "Reset settings",
            ),
        }
    }
}

#[derive(Default)]
struct MeetNote {
    sample_mode: bool,
    reduce_transparency: bool,
    route: Route,
    shell: shell::Shell,
    documents: documents::Workspace,
    workspace_host: workspace_host::WorkspaceHost,
    assistant: assistant::Assistant,
    settings: settings::Preferences,
    models: settings::Models,
    select: Option<&'static str>,
    confirm: Option<Confirm>,
    toast: Option<(String, f64)>,
    now: f64,
}

impl MeetNote {
    fn sample() -> Self {
        Self {
            sample_mode: true,
            documents: documents::Workspace::sample(),
            ..Self::default()
        }
    }

    fn show_sample(&mut self) {
        if !self.sample_mode {
            *self = Self::sample();
        }
    }

    fn locale(&self) -> Locale {
        self.workspace_host.preferences.draft.as_ref().map_or(
            if self.sample_mode {
                Locale::EnUs
            } else {
                Locale::ZhCn
            },
            workspace_host::preferences::PreferenceDraft::saved_locale,
        )
    }

    fn tr(&self, key: &str) -> &'static str {
        ui_text::text(self.locale(), key)
    }

    fn notify(&mut self, message: impl Into<String>, tx: &mut Tx) {
        self.toast = Some((message.into(), self.now + 4.0));
        tx.wake_after(Duration::from_secs(4));
    }

    fn confirm(&mut self, tx: &mut Tx) {
        match self.confirm.take() {
            Some(Confirm::Import) => {
                let id = self.documents.import();
                self.select_document(&id);
                self.notify("Sample recording imported", tx);
            }
            Some(Confirm::ApplyAssistant) => self.apply_assistant(tx),
            Some(Confirm::MergeSpeaker {
                document,
                from,
                into: Some(into),
            }) => {
                self.confirm_speaker_merge(&document, from, into, tx);
            }
            Some(Confirm::MergeSpeaker { into: None, .. }) => {}
            Some(Confirm::RemoveSpeaker) => {
                self.settings.speaker = false;
                self.notify("Speaker removed", tx);
            }
            Some(Confirm::DeleteLibraryProfile(id)) => {
                self.change_speaker_profiles(LibraryChange::Delete(id), tx);
            }
            Some(Confirm::MoveSpeakerLibrary(paths)) => {
                self.switch_speaker_library(paths, Some(true), tx);
            }
            Some(Confirm::MergeLibraryProfile {
                source,
                target: Some(target),
            }) => self.change_speaker_profiles(LibraryChange::Merge { source, target }, tx),
            Some(Confirm::MergeLibraryProfile { target: None, .. }) => {}
            Some(Confirm::ResetSettings) => {
                self.settings = settings::Preferences::default();
                self.notify("Sample settings reset", tx);
            }
            None => {}
        }
    }

    fn poll(&mut self, now: f64, tx: &mut Tx) -> bool {
        self.now = now;
        let expired = self.toast.as_ref().is_some_and(|(_, until)| now >= *until);
        if expired {
            self.toast = None;
        }
        let workspace = self.poll_workspace(tx);
        let processed = self.documents.poll(now, tx);
        let downloaded = self.models.poll(now, tx);
        let answered = self.poll_assistant(now, tx);
        expired || workspace || processed || downloaded || answered
    }
}

fn view(s: &MeetNote, cx: &mut Cx<MeetNote>) -> El {
    let k = &controls::theme(s.settings.dark(cx.system_dark()));
    let mut overlays = Vec::new();
    let page = if !s.sample_mode && s.workspace_host.live.is_none() {
        let body = col().key("workspace-startup").gap(12.0).padding(24.0);
        if s.workspace_host.busy() {
            body.child(controls::title(k, s.tr("documents_root_switching"), 24.0))
        } else {
            body.child(controls::title(k, s.tr("documents_root"), 24.0))
                .child(
                    rune_kit::Button::new(s.tr("choose_directory"))
                        .on_press_tx(MeetNote::choose_workspace)
                        .build(k),
                )
        }
    } else {
        match s.route {
            Route::Documents => documents::page(s, k, &mut overlays),
            Route::Assistant => assistant::page(s, k, &mut overlays),
            Route::Models => settings::models_page(s, k),
            Route::Settings => settings::page(s, k, &mut overlays),
        }
    };
    if let Some((message, _)) = &s.toast {
        overlays.push(
            ToastRegion::new()
                .toast(
                    Toast::new("preview-toast", message)
                        .on_close(|s: &mut MeetNote| s.toast = None)
                        .build(k),
                )
                .build(k),
        );
    }
    if let Some(confirm) = &s.confirm {
        let (heading, description, accept) = confirm.copy(s);
        let mut dialog = Dialog::new("confirm-dialog", heading)
            .open(true)
            .fit()
            .description(description)
            .on_close(|s: &mut MeetNote| s.confirm = None);
        if let Confirm::MergeSpeaker {
            document,
            from,
            into,
        } = confirm
        {
            dialog = dialog.child(rune::col().px(16.0).pb(12.0).child(s.speaker_merge_choices(
                document,
                from,
                into.as_deref(),
                k,
            )));
        }
        if let Confirm::MergeLibraryProfile { source, target } = confirm {
            dialog = dialog.child(rune::col().px(16.0).pb(12.0).child(
                settings::live_speaker_library::merge_choices(s, k, source, target.as_deref()),
            ));
        }
        dialog
            .child(
                row()
                    .gap(8.0)
                    .px(16.0)
                    .pb(16.0)
                    .child(el().grow(1.0))
                    .children(matches!(confirm, Confirm::MoveSpeakerLibrary(_)).then(|| {
                        rune_kit::Button::new(s.tr("use_new_path"))
                            .variant(ButtonVariant::Outline)
                            .on_press_tx(|s: &mut MeetNote, tx| {
                                if let Some(Confirm::MoveSpeakerLibrary(paths)) = s.confirm.take() {
                                    s.switch_speaker_library(paths, Some(false), tx);
                                }
                            })
                            .build(k)
                            .key("library-switch-without-copy")
                    }))
                    .child(action(
                        k,
                        "dialog-cancel",
                        s.tr("cancel"),
                        ButtonVariant::Neutral,
                        |s| s.confirm = None,
                    ))
                    .child(
                        rune_kit::Button::new(accept)
                            .disabled(matches!(
                                confirm,
                                Confirm::MergeSpeaker { into: None, .. }
                                    | Confirm::MergeLibraryProfile { target: None, .. }
                            ))
                            .variant(
                                if matches!(
                                    confirm,
                                    Confirm::RemoveSpeaker | Confirm::DeleteLibraryProfile(_)
                                ) {
                                    ButtonVariant::Danger
                                } else {
                                    ButtonVariant::Submit
                                },
                            )
                            .on_press_tx(MeetNote::confirm)
                            .build(k)
                            .key("dialog-confirm")
                            .default_button(),
                    ),
            )
            .build(k, &mut overlays);
    }
    rune_kit::root(shell::layout(s, k, cx, page), overlays)
}

fn main() {
    if meetnote::desktop::run_command_line() {
        return;
    }
    let spec = AppSpec::<MeetNote> {
        min_size: Some((900.0, 600.0)),
        scenes: &[
            ("documents", MeetNote::show_sample),
            ("details", |s| {
                s.show_sample();
                s.documents.detail = true;
            }),
            ("original", |s| {
                s.show_sample();
                s.documents.mode = documents::Mode::Original;
            }),
            ("edit", |s| {
                s.show_sample();
                s.documents.mode = documents::Mode::Edit;
            }),
            ("assistant", |s| {
                s.show_sample();
                s.navigate(Route::Assistant);
            }),
            ("assistant-review", |s| {
                s.show_sample();
                s.navigate(Route::Assistant);
                s.assistant.sample_review();
            }),
            ("models", |s| {
                s.show_sample();
                s.navigate(Route::Models);
            }),
            ("settings", |s| {
                s.show_sample();
                s.navigate(Route::Settings);
            }),
            ("settings-menu", |s| {
                s.show_sample();
                s.navigate(Route::Settings);
                s.select = Some("theme");
            }),
            ("import", |s| {
                s.show_sample();
                s.confirm = Some(Confirm::Import);
            }),
        ],
        setup: Some(MeetNote::setup),
        poll: Some(MeetNote::poll),
        on_key: Some(shell::shortcuts),
        ..AppSpec::new(
            "MeetNote",
            (1180.0, 760.0),
            || {
                if rune::extra_args().iter().any(|arg| arg == "--sample") {
                    MeetNote::sample()
                } else {
                    MeetNote::default()
                }
            },
            view,
        )
    };
    App::new(spec)
        .window_options(rune::WindowOptions {
            transparent: cfg!(target_os = "macos"),
            ..Default::default()
        })
        .chrome(Chrome {
            traffic_lights: Some((15.0, 15.0)),
            drag_regions: true,
            ..Chrome::DEFAULT
        })
        .run();
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::{Env, Key, KeyEvent, Mods, Runtime};
    use std::ops::{Deref, DerefMut};
    use std::path::PathBuf;

    struct TestRuntime {
        runtime: Runtime<MeetNote>,
        _settings_directory: tempfile::TempDir,
    }

    impl Deref for TestRuntime {
        type Target = Runtime<MeetNote>;

        fn deref(&self) -> &Self::Target {
            &self.runtime
        }
    }

    impl DerefMut for TestRuntime {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.runtime
        }
    }

    impl TestRuntime {
        fn configure_processing(&mut self, settings: Option<PathBuf>, models: Option<PathBuf>) {
            use ::settings::{JsonSettingsRepository, SettingsRepository};

            if let Some(path) = &settings {
                let repository = JsonSettingsRepository::open(path);
                let mut fixture = repository.load().unwrap();
                fixture.ui_locale = Locale::EnUs.code().into();
                if std::path::Path::new(&fixture.speaker_library_path).is_relative() {
                    fixture.speaker_library_path =
                        path.with_file_name("library.json").display().to_string();
                }
                repository.save(&fixture).unwrap();
            }
            self.state
                .workspace_host
                .configure_processing(settings, models);
        }
    }

    fn runtime() -> TestRuntime {
        let mut rt = Runtime::new(
            MeetNote::sample(),
            view,
            (900.0, 600.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.poll = Some(MeetNote::poll);
        rt.on_key = Some(shell::shortcuts);
        rt.frame(0.0);
        let settings_directory = tempfile::tempdir().unwrap();
        let settings = settings_directory.path().join("settings.json");
        let mut rt = TestRuntime {
            runtime: rt,
            _settings_directory: settings_directory,
        };
        rt.configure_processing(Some(settings), None);
        rt
    }

    fn advance(rt: &mut Runtime<MeetNote>, frames: usize) {
        for _ in 0..frames {
            rt.frame(1.0 / 60.0);
        }
    }

    fn click(rt: &mut Runtime<MeetNote>, key: &str) {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        advance(rt, 30);
    }

    fn key(rt: &mut Runtime<MeetNote>, key: Key) {
        rt.key(KeyEvent {
            key,
            mods: Mods::default(),
        });
        advance(rt, 30);
    }

    fn until(rt: &mut Runtime<MeetNote>, ready: impl FnMut(&Runtime<MeetNote>) -> bool) {
        until_within(rt, Duration::from_secs(5), ready);
    }

    fn until_within(
        rt: &mut Runtime<MeetNote>,
        timeout: Duration,
        mut ready: impl FnMut(&Runtime<MeetNote>) -> bool,
    ) {
        let deadline = std::time::Instant::now() + timeout;
        while !ready(rt) {
            assert!(
                std::time::Instant::now() < deadline,
                "workspace task timed out"
            );
            std::thread::sleep(Duration::from_millis(2));
            advance(rt, 1);
        }
    }

    fn open_test_workspace(rt: &mut Runtime<MeetNote>, path: &std::path::Path) {
        click(rt, "nav-settings");
        click(rt, "category-workspace");
        click(rt, "documents-folder");
        assert!(matches!(
            rt.file_dialog(),
            Some((rune::FileDialogKind::Folder, _))
        ));
        rt.resolve_file_dialog(Some(path.to_path_buf())).unwrap();
        until(rt, |rt| rt.state.workspace_host.live.is_some());
    }

    #[test]
    fn native_picker_import_and_save_survive_reopen_and_revision_conflict() {
        use application::{DocumentCommandService, DocumentService};
        use contracts::DocumentPatch;
        use persistence::FileDocumentRepository;
        use std::sync::Arc;

        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("documents");
        std::fs::create_dir(&root).unwrap();
        let audio = fixture.path().join("recording.wav");
        let mut wav = hound::WavWriter::create(
            &audio,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        wav.write_sample(0i16).unwrap();
        wav.finalize().unwrap();

        let mut rt = runtime();
        open_test_workspace(&mut rt, &root);
        assert!(visible_text(&rt, "No recordings"));
        click(&mut rt, "empty-import-audio");
        assert!(matches!(
            rt.file_dialog(),
            Some((rune::FileDialogKind::Open, _))
        ));
        rt.resolve_file_dialog(Some(audio.clone())).unwrap();
        until(&mut rt, |rt| rt.state.documents.has_selection());
        let id = domain::ProjectId::new(rt.state.documents.selection().0);
        click(&mut rt, "document-modes/segment3");
        click(&mut rt, "document-title-edit/wrapper/input");
        key(&mut rt, Key::End);
        key(&mut rt, Key::Text(" reviewed".into()));
        command(&mut rt, 's', false, false);
        until(&mut rt, |rt| {
            rt.state.documents.selection().1 == "recording reviewed"
        });
        drop(rt);

        let documents =
            DocumentService::new(Arc::new(FileDocumentRepository::open(&root).unwrap()));
        let saved = documents.read(&id).unwrap().unwrap();
        assert_eq!(saved.state.title(), "recording reviewed");
        assert_eq!(saved.state.audio_path(), audio.to_str().unwrap());
        let mut rt = runtime();
        open_test_workspace(&mut rt, &root);
        assert_eq!(rt.state.documents.selection().1, "recording reviewed");
        click(&mut rt, "document-modes/segment3");
        until(&mut rt, |rt| rt.find("document-title-edit").is_some());
        click(&mut rt, "document-title-edit/wrapper/input");
        key(&mut rt, Key::End);
        key(&mut rt, Key::Text(" local".into()));
        DocumentCommandService::new(documents.clone())
            .update_document(
                &id,
                saved.state.revision(),
                DocumentPatch {
                    title: Some("External title".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        command(&mut rt, 's', false, false);
        until(&mut rt, |rt| rt.find("reload-draft-notes").is_some());
        assert!(visible_text(&rt, "recording reviewed local"));
        assert_eq!(
            documents.read(&id).unwrap().unwrap().state.title(),
            "External title"
        );
    }

    #[test]
    fn file_workspace_relinks_moved_audio_through_the_native_picker() {
        use application::DocumentService;
        use persistence::FileDocumentRepository;
        use std::sync::Arc;

        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("documents");
        let original = fixture.path().join("recording.wav");
        let moved = fixture.path().join("renamed.wav");
        std::fs::write(&original, b"unprocessed recording").unwrap();
        let documents =
            DocumentService::new(Arc::new(FileDocumentRepository::open(&root).unwrap()));
        let state = documents.import_audio(original.to_str().unwrap()).unwrap();
        std::fs::rename(&original, &moved).unwrap();

        let mut rt = runtime();
        open_test_workspace(&mut rt, &root);
        until(&mut rt, |rt| rt.find("audio-source-missing").is_some());
        click(&mut rt, "relink-audio");
        assert!(matches!(
            rt.file_dialog(),
            Some((rune::FileDialogKind::Open, _))
        ));
        rt.resolve_file_dialog(Some(moved.clone())).unwrap();
        until(&mut rt, |rt| visible_text(rt, "Recording reconnected"));
        assert_eq!(
            documents
                .read(state.id())
                .unwrap()
                .unwrap()
                .state
                .audio_path(),
            moved.to_str().unwrap()
        );
        assert!(rt.find("audio-source-missing").is_none());

        let mut reopened = runtime();
        open_test_workspace(&mut reopened, &root);
        assert!(visible_text(&reopened, moved.to_str().unwrap()));
    }

    #[test]
    fn file_workspace_saves_an_assistant_connection_from_settings() {
        use ::settings::{JsonSettingsRepository, SettingsRepository};

        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("documents");
        let settings_path = fixture.path().join("settings.json");
        std::fs::create_dir(&root).unwrap();
        let mut rt = runtime();
        rt.configure_processing(Some(settings_path.clone()), None);
        open_test_workspace(&mut rt, &root);
        click(&mut rt, "nav-settings");
        click(&mut rt, "category-assistant");
        for (field, value) in [
            ("live-connection-name", "Local"),
            ("live-connection-endpoint", "http://127.0.0.1:4000"),
            ("live-connection-model", "local-model"),
            ("live-connection-context-window", "16384"),
            ("live-connection-timeout", "9500"),
            ("live-connection-output-limit", "1024"),
            ("live-connection-tool-rounds", "6"),
            ("live-connection-validation-retries", "2"),
        ] {
            rt.update(|_, tx| tx.scroll_into_view(field));
            advance(&mut rt, 30);
            click(&mut rt, &format!("{field}/wrapper/input"));
            command(&mut rt, 'a', false, false);
            key(&mut rt, Key::Text(value.into()));
        }
        rt.update(|_, tx| tx.scroll_into_view("save-live-connection"));
        advance(&mut rt, 30);
        click(&mut rt, "save-live-connection");
        until(&mut rt, |rt| {
            visible_text(rt, "Assistant connection updated")
        });
        let saved = JsonSettingsRepository::open(&settings_path).load().unwrap();
        let active = saved.assistant.active_connection().unwrap();
        assert_eq!(active.name, "Local");
        assert_eq!(active.base_url, "http://127.0.0.1:4000");
        assert_eq!(active.model, "local-model");
        assert_eq!(active.context_window_tokens, Some(16_384));
        assert_eq!(active.timeout_ms, 9_500);
        assert_eq!(active.max_output_tokens, 1_024);
        assert_eq!(saved.assistant.max_tool_rounds, 6);
        assert_eq!(saved.assistant.max_validation_retries, 2);
        assert!(
            rt.state
                .workspace_host
                .assistant
                .settings
                .active_connection()
                .is_some()
        );
    }

    fn assistant_settings(
        path: &std::path::Path,
        address: std::net::SocketAddr,
        write_mode: contracts::AgentWriteMode,
    ) {
        use ::settings::{
            AppSettings, AssistantAuthMode, AssistantConnectionProfile, JsonSettingsRepository,
            LlmProtocol, SettingsRepository,
        };
        let mut settings = AppSettings::default();
        settings.assistant.write_mode = write_mode;
        settings.assistant.active_connection_id = Some("loopback".into());
        settings
            .assistant
            .connections
            .push(AssistantConnectionProfile {
                id: "loopback".into(),
                name: "Loopback".into(),
                protocol: LlmProtocol::OpenAiChatCompletions,
                base_url: format!("http://{address}"),
                model: "loopback".into(),
                auth_mode: AssistantAuthMode::Bearer,
                api_key: None,
                api_key_file: None,
                context_window_tokens: Some(16000),
                timeout_ms: 3000,
                max_output_tokens: 256,
            });
        JsonSettingsRepository::open(path).save(&settings).unwrap();
    }

    fn loopback_assistant(
        reply: impl FnOnce(serde_json::Value) -> serde_json::Value + Send + 'static,
    ) -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(&mut stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("POST /v1/chat/completions "));
            let mut length = 0;
            loop {
                line.clear();
                reader.read_line(&mut line).unwrap();
                if line.trim().is_empty() {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let response = reply(serde_json::from_slice(&body).unwrap()).to_string();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
        });
        (address, server)
    }

    #[test]
    fn file_workspace_assistant_reads_the_authorized_document_over_http() {
        use application::DocumentService;
        use persistence::FileDocumentRepository;
        use serde_json::json;
        use std::sync::Arc;

        let (address, server) = loopback_assistant(|request| {
            let body = request.to_string();
            assert!(
                body.contains("What was decided?") && body.contains("Recorded decision"),
                "{body}"
            );
            json!({
                "model": "loopback",
                "choices": [{
                    "message": {"role": "assistant", "content": "The decision is to publish the notes."},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 10, "completion_tokens": 8}
            })
        });

        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("documents");
        let settings_path = fixture.path().join("settings.json");
        let documents =
            DocumentService::new(Arc::new(FileDocumentRepository::open(&root).unwrap()));
        let audio = fixture.path().join("recording.wav");
        std::fs::write(&audio, []).unwrap();
        let document = documents.import_audio(audio.to_str().unwrap()).unwrap();
        application::DocumentCommandService::new(documents)
            .update_document(
                document.id(),
                document.revision(),
                contracts::DocumentPatch {
                    markdown: Some("Recorded decision: publish the notes.".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assistant_settings(
            &settings_path,
            address,
            contracts::AgentWriteMode::Automatic,
        );
        let mut rt = runtime();
        rt.configure_processing(Some(settings_path), None);
        open_test_workspace(&mut rt, &root);
        click(&mut rt, "nav-assistant");
        click(&mut rt, "live-assistant-input/value/input");
        key(&mut rt, Key::Text("What was decided?".into()));
        click(&mut rt, "live-assistant-send");
        until(&mut rt, |rt| {
            visible_text(rt, "The decision is to publish the notes.")
        });
        assert!(visible_text(&rt, "10 / 8 / 18"));
        server.join().unwrap();
    }

    #[test]
    fn file_workspace_assistant_reviews_and_undoes_a_document_write() {
        use application::DocumentService;
        use contracts::{AgentScene, AgentWriteMode};
        use persistence::FileDocumentRepository;
        use serde_json::json;
        use std::sync::Arc;

        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("documents");
        let settings = fixture.path().join("settings.json");
        let audio = fixture.path().join("recording.wav");
        std::fs::write(&audio, []).unwrap();
        let documents =
            DocumentService::new(Arc::new(FileDocumentRepository::open(&root).unwrap()));
        let document = documents.import_audio(audio.to_str().unwrap()).unwrap();
        let id = document.id().clone();
        let initial_title = document.title().to_owned();
        let replacement = "Reviewed recording";
        let arguments = json!({
            "documentId": id.as_str(),
            "expectedRevision": document.revision(),
            "patch": {"title": replacement}
        });
        let (address, server) = loopback_assistant(move |request| {
            assert!(
                request["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|tool| { tool["function"]["name"] == "update_document" })
            );
            json!({
                "model": "loopback",
                "choices": [{
                    "finish_reason": "tool_calls",
                    "message": {"role": "assistant", "content": null, "tool_calls": [{
                        "id": "call-rename", "type": "function", "function": {
                            "name": "update_document", "arguments": arguments.to_string()
                        }
                    }]}
                }],
                "usage": {"prompt_tokens": 10, "completion_tokens": 8}
            })
        });
        assistant_settings(&settings, address, AgentWriteMode::ConfirmEach);
        let mut rt = runtime();
        rt.configure_processing(Some(settings), None);
        open_test_workspace(&mut rt, &root);
        click(&mut rt, "nav-assistant");
        rt.update(|s, _| {
            s.workspace_host
                .assistant
                .controller
                .select_scene(AgentScene::Organize)
        });
        advance(&mut rt, 30);
        click(&mut rt, "live-assistant-input/value/input");
        key(&mut rt, Key::Text("Update the recording title.".into()));
        click(&mut rt, "live-assistant-send");
        until(&mut rt, |rt| visible_text(rt, "Review proposed changes"));
        assert_eq!(
            documents.read(&id).unwrap().unwrap().state.title(),
            initial_title
        );
        assert!(visible_text(&rt, replacement));

        rt.update(|_, tx| tx.scroll_into_view("live-assistant-approve"));
        advance(&mut rt, 30);
        click(&mut rt, "live-assistant-approve");
        let undo_key = |rt: &Runtime<MeetNote>| {
            rt.elements().into_iter().find_map(|element| {
                element
                    .key
                    .filter(|key| key.starts_with("live-assistant-undo-"))
            })
        };
        until(&mut rt, |rt| {
            undo_key(rt).is_some() && !rt.state.workspace_host.busy()
        });
        assert_eq!(
            documents.read(&id).unwrap().unwrap().state.title(),
            replacement
        );

        let undo = undo_key(&rt).unwrap();
        rt.update(|_, tx| tx.scroll_into_view(undo.clone()));
        advance(&mut rt, 30);
        click(&mut rt, &undo);
        until(&mut rt, |rt| visible_text(rt, "Undone"));
        assert_eq!(
            documents.read(&id).unwrap().unwrap().state.title(),
            initial_title
        );
        server.join().unwrap();
    }

    #[test]
    fn file_workspace_saves_preferences_without_overwriting_other_fields() {
        use ::settings::{JsonSettingsRepository, MaterialPreference, SettingsRepository};

        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("documents");
        let config = fixture.path().join("settings.json");
        std::fs::create_dir(&root).unwrap();
        let mut rt = runtime();
        rt.configure_processing(Some(config.clone()), None);
        open_test_workspace(&mut rt, &root);
        click(&mut rt, "nav-settings");
        until(&mut rt, |rt| rt.find("live-glass-material").is_some());
        click(&mut rt, "live-glass-material");

        let repository = JsonSettingsRepository::open(&config);
        let mut external = repository.load().unwrap();
        external.expected_speakers = Some(3);
        repository.save(&external).unwrap();
        rt.update(|_, tx| tx.scroll_into_view("save-live-settings"));
        advance(&mut rt, 30);
        click(&mut rt, "save-live-settings");
        until(&mut rt, |rt| visible_text(rt, "Settings saved"));
        let saved = repository.load().unwrap();
        assert_eq!(saved.appearance.material, MaterialPreference::Opaque);
        assert_eq!(saved.expected_speakers, Some(3));
        assert_eq!(saved.documents_root, root.to_str().unwrap());
        assert!(rt.find("workspace-glass").is_none());
        let mut reopened = runtime();
        reopened.configure_processing(Some(config), None);
        reopened.state.open_saved_workspace();
        until(&mut reopened, |rt| rt.state.workspace_host.live.is_some());
        assert_eq!(
            reopened.state.workspace_host.live.as_ref().unwrap().root(),
            root
        );
        assert!(reopened.find("workspace-glass").is_none());
    }

    #[test]
    fn file_workspace_uses_and_persists_the_selected_interface_language() {
        use ::settings::{AppSettings, JsonSettingsRepository, SettingsRepository};

        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("documents");
        let path = fixture.path().join("settings.json");
        std::fs::create_dir(&root).unwrap();
        let repository = JsonSettingsRepository::open(&path);
        repository
            .save(&AppSettings {
                documents_root: root.display().to_string(),
                speaker_library_path: fixture.path().join("library.json").display().to_string(),
                ..AppSettings::default()
            })
            .unwrap();
        let nav_label = |rt: &Runtime<MeetNote>| {
            rt.elements()
                .into_iter()
                .find(|element| element.key.as_deref() == Some("nav-settings"))
                .and_then(|element| element.label)
        };
        let mut rt = runtime();
        rt.state
            .workspace_host
            .configure_processing(Some(path.clone()), None);
        rt.state.open_saved_workspace();
        until(&mut rt, |rt| rt.state.workspace_host.live.is_some());
        assert_eq!(nav_label(&rt).as_deref(), Some("设置"));
        assert!(visible_text(&rt, "还没有录音"));
        click(&mut rt, "nav-models");
        assert!(visible_text(&rt, "模型"));
        click(&mut rt, "nav-assistant");
        assert!(visible_text(&rt, "就工作区文档提问"));
        click(&mut rt, "nav-settings");
        click(&mut rt, "category-speaker-library");
        until(&mut rt, |rt| visible_text(rt, "声纹库中还没有说话人档案"));
        click(&mut rt, "category-appearance");
        click(&mut rt, "live-ui-language-trigger");
        click(&mut rt, "option1");
        click(&mut rt, "save-live-settings");
        until(&mut rt, |rt| nav_label(rt).as_deref() == Some("Settings"));
        assert_eq!(repository.load().unwrap().ui_locale, "en-US");

        let mut reopened = runtime();
        reopened
            .state
            .workspace_host
            .configure_processing(Some(path), None);
        reopened.state.open_saved_workspace();
        until(&mut reopened, |rt| rt.state.workspace_host.live.is_some());
        assert_eq!(nav_label(&reopened).as_deref(), Some("Settings"));
    }

    #[test]
    fn file_workspace_manages_speaker_profiles_and_preserves_their_evidence() {
        use ::settings::{AppSettings, JsonSettingsRepository, SettingsRepository};
        use application::{JsonSpeakerProfileStore, SpeakerProfileStore};
        use contracts::{
            EmbeddingFrontend, EmbeddingNormalization, EmbeddingSpace, SpeakerEmbeddingSample,
            SpeakerLibraryDocument, SpeakerProfile, SpeakerSampleSource,
        };

        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("documents");
        let config = fixture.path().join("settings.json");
        let library = fixture.path().join("library.json");
        let evidence = fixture.path().join("evidence.wav");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(&evidence, []).unwrap();
        JsonSettingsRepository::open(&config)
            .save(&AppSettings {
                speaker_library_path: library.display().to_string(),
                ..AppSettings::default()
            })
            .unwrap();
        let profile = |id: &str, name: &str| SpeakerProfile {
            id: id.into(),
            display_name: name.into(),
            aliases: vec![],
            embedding_space: EmbeddingSpace {
                backend_id: "speaker-backend".into(),
                model_id: "speaker-model".into(),
                model_revision: None,
                dimension: 2,
                sample_rate_hz: 16000,
                frontend: EmbeddingFrontend::default(),
                legacy_feature_config: None,
                normalization: EmbeddingNormalization::L2,
            },
            samples: vec![SpeakerEmbeddingSample {
                id: format!("sample-{id}"),
                vector: vec![1.0, 0.0],
                quality: 1.0,
                source: SpeakerSampleSource {
                    project_id: None,
                    source_segment_id: None,
                    start_ms: 0,
                    end_ms: 2000,
                    audio_path: Some(evidence.display().to_string()),
                    audio_sha256: Some("0".repeat(64)),
                    backup_path: None,
                },
                created_at_ms: 1,
            }],
            match_threshold: 0.6,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        let store = JsonSpeakerProfileStore::new(&library);
        store
            .replace(&SpeakerLibraryDocument {
                profiles: vec![profile("one", "Mira"), profile("two", "Mira copy")],
                ..SpeakerLibraryDocument::default()
            })
            .unwrap();

        let mut rt = runtime();
        rt.configure_processing(Some(config), None);
        open_test_workspace(&mut rt, &root);
        click(&mut rt, "nav-settings");
        click(&mut rt, "category-speaker-library");
        until(&mut rt, |rt| {
            rt.find("library-name-one/wrapper/input").is_some()
        });
        rt.update(|_, tx| tx.scroll_into_view("library-name-one"));
        advance(&mut rt, 30);
        click(&mut rt, "library-name-one/wrapper/input");
        key(&mut rt, Key::End);
        key(&mut rt, Key::Text(" Chen".into()));
        click(&mut rt, "library-rename-one");
        until(&mut rt, |rt| visible_text(rt, "Speaker profile renamed"));
        assert_eq!(store.load().unwrap().profiles[0].display_name, "Mira Chen");

        rt.update(|_, tx| tx.scroll_into_view("library-merge-two"));
        advance(&mut rt, 30);
        click(&mut rt, "library-merge-two");
        click(&mut rt, "library-merge-targets/items/item0");
        click(&mut rt, "dialog-confirm");
        until(&mut rt, |rt| visible_text(rt, "Speaker profiles merged"));
        let merged = store.load().unwrap().profiles;
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].aliases, ["Mira copy"]);
        assert_eq!(merged[0].samples.len(), 2);

        rt.update(|_, tx| tx.scroll_into_view("library-delete-one"));
        advance(&mut rt, 30);
        click(&mut rt, "library-delete-one");
        click(&mut rt, "dialog-cancel");
        assert_eq!(store.load().unwrap().profiles.len(), 1);
        click(&mut rt, "library-delete-one");
        click(&mut rt, "dialog-confirm");
        until(&mut rt, |rt| visible_text(rt, "Speaker profile deleted"));
        assert!(store.load().unwrap().profiles.is_empty());
        assert!(evidence.exists());
    }

    #[test]
    fn file_workspace_switches_speaker_library_directories_without_overwriting_files() {
        use ::settings::{AppSettings, JsonSettingsRepository, SettingsRepository};

        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("documents");
        let config = fixture.path().join("settings.json");
        let source = fixture.path().join("speakers.json");
        let destination = fixture.path().join("new-library");
        let existing = fixture.path().join("existing-library");
        let empty = fixture.path().join("empty-library");
        for directory in [&root, &destination, &existing, &empty] {
            std::fs::create_dir(directory).unwrap();
        }
        let library = br#"{"schemaVersion":1,"profiles":[]}"#;
        let existing_library = br#"{"schemaVersion":1}"#;
        std::fs::write(&source, library).unwrap();
        std::fs::write(existing.join("library.json"), existing_library).unwrap();
        let repository = JsonSettingsRepository::open(&config);
        repository
            .save(&AppSettings {
                speaker_library_path: source.display().to_string(),
                ..AppSettings::default()
            })
            .unwrap();
        let mut rt = runtime();
        rt.configure_processing(Some(config), None);
        open_test_workspace(&mut rt, &root);
        click(&mut rt, "nav-settings");
        click(&mut rt, "category-speaker-library");
        until(&mut rt, |rt| {
            visible_text(rt, "The speaker library has no profiles yet.")
        });
        let choose = |rt: &mut Runtime<MeetNote>, directory: &std::path::Path| {
            click(rt, "choose-speaker-library");
            assert!(matches!(
                rt.file_dialog(),
                Some((rune::FileDialogKind::Folder, _))
            ));
            rt.resolve_file_dialog(Some(directory.to_owned())).unwrap();
        };
        choose(&mut rt, &destination);
        until(&mut rt, |rt| rt.find("dialog-confirm").is_some());
        assert!(!destination.join("library.json").exists());
        click(&mut rt, "dialog-cancel");
        assert_eq!(
            repository.load().unwrap().speaker_library_path,
            source.display().to_string()
        );
        choose(&mut rt, &destination);
        until(&mut rt, |rt| rt.find("dialog-confirm").is_some());
        click(&mut rt, "dialog-confirm");
        until(&mut rt, |rt| {
            visible_text(rt, "Speaker Library location saved")
        });
        assert_eq!(
            std::fs::read(destination.join("library.json")).unwrap(),
            library
        );
        assert_eq!(std::fs::read(&source).unwrap(), library);
        until(&mut rt, |rt| {
            visible_text(rt, "The speaker library has no profiles yet.")
        });

        choose(&mut rt, &existing);
        let existing_path = existing.join("library.json");
        until(&mut rt, |rt| {
            visible_text(rt, existing_path.to_str().unwrap())
        });
        assert!(rt.find("dialog-confirm").is_none());
        assert_eq!(
            repository.load().unwrap().speaker_library_path,
            existing_path.display().to_string()
        );
        assert_eq!(std::fs::read(existing_path).unwrap(), existing_library);

        choose(&mut rt, &empty);
        until(&mut rt, |rt| rt.find("dialog-confirm").is_some());
        click(&mut rt, "library-switch-without-copy");
        let empty_path = empty.join("library.json");
        until(&mut rt, |rt| visible_text(rt, empty_path.to_str().unwrap()));
        assert_eq!(
            repository.load().unwrap().speaker_library_path,
            empty_path.display().to_string()
        );
        assert!(!empty_path.exists());
    }

    #[test]
    fn file_workspace_saves_configurable_diarization_stages() {
        use ::settings::{AppSettings, JsonSettingsRepository, SettingsRepository};
        use contracts::{ModelArtifactRef, ModelRef};

        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("documents");
        let config = fixture.path().join("settings.json");
        let models = fixture.path().join("models");
        std::fs::create_dir(&root).unwrap();
        let repository = JsonSettingsRepository::open(&config);
        repository
            .save(&AppSettings {
                diarization_backend_id: Some("diarization-native".into()),
                diarization_model: Some(ModelRef {
                    backend_id: "diarization-native".into(),
                    model_id: "speakrs-community-1-cpu".into(),
                    revision: None,
                    artifact_path: models.display().to_string(),
                    artifacts: vec![ModelArtifactRef {
                        filename: "segmentation-3.0.onnx".into(),
                        expected_sha256: Some("a".repeat(64)),
                        archive_member: None,
                    }],
                }),
                ..AppSettings::default()
            })
            .unwrap();
        let mut rt = runtime();
        rt.configure_processing(Some(config), Some(models.clone()));
        open_test_workspace(&mut rt, &root);
        click(&mut rt, "nav-settings");
        click(&mut rt, "category-audio");
        let trigger = "live-diarization-segmentation-trigger";
        assert!(rt.find(trigger).is_some());
        rt.update(|_, tx| tx.scroll_into_view(trigger));
        advance(&mut rt, 30);
        click(&mut rt, trigger);
        click(&mut rt, "option1");
        rt.update(|_, tx| tx.scroll_into_view("save-live-settings"));
        advance(&mut rt, 30);
        click(&mut rt, "save-live-settings");
        until(&mut rt, |rt| visible_text(rt, "Settings saved"));
        let mut saved = repository.load().unwrap();
        let stage = saved.diarization_stages.segmentation_model.take().unwrap();
        assert_eq!(stage.model_id, "pyannote-segmentation-3.0");
        assert_eq!(
            stage.artifact_path,
            models.join("segmentation-3.0.onnx").display().to_string()
        );
        assert_eq!(
            stage.artifacts[0].expected_sha256.as_deref(),
            Some("a".repeat(64).as_str())
        );

        saved.diarization_model.as_mut().unwrap().model_id = "speakrs-community-1-coreml".into();
        repository.save(&saved).unwrap();
        click(&mut rt, "reload-live-settings");
        until(&mut rt, |rt| visible_text(rt, "Settings reloaded"));
        assert!(rt.find(trigger).is_none());
    }

    #[test]
    fn file_workspace_models_use_the_catalog_and_save_the_selected_reference() {
        use ::settings::{JsonSettingsRepository, SettingsRepository};

        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("documents");
        let config = fixture.path().join("settings.json");
        let models = fixture.path().join("models");
        let artifact = models.join("asr/whisper-base-en-ggml/ggml-base.en.bin");
        std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        std::fs::write(
            &artifact,
            b"invalid weights for the checksum rejection test",
        )
        .unwrap();
        std::fs::create_dir(&root).unwrap();

        let mut rt = runtime();
        rt.configure_processing(Some(config.clone()), Some(models.clone()));
        open_test_workspace(&mut rt, &root);
        click(&mut rt, "nav-models");
        until(&mut rt, |rt| {
            rt.find("model-whisper-base-en-ggml-details").is_some()
        });
        rt.update(|_, tx| tx.scroll_into_view("model-whisper-base-en-ggml-details"));
        advance(&mut rt, 30);
        click(&mut rt, "model-whisper-base-en-ggml-details");
        rt.update(|_, tx| tx.scroll_into_view("model-whisper-base-en-ggml-select-whisper-rs"));
        advance(&mut rt, 30);
        click(&mut rt, "model-whisper-base-en-ggml-select-whisper-rs");
        until(&mut rt, |rt| {
            rt.find("model-whisper-base-en-ggml-selected").is_some()
        });

        let saved = JsonSettingsRepository::open(&config).load().unwrap();
        assert_eq!(
            saved.asr_model.unwrap().artifact_path,
            artifact.to_str().unwrap()
        );
        rt.update(|_, tx| tx.scroll_into_view("model-whisper-base-en-ggml-probe"));
        advance(&mut rt, 30);
        click(&mut rt, "model-whisper-base-en-ggml-probe");
        until(&mut rt, |rt| {
            rt.find("model-whisper-base-en-ggml-probe-failed").is_some()
        });
        assert!(rt.find("model-whisper-base-en-ggml-checksum-0").is_some());

        let mut reopened = runtime();
        reopened.configure_processing(Some(config), Some(models));
        open_test_workspace(&mut reopened, &root);
        click(&mut reopened, "nav-models");
        until(&mut reopened, |rt| {
            rt.find("model-whisper-base-en-ggml-selected").is_some()
        });
    }

    #[test]
    #[ignore = "downloads and probes the pinned public Speakrs CPU bundle"]
    fn file_workspace_downloads_and_probes_a_real_catalog_model() {
        use models::manager::DownloadState;

        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("documents");
        let model_root = fixture.path().join("models");
        std::fs::create_dir(&root).unwrap();
        let mut rt = runtime();
        rt.configure_processing(
            Some(fixture.path().join("settings.json")),
            Some(model_root.clone()),
        );
        open_test_workspace(&mut rt, &root);
        click(&mut rt, "nav-models");
        let id = "speakrs-community-1-cpu";
        let details = format!("model-{id}-details");
        until(&mut rt, |rt| rt.find(&details).is_some());
        rt.update(|_, tx| tx.scroll_into_view(details.clone()));
        advance(&mut rt, 30);
        click(&mut rt, &details);
        let download = format!("model-{id}-download");
        rt.update(|_, tx| tx.scroll_into_view(download.clone()));
        advance(&mut rt, 30);
        click(&mut rt, &download);
        until_within(&mut rt, Duration::from_secs(180), |rt| {
            rt.state
                .workspace_host
                .models
                .progress(id)
                .is_some_and(|task| task.state.is_terminal())
        });
        let task = rt.state.workspace_host.models.progress(id).unwrap();
        assert_eq!(task.state, DownloadState::Completed);
        assert!(task.artifact_count > 1);
        assert_eq!(task.artifact_index, task.artifact_count);
        until(&mut rt, |rt| {
            rt.find(&format!("model-{id}-probe-passed")).is_some()
        });
        let segmentation = model_root
            .join("diarization")
            .join(id)
            .join("segmentation-3.0.onnx");
        assert!(segmentation.metadata().unwrap().len() > 0);
    }

    #[test]
    fn file_workspace_requires_installed_models_for_processing() {
        use application::DocumentService;
        use persistence::FileDocumentRepository;
        use std::sync::Arc;

        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("documents");
        let audio = fixture.path().join("recording.wav");
        std::fs::write(&audio, []).unwrap();
        let documents =
            DocumentService::new(Arc::new(FileDocumentRepository::open(&root).unwrap()));
        let state = documents.import_audio(audio.to_str().unwrap()).unwrap();

        let mut rt = runtime();
        rt.configure_processing(
            Some(fixture.path().join("settings.json")),
            Some(fixture.path().join("models")),
        );
        open_test_workspace(&mut rt, &root);
        until(&mut rt, |rt| rt.find("audio-source-available").is_some());
        click(&mut rt, "process-document");
        until(&mut rt, |rt| visible_text(rt, "Install an ASR model"));
        assert!(!rt.state.workspace_host.busy());
        assert!(
            documents
                .read(state.id())
                .unwrap()
                .unwrap()
                .state
                .processing
                .latest
                .is_none()
        );
    }

    #[test]
    #[ignore = "requires explicit MEETNOTE_RUNE_TEST_MODEL_ROOT and MEETNOTE_RUNE_TEST_AUDIO"]
    fn file_workspace_processes_real_audio_and_reopens_the_record() {
        use ::settings::{JsonSettingsRepository, SettingsRepository};
        use application::DocumentService;
        use persistence::FileDocumentRepository;
        use std::path::PathBuf;
        use std::sync::Arc;

        let model_root = PathBuf::from(std::env::var_os("MEETNOTE_RUNE_TEST_MODEL_ROOT").unwrap());
        let audio = PathBuf::from(std::env::var_os("MEETNOTE_RUNE_TEST_AUDIO").unwrap());
        assert!(model_root.is_dir() && audio.is_file());
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("documents");
        let settings_path = fixture.path().join("settings.json");
        let documents =
            DocumentService::new(Arc::new(FileDocumentRepository::open(&root).unwrap()));
        let mut rt = runtime();
        rt.configure_processing(Some(settings_path.clone()), Some(model_root));
        open_test_workspace(&mut rt, &root);
        click(&mut rt, "nav-models");
        let model = "model-speakrs-community-1-cpu-details";
        until(&mut rt, |rt| rt.find(model).is_some());
        rt.update(|_, tx| tx.scroll_into_view(model));
        advance(&mut rt, 30);
        click(&mut rt, model);
        let select = "model-speakrs-community-1-cpu-select-diarization-native";
        rt.update(|_, tx| tx.scroll_into_view(select));
        advance(&mut rt, 30);
        click(&mut rt, select);
        until(&mut rt, |rt| {
            rt.find("model-speakrs-community-1-cpu-selected").is_some()
        });
        click(&mut rt, "nav-settings");
        click(&mut rt, "category-audio");
        for stage in ["segmentation", "embedding", "clustering"] {
            let trigger = format!("live-diarization-{stage}-trigger");
            rt.update(|_, tx| tx.scroll_into_view(trigger.clone()));
            advance(&mut rt, 30);
            click(&mut rt, &trigger);
            click(&mut rt, "option1");
        }
        rt.update(|_, tx| tx.scroll_into_view("save-live-settings"));
        advance(&mut rt, 30);
        click(&mut rt, "save-live-settings");
        until(&mut rt, |rt| visible_text(rt, "Settings saved"));
        let stages = JsonSettingsRepository::open(&settings_path)
            .load()
            .unwrap()
            .diarization_stages;
        assert!(stages.segmentation_model.is_some());
        assert!(stages.embedding_model.is_some());
        assert!(stages.clustering_algorithm.is_some());
        click(&mut rt, "nav-documents");
        click(&mut rt, "empty-import-audio");
        rt.resolve_file_dialog(Some(audio)).unwrap();
        until(&mut rt, |rt| rt.find("audio-source-available").is_some());
        let id = domain::ProjectId::new(rt.state.documents.selection().0);
        click(&mut rt, "process-document");
        until_within(&mut rt, Duration::from_secs(150), |rt| {
            !rt.state.workspace_host.busy()
        });
        assert!(
            visible_text(&rt, "Audio processing finished"),
            "{:?}",
            rt.state.toast
        );
        let saved = documents.read(&id).unwrap().unwrap();
        assert!(!saved.state.results.transcript.is_empty());
        assert!(!saved.state.results.diarization.is_empty());
        assert!(!saved.state.results.speaker_segments.is_empty());
        let processing = saved.state.processing.latest.unwrap();
        assert_eq!(processing.asr.backend_id, "whisper-rs");
        assert_eq!(processing.diarization.backend_id, "diarization-native");
        let checksum = processing.input_audio_checksum.unwrap();

        let mut reopened = runtime();
        open_test_workspace(&mut reopened, &root);
        click(&mut reopened, "detail-toggle");
        let detail_value = |key: &str| {
            reopened
                .elements()
                .into_iter()
                .find(|element| element.key.as_deref() == Some(key))
                .and_then(|element| element.text)
                .unwrap_or_else(|| panic!("missing detail field {key}"))
        };
        assert_eq!(detail_value("detail_input_checksum"), checksum);
        assert!(detail_value("detail_asr").contains("whisper-rs"));
        assert!(detail_value("detail_diarization").contains("diarization-native"));
        assert!(
            detail_value("detail_transcript_segments")
                .parse::<usize>()
                .unwrap()
                > 0
        );
        assert!(reopened.find("detail-transcript-0").is_some());
        click(&mut reopened, "document-modes/segment1");
        assert!(reopened.find("transcript-0").is_some());
    }

    fn visible_text(rt: &Runtime<MeetNote>, text: &str) -> bool {
        rt.elements().iter().any(|element| {
            !element.exiting
                && element.visible[2] > 0.0
                && element.visible[3] > 0.0
                && element
                    .text
                    .as_deref()
                    .is_some_and(|value| value.contains(text))
        })
    }

    fn accessibility_node<'a>(
        tree: &'a serde_json::Value,
        key: &str,
    ) -> Option<&'a serde_json::Value> {
        if tree["key"] == key {
            return Some(tree);
        }
        tree["children"]
            .as_array()?
            .iter()
            .find_map(|node| accessibility_node(node, key))
    }

    #[test]
    fn accessibility_exposes_the_project_tree_pane_and_keyboard_modal() {
        let mut rt = runtime();
        rt.set_full_keyboard_access(true);
        let tree = rt.accessibility_json();
        let sidebar = accessibility_node(&tree, "pane-resize").unwrap();
        assert_eq!(sidebar["role"], "splitter");
        assert_eq!(sidebar["name"], "Resize sidebar");
        assert!(
            sidebar["actions"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("increment"))
        );
        assert_eq!(
            accessibility_node(&tree, "document-tree").unwrap()["role"],
            "tree"
        );
        assert_eq!(
            accessibility_node(&tree, "nav-documents").unwrap()["role"],
            "tab"
        );

        rt.update(|_, tx| tx.focus("import-audio", true));
        advance(&mut rt, 2);
        key(&mut rt, Key::Text(" ".into()));
        let modal = rt.accessibility_json();
        assert_eq!(
            accessibility_node(&modal, "confirm-dialog-dialog").unwrap()["role"],
            "dialog"
        );
        assert!(accessibility_node(&modal, "nav-documents").is_none());
        key(&mut rt, Key::Escape);
        assert!(
            rt.elements()
                .iter()
                .any(|element| element.key.as_deref() == Some("import-audio") && element.focused)
        );
    }

    #[test]
    fn reduced_transparency_uses_opaque_material_without_changing_the_preference() {
        let mut rt = runtime();
        assert!(rt.find("workspace-glass").is_some());
        rt.update(|state, _| state.reduce_transparency = true);
        advance(&mut rt, 30);
        assert!(rt.find("workspace-glass").is_none());
        assert!(rt.find("shell-glass").is_none());
        assert!(rt.state.settings.glass);
    }

    #[test]
    fn document_search_shows_results_after_enter_and_keeps_them_until_cleared() {
        let mut rt = runtime();
        click(&mut rt, "workspace-search/value/input");
        key(&mut rt, Key::Text("Design review".into()));
        assert!(rt.find("research").is_some());
        key(&mut rt, Key::Enter);
        assert_eq!(rt.state.documents.selection().0, "product");
        assert!(rt.find("document-tree").is_none());
        assert!(visible_text(&rt, "Title"));
        click(&mut rt, "search-result-design");
        assert_eq!(rt.state.documents.selection().1, "Design review");
        assert!(rt.find("search-result-design").is_some());
        click(&mut rt, "workspace-search/action");
        assert!(rt.find("research").is_some());
    }

    #[test]
    fn file_workspace_search_finds_unopened_markdown() {
        use application::{DocumentCommandService, DocumentService};
        use contracts::DocumentPatch;
        use persistence::FileDocumentRepository;
        use std::sync::Arc;

        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("documents");
        let documents =
            DocumentService::new(Arc::new(FileDocumentRepository::open(&root).unwrap()));
        let source = fixture.path().join("recording.wav");
        std::fs::write(&source, []).unwrap();
        let state = documents.import_audio(source.to_str().unwrap()).unwrap();
        DocumentCommandService::new(documents)
            .update_document(
                state.id(),
                state.revision(),
                DocumentPatch {
                    markdown: Some("# Notes\n\n独立组件验收".into()),
                    ..Default::default()
                },
            )
            .unwrap();

        let mut rt = runtime();
        open_test_workspace(&mut rt, &root);
        click(&mut rt, "workspace-search/value/input");
        key(&mut rt, Key::Text("组件验收".into()));
        key(&mut rt, Key::Enter);
        let result = format!("search-result-{}", state.id().as_str());
        until(&mut rt, |rt| rt.find(&result).is_some());
        click(&mut rt, &result);
        assert!(rt.state.documents.mode == documents::Mode::Notes);
        until(&mut rt, |rt| visible_text(rt, "独立组件验收"));
    }

    #[test]
    fn file_workspace_observes_external_markdown_without_discarding_a_draft() {
        use application::DocumentService;
        use persistence::{DOCUMENT_MARKDOWN_FILENAME, FileDocumentRepository};
        use std::sync::Arc;

        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("documents");
        let documents =
            DocumentService::new(Arc::new(FileDocumentRepository::open(&root).unwrap()));
        let source = fixture.path().join("recording.wav");
        std::fs::write(&source, []).unwrap();
        let state = documents.import_audio(source.to_str().unwrap()).unwrap();
        let markdown = root
            .join(state.id().as_str())
            .join(DOCUMENT_MARKDOWN_FILENAME);

        let mut rt = runtime();
        open_test_workspace(&mut rt, &root);
        click(&mut rt, "document-modes/segment2");
        until(&mut rt, |rt| rt.find("copy-notes").is_some());
        std::fs::write(&markdown, "# Revised outside MeetNote\n").unwrap();
        until(&mut rt, |rt| visible_text(rt, "Revised outside MeetNote"));

        click(&mut rt, "document-modes/segment3");
        click(&mut rt, "document-title-edit/wrapper/input");
        key(&mut rt, Key::End);
        key(&mut rt, Key::Text(" local draft".into()));
        std::fs::write(&markdown, "# Second external edit\n").unwrap();
        until(&mut rt, |rt| rt.find("reload-draft-notes").is_some());
        assert!(visible_text(&rt, "recording local draft"));
        assert_eq!(
            documents
                .read(state.id())
                .unwrap()
                .unwrap()
                .state
                .revision(),
            state.revision()
        );
    }

    #[test]
    fn file_workspace_save_checks_markdown_before_a_watch_event() {
        use application::DocumentService;
        use persistence::{DOCUMENT_MARKDOWN_FILENAME, FileDocumentRepository};
        use std::sync::Arc;

        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("documents");
        let documents =
            DocumentService::new(Arc::new(FileDocumentRepository::open(&root).unwrap()));
        let source = fixture.path().join("recording.wav");
        std::fs::write(&source, []).unwrap();
        let state = documents.import_audio(source.to_str().unwrap()).unwrap();
        let markdown = root
            .join(state.id().as_str())
            .join(DOCUMENT_MARKDOWN_FILENAME);

        let mut rt = runtime();
        open_test_workspace(&mut rt, &root);
        click(&mut rt, "document-modes/segment3");
        until(&mut rt, |rt| rt.find("document-notes-edit/input").is_some());
        rt.update(|_, tx| tx.scroll_into_view("document-notes-edit"));
        advance(&mut rt, 30);
        click(&mut rt, "document-notes-edit/input");
        key(&mut rt, Key::Text("Local draft text".into()));
        assert!(rt.state.documents.has_unsaved_edits());
        rt.update(|s, tx| {
            std::fs::write(&markdown, "# External edit before saving\n").unwrap();
            s.save_document(tx);
        });
        until(&mut rt, |rt| !rt.state.workspace_host.busy());
        assert_eq!(
            std::fs::read_to_string(&markdown).unwrap(),
            "# External edit before saving\n"
        );
        assert!(rt.state.documents.has_unsaved_edits());
    }

    #[test]
    fn transcript_search_opens_and_reveals_a_later_transcript_paragraph() {
        let mut rt = runtime();
        click(&mut rt, "workspace-search/value/input");
        key(&mut rt, Key::Text("draft checklist".into()));
        key(&mut rt, Key::Enter);
        assert!(visible_text(&rt, "Transcript · 11:42"));
        click(&mut rt, "search-result-product");
        assert!(rt.state.documents.mode == documents::Mode::Original);
        assert!(visible_text(&rt, "draft checklist on Thursday"));
        assert!(visible_text(&rt, "Search match"));
    }

    #[test]
    fn transcript_speaker_rename_and_confirmed_merge_keep_the_original_words() {
        let mut rt = runtime();
        click(&mut rt, "document-modes/segment1");
        click(&mut rt, "speakers");
        click(&mut rt, "speaker-name-speaker-1/value/input");
        command(&mut rt, 'a', false, false);
        key(&mut rt, Key::Text("Facilitator".into()));
        key(&mut rt, Key::Enter);
        assert!(visible_text(&rt, "Facilitator"));
        click(&mut rt, "speaker-merge-speaker-2");
        assert!(rt.find("confirm-dialog-dialog").is_some());
        key(&mut rt, Key::Escape);
        assert!(rt.find("speaker-name-speaker-2").is_some());
        click(&mut rt, "speaker-merge-speaker-2");
        click(&mut rt, "merge-targets/items/item0");
        click(&mut rt, "dialog-confirm");
        assert!(rt.find("speaker-name-speaker-2").is_none());
        assert!(visible_text(&rt, "Speakers · 1"));
        assert!(!visible_text(&rt, "Morgan Park"));
        let transcript: Vec<_> = rt
            .elements()
            .into_iter()
            .filter(|node| node.path.contains("/transcript-1/"))
            .filter_map(|node| node.text)
            .collect();
        assert!(transcript.iter().any(|text| text == "Facilitator"));
        assert!(
            transcript
                .iter()
                .any(|text| text.starts_with("Search is ready for the pilot."))
        );
    }

    #[test]
    fn file_workspace_persists_speaker_corrections_and_keeps_transcript_words() {
        use application::DocumentService;
        use domain::SpeakerTaggedSegment;
        use persistence::FileDocumentRepository;
        use std::sync::Arc;

        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("documents");
        let documents =
            DocumentService::new(Arc::new(FileDocumentRepository::open(&root).unwrap()));
        let source = fixture.path().join("recording.wav");
        std::fs::write(&source, []).unwrap();
        let mut state = documents.import_audio(source.to_str().unwrap()).unwrap();
        let id = state.id().clone();
        for index in 1..=2 {
            state.results.speaker_segments.push(SpeakerTaggedSegment {
                transcript_segment_id: format!("segment-{index}"),
                speaker_id: format!("speaker-{index}"),
                speaker_profile_id: None,
                speaker_label: None,
                start_ms: index * 2000,
                end_ms: index * 2000 + 1000,
                text: format!("Original words {index}"),
            });
        }
        documents.save(state).unwrap();

        let mut rt = runtime();
        open_test_workspace(&mut rt, &root);
        click(&mut rt, "document-modes/segment1");
        click(&mut rt, "speakers");
        click(&mut rt, "speaker-name-speaker-1/value/input");
        command(&mut rt, 'a', false, false);
        key(&mut rt, Key::Text("Facilitator".into()));
        key(&mut rt, Key::Enter);
        until(&mut rt, |rt| !rt.state.workspace_host.busy());
        let renamed = documents.read(&id).unwrap().unwrap();
        assert_eq!(
            renamed.state.results.speaker_segments[0]
                .speaker_label
                .as_deref(),
            Some("Facilitator")
        );
        click(&mut rt, "speaker-merge-speaker-2");
        click(&mut rt, "merge-targets/items/item0");
        click(&mut rt, "dialog-confirm");
        until(&mut rt, |rt| rt.find("speaker-name-speaker-2").is_none());

        let saved = documents.read(&id).unwrap().unwrap();
        for (index, segment) in saved.state.results.speaker_segments.iter().enumerate() {
            assert_eq!(segment.speaker_id, "speaker-1");
            assert_eq!(segment.speaker_label.as_deref(), Some("Facilitator"));
            assert_eq!(segment.text, format!("Original words {}", index + 1));
        }
    }

    #[test]
    fn document_routes_and_modal_are_interactive_at_minimum_window_size() {
        let mut rt = runtime();
        click(&mut rt, "detail-toggle");
        assert!(rt.find("detail-panel").is_some());
        click(&mut rt, "detail-close");
        assert!(rt.find("detail-panel").is_none());
        click(&mut rt, "document-modes/segment1");
        assert!(visible_text(&rt, "Original transcript"));
        click(&mut rt, "import-audio");
        assert!(rt.find("confirm-dialog-dialog").is_some());
        key(&mut rt, Key::Escape);
        assert!(rt.find("confirm-dialog-dialog").is_none());
        click(&mut rt, "import-audio");
        click(&mut rt, "dialog-confirm");
        assert_eq!(rt.state.documents.selection().1, "Sample recording 1");
        click(&mut rt, "process-sample");
        assert!(rt.find("audio-progress").is_some());
        advance(&mut rt, 190);
        assert!(visible_text(&rt, "Sample transcription finished"));
    }

    #[test]
    fn settings_dropdown_and_download_controls_update_visible_state() {
        let mut rt = runtime();
        click(&mut rt, "nav-settings");
        click(&mut rt, "theme-trigger");
        click(&mut rt, "option1");
        assert!(!rt.state.settings.dark(true));
        click(&mut rt, "nav-models");
        rt.wheel(500.0, 400.0, -175.0);
        advance(&mut rt, 5);
        click(&mut rt, "model-qwen-start");
        assert!(rt.find("model-qwen-progress").is_some());
        click(&mut rt, "model-qwen-pause");
        assert!(visible_text(&rt, "Paused"));
        click(&mut rt, "model-qwen-start");
        advance(&mut rt, 320);
        assert!(rt.find("model-qwen-probe").is_some());
    }

    #[test]
    fn assistant_review_applies_and_undoes_saved_notes() {
        let mut rt = runtime();
        click(&mut rt, "nav-assistant");
        click(&mut rt, "suggestion-1");
        click(&mut rt, "assistant-send");
        advance(&mut rt, 50);
        click(&mut rt, "assistant-accept");
        click(&mut rt, "dialog-confirm");
        assert!(visible_text(&rt, "Action item appended"));
        click(&mut rt, "nav-documents");
        click(&mut rt, "document-modes/segment2");
        assert!(visible_text(&rt, "Share the workspace recovery checklist"));
        click(&mut rt, "nav-assistant");
        click(&mut rt, "assistant-undo");
        assert!(visible_text(&rt, "The original notes were restored"));
        click(&mut rt, "nav-documents");
        assert!(!visible_text(&rt, "Share the workspace recovery checklist"));
    }
    fn command(rt: &mut Runtime<MeetNote>, character: char, ctrl: bool, shift: bool) {
        rt.key(KeyEvent {
            key: Key::Command(character),
            mods: Mods {
                cmd: true,
                ctrl,
                shift,
                ..Mods::default()
            },
        });
        advance(rt, 30);
    }

    #[test]
    fn history_restores_documents_and_discards_forward_entries_on_a_new_route() {
        let mut rt = runtime();
        click(&mut rt, "design");
        command(&mut rt, '4', false, false);
        assert!(rt.state.route == Route::Settings);
        click(&mut rt, "navigate-back");
        assert_eq!(rt.state.documents.selection().0, "design");
        command(&mut rt, '[', false, false);
        assert_eq!(rt.state.documents.selection().0, "product");
        click(&mut rt, "navigate-forward");
        assert_eq!(rt.state.documents.selection().0, "design");
        command(&mut rt, '3', false, false);
        command(&mut rt, ']', false, false);
        assert!(rt.state.route == Route::Models);
    }

    #[test]
    fn sidebar_folds_keeps_navigation_and_reopens_for_search() {
        let mut rt = runtime();
        let (rail, _) = rt.presented("navigation-rail").unwrap();
        command(&mut rt, 's', true, false);
        assert!(rt.presented("sidebar-slot").unwrap().0[2] < 0.5);
        assert_eq!(rt.presented("navigation-rail").unwrap().0, rail);
        command(&mut rt, 'f', false, true);
        assert!(rt.find("project-sidebar").is_some());
        key(&mut rt, Key::Text("Design review".into()));
        key(&mut rt, Key::Enter);
        assert!(rt.find("search-result-design").is_some());
    }

    #[test]
    fn window_shell_wraps_the_workspace_in_both_materials() {
        for glass in [true, false] {
            let mut state = MeetNote::sample();
            state.settings.glass = glass;
            for route in [Route::Documents, Route::Settings] {
                state.navigate(route);
                let mut rt = Runtime::new(
                    state,
                    view,
                    (900.0, 600.0),
                    Env {
                        system_dark: !glass,
                        scale: 1.0,
                    },
                );
                rt.frame(0.0);

                let elements = rt.elements();
                let element = |key: &str| {
                    elements
                        .iter()
                        .find(|el| el.key.as_deref() == Some(key))
                        .unwrap_or_else(|| panic!("missing {key}"))
                };
                let shell = element("window-shell");
                assert_eq!(shell.rect, [0.0, 0.0, 900.0, 600.0]);

                let frame = element("inner-frame");
                assert_eq!(frame.rect, [64.0, 46.0, 831.0, 549.0]);
                assert_eq!(element("frame-outline").rect, frame.rect);
                assert_eq!(element("frame-outline").radius, 10.0);
                assert_eq!(element("sidebar-slot").radius, 0.0);
                assert_eq!(element("sidebar-slot").fill.a, 0.0);
                assert_eq!(element("content-plane").fill.a, 0.0);
                if glass {
                    let (shell_backdrop, shell_opacity) = rt.presented("shell-glass").unwrap();
                    let (workspace_backdrop, workspace_opacity) =
                        rt.presented("workspace-glass").unwrap();
                    assert_eq!(shell_backdrop, shell.rect);
                    assert_eq!(workspace_backdrop, frame.rect);
                    assert_eq!(element("workspace-glass").radius, 10.0);
                    assert_eq!(shell_opacity, 1.0);
                    assert_eq!(workspace_opacity, 1.0);
                } else {
                    assert_eq!(shell.fill.a, 1.0);
                    assert_eq!(frame.fill.a, 1.0);
                    assert!(rt.presented("shell-glass").is_none());
                    assert!(rt.presented("workspace-glass").is_none());
                }
                state = rt.state;
            }
        }
    }

    #[test]
    fn sidebar_fold_animates_the_seam_and_can_reverse_without_jumping() {
        let mut rt = runtime();
        let left = |rt: &Runtime<MeetNote>| rt.presented("content-plane").unwrap().0[0];
        let open = left(&rt);
        let (x, y) = rt.find("toggle-sidebar").unwrap();
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        advance(&mut rt, 5);
        let folding = left(&rt);
        assert!(folding > 64.0 && folding < open);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        advance(&mut rt, 1);
        assert!((left(&rt) - folding).abs() < 35.0);
        advance(&mut rt, 40);
        assert!((left(&rt) - open).abs() < 0.5);
    }

    #[test]
    fn sidebar_honors_reduce_motion_and_excludes_closed_controls_from_focus() {
        let mut rt = runtime();
        rt.set_system_reduced_motion(true);
        advance(&mut rt, 1);
        let (x, y) = rt.find("toggle-sidebar").unwrap();
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        advance(&mut rt, 2);
        assert!(rt.presented("sidebar-slot").unwrap().0[2] < 0.5);
        assert!(
            rt.elements()
                .iter()
                .any(|node| node.key.as_deref() == Some("document-tree")
                    && node.inert
                    && !node.focused)
        );
    }

    #[test]
    fn detail_close_returns_keyboard_focus_to_its_trigger() {
        let mut rt = runtime();
        rt.set_full_keyboard_access(true);
        click(&mut rt, "detail-toggle");
        let close_focused = |rt: &Runtime<MeetNote>| {
            rt.elements()
                .iter()
                .any(|node| node.key.as_deref() == Some("detail-close") && node.focused)
        };
        for _ in 0..20 {
            if close_focused(&rt) {
                break;
            }
            key(&mut rt, Key::Tab);
        }
        assert!(close_focused(&rt));
        key(&mut rt, Key::Text(" ".into()));
        assert!(rt.find("detail-panel").is_none());
        assert!(
            rt.elements()
                .iter()
                .any(|node| node.key.as_deref() == Some("detail-toggle")
                    && node.focused
                    && node.focus_visible)
        );
    }

    #[test]
    fn project_tree_option_arrows_reorder_and_move_between_levels() {
        let mut rt = runtime();
        let option = |rt: &mut Runtime<MeetNote>, key| {
            rt.key(KeyEvent {
                key,
                mods: Mods {
                    alt: true,
                    ..Mods::default()
                },
            });
            advance(rt, 30);
        };
        click(&mut rt, "research");
        option(&mut rt, Key::Up);
        assert!(rt.find("research").unwrap().1 < rt.find("design").unwrap().1);
        click(&mut rt, "actions");
        option(&mut rt, Key::Left);
        assert!(
            rt.elements()
                .iter()
                .any(|n| n.key.as_deref() == Some("actions") && n.level == Some(2))
        );
        option(&mut rt, Key::Right);
        assert!(
            rt.elements()
                .iter()
                .any(|n| n.key.as_deref() == Some("actions") && n.level == Some(3))
        );
    }

    #[test]
    fn project_tree_drag_reorders_and_rejects_a_cross_audio_drop() {
        let mut rt = runtime();
        let drag = |rt: &mut Runtime<MeetNote>, source: &str, target: &str, offset: f32| {
            let (x, y) = rt.find(source).unwrap();
            let (target_x, target_y) = rt.find(target).unwrap();
            rt.pointer_move(x, y);
            rt.pointer_down(x, y);
            advance(rt, 1);
            rt.pointer_move(target_x, target_y + offset);
            advance(rt, 3);
            rt.pointer_up(target_x, target_y + offset);
            advance(rt, 30);
        };
        drag(&mut rt, "research", "design", -9.0);
        assert!(rt.find("research").unwrap().1 < rt.find("design").unwrap().1);
        drag(&mut rt, "research", "design", 0.0);
        assert!(visible_text(&rt, "same audio source"));
        assert!(
            rt.elements()
                .iter()
                .any(|n| n.key.as_deref() == Some("research") && n.level == Some(1))
        );
    }

    #[test]
    fn file_workspace_drag_persists_sibling_order() {
        use application::DocumentService;
        use persistence::FileDocumentRepository;
        use std::sync::Arc;

        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("documents");
        let documents =
            DocumentService::new(Arc::new(FileDocumentRepository::open(&root).unwrap()));
        for file in ["first.wav", "second.wav"] {
            let source = fixture.path().join(file);
            std::fs::write(&source, []).unwrap();
            documents.import_audio(source.to_str().unwrap()).unwrap();
        }

        let mut rt = runtime();
        open_test_workspace(&mut rt, &root);
        advance(&mut rt, 30);
        let mut ids = documents.list().unwrap();
        ids.sort_by(|a, b| {
            rt.find(a.id().as_str())
                .unwrap()
                .1
                .total_cmp(&rt.find(b.id().as_str()).unwrap().1)
        });
        let (target, source) = (ids[0].id(), ids[1].id());
        let (x, y) = rt.find(source.as_str()).unwrap();
        let (tx, ty) = rt
            .find(&format!("drop-before:{}", target.as_str()))
            .unwrap();
        assert!(
            (ty - y).abs() > 4.0,
            "the pointer must move beyond click slop"
        );
        rt.pointer_down(x, y);
        advance(&mut rt, 1);
        rt.pointer_move(tx, ty);
        advance(&mut rt, 3);
        assert_eq!(
            documents.read(source).unwrap().unwrap().state.revision(),
            ids[1].revision()
        );
        rt.pointer_up(tx, ty);
        until(&mut rt, |rt| {
            rt.find(source.as_str()).unwrap().1 < rt.find(target.as_str()).unwrap().1
        });

        let mut reopened = runtime();
        open_test_workspace(&mut reopened, &root);
        assert!(
            reopened.find(source.as_str()).unwrap().1 < reopened.find(target.as_str()).unwrap().1
        );
    }

    #[test]
    fn project_tree_drag_changes_parent_only_when_released() {
        let mut rt = runtime();
        let level = |rt: &Runtime<MeetNote>| {
            rt.elements()
                .into_iter()
                .find(|node| node.key.as_deref() == Some("actions"))
                .and_then(|node| node.level)
        };
        let (x, y) = rt.find("actions").unwrap();
        let (target_x, target_y) = rt.find("product").unwrap();
        rt.pointer_down(x, y);
        advance(&mut rt, 1);
        rt.pointer_move(target_x, target_y);
        advance(&mut rt, 4);
        assert_eq!(level(&rt), Some(3));
        key(&mut rt, Key::Escape);
        rt.pointer_up(target_x, target_y);
        advance(&mut rt, 25);
        assert_eq!(level(&rt), Some(3));

        rt.pointer_down(x, y);
        advance(&mut rt, 1);
        rt.pointer_move(target_x, target_y);
        advance(&mut rt, 4);
        rt.pointer_up(target_x, target_y);
        advance(&mut rt, 30);
        assert_eq!(level(&rt), Some(2));
    }

    #[test]
    fn sidebar_drag_clamps_width_and_does_not_move_the_rail() {
        let mut rt = runtime();
        let rail = rt.presented("navigation-rail").unwrap().0;
        let (x, y) = rt.find("pane-resize").unwrap();
        rt.pointer_down(x, y);
        advance(&mut rt, 1);
        rt.pointer_move(x + 60.0, y);
        advance(&mut rt, 1);
        assert_eq!(rt.presented("project-sidebar").unwrap().0[2], 286.0);
        rt.pointer_move(x + 450.0, y);
        advance(&mut rt, 1);
        let seam = |rt: &Runtime<MeetNote>| rt.presented("content-plane").unwrap().0[0];
        let limit = 64.0 + 360.0;
        assert!(seam(&rt) > limit + 5.0 && seam(&rt) <= limit + 16.0);
        rt.pointer_up(x + 450.0, y);
        let mut crossed_limit = false;
        for _ in 0..60 {
            advance(&mut rt, 1);
            crossed_limit |= seam(&rt) < limit - 0.5;
            let divider = rt.presented("pane-divider").unwrap().0[0];
            assert!((divider - seam(&rt)).abs() < 0.5);
        }
        assert!(
            crossed_limit,
            "the released divider should bounce past its resting edge"
        );
        assert!((seam(&rt) - limit).abs() < 0.5);
        assert_eq!(rt.presented("project-sidebar").unwrap().0[2], 360.0);

        let (x, y) = rt.find("pane-resize").unwrap();
        rt.pointer_down(x, y);
        advance(&mut rt, 1);
        rt.pointer_move(x - 400.0, y);
        advance(&mut rt, 1);
        let limit = 64.0 + 192.0;
        assert!(seam(&rt) < limit - 5.0 && seam(&rt) >= limit - 16.0);
        rt.pointer_up(x - 400.0, y);
        let mut crossed_limit = false;
        for _ in 0..60 {
            advance(&mut rt, 1);
            crossed_limit |= seam(&rt) > limit + 0.5;
        }
        assert!(crossed_limit, "the minimum width should rebound as well");
        assert!((seam(&rt) - limit).abs() < 0.5);
        assert_eq!(rt.presented("navigation-rail").unwrap().0, rail);
    }

    #[test]
    fn sidebar_glow_eases_in_and_out_and_reduced_motion_skips_resistance() {
        let mut rt = runtime();
        let (x, y) = rt.find("pane-resize").unwrap();
        let glow = |rt: &Runtime<MeetNote>| rt.presented("pane-resize-glow").unwrap().1;
        assert_eq!(glow(&rt), 0.0);
        rt.pointer_move(x, y);
        advance(&mut rt, 2);
        assert!((0.0..1.0).contains(&glow(&rt)) && glow(&rt) > 0.0);
        advance(&mut rt, 30);
        assert!(glow(&rt) > 0.99);
        let ([left, _, width, _], _) = rt.presented("pane-resize-glow").unwrap();
        let seam = rt.presented("content-plane").unwrap().0[0];
        assert!((left + width / 2.0 - seam).abs() < 0.5);
        assert!(left < seam - 4.0 && left + width > seam + 4.0);
        assert!((width - 9.0).abs() < 0.5, "glow width was {width}");
        rt.pointer_move(x + 100.0, y);
        advance(&mut rt, 2);
        assert!((0.0..1.0).contains(&glow(&rt)) && glow(&rt) > 0.0);
        advance(&mut rt, 30);
        assert!(glow(&rt) < 0.01);

        rt.set_system_reduced_motion(true);
        advance(&mut rt, 1);
        rt.pointer_down(x, y);
        rt.pointer_move(x - 100.0, y);
        advance(&mut rt, 1);
        let limit = 64.0 + 192.0;
        assert_eq!(rt.presented("content-plane").unwrap().0[0], limit);
        rt.pointer_up(x - 100.0, y);
        advance(&mut rt, 2);
        assert_eq!(rt.presented("content-plane").unwrap().0[0], limit);
    }

    #[test]
    fn settings_categories_reveal_offscreen_sections() {
        let mut rt = runtime();
        click(&mut rt, "nav-settings");
        assert!(!visible_text(&rt, "Manage the sample speaker profile."));
        click(&mut rt, "category-speaker-library");
        assert!(visible_text(&rt, "Manage the sample speaker profile."));
        click(&mut rt, "category-appearance");
        assert!(visible_text(&rt, "Customize the MeetNote window."));
    }

    #[test]
    fn edited_tags_are_searchable_only_after_saving() {
        let mut rt = runtime();
        click(&mut rt, "document-modes/segment3");
        click(&mut rt, "document-tags-edit/wrapper/input");
        key(&mut rt, Key::Text("pilot, rollout-priority，pilot".into()));
        command(&mut rt, 'f', false, true);
        key(&mut rt, Key::Text("rollout-priority".into()));
        key(&mut rt, Key::Enter);
        assert!(rt.find("search-result-product").is_none());
        command(&mut rt, 's', false, false);
        assert!(rt.find("search-result-product").is_some());
        assert!(visible_text(&rt, "Tag"));
    }

    #[test]
    fn switching_documents_preserves_an_unsaved_draft_without_saving_it() {
        let mut rt = runtime();
        click(&mut rt, "document-modes/segment3");
        click(&mut rt, "document-title-edit/wrapper/input");
        key(&mut rt, Key::End);
        key(&mut rt, Key::Text(" draft".into()));
        click(&mut rt, "design");
        click(&mut rt, "product");
        assert_eq!(rt.state.documents.selection().1, "Product sync");
        click(&mut rt, "document-modes/segment3");
        assert!(visible_text(&rt, "Product sync draft"));
    }
}
