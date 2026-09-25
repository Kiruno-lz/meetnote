//! Entry points for the GPUI tests under `crates/ui/tests`: they set the app
//! up and open the desktop window exactly as `run` does, then switch routes.
//! `run` never calls them; the module is hidden from the docs and is not a
//! stable API.

use crate::controller::{self, RootView};
use application::BackendInventory;
use gpui::{AnyWindowHandle, App, Entity, WindowOptions};

pub use crate::route::Route;

/// The app-wide setup `run` performs before it opens the window, with every
/// accessibility display flag off, so no test depends on the machine's.
pub fn init(cx: &mut App) {
    controller::init_app(crate::theme::Accessibility::default(), cx);
}

/// The desktop window as `run` opens it.
pub struct MainWindow {
    window: AnyWindowHandle,
    view: Entity<RootView>,
}

impl MainWindow {
    pub fn open(
        inventory: BackendInventory,
        services: crate::DesktopServices,
        options: WindowOptions,
        cx: &mut App,
    ) -> gpui::Result<Self> {
        let (window, view) = controller::open_main_window(inventory, services, options, cx)?;
        Ok(Self { window, view })
    }

    pub fn window(&self) -> AnyWindowHandle {
        self.window
    }

    pub fn route(&self, cx: &App) -> Route {
        self.view.read(cx).ui.route
    }

    pub fn navigate(&self, route: Route, cx: &mut App) {
        self.view.update(cx, |root, cx| root.navigate(route, cx));
    }

    /// Whether the startup installation probe and project list, which load
    /// on worker threads, have reached the view.
    pub fn startup_loaded(&self, cx: &App) -> bool {
        let root = self.view.read(cx);
        root.projects_loaded && !root.installation.catalog.is_empty()
    }
}
