//! L3: offscreen screenshots of every route, compared with the baselines in
//! `tests/visual/`.
//!
//! Compare with the baselines:
//!
//! ```text
//! MEETNOTE_VISUAL=1 cargo test -p ui --test visual
//! ```
//!
//! Rewrite the baselines after an intended change, then review every PNG
//! against the visual checklist in `docs/plan/ui-plan.md`:
//!
//! ```text
//! MEETNOTE_VISUAL=1 MEETNOTE_VISUAL_UPDATE=1 cargo test -p ui --test visual
//! ```
//!
//! Without `MEETNOTE_VISUAL=1`, or off macOS, the target prints why it
//! skipped and succeeds, so `cargo test -p ui` stays headless.
//!
//! The target has `harness = false`: `main` runs on the process main thread,
//! where the macOS platform creates its AppKit windows and Metal layers.
//! libtest runs every test on a spawned thread, even with
//! `--test-threads=1`; `main` asserts it is on the main thread.
//!
//! Each route renders in each of the four pinned appearances, dark/light ×
//! glass/opaque, into `<route>-<dark|light>-<glass|opaque>.png`; so do the
//! overlays over their route (an open Settings select and the migration
//! dialog), into `<overlay>-<appearance>.png`, once their entrance animation
//! has finished. Under glass
//! the window background is clear, so the chrome's pixels carry the glass
//! alpha: the capture has no desktop behind it, and the desktop blur is
//! checked by hand.
//!
//! Each screenshot renders in its own 1180×760 window on the shared fixture
//! (`support`) through `VisualTestAppContext`, which pairs the real macOS
//! platform and Metal renderer with GPUI's deterministic scheduler, and is
//! captured with `Window::render_to_image` once two consecutive frames are
//! identical. `baseline.json` records the display scale factor, window size,
//! locale and GPUI revisions; screenshots are compared only with baselines of
//! the same scale factor. `pixels.rs` defines a change; the actual and diff
//! images of a changed route are written to a temporary directory whose
//! paths are printed.
//!
//! With `MEETNOTE_RESOURCE_PROBE=1` as well, the target then probes the
//! floating surfaces' GPU cost (`resource_probe`). The probe measures frame
//! time, which only an optimized build shows, so it runs in release; a build
//! with debug assertions refuses it and fails at once:
//!
//! ```text
//! MEETNOTE_VISUAL=1 MEETNOTE_RESOURCE_PROBE=1 cargo test --release -p ui --test visual
//! ```

#[cfg(target_os = "macos")]
#[path = "support/pixels.rs"]
mod pixels;
#[cfg(target_os = "macos")]
mod support;

use std::process::ExitCode;

#[cfg(not(target_os = "macos"))]
fn main() -> ExitCode {
    println!("visual: skipped; the L3 screenshots render with Metal on macOS");
    ExitCode::SUCCESS
}

#[cfg(target_os = "macos")]
fn main() -> ExitCode {
    if std::env::var("MEETNOTE_VISUAL").as_deref() != Ok("1") {
        println!("visual: skipped; set MEETNOTE_VISUAL=1 to compare the L3 route screenshots");
        return ExitCode::SUCCESS;
    }
    let probe_requested = std::env::var("MEETNOTE_RESOURCE_PROBE").as_deref() == Ok("1");
    if probe_requested && cfg!(debug_assertions) {
        println!(
            "resource probe: the resource probe measures frame time; run it with \
             `cargo test --release -p ui --test visual`"
        );
        return ExitCode::FAILURE;
    }
    let mut cx = screenshots::context();
    let screenshots = screenshots::run(
        &mut cx,
        std::env::var("MEETNOTE_VISUAL_UPDATE").as_deref() == Ok("1"),
    );
    if !probe_requested {
        return screenshots;
    }
    let probe = resource_probe::run(&mut cx);
    if screenshots == ExitCode::SUCCESS && !probe {
        return ExitCode::FAILURE;
    }
    screenshots
}

#[cfg(target_os = "macos")]
mod screenshots {
    use crate::{pixels, support};
    use gpui::VisualTestAppContext;
    use settings::{AppearanceSettings, MaterialPreference, ThemePreference};
    use std::path::Path;
    use std::process::ExitCode;
    use std::sync::Arc;
    use ui::test_support::{MainWindow, Overlay, Route};

