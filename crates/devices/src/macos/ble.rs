//! CoreBluetooth scanner and LE GATT identification.
//!
//! Every advertisement is reported. A peripheral of a whitelisted model
//! ([`crate::ble::model_to_verify`]) is connected over LE once: MeetNote
//! subscribes to the model's notify characteristic, writes what its
//! [`GattSession`] asks for, feeds it every notification and disconnects as
//! soon as the outcome is known. The protocol itself lives in the session. CoreBluetooth runs on a private serial dispatch queue, so neither
//! the app's main thread nor a CLI without a run loop has to pump events.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
use futures::channel::mpsc::UnboundedSender;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, DefinedClass, Message, define_class, msg_send};
use objc2_core_bluetooth::{
    CBAdvertisementDataLocalNameKey, CBAdvertisementDataManufacturerDataKey,
    CBAdvertisementDataServiceDataKey, CBAdvertisementDataServiceUUIDsKey, CBCentralManager,
    CBCentralManagerDelegate, CBCentralManagerScanOptionAllowDuplicatesKey, CBCharacteristic,
    CBCharacteristicWriteType, CBManager, CBManagerAuthorization, CBManagerState, CBPeripheral,
    CBPeripheralDelegate, CBService, CBUUID,
};
use objc2_foundation::{NSArray, NSBundle, NSData, NSDictionary, NSError, NSNumber, NSString};

use crate::ble::{
    Advertisement, BleEvent, BleScan, BluetoothBlock, BluetoothState, model_to_verify,
};
use crate::catalog::Model;
use crate::verify::{self, GattProfile, GattSession, VerifiedDevice};

/// Gives up on a connection that has not produced an outcome by then.
const IDENTIFY_TIMEOUT: Duration = Duration::from_secs(20);
/// Attempts per peripheral and scan, and the pause before retrying.
const MAX_ATTEMPTS: u8 = 3;
const RETRY_AFTER: Duration = Duration::from_secs(3);

/// Developer override: `MEETNOTE_BLUETOOTH=1` skips the app bundle check.
const OVERRIDE_ENV: &str = "MEETNOTE_BLUETOOTH";
const USAGE_KEY: &str = "NSBluetoothAlwaysUsageDescription";

/// Whether this process may touch CoreBluetooth. macOS terminates (TCC,
/// SIGABRT) a process whose responsible app declares no Bluetooth usage, so an
/// unbundled binary run from a terminal must never create a manager.
fn gate(override_on: bool, bundle_path: &str, has_usage: bool) -> Result<(), BluetoothBlock> {
    if override_on || (bundle_path.ends_with(".app") && has_usage) {
        Ok(())
    } else {
        Err(BluetoothBlock::NeedsAppBundle)
    }
}

fn map_state(state: CBManagerState) -> BluetoothState {
    match state {
        CBManagerState::PoweredOn => BluetoothState::PoweredOn,
        CBManagerState::PoweredOff => BluetoothState::PoweredOff,
        CBManagerState::Unauthorized => BluetoothState::Unauthorized,
        CBManagerState::Unsupported => BluetoothState::Unsupported,
        CBManagerState::Resetting => BluetoothState::Resetting,
        _ => BluetoothState::Unknown,
    }
}

/// One peripheral being identified, touched only on the scan's queue.
struct Link {
    peripheral: Retained<CBPeripheral>,
    /// The catalog model the advertisement matched; the session verifies it.
    model: &'static Model,
    profile: GattProfile,
    session: Box<dyn GattSession>,
    write: Option<Retained<CBCharacteristic>>,
    attempts: u8,
    started: Instant,
    /// When the last attempt ended without an outcome.
    failed_at: Option<Instant>,
    done: bool,
}

pub struct DelegateIvars {
    events: UnboundedSender<BleEvent>,
    /// Kept to cancel connections from peripheral callbacks; the manager
    /// holds this delegate weakly, so there is no cycle.
    central: RefCell<Option<Retained<CBCentralManager>>>,
    links: RefCell<HashMap<String, Link>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and the class
    // implements no Drop.
    #[unsafe(super(NSObject))]
    #[name = "MeetNoteBleScanDelegate"]
    #[ivars = DelegateIvars]
    struct ScanDelegate;

    unsafe impl NSObjectProtocol for ScanDelegate {}

