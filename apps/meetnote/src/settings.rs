use std::time::Duration;

use rune::{El, TextEdit, Tx, col, el, row};
use rune_kit::{
    Badge, BadgeVariant, Button, ButtonVariant, Collapsible, Divider, Kit, Progress, RadioGroup,
    RadioItem, SidebarRow, Switch, TextField, TextInput,
};

use crate::controls::{
    action, caption, card, dot, eyebrow, field, label, page_header, scroll, select, title,
};
use crate::{Confirm, MeetNote};

mod live_connection;
mod live_models;
mod live_preferences;
pub(crate) mod live_speaker_library;

pub struct Preferences {
    theme: usize,
    pub glass: bool,
    library: TextEdit,
    locale: usize,
    asr: usize,
    diarization: usize,
    language: usize,
    device: usize,
    speakers: usize,
    stages: bool,
    pub speaker: bool,
    speaker_name: TextEdit,
    endpoint: TextEdit,
    model: TextEdit,
    secret: TextEdit,
    pub confirm_writes: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: 0,
            glass: true,
            library: TextEdit::new("~/MeetNote/Speakers"),
            locale: 0,
            asr: 0,
            diarization: 0,
            language: 0,
            device: 0,
            speakers: 0,
            stages: true,
            speaker: true,
            speaker_name: TextEdit::new("Morgan Park"),
            endpoint: TextEdit::new("https://example.test/v1"),
            model: TextEdit::new("sample-assistant"),
            secret: TextEdit::new(""),
            confirm_writes: true,
        }
    }
}

impl Preferences {
    pub(crate) fn apply_appearance(&mut self, appearance: ::settings::AppearanceSettings) {
        use ::settings::{MaterialPreference, ThemePreference};
        self.theme = match appearance.theme {
            ThemePreference::System => 0,
            ThemePreference::Light => 1,
            ThemePreference::Dark => 2,
        };
        self.glass = appearance.material == MaterialPreference::Glass;
    }

    pub fn dark(&self, system: bool) -> bool {
        match self.theme {
            1 => false,
            2 => true,
            _ => system,
        }
    }
}

const SECTIONS: [(&str, &str); 5] = [
    ("appearance", "appearance"),
    ("workspace", "workspace"),
    ("audio", "audio_processing"),
    ("speaker-library", "speaker_library"),
    ("assistant", "assistant_connection"),
];

pub fn sidebar(s: &MeetNote, k: &Kit) -> El {
    col()
        .w_full()
        .gap(5.0)
        .role("navigation")
        .label(s.tr("settings"))
        .child(eyebrow(k, s.tr("settings")))
        .children(SECTIONS.into_iter().map(|(id, name)| {
            SidebarRow::new(s.tr(name))
                .on_select(move |s: &mut MeetNote, tx| {
                    s.select = None;
                    if id == "appearance" {
                        tx.scroll_to_y("settings-scroll", 0.0);
                    } else {
                        tx.scroll_into_view(format!("settings-section-{id}"));
                    }
                })
                .build(k)
                .key(format!("category-{id}"))
        }))
}

