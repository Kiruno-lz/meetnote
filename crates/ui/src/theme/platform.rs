//! System accessibility display flags that change how MeetNote paints.

/// The display flags MeetNote honors. Tests pass fixed values; only the
/// desktop startup reads the system.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Accessibility {
    /// Glass is replaced by the opaque material.
    pub(crate) reduce_transparency: bool,
    /// Animations snap to their end state (`App::set_reduce_motion`).
    pub(crate) reduce_motion: bool,
}

/// The flags from `NSWorkspace` at startup; all off where there is none.
pub(crate) fn system_accessibility() -> Accessibility {
    #[cfg(target_os = "macos")]
    {
        let workspace = objc2_app_kit::NSWorkspace::sharedWorkspace();
        Accessibility {
            reduce_transparency: workspace.accessibilityDisplayShouldReduceTransparency(),
            reduce_motion: workspace.accessibilityDisplayShouldReduceMotion(),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Accessibility::default()
    }
}