    unsafe impl CBCentralManagerDelegate for ScanDelegate {
        #[unsafe(method(centralManagerDidUpdateState:))]
        fn did_update_state(&self, central: &CBCentralManager) {
            // SAFETY: called on the manager's queue, as CoreBluetooth requires.
            let state = unsafe { central.state() };
            if state == CBManagerState::PoweredOn {
                start_scanning(central);
            }
            *self.ivars().central.borrow_mut() = Some(central.retain());
            self.send(BleEvent::State(map_state(state)));
        }

        #[unsafe(method(centralManager:didDiscoverPeripheral:advertisementData:RSSI:))]
        fn did_discover(
            &self,
            central: &CBCentralManager,
            peripheral: &CBPeripheral,
            advertisement_data: &NSDictionary<NSString, AnyObject>,
            rssi: &NSNumber,
        ) {
            if self.ivars().events.is_closed() {
                return;
            }
            self.expire_links(central);
            let advertisement = read_advertisement(peripheral, advertisement_data, rssi);
            let model = model_to_verify(&advertisement);
            let id = advertisement.peripheral.clone();
            self.send(BleEvent::Seen(advertisement));
            if let Some(model) = model {
                self.identify(central, peripheral, id, model);
            }
        }

        #[unsafe(method(centralManager:didConnectPeripheral:))]
        fn did_connect(&self, _central: &CBCentralManager, peripheral: &CBPeripheral) {
            let Some(profile) = self.profile(peripheral) else {
                return;
            };
            let service = uuids(&[profile.service]);
            // SAFETY: on the manager's queue with a connected peripheral.
            unsafe { peripheral.discoverServices(Some(&service)) };
        }

        #[unsafe(method(centralManager:didFailToConnectPeripheral:error:))]
        fn did_fail_to_connect(
            &self,
            _central: &CBCentralManager,
            peripheral: &CBPeripheral,
            error: Option<&NSError>,
        ) {
            self.fail(peripheral, &describe("could not connect", error));
        }

        #[unsafe(method(centralManager:didDisconnectPeripheral:error:))]
        fn did_disconnect(
            &self,
            _central: &CBCentralManager,
            peripheral: &CBPeripheral,
            error: Option<&NSError>,
        ) {
            self.fail(
                peripheral,
                &describe("disconnected before identifying", error),
            );
        }
    }

    unsafe impl CBPeripheralDelegate for ScanDelegate {
        #[unsafe(method(peripheral:didDiscoverServices:))]
        fn did_discover_services(&self, peripheral: &CBPeripheral, error: Option<&NSError>) {
            let Some(profile) = self.profile(peripheral) else {
                return;
            };
            // SAFETY: on the manager's queue; services were just discovered.
            let service = unsafe { peripheral.services() }
                .into_iter()
                .flat_map(|services| services.to_vec())
                .find(|service| uuid_is(&*unsafe { service.UUID() }, profile.service));
            match service {
                Some(service) => {
                    let characteristics = uuids(&[profile.notify, profile.write]);
                    // SAFETY: the service belongs to this connected peripheral.
                    unsafe {
                        peripheral
                            .discoverCharacteristics_forService(Some(&characteristics), &service)
                    };
                }
                None => self.give_up(
                    peripheral,
                    &describe(&format!("no {} service", profile.service), error),
                ),
            }
        }

        #[unsafe(method(peripheral:didDiscoverCharacteristicsForService:error:))]
        fn did_discover_characteristics(
            &self,
            peripheral: &CBPeripheral,
            service: &CBService,
            error: Option<&NSError>,
        ) {
            let Some(profile) = self.profile(peripheral) else {
                return;
            };
            // SAFETY: on the manager's queue; the characteristics belong to
            // `service` of this connected peripheral.
            let characteristics: Vec<_> = unsafe { service.characteristics() }
                .map(|list| list.to_vec())
                .unwrap_or_default();
            let find = |uuid| {
                characteristics
                    .iter()
                    .find(|characteristic| uuid_is(&*unsafe { characteristic.UUID() }, uuid))
                    .cloned()
            };
            let (Some(notify), Some(write)) = (find(profile.notify), find(profile.write)) else {
                let missing = format!("{}/{} missing", profile.notify, profile.write);
                self.give_up(peripheral, &describe(&missing, error));
                return;
            };
            // SAFETY: as above.
            unsafe { peripheral.setNotifyValue_forCharacteristic(true, &notify) };
            if let Some(link) = self.ivars().links.borrow_mut().get_mut(&key(peripheral)) {
                link.write = Some(write);
                query(link);
            }
        }