pub fn page(s: &MeetNote, k: &Kit, overlays: &mut Vec<El>) -> El {
    let prefs = &s.settings;
    let live = s.workspace_host.preferences.draft.is_some();
    let appearance = if live {
        live_preferences::appearance(s, k, overlays)
    } else {
        section(k, "Appearance", "Customize the MeetNote window.")
            .child(field(
                k,
                "Theme",
                select(
                    s,
                    k,
                    overlays,
                    ("theme", "Theme"),
                    &["System", "Light", "Dark"],
                    prefs.theme,
                    |s, choice| s.settings.theme = choice,
                ),
            ))
            .child(field(
                k,
                "Glass material",
                Switch::new(prefs.glass)
                    .label("Glass material")
                    .hide_label()
                    .on_toggle(|s: &mut MeetNote| s.settings.glass = !s.settings.glass)
                    .build(k)
                    .key("glass-material"),
            ))
            .child(field(
                k,
                "Preferred language",
                select(
                    s,
                    k,
                    overlays,
                    ("locale", "Preferred language"),
                    &["English", "简体中文"],
                    prefs.locale,
                    |s, choice| s.settings.locale = choice,
                ),
            ))
            .child(caption(k, "Sample content is in English."))
    };
    let workspace = section(
        k,
        "Workspace",
        "Open the folder that holds your MeetNote documents.",
    )
    .child(caption(
        k,
        s.workspace_host.live.as_ref().map_or_else(
            || "Sample workspace".into(),
            |live| live.root().display().to_string(),
        ),
    ))
    .child(
        Button::new("Open workspace folder")
            .disabled(s.workspace_host.busy())
            .on_press_tx(MeetNote::choose_workspace)
            .build(k)
            .key("documents-folder"),
    )
    .children((!live).then(|| {
        TextField::new("Speaker Library", &prefs.library)
            .on_edit(|s: &mut MeetNote| &mut s.settings.library)
            .build(k)
            .key("speaker-library")
    }));
    let processing = if live {
        live_preferences::audio(s, k, overlays)
    } else {
        section(
            k,
            "Audio processing",
            "Sample backend, model and runtime preferences.",
        )
        .child(field(
            k,
            "Transcription",
            select(
                s,
                k,
                overlays,
                ("asr", "Transcription backend and model"),
                &["Whisper · Base", "Sherpa · Qwen3"],
                prefs.asr,
                |s, choice| s.settings.asr = choice,
            ),
        ))
        .child(field(
            k,
            "Speaker identification",
            select(
                s,
                k,
                overlays,
                ("diarization", "Speaker backend and model"),
                &["Native · Pyannote", "Sherpa · 3D-Speaker"],
                prefs.diarization,
                |s, choice| s.settings.diarization = choice,
            ),
        ))
        .child(field(
            k,
            "Audio language",
            select(
                s,
                k,
                overlays,
                ("language", "Audio language"),
                &["Auto-detect", "English", "Chinese"],
                prefs.language,
                |s, choice| s.settings.language = choice,
            ),
        ))
        .child(field(
            k,
            "Compute device",
            select(
                s,
                k,
                overlays,
                ("device", "Compute device"),
                &["Automatic", "CPU", "GPU"],
                prefs.device,
                |s, choice| s.settings.device = choice,
            ),
        ))
        .child(field(
            k,
            "Expected speakers",
            select(
                s,
                k,
                overlays,
                ("speaker-count", "Expected speakers"),
                &["Automatic", "1", "2", "3", "4"],
                prefs.speakers,
                |s, choice| s.settings.speakers = choice,
            ),
        ))
        .child(field(
            k,
            "Run diarization stage",
            Switch::new(prefs.stages)
                .label("Run diarization stage")
                .hide_label()
                .on_toggle(|s: &mut MeetNote| s.settings.stages = !s.settings.stages)
                .build(k)
                .key("diarization-stage"),
        ))
    };
    let library = if live {
        live_speaker_library::page(s, k)
    } else {
        section(k, "Speaker Library", "Manage the sample speaker profile.").child(
            if prefs.speaker {
                row()
                    .gap(10.0)
                    .items_center()
                    .child(
                        TextInput::new(&prefs.speaker_name)
                            .label("Speaker display name")
                            .on_edit(|s: &mut MeetNote| &mut s.settings.speaker_name)
                            .build(k)
                            .key("speaker-name")
                            .grow(1.0)
                            .min_w(0.0),
                    )
                    .child(action(
                        k,
                        "remove-speaker",
                        "Remove",
                        ButtonVariant::Danger,
                        |s| s.confirm = Some(Confirm::RemoveSpeaker),
                    ))
            } else {
                caption(k, "The sample Speaker Library is empty.")
            },
        )
    };
    let connection = if live {
        live_connection::page(s, k, overlays)
    } else {
        section(
            k,
            "Assistant connection",
            "Connection fields and masked input use in-memory values.",
        )
        .child(
            TextField::new("Endpoint", &prefs.endpoint)
                .on_edit(|s: &mut MeetNote| &mut s.settings.endpoint)
                .build(k)
                .key("endpoint"),
        )
        .child(
            TextField::new("Model", &prefs.model)
                .on_edit(|s: &mut MeetNote| &mut s.settings.model)
                .build(k)
                .key("assistant-model"),
        )
        .child(label(k, "API key"))
        .child(
            TextInput::new(&prefs.secret)
                .label("API key")
                .secure()
                .placeholder("Sample credential")
                .on_edit(|s: &mut MeetNote| &mut s.settings.secret)
                .build(k)
                .key("api-key"),
        )
        .child(Divider::horizontal().build(k))
        .child(
            RadioGroup::new()
                .label("Workspace writes")
                .item(
                    RadioItem::new("Confirm each write", prefs.confirm_writes)
                        .on_select(|s: &mut MeetNote| s.settings.confirm_writes = true),
                )
                .item(
                    RadioItem::new("Apply sample writes automatically", !prefs.confirm_writes)
                        .on_select(|s: &mut MeetNote| s.settings.confirm_writes = false),
                )
                .build(k)
                .key("write-policy"),
        )
    };
    let body = col()
        .w_full()
        .padding(24.0)
        .gap(16.0)
        .child(page_header(
            k,
            s.tr("settings_workspace"),
            s.tr("settings"),
            "Appearance, workspace and processing preferences.",
        ))
        .children(
            SECTIONS
                .into_iter()
                .zip([appearance, workspace, processing, library, connection])
                .map(|((id, _), section)| section.key(format!("settings-section-{id}"))),
        )
        .children((!live).then(|| {
            row().child(action(
                k,
                "reset-settings",
                "Reset preview settings",
                ButtonVariant::Outline,
                |s| {
                    s.confirm = Some(Confirm::ResetSettings);
                },
            ))
        }));
    col()
        .key("settings-page")
        .h_full()
        .w_full()
        .min_h(0.0)
        .child(scroll(k, "settings-scroll", body))
        .children(live.then(|| live_preferences::actions(s, k)))
}