    /// Every appearance with its baseline file name suffix; pinned, never
    /// `ThemePreference::System`.
    const APPEARANCES: [(&str, ThemePreference, MaterialPreference); 4] = [
        (
            "dark-glass",
            ThemePreference::Dark,
            MaterialPreference::Glass,
        ),
        (
            "dark-opaque",
            ThemePreference::Dark,
            MaterialPreference::Opaque,
        ),
        (
            "light-glass",
            ThemePreference::Light,
            MaterialPreference::Glass,
        ),
        (
            "light-opaque",
            ThemePreference::Light,
            MaterialPreference::Opaque,
        ),
    ];
    /// Every route with its baseline file name prefix.
    const ROUTES: [(Route, &str); 4] = [
        (Route::Assistant, "assistant"),
        (Route::Meetings, "meetings"),
        (Route::Models, "models"),
        (Route::Settings, "settings"),
    ];
    /// Overlays captured over a route, with their baseline file name prefix.
    const OVERLAYS: [(Route, Overlay, &str); 2] = [
        (Route::Settings, Overlay::ThemeSelect, "settings-select"),
        (
            Route::Settings,
            Overlay::SpeakerLibraryDialog,
            "settings-dialog",
        ),
    ];
    /// Frames drawn at most while waiting for two identical ones.
    const MAX_FRAMES: usize = 8;
    /// Longer than every overlay entrance animation, which runs on the wall
    /// clock.
    const OVERLAY_SETTLE: std::time::Duration = std::time::Duration::from_millis(400);

    unsafe extern "C" {
        fn pthread_main_np() -> i32;
    }

    /// The app on the real macOS platform and Metal renderer, set up as
    /// `run` does. It must be created on the process main thread.
    pub fn context() -> VisualTestAppContext {
        // SAFETY: `pthread_main_np` only reads the calling thread's identity.
        assert_eq!(
            unsafe { pthread_main_np() },
            1,
            "the L3 screenshots must run on the process main thread"
        );
        let mut cx = VisualTestAppContext::with_asset_source(
            gpui_platform::current_platform(false),
            Arc::new(gpui_component_assets::Assets),
        );
        // Service calls complete on worker threads outside the scheduler.
        cx.executor().allow_parking();
        cx.update(support::init);
        cx
    }

