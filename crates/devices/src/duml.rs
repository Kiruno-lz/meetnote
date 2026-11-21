//! DJI DUML v1 frames on a transmitter's LE GATT channel, and the
//! verification that turns them into a model, serial number and firmware.
//!
//! A DJI Mic 3 TX pushes `5B/03` device information (firmware + SN) on
//! `FFF4` and answers the read-only version query `00/01` written to `FFF5`.
//! A device is identified only when both frames pass their CRCs, the
//! hardware version is exactly the catalog model's and both frames report
//! the same firmware. See `docs/reserch/dji-mic3-ble-identification.md`.

use crate::verify::{GattProfile, GattSession, Rejection, VerifiedDevice, readable_serial};

/// The DUML channel: notifications on `FFF4`, Write Command on `FFF5`.
pub const GATT: GattProfile = GattProfile {
    service: "FFF0",
    notify: "FFF4",
    write: "FFF5",
};

/// Version queries per connection; the device answers in about 0.1 s.
const MAX_QUERIES: u8 = 4;

const SOF: u8 = 0x55;
const HEADER: usize = 11;
const MIN_FRAME: usize = HEADER + 2;
const APP: u8 = 0x02;
const TRANSMITTER: u8 = 0x3A;
const REQUEST: u8 = 0x40;
const RESPONSE: u8 = 0x80;

pub fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0x77u8;
    for byte in data {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0x8C
            } else {
                crc >> 1
            };
        }
    }
    crc
}

pub fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0x3692u16;
    for byte in data {
        crc ^= u16::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0x8408
            } else {
                crc >> 1
            };
        }
    }
    crc
}

/// A frame with both CRCs set.
pub fn build(
    sender: u8,
    receiver: u8,
    seq: u16,
    cmd_type: u8,
    cmdset: u8,
    cmdid: u8,
    payload: &[u8],
) -> Vec<u8> {
    let length = MIN_FRAME + payload.len();
    let mut frame = vec![
        SOF,
        (length & 0xFF) as u8,
        ((length >> 8) & 0x03) as u8 | (1 << 2),
    ];
    frame.push(crc8(&frame));
    frame.extend([sender, receiver]);
    frame.extend(seq.to_le_bytes());
    frame.extend([cmd_type, cmdset, cmdid]);
    frame.extend(payload);
    let crc = crc16(&frame);
    frame.extend(crc.to_le_bytes());
    frame
}

/// The read-only version query `00/01` the app writes to `FFF5`.
pub fn version_query(seq: u16) -> Vec<u8> {
    build(APP, TRANSMITTER, seq, REQUEST, 0x00, 0x01, &[])
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub sender: u8,
    pub receiver: u8,
    pub seq: u16,
    pub cmd_type: u8,
    pub cmdset: u8,
    pub cmdid: u8,
    pub payload: Vec<u8>,
}

/// Splits a byte stream into CRC-valid frames. Bytes that do not start a
/// valid frame are skipped one at a time; an incomplete frame at the end is
/// left in `buffer` for the next notification.
pub fn drain_frames(buffer: &mut Vec<u8>) -> Vec<Frame> {
    let mut frames = Vec::new();
    let mut start = 0;
    while start + MIN_FRAME <= buffer.len() {
        let bytes = &buffer[start..];
        let length = usize::from(bytes[1]) | (usize::from(bytes[2] & 0x03) << 8);
        let plausible = bytes[0] == SOF && length >= MIN_FRAME && crc8(&bytes[..3]) == bytes[3];
        if !plausible {
            start += 1;
            continue;
        }
        if bytes.len() < length {
            break;
        }
        let frame = &bytes[..length];
        let crc = u16::from_le_bytes([frame[length - 2], frame[length - 1]]);
        if crc != crc16(&frame[..length - 2]) {
            start += 1;
            continue;
        }
        frames.push(Frame {
            sender: frame[4],
            receiver: frame[5],
            seq: u16::from_le_bytes([frame[6], frame[7]]),
            cmd_type: frame[8],
            cmdset: frame[9],
            cmdid: frame[10],
            payload: frame[HEADER..length - 2].to_vec(),
        });
        start += length;
    }
    buffer.drain(..start.min(buffer.len()));
    frames
}

