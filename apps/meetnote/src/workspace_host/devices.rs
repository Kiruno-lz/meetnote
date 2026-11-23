//! Recording devices: the paired list, USB arrival, Bluetooth search while
//! pairing, and the import that runs whenever a paired device is plugged in.
//! Every file and radio operation runs off the UI thread; the page only reads
//! this state and sends intents back.

use std::collections::{BTreeSet, VecDeque};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use application::device_import::{DeviceImport, DeviceImportProgress};
use devices::ble::{Sightings, Verification};
use devices::catalog::MacSource;
use devices::discovery::UsbUpdate;
use devices::rate::TransferRate;
use devices::{
    AttachedDevice, BleCandidate, BleEvent, BleScan, BluetoothBlock, BluetoothState,
    DeviceDiscovery, DeviceRegistry, PairedDevice, RegistryStore, UsbWatcher,
};
use domain::ProjectId;
use futures::Stream;
use futures::channel::mpsc::{self, UnboundedReceiver};
use rune::Tx;
use runtime::CancellationToken;

use super::processing::ProcessingRun;
use super::{Task, WorkspaceHost};
use crate::MeetNote;

const USB_INTERVAL: Duration = Duration::from_millis(1500);
/// How long a verified device may still go without its advertised address
/// before it is paired without one: the address arrives in packets of its
/// own, and none come while the GATT connection is open.
const MAC_GRACE: f64 = 5.0;

/// A preference change on one paired device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RegistryChange {
    Pair(Box<PairedDevice>),
    Remove(String),
    ImportDir(String, Option<PathBuf>),
    AutoProcess(String, bool),
    DeleteAfterImport(String, bool),
    /// The address of a device paired before its address was seen.
    FillMac(String, String),
}

