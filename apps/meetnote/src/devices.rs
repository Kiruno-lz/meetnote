//! The Devices page: paired recording devices as cards, and the window that
//! finds and pairs a new one. Reads `DeviceHost`; never touches files or radios.

use application::device_import::DeviceImportProgress;
use devices::ble::{Verification, signal_bars};
use devices::{AttachedDevice, BleCandidate, PairedDevice};
use rune::{Beat, Cycle, Easing, El, Icon, Sequence, TextStyle, col, el, icon, row, text};
use rune_kit::{
    Badge, BadgeVariant, Button, ButtonVariant, ControlSize, Dialog, Kit, Progress, Spinner, Switch,
};

use crate::controls::{caption, eyebrow, scroll, title};
use crate::workspace_host::devices::{PairStep, Radio, RegistryChange};
use crate::{Confirm, MeetNote};

const CARD_WIDTH: f32 = 760.0;

/// A clip-on wireless transmitter drawn from primitives on a 96-unit grid:
/// graphite body, perforated grille, status screen and button.
pub fn device_art(size: f32) -> El {
    let u = size / 96.0;
    let color = |value: &str| rune::color::hex(value).unwrap_or(rune::Color::BLACK);
    let part = |x: f32, y: f32, w: f32, h: f32, r: f32| {
        el().absolute(x * u, y * u).size(w * u, h * u).radius(r * u)
    };
    let mut grille = part(28.0, 21.0, 40.0, 33.0, 11.0).linear_gradient(
        std::f32::consts::FRAC_PI_4 * 3.0,
        [(0.0, color("#30343c")), (1.0, color("#121418"))],
    );
    for row_index in 0..4 {
        for column in 0..5 {
            grille = grille.child(
                el().absolute(
                    (6.6 + column as f32 * 6.0) * u,
                    (6.6 + row_index as f32 * 6.0) * u,
                )
                .size(2.8 * u, 2.8 * u)
                .radius(1.4 * u)
                .fill(color("#9aa1ad").alpha(0.5)),
            );
        }
    }
    el().key("device-art")
        .size(size, size)
        .a11y_hidden()
        .child(part(38.0, 8.0, 20.0, 9.0, 4.0).fill(color("#23262c")))
        .child(
            part(20.0, 13.0, 56.0, 72.0, 17.0)
                .fill(color("#2c3037"))
                .linear_gradient(
                    std::f32::consts::PI,
                    [
                        (0.0, color("#5d636e")),
                        (0.55, color("#2c3037")),
                        (1.0, color("#1a1c21")),
                    ],
                )
                .border(u.max(0.6), rune::Color::WHITE.alpha(0.14))
                .shadow(2.0 * u, 6.0 * u, rune::Color::BLACK.alpha(0.28)),
        )
        .child(grille)
        .child(part(32.0, 61.0, 26.0, 15.0, 4.0).fill(color("#0c0e11")))
        .child(part(35.0, 64.5, 10.0, 2.4, 1.2).fill(color("#7fe0b0")))
        .child(part(35.0, 69.5, 16.0, 2.4, 1.2).fill(color("#4b5260")))
        .child(
            part(62.0, 65.5, 6.0, 6.0, 3.0)
                .fill(color("#3b4049"))
                .border(u.max(0.5), rune::Color::WHITE.alpha(0.12)),
        )
}

/// The rail glyph: the transmitter in outline.
pub static DEVICE_GLYPH: Icon = Icon {
    name: "meetnote-devices",
    view: 16.0,
    body: r#"<rect x="3.5" y="2.5" width="9" height="11.5" rx="3" fill="none" stroke="currentColor" stroke-width="1.3"/><rect x="5.5" y="4.6" width="5" height="3.6" rx="1.2" fill="none" stroke="currentColor" stroke-width="1.1"/><path d="M6 11.2h2.4" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/><path d="M6.6 1.4h2.8" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>"#,
};

static CHECK: Icon = Icon {
    name: "meetnote-pair-check",
    view: 24.0,
    body: r#"<path d="M6.5 12.5l3.6 3.6 7.4-8.2" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/>"#,
};