    pub fn run(cx: &mut VisualTestAppContext, update: bool) -> ExitCode {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let baselines = manifest.join("tests/visual");
        let lock = std::fs::read_to_string(manifest.join("../../Cargo.lock"))
            .expect("the workspace Cargo.lock is readable");
        let revisions = [
            ("gpui", locked_revision(&lock, "gpui")),
            ("gpui_component", locked_revision(&lock, "gpui-component")),
        ];
        let recorded = (!update).then(|| {
            std::fs::read_to_string(baselines.join("baseline.json")).unwrap_or_else(|error| {
                panic!("no baseline.json ({error}); generate it with MEETNOTE_VISUAL_UPDATE=1")
            })
        });
        if let Some(recorded) = &recorded {
            for (key, current) in &revisions {
                println!(
                    "visual: {key} baseline {} / current {current}",
                    json_field(recorded, key).unwrap_or("?")
                );
            }
        }

        let failures_directory =
            std::env::temp_dir().join(format!("meetnote-visual-{}", std::process::id()));
        let mut scale_factor = None;
        let mut failures = 0;
        let screenshots = APPEARANCES.iter().flat_map(|appearance| {
            ROUTES
                .iter()
                .map(move |(route, route_name)| (*appearance, *route, None, *route_name))
                .chain(OVERLAYS.iter().map(move |(route, overlay, name)| {
                    (*appearance, *route, Some(*overlay), *name)
                }))
        });
        for ((appearance_name, theme, material), route, overlay, route_name) in screenshots {
            let name = format!("{route_name}-{appearance_name}");
            let appearance = AppearanceSettings { theme, material };
            let window = cx.update(|cx| support::open_main_window_in(appearance, cx));
            wait_for_startup(cx, &window);
            cx.update(|cx| window.navigate(route, cx));
            let handle = window.window();
            if let Some(overlay) = overlay {
                // The route draws once, so a select knows where its trigger is.
                cx.run_until_parked();
                cx.update_window(handle, |_, window, cx| window.draw(cx).clear())
                    .expect("the route window draws");
                cx.update(|cx| window.open_overlay(overlay, cx));
                cx.run_until_parked();
                cx.update_window(handle, |_, window, cx| window.draw(cx).clear())
                    .expect("the route window draws");
                std::thread::sleep(OVERLAY_SETTLE);
            }
            let mut previous: Option<Vec<u8>> = None;
            let mut stable = None;
            for _ in 0..MAX_FRAMES {
                cx.run_until_parked();
                cx.update_window(handle, |_, window, cx| window.draw(cx).clear())
                    .expect("the route window draws");
                let screenshot = cx
                    .capture_screenshot(handle)
                    .expect("the Metal renderer captures the route window");
                let settled = previous.as_deref() == Some(screenshot.as_raw().as_slice());
                previous = Some(screenshot.as_raw().clone());
                if settled {
                    stable = Some(screenshot);
                    break;
                }
            }
            let screenshot =
                stable.unwrap_or_else(|| panic!("{name}: frames still change after {MAX_FRAMES}"));
            let scale = cx
                .update_window(handle, |_, window, _| window.scale_factor())
                .expect("the route window is open");
            cx.update_window(handle, |_, window, _| window.remove_window())
                .expect("the route window closes");
            cx.run_until_parked();
            assert!(
                scale_factor.is_none_or(|first| first == scale),
                "{name}: the scale factor changed between routes"
            );
            scale_factor = Some(scale);

            let path = baselines.join(format!("{name}.png"));
            if update {
                std::fs::create_dir_all(&baselines).expect("the baseline directory");
                screenshot.save(&path).expect("the baseline PNG is written");
                println!("visual: {name}: wrote {}", path.display());
                continue;
            }
            let recorded_scale = recorded
                .as_deref()
                .and_then(|recorded| json_field(recorded, "scale_factor"))
                .and_then(|value| value.parse::<f32>().ok());
            if recorded_scale != Some(scale) {
                println!(
                    "visual: the baselines were captured at scale factor {recorded_scale:?}, \
                     this display renders at {scale}; compare on a display with the \
                     baseline scale factor"
                );
                return ExitCode::FAILURE;
            }
            let actual = pixels::Rgba {
                width: screenshot.width(),
                height: screenshot.height(),
                pixels: screenshot.as_raw().clone(),
            };
            let expected = match read_png(&path) {
                Ok(expected) => expected,
                Err(error) => {
                    println!("visual: {name}: {error}; generate it with MEETNOTE_VISUAL_UPDATE=1");
                    failures += 1;
                    continue;
                }
            };
            let comparison = pixels::compare(&expected, &actual);
            match comparison {
                pixels::Comparison::Compared { differing, total } => println!(
                    "visual: {name}: {differing} of {total} pixels differ ({:.3}%)",
                    differing as f64 * 100. / total as f64
                ),
                pixels::Comparison::SizeChanged { expected, actual } => {
                    println!("visual: {name}: size changed from {expected:?} to {actual:?}")
                }
            }
            if comparison.changed() {
                failures += 1;
                std::fs::create_dir_all(&failures_directory).expect("the failure directory");
                let actual_path = failures_directory.join(format!("{name}-actual.png"));
                screenshot
                    .save(&actual_path)
                    .expect("the actual PNG is written");
                println!("visual: {name}: actual {}", actual_path.display());
                if let Some(diff) = pixels::diff(&expected, &actual) {
                    let diff_path = failures_directory.join(format!("{name}-diff.png"));
                    let mut image = screenshot.clone();
                    image.copy_from_slice(&diff.pixels);
                    image.save(&diff_path).expect("the diff PNG is written");
                    println!("visual: {name}: diff {}", diff_path.display());
                }
            }
        }

        let scale_factor = scale_factor.expect("at least one route rendered");
        if update {
            let path = baselines.join("baseline.json");
            std::fs::write(&path, metadata(scale_factor, &revisions))
                .expect("baseline.json is written");
            println!(
                "visual: wrote {} at scale factor {scale_factor}; review every PNG",
                path.display()
            );
            return ExitCode::SUCCESS;
        }
        let total = APPEARANCES.len() * (ROUTES.len() + OVERLAYS.len());
        if failures > 0 {
            println!(
                "visual: {failures} of {total} screenshots changed by more than {}‰ of their pixels",
                pixels::MAX_DIFFERING_PER_MILLE
            );
            return ExitCode::FAILURE;
        }
        println!("visual: {total} screenshots match their baselines");
        ExitCode::SUCCESS
    }