/// `0A 00 01 03` → `03.01.00.10`.
pub fn version(bytes: &[u8]) -> Option<String> {
    let bytes: [u8; 4] = bytes.try_into().ok()?;
    Some(
        bytes
            .iter()
            .rev()
            .map(|byte| format!("{byte:02}"))
            .collect::<Vec<_>>()
            .join("."),
    )
}

/// Collects frames from one transmitter until it can be verified.
#[derive(Debug)]
pub struct Identifier {
    /// The catalog model's hardware version, compared exactly, and its code.
    hardware: &'static str,
    model_code: &'static str,
    buffer: Vec<u8>,
    /// From the `00/01` reply: hardware, loader, app version.
    version_reply: Option<(String, Option<String>, Option<String>)>,
    /// From the `5B/03` push: firmware and serial bytes, name suffix.
    info: Option<(String, Vec<u8>)>,
    name_suffix: Option<String>,
}

impl Identifier {
    pub fn new(hardware: &'static str, model_code: &'static str) -> Self {
        Self {
            hardware,
            model_code,
            buffer: Vec::new(),
            version_reply: None,
            info: None,
            name_suffix: None,
        }
    }

    /// Feeds one notification; returns the outcome once both frames arrived.
    pub fn feed(&mut self, bytes: &[u8]) -> Option<Result<VerifiedDevice, Rejection>> {
        self.buffer.extend_from_slice(bytes);
        for frame in drain_frames(&mut self.buffer) {
            self.accept(&frame);
        }
        self.outcome()
    }

    /// Whether the version query has been answered.
    pub fn has_version_reply(&self) -> bool {
        self.version_reply.is_some()
    }