const LED: Cycle = Cycle {
    period: 1.6,
    phase: 0.0,
    stops: &[(0.0, 0.35), (0.5, 1.0), (1.0, 0.35)],
    still: 1.0,
    fps: 20.0,
};

static ENTER: [Beat; 3] = [
    Beat::new(0.0).opacity(0.0).scale(0.96),
    Beat::new(0.22)
        .opacity(1.0)
        .scale(1.0)
        .easing(Easing::EASE_OUT),
    Beat::new(0.3),
];

static POP: [Beat; 4] = [
    Beat::new(0.0).opacity(0.0).scale(0.5),
    Beat::new(0.22)
        .opacity(1.0)
        .scale(1.12)
        .easing(Easing::EASE_OUT),
    Beat::new(0.36).scale(0.98),
    Beat::new(0.48),
];

pub fn page(s: &MeetNote, k: &Kit, overlays: &mut Vec<El>) -> El {
    let host = &s.workspace_host.devices;
    let header = col()
        .w_full()
        .gap(7.0)
        .child(eyebrow(k, s.tr("devices_eyebrow")))
        .child(title(k, s.tr("devices"), 30.0))
        .child(caption(k, s.tr("devices_description")));
    let mut body = col()
        .key("devices-body")
        .w_full()
        .max_w(CARD_WIDTH)
        .self_center()
        .gap(16.0)
        .padding(24.0)
        .child(header);
    if let Some(device) = host.unpaired().next() {
        body = body.child(unpaired_banner(s, k, device));
    }
    if let Some(error) = &host.error {
        body = body.child(caption(k, error.clone()).color(k.c.danger.fg));
    }
    let page = if host.registry.devices.is_empty() {
        col()
            .key("devices-page")
            .w_full()
            .h_full()
            .min_h(0.0)
            .child(body)
            .child(empty_state(s, k))
    } else {
        for device in &host.registry.devices {
            body = body.child(device_card(s, k, device));
        }
        body = body
            .child(row().w_full().justify_center().pt(4.0).child(add_button(
                s,
                k,
                ButtonVariant::Outline,
            )))
            // Room below the last action, so it never sits on the window edge.
            .child(el().h(160.0));
        col()
            .key("devices-page")
            .w_full()
            .h_full()
            .min_h(0.0)
            .child(scroll(k, "devices-scroll", body))
    };
    if host.pairing.is_some() {
        pairing_dialog(s, k, overlays);
    }
    page
}

fn add_button(s: &MeetNote, k: &Kit, variant: ButtonVariant) -> El {
    Button::new(s.tr("device_add"))
        .control_size(ControlSize::Large)
        .variant(variant)
        .on_press_tx(|s: &mut MeetNote, tx| {
            s.workspace_host.devices.open_pairing(s.now);
            tx.wake_after(std::time::Duration::ZERO);
        })
        .build(k)
        .key("device-add")
}

fn empty_state(s: &MeetNote, k: &Kit) -> El {
    col()
        .key("devices-empty")
        .grow(1.0)
        .w_full()
        .items_center()
        .justify_center()
        .gap(14.0)
        .pb(72.0)
        .child(
            el().size(120.0, 120.0)
                .radius(60.0)
                .radial_gradient(
                    [60.0, 60.0],
                    60.0,
                    [
                        (0.0, k.c.text_accent.alpha(0.16)),
                        (1.0, k.c.text_accent.alpha(0.0)),
                    ],
                )
                .items_center()
                .justify_center()
                .child(device_art(84.0)),
        )
        .child(title(k, s.tr("devices_empty_title"), 18.0))
        .child(
            caption(k, s.tr("devices_empty_hint"))
                .max_w(380.0)
                .align(rune::Align::Center),
        )
        .child(el().h(4.0))
        .child(add_button(s, k, ButtonVariant::Submit))
}