        #[unsafe(method(peripheral:didUpdateValueForCharacteristic:error:))]
        fn did_update_value(
            &self,
            peripheral: &CBPeripheral,
            characteristic: &CBCharacteristic,
            _error: Option<&NSError>,
        ) {
            // SAFETY: on the manager's queue; reading the delivered value.
            let Some(value) = (unsafe { characteristic.value() }) else {
                return;
            };
            let id = key(peripheral);
            let outcome = {
                let mut links = self.ivars().links.borrow_mut();
                let Some(link) = links.get_mut(&id).filter(|link| !link.done) else {
                    return;
                };
                let outcome = link.session.feed(&value.to_vec());
                if outcome.is_none() {
                    query(link);
                }
                outcome
            };
            if let Some(outcome) = outcome {
                self.finish(
                    peripheral,
                    outcome.map_err(|rejection| rejection.to_string()),
                );
            }
        }
    }
);

impl ScanDelegate {
    fn new(events: UnboundedSender<BleEvent>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(DelegateIvars {
            events,
            central: RefCell::new(None),
            links: RefCell::new(HashMap::new()),
        });
        // SAFETY: NSObject's `init` on a freshly allocated instance.
        unsafe { msg_send![super(this), init] }
    }

    /// A dropped receiver means nobody listens any more; ignore the event.
    fn send(&self, event: BleEvent) {
        let _ = self.ivars().events.unbounded_send(event);
    }

    /// The GATT profile of the model `peripheral` is being verified as.
    fn profile(&self, peripheral: &CBPeripheral) -> Option<GattProfile> {
        self.ivars()
            .links
            .borrow()
            .get(&key(peripheral))
            .map(|link| link.profile)
    }

    /// Connects to `peripheral` to verify it as `model` unless it is
    /// identified, being identified, out of attempts or failed only moments
    /// ago.
    fn identify(
        &self,
        central: &CBCentralManager,
        peripheral: &CBPeripheral,
        id: String,
        model: &'static Model,
    ) {
        {
            let mut links = self.ivars().links.borrow_mut();
            match links.get_mut(&id) {
                Some(link) => {
                    let retry = !link.done
                        && link.attempts < MAX_ATTEMPTS
                        && link
                            .failed_at
                            .is_some_and(|failed| failed.elapsed() >= RETRY_AFTER);
                    if !retry {
                        return;
                    }
                    link.attempts += 1;
                    link.failed_at = None;
                    link.started = Instant::now();
                    (link.profile, link.session) = verify::session(link.model);
                    link.write = None;
                }
                None => {
                    let (profile, session) = verify::session(model);
                    links.insert(
                        id.clone(),
                        Link {
                            peripheral: peripheral.retain(),
                            model,
                            profile,
                            session,
                            write: None,
                            attempts: 1,
                            started: Instant::now(),
                            failed_at: None,
                            done: false,
                        },
                    );
                }
            }
        }
        // SAFETY: on the manager's queue; the delegate outlives the link (the
        // session clears it before releasing either). LE connection only.
        unsafe {
            peripheral.setDelegate(Some(ProtocolObject::from_ref(self)));
            central.connectPeripheral_options(peripheral, None);
        }
        self.send(BleEvent::Identifying(id));
    }

    /// Reports the outcome once and disconnects.
    fn finish(&self, peripheral: &CBPeripheral, outcome: Result<VerifiedDevice, String>) {
        let id = key(peripheral);
        {
            let mut links = self.ivars().links.borrow_mut();
            let Some(link) = links.get_mut(&id).filter(|link| !link.done) else {
                return;
            };
            link.done = true;
        }
        self.disconnect(peripheral);
        self.send(BleEvent::Identified {
            peripheral: id,
            outcome,
        });
    }

    /// An attempt ended without an outcome: retry later, or report failure
    /// after the last attempt.
    fn fail(&self, peripheral: &CBPeripheral, reason: &str) {
        let id = key(peripheral);
        let last = {
            let mut links = self.ivars().links.borrow_mut();
            let Some(link) = links
                .get_mut(&id)
                .filter(|link| !link.done && link.failed_at.is_none())
            else {
                return;
            };
            link.failed_at = Some(Instant::now());
            link.attempts >= MAX_ATTEMPTS
        };
        if last {
            self.finish(peripheral, Err(reason.to_owned()));
        }
    }

    /// A device without the expected GATT layout: no point retrying.
    fn give_up(&self, peripheral: &CBPeripheral, reason: &str) {
        self.finish(peripheral, Err(reason.to_owned()));
    }

