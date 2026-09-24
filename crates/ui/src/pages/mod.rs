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
}