fn unpaired_banner(s: &MeetNote, k: &Kit, device: &AttachedDevice) -> El {
    row()
        .key("device-unpaired")
        .w_full()
        .items_center()
        .gap(12.0)
        .padding(12.0)
        .radius(10.0)
        .fill(k.c.info.bg)
        .border(1.0, k.c.info.border)
        .child(icon(&DEVICE_GLYPH, 18.0).color(k.c.info.icon))
        .child(
            text(
                s.tr("device_unpaired_connected")
                    .replace("{name}", &display_name(&device.identity.name, s)),
            )
            .font(TextStyle::new(13.0, 500.0))
            .color(k.c.text_base)
            .wrap()
            .grow(1.0)
            .min_w(0.0),
        )
        .child(
            Button::new(s.tr("device_pair"))
                .variant(ButtonVariant::Submit)
                .on_press_tx(|s: &mut MeetNote, tx| {
                    s.workspace_host.devices.open_pairing(s.now);
                    tx.wake_after(std::time::Duration::ZERO);
                })
                .build(k)
                .key("device-unpaired-pair"),
        )
}

fn display_name(name: &str, s: &MeetNote) -> String {
    if name.is_empty() {
        s.tr("device_unknown").to_owned()
    } else {
        name.to_owned()
    }
}

/// Art on a soft tile; a breathing light while the device is connected.
fn art_tile(k: &Kit, size: f32, connected: bool) -> El {
    let led = connected.then(|| {
        el().key("device-led")
            .absolute(size - 16.0, 10.0)
            .size(8.0, 8.0)
            .radius(4.0)
            .fill(k.c.success.icon)
            .shadow(0.0, 6.0, k.c.success.icon.alpha(0.6))
            .cycle(LED)
    });
    el().size(size, size)
        .radius(18.0)
        .linear_gradient(
            std::f32::consts::PI,
            [(0.0, k.c.layer_02), (1.0, k.c.layer_03)],
        )
        .border(1.0, k.c.border_muted)
        .items_center()
        .justify_center()
        .child(device_art(size * 0.82))
        .children(led)
}

fn fact(k: &Kit, name: &str, value: Option<&str>, key: String) -> El {
    col()
        .gap(3.0)
        .min_w(0.0)
        .child(
            text(name.to_owned())
                .font(TextStyle::new(11.0, 600.0))
                .color(k.c.text_faint),
        )
        .child(
            text(value.unwrap_or("—").to_owned())
                .key(key)
                .font(TextStyle::new(13.0, 500.0).features("'tnum' 1"))
                .color(if value.is_some() {
                    k.c.text_base
                } else {
                    k.c.text_faint
                })
                .selectable(true)
                .truncate(),
        )
}

