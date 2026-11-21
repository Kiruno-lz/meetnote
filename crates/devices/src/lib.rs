//! Recording devices as audio sources: Bluetooth discovery, USB attachment,
//! the paired-device registry and verified copies of their recordings.
//!
//! Only models in the [`catalog`] whitelist are recognised; each names how it
//! is verified ([`verify`]). Recognition, parsing and import planning are
//! pure functions over captured data. Platform IO stays in [`ble`] (CoreBluetooth) and [`usb`] (`ioreg` and
//! `mount`), so every rule can be checked against recorded fixtures.

pub mod ble;
pub mod catalog;
pub mod discovery;
pub mod duml;
pub mod identity;
pub mod ingest;
pub mod rate;
pub mod registry;
pub mod usb;
pub mod verify;

#[cfg(target_os = "macos")]
mod macos;

pub use ble::{Advertisement, BleCandidate, BleEvent, BleScan, BluetoothBlock, BluetoothState};
pub use discovery::{DeviceDiscovery, SystemDiscovery, UsbWatcher};
pub use identity::DeviceIdentity;
pub use registry::{DeviceRegistry, ImportSummary, ImportedFile, PairedDevice, RegistryStore};
pub use usb::{AttachedDevice, UsbDevice};
