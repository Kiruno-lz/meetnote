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

    /// The crate's code without its tests: test-only modules are skipped and
    /// a file's `#[cfg(test)] mod tests`, always its last item, is cut off.
    fn production_sources() -> impl Iterator<Item = (&'static str, &'static str)> {
        const TEST_ONLY: [&str; 4] = [
            "document_tests.rs",
            "source_files.rs",
            "speaker_tests.rs",
            "workspace_tests.rs",
        ];
        crate::source_files::SOURCES
            .iter()
            .filter(|(path, _)| !TEST_ONLY.contains(path))
            .map(|(path, source)| {
                let end = source
                    .find("#[cfg(test)]\nmod tests")
                    .unwrap_or(source.len());
                (*path, &source[..end])
            })
    }

    #[test]
    fn production_sources_leave_out_the_tests() {
        let lib = production_sources()
            .find(|(path, _)| *path == "lib.rs")
            .map(|(_, source)| source)
            .unwrap();
        assert!(lib.contains("pub use controller::"));
        assert!(!lib.contains("mod tests"));
        assert!(production_sources().all(|(path, _)| path != "speaker_tests.rs"));
    }

    /// Speakers are corrected by revision-checked commands on the speakers
    /// the document has: the UI neither calls the workspace's unchecked
    /// merge nor names a speaker of its own.
    #[test]
    fn the_ui_merges_no_fixed_speaker_and_no_unchecked_merge() {
        const FORBIDDEN: [&str; 3] = [".merge_speaker(", "::merge_speaker(", "speaker-1"];
        let found = production_sources()
            .flat_map(|(path, source)| {
                source.lines().enumerate().flat_map(move |(index, line)| {
                    FORBIDDEN
                        .iter()
                        .filter(move |pattern| line.contains(**pattern))
                        .map(move |pattern| format!("{path}:{}: {pattern}", index + 1))
                })
            })
            .collect::<Vec<_>>();
        assert!(
            found.is_empty(),
            "fixed or unchecked speaker merges: {found:#?}"
        );
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