    fn accept(&mut self, frame: &Frame) {
        if frame.sender != TRANSMITTER {
            return;
        }
        let payload = frame.payload.as_slice();
        match (frame.cmdset, frame.cmdid) {
            (0x00, 0x01) if frame.cmd_type & RESPONSE != 0 => {
                if payload.first() != Some(&0) || payload.len() < 18 {
                    return;
                }
                let hardware = &payload[2..18];
                let end = hardware
                    .iter()
                    .position(|byte| *byte == 0)
                    .unwrap_or(hardware.len());
                let hardware = String::from_utf8_lossy(&hardware[..end]).into_owned();
                let loader = payload.get(18..22).and_then(version);
                let app = payload.get(22..26).and_then(version);
                self.version_reply = Some((hardware, loader, app));
            }
            (0x5B, 0x03) if payload.first() == Some(&0x03) => {
                for (kind, value) in records(payload) {
                    match kind {
                        0x01 if value.len() > 4 => {
                            if let Some(firmware) = version(&value[..4]) {
                                self.info = Some((firmware, value[4..].to_vec()));
                            }
                        }
                        0x04 => {
                            self.name_suffix =
                                Some(String::from_utf8_lossy(value).trim().to_owned());
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn outcome(&self) -> Option<Result<VerifiedDevice, Rejection>> {
        let (hardware, loader, app) = self.version_reply.as_ref()?;
        let (firmware, serial) = self.info.as_ref()?;
        if hardware != self.hardware {
            return Some(Err(Rejection::UnknownHardware(hardware.clone())));
        }
        if app.as_ref() != Some(firmware) {
            return Some(Err(Rejection::FirmwareMismatch {
                info: firmware.clone(),
                version: app.clone().unwrap_or_default(),
            }));
        }
        let Some(serial) = readable_serial(serial) else {
            return Some(Err(Rejection::InvalidSerial));
        };
        Some(Ok(VerifiedDevice {
            hardware: hardware.clone(),
            model_code: self.model_code.to_owned(),
            serial,
            firmware: firmware.clone(),
            loader: loader.clone(),
            name_suffix: self.name_suffix.clone(),
        }))
    }
}

/// Verifies one transmitter: asks for the version until it answers.
#[derive(Debug)]
pub struct Session {
    identifier: Identifier,
    queries: u8,
}

impl Session {
    pub fn new(hardware: &'static str, model_code: &'static str) -> Self {
        Self {
            identifier: Identifier::new(hardware, model_code),
            queries: 0,
        }
    }
}

impl GattSession for Session {
    fn next_request(&mut self) -> Option<Vec<u8>> {
        if self.identifier.has_version_reply() || self.queries >= MAX_QUERIES {
            return None;
        }
        let query = version_query(0x1001 + u16::from(self.queries));
        self.queries += 1;
        Some(query)
    }

    fn feed(&mut self, notification: &[u8]) -> Option<Result<VerifiedDevice, Rejection>> {
        self.identifier.feed(notification)
    }
}

/// `5B/03` payload: `sub(1) total(2)` then `type(1) reserved(4) len(1) value`.
fn records(payload: &[u8]) -> Vec<(u8, &[u8])> {
    let Some(total) = payload.get(1..3) else {
        return Vec::new();
    };
    let end = payload
        .len()
        .min(3 + usize::from(u16::from_le_bytes([total[0], total[1]])));
    let mut records = Vec::new();
    let mut at = 3;
    while at + 6 <= end {
        let length = usize::from(payload[at + 5]);
        let Some(value) = payload.get(at + 6..at + 6 + length) else {
            break;
        };
        records.push((payload[at], value));
        at += 6 + length;
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(text: &str) -> Vec<u8> {
        text.split_whitespace()
            .map(|byte| u8::from_str_radix(byte, 16).unwrap())
            .collect()
    }

    /// The `00/01` reply and `5B/03` push a DJI Mic 3 TX (firmware
    /// 03.01.00.10) sent on 2026-11-20.
    const VERSION_REPLY: &str = "55 2B 04 58 3A 02 3A 10 80 00 01 00 01 44 4D 33 30 33 5F 54 58 00 00 00 00 00 00 00 00 10 05 19 63 0A 00 01 03 01 00 00 00 78 DE";
    const DEVICE_INFO: &str = "55 34 04 AC 3A 02 7F 63 00 5B 03 03 24 00 01 00 00 00 00 12 0A 00 01 03 39 53 46 58 4E 38 4E 30 32 34 58 55 38 31 04 00 00 00 00 06 61 37 33 62 34 32 AA 6F";

    fn mic3() -> Identifier {
        Identifier::new("DM303_TX", "DMT03")
    }

    /// `frame` with a new payload byte at `at` and both CRCs recomputed.
    fn rewrite(frame: &[u8], at: usize, bytes: &[u8]) -> Vec<u8> {
        let mut frame = frame.to_vec();
        frame[at..at + bytes.len()].copy_from_slice(bytes);
        let length = frame.len();
        let crc = crc16(&frame[..length - 2]);
        frame[length - 2..].copy_from_slice(&crc.to_le_bytes());
        frame
    }

    #[test]
    fn builds_the_version_query_byte_for_byte() {
        assert_eq!(
            build(0x02, 0x3A, 0x1001, 0x40, 0x00, 0x01, &[]),
            hex("55 0D 04 33 02 3A 01 10 40 00 01 9D C2")
        );
        assert_eq!(
            version_query(0x1001),
            hex("55 0D 04 33 02 3A 01 10 40 00 01 9D C2")
        );
    }

    #[test]
    fn both_frames_identify_the_transmitter() {
        let mut identifier = mic3();
        assert_eq!(
            identifier.feed(&hex(DEVICE_INFO)),
            None,
            "model still unknown"
        );
        let device = identifier.feed(&hex(VERSION_REPLY)).unwrap().unwrap();
        assert_eq!(
            device,
            VerifiedDevice {
                hardware: "DM303_TX".into(),
                model_code: "DMT03".into(),
                serial: "9SFXN8N024XU81".into(),
                firmware: "03.01.00.10".into(),
                loader: Some("99.25.05.16".into()),
                name_suffix: Some("a73b42".into()),
            }
        );
    }

    #[test]
    fn device_info_alone_does_not_identify() {
        let mut identifier = mic3();
        for _ in 0..3 {
            assert_eq!(identifier.feed(&hex(DEVICE_INFO)), None);
        }
        assert!(!identifier.has_version_reply());
    }

    #[test]
    fn a_corrupted_crc_drops_the_frame() {
        let mut reply = hex(VERSION_REPLY);
        *reply.last_mut().unwrap() ^= 0xFF;
        let mut identifier = mic3();
        identifier.feed(&hex(DEVICE_INFO));
        assert_eq!(identifier.feed(&reply), None);
        assert!(!identifier.has_version_reply());
    }

    #[test]
    fn hardware_must_match_exactly() {
        // `DM303_TX` → `DMT03_TX`: offset 11 (payload) + 2 + 2.
        let reply = rewrite(&hex(VERSION_REPLY), 15, b"T0");
        let mut identifier = mic3();
        identifier.feed(&hex(DEVICE_INFO));
        assert_eq!(
            identifier.feed(&reply),
            Some(Err(Rejection::UnknownHardware("DMT03_TX".into())))
        );
    }

    #[test]
    fn firmware_must_agree_between_frames() {
        // App version at payload 22..26 → `04.01.00.10`.
        let reply = rewrite(&hex(VERSION_REPLY), HEADER + 22, &[0x0A, 0x00, 0x01, 0x04]);
        let mut identifier = mic3();
        identifier.feed(&hex(DEVICE_INFO));
        assert_eq!(
            identifier.feed(&reply),
            Some(Err(Rejection::FirmwareMismatch {
                info: "03.01.00.10".into(),
                version: "04.01.00.10".into(),
            }))
        );
    }

    #[test]
    fn frames_split_across_notifications_and_noise_are_reassembled() {
        let mut stream = vec![0x00, 0x55, 0x13];
        stream.extend(hex(DEVICE_INFO));
        stream.extend(hex(VERSION_REPLY));
        let mut identifier = mic3();
        let mut outcome = None;
        for chunk in stream.chunks(20) {
            outcome = identifier.feed(chunk).or(outcome);
        }
        assert_eq!(outcome.unwrap().unwrap().serial, "9SFXN8N024XU81");
    }

    #[test]
    fn frames_from_other_senders_are_ignored() {
        let mut identifier = mic3();
        let push = rewrite(&hex(DEVICE_INFO), 4, &[0x1A]);
        // The header CRC covers only bytes 0..3, so the frame stays valid.
        identifier.feed(&push);
        assert_eq!(identifier.feed(&hex(VERSION_REPLY)), None);
    }

    #[test]
    fn a_session_queries_until_the_version_is_answered() {
        let mut session = Session::new("DM303_TX", "DMT03");
        assert_eq!(session.next_request(), Some(version_query(0x1001)));
        assert_eq!(session.feed(&hex(DEVICE_INFO)), None);
        assert_eq!(session.next_request(), Some(version_query(0x1002)));
        assert!(session.feed(&hex(VERSION_REPLY)).unwrap().is_ok());
        assert_eq!(session.next_request(), None, "answered");

        let mut silent = Session::new("DM303_TX", "DMT03");
        let sent = std::iter::from_fn(|| silent.next_request()).count();
        assert_eq!(sent, usize::from(MAX_QUERIES));
    }

    #[test]
    fn the_session_verifies_against_its_catalog_model() {
        // The same transmitter checked as if the catalog expected another
        // model is rejected, never relabelled.
        let mut session = Session::new("DM303_RX", "DMR03");
        session.feed(&hex(DEVICE_INFO));
        assert_eq!(
            session.feed(&hex(VERSION_REPLY)),
            Some(Err(Rejection::UnknownHardware("DM303_TX".into())))
        );
    }
}