impl RegistryChange {
    fn apply(&self, registry: &mut DeviceRegistry) {
        match self {
            Self::Pair(device) => registry.pair((**device).clone()),
            Self::Remove(key) => {
                registry.remove(key);
            }
            Self::ImportDir(key, directory) => {
                if let Some(device) = registry.device_mut(key) {
                    device.import_dir.clone_from(directory);
                }
            }
            Self::AutoProcess(key, enabled) => {
                if let Some(device) = registry.device_mut(key) {
                    device.auto_process = *enabled;
                }
            }
            Self::FillMac(key, mac) => {
                if let Some(device) = registry.device_mut(key)
                    && device.identity.mac.is_none()
                {
                    device.identity.mac = Some(mac.clone());
                }
            }
            Self::DeleteAfterImport(key, enabled) => {
                if let Some(device) = registry.device_mut(key) {
                    device.delete_after_import = *enabled;
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PairStep {
    /// Bluetooth verifies the device while USB waits for its cable.
    Connect,
    Done,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Radio {
    Starting,
    Scanning,
    Off,
    Unauthorized,
    /// Bluetooth cannot run in this process or on this system.
    Unavailable(String),
}

pub(crate) struct Pairing {
    pub step: PairStep,
    pub radio: Radio,
    _scan: Option<BleScan>,
    events: Option<UnboundedReceiver<BleEvent>>,
    sightings: Sightings,
    pub candidates: Vec<BleCandidate>,
    /// The peripheral the user picked when several verified.
    pub chosen: Option<String>,
    /// The device this dialog paired.
    pub paired: Option<String>,
    /// When the current step began, for its entrance motion.
    pub step_at: f64,
    /// Bluetooth settings were opened for this dialog already.
    settings_opened: bool,
    /// The target ready to pair but for its address, and since when.
    mac_wait: Option<(String, f64)>,
}

impl Pairing {
    /// The verified candidate to pair: the chosen one, else the only one.
    pub fn target(&self) -> Option<&BleCandidate> {
        let mut verified = self
            .candidates
            .iter()
            .filter(|candidate| candidate.verified().is_some());
        if let Some(chosen) = &self.chosen {
            return self.candidates.iter().find(|candidate| {
                &candidate.peripheral == chosen && candidate.verified().is_some()
            });
        }
        let first = verified.next()?;
        verified.next().is_none().then_some(first)
    }

    /// Several devices verified and none chosen.
    pub fn needs_choice(&self) -> bool {
        self.target().is_none()
            && self
                .candidates
                .iter()
                .filter(|candidate| candidate.verified().is_some())
                .count()
                > 1
    }

    fn poll(&mut self, cx: &mut Context<'_>, tx: &mut Tx) -> bool {
        let Some(events) = &mut self.events else {
            return false;
        };
        let mut changed = false;
        for _ in 0..64 {
            let verification = match Pin::new(&mut *events).poll_next(cx) {
                Poll::Ready(Some(BleEvent::State(state))) => {
                    let radio = match state {
                        BluetoothState::PoweredOn => Radio::Scanning,
                        BluetoothState::PoweredOff => Radio::Off,
                        BluetoothState::Unauthorized => Radio::Unauthorized,
                        BluetoothState::Unsupported => {
                            Radio::Unavailable(BluetoothBlock::Unsupported.to_string())
                        }
                        BluetoothState::Starting
                        | BluetoothState::Resetting
                        | BluetoothState::Unknown => Radio::Starting,
                    };
                    if radio != Radio::Scanning {
                        self.sightings.clear();
                        self.candidates.clear();
                    }
                    changed |= radio != self.radio;
                    self.radio = radio;
                    continue;
                }
                Poll::Ready(Some(BleEvent::Seen(advertisement))) => {
                    if self.sightings.observe(advertisement) {
                        self.candidates = self.sightings.candidates();
                        changed = true;
                    }
                    continue;
                }
                Poll::Ready(Some(BleEvent::Identifying(peripheral))) => {
                    (peripheral, Verification::Reading)
                }
                Poll::Ready(Some(BleEvent::Identified {
                    peripheral,
                    outcome,
                })) => (
                    peripheral,
                    match outcome {
                        Ok(device) => Verification::Verified(device),
                        Err(reason) => Verification::Rejected(reason),
                    },
                ),
                Poll::Ready(None) => {
                    self.events = None;
                    return changed;
                }
                Poll::Pending => return changed,
            };
            let (peripheral, verification) = verification;
            if self.sightings.verify(&peripheral, verification) {
                self.candidates = self.sightings.candidates();
                changed = true;
            }
        }
        tx.wake_after(Duration::ZERO);
        changed
    }
}

pub(crate) struct ImportRun {
    pub key: String,
    pub progress: Option<DeviceImportProgress>,
    /// Copy throughput across the whole run.
    rate: TransferRate,
    started: Instant,
    /// Bytes of the files finished before the one being copied.
    finished_bytes: u64,
    updates: UnboundedReceiver<DeviceImportProgress>,
    cancellation: CancellationToken,
    completion: Task<(
        application::device_import::DeviceImportReport,
        DeviceRegistry,
    )>,
}

impl ImportRun {
    /// Takes one progress report made `at` into the run.
    fn observe(&mut self, progress: DeviceImportProgress, at: Duration) {
        if let DeviceImportProgress::Copying { index, copied, .. } = &progress {
            if let Some(DeviceImportProgress::Copying {
                index: previous,
                size,
                ..
            }) = &self.progress
                && previous != index
            {
                self.finished_bytes += size;
            }
            self.rate.record(at, self.finished_bytes + copied);
        }
        self.progress = Some(progress);
    }

    /// Bytes per second, once enough of the copy has been seen.
    pub fn speed(&self) -> Option<f64> {
        self.rate.bytes_per_second()
    }
}

impl Drop for ImportRun {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

#[derive(Default)]
pub(crate) struct DeviceHost {
    discovery: Option<Arc<dyn DeviceDiscovery>>,
    store: Option<RegistryStore>,
    pub registry: DeviceRegistry,
    pub error: Option<String>,
    load: Option<Task<DeviceRegistry>>,
    write: Option<Task<DeviceRegistry>>,
    pending: VecDeque<RegistryChange>,
    /// Changes the running write is saving: not on disk yet, so a registry
    /// read before it lands (an import's) still needs them.
    saving: Vec<RegistryChange>,
    /// Changes whose write landed while the import ran: its read back may
    /// predate them. Reapplying one already on disk changes nothing.
    landed: Vec<RegistryChange>,
    watcher: Option<UsbWatcher>,
    usb: Option<UnboundedReceiver<UsbUpdate>>,
    pub attached: Vec<AttachedDevice>,
    pub usb_error: Option<String>,
    pub pairing: Option<Pairing>,
    pub import: Option<ImportRun>,
    /// Attached devices already imported this connection; unplugging clears.
    synced: BTreeSet<String>,
    /// Imported documents waiting for automatic analysis.
    pub analysis: VecDeque<(ProjectId, u64)>,
    /// The paired device whose “Advanced” section is open.
    pub advanced: BTreeSet<String>,
}

impl DeviceHost {
    /// Starts watching USB and reading the registry. Without a store the list
    /// lives in memory only (the sample workspace).
    pub fn start(&mut self, discovery: Arc<dyn DeviceDiscovery>, store: Option<RegistryStore>) {
        let (sender, receiver) = mpsc::unbounded();
        self.watcher = Some(UsbWatcher::spawn(discovery.clone(), USB_INTERVAL, sender));
        self.usb = Some(receiver);
        self.discovery = Some(discovery);
        if let Some(store) = &store {
            let store = store.clone();
            self.load = Some(Box::pin(runtime::spawn_blocking(move || {
                store.load().map_err(|error| error.to_string())
            })));
        }
        self.store = store;
    }

    /// The USB attachment of a paired device, if it is plugged in.
    pub fn attached(&self, key: &str) -> Option<&AttachedDevice> {
        let device = self.registry.device(key)?;
        self.attached.iter().find(|attached| {
            let attachment = attached.key();
            attachment == device.key || device.usb_links.contains(&attachment)
        })
    }

    /// Recognised devices on USB that are not paired yet.
    pub fn unpaired(&self) -> impl Iterator<Item = &AttachedDevice> {
        self.attached
            .iter()
            .filter(|device| self.registry.linked(&device.key()).is_none())
    }

    pub fn change(&mut self, change: RegistryChange) {
        if let RegistryChange::Remove(key) = &change
            && let Some(device) = self.registry.device(key)
        {
            for link in &device.usb_links {
                self.synced.remove(link);
            }
        }
        change.apply(&mut self.registry);
        if let Some(key) = match &change {
            RegistryChange::Remove(key) => Some(key),
            _ => None,
        } {
            self.advanced.remove(key);
            if self.import.as_ref().is_some_and(|run| &run.key == key) {
                self.import = None;
                self.landed.clear();
            }
        }
        if self.store.is_some() {
            self.pending.push_back(change);
        }
    }

    pub fn open_pairing(&mut self, now: f64) {
        let mut pairing = Pairing {
            step: PairStep::Connect,
            radio: Radio::Starting,
            _scan: None,
            events: None,
            sightings: Sightings::default(),
            candidates: Vec::new(),
            chosen: None,
            paired: None,
            step_at: now,
            settings_opened: false,
            mac_wait: None,
        };
        if let Some(discovery) = &self.discovery {
            let (sender, receiver) = mpsc::unbounded();
            match discovery.start_bluetooth(sender) {
                Ok(scan) => {
                    pairing._scan = Some(scan);
                    pairing.events = Some(receiver);
                }
                Err(block) => pairing.radio = Radio::Unavailable(block.to_string()),
            }
        } else {
            pairing.radio = Radio::Unavailable(BluetoothBlock::Unsupported.to_string());
        }
        if let Some(watcher) = &self.watcher {
            watcher.set_eager(true);
        }
        self.pairing = Some(pairing);
    }

    /// Picks which verified device to pair when several are nearby.
    pub fn choose(&mut self, peripheral: String) {
        if let Some(pairing) = &mut self.pairing {
            pairing.chosen = Some(peripheral);
        }
    }

    pub fn open_bluetooth_settings(&self) {
        if let Some(discovery) = &self.discovery {
            discovery.open_bluetooth_settings();
        }
    }

    /// Denied Bluetooth access is only granted in System Settings: open it
    /// once per dialog, as soon as the denial is known.
    fn request_bluetooth_access(&mut self) {
        let Some(pairing) = &mut self.pairing else {
            return;
        };
        if pairing.radio == Radio::Unauthorized && !pairing.settings_opened {
            pairing.settings_opened = true;
            self.open_bluetooth_settings();
        }
    }

    pub fn close_pairing(&mut self) {
        self.pairing = None;
        if let Some(watcher) = &self.watcher {
            watcher.set_eager(false);
        }
    }

    /// Pairs the verified device with the unpaired USB attachment of the
    /// same model, keyed by its serial number. The attachment must already
    /// have a stable key: the cable enumerates the device before its volume
    /// is mounted and read (longer while macOS asks for volume access), and
    /// a link to the USB port would not match the volume that follows.
    fn pair_attached(&mut self, now: f64) -> Option<String> {
        self.fill_missing_macs();
        let pairing = self.pairing.as_ref()?;
        if pairing.step != PairStep::Connect {
            return None;
        }
        let target = pairing.target()?.clone();
        let verified = target.verified()?.clone();
        let attached = self
            .unpaired()
            .find(|attached| {
                attached.identity.model_code.as_deref() == Some(verified.model_code.as_str())
                    && attached.stable_key().is_some()
            })?
            .clone();
        if target.identity.mac.is_none() && target.model.advertising.mac != MacSource::Absent {
            let pairing = self.pairing.as_mut()?;
            let since = match &pairing.mac_wait {
                Some((peripheral, since)) if *peripheral == target.peripheral => *since,
                _ => {
                    pairing.mac_wait = Some((target.peripheral.clone(), now));
                    now
                }
            };
            if now - since < MAC_GRACE {
                return None;
            }
        }
        let mut identity = target.identity.clone();
        identity.merge(&attached.identity);
        let key = format!("sn:{}", verified.serial);
        let mut device = PairedDevice::new(key.clone(), identity, now_ms());
        device.usb_links = vec![attached.key()];
        self.change(RegistryChange::Pair(Box::new(device)));
        let pairing = self.pairing.as_mut()?;
        pairing.step = PairStep::Done;
        pairing.paired = Some(key.clone());
        pairing.step_at = now;
        // The radio is no longer needed once the device is paired.
        pairing._scan = None;
        pairing.events = None;
        if let Some(watcher) = &self.watcher {
            watcher.set_eager(false);
        }
        Some(key)
    }

    /// Seconds until a target waiting for its address pairs anyway.
    pub fn mac_wait_left(&self, now: f64) -> Option<f64> {
        let (_, since) = self.pairing.as_ref()?.mac_wait.as_ref()?;
        Some((since + MAC_GRACE - now).max(0.0))
    }

    /// Records addresses seen for devices paired without one.
    fn fill_missing_macs(&mut self) {
        let Some(pairing) = &self.pairing else {
            return;
        };
        let found: Vec<(String, String)> = pairing
            .candidates
            .iter()
            .filter_map(|candidate| {
                let key = format!("sn:{}", candidate.verified()?.serial);
                let mac = candidate.identity.mac.clone()?;
                self.registry
                    .device(&key)
                    .is_some_and(|device| device.identity.mac.is_none())
                    .then_some((key, mac))
            })
            .collect();
        for (key, mac) in found {
            self.change(RegistryChange::FillMac(key, mac));
        }
    }

    fn poll_usb(&mut self, cx: &mut Context<'_>) -> bool {
        let Some(usb) = &mut self.usb else {
            return false;
        };
        let mut latest = None;
        loop {
            match Pin::new(&mut *usb).poll_next(cx) {
                Poll::Ready(Some(update)) => latest = Some(update),
                Poll::Ready(None) => {
                    self.usb = None;
                    break;
                }
                Poll::Pending => break,
            }
        }
        match latest {
            Some(UsbUpdate::Attached(attached)) => {
                let present: BTreeSet<String> = attached.iter().map(AttachedDevice::key).collect();
                self.synced.retain(|key| present.contains(key));
                self.attached = attached;
                self.usb_error = None;
                true
            }
            Some(UsbUpdate::Failed(error)) => {
                let changed = self.usb_error.as_ref() != Some(&error);
                self.usb_error = Some(error);
                changed
            }
            None => false,
        }
    }

    /// The running write saved `registry`.
    fn write_landed(&mut self, mut registry: DeviceRegistry) {
        if self.import.is_some() {
            self.landed.append(&mut self.saving);
        } else {
            self.saving.clear();
        }
        for change in &self.pending {
            change.apply(&mut registry);
        }
        self.registry = registry;
    }

    /// The running import ended; `registry` is what it read back after.
    fn import_finished(&mut self, registry: Option<DeviceRegistry>) {
        self.import = None;
        let landed = std::mem::take(&mut self.landed);
        if let Some(mut registry) = registry {
            for change in landed.iter().chain(&self.saving).chain(&self.pending) {
                change.apply(&mut registry);
            }
            self.registry = registry;
        }
    }

    fn poll_registry(&mut self, waker: Option<&Waker>, tx: &mut Tx) -> Result<bool, String> {
        let mut changed = false;
        if let Some(load) = &mut self.load
            && let Some(result) = WorkspaceHost::poll(load, waker, tx)
        {
            self.load = None;
            changed = true;
            let mut registry = result?;
            // Changes made before the file was read still apply.
            for change in &self.pending {
                change.apply(&mut registry);
            }
            self.registry = registry;
        }
        if let Some(write) = &mut self.write
            && let Some(result) = WorkspaceHost::poll(write, waker, tx)
        {
            self.write = None;
            changed = true;
            match result {
                Ok(registry) => self.write_landed(registry),
                Err(error) => {
                    self.saving.clear();
                    return Err(error);
                }
            }
        }
        if self.load.is_none()
            && self.write.is_none()
            && let Some(store) = self.store.clone()
            && !self.pending.is_empty()
        {
            let changes: Vec<_> = self.pending.drain(..).collect();
            self.saving.clone_from(&changes);
            self.write = Some(Box::pin(runtime::spawn_blocking(move || {
                store
                    .update(|registry| {
                        for change in &changes {
                            change.apply(registry);
                        }
                    })
                    .map(|(registry, ())| registry)
                    .map_err(|error| error.to_string())
            })));
            tx.wake_after(Duration::ZERO);
        }
        Ok(changed)
    }

    /// The next paired, mounted device with recordings not yet imported this
    /// connection: its attachment key, the device and its volumes.
    fn next_import(&self) -> Option<(String, PairedDevice, Vec<PathBuf>)> {
        if self.import.is_some() || self.load.is_some() {
            return None;
        }
        self.attached.iter().find_map(|attached| {
            let key = attached.key();
            if attached.volumes.is_empty() || self.synced.contains(&key) {
                return None;
            }
            let paired = self.registry.linked(&key)?;
            Some((key, paired.clone(), attached.volumes.clone()))
        })
    }

    pub fn import_fraction(&self) -> Option<f32> {
        match self.import.as_ref()?.progress.as_ref()? {
            DeviceImportProgress::Planned { .. } => None,
            DeviceImportProgress::Copying {
                index,
                total,
                copied,
                size,
                ..
            } => {
                let file = if *size == 0 {
                    1.0
                } else {
                    *copied as f32 / *size as f32
                };
                Some(((*index as f32 + file) / (*total).max(1) as f32).clamp(0.0, 1.0))
            }
        }
    }
}

impl MeetNote {
    pub(crate) fn start_device_host(&mut self, discovery: Arc<dyn DeviceDiscovery>) {
        let store =
            RegistryStore::beside_settings(self.workspace_host.processing_config.settings_path());
        self.workspace_host.devices.start(discovery, Some(store));
    }

    pub(crate) fn change_device(&mut self, change: RegistryChange, tx: &mut Tx) {
        self.workspace_host.devices.change(change);
        tx.wake_after(Duration::ZERO);
    }

    pub(crate) fn choose_import_dir(&mut self, key: String, tx: &mut Tx) {
        let current = self
            .workspace_host
            .devices
            .registry
            .device(&key)
            .and_then(|device| device.import_dir.clone());
        tx.open_folder(
            rune::FileDialogOptions {
                title: self.tr("device_import_dir_choose").into(),
                directory: current,
                ..Default::default()
            },
            move |s: &mut MeetNote, result, tx| match result {
                Ok(Some(path)) => match s.import_dir_problem(&path) {
                    Some(problem) => s.notify(problem, tx),
                    None => s.change_device(RegistryChange::ImportDir(key.clone(), Some(path)), tx),
                },
                Ok(None) => {}
                Err(error) => s.notify(error.to_string(), tx),
            },
        );
    }

    /// A custom import directory inside the workspace would read as an
    /// invalid package; one on the device would copy recordings onto itself.
    fn import_dir_problem(&self, path: &std::path::Path) -> Option<&'static str> {
        let inside_workspace = self
            .workspace_host
            .live
            .as_ref()
            .is_some_and(|live| path.starts_with(live.root()));
        let on_device = self
            .workspace_host
            .devices
            .attached
            .iter()
            .flat_map(|device| &device.volumes)
            .any(|volume| path.starts_with(volume));
        if inside_workspace {
            Some(self.tr("device_import_dir_in_workspace"))
        } else if on_device {
            Some(self.tr("device_import_dir_on_device"))
        } else {
            None
        }
    }

    pub(super) fn poll_devices(&mut self, tx: &mut Tx) -> bool {
        let host = &mut self.workspace_host;
        let waker = host.waker.clone();
        let mut cx = Context::from_waker(waker.as_ref().unwrap_or_else(|| Waker::noop()));
        let mut changed = host.devices.poll_usb(&mut cx);
        if let Some(pairing) = &mut host.devices.pairing {
            changed |= pairing.poll(&mut cx, tx);
        }
        host.devices.request_bluetooth_access();
        if host.devices.watcher.is_some() && waker.is_none() {
            // Tests and headless runs have no window waker: keep polling.
            tx.wake_after(Duration::from_millis(50));
        }
        match host.devices.poll_registry(waker.as_ref(), tx) {
            Ok(registry) => changed |= registry,
            Err(error) => {
                host.devices.error = Some(error.clone());
                self.notify(error, tx);
                changed = true;
            }
        }
        if self
            .workspace_host
            .devices
            .pair_attached(self.now)
            .is_some()
        {
            self.notify(self.tr("device_paired"), tx);
            changed = true;
        }
        if let Some(left) = self.workspace_host.devices.mac_wait_left(self.now) {
            tx.wake_after(Duration::from_secs_f64(left));
        }
        changed |= self.poll_device_import(tx);
        self.start_device_import(tx);
        self.start_device_analysis(tx);
        changed
    }

    fn start_device_import(&mut self, tx: &mut Tx) {
        let Some(live) = self.workspace_host.live.clone() else {
            return;
        };
        let host = &mut self.workspace_host.devices;
        let Some(store) = host.store.clone() else {
            return;
        };
        let Some((attachment, device, volumes)) = host.next_import() else {
            return;
        };
        host.synced.insert(attachment);
        let (sender, updates) = mpsc::unbounded();
        let cancellation = CancellationToken::new();
        let token = cancellation.clone();
        let key = device.key.clone();
        host.import = Some(ImportRun {
            key,
            progress: None,
            rate: TransferRate::default(),
            started: Instant::now(),
            finished_bytes: 0,
            updates,
            cancellation,
            completion: Box::pin(runtime::spawn_blocking(move || {
                let report = DeviceImport {
                    documents: &live.documents,
                    workspace_root: &live.root,
                    registry: &store,
                    device: &device,
                    volumes: &volumes,
                }
                .run(
                    &mut |progress| {
                        let _ = sender.unbounded_send(progress);
                    },
                    &token,
                )?;
                let registry = store.load().map_err(|error| error.to_string())?;
                Ok((report, registry))
            })),
        });
        tx.wake_after(Duration::ZERO);
    }

    fn poll_device_import(&mut self, tx: &mut Tx) -> bool {
        let host = &mut self.workspace_host;
        let Some(run) = &mut host.devices.import else {
            return false;
        };
        let mut changed = false;
        let mut cx = Context::from_waker(host.waker.as_ref().unwrap_or_else(|| Waker::noop()));
        while let Poll::Ready(Some(progress)) = Pin::new(&mut run.updates).poll_next(&mut cx) {
            let at = run.started.elapsed();
            run.observe(progress, at);
            changed = true;
        }
        let Some(result) = WorkspaceHost::poll(&mut run.completion, host.waker.as_ref(), tx) else {
            return changed;
        };
        let key = run.key.clone();
        let (result, registry) = match result {
            Ok((report, registry)) => (Ok(report), Some(registry)),
            Err(error) => (Err(error), None),
        };
        host.devices.import_finished(registry);
        match result {
            Ok(report) => {
                let auto_process = host
                    .devices
                    .registry
                    .device(&key)
                    .is_some_and(|device| device.auto_process);
                if auto_process {
                    host.devices.analysis.extend(
                        report
                            .imported
                            .iter()
                            .map(|state| (state.id().clone(), state.revision())),
                    );
                }
                if let Some(watch) = &mut host.watch {
                    watch.refresh_due.get_or_insert(tx.clock());
                }
                let imported = report.imported.len();
                if !report.failed.is_empty() {
                    let (name, reason) = &report.failed[0];
                    self.notify(
                        format!(
                            "{} {name}: {reason}",
                            ui_text::text(self.locale(), "device_import_failed")
                        ),
                        tx,
                    );
                }
                if imported > 0 {
                    self.notify(
                        ui_text::text(self.locale(), "device_imported")
                            .replace("{count}", &imported.to_string()),
                        tx,
                    );
                }
            }
            Err(error) => self.notify(error, tx),
        }
        tx.wake_after(Duration::ZERO);
        true
    }

    fn start_device_analysis(&mut self, tx: &mut Tx) {
        let locale = self.locale();
        let host = &mut self.workspace_host;
        if host.processing.is_some() || host.task.is_some() {
            return;
        }
        let Some(live) = host.live.clone() else {
            return;
        };
        let Some((id, revision)) = host.devices.analysis.pop_front() else {
            return;
        };
        host.processing = Some(ProcessingRun::start(
            host.processing_config.clone(),
            live,
            id,
            revision,
            true,
            locale,
        ));
        tx.wake_after(Duration::ZERO);
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

/// Scenes for the headless snapshots and tests: the user's DJI Mic 3 TX.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PairScene {
    /// Found nearby, identity being read.
    Reading,
    /// Verified, waiting for the cable.
    Verified,
    Done,
}

pub(crate) fn sample_identity() -> devices::DeviceIdentity {
    devices::DeviceIdentity {
        name: "DJI Mic 3 TX".into(),
        model_code: Some("DMT03".into()),
        serial: Some("9SFXN8N024XU81".into()),
        mac: Some("8C:58:23:A7:3B:43".into()),
        firmware: Some("03.01.00.10".into()),
        bluetooth_name: Some("DJI Mic 3 TX-a73b42".into()),
    }
}

pub(crate) fn sample_verified() -> devices::verify::VerifiedDevice {
    devices::verify::VerifiedDevice {
        hardware: "DM303_TX".into(),
        model_code: "DMT03".into(),
        serial: "9SFXN8N024XU81".into(),
        firmware: "03.01.00.10".into(),
        loader: Some("99.25.05.16".into()),
        name_suffix: Some("a73b42".into()),
    }
}

/// The DJI Mic 3 TX on USB as the real unit reports itself.
pub(crate) fn sample_attachment(volume: PathBuf) -> AttachedDevice {
    AttachedDevice {
        usb: devices::UsbDevice {
            vendor_id: 0xBE57,
            product_id: 0x0104,
            vendor_name: Some("bestechnic".into()),
            product_name: Some("Wireles Mic Tx".into()),
            serial: Some("20151006.1".into()),
            bcd_device: Some(0x0100),
            location_id: Some(0x0110_0000),
            bsd_names: vec!["disk4".into()],
        },
        identity: devices::DeviceIdentity {
            name: "DJI Mic 3 TX".into(),
            model_code: Some("DMT03".into()),
            ..Default::default()
        },
        volumes: vec![volume],
        volume_uuid: Some("645BECBB-3236-3D95-9BAC-7E99F3B01EED".into()),
    }
}

impl DeviceHost {
    pub fn sample_pairing(&mut self, scene: PairScene, now: f64) {
        self.open_pairing(now);
        let Some(pairing) = &mut self.pairing else {
            return;
        };
        pairing.radio = Radio::Scanning;
        let verification = match scene {
            PairScene::Reading => Verification::Reading,
            PairScene::Verified | PairScene::Done => Verification::Verified(sample_verified()),
        };
        let identity = match scene {
            PairScene::Reading => devices::DeviceIdentity {
                serial: None,
                model_code: None,
                firmware: None,
                ..sample_identity()
            },
            _ => sample_identity(),
        };
        pairing.candidates = vec![BleCandidate {
            peripheral: "sample".into(),
            model: &devices::catalog::DJI_MIC3_TX,
            identity,
            rssi: -52,
            verification,
        }];
        if scene == PairScene::Done {
            let mut device = PairedDevice::new("sn:9SFXN8N024XU81".into(), sample_identity(), 0);
            device.usb_links = vec!["vol:645BECBB-3236-3D95-9BAC-7E99F3B01EED".into()];
            self.change(RegistryChange::Pair(Box::new(device)));
            if let Some(pairing) = &mut self.pairing {
                pairing.step = PairStep::Done;
                pairing.paired = Some("sn:9SFXN8N024XU81".into());
            }
        }
    }
}

impl MeetNote {
    /// Two paired devices: the user's Mic 3 connected and importing, and a
    /// second one offline with a custom import directory.
    pub(crate) fn show_sample_devices(&mut self, importing: bool) {
        let now = now_ms();
        let host = &mut self.workspace_host.devices;
        let mut mic = PairedDevice::new("sn:9SFXN8N024XU81".into(), sample_identity(), now);
        mic.usb_links = vec!["vol:645BECBB-3236-3D95-9BAC-7E99F3B01EED".into()];
        mic.last_import = Some(devices::ImportSummary {
            finished_at_ms: now.saturating_sub(2 * 3_600_000),
            imported: 4,
            failed: 0,
        });
        let mut second = PairedDevice::new(
            "sn:9SFXN8N031KD27".into(),
            devices::DeviceIdentity {
                serial: Some("9SFXN8N031KD27".into()),
                mac: Some("8C:58:23:A7:51:0E".into()),
                bluetooth_name: Some("DJI Mic 3 TX-a7510d".into()),
                ..sample_identity()
            },
            now,
        );
        second.import_dir = Some("/Users/Shared/Recordings/Interviews".into());
        second.auto_process = false;
        second.last_import = Some(devices::ImportSummary {
            finished_at_ms: now.saturating_sub(3 * 86_400_000),
            imported: 2,
            failed: 0,
        });
        host.registry.devices = vec![mic, second];
        host.advanced.insert("sn:9SFXN8N024XU81".into());
        host.attached = vec![sample_attachment("/Volumes/NO NAME".into())];
        if importing {
            let (_sender, updates) = mpsc::unbounded();
            let mut run = ImportRun {
                key: "sn:9SFXN8N024XU81".into(),
                progress: None,
                rate: TransferRate::default(),
                started: Instant::now(),
                finished_bytes: 0,
                updates,
                cancellation: CancellationToken::new(),
                completion: Box::pin(futures::future::pending()),
            };
            // Two reports a second apart at 38.4 MB/s.
            for (at, copied) in [(0, 0), (1_000, 38_400_000)] {
                run.observe(
                    DeviceImportProgress::Copying {
                        index: 2,
                        total: 5,
                        name: "DJI_20261120_093012_14.WAV".into(),
                        copied,
                        size: 92_000_000,
                    },
                    Duration::from_millis(at),
                );
            }
            host.import = Some(run);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run() -> ImportRun {
        let (_sender, updates) = mpsc::unbounded();
        ImportRun {
            key: "sn:A".into(),
            progress: None,
            rate: TransferRate::default(),
            started: Instant::now(),
            finished_bytes: 0,
            updates,
            cancellation: CancellationToken::new(),
            completion: Box::pin(futures::future::pending()),
        }
    }

    fn copying(index: usize, copied: u64, size: u64) -> DeviceImportProgress {
        DeviceImportProgress::Copying {
            index,
            total: 2,
            name: format!("{index}.wav"),
            copied,
            size,
        }
    }

    #[test]
    fn speed_counts_bytes_across_file_boundaries() {
        let mut run = run();
        run.observe(DeviceImportProgress::Planned { total: 2 }, Duration::ZERO);
        run.observe(copying(0, 0, 4_000_000), Duration::ZERO);
        assert_eq!(run.speed(), None);
        run.observe(copying(0, 4_000_000, 4_000_000), Duration::from_millis(500));
        // The next file restarts its own count; the run's total does not.
        run.observe(
            copying(1, 1_000_000, 9_000_000),
            Duration::from_millis(1_000),
        );
        assert_eq!(run.speed(), Some(5_000_000.0));
    }

    #[test]
    fn import_read_before_a_write_landed_keeps_that_write() {
        let mut device = PairedDevice::new("sn:A".into(), devices::DeviceIdentity::default(), 0);
        device.usb_links = vec!["vol:A".into()];
        let mut host = DeviceHost::default();
        // The import reads the registry while pairing is still being saved.
        let read_by_import = host.registry.clone();
        host.saving = vec![RegistryChange::Pair(Box::new(device.clone()))];
        host.import = Some(run());
        // The write lands and is polled before the import result.
        let mut saved = DeviceRegistry::default();
        saved.pair(device);
        host.write_landed(saved);
        host.import_finished(Some(read_by_import));
        assert_eq!(
            host.registry.linked("vol:A").map(|d| d.key.as_str()),
            Some("sn:A")
        );
        // The next import reads after that write, so nothing is replayed.
        host.import = Some(run());
        host.import_finished(Some(DeviceRegistry::default()));
        assert!(host.registry.devices.is_empty());
    }
}
