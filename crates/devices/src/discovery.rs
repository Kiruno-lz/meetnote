//! Where devices come from: the system (`ioreg`, `mount`, CoreBluetooth) or,
//! in tests, a scripted source with the same contract.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use futures::channel::mpsc::UnboundedSender;

use crate::ble::{self, BleEvent, BleScan, BluetoothBlock};
use crate::usb::{self, AttachedDevice};

pub trait DeviceDiscovery: Send + Sync + 'static {
    /// A cheap fingerprint of the mounted volumes; USB descriptors are read
    /// again only when it changes.
    fn mount_signature(&self) -> Result<String, String>;
    /// Recognised recording devices on USB with their mounted volumes.
    fn attached(&self) -> Result<Vec<AttachedDevice>, String>;
    fn start_bluetooth(&self, events: UnboundedSender<BleEvent>)
    -> Result<BleScan, BluetoothBlock>;
    /// Shows the system's Bluetooth privacy settings, where access denied
    /// earlier is granted; macOS does not ask twice.
    fn open_bluetooth_settings(&self) {}
}

/// System Settings › Privacy & Security › Bluetooth.
pub const BLUETOOTH_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Bluetooth";

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemDiscovery;

impl DeviceDiscovery for SystemDiscovery {
    fn mount_signature(&self) -> Result<String, String> {
        usb::mount_table()
    }

    fn attached(&self) -> Result<Vec<AttachedDevice>, String> {
        let devices = usb::usb_devices()?;
        let mounts = usb::parse_mount(&usb::mount_table()?);
        let mut attached = usb::attached(devices, &mounts);
        for device in &mut attached {
            device.volume_uuid = device
                .volumes
                .first()
                .and_then(|volume| usb::volume_uuid(volume));
        }
        Ok(attached)
    }

    fn start_bluetooth(
        &self,
        events: UnboundedSender<BleEvent>,
    ) -> Result<BleScan, BluetoothBlock> {
        ble::start_scan(events)
    }

    fn open_bluetooth_settings(&self) {
        if cfg!(target_os = "macos") {
            // Fire and forget: `open` returns once Settings is asked to show.
            let _ = std::process::Command::new("/usr/bin/open")
                .arg(BLUETOOTH_SETTINGS_URL)
                .spawn();
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UsbUpdate {
    Attached(Vec<AttachedDevice>),
    Failed(String),
}

/// Polls for attached devices on a background thread and sends each change.
/// Dropping it stops the thread at its next wake.
pub struct UsbWatcher {
    stop: Arc<AtomicBool>,
    eager: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl UsbWatcher {
    pub fn spawn(
        discovery: Arc<dyn DeviceDiscovery>,
        interval: Duration,
        updates: UnboundedSender<UsbUpdate>,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let eager = Arc::new(AtomicBool::new(false));
        let thread = {
            let (stop, eager) = (stop.clone(), eager.clone());
            thread::Builder::new()
                .name("meetnote-usb-watch".into())
                .spawn(move || watch(&*discovery, interval, &updates, &stop, &eager))
                .ok()
        };
        Self {
            stop,
            eager,
            thread,
        }
    }

    /// While pairing, read USB descriptors on every tick: a device in
    /// pairing mode may enumerate without mounting a volume.
    pub fn set_eager(&self, eager: bool) {
        self.eager.store(eager, Ordering::Relaxed);
        if eager && let Some(thread) = &self.thread {
            thread.thread().unpark();
        }
    }
}

impl Drop for UsbWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = &self.thread {
            thread.thread().unpark();
        }
    }
}

fn watch(
    discovery: &dyn DeviceDiscovery,
    interval: Duration,
    updates: &UnboundedSender<UsbUpdate>,
    stop: &AtomicBool,
    eager: &AtomicBool,
) {
    let mut signature: Option<String> = None;
    let mut last: Option<UsbUpdate> = None;
    while !stop.load(Ordering::Relaxed) {
        let current = discovery.mount_signature();
        let changed = match (&current, &signature) {
            (Ok(current), Some(previous)) => current != previous,
            _ => true,
        };
        if changed || eager.load(Ordering::Relaxed) {
            let update = match current.and_then(|current| {
                signature = Some(current);
                discovery.attached()
            }) {
                Ok(attached) => UsbUpdate::Attached(attached),
                Err(error) => {
                    signature = None;
                    UsbUpdate::Failed(error)
                }
            };
            if last.as_ref() != Some(&update) {
                if updates.unbounded_send(update.clone()).is_err() {
                    return;
                }
                last = Some(update);
            }
        }
        thread::park_timeout(interval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::DeviceIdentity;
    use crate::usb::UsbDevice;
    use futures::StreamExt;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    #[derive(Default)]
    struct Scripted {
        mounts: Mutex<String>,
        attached: Mutex<Vec<AttachedDevice>>,
        reads: AtomicUsize,
    }

    impl DeviceDiscovery for Scripted {
        fn mount_signature(&self) -> Result<String, String> {
            Ok(self.mounts.lock().unwrap().clone())
        }
        fn attached(&self) -> Result<Vec<AttachedDevice>, String> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            Ok(self.attached.lock().unwrap().clone())
        }
        fn start_bluetooth(&self, _: UnboundedSender<BleEvent>) -> Result<BleScan, BluetoothBlock> {
            Err(BluetoothBlock::Unsupported)
        }
    }

    fn dji() -> AttachedDevice {
        AttachedDevice {
            usb: UsbDevice::default(),
            identity: DeviceIdentity {
                name: "DJI MIC3".into(),
                serial: Some("9SFXN8N024XU81".into()),
                ..Default::default()
            },
            volumes: vec!["/Volumes/DJI MIC3".into()],
            volume_uuid: None,
        }
    }

    #[test]
    fn sends_the_first_state_then_only_changes_and_reads_usb_only_on_mount_changes() {
        let discovery = Arc::new(Scripted::default());
        let (sender, mut receiver) = futures::channel::mpsc::unbounded();
        let watcher = UsbWatcher::spawn(discovery.clone(), Duration::from_millis(5), sender);
        let first = futures::executor::block_on(receiver.next()).unwrap();
        assert_eq!(first, UsbUpdate::Attached(Vec::new()));
        thread::sleep(Duration::from_millis(40));
        assert_eq!(
            discovery.reads.load(Ordering::SeqCst),
            1,
            "mounts unchanged"
        );

        *discovery.attached.lock().unwrap() = vec![dji()];
        *discovery.mounts.lock().unwrap() = "/dev/disk4s1 on /Volumes/DJI MIC3 (msdos)".into();
        let second = futures::executor::block_on(receiver.next()).unwrap();
        assert_eq!(second, UsbUpdate::Attached(vec![dji()]));

        watcher.set_eager(true);
        thread::sleep(Duration::from_millis(40));
        assert!(
            discovery.reads.load(Ordering::SeqCst) > 3,
            "eager reads every tick"
        );
        assert!(
            receiver.try_recv().is_err(),
            "no duplicate update while unchanged"
        );
        drop(watcher);
    }
}
