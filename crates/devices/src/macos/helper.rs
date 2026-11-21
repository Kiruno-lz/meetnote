//! Bluetooth for an unbundled MeetNote binary.
//!
//! macOS attributes Bluetooth use to the *responsible* process. A binary run
//! from a terminal is attributed to the terminal, and TCC kills it when the
//! terminal declares no Bluetooth usage. So the binary — which carries its
//! own `__info_plist` with `NSBluetoothAlwaysUsageDescription` — starts
//! itself again with `responsibility_spawnattrs_setdisclaim`: the child is
//! its own responsible process, macOS prompts for it on first use, and it
//! streams [`BleEvent`]s back as JSON lines. The child exits when its stdin
//! closes, so it never outlives the scan.

use std::ffi::{CString, c_char, c_int, c_void};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::os::fd::FromRawFd;
use std::thread;

use futures::StreamExt;
use futures::channel::mpsc::UnboundedSender;

use crate::ble::{BleEvent, BleScan, BluetoothBlock, BluetoothState};

use crate::ble::HELPER_ARG;

type SpawnAttr = *mut c_void;
type FileActions = *mut c_void;

unsafe extern "C" {
    static environ: *const *const c_char;
    fn posix_spawn(
        pid: *mut i32,
        path: *const c_char,
        file_actions: *const FileActions,
        attrp: *const SpawnAttr,
        argv: *const *const c_char,
        envp: *const *const c_char,
    ) -> c_int;
    fn posix_spawnattr_init(attr: *mut SpawnAttr) -> c_int;
    fn posix_spawnattr_destroy(attr: *mut SpawnAttr) -> c_int;
    fn posix_spawn_file_actions_init(actions: *mut FileActions) -> c_int;
    fn posix_spawn_file_actions_adddup2(actions: *mut FileActions, fd: c_int, to: c_int) -> c_int;
    fn posix_spawn_file_actions_addclose(actions: *mut FileActions, fd: c_int) -> c_int;
    fn posix_spawn_file_actions_destroy(actions: *mut FileActions) -> c_int;
    /// libsystem's private but stable SPI used by Xcode and LLDB.
    fn responsibility_spawnattrs_setdisclaim(attr: *mut SpawnAttr, disclaim: c_int) -> c_int;
    fn pipe(fds: *mut c_int) -> c_int;
    fn close(fd: c_int) -> c_int;
    fn kill(pid: i32, signal: c_int) -> c_int;
    fn waitpid(pid: i32, status: *mut c_int, options: c_int) -> i32;
}

const SIGTERM: c_int = 15;

/// The running helper; dropping it closes its stdin and reaps it.
struct Helper {
    pid: i32,
    stdin: Option<File>,
}

impl Drop for Helper {
    fn drop(&mut self) {
        self.stdin.take();
        let pid = self.pid;
        // SAFETY: `pid` is our own child; signalling and reaping it once.
        unsafe { kill(pid, SIGTERM) };
        thread::spawn(move || {
            let mut status = 0;
            // SAFETY: as above.
            unsafe { waitpid(pid, &mut status, 0) };
        });
    }
}

fn pipe_pair() -> Result<(c_int, c_int), BluetoothBlock> {
    let mut fds = [0; 2];
    // SAFETY: `fds` has room for the two descriptors pipe writes.
    if unsafe { pipe(fds.as_mut_ptr()) } != 0 {
        return Err(BluetoothBlock::Failed(
            std::io::Error::last_os_error().to_string(),
        ));
    }
    Ok((fds[0], fds[1]))
}

