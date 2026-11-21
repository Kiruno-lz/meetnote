use serde::{Deserialize, Serialize};

/// What MeetNote knows about one physical device. Bluetooth and USB each
/// reveal part of it; pairing merges both.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceIdentity {
    /// Display name without the per-unit suffix, e.g. `DJI Mic 3 TX`.
    pub name: String,
    /// Manufacturer model code, e.g. `DMT03`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    /// Bluetooth LE address, upper-case and colon separated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware: Option<String>,
    /// The advertised local name, e.g. `DJI Mic 3 TX-a73b42`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bluetooth_name: Option<String>,
}

impl DeviceIdentity {
    /// The stable registry key: the serial number, else the Bluetooth address.
    pub fn key(&self) -> Option<String> {
        self.serial
            .as_ref()
            .map(|serial| format!("sn:{serial}"))
            .or_else(|| self.mac.as_ref().map(|mac| format!("mac:{mac}")))
    }

    /// Fills the fields `self` lacks from `other`. `self` keeps its own name
    /// unless it has none.
    pub fn merge(&mut self, other: &DeviceIdentity) {
        if self.name.is_empty() {
            self.name.clone_from(&other.name);
        }
        for (mine, theirs) in [
            (&mut self.model_code, &other.model_code),
            (&mut self.serial, &other.serial),
            (&mut self.mac, &other.mac),
            (&mut self.firmware, &other.firmware),
            (&mut self.bluetooth_name, &other.bluetooth_name),
        ] {
            if mine.is_none() {
                mine.clone_from(theirs);
            }
        }
    }
}

/// `bytes` as `AA:BB:CC:DD:EE:FF`.
pub fn format_mac(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_prefers_the_serial_number_over_the_address() {
        let mut identity = DeviceIdentity {
            mac: Some("8C:58:23:A7:3B:43".into()),
            ..Default::default()
        };
        assert_eq!(identity.key().as_deref(), Some("mac:8C:58:23:A7:3B:43"));
        identity.serial = Some("9SFXN8N024XU81".into());
        assert_eq!(identity.key().as_deref(), Some("sn:9SFXN8N024XU81"));
        assert_eq!(DeviceIdentity::default().key(), None);
    }

    #[test]
    fn merge_fills_only_missing_fields() {
        let mut bluetooth = DeviceIdentity {
            name: "DJI Mic 3 TX".into(),
            mac: Some("8C:58:23:A7:3B:43".into()),
            ..Default::default()
        };
        bluetooth.merge(&DeviceIdentity {
            name: "DJI MIC3".into(),
            serial: Some("9SFXN8N024XU81".into()),
            model_code: Some("DMT03".into()),
            mac: Some("00:00:00:00:00:00".into()),
            ..Default::default()
        });
        assert_eq!(bluetooth.name, "DJI Mic 3 TX");
        assert_eq!(bluetooth.mac.as_deref(), Some("8C:58:23:A7:3B:43"));
        assert_eq!(bluetooth.serial.as_deref(), Some("9SFXN8N024XU81"));
        assert_eq!(bluetooth.model_code.as_deref(), Some("DMT03"));
    }

    #[test]
    fn formats_addresses() {
        assert_eq!(
            format_mac(&[0x8c, 0x58, 0x23, 0xa7, 0x3b, 0x43]),
            "8C:58:23:A7:3B:43"
        );
    }
}