    fn disconnect(&self, peripheral: &CBPeripheral) {
        if let Some(central) = self.ivars().central.borrow().as_ref() {
            // SAFETY: on the manager's queue; cancelling an idle or finished
            // connection is a no-op.
            unsafe { central.cancelPeripheralConnection(peripheral) };
        }
    }

    /// Ends attempts that ran past [`IDENTIFY_TIMEOUT`].
    fn expire_links(&self, central: &CBCentralManager) {
        let expired: Vec<Retained<CBPeripheral>> = self
            .ivars()
            .links
            .borrow()
            .values()
            .filter(|link| {
                !link.done && link.failed_at.is_none() && link.started.elapsed() > IDENTIFY_TIMEOUT
            })
            .map(|link| link.peripheral.clone())
            .collect();
        for peripheral in expired {
            // SAFETY: on the manager's queue.
            unsafe { central.cancelPeripheralConnection(&peripheral) };
            self.fail(&peripheral, "timed out");
        }
    }

    /// Disconnects every peripheral; the scan is ending.
    fn disconnect_all(&self) {
        let peripherals: Vec<_> = self
            .ivars()
            .links
            .borrow_mut()
            .drain()
            .map(|(_, link)| link.peripheral)
            .collect();
        for peripheral in peripherals {
            self.disconnect(&peripheral);
        }
        self.ivars().central.borrow_mut().take();
    }
}

/// Writes the session's next request, if it has one due.
fn query(link: &mut Link) {
    let Some(write) = &link.write else {
        return;
    };
    let Some(request) = link.session.next_request() else {
        return;
    };
    let data = NSData::with_bytes(&request);
    // SAFETY: on the manager's queue; `write` is the profile's write
    // characteristic of this connected peripheral, which takes Write Command.
    unsafe {
        link.peripheral.writeValue_forCharacteristic_type(
            &data,
            write,
            CBCharacteristicWriteType::WithoutResponse,
        )
    };
}

fn key(peripheral: &CBPeripheral) -> String {
    // SAFETY: identifier has no preconditions.
    unsafe { peripheral.identifier() }.UUIDString().to_string()
}

fn uuids(names: &[&str]) -> Retained<NSArray<CBUUID>> {
    let uuids: Vec<_> = names
        .iter()
        // SAFETY: a 16-bit UUID string, as CBUUID documents.
        .map(|name| unsafe { CBUUID::UUIDWithString(&NSString::from_str(name)) })
        .collect();
    NSArray::from_retained_slice(&uuids)
}

fn uuid_is(uuid: &CBUUID, name: &str) -> bool {
    // SAFETY: UUIDString has no preconditions.
    unsafe { uuid.UUIDString() }
        .to_string()
        .eq_ignore_ascii_case(name)
}

fn describe(what: &str, error: Option<&NSError>) -> String {
    match error {
        Some(error) => format!("{what}: {}", error.localizedDescription()),
        None => what.to_owned(),
    }
}

fn start_scanning(central: &CBCentralManager) {
    let allow_duplicates = NSNumber::new_bool(true);
    // SAFETY: the key is a CoreBluetooth constant string.
    let key: &NSString = unsafe { CBCentralManagerScanOptionAllowDuplicatesKey };
    let value: &AnyObject = &allow_duplicates;
    let options = NSDictionary::<NSString, AnyObject>::from_slices(&[key], &[value]);
    // SAFETY: nil services scans for every peripheral; the option value is
    // the NSNumber boolean CoreBluetooth documents for this key.
    unsafe { central.scanForPeripheralsWithServices_options(None, Some(&options)) };
}

