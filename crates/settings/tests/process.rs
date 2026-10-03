use settings::{
    AppSettings, JsonSettingsRepository, SettingsError, SettingsRepository, SettingsService,
};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const WORKER_ENV: &str = "MEETNOTE_SETTINGS_PROCESS_WORKER";
const PATH_ENV: &str = "MEETNOTE_SETTINGS_PROCESS_PATH";
const OP_ENV: &str = "MEETNOTE_SETTINGS_PROCESS_OPERATION";
const READY_ENV: &str = "MEETNOTE_SETTINGS_PROCESS_READY";
const GO_ENV: &str = "MEETNOTE_SETTINGS_PROCESS_GO";
const VALUE_ENV: &str = "MEETNOTE_SETTINGS_PROCESS_VALUE";

#[test]
fn settings_process_worker() {
    if env::var_os(WORKER_ENV).is_none() {
        return;
    }

    let path = PathBuf::from(env::var_os(PATH_ENV).expect("worker settings path"));
    let operation = env::var(OP_ENV).expect("worker operation");
    let service = SettingsService::open(std::sync::Arc::new(JsonSettingsRepository::open(&path)))
        .expect("worker service should open");

    if let Some(ready) = env::var_os(READY_ENV) {
        fs::write(ready, b"ready").expect("worker should publish readiness");
        let go = PathBuf::from(env::var_os(GO_ENV).expect("worker go path"));
        let deadline = Instant::now() + Duration::from_secs(15);
        while !go.is_file() {
            assert!(
                Instant::now() < deadline,
                "worker did not receive go signal"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }

    let result = match operation.as_str() {
        "update" => {
            let mut settings = service.snapshot().expect("worker snapshot");
            settings.ui_locale = env::var(VALUE_ENV).expect("worker update value");
            service.update(settings).map(|_| ())
        }
        "reset" => service.reset().map(|_| ()),
        other => panic!("unsupported worker operation: {other}"),
    };

    match result {
        Ok(()) => println!("worker-result:success"),
        Err(SettingsError::RevisionConflict) => println!("worker-result:conflict"),
        Err(error) => panic!("worker operation failed unexpectedly: {error}"),
    }
}

#[test]
fn independent_process_services_cas_update_and_reset_are_atomic() {
    let directory = unique_directory("race");
    fs::create_dir_all(&directory).expect("create process test directory");
    let path = directory.join("settings.json");
    let ready_update = directory.join("update.ready");
    let ready_reset = directory.join("reset.ready");
    let go = directory.join("go");

    let initial = AppSettings {
        ui_locale: "de-DE".into(),
        ..AppSettings::default()
    };
    JsonSettingsRepository::open(&path)
        .save(&initial)
        .expect("write initial settings");

    let executable = env::current_exe().expect("current test executable");
    let update = spawn_worker(
        &executable,
        &path,
        "update",
        Some(&ready_update),
        Some(&go),
        Some("en-US"),
    )
    .expect("spawn update worker");
    let reset = spawn_worker(
        &executable,
        &path,
        "reset",
        Some(&ready_reset),
        Some(&go),
        None,
    )
    .expect("spawn reset worker");

    wait_for_file(&ready_update);
    wait_for_file(&ready_reset);
    fs::write(&go, b"go").expect("release workers");

    let update_output = wait_worker_while_reading(update, &path);
    let reset_output = wait_worker_while_reading(reset, &path);
    assert_worker_result(&update_output);
    assert_worker_result(&reset_output);

    let update_succeeded = worker_stdout(&update_output).contains("worker-result:success");
    let reset_succeeded = worker_stdout(&reset_output).contains("worker-result:success");
    assert_ne!(
        update_succeeded,
        reset_succeeded,
        "exactly one stale service must win the disk CAS\nupdate: {}\nreset: {}",
        worker_stdout(&update_output),
        worker_stdout(&reset_output)
    );

    let raw = fs::read_to_string(&path).expect("final settings file should exist");
    let final_settings: AppSettings =
        serde_json::from_str(&raw).expect("concurrent atomic writes must leave valid JSON");
    assert!(
        final_settings == AppSettings::default() || final_settings.ui_locale == "en-US",
        "final settings must come from one complete committed value: {final_settings:?}"
    );
    assert_no_temporary_files(&directory);

    let recovery =
        spawn_worker(&executable, &path, "reset", None, None, None).expect("spawn recovery worker");
    let recovery_output = wait_worker_while_reading(recovery, &path);
    assert_worker_result(&recovery_output);
    assert!(worker_stdout(&recovery_output).contains("worker-result:success"));
    assert_eq!(
        JsonSettingsRepository::open(&path)
            .load()
            .expect("settings should remain readable after recovery"),
        AppSettings::default()
    );
    assert_no_temporary_files(&directory);

    fs::remove_dir_all(&directory).expect("clean process test directory");
}

#[test]
fn failed_atomic_write_releases_lock_and_recovers() {
    let directory = unique_directory("failure");
    fs::create_dir_all(&directory).expect("create failure test directory");
    let path = directory.join("settings.json");
    fs::create_dir(&path).expect("make settings target an invalid file target");

    let repository = std::sync::Arc::new(JsonSettingsRepository::open(&path));
    let service = SettingsService::open(repository.clone()).expect("open service");
    let mut changed = service.snapshot().expect("read initial settings");
    changed.ui_locale = "en-US".into();

    assert!(matches!(
        service.update(changed.clone()),
        Err(SettingsError::Write { .. })
    ));
    assert_no_temporary_files(&directory);

    fs::remove_dir(&path).expect("remove invalid target after failed write");
    service
        .update(changed.clone())
        .expect("same service should recover after persistence failure");
    assert_eq!(repository.load().expect("load recovered settings"), changed);
    assert_no_temporary_files(&directory);

    fs::remove_dir_all(&directory).expect("clean failure test directory");
}

fn spawn_worker(
    executable: &Path,
    path: &Path,
    operation: &str,
    ready: Option<&Path>,
    go: Option<&Path>,
    value: Option<&str>,
) -> io::Result<Child> {
    let mut command = Command::new(executable);
    command
        .args(["--exact", "settings_process_worker", "--nocapture"])
        .env(WORKER_ENV, "1")
        .env(PATH_ENV, path)
        .env(OP_ENV, operation)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(ready) = ready {
        command.env(READY_ENV, ready);
    }
    if let Some(go) = go {
        command.env(GO_ENV, go);
    }
    if let Some(value) = value {
        command.env(VALUE_ENV, value);
    }
    command.spawn()
}

fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !path.is_file() {
        assert!(
            Instant::now() < deadline,
            "worker did not become ready: {path:?}"
        );
        thread::sleep(Duration::from_millis(2));
    }
}

fn wait_worker_while_reading(mut child: Child, settings_path: &Path) -> Output {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match child.try_wait().expect("poll worker") {
            Some(_) => return child.wait_with_output().expect("collect worker output"),
            None => {
                let raw = fs::read_to_string(settings_path).expect("settings file should exist");
                serde_json::from_str::<AppSettings>(&raw)
                    .expect("reader must never observe a half-written settings file");
                assert!(Instant::now() < deadline, "worker did not finish");
                thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

fn assert_worker_result(output: &Output) {
    assert!(
        output.status.success(),
        "worker failed\nstdout: {}\nstderr: {}",
        worker_stdout(output),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = worker_stdout(output);
    assert!(
        stdout.contains("worker-result:success") || stdout.contains("worker-result:conflict"),
        "worker did not report a CAS result: {stdout}"
    );
}

fn worker_stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn assert_no_temporary_files(directory: &Path) {
    let temporary_files = fs::read_dir(directory)
        .expect("read test directory")
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with(".settings.json.") && name.ends_with(".tmp"))
        .collect::<Vec<_>>();
    assert!(
        temporary_files.is_empty(),
        "failed writes must remove temporary files: {temporary_files:?}"
    );
}

fn unique_directory(label: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    env::temp_dir().join(format!(
        "meetnote-settings-process-{label}-{}-{timestamp}",
        std::process::id()
    ))
}
