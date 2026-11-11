use models::manager::{DownloadProgress, DownloadState};
use models::{
    ArtifactChecksum, BackendCheck, ChecksumResult, ModelAvailability, ModelInstallState,
    ModelOperation,
};
use rune::{El, col, row};
use rune_kit::{Badge, BadgeVariant, Button, ButtonVariant, Collapsible, Kit, Progress};

use crate::MeetNote;
use crate::controls::{caption, card, label, scroll, title};
use crate::workspace_host::models::ModelAction;

pub(super) fn page(s: &MeetNote, k: &Kit) -> El {
    let host = &s.workspace_host.models;
    let mut body = col().w_full().gap(12.0).padding(24.0).child(
        row()
            .items_center()
            .gap(12.0)
            .child(title(k, s.tr("models"), 30.0).grow(1.0))
            .child(
                Button::new(s.tr("model_refresh"))
                    .variant(ButtonVariant::Outline)
                    .disabled(host.busy())
                    .on_press_tx(MeetNote::refresh_models)
                    .build(k)
                    .key("models-refresh"),
            ),
    );
    if let Some(error) = &host.error {
        body = body.child(label(k, error));
    }
    if let Some(catalog) = host.catalog() {
        for task in ["asr", "diarization"] {
            body = body.child(title(
                k,
                if task == "asr" {
                    s.tr("transcription")
                } else {
                    s.tr("speaker_separation")
                },
                18.0,
            ));
            for entry in catalog
                .entries
                .iter()
                .filter(|entry| entry.model.task == task)
            {
                body = body.child(model(s, k, entry, catalog.selected(entry)));
            }
        }
    } else if host.error.is_none() {
        body = body.child(caption(k, s.tr("model_catalog_loading")));
    }
    col()
        .key("models-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(scroll(k, "models-scroll", body))
}

fn model(s: &MeetNote, k: &Kit, entry: &ModelAvailability, selected: Option<&str>) -> El {
    let host = &s.workspace_host.models;
    let entry_model = &entry.model;
    let id = &entry_model.id;
    let key = |part| format!("model-{id}-{part}");
    let download = host.progress(id);
    let active = download.filter(|task| !task.state.is_terminal());
    let status = match (active.map(|task| &task.state), entry.install_state) {
        (Some(DownloadState::Queued), _) => s.tr("model_queued"),
        (Some(DownloadState::Downloading), _) => s.tr("downloading"),
        (Some(DownloadState::Paused), _) => s.tr("model_paused"),
        (Some(DownloadState::Verifying), _) => s.tr("verifying"),
        (_, ModelInstallState::Installed) => s.tr("model_install_complete"),
        (_, ModelInstallState::Partial) => s.tr("model_install_partial"),
        _ if entry_model.requires_auth => s.tr("model_requires_auth"),
        _ if entry_model.status == "ready" => s.tr("model_download_available"),
        _ => s.tr("model_catalog_candidate"),
    };
    let mut details = col().gap(8.0).w_full().child(caption(
        k,
        format!(
            "{} · {} · {}",
            entry_model.model_family, entry_model.source_format, entry_model.license
        ),
    ));
    if let Some(revision) = &entry_model.revision {
        details = details.child(caption(k, format!("Revision {revision}")));
    }
    details = details.child(
        caption(
            k,
            format!("{} · {}", s.tr("model_source"), entry_model.source),
        )
        .selectable(true),
    );
    if entry_model.artifacts.is_empty() {
        if let Some(checksum) = &entry_model.sha256 {
            details = details.child(caption(k, format!("SHA-256 · {checksum}")).selectable(true));
        }
    } else {
        details = details.children(entry_model.artifacts.iter().filter_map(|artifact| {
            artifact.sha256.as_ref().map(|checksum| {
                caption(k, format!("{} · SHA-256 · {checksum}", artifact.filename)).selectable(true)
            })
        }));
    }
    if let Some(error) = &entry.record_error {
        details = details.child(label(k, error.to_string()));
    }
    if let Some(probe) = &entry.probe {
        let kind = match probe.kind {
            ModelOperation::Download => s.tr("probe_kind_download"),
            ModelOperation::Reprobe => s.tr("probe_kind_reprobe"),
        };
        details = details.child(
            caption(
                k,
                if probe.passed() {
                    s.tr("model_probe_passed")
                } else {
                    s.tr("model_probe_failed")
                },
            )
            .key(if probe.passed() {
                key("probe-passed")
            } else {
                key("probe-failed")
            }),
        );
        details = details
            .child(caption(k, kind))
            .children(probe.checksums.iter().enumerate().map(|(index, checksum)| {
                checksum_row(s, k, checksum).key(format!("model-{id}-checksum-{index}"))
            }));
        for result in &probe.backends {
            let outcome = match &result.result {
                BackendCheck::Passed => s.tr("model_backend_passed"),
                BackendCheck::Failed { message } => message,
                BackendCheck::NotRegistered => s.tr("model_backend_unavailable"),
                BackendCheck::SkippedChecksumFailure => s.tr("model_backend_checksum_skipped"),
            };
            details = details.child(caption(k, format!("{} · {outcome}", result.backend_id)));
        }
    }
    if let Some(failure) = &entry.failure {
        details = details.child(label(k, failure.reason.to_string()));
    }
    let disabled = host.busy();
    let mut controls = row().flex_wrap().gap(8.0).items_center();
    if let Some(task) = active {
        let fraction = task
            .total_bytes
            .filter(|&total| total > 0)
            .map(|total| (task.completed_bytes as f32 / total as f32).clamp(0.0, 1.0));
        details = details
            .child(
                Progress::new(fraction)
                    .label(status)
                    .show_value()
                    .build(k)
                    .key(key("progress")),
            )
            .child(caption(k, download_label(task)));
        controls = match task.state {
            DownloadState::Downloading | DownloadState::Queued => controls.child(button(
                k,
                key("pause"),
                s.tr("pause"),
                disabled,
                ModelAction::Pause(task.id),
            )),
            DownloadState::Paused => controls.child(button(
                k,
                key("resume"),
                s.tr("resume"),
                disabled,
                ModelAction::Resume(task.id),
            )),
            _ => controls,
        };
        controls = controls.child(button(
            k,
            key("cancel"),
            s.tr("cancel"),
            disabled,
            ModelAction::Cancel(task.id),
        ));
    } else if entry.install_state == ModelInstallState::Installed {
        controls = controls.child(button(
            k,
            key("probe"),
            s.tr("reprobe"),
            disabled,
            ModelAction::Probe(id.clone()),
        ));
        for backend in &entry_model.candidate_backends {
            controls = controls.child(button(
                k,
                format!("model-{id}-select-{backend}"),
                format!("Use with {backend}"),
                disabled || selected == Some(backend.as_str()),
                ModelAction::Select {
                    id: id.clone(),
                    backend: backend.clone(),
                },
            ));
        }
    } else if entry_model.status == "ready" && !entry_model.requires_auth {
        if let Some(task) = download
            && let DownloadState::Failed(message) = &task.state
        {
            details = details.child(label(k, message));
        }
        controls = controls.child(button(
            k,
            key("download"),
            s.tr("download"),
            disabled,
            ModelAction::Download(id.clone()),
        ));
    }
    let model_id = id.clone();
    card(k)
        .padding(12.0)
        .gap(4.0)
        .child(
            row()
                .gap(8.0)
                .items_center()
                .flex_wrap()
                .child(title(k, id, 16.0).grow(1.0))
                .child(Badge::new(status).build(k))
                .children(selected.map(|backend| {
                    Badge::new(format!("Selected · {backend}"))
                        .variant(BadgeVariant::Accent)
                        .build(k)
                        .key(key("selected"))
                })),
        )
        .child(
            Collapsible::new(s.tr("model_details_actions"), host.expanded.contains(id))
                .on_toggle(move |s: &mut MeetNote| {
                    let expanded = &mut s.workspace_host.models.expanded;
                    if !expanded.remove(&model_id) {
                        expanded.insert(model_id.clone());
                    }
                })
                .child(details.child(controls))
                .build(k)
                .key(key("details")),
        )
}

fn checksum_row(s: &MeetNote, k: &Kit, artifact: &ArtifactChecksum) -> El {
    let (status, detail) = match &artifact.result {
        ChecksumResult::Matched => (s.tr("checksum_matched"), None),
        ChecksumResult::ArchiveMatched => (s.tr("checksum_archive_matched"), None),
        ChecksumResult::ArchiveNotRechecked => (s.tr("checksum_archive_not_rechecked"), None),
        ChecksumResult::NotDeclared => (s.tr("checksum_not_declared"), None),
        ChecksumResult::Mismatched { actual_sha256 } => (
            s.tr("model_checksum_mismatch"),
            Some(actual_sha256.as_str()),
        ),
        ChecksumResult::Unreadable { message } => {
            (s.tr("model_checksum_unreadable"), Some(message.as_str()))
        }
    };
    col()
        .gap(3.0)
        .child(caption(k, format!("{} · {status}", artifact.filename)))
        .children(detail.map(|detail| caption(k, detail).selectable(true)))
}

fn button(
    k: &Kit,
    key: String,
    title: impl Into<String>,
    disabled: bool,
    action: ModelAction,
) -> El {
    Button::new(title)
        .variant(ButtonVariant::Outline)
        .disabled(disabled)
        .on_press_tx(move |s: &mut MeetNote, tx| s.change_model(action.clone(), tx))
        .build(k)
        .key(key)
}

fn download_label(task: &DownloadProgress) -> String {
    let mib = 1024.0 * 1024.0;
    let amount = task.completed_bytes as f64 / mib;
    let mut text = task.total_bytes.map_or_else(
        || format!("{amount:.1} MiB"),
        |total| format!("{amount:.1} / {:.1} MiB", total as f64 / mib),
    );
    if let Some(speed) = task.speed_bytes_per_second {
        text.push_str(&format!(" · {:.1} MiB/s", speed as f64 / mib));
    }
    text
}
