//! Bluetooth LE discovery. Devices in pairing mode advertise their name and
//! address only. Only advertisements of a [`crate::catalog`] model become
//! candidates; each is connected over LE GATT once to read and verify its
//! model, serial number and firmware ([`crate::verify`]). MeetNote never
//! opens a classic (HFP) connection.

use std::collections::BTreeMap;

use futures::channel::mpsc::UnboundedSender;
use serde::{Deserialize, Serialize};

use crate::catalog::{self, Model};
use crate::identity::DeviceIdentity;
use crate::verify::VerifiedDevice;

/// One advertisement as CoreBluetooth parses it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Advertisement {
    /// The platform's identifier for the peripheral (a UUID on macOS).
    pub peripheral: String,
    pub local_name: Option<String>,
    /// Manufacturer data including its two-byte little-endian company ID.
    pub manufacturer_data: Option<Vec<u8>>,
    /// Service data keyed by the service UUID string.
    pub service_data: Vec<(String, Vec<u8>)>,
    pub service_uuids: Vec<String>,
    pub rssi: i16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BluetoothState {
    Starting,
    PoweredOn,
    PoweredOff,
    Unauthorized,
    Unsupported,
    Resetting,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BleEvent {
    State(BluetoothState),
    Seen(Advertisement),
    /// Connected over LE to read the peripheral's identity.
    Identifying(String),
    /// The identity read, verified or rejected with a reason.
    Identified {
        peripheral: String,
        outcome: Result<VerifiedDevice, String>,
    },
}

/// The whitelisted model to verify an advertising peripheral as, if any.
/// Only selects whom to connect to and what to expect; never identifies.
pub fn model_to_verify(advertisement: &Advertisement) -> Option<&'static Model> {
    catalog::for_advertisement(
        advertisement.local_name.as_deref(),
        advertisement.manufacturer_data.as_deref(),
    )
}

/// How far identification of one candidate got.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Verification {
    #[default]
    Waiting,
    Reading,
    Verified(VerifiedDevice),
    Rejected(String),
}

/// Why scanning cannot start in this process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BluetoothBlock {
    /// The process has no `NSBluetoothAlwaysUsageDescription`; macOS would
    /// terminate it on first Bluetooth use, so scanning never starts.
    NeedsAppBundle,
    Unsupported,
    Failed(String),
}

impl std::fmt::Display for BluetoothBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NeedsAppBundle => f.write_str(
                "Bluetooth search needs MeetNote.app: this process declares no Bluetooth usage",
            ),
            Self::Unsupported => f.write_str("Bluetooth search is not supported on this system"),
            Self::Failed(message) => write!(f, "Bluetooth search could not start: {message}"),
        }
    }
}

/// A running scan; dropping it stops scanning and releases the radio.
pub struct BleScan {
    _session: Box<dyn Send>,
}

impl BleScan {
    pub fn new(session: impl Send + 'static) -> Self {
        Self {
            _session: Box::new(session),
        }
    }
}

/// Starts listening for advertisements. Events arrive on `events` from a
/// background queue until the returned scan is dropped.
pub fn start_scan(events: UnboundedSender<BleEvent>) -> Result<BleScan, BluetoothBlock> {
    #[cfg(target_os = "macos")]
    {
        crate::macos::ble::start_scan(events)
    }
    #[cfg(not(target_os = "macos"))]
    {
        drop(events);
        Err(BluetoothBlock::Unsupported)
    }
}

/// The argument that makes a MeetNote binary act as its own Bluetooth helper;
/// `main` hands it to [`run_helper`] before anything else.
pub const HELPER_ARG: &str = "__meetnote-bluetooth-helper";

/// Runs the Bluetooth helper until its parent closes stdin; the exit code.
pub fn run_helper() -> i32 {
    #[cfg(target_os = "macos")]
    {
        crate::macos::helper::run()
    }
    #[cfg(not(target_os = "macos"))]
    {
        1
    }
}

