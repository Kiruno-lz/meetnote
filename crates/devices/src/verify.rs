//! The protocol-independent contract between the Bluetooth transport and a
//! model's identity verification. The transport connects, subscribes to the
//! profile's notify characteristic, writes whatever the session asks for and
//! feeds it every notification; the session alone decides what is sent and
//! whether the device is the model the catalog says it is.

use serde::{Deserialize, Serialize};

use crate::catalog::{Model, Verifier};
use crate::duml;

/// GATT service and characteristics, as CoreBluetooth spells their UUIDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GattProfile {
    pub service: &'static str,
    /// Device → app notifications.
    pub notify: &'static str,
    /// App → device, written without response.
    pub write: &'static str,
}

/// One verification attempt over one connection.
pub trait GattSession {
    /// The next request to write, if one is due. Asked once notifications
    /// are enabled and again after each notification without an outcome.
    fn next_request(&mut self) -> Option<Vec<u8>>;
    /// Feeds one notification; the outcome once it is known.
    fn feed(&mut self, notification: &[u8]) -> Option<Result<VerifiedDevice, Rejection>>;
}

/// What a verified device reports about itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedDevice {
    /// The hardware revision exactly as the device reports it.
    pub hardware: String,
    /// The catalog model's code.
    pub model_code: String,
    pub serial: String,
    pub firmware: String,
    /// Read from the standard field position only; not cross-checked.
    pub loader: Option<String>,
    pub name_suffix: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rejection {
    /// The device reports hardware other than the catalog model's.
    UnknownHardware(String),
    /// Two replies disagree on the firmware.
    FirmwareMismatch { info: String, version: String },
    /// The serial number is not printable ASCII.
    InvalidSerial,
}

impl std::fmt::Display for Rejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownHardware(hardware) => {
                write!(f, "unsupported hardware version {hardware:?}")
            }
            Self::FirmwareMismatch { info, version } => write!(
                f,
                "firmware differs between device info ({info}) and version reply ({version})"
            ),
            Self::InvalidSerial => f.write_str("the serial number is not readable"),
        }
    }
}

/// The GATT profile and a fresh session verifying `model`.
pub fn session(model: &'static Model) -> (GattProfile, Box<dyn GattSession>) {
    match model.verifier {
        Verifier::DjiDuml { hardware } => (
            duml::GATT,
            Box::new(duml::Session::new(hardware, model.model_code)),
        ),
    }
}

/// Whether `serial` is a usable serial number.
pub(crate) fn readable_serial(serial: &[u8]) -> Option<String> {
    let serial = std::str::from_utf8(serial).ok()?;
    (!serial.is_empty() && serial.bytes().all(|byte| byte.is_ascii_graphic()))
        .then(|| serial.to_owned())
}