fn device_card(s: &MeetNote, k: &Kit, device: &PairedDevice) -> El {
    let host = &s.workspace_host.devices;
    let key = device.key.clone();
    let id = card_id(&key);
    let attached = host.attached(&key);
    let importing = host.import.as_ref().filter(|run| run.key == key);
    let (status, color) = if let Some(run) = importing {
        let label = match &run.progress {
            Some(DeviceImportProgress::Copying { index, total, .. }) => s
                .tr("device_status_importing")
                .replace("{index}", &(index + 1).to_string())
                .replace("{total}", &total.to_string()),
            _ => s.tr("device_status_scanning").to_owned(),
        };
        (label, k.c.info.icon)
    } else if attached.is_some_and(|device| device.volumes.is_empty()) {
        (s.tr("device_status_mounting").to_owned(), k.c.warning.icon)
    } else if attached.is_some() {
        (s.tr("device_status_synced").to_owned(), k.c.success.icon)
    } else {
        (s.tr("device_status_offline").to_owned(), k.c.text_faint)
    };
    let last_import = device.last_import.map(|summary| {
        let when = relative_time(s, summary.finished_at_ms);
        s.tr("device_last_import")
            .replace("{when}", &when)
            .replace("{count}", &summary.imported.to_string())
    });
    let name = display_name(&device.identity.name, s);
    let heading = row()
        .w_full()
        .items_center()
        .gap(8.0)
        .child(
            text(name)
                .font(TextStyle::new(17.0, 650.0))
                .color(k.c.text_base)
                .truncate()
                .min_w(0.0),
        )
        .children(device.identity.model_code.as_ref().map(|code| {
            Badge::new(code.clone())
                .variant(BadgeVariant::Subtle)
                .build(k)
        }))
        .child(el().grow(1.0))
        .child(
            Button::new(s.tr("device_remove"))
                .variant(ButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .on_press({
                    let key = key.clone();
                    move |s: &mut MeetNote| s.confirm = Some(Confirm::RemoveDevice(key.clone()))
                })
                .build(k)
                .key(format!("{id}-remove")),
        );
    let status_row = row()
        .items_center()
        .gap(6.0)
        .child(crate::controls::dot(color))
        .child(
            text(status)
                .key(format!("{id}-status"))
                .font(TextStyle::new(12.5, 500.0))
                .color(k.c.text_muted),
        )
        .children(last_import.map(|line| {
            text(format!("·  {line}"))
                .font(TextStyle::new(12.5, 400.0))
                .color(k.c.text_faint)
        }));
    let facts = row()
        .w_full()
        .gap(16.0)
        .equal_columns(3)
        .child(fact(
            k,
            "SN",
            device.identity.serial.as_deref(),
            format!("{id}-serial"),
        ))
        .child(fact(
            k,
            "MAC",
            device.identity.mac.as_deref(),
            format!("{id}-mac"),
        ))
        .child(fact(
            k,
            s.tr("device_firmware"),
            device.identity.firmware.as_deref(),
            format!("{id}-firmware"),
        ));
    let summary = row()
        .w_full()
        .gap(18.0)
        .items_center()
        .child(art_tile(k, 92.0, attached.is_some()))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(10.0)
                .child(heading)
                .child(status_row)
                .child(facts),
        );
    let progress = importing.map(|run| {
        let label = match &run.progress {
            Some(DeviceImportProgress::Copying { name, .. }) => name.clone(),
            _ => s.tr("device_status_scanning").to_owned(),
        };
        let fraction = host.import_fraction();
        // File name, then speed beside the percentage.
        let small = |line: String, key: &str, color| {
            text(line)
                .key(format!("{id}-progress-{key}"))
                .font(TextStyle::new(12.5, 400.0).features("'tnum' 1"))
                .color(color)
        };
        let header = row()
            .w_full()
            .gap(12.0)
            .items_center()
            .child(
                small(label.clone(), "name", k.c.text_base)
                    .truncate()
                    .grow(1.0)
                    .min_w(0.0),
            )
            .children(
                run.speed()
                    .map(|speed| small(devices::rate::format_rate(speed), "speed", k.c.text_muted)),
            )
            .children(fraction.map(|fraction| {
                small(
                    format!("{}%", (fraction * 100.0).round()),
                    "value",
                    k.c.text_muted,
                )
            }));
        col()
            .key(format!("{id}-progress"))
            .label(label)
            .w_full()
            .gap(6.0)
            .child(header.a11y_hidden())
            .child(Progress::new(fraction).build(k))
    });
    let directory = import_dir_row(s, k, device, &id);
    let auto = row()
        .w_full()
        .items_center()
        .gap(12.0)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(s.tr("device_auto_process"))
                        .font(TextStyle::new(13.0, 500.0))
                        .color(k.c.text_base),
                )
                .child(caption(k, s.tr("device_auto_process_hint"))),
        )
        .child(
            Switch::new(device.auto_process)
                .label(s.tr("device_auto_process"))
                .hide_label()
                .on_toggle_tx({
                    let key = key.clone();
                    let enabled = !device.auto_process;
                    move |s: &mut MeetNote, tx| {
                        s.change_device(RegistryChange::AutoProcess(key.clone(), enabled), tx)
                    }
                })
                .build(k)
                .key(format!("{id}-auto")),
        );
    let advanced_open = host.advanced.contains(&key);
    let delete_sources = row()
        .key(format!("{id}-advanced-body"))
        .w_full()
        .items_center()
        .gap(12.0)
        .padding(12.0)
        .radius(10.0)
        .fill(k.c.layer_02)
        .sequence(Sequence::new(&ENTER))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(s.tr("device_delete_after_import"))
                        .font(TextStyle::new(13.0, 500.0))
                        .color(k.c.text_base),
                )
                .child(caption(k, s.tr("device_delete_after_import_hint"))),
        )
        .child(
            Switch::new(device.delete_after_import)
                .label(s.tr("device_delete_after_import"))
                .hide_label()
                .on_toggle_tx({
                    let key = key.clone();
                    let enabled = !device.delete_after_import;
                    move |s: &mut MeetNote, tx| {
                        s.change_device(RegistryChange::DeleteAfterImport(key.clone(), enabled), tx)
                    }
                })
                .build(k)
                .key(format!("{id}-delete-sources")),
        );
    let advanced = col()
        .w_full()
        .gap(8.0)
        .child(
            row()
                .key(format!("{id}-advanced"))
                .role("button")
                .label(s.tr("device_advanced"))
                .expanded(advanced_open)
                .focusable(true)
                .focus_ring(2.0, 0.0, k.c.border_focus)
                .self_start()
                .items_center()
                .gap(6.0)
                .py(4.0)
                .cursor(rune::Cursor::Pointer)
                .on_press({
                    let key = key.clone();
                    move |s: &mut MeetNote| {
                        let open = &mut s.workspace_host.devices.advanced;
                        if !open.remove(&key) {
                            open.insert(key.clone());
                        }
                    }
                })
                .child(
                    text(s.tr("device_advanced"))
                        .font(TextStyle::new(13.0, 600.0))
                        .color(k.c.text_muted),
                )
                .child(
                    icon(&rune_kit::glyphs::CHEVRON_DOWN, 14.0)
                        .color(k.c.icon_muted)
                        .rotate(if advanced_open {
                            0.0
                        } else {
                            -std::f32::consts::FRAC_PI_2
                        }),
                ),
        )
        .children(advanced_open.then_some(delete_sources));
    col()
        .key(id.clone())
        .w_full()
        .min_w(0.0)
        .gap(16.0)
        .padding(20.0)
        .radius(14.0)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
        .shadow(1.0, 3.0, rune::Color::BLACK.alpha(0.04))
        .child(summary)
        .children(progress)
        .child(rune_kit::Divider::horizontal().build(k))
        .child(directory)
        .child(auto)
        .child(advanced)
}