fn read_advertisement(
    peripheral: &CBPeripheral,
    data: &NSDictionary<NSString, AnyObject>,
    rssi: &NSNumber,
) -> Advertisement {
    // SAFETY: the keys are CoreBluetooth constant strings, the peripheral is
    // valid for the callback and these getters have no preconditions.
    let (name_key, manufacturer_key, service_data_key, uuids_key, identifier, fallback_name) = unsafe {
        (
            CBAdvertisementDataLocalNameKey,
            CBAdvertisementDataManufacturerDataKey,
            CBAdvertisementDataServiceDataKey,
            CBAdvertisementDataServiceUUIDsKey,
            peripheral.identifier().UUIDString().to_string(),
            peripheral.name(),
        )
    };
    let local_name = data
        .objectForKey(name_key)
        .and_then(|value| value.downcast::<NSString>().ok())
        .or(fallback_name)
        .map(|name| name.to_string());
    let manufacturer_data = data
        .objectForKey(manufacturer_key)
        .and_then(|value| value.downcast::<NSData>().ok())
        .map(|bytes| bytes.to_vec());
    let service_data = data
        .objectForKey(service_data_key)
        .and_then(|value| value.downcast::<NSDictionary>().ok())
        .map(|services| {
            let (keys, values) = services.to_vecs();
            keys.into_iter()
                .zip(values)
                .filter_map(|(uuid, bytes)| {
                    let uuid = uuid.downcast::<CBUUID>().ok()?;
                    let bytes = bytes.downcast::<NSData>().ok()?;
                    // SAFETY: UUIDString has no preconditions.
                    Some((unsafe { uuid.UUIDString() }.to_string(), bytes.to_vec()))
                })
                .collect()
        })
        .unwrap_or_default();
    let service_uuids = data
        .objectForKey(uuids_key)
        .and_then(|value| value.downcast::<NSArray>().ok())
        .map(|uuids| {
            uuids
                .to_vec()
                .into_iter()
                .filter_map(|uuid| uuid.downcast::<CBUUID>().ok())
                // SAFETY: UUIDString has no preconditions.
                .map(|uuid| unsafe { uuid.UUIDString() }.to_string())
                .collect()
        })
        .unwrap_or_default();
    Advertisement {
        peripheral: identifier,
        local_name,
        manufacturer_data,
        service_data,
        service_uuids,
        rssi: rssi.shortValue(),
    }
}

/// The CoreBluetooth objects of one scan, touched only on `queue`.
struct Radio {
    manager: Retained<CBCentralManager>,
    delegate: Retained<ScanDelegate>,
}

// SAFETY: `Radio` is created on the caller's thread and afterwards only moved
// into a closure that runs on the scan's serial queue (see `Session::drop`).
// CBCentralManager is documented as usable from the queue it was given;
// ScanDelegate's state (an `UnboundedSender`, the links and the manager) is
// only touched from callbacks on that queue and from `Session::drop`, which
// runs on it synchronously.
// Retain/release of Objective-C objects is thread-safe.
unsafe impl Send for Radio {}

/// Keeps a scan alive; dropping it stops scanning and releases the manager.
struct Session {
    queue: DispatchRetained<DispatchQueue>,
    radio: Option<Radio>,
}

impl Drop for Session {
    fn drop(&mut self) {
        let Some(radio) = self.radio.take() else {
            return;
        };
        // Synchronous so no callback runs after the scan is gone, and so the
        // manager and delegate are released on their own queue. A callback
        // never drops the scan, so this cannot deadlock on the queue.
        self.queue.exec_sync(move || {
            // Bind the whole value: destructuring in the pattern would make
            // the closure capture the non-Send fields individually.
            let whole = radio;
            let Radio { manager, delegate } = whole;
            delegate.disconnect_all();
            // SAFETY: on the manager's queue; clearing the (weak) delegate
            // stops further callbacks before the delegate is released.
            unsafe {
                manager.stopScan();
                manager.setDelegate(None);
            }
            drop(manager);
            drop(delegate);
        });
    }
}

/// Where scanning runs for this process.
#[derive(Debug, PartialEq, Eq)]
enum Placement {
    InProcess,
    /// An unbundled binary that declares Bluetooth usage in its embedded
    /// `__info_plist`: scan in a disclaimed helper ([`super::helper`]).
    Helper,
}

fn placement(
    override_on: bool,
    bundle_path: &str,
    has_usage: bool,
) -> Result<Placement, BluetoothBlock> {
    match gate(override_on, bundle_path, has_usage) {
        Ok(()) => Ok(Placement::InProcess),
        Err(_) if has_usage => Ok(Placement::Helper),
        Err(block) => Err(block),
    }
}

/// Starts scanning; see [`crate::ble::start_scan`].
pub fn start_scan(events: UnboundedSender<BleEvent>) -> Result<BleScan, BluetoothBlock> {
    let override_on = std::env::var(OVERRIDE_ENV).is_ok_and(|value| value == "1");
    let bundle = NSBundle::mainBundle();
    let bundle_path = bundle.bundlePath().to_string();
    let has_usage = bundle
        .objectForInfoDictionaryKey(&NSString::from_str(USAGE_KEY))
        .is_some();
    match placement(override_on, &bundle_path, has_usage)? {
        Placement::InProcess => start(events),
        Placement::Helper => super::helper::spawn(events),
    }
}