/// A recognised device near the computer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BleCandidate {
    pub peripheral: String,
    /// The whitelisted model its advertisements match.
    pub model: &'static Model,
    /// Advertised facts, completed with the verified ones.
    pub identity: DeviceIdentity,
    pub rssi: i16,
    pub verification: Verification,
}

impl BleCandidate {
    pub fn verified(&self) -> Option<&VerifiedDevice> {
        match &self.verification {
            Verification::Verified(device) => Some(device),
            _ => None,
        }
    }
}

/// Advertisements of one peripheral, merged: devices split their name,
/// manufacturer data and service data across packets.
#[derive(Clone, Debug, Default)]
struct Sighting {
    names: Vec<String>,
    manufacturer_data: Option<Vec<u8>>,
    service_data: BTreeMap<String, Vec<u8>>,
    rssi: i16,
}

#[derive(Debug, Default)]
pub struct Sightings {
    by_peripheral: BTreeMap<String, Sighting>,
    verification: BTreeMap<String, Verification>,
}

impl Sightings {
    /// Applies an identification event; true when a candidate changed.
    pub fn verify(&mut self, peripheral: &str, verification: Verification) -> bool {
        let previous = self
            .verification
            .insert(peripheral.to_owned(), verification.clone());
        previous.as_ref() != Some(&verification) && self.by_peripheral.contains_key(peripheral)
    }

    /// Records `advertisement`; true when the recognised candidates changed.
    pub fn observe(&mut self, advertisement: Advertisement) -> bool {
        let before = self.candidate(&advertisement.peripheral);
        let sighting = self
            .by_peripheral
            .entry(advertisement.peripheral.clone())
            .or_default();
        if let Some(name) = advertisement.local_name.filter(|name| !name.is_empty())
            && !sighting.names.contains(&name)
        {
            sighting.names.push(name);
        }
        if advertisement.manufacturer_data.is_some() {
            sighting.manufacturer_data = advertisement.manufacturer_data;
        }
        sighting.service_data.extend(advertisement.service_data);
        sighting.rssi = advertisement.rssi;
        let after = self.candidate(&advertisement.peripheral);
        // Signal strength alone moves by a few dB every packet; only count a
        // whole signal bar as a change so the list does not redraw constantly.
        match (&before, &after) {
            (Some(before), Some(after)) => {
                before.identity != after.identity
                    || signal_bars(before.rssi) != signal_bars(after.rssi)
            }
            (None, None) => false,
            _ => true,
        }
    }

    pub fn clear(&mut self) {
        self.by_peripheral.clear();
        self.verification.clear();
    }

    fn candidate(&self, peripheral: &str) -> Option<BleCandidate> {
        let sighting = self.by_peripheral.get(peripheral)?;
        let (model, mut identity) = recognize(sighting)?;
        let verification = self
            .verification
            .get(peripheral)
            .cloned()
            .unwrap_or_default();
        if let Verification::Verified(device) = &verification {
            identity.model_code = Some(device.model_code.clone());
            identity.serial = Some(device.serial.clone());
            identity.firmware = Some(device.firmware.clone());
        }
        Some(BleCandidate {
            peripheral: peripheral.to_owned(),
            model,
            identity,
            rssi: sighting.rssi,
            verification,
        })
    }

    /// Recognised devices, strongest signal first.
    pub fn candidates(&self) -> Vec<BleCandidate> {
        let mut candidates: Vec<_> = self
            .by_peripheral
            .keys()
            .filter_map(|peripheral| self.candidate(peripheral))
            .collect();
        candidates.sort_by(|a, b| b.rssi.cmp(&a.rssi).then(a.peripheral.cmp(&b.peripheral)));
        candidates
    }
}

/// Signal strength in four bars, as a phone shows it.
pub fn signal_bars(rssi: i16) -> u8 {
    match rssi {
        r if r >= -55 => 4,
        r if r >= -67 => 3,
        r if r >= -80 => 2,
        _ => 1,
    }
}

