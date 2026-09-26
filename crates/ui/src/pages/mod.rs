pub(crate) mod assistant;
pub(crate) mod chrome;
pub(crate) mod detail;
pub(crate) mod meetings;
pub(crate) mod models;
pub(crate) mod settings;

#[cfg(test)]
mod tests {
    /// Rendering reads UI state only; file-system facts are probed by the
    /// controller off the main thread and cached.
    const RENDER_SOURCES: &[(&str, &str)] = &[
        ("pages/assistant.rs", include_str!("assistant.rs")),
        ("pages/chrome.rs", include_str!("chrome.rs")),
        ("pages/detail.rs", include_str!("detail.rs")),
        ("pages/meetings.rs", include_str!("meetings.rs")),
        ("pages/models.rs", include_str!("models.rs")),
        ("pages/settings.rs", include_str!("settings.rs")),
        ("components.rs", include_str!("../components.rs")),
    ];

    #[test]
    fn render_sources_do_not_probe_the_file_system() {
        let probes = [
            "std::fs",
            ".is_file(",
            ".is_dir(",
            ".exists(",
            "metadata(",
            ".browse(",
        ];
        let found = RENDER_SOURCES
            .iter()
            .flat_map(|(file, source)| {
                probes
                    .iter()
                    .filter(|probe| source.contains(**probe))
                    .map(move |probe| format!("{file}: {probe}"))
            })
            .collect::<Vec<_>>();
        assert!(found.is_empty(), "render paths probe files: {found:#?}");
    }

    /// Every document write the UI makes names the revision it was read at
    /// (`DocumentCommandService`); the workspace's unchecked writes, which
    /// overwrite newer documents, are not called anywhere in the crate.
    #[test]
    fn the_ui_calls_no_unchecked_document_write() {
        const UNCHECKED: [&str; 3] = ["reparent_project(", "reorder_project(", "update_summary("];
        let found = crate::source_files::SOURCES
            .iter()
            .filter(|(path, _)| *path != "pages/mod.rs")
            .flat_map(|(path, source)| {
                source.lines().enumerate().flat_map(move |(index, line)| {
                    UNCHECKED
                        .iter()
                        .filter(move |call| line.contains(**call))
                        .map(move |call| format!("{path}:{}: {call}", index + 1))
                })
            })
            .collect::<Vec<_>>();
        assert!(found.is_empty(), "unchecked document writes: {found:#?}");
    }

    /// Installation facts, downloads and document changes arrive from
    /// service subscriptions; the controller neither probes files nor runs
    /// threads of its own.
    #[test]
    fn the_controller_does_not_probe_files_or_spawn_threads() {
        let source = include_str!("../controller.rs");
        let found = [
            "std::fs",
            ".is_file(",
            ".is_dir(",
            ".exists(",
            "metadata(",
            ".browse(",
            "thread::",
        ]
        .into_iter()
        .filter(|probe| source.contains(probe))
        .collect::<Vec<_>>();
        assert!(
            found.is_empty(),
            "controller.rs probes or spawns: {found:?}"
        );
    }
}