fn import_dir_row(s: &MeetNote, k: &Kit, device: &PairedDevice, id: &str) -> El {
    let key = device.key.clone();
    let (path, custom) = match &device.import_dir {
        Some(path) => (path.display().to_string(), true),
        None => (s.tr("device_import_dir_default").to_owned(), false),
    };
    row()
        .w_full()
        .items_center()
        .gap(12.0)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(s.tr("device_import_dir"))
                        .font(TextStyle::new(13.0, 500.0))
                        .color(k.c.text_base),
                )
                .child(
                    text(path)
                        .key(format!("{id}-import-dir"))
                        .font(TextStyle::new(12.0, 400.0))
                        .color(k.c.text_muted)
                        .truncate_middle(),
                ),
        )
        .children(custom.then(|| {
            Button::new(s.tr("device_import_dir_reset"))
                .variant(ButtonVariant::GhostMuted)
                .on_press_tx({
                    let key = key.clone();
                    move |s: &mut MeetNote, tx| {
                        s.change_device(RegistryChange::ImportDir(key.clone(), None), tx)
                    }
                })
                .build(k)
                .key(format!("{id}-import-dir-reset"))
        }))
        .child(
            Button::new(s.tr("device_import_dir_change"))
                .variant(ButtonVariant::Outline)
                .on_press_tx(move |s: &mut MeetNote, tx| s.choose_import_dir(key.clone(), tx))
                .build(k)
                .key(format!("{id}-import-dir-change")),
        )
}

/// Element keys stay ASCII and stable: the registry key without separators.
pub fn card_id(key: &str) -> String {
    let clean: String = key
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("device-{clean}")
}