fn recognize(sighting: &Sighting) -> Option<(&'static Model, DeviceIdentity)> {
    let manufacturer = sighting.manufacturer_data.as_deref();
    let model = catalog::for_advertisement(None, manufacturer).or_else(|| {
        sighting
            .names
            .iter()
            .find_map(|name| catalog::for_advertisement(Some(name), None))
    })?;
    let bluetooth_name = sighting
        .names
        .iter()
        .find(|name| name.starts_with(model.advertising.name_prefix))
        .cloned();
    // Model code, serial and firmware never come from advertisements: only
    // the verified GATT identity supplies them.
    let identity = DeviceIdentity {
        name: bluetooth_name
            .as_deref()
            .map_or(model.name, display_name)
            .to_owned(),
        model_code: None,
        serial: None,
        mac: model.mac(manufacturer),
        firmware: None,
        bluetooth_name,
    };
    Some((model, identity))
}

/// `DJI Mic 3 TX-a73b42` → `DJI Mic 3 TX`: the suffix names one unit.
fn display_name(name: &str) -> &str {
    match name.rsplit_once('-') {
        Some((base, suffix))
            if !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_hexdigit()) =>
        {
            base.trim_end()
        }
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    const PERIPHERAL: &str = "3E01A09A-401A-1DEF-0B0A-A25C0C767481";

    /// The four packet shapes a DMT03 in pairing mode sent on 2026-11-20.
    fn dmt03_packets() -> Vec<Advertisement> {
        let fdee = (
            "FDEE".to_owned(),
            hex("010107040411f81232534543ff0015023831"),
        );
        let base = Advertisement {
            peripheral: PERIPHERAL.into(),
            rssi: -45,
            ..Default::default()
        };
        vec![
            Advertisement {
                local_name: Some("BES_BLE".into()),
                manufacturer_data: Some(hex("aa08b200008c5823a73b43")),
                service_data: vec![fdee.clone()],
                service_uuids: vec!["FFF0".into()],
                ..base.clone()
            },
            Advertisement {
                local_name: Some("DJI Mic 3 TX-a73b42".into()),
                ..base.clone()
            },
            Advertisement {
                local_name: Some("DJI Mic 3 TX-a73b42".into()),
                manufacturer_data: Some(hex("aa08b200008c5823a73b43")),
                service_uuids: vec!["FFF0".into()],
                ..base.clone()
            },
            Advertisement {
                local_name: Some("DJI Mic 3 TX-a73b42".into()),
                service_data: vec![fdee],
                ..base
            },
        ]
    }

    #[test]
    fn recognises_the_captured_dmt03_and_reads_its_address() {
        let mut sightings = Sightings::default();
        let mut changed = false;
        for packet in dmt03_packets() {
            changed |= sightings.observe(packet);
        }
        assert!(changed);
        let candidates = sightings.candidates();
        assert_eq!(candidates.len(), 1);
        let identity = &candidates[0].identity;
        assert_eq!(identity.name, "DJI Mic 3 TX");
        assert_eq!(identity.mac.as_deref(), Some("8C:58:23:A7:3B:43"));
        assert_eq!(
            identity.bluetooth_name.as_deref(),
            Some("DJI Mic 3 TX-a73b42")
        );
        // The advertisement carries neither: GATT identification does.
        assert_eq!(identity.serial, None);
        assert_eq!(identity.firmware, None);
    }

    #[test]
    fn manufacturer_data_alone_names_a_known_product() {
        let mut sightings = Sightings::default();
        sightings.observe(dmt03_packets().remove(0));
        let identity = &sightings.candidates()[0].identity;
        assert_eq!(identity.name, "DJI Mic 3 TX");
        assert_eq!(identity.bluetooth_name, None);
    }

    #[test]
    fn rejects_other_manufacturers() {
        let mut sightings = Sightings::default();
        for (name, data) in [
            (None, "4c0012020000"),
            (Some("firefly key 773D"), "434602"),
            (Some("XiaoanTech"), "108063542527"),
        ] {
            assert!(!sightings.observe(Advertisement {
                peripheral: format!("other-{data}"),
                local_name: name.map(str::to_owned),
                manufacturer_data: Some(hex(data)),
                rssi: -60,
                ..Default::default()
            }));
        }
        assert!(sightings.candidates().is_empty());
    }

    #[test]
    fn advertised_text_never_supplies_serial_or_firmware() {
        let mut sightings = Sightings::default();
        let mut data = hex("aa08b20000");
        data.extend(b"9SFXN8N024XU81");
        sightings.observe(Advertisement {
            peripheral: "p".into(),
            manufacturer_data: Some(data),
            service_data: vec![("FFF0".into(), b"v03.01.00.10".to_vec())],
            rssi: -50,
            ..Default::default()
        });
        let identity = &sightings.candidates()[0].identity;
        assert_eq!(identity.serial, None);
        assert_eq!(identity.firmware, None);
    }

    #[test]
    fn a_verified_identity_completes_the_candidate() {
        let mut sightings = Sightings::default();
        for packet in dmt03_packets() {
            sightings.observe(packet);
        }
        assert_eq!(
            sightings.candidates()[0].verification,
            Verification::Waiting
        );
        assert!(sightings.verify(PERIPHERAL, Verification::Reading));
        assert!(
            !sightings.verify(PERIPHERAL, Verification::Reading),
            "unchanged"
        );
        let device = VerifiedDevice {
            hardware: "DM303_TX".into(),
            model_code: "DMT03".into(),
            serial: "9SFXN8N024XU81".into(),
            firmware: "03.01.00.10".into(),
            loader: None,
            name_suffix: Some("a73b42".into()),
        };
        assert!(sightings.verify(PERIPHERAL, Verification::Verified(device.clone())));
        let candidate = &sightings.candidates()[0];
        assert_eq!(candidate.verified(), Some(&device));
        assert_eq!(candidate.identity.serial.as_deref(), Some("9SFXN8N024XU81"));
        assert_eq!(candidate.identity.model_code.as_deref(), Some("DMT03"));
        assert_eq!(candidate.identity.firmware.as_deref(), Some("03.01.00.10"));
        assert!(!sightings.verify("unseen", Verification::Reading));
    }

    #[test]
    fn only_whitelisted_models_are_verified_or_offered() {
        let mut packets = dmt03_packets();
        assert_eq!(model_to_verify(&packets[0]), Some(&catalog::DJI_MIC3_TX));
        assert_eq!(model_to_verify(&packets[1]), Some(&catalog::DJI_MIC3_TX));
        packets[1].local_name = Some("DJI Osmo Pocket 3".into());
        assert_eq!(model_to_verify(&packets[1]), None);
        // DJI's company ID with another product code: a DJI device MeetNote
        // does not support is neither connected to nor listed.
        let other_dji = Advertisement {
            peripheral: "osmo".into(),
            local_name: Some("DJI Osmo Pocket 3".into()),
            manufacturer_data: Some(hex("aa08c100001122334455aa")),
            rssi: -40,
            ..Default::default()
        };
        assert_eq!(model_to_verify(&other_dji), None);
        let mut sightings = Sightings::default();
        assert!(!sightings.observe(other_dji));
        assert!(sightings.candidates().is_empty());
    }

    #[test]
    fn candidates_sort_by_signal_and_rssi_jitter_is_not_a_change() {
        let mut sightings = Sightings::default();
        let packet = |peripheral: &str, rssi| Advertisement {
            peripheral: peripheral.into(),
            local_name: Some("DJI Mic 3 TX-000001".into()),
            rssi,
            ..Default::default()
        };
        assert!(sightings.observe(packet("far", -85)));
        assert!(sightings.observe(packet("near", -40)));
        assert!(!sightings.observe(packet("near", -42)), "same bar");
        assert!(sightings.observe(packet("near", -70)), "bar changed");
        let order: Vec<_> = sightings
            .candidates()
            .into_iter()
            .map(|candidate| candidate.peripheral)
            .collect();
        assert_eq!(order, ["near", "far"]);
        assert_eq!(signal_bars(-45), 4);
        assert_eq!(signal_bars(-90), 1);
    }
}