    /// Drives the scheduler until the startup installation probe and project
    /// list, which load on worker threads, have reached the view.
    pub fn wait_for_startup(cx: &mut VisualTestAppContext, window: &MainWindow) {
        for _ in 0..1_000 {
            cx.run_until_parked();
            if cx.read(|cx| window.startup_loaded(cx)) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("the startup loads did not finish");
    }

    /// Decodes a baseline through GPUI's image loader, which yields BGRA.
    fn read_png(path: &Path) -> Result<pixels::Rgba, String> {
        let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let image = gpui::Image::from_bytes(gpui::ImageFormat::Png, bytes)
            .to_image_data(gpui::SvgRenderer::new(Arc::new(())))
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let size = image.size(0);
        let mut pixels = image
            .as_bytes(0)
            .ok_or_else(|| format!("{}: no frame", path.display()))?
            .to_vec();
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        Ok(pixels::Rgba {
            width: size.width.0 as u32,
            height: size.height.0 as u32,
            pixels,
        })
    }

    /// The git revision `Cargo.lock` pins `package` to.
    fn locked_revision(lock: &str, package: &str) -> String {
        let name = format!("name = \"{package}\"");
        lock.split("[[package]]")
            .find(|block| block.lines().any(|line| line == name))
            .and_then(|block| {
                block
                    .lines()
                    .find_map(|line| line.strip_prefix("source = \""))
            })
            .and_then(|source| source.trim_end_matches('"').split('#').nth(1))
            .unwrap_or_else(|| panic!("Cargo.lock pins no git revision for {package}"))
            .to_string()
    }

    fn metadata(scale_factor: f32, revisions: &[(&str, String)]) -> String {
        let revisions = revisions
            .iter()
            .map(|(key, revision)| format!("  \"{key}\": \"{revision}\",\n"))
            .collect::<String>();
        let list = |names: Vec<&str>| {
            names
                .iter()
                .map(|name| format!("\"{name}\""))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let routes = list(ROUTES.iter().map(|(_, name)| *name).collect());
        let overlays = list(OVERLAYS.iter().map(|(_, _, name)| *name).collect());
        let appearances = list(APPEARANCES.iter().map(|(name, ..)| *name).collect());
        format!(
            "{{\n{revisions}  \"scale_factor\": {scale_factor:?},\n  \"window\": {{ \"width\": {}, \"height\": {} }},\n  \"locale\": \"{}\",\n  \"routes\": [{routes}],\n  \"overlays\": [{overlays}],\n  \"appearances\": [{appearances}]\n}}\n",
            support::WINDOW_WIDTH,
            support::WINDOW_HEIGHT,
            support::LOCALE,
        )
    }

    /// The raw value of `key` in the flat JSON `metadata` writes.
    fn json_field<'a>(json: &'a str, key: &str) -> Option<&'a str> {
        let value = json.split(&format!("\"{key}\":")).nth(1)?.trim_start();
        match value.strip_prefix('"') {
            Some(string) => string.split('"').next(),
            None => value.split([',', '}', '\n']).next().map(str::trim),
        }
    }
}

/// The GPU cost of the floating surfaces: each overlay over the Settings
/// route, under dark glass so its backdrop blur runs, opens and closes
/// `WARMUP_CYCLES` times, then `MEASURED_CYCLES` times more. Every frame is
/// drawn (`Window::draw`, the frame's CPU work) and rendered by the window's
/// Metal renderer (`Window::render_to_image`: encoding, GPU execution and a
/// pixel readback that presenting would not do). The probe prints the
/// measured cycles' growth of `MTLDevice.currentAllocatedSize` and the p95 of
/// the frame times, and fails when the growth exceeds `MAX_GROWTH` or the
/// draw p95 exceeds `MAX_DRAW_P95`.
#[cfg(target_os = "macos")]
mod resource_probe {
    use crate::{screenshots, support};
    use gpui::{AnyWindowHandle, VisualTestAppContext};
    use settings::{AppearanceSettings, MaterialPreference, ThemePreference};
    use std::time::{Duration, Instant};
    use ui::test_support::{Overlay, Route};

    const WARMUP_CYCLES: usize = 10;
    const MEASURED_CYCLES: usize = 50;
    /// The most `currentAllocatedSize` may grow over the measured cycles.
    const MAX_GROWTH: i64 = 8 * 1024 * 1024;
    /// One frame at 60 Hz.
    const MAX_DRAW_P95: Duration = Duration::from_micros(16_700);
    /// Frames are drawn for this long after each open, past the longest
    /// entrance animation (the dialog's 180 ms).
    const OPEN_FRAMES_FOR: Duration = Duration::from_millis(200);
    /// Frames drawn after each close.
    const CLOSE_FRAMES: usize = 2;
    const OVERLAYS: [(Overlay, &str); 2] = [
        (Overlay::ThemeSelect, "settings-select"),
        (Overlay::SpeakerLibraryDialog, "settings-dialog"),
    ];