fn pairing_dialog(s: &MeetNote, k: &Kit, overlays: &mut Vec<El>) {
    let Some(pairing) = &s.workspace_host.devices.pairing else {
        return;
    };
    let (heading, body, step) = match pairing.step {
        PairStep::Connect => (s.tr("device_pair_title"), connect_step(s, k), 0),
        PairStep::Done => (s.tr("device_pair_done_title"), done_step(s, k), 1),
    };
    let dots = row()
        .gap(6.0)
        .justify_center()
        .children((0..2).map(|index| {
            el().size(if index == step { 18.0 } else { 6.0 }, 6.0)
                .radius(3.0)
                .fill(if index <= step {
                    k.c.text_accent
                } else {
                    k.c.border_base
                })
        }));
    Dialog::new("device-pairing", heading)
        .open(true)
        .fit()
        .width(480.0)
        .on_close(|s: &mut MeetNote| s.workspace_host.devices.close_pairing())
        .child(
            col()
                .key(format!("device-pair-step-{step}"))
                .w_full()
                .gap(16.0)
                .px(20.0)
                .pb(20.0)
                .child(dots)
                .child(
                    col()
                        .w_full()
                        .gap(16.0)
                        .sequence(Sequence::new(&ENTER))
                        .child(body),
                ),
        )
        .build(k, overlays);
}

/// Bluetooth verification and the USB cable on one page: the steps check off
/// as the device is found, verified and plugged in.
fn connect_step(s: &MeetNote, k: &Kit) -> El {
    let pairing = s.workspace_host.devices.pairing.as_ref().unwrap();
    let found = !pairing.candidates.is_empty();
    let target = pairing.target();
    let steps = col()
        .key("device-pair-steps")
        .w_full()
        .gap(8.0)
        .child(step_line(k, 1, s.tr("device_pair_keep_pairing"), found))
        .child(step_line(
            k,
            2,
            s.tr("device_pair_verify"),
            target.is_some(),
        ))
        .child(step_line(k, 3, s.tr("device_pair_connect"), false));
    let candidates = found.then(|| {
        col().key("device-candidates").w_full().gap(6.0).children(
            pairing
                .candidates
                .iter()
                .enumerate()
                .map(|(index, candidate)| {
                    let selected =
                        target.is_some_and(|target| target.peripheral == candidate.peripheral);
                    candidate_row(s, k, index, candidate, selected)
                }),
        )
    });
    col()
        .w_full()
        .gap(14.0)
        .child(steps)
        .children(candidates)
        .child(connect_status(s, k))
}

/// One line under the steps: what pairing waits for, or why it cannot go on.
fn connect_status(s: &MeetNote, k: &Kit) -> El {
    let host = &s.workspace_host.devices;
    let pairing = host.pairing.as_ref().unwrap();
    let problem = match &pairing.radio {
        Radio::Off => Some(s.tr("device_bluetooth_off").to_owned()),
        Radio::Unauthorized => Some(s.tr("device_bluetooth_unauthorized").to_owned()),
        Radio::Unavailable(reason) => Some(format!(
            "{}\n{reason}",
            s.tr("device_bluetooth_unavailable")
        )),
        Radio::Starting | Radio::Scanning => None,
    };
    if let Some(problem) = problem {
        let settings = (pairing.radio == Radio::Unauthorized).then(|| {
            Button::new(s.tr("device_bluetooth_settings"))
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .on_press(|s: &mut MeetNote| s.workspace_host.devices.open_bluetooth_settings())
                .build(k)
                .key("device-bluetooth-settings")
        });
        return row()
            .key("device-pair-status")
            .w_full()
            .items_center()
            .gap(10.0)
            .padding(10.0)
            .radius(10.0)
            .fill(k.c.warning.bg)
            .border(1.0, k.c.warning.border)
            .child(
                text(problem)
                    .key("device-pair-problem")
                    .font(TextStyle::new(12.5, 500.0))
                    .color(k.c.warning.fg)
                    .wrap()
                    .grow(1.0)
                    .min_w(0.0),
            )
            .children(settings);
    }
    let line = if pairing.needs_choice() {
        s.tr("device_wait_choose").to_owned()
    } else if let Some(target) = pairing.target() {
        s.tr("device_wait_usb")
            .replace("{name}", &display_name(&target.identity.name, s))
    } else if pairing.candidates.is_empty() {
        s.tr("device_pair_searching").to_owned()
    } else {
        s.tr("device_wait_identity").to_owned()
    };
    row()
        .key("device-pair-status")
        .w_full()
        .items_center()
        .gap(8.0)
        .padding(10.0)
        .radius(10.0)
        .fill(k.c.layer_02)
        .child(Spinner::new().size(14.0).color(k.c.icon_accent).build(k))
        .child(
            text(line)
                .key("device-pair-waiting")
                .font(TextStyle::new(12.5, 500.0))
                .color(k.c.text_muted)
                .wrap()
                .grow(1.0)
                .min_w(0.0),
        )
}