/// Starts the helper and forwards its events to `events`.
pub fn spawn(events: UnboundedSender<BleEvent>) -> Result<BleScan, BluetoothBlock> {
    let failed = |message: String| BluetoothBlock::Failed(message);
    let exe = std::env::current_exe().map_err(|error| failed(error.to_string()))?;
    let path = CString::new(exe.as_os_str().as_encoded_bytes())
        .map_err(|error| failed(error.to_string()))?;
    let argument = CString::new(HELPER_ARG).expect("no NUL");
    let argv = [path.as_ptr(), argument.as_ptr(), std::ptr::null()];
    // The helper runs CoreBluetooth in-process: skip the bundle gate there.
    let override_env = CString::new("MEETNOTE_BLUETOOTH=1").expect("no NUL");
    let mut envp: Vec<*const c_char> = vec![override_env.as_ptr()];
    // SAFETY: `environ` is the process environment, NULL-terminated.
    unsafe {
        let mut entry = environ;
        while !(*entry).is_null() {
            envp.push(*entry);
            entry = entry.add(1);
        }
    }
    envp.push(std::ptr::null());

    let (child_stdin, parent_stdin) = pipe_pair()?;
    let (parent_stdout, child_stdout) = pipe_pair()?;
    let mut pid = 0;
    // SAFETY: every attribute and action object is initialised before use
    // and destroyed after; argv/envp are NULL-terminated arrays of C strings
    // that outlive the call; the descriptors are the pipes created above.
    let status = unsafe {
        let mut attr: SpawnAttr = std::ptr::null_mut();
        let mut actions: FileActions = std::ptr::null_mut();
        posix_spawnattr_init(&mut attr);
        responsibility_spawnattrs_setdisclaim(&mut attr, 1);
        posix_spawn_file_actions_init(&mut actions);
        posix_spawn_file_actions_adddup2(&mut actions, child_stdin, 0);
        posix_spawn_file_actions_adddup2(&mut actions, child_stdout, 1);
        for fd in [child_stdin, child_stdout, parent_stdin, parent_stdout] {
            posix_spawn_file_actions_addclose(&mut actions, fd);
        }
        let status = posix_spawn(
            &mut pid,
            path.as_ptr(),
            &actions,
            &attr,
            argv.as_ptr(),
            envp.as_ptr(),
        );
        posix_spawn_file_actions_destroy(&mut actions);
        posix_spawnattr_destroy(&mut attr);
        close(child_stdin);
        close(child_stdout);
        status
    };
    if status != 0 {
        // SAFETY: the parent ends are still ours to close.
        unsafe {
            close(parent_stdin);
            close(parent_stdout);
        }
        return Err(failed(
            std::io::Error::from_raw_os_error(status).to_string(),
        ));
    }
    // SAFETY: the parent ends are open descriptors owned from here on.
    let (stdin, stdout) = unsafe {
        (
            File::from_raw_fd(parent_stdin),
            File::from_raw_fd(parent_stdout),
        )
    };
    thread::Builder::new()
        .name("meetnote-bluetooth-helper".into())
        .spawn(move || forward(BufReader::new(stdout), &events))
        .map_err(|error| failed(error.to_string()))?;
    Ok(BleScan::new(Helper {
        pid,
        stdin: Some(stdin),
    }))
}

/// Forwards each event line. A helper that ends before reporting any state
/// was terminated by TCC: Bluetooth access is denied.
fn forward(lines: impl BufRead, events: &UnboundedSender<BleEvent>) {
    let mut reported = false;
    for line in lines.lines() {
        let Ok(line) = line else { break };
        let Ok(event) = serde_json::from_str::<BleEvent>(&line) else {
            continue;
        };
        reported |= matches!(event, BleEvent::State(_));
        if events.unbounded_send(event).is_err() {
            return;
        }
    }
    if !reported {
        let _ = events.unbounded_send(BleEvent::State(BluetoothState::Unauthorized));
    }
}

/// The helper's body: scan in-process and print each event as one JSON line
/// until stdin closes.
pub fn run() -> i32 {
    let (sender, receiver) = futures::channel::mpsc::unbounded();
    let _scan = match super::ble::start(sender) {
        Ok(scan) => scan,
        Err(block) => {
            eprintln!("{block}");
            return 1;
        }
    };
    thread::spawn(|| {
        let mut sink = Vec::new();
        let _ = std::io::Read::read_to_end(&mut std::io::stdin(), &mut sink);
        std::process::exit(0);
    });
    let mut stdout = std::io::stdout().lock();
    futures::executor::block_on(receiver.for_each(|event| {
        if let Ok(line) = serde_json::to_string(&event) {
            let _ = writeln!(stdout, "{line}");
            let _ = stdout.flush();
        }
        futures::future::ready(())
    }));
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ble::Advertisement;

    #[test]
    fn forwards_event_lines_and_skips_noise() {
        let seen = BleEvent::Seen(Advertisement {
            peripheral: "p".into(),
            rssi: -40,
            ..Default::default()
        });
        let text = format!(
            "{}\nnot json\n{}\n",
            serde_json::to_string(&BleEvent::State(BluetoothState::PoweredOn)).unwrap(),
            serde_json::to_string(&seen).unwrap()
        );
        let (sender, mut receiver) = futures::channel::mpsc::unbounded();
        forward(text.as_bytes(), &sender);
        assert_eq!(
            receiver.try_recv().unwrap(),
            BleEvent::State(BluetoothState::PoweredOn)
        );
        assert_eq!(receiver.try_recv().unwrap(), seen);
        assert!(receiver.try_recv().is_err(), "no synthetic state");
    }

    #[test]
    fn a_helper_that_dies_silently_means_denied() {
        let (sender, mut receiver) = futures::channel::mpsc::unbounded();
        forward(&b""[..], &sender);
        assert_eq!(
            receiver.try_recv().unwrap(),
            BleEvent::State(BluetoothState::Unauthorized)
        );
    }
}
