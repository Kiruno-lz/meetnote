use ::settings::{MaterialPreference, ThemePreference};
use contracts::{ComputeDevice, DiarizationStageKind};
use rune::{El, row};
use rune_kit::{Button, ButtonVariant, Kit, Switch, TextField};
use ui_text::Locale;

use super::section;
use crate::MeetNote;
use crate::controls::{field, select};
use crate::workspace_host::diarization_stages::StageDraft;

const THEMES: [ThemePreference; 3] = [
    ThemePreference::System,
    ThemePreference::Light,
    ThemePreference::Dark,
];
const DEVICES: [ComputeDevice; 4] = [
    ComputeDevice::Auto,
    ComputeDevice::Cpu,
    ComputeDevice::Metal,
    ComputeDevice::CoreMl,
];
const LOCALES: [Locale; 2] = [Locale::ZhCn, Locale::EnUs];

pub(super) fn appearance(s: &MeetNote, k: &Kit, overlays: &mut Vec<El>) -> El {
    let draft = s
        .workspace_host
        .preferences
        .draft
        .as_ref()
        .expect("live preferences were loaded");
    let form = section(k, s.tr("appearance"), s.tr("appearance_description"))
        .child(field(
            k,
            s.tr("theme"),
            select(
                s,
                k,
                overlays,
                ("live-theme", s.tr("theme")),
                &[
                    s.tr("theme_system"),
                    s.tr("theme_light"),
                    s.tr("theme_dark"),
                ],
                THEMES
                    .iter()
                    .position(|theme| *theme == draft.appearance.theme)
                    .unwrap_or_default(),
                |s, index| {
                    s.workspace_host
                        .preferences
                        .draft
                        .as_mut()
                        .unwrap()
                        .appearance
                        .theme = THEMES[index]
                },
            ),
        ))
        .child(field(
            k,
            s.tr("material_glass"),
            Switch::new(draft.appearance.material == MaterialPreference::Glass)
                .label(s.tr("material_glass"))
                .hide_label()
                .on_toggle(|s: &mut MeetNote| {
                    let appearance = &mut s
                        .workspace_host
                        .preferences
                        .draft
                        .as_mut()
                        .unwrap()
                        .appearance;
                    appearance.material = match appearance.material {
                        MaterialPreference::Glass => MaterialPreference::Opaque,
                        MaterialPreference::Opaque => MaterialPreference::Glass,
                    };
                })
                .build(k)
                .key("live-glass-material"),
        ))
        .child(field(
            k,
            s.tr("language"),
            select(
                s,
                k,
                overlays,
                ("live-ui-language", s.tr("language")),
                &["简体中文", "English"],
                usize::from(Locale::from_code(&draft.locale) == Locale::EnUs),
                |s, index| {
                    s.workspace_host.preferences.draft.as_mut().unwrap().locale =
                        LOCALES[index].code().into();
                },
            ),
        ));
    if s.workspace_host.preferences.busy() {
        form.inert()
    } else {
        form
    }
}

pub(super) fn audio(s: &MeetNote, k: &Kit, overlays: &mut Vec<El>) -> El {
    let host = &s.workspace_host.preferences;
    let draft = host.draft.as_ref().expect("live preferences were loaded");
    let form = section(
        k,
        s.tr("audio_processing"),
        s.tr("audio_processing_description"),
    )
    .child(
        Button::new(s.tr("open_models"))
            .variant(ButtonVariant::Outline)
            .on_press(|s: &mut MeetNote| s.navigate(crate::Route::Models))
            .build(k),
    )
    .children(diarization_stages(s, k, overlays, &draft.stages))
    .child(
        TextField::new(s.tr("transcription_language"), &draft.language)
            .placeholder(s.tr("auto"))
            .on_edit(|s: &mut MeetNote| {
                &mut s
                    .workspace_host
                    .preferences
                    .draft
                    .as_mut()
                    .unwrap()
                    .language
            })
            .build(k),
    )
    .child(field(
        k,
        s.tr("compute_device"),
        select(
            s,
            k,
            overlays,
            ("live-device", s.tr("compute_device")),
            &[s.tr("auto"), "CPU", "Metal", "Core ML"],
            DEVICES
                .iter()
                .position(|device| *device == draft.compute_device)
                .unwrap_or_default(),
            |s, index| {
                s.workspace_host
                    .preferences
                    .draft
                    .as_mut()
                    .unwrap()
                    .compute_device = DEVICES[index]
            },
        ),
    ))
    .child(
        TextField::new(s.tr("expected_speakers"), &draft.expected_speakers)
            .placeholder(s.tr("auto"))
            .on_edit(|s: &mut MeetNote| {
                &mut s
                    .workspace_host
                    .preferences
                    .draft
                    .as_mut()
                    .unwrap()
                    .expected_speakers
            })
            .build(k),
    );
    if host.busy() { form.inert() } else { form }
}

pub(super) fn actions(s: &MeetNote, k: &Kit) -> El {
    row()
        .px(24.0)
        .py(10.0)
        .gap(8.0)
        .flex_wrap()
        .child(
            Button::new(s.tr("save_settings"))
                .disabled(s.workspace_host.busy())
                .on_press_tx(MeetNote::save_live_preferences)
                .build(k)
                .key("save-live-settings"),
        )
        .child(
            Button::new(s.tr("reload_settings"))
                .variant(ButtonVariant::Outline)
                .disabled(s.workspace_host.busy())
                .on_press_tx(MeetNote::reload_live_preferences)
                .build(k)
                .key("reload-live-settings"),
        )
}

fn diarization_stages(
    s: &MeetNote,
    k: &Kit,
    overlays: &mut Vec<El>,
    draft: &StageDraft,
) -> Vec<El> {
    [
        (
            DiarizationStageKind::Segmentation,
            "live-diarization-segmentation",
            "diarization_segmentation_model",
        ),
        (
            DiarizationStageKind::Embedding,
            "live-diarization-embedding",
            "diarization_embedding_model",
        ),
        (
            DiarizationStageKind::Clustering,
            "live-diarization-clustering",
            "diarization_clustering",
        ),
    ]
    .into_iter()
    .filter_map(|(stage, key, title_key)| {
        let title = s.tr(title_key);
        let ids: Vec<_> = draft
            .options(stage)
            .map(|option| option.id.clone())
            .collect();
        if ids.is_empty() {
            return None;
        }
        let selected = draft
            .selected_id(stage)
            .and_then(|id| ids.iter().position(|option| option == id))
            .map_or(0, |index| index + 1);
        let choices: Vec<_> = std::iter::once(s.tr("model_default"))
            .chain(ids.iter().map(String::as_str))
            .collect();
        let values = ids.clone();
        Some(field(
            k,
            title,
            select(
                s,
                k,
                overlays,
                (key, title),
                &choices,
                selected,
                move |s, index| {
                    let choice = index
                        .checked_sub(1)
                        .and_then(|index| values.get(index))
                        .map(String::as_str);
                    s.workspace_host
                        .preferences
                        .draft
                        .as_mut()
                        .unwrap()
                        .stages
                        .select(stage, choice);
                },
            ),
        ))
    })
    .collect()
}