/// Scans in this process. Creating the manager is what makes macOS ask for
/// Bluetooth access the first time.
pub(super) fn start(events: UnboundedSender<BleEvent>) -> Result<BleScan, BluetoothBlock> {
    let queue = DispatchQueue::new("app.meetnote.devices.ble", DispatchQueueAttr::SERIAL);
    // SAFETY: a class property that reads the TCC record without prompting.
    let authorization = unsafe { CBManager::authorization_class() };
    if authorization == CBManagerAuthorization::Denied
        || authorization == CBManagerAuthorization::Restricted
    {
        let _ = events.unbounded_send(BleEvent::State(BluetoothState::Unauthorized));
        return Ok(BleScan::new(Session { queue, radio: None }));
    }

    let delegate = ScanDelegate::new(events);
    // SAFETY: the delegate outlives the manager's use of it (Session keeps
    // both and clears the delegate first); events arrive on our serial queue.
    let manager = unsafe {
        CBCentralManager::initWithDelegate_queue(
            CBCentralManager::alloc(),
            Some(ProtocolObject::from_ref(&*delegate)),
            Some(&queue),
        )
    };
    Ok(BleScan::new(Session {
        queue,
        radio: Some(Radio { manager, delegate }),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_requires_an_app_bundle_with_usage_text() {
        let app = "/Applications/MeetNote.app";
        assert_eq!(gate(false, app, true), Ok(()));
        assert_eq!(gate(false, app, false), Err(BluetoothBlock::NeedsAppBundle));
        assert_eq!(
            gate(false, "/Users/me/meetnote/target/debug", true),
            Err(BluetoothBlock::NeedsAppBundle)
        );
        assert_eq!(
            gate(false, "/Users/me/meetnote/target/debug", false),
            Err(BluetoothBlock::NeedsAppBundle)
        );
    }

    #[test]
    fn gate_override_skips_the_bundle_check() {
        assert_eq!(gate(true, "/usr/local/bin", false), Ok(()));
    }

    #[test]
    fn an_unbundled_binary_that_declares_usage_scans_in_a_helper() {
        let debug = "/Users/me/meetnote/target/debug";
        assert_eq!(placement(false, debug, true), Ok(Placement::Helper));
        assert_eq!(placement(true, debug, true), Ok(Placement::InProcess));
        assert_eq!(
            placement(false, "/Applications/MeetNote.app", true),
            Ok(Placement::InProcess)
        );
        assert_eq!(
            placement(false, debug, false),
            Err(BluetoothBlock::NeedsAppBundle)
        );
    }

    #[test]
    fn unbundled_test_binary_is_refused_before_touching_bluetooth() {
        // With the override set this would create a manager and TCC would
        // kill the test process, so only check the default path.
        if std::env::var(OVERRIDE_ENV).is_ok_and(|value| value == "1") {
            return;
        }
        let (sender, _receiver) = futures::channel::mpsc::unbounded();
        assert_eq!(
            start_scan(sender).err(),
            Some(BluetoothBlock::NeedsAppBundle)
        );
    }

    #[test]
    fn maps_every_manager_state() {
        let cases = [
            (CBManagerState::PoweredOn, BluetoothState::PoweredOn),
            (CBManagerState::PoweredOff, BluetoothState::PoweredOff),
            (CBManagerState::Unauthorized, BluetoothState::Unauthorized),
            (CBManagerState::Unsupported, BluetoothState::Unsupported),
            (CBManagerState::Resetting, BluetoothState::Resetting),
            (CBManagerState::Unknown, BluetoothState::Unknown),
        ];
        for (state, expected) in cases {
            assert_eq!(map_state(state), expected);
        }
    }

    // A real scan needs a signed MeetNote.app with
    // NSBluetoothAlwaysUsageDescription: macOS kills an unbundled test binary
    // on first Bluetooth use. Run inside the bundle with MEETNOTE_BLUETOOTH=1.
    #[test]
    #[ignore = "needs MeetNote.app and a Bluetooth radio"]
    fn real_scan_reports_a_state() {
        use futures::StreamExt;
        let (sender, mut receiver) = futures::channel::mpsc::unbounded();
        let scan = start_scan(sender).expect("scan starts");
        let event = futures::executor::block_on(receiver.next());
        assert!(matches!(event, Some(BleEvent::State(_))));
        drop(scan);
    }
}
