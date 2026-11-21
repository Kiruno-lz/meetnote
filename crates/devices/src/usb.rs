//! Wired devices: USB descriptors from `ioreg` joined with mounted volumes
//! from `mount`, so a recording device's storage is found by its USB identity.

use std::path::PathBuf;
use std::process::Command;

use crate::catalog;
use crate::identity::DeviceIdentity;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UsbDevice {
    pub vendor_id: u16,
    pub product_id: u16,
    pub vendor_name: Option<String>,
    pub product_name: Option<String>,
    pub serial: Option<String>,
    pub bcd_device: Option<u16>,
    pub location_id: Option<u32>,
    /// Whole disks and partitions published under the device (`disk4`, `disk4s1`).
    pub bsd_names: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    /// The BSD device without `/dev/`, e.g. `disk4s1`.
    pub device: String,
    pub path: PathBuf,
}

/// A recognised recording device on the USB bus and its mounted volumes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachedDevice {
    pub usb: UsbDevice,
    pub identity: DeviceIdentity,
    pub volumes: Vec<PathBuf>,
    /// The file system UUID of the first volume: stable across ports and
    /// replugs, new only when the card is formatted.
    pub volume_uuid: Option<String>,
}

impl AttachedDevice {
    /// The key that names this unit wherever it is plugged in: the
    /// identity's or the volume's. `None` while it is known only by its USB
    /// port, as between enumeration and the volume being mounted and read.
    pub fn stable_key(&self) -> Option<String> {
        self.identity
            .key()
            .or_else(|| self.volume_uuid.as_ref().map(|uuid| format!("vol:{uuid}")))
    }

    /// The registry key: the identity's, else the volume's, else one built
    /// from the USB port.
    pub fn key(&self) -> String {
        self.stable_key().unwrap_or_else(|| {
            format!(
                "usb:{:04x}:{:04x}:{:08x}",
                self.usb.vendor_id,
                self.usb.product_id,
                self.usb.location_id.unwrap_or_default()
            )
        })
    }
}

/// Parses `ioreg -r -c IOUSBHostDevice -l -w0`: one `+-o` block per device at
/// column zero, its own properties first and its descendants (interfaces,
/// mass storage, media) indented below.
pub fn parse_ioreg(text: &str) -> Vec<UsbDevice> {
    let mut devices = Vec::new();
    let mut current: Option<(UsbDevice, bool)> = None;
    for line in text.lines() {
        if line.starts_with("+-o ") {
            devices.extend(current.take().map(|(device, _)| device));
            current = Some((UsbDevice::default(), true));
            continue;
        }
        let Some((device, own)) = current.as_mut() else {
            continue;
        };
        let body = line.trim_start_matches([' ', '|']);
        if body.starts_with("+-o ") {
            // A descendant begins: only BSD names are taken from here on.
            *own = false;
            continue;
        }
        let Some((key, value)) = property(body) else {
            continue;
        };
        if key == "BSD Name" {
            if let Some(name) = string(value)
                && !device.bsd_names.contains(&name)
            {
                device.bsd_names.push(name);
            }
            continue;
        }
        if !*own {
            continue;
        }
        match key {
            "idVendor" => device.vendor_id = number(value).unwrap_or_default() as u16,
            "idProduct" => device.product_id = number(value).unwrap_or_default() as u16,
            "bcdDevice" => device.bcd_device = number(value).map(|v| v as u16),
            "locationID" => device.location_id = number(value).map(|v| v as u32),
            "USB Vendor Name" | "kUSBVendorString" => {
                device.vendor_name = device.vendor_name.take().or_else(|| string(value));
            }
            "USB Product Name" | "kUSBProductString" => {
                device.product_name = device.product_name.take().or_else(|| string(value));
            }
            "USB Serial Number" | "kUSBSerialNumberString" => {
                device.serial = device.serial.take().or_else(|| string(value));
            }
            _ => {}
        }
    }
    devices.extend(current.map(|(device, _)| device));
    devices
}

fn property(body: &str) -> Option<(&str, &str)> {
    let rest = body.strip_prefix('"')?;
    let (key, rest) = rest.split_once('"')?;
    let value = rest.trim_start().strip_prefix('=')?.trim();
    Some((key, value))
}