fn candidate_row(
    s: &MeetNote,
    k: &Kit,
    index: usize,
    candidate: &BleCandidate,
    selected: bool,
) -> El {
    let bars = signal_bars(candidate.rssi);
    let meter = row().gap(2.0).items_end().children((1..=4u8).map(|bar| {
        el().size(3.0, 3.0 + f32::from(bar) * 2.5)
            .radius(1.5)
            .fill(if bar <= bars {
                k.c.text_accent
            } else {
                k.c.border_base
            })
    }));
    let identity = &candidate.identity;
    let detail = match &candidate.verification {
        // SN and firmware each on their own line.
        Verification::Verified(device) => format!(
            "SN {}\n{} {}",
            device.serial,
            s.tr("device_firmware"),
            device.firmware
        ),
        Verification::Rejected(reason) => format!("{}: {reason}", s.tr("device_rejected")),
        Verification::Reading => s.tr("device_verifying").to_owned(),
        Verification::Waiting => identity
            .mac
            .clone()
            .or_else(|| identity.bluetooth_name.clone())
            .unwrap_or_default(),
    };
    let badge = match &candidate.verification {
        Verification::Verified(_) => Some(pill(
            s.tr("device_verified"),
            k.c.success.bg,
            k.c.success.fg,
        )),
        Verification::Rejected(_) => Some(pill(
            s.tr("device_rejected"),
            k.c.warning.bg,
            k.c.warning.fg,
        )),
        Verification::Reading => Some(Spinner::new().size(14.0).color(k.c.icon_accent).build(k)),
        Verification::Waiting => None,
    };
    let verified = candidate.verified().is_some();
    let peripheral = candidate.peripheral.clone();
    let mut item = row()
        .key(format!("device-candidate-{index}"))
        .label(identity.name.clone())
        .w_full()
        .items_center()
        .gap(12.0)
        .padding(10.0)
        .radius(10.0)
        .fill(k.c.layer_02)
        .border(
            if selected { 1.5 } else { 1.0 },
            if selected {
                k.c.text_accent
            } else {
                k.c.border_muted
            },
        )
        .sequence(Sequence::new(&POP))
        .child(device_art(34.0))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(display_name(&identity.name, s))
                        .font(TextStyle::new(13.5, 600.0))
                        .color(k.c.text_base)
                        .truncate(),
                )
                .child(
                    text(detail)
                        .key(format!("device-candidate-{index}-detail"))
                        .font(TextStyle::new(11.5, 400.0).lh(16.0).features("'tnum' 1"))
                        .color(k.c.text_muted)
                        .selectable(true)
                        .wrap(),
                ),
        )
        .children(badge)
        .child(meter);
    if verified {
        item = item
            .role("button")
            .hover(k.c.layer_03)
            .press_scale(0.99)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |s: &mut MeetNote| s.workspace_host.devices.choose(peripheral.clone()));
    }
    item
}

fn pill(label: &str, fill: rune::Color, color: rune::Color) -> El {
    el().px(8.0).py(3.0).radius(999.0).fill(fill).child(
        text(label.to_owned())
            .font(TextStyle::new(11.0, 600.0))
            .color(color),
    )
}