fn section(k: &Kit, heading: &str, description: &str) -> El {
    card(k)
        .child(title(k, heading, 18.0))
        .child(caption(k, description))
}

struct Model {
    id: &'static str,
    family: &'static str,
    title: &'static str,
    description: &'static str,
    metadata: &'static str,
}

const CATALOG: [Model; 3] = [
    Model {
        id: "whisper",
        family: "TRANSCRIPTION",
        title: "Whisper Base",
        description: "General-purpose multilingual speech recognition.",
        metadata: "Whisper.cpp   ·   MIT   ·   142 MB",
    },
    Model {
        id: "qwen",
        family: "TRANSCRIPTION",
        title: "Qwen3-ASR 0.6B",
        description: "Multilingual speech recognition with automatic language detection.",
        metadata: "Sherpa ONNX   ·   Apache 2.0   ·   Sample bundle",
    },
    Model {
        id: "pyannote",
        family: "SPEAKER IDENTIFICATION",
        title: "Pyannote speaker pipeline",
        description: "Segmentation and speaker clustering for recordings.",
        metadata: "Native diarization   ·   Sample bundle",
    },
];

#[derive(Clone, Copy)]
enum Download {
    Missing,
    Active { progress: f64, started: f64 },
    Paused(f64),
    Installed,
}

pub struct Models {
    downloads: [Download; 3],
    expanded: [bool; 3],
}

impl Default for Models {
    fn default() -> Self {
        Self {
            downloads: [Download::Installed, Download::Missing, Download::Installed],
            expanded: [false; 3],
        }
    }
}

impl Models {
    pub fn poll(&mut self, now: f64, tx: &mut Tx) -> bool {
        let mut changed = false;
        for download in &mut self.downloads {
            if let Download::Active { progress, started } = *download {
                changed = true;
                if progress + (now - started) / 5.0 >= 1.0 {
                    *download = Download::Installed;
                } else {
                    tx.wake_after(Duration::from_millis(120));
                }
            }
        }
        changed
    }
}