fn string(value: &str) -> Option<String> {
    let inner = value.strip_prefix('"')?.strip_suffix('"')?.trim();
    (!inner.is_empty()).then(|| inner.to_owned())
}

fn number(value: &str) -> Option<u64> {
    match value.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => value.parse().ok(),
    }
}

/// Parses `/sbin/mount`: `/dev/disk4s1 on /Volumes/DJI MIC3 (msdos, local, …)`.
pub fn parse_mount(text: &str) -> Vec<Mount> {
    text.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("/dev/")?;
            let (device, rest) = rest.split_once(" on ")?;
            let (path, _) = rest.rsplit_once(" (")?;
            Some(Mount {
                device: device.to_owned(),
                path: PathBuf::from(path),
            })
        })
        .collect()
}

/// The identity a whitelisted recorder's USB descriptor gives: its catalog
/// model only. USB serial numbers and versions are not trusted — the DJI Mic
/// 3 TX reports its chip vendor's IDs and a serial every unit shares — so a
/// unit is told apart by its volume and identified over Bluetooth.
pub fn usb_identity(device: &UsbDevice) -> Option<DeviceIdentity> {
    catalog::for_usb(device).map(|model| DeviceIdentity {
        name: model.name.to_owned(),
        model_code: Some(model.model_code.to_owned()),
        ..Default::default()
    })
}

/// Recognised devices joined with the volumes mounted from their disks.
pub fn attached(devices: Vec<UsbDevice>, mounts: &[Mount]) -> Vec<AttachedDevice> {
    devices
        .into_iter()
        .filter_map(|usb| {
            let identity = usb_identity(&usb)?;
            let mut volumes: Vec<PathBuf> = mounts
                .iter()
                .filter(|mount| {
                    usb.bsd_names.iter().any(|bsd| {
                        mount.device == *bsd
                            || mount
                                .device
                                .strip_prefix(bsd.as_str())
                                .is_some_and(|rest| rest.starts_with('s'))
                    })
                })
                .map(|mount| mount.path.clone())
                .collect();
            volumes.sort();
            volumes.dedup();
            Some(AttachedDevice {
                usb,
                identity,
                volumes,
                volume_uuid: None,
            })
        })
        .collect()
}

fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("cannot run {program}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `VolumeUUID` from `diskutil info -plist`.
pub fn parse_volume_uuid(plist: &str) -> Option<String> {
    let rest = plist.split("<key>VolumeUUID</key>").nth(1)?;
    let value = rest.trim_start().strip_prefix("<string>")?;
    let (uuid, _) = value.split_once("</string>")?;
    (!uuid.is_empty()).then(|| uuid.to_owned())
}

pub fn volume_uuid(volume: &std::path::Path) -> Option<String> {
    let volume = volume.to_str()?;
    run("/usr/sbin/diskutil", &["info", "-plist", volume])
        .ok()
        .and_then(|plist| parse_volume_uuid(&plist))
}

/// The mount table as text; cheap enough to compare on every poll.
pub fn mount_table() -> Result<String, String> {
    run("/sbin/mount", &[])
}

/// All USB devices as `ioreg` describes them.
pub fn usb_devices() -> Result<Vec<UsbDevice>, String> {
    run(
        "/usr/sbin/ioreg",
        &["-r", "-c", "IOUSBHostDevice", "-l", "-w0"],
    )
    .map(|text| parse_ioreg(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ioreg` text in the shape macOS 27 prints it: a keyboard, then a DJI
    /// transmitter whose mass storage publishes `disk4` and `disk4s1`.
    const IOREG: &str = r#"+-o Magic Keyboard@01100000  <class IOUSBHostDevice, id 0x100001234, registered, matched, active, busy 0 (5 ms), retain 30>
  | {
  |   "idProduct" = 802
  |   "USB Product Name" = "Magic Keyboard"
  |   "USB Vendor Name" = "Apple Inc."
  |   "idVendor" = 1452
  |   "USB Serial Number" = "F0T1234"
  |   "locationID" = 17825792
  | }
  |
  +-o IOUSBHostInterface@0  <class IOUSBHostInterface, id 0x100001235, registered, matched, active, busy 0 (1 ms), retain 8>
+-o DJI MIC3@02100000  <class IOUSBHostDevice, id 0x100002000, registered, matched, active, busy 0 (40 ms), retain 25>
  | {
  |   "sessionID" = 123456789
  |   "USB Product Name" = "DJI MIC3 DMT03"
  |   "idProduct" = 0x0020
  |   "kUSBSerialNumberString" = "9SFXN8N024XU81"
  |   "USB Vendor Name" = "DJI"
  |   "bcdDevice" = 256
  |   "idVendor" = 11427
  |   "locationID" = 34603008
  | }
  |
  +-o IOUSBHostInterface@0  <class IOUSBHostInterface, id 0x100002001, registered, matched, active, busy 0 (30 ms), retain 9>
    | {
    |   "idProduct" = 999
    |   "USB Product Name" = "Mass Storage"
    | }
    |
    +-o IOUSBMassStorageInterfaceNub  <class IOUSBMassStorageInterfaceNub, id 0x100002003, registered, matched, active, busy 0 (28 ms), retain 7>
      +-o DJI MIC3 Media  <class IOMedia, id 0x100002010, registered, matched, active, busy 0 (12 ms), retain 12>
        | {
        |   "BSD Name" = "disk4"
        | }
        |
        +-o Untitled 1@1  <class IOMedia, id 0x100002012, registered, matched, active, busy 0 (10 ms), retain 11>
          | {
          |   "BSD Name" = "disk4s1"
          | }
"#;

    /// A DJI Mic 3 TX as macOS 27 reports it: the Bestechnic chip's IDs, a
    /// serial every unit shares and a FAT volume named `NO NAME`.
    const IOREG_MIC3: &str = r#"+-o Wireles Mic Tx@00100000  <class IOUSBHostDevice, id 0x10000a1b2, registered, matched, active, busy 0 (12 ms), retain 24>
  | {
  |   "USB Product Name" = "Wireles Mic Tx"
  |   "idProduct" = 260
  |   "USB Vendor Name" = "bestechnic"
  |   "kUSBSerialNumberString" = "20151006.1"
  |   "bcdDevice" = 256
  |   "idVendor" = 48727
  |   "locationID" = 1048576
  | }
  |
  +-o IOUSBHostInterface@0  <class IOUSBHostInterface, id 0x10000a1b4, registered, matched, active, busy 0 (5 ms), retain 8>
    +-o IOUSBMassStorageInterfaceNub  <class IOUSBMassStorageInterfaceNub, id 0x10000a1b6, registered, matched, active, busy 0 (4 ms), retain 7>
      +-o Wireless  Mic Tx Media  <class IOMedia, id 0x10000a1c0, registered, matched, active, busy 0 (2 ms), retain 12>
        | {
        |   "BSD Name" = "disk4"
        | }
        |
        +-o Untitled 1@1  <class IOMedia, id 0x10000a1c2, registered, matched, active, busy 0 (1 ms), retain 11>
          | {
          |   "BSD Name" = "disk4s1"
          | }
"#;

    #[test]
    fn recognises_the_mic3_transmitter_by_its_chip_and_keys_it_by_volume() {
        let mounts = parse_mount(
            "/dev/disk4s1 on /Volumes/NO NAME (msdos, local, nodev, nosuid, noowners, noatime, fskit)\n",
        );
        let mut found = attached(parse_ioreg(IOREG_MIC3), &mounts);
        assert_eq!(found.len(), 1);
        let device = &mut found[0];
        assert_eq!(device.usb.vendor_id, 0xBE57);
        assert_eq!(
            device.identity.serial, None,
            "the chip serial is not the unit's"
        );
        assert_eq!(device.identity.name, "DJI Mic 3 TX");
        assert_eq!(device.identity.model_code.as_deref(), Some("DMT03"));
        assert_eq!(device.volumes, [PathBuf::from("/Volumes/NO NAME")]);

        device.volume_uuid = parse_volume_uuid(
            "<dict>\n\t<key>VolumeName</key>\n\t<string>NO NAME</string>\n\t<key>VolumeUUID</key>\n\t<string>645BECBB-3236-3D95-9BAC-7E99F3B01EED</string>\n</dict>",
        );
        assert_eq!(device.key(), "vol:645BECBB-3236-3D95-9BAC-7E99F3B01EED");

        let other_bes = UsbDevice {
            vendor_id: 0xBE57,
            product_id: 0x0104,
            product_name: Some("BES Headset".into()),
            ..Default::default()
        };
        assert!(
            usb_identity(&other_bes).is_none(),
            "other Bestechnic products are not recorders"
        );
    }

    const MOUNT: &str = "/dev/disk3s3s1 on / (apfs, sealed, local, read-only, journaled)
devfs on /dev (devfs, local, nobrowse)
/dev/disk4s1 on /Volumes/DJI MIC3 (msdos, local, nodev, nosuid, noowners, noatime, fskit)
/dev/disk40s1 on /Volumes/Other (msdos, local)
";

    #[test]
    fn parses_device_properties_and_descendant_disks() {
        let devices = parse_ioreg(IOREG);
        assert_eq!(devices.len(), 2);
        let dji = &devices[1];
        assert_eq!(dji.vendor_id, 0x2CA3);
        assert_eq!(dji.product_id, 0x20, "the interface's idProduct is ignored");
        assert_eq!(dji.product_name.as_deref(), Some("DJI MIC3 DMT03"));
        assert_eq!(dji.serial.as_deref(), Some("9SFXN8N024XU81"));
        assert_eq!(dji.bcd_device, Some(256));
        assert_eq!(dji.location_id, Some(34603008));
        assert_eq!(dji.bsd_names, ["disk4", "disk4s1"]);
        assert!(devices[0].bsd_names.is_empty());
    }

    #[test]
    fn parses_mounts_with_spaces_and_skips_non_devices() {
        let mounts = parse_mount(MOUNT);
        assert_eq!(mounts.len(), 3);
        assert_eq!(mounts[1].device, "disk4s1");
        assert_eq!(mounts[1].path, PathBuf::from("/Volumes/DJI MIC3"));
    }

    #[test]
    fn joins_only_whitelisted_devices_with_their_own_volumes() {
        assert!(
            attached(parse_ioreg(IOREG), &parse_mount(MOUNT)).is_empty(),
            "neither the keyboard nor an unlisted DJI product is a recorder"
        );
    }

    /// Recorded on a real DMT03: the cable enumerates the device, then its
    /// disk, and only then is the volume mounted and its UUID read.
    #[test]
    fn only_a_mounted_volume_gives_a_stable_key() {
        let devices = parse_ioreg(IOREG_MIC3);
        let enumerated = attached(devices.clone(), &[]).remove(0);
        assert_eq!(enumerated.key(), "usb:be57:0104:00100000");
        assert_eq!(enumerated.stable_key(), None, "the port is not the unit");

        let mut mounted = attached(
            devices,
            &parse_mount("/dev/disk4 on /Volumes/NO NAME (msdos, local, fskit)\n"),
        )
        .remove(0);
        assert_eq!(mounted.stable_key(), None, "the uuid is not read yet");
        mounted.volume_uuid = Some("645BECBB-3236-3D95-9BAC-7E99F3B01EED".into());
        assert_eq!(
            mounted.stable_key().as_deref(),
            Some("vol:645BECBB-3236-3D95-9BAC-7E99F3B01EED")
        );
    }

    #[test]
    fn a_device_without_serial_is_keyed_by_its_port() {
        let device = AttachedDevice {
            usb: UsbDevice {
                vendor_id: 0x2CA3,
                product_id: 0x20,
                location_id: Some(0x0210_0000),
                ..Default::default()
            },
            identity: DeviceIdentity::default(),
            volumes: Vec::new(),
            volume_uuid: None,
        };
        assert_eq!(device.key(), "usb:2ca3:0020:02100000");
        assert!(
            usb_identity(&UsbDevice {
                vendor_id: 0x05ac,
                product_name: Some("Keyboard".into()),
                ..Default::default()
            })
            .is_none()
        );
    }
}
