//! The whitelist of supported recorders. A device MeetNote has no entry for
//! is never connected over Bluetooth, never offered for pairing and never
//! imported from, even when it comes from a known manufacturer.
//!
//! Each entry gathers everything model-specific: how it advertises, how it
//! enumerates on USB, which protocol verifies its identity and how it lays
//! out recordings. Supporting another model of a known protocol is one more
//! entry; a new protocol is one more [`Verifier`] variant and its session in
//! [`crate::verify`].

use crate::usb::UsbDevice;

/// One supported recorder model.
#[derive(Debug, PartialEq, Eq)]
pub struct Model {
    /// Stable identifier, never shown.
    pub id: &'static str,
    /// Display name without a per-unit suffix.
    pub name: &'static str,
    /// The manufacturer's model code; unique in the catalog, it joins a
    /// verified Bluetooth identity with a USB attachment.
    pub model_code: &'static str,
    pub advertising: Advertising,
    /// USB descriptors the model enumerates with; any one matches.
    pub usb: &'static [UsbMatch],
    pub verifier: Verifier,
    pub recordings: Recordings,
}

/// What selects the model from Bluetooth LE advertisements; either field
/// matching is enough, since devices split them across packets.
#[derive(Debug, PartialEq, Eq)]
pub struct Advertising {
    /// The local name starts with this.
    pub name_prefix: &'static str,
    /// Manufacturer data, including its little-endian company ID, starts with
    /// these bytes.
    pub manufacturer_prefix: &'static [u8],
    pub mac: MacSource,
}

/// Where an advertisement carries the device's Bluetooth address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MacSource {
    /// The last six bytes of the manufacturer data, in transmission order.
    ManufacturerTail,
    Absent,
}

#[derive(Debug, PartialEq, Eq)]
pub struct UsbMatch {
    pub vendor_id: u16,
    pub product_id: u16,
    /// A lower-case fragment of the USB product name; guards against other
    /// products that share a chip vendor's IDs.
    pub product_fragment: &'static str,
}

/// How a model proves its identity over LE GATT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verifier {
    /// DJI DUML on `FFF0`: the version reply must report exactly `hardware`
    /// (see [`crate::duml`]).
    DjiDuml { hardware: &'static str },
}

/// Which recording files on the device are imported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Recordings {
    #[default]
    All,
    /// Each take is written as `…_orig` and a processed `…_edit`; only the
    /// original is imported, unless it is missing.
    OriginalsOverEdits,
}

pub static DJI_MIC3_TX: Model = Model {
    id: "dji-mic3-tx",
    name: "DJI Mic 3 TX",
    model_code: "DMT03",
    advertising: Advertising {
        name_prefix: "DJI Mic 3 TX",
        // DJI's company ID 0x08AA, then the product code `b20000`.
        manufacturer_prefix: &[0xAA, 0x08, 0xB2, 0x00, 0x00],
        mac: MacSource::ManufacturerTail,
    },
    // The transmitter enumerates under its Bestechnic chip's IDs, with a
    // serial number shared by every unit.
    usb: &[UsbMatch {
        vendor_id: 0xBE57,
        product_id: 0x0104,
        product_fragment: "mic tx",
    }],
    verifier: Verifier::DjiDuml {
        hardware: "DM303_TX",
    },
    recordings: Recordings::OriginalsOverEdits,
};

/// Every supported model.
pub static MODELS: &[&Model] = &[&DJI_MIC3_TX];

/// The model an advertisement packet belongs to.
pub fn for_advertisement(
    name: Option<&str>,
    manufacturer_data: Option<&[u8]>,
) -> Option<&'static Model> {
    MODELS.iter().copied().find(|model| {
        let advertising = &model.advertising;
        name.is_some_and(|name| name.starts_with(advertising.name_prefix))
            || manufacturer_data
                .is_some_and(|data| data.starts_with(advertising.manufacturer_prefix))
    })
}

/// The model a USB device enumerates as.
pub fn for_usb(device: &UsbDevice) -> Option<&'static Model> {
    let product = device.product_name.as_deref()?.to_ascii_lowercase();
    MODELS.iter().copied().find(|model| {
        model.usb.iter().any(|usb| {
            device.vendor_id == usb.vendor_id
                && device.product_id == usb.product_id
                && product.contains(usb.product_fragment)
        })
    })
}

pub fn by_model_code(model_code: &str) -> Option<&'static Model> {
    MODELS
        .iter()
        .copied()
        .find(|model| model.model_code == model_code)
}

impl Model {
    /// The Bluetooth address in this model's manufacturer data.
    pub fn mac(&self, manufacturer_data: Option<&[u8]>) -> Option<String> {
        match self.advertising.mac {
            MacSource::ManufacturerTail => manufacturer_data
                .filter(|data| {
                    data.starts_with(self.advertising.manufacturer_prefix)
                        && data.len() >= self.advertising.manufacturer_prefix.len() + 6
                })
                .map(|data| crate::identity::format_mac(&data[data.len() - 6..])),
            MacSource::Absent => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_model_codes_are_unique() {
        for (index, model) in MODELS.iter().enumerate() {
            for other in &MODELS[index + 1..] {
                assert_ne!(model.id, other.id);
                assert_ne!(model.model_code, other.model_code);
            }
            assert!(!model.usb.is_empty(), "{} needs a USB match", model.id);
            assert!(
                model
                    .usb
                    .iter()
                    .all(|usb| usb.product_fragment == usb.product_fragment.to_lowercase()),
                "fragments compare against lower-cased names"
            );
        }
    }

    #[test]
    fn only_listed_advertisements_match() {
        let mic = Some(&DJI_MIC3_TX);
        assert_eq!(for_advertisement(Some("DJI Mic 3 TX-a73b42"), None), mic);
        let data = [
            0xAA, 0x08, 0xB2, 0x00, 0x00, 0x8C, 0x58, 0x23, 0xA7, 0x3B, 0x43,
        ];
        assert_eq!(for_advertisement(Some("BES_BLE"), Some(&data)), mic);
        assert_eq!(
            DJI_MIC3_TX.mac(Some(&data)).as_deref(),
            Some("8C:58:23:A7:3B:43")
        );
        // Other DJI products share the company ID but not the product code.
        let osmo = [0xAA, 0x08, 0xC1, 0x00, 0x00, 1, 2, 3, 4, 5, 6];
        assert_eq!(
            for_advertisement(Some("DJI Osmo Pocket 3"), Some(&osmo)),
            None
        );
        assert_eq!(for_advertisement(Some("DJI Mic 3 RX"), None), None);
        assert_eq!(DJI_MIC3_TX.mac(Some(&osmo)), None);
    }

    #[test]
    fn only_listed_usb_devices_match() {
        let mic = UsbDevice {
            vendor_id: 0xBE57,
            product_id: 0x0104,
            product_name: Some("Wireles Mic Tx".into()),
            ..Default::default()
        };
        assert_eq!(for_usb(&mic), Some(&DJI_MIC3_TX));
        let same_chip = UsbDevice {
            product_name: Some("BES Headset".into()),
            ..mic.clone()
        };
        assert_eq!(for_usb(&same_chip), None);
        let dji_vendor = UsbDevice {
            vendor_id: 0x2CA3,
            product_id: 0x0020,
            product_name: Some("DJI MIC3 DMT03".into()),
            ..Default::default()
        };
        assert_eq!(for_usb(&dji_vendor), None, "not on the list");
        assert_eq!(by_model_code("DMT03"), Some(&DJI_MIC3_TX));
        assert_eq!(by_model_code("DMT04"), None);
    }
}