    #[derive(Default)]
    struct Frames {
        draw: Vec<Duration>,
        render: Vec<Duration>,
    }

    /// Whether every overlay stays within the bounds.
    pub fn run(cx: &mut VisualTestAppContext) -> bool {
        let device = renderer_device();
        println!(
            "resource probe: {} ({WARMUP_CYCLES} warm-up + {MEASURED_CYCLES} measured \
             open/close cycles per overlay, dark glass)",
            device.name()
        );
        let appearance = AppearanceSettings {
            theme: ThemePreference::Dark,
            material: MaterialPreference::Glass,
        };
        let mut passed = true;
        for (overlay, name) in OVERLAYS {
            let window = cx.update(|cx| support::open_main_window_in(appearance, cx));
            screenshots::wait_for_startup(cx, &window);
            cx.update(|cx| window.navigate(Route::Settings, cx));
            let handle = window.window();
            // The route draws once, so the select knows where its trigger is.
            frame(cx, handle, &mut Frames::default());
            let mut frames = Frames::default();
            let mut start = 0;
            for cycle in 0..WARMUP_CYCLES + MEASURED_CYCLES {
                if cycle == WARMUP_CYCLES {
                    start = device.current_allocated_size();
                    frames = Frames::default();
                }
                cx.update(|cx| window.open_overlay(overlay, cx));
                let opened = Instant::now();
                while opened.elapsed() < OPEN_FRAMES_FOR {
                    frame(cx, handle, &mut frames);
                }
                cx.update(|cx| window.close_overlay(overlay, cx));
                for _ in 0..CLOSE_FRAMES {
                    frame(cx, handle, &mut frames);
                }
            }
            let end = device.current_allocated_size();
            cx.update_window(handle, |_, window, _| window.remove_window())
                .expect("the probe window closes");
            cx.run_until_parked();

            let growth = end as i64 - start as i64;
            let draw = p95(&mut frames.draw);
            let render = p95(&mut frames.render);
            println!(
                "resource probe: {name}: currentAllocatedSize {} → {} ({:+.2} MiB); \
                 {} frames, draw p95 {:.2} ms, render p95 {:.2} ms",
                mebibytes(start as i64),
                mebibytes(end as i64),
                growth as f64 / (1024. * 1024.),
                frames.draw.len(),
                milliseconds(draw),
                milliseconds(render),
            );
            if growth > MAX_GROWTH {
                println!("resource probe: {name}: allocation grew past 8 MiB");
                passed = false;
            }
            if draw > MAX_DRAW_P95 {
                println!("resource probe: {name}: draw p95 exceeds 16.7 ms");
                passed = false;
            }
        }
        passed
    }

    /// Draws and renders one frame, recording both times. The frame runs
    /// in its own autorelease pool, as gpui's presented frames do: this
    /// harness runs no AppKit run loop to drain one, and each rendered frame
    /// autoreleases a drawable of the window's full size.
    fn frame(cx: &mut VisualTestAppContext, handle: AnyWindowHandle, frames: &mut Frames) {
        cx.run_until_parked();
        metal::objc::rc::autoreleasepool(|| {
            let started = Instant::now();
            cx.update_window(handle, |_, window, cx| window.draw(cx).clear())
                .expect("the probe window draws");
            let drawn = Instant::now();
            cx.capture_screenshot(handle)
                .expect("the Metal renderer renders the probe window");
            frames.draw.push(drawn - started);
            frames.render.push(drawn.elapsed());
        });
    }

    /// The nearest-rank 95th percentile.
    fn p95(samples: &mut [Duration]) -> Duration {
        samples.sort_unstable();
        let rank = (samples.len() * 95).div_ceil(100).max(1);
        samples[rank - 1]
    }

    fn milliseconds(duration: Duration) -> f64 {
        duration.as_secs_f64() * 1_000.
    }

    fn mebibytes(bytes: i64) -> String {
        format!("{:.2} MiB", bytes as f64 / (1024. * 1024.))
    }

    /// The device gpui's Metal renderer picks (`MetalRenderer::create_device`
    /// in `gpui_macos`): the first non-removable, non-low-power device, or the
    /// system default. Metal returns the same device object to every caller
    /// in the process, so its allocation count includes gpui's.
    fn renderer_device() -> metal::Device {
        metal::Device::all()
            .into_iter()
            .min_by_key(|device| (device.is_removable(), !device.is_low_power()))
            .or_else(metal::Device::system_default)
            .expect("a Metal device")
    }
}