pub fn models_page(s: &MeetNote, k: &Kit) -> El {
    if s.workspace_host.live.is_some() {
        return live_models::page(s, k);
    }
    let mut body = col().w_full().gap(16.0).padding(24.0).child(page_header(
        k,
        "MODELS  /  AUDIO",
        "Models",
        "Browse the sample catalog and try the download controls.",
    ));
    for (index, model) in CATALOG.iter().enumerate() {
        body = body.child(model_card(s, k, model, index));
    }
    col()
        .key("models-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(scroll(k, "models-scroll", body))
}

fn model_card(s: &MeetNote, k: &Kit, model: &Model, index: usize) -> El {
    let download = s.models.downloads[index];
    let (status, color) = match download {
        Download::Missing => ("Available", k.c.text_muted),
        Download::Active { .. } => ("Downloading", k.c.info.icon),
        Download::Paused(_) => ("Paused", k.c.warning.icon),
        Download::Installed => ("Installed", k.c.success.icon),
    };
    let status = row()
        .items_center()
        .gap(6.0)
        .child(dot(color))
        .child(caption(k, status));
    let progress = match download {
        Download::Active { progress, started } => Some(progress + (s.now - started) / 5.0),
        Download::Paused(progress) => Some(progress),
        _ => None,
    };
    let key = |suffix| format!("model-{}-{suffix}", model.id);
    let control = match download {
        Download::Installed => Button::new("Run sample probe")
            .variant(ButtonVariant::Outline)
            .on_press_tx(|s: &mut MeetNote, tx| s.notify("Sample probe passed", tx))
            .build(k)
            .key(key("probe")),
        Download::Missing | Download::Paused(_) => {
            let (name, progress) = match download {
                Download::Paused(progress) => ("Resume", progress),
                _ => ("Download sample", 0.0),
            };
            Button::new(name)
                .variant(ButtonVariant::Submit)
                .on_press_tx(move |s: &mut MeetNote, tx| {
                    s.models.downloads[index] = Download::Active {
                        progress,
                        started: s.now,
                    };
                    tx.wake_after(Duration::from_millis(120));
                })
                .build(k)
                .key(key("start"))
        }
        Download::Active { .. } => action(
            k,
            &key("pause"),
            "Pause",
            ButtonVariant::Outline,
            move |s| {
                if let Download::Active { progress, started } = s.models.downloads[index] {
                    s.models.downloads[index] =
                        Download::Paused((progress + (s.now - started) / 5.0).clamp(0.0, 1.0));
                }
            },
        ),
    };
    let is_selected = index == s.settings.asr || (index == 2 && s.settings.diarization == 0);
    let mut panel = card(k)
        .child(
            row()
                .items_center()
                .gap(9.0)
                .child(eyebrow(k, model.family))
                .child(el().grow(1.0))
                .child(status),
        )
        .child(
            row()
                .items_center()
                .gap(9.0)
                .flex_wrap()
                .child(title(k, model.title, 20.0))
                .children(is_selected.then(|| {
                    Badge::new("Selected")
                        .variant(BadgeVariant::Accent)
                        .build(k)
                })),
        )
        .child(label(k, model.description))
        .child(caption(k, model.metadata));
    if let Some(progress) = progress {
        panel = panel.child(
            Progress::new(Some(progress.clamp(0.0, 1.0) as f32))
                .label(format!("{} sample download progress", model.title))
                .show_value()
                .build(k)
                .key(key("progress")),
        );
    }
    let buttons = row().gap(8.0).child(control).children(progress.map(|_| {
        action(
            k,
            &key("cancel"),
            "Cancel",
            ButtonVariant::Ghost,
            move |s| {
                s.models.downloads[index] = Download::Missing;
            },
        )
    }));
    panel.child(buttons).child(
        Collapsible::new("Provenance and probe", s.models.expanded[index])
            .on_toggle(move |s: &mut MeetNote| s.models.expanded[index] = !s.models.expanded[index])
            .child(caption(
                k,
                format!("Catalog ID: {}   ·   Provenance: Sample catalog", model.id),
            ))
            .build(k)
            .key(key("details")),
    )
}
