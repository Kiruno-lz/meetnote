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
//! glass/opaque, into `<route>-<dark|light>-<glass|opaque>.png`. Under glass
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
    screenshots::run(std::env::var("MEETNOTE_VISUAL_UPDATE").as_deref() == Ok("1"))
}

#[cfg(target_os = "macos")]
mod screenshots {
    use crate::{pixels, support};
    use gpui::VisualTestAppContext;
    use settings::{AppearanceSettings, MaterialPreference, ThemePreference};
    use std::path::Path;
    use std::process::ExitCode;
    use std::sync::Arc;
    use ui::test_support::{MainWindow, Route};

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
    /// Frames drawn at most while waiting for two identical ones.
    const MAX_FRAMES: usize = 8;

    unsafe extern "C" {
        fn pthread_main_np() -> i32;
    }

    pub fn run(update: bool) -> ExitCode {
        // SAFETY: `pthread_main_np` only reads the calling thread's identity.
        assert_eq!(
            unsafe { pthread_main_np() },
            1,
            "the L3 screenshots must run on the process main thread"
        );
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

        let mut cx = VisualTestAppContext::with_asset_source(
            gpui_platform::current_platform(false),
            Arc::new(gpui_component_assets::Assets),
        );
        // Service calls complete on worker threads outside the scheduler.
        cx.executor().allow_parking();
        cx.update(support::init);

        let failures_directory =
            std::env::temp_dir().join(format!("meetnote-visual-{}", std::process::id()));
        let mut scale_factor = None;
        let mut failures = 0;
        let screenshots = APPEARANCES.iter().flat_map(|appearance| {
            ROUTES
                .iter()
                .map(move |(route, route_name)| (*appearance, *route, *route_name))
        });
        for ((appearance_name, theme, material), route, route_name) in screenshots {
            let name = format!("{route_name}-{appearance_name}");
            let appearance = AppearanceSettings { theme, material };
            let window = cx.update(|cx| support::open_main_window_in(appearance, cx));
            wait_for_installation(&mut cx, &window);
            cx.update(|cx| window.navigate(route, cx));
            let handle = window.window();
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
        let total = APPEARANCES.len() * ROUTES.len();
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

    /// Drives the scheduler until the startup installation probe, which
    /// runs on a worker thread, has reached the view.
    fn wait_for_installation(cx: &mut VisualTestAppContext, window: &MainWindow) {
        for _ in 0..1_000 {
            cx.run_until_parked();
            if cx.read(|cx| window.installation_loaded(cx)) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("the installation probe did not finish");
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
        let appearances = list(APPEARANCES.iter().map(|(name, ..)| *name).collect());
        format!(
            "{{\n{revisions}  \"scale_factor\": {scale_factor:?},\n  \"window\": {{ \"width\": {}, \"height\": {} }},\n  \"locale\": \"{}\",\n  \"routes\": [{routes}],\n  \"appearances\": [{appearances}]\n}}\n",
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