fn step_line(k: &Kit, number: u8, line: &str, done: bool) -> El {
    let mark = if done {
        el().size(20.0, 20.0)
            .radius(10.0)
            .fill(k.c.success.bg)
            .items_center()
            .justify_center()
            .child(icon(&CHECK, 12.0).color(k.c.success.icon))
    } else {
        el().size(20.0, 20.0)
            .radius(10.0)
            .fill(k.c.text_accent.alpha(0.14))
            .items_center()
            .justify_center()
            .child(
                text(number.to_string())
                    .font(TextStyle::new(11.0, 700.0))
                    .color(k.c.text_accent),
            )
    };
    row()
        .key(format!("device-pair-step-line-{number}"))
        .w_full()
        .gap(10.0)
        .items_start()
        .child(mark)
        .child(
            text(line.to_owned())
                .font(TextStyle::new(13.0, 400.0).lh(20.0))
                .color(if done { k.c.text_muted } else { k.c.text_base })
                .wrap()
                .grow(1.0)
                .min_w(0.0),
        )
}

fn done_step(s: &MeetNote, k: &Kit) -> El {
    let host = &s.workspace_host.devices;
    let device = host
        .pairing
        .as_ref()
        .and_then(|pairing| pairing.paired.as_deref())
        .and_then(|key| host.registry.device(key));
    let name = device.map_or_else(
        || s.tr("device_generic").to_owned(),
        |device| display_name(&device.identity.name, s),
    );
    let serial = device
        .and_then(|device| device.identity.serial.clone())
        .map(|serial| format!("SN  {serial}"));
    col()
        .w_full()
        .items_center()
        .gap(10.0)
        .child(
            el().key("device-pair-check")
                .size(72.0, 72.0)
                .radius(36.0)
                .fill(k.c.success.bg)
                .border(1.0, k.c.success.border)
                .items_center()
                .justify_center()
                .sequence(Sequence::new(&POP))
                .child(icon(&CHECK, 36.0).color(k.c.success.icon)),
        )
        .child(title(k, name, 17.0).key("device-pair-name"))
        .children(serial.map(|serial| {
            text(serial)
                .font(TextStyle::new(12.0, 500.0).features("'tnum' 1"))
                .color(k.c.text_muted)
        }))
        .child(
            caption(k, s.tr("device_pair_done_hint"))
                .align(rune::Align::Center)
                .max_w(340.0),
        )
        .child(el().h(4.0))
        .child(
            Button::new(s.tr("done"))
                .variant(ButtonVariant::Submit)
                .control_size(ControlSize::Large)
                .on_press(|s: &mut MeetNote| s.workspace_host.devices.close_pairing())
                .build(k)
                .key("device-pair-finish")
                .default_button(),
        )
}

fn relative_time(s: &MeetNote, at_ms: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default();
    relative(s.locale(), now.saturating_sub(at_ms) / 1000)
}

/// `seconds` ago, in the locale's words.
pub fn relative(locale: ui_text::Locale, seconds: u64) -> String {
    let (key, value) = match seconds {
        0..60 => return ui_text::text(locale, "time_just_now").to_owned(),
        60..3600 => ("time_minutes_ago", seconds / 60),
        3600..86_400 => ("time_hours_ago", seconds / 3600),
        _ => ("time_days_ago", seconds / 86_400),
    };
    ui_text::text(locale, key).replace("{n}", &value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui_text::Locale;

    #[test]
    fn relative_time_picks_the_largest_whole_unit() {
        assert_eq!(relative(Locale::EnUs, 5), "just now");
        assert_eq!(relative(Locale::EnUs, 125), "2 min ago");
        assert_eq!(relative(Locale::ZhCn, 7200), "2 小时前");
        assert_eq!(relative(Locale::ZhCn, 3 * 86_400 + 5), "3 天前");
    }

    #[test]
    fn card_ids_are_ascii_and_stable() {
        assert_eq!(card_id("sn:9SFXN8N024XU81"), "device-sn-9SFXN8N024XU81");
        assert_eq!(
            card_id("mac:8C:58:23:A7:3B:43"),
            "device-mac-8C-58-23-A7-3B-43"
        );
    }
}
