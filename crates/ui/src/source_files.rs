//! Every source file of the crate for the source-scan tests (translation
//! keys, color literals). The guard below fails when a module file is added
//! or removed without updating `SOURCES`.

use std::collections::BTreeSet;

/// Every `.rs` file under `crates/ui/src`, by its path relative to it.
pub(crate) const SOURCES: &[(&str, &str)] = &[
    ("assistant_ui.rs", include_str!("assistant_ui.rs")),
    ("audio.rs", include_str!("audio.rs")),
    ("components.rs", include_str!("components.rs")),
    ("controller.rs", include_str!("controller.rs")),
    ("controls/dialog.rs", include_str!("controls/dialog.rs")),
    ("controls/mod.rs", include_str!("controls/mod.rs")),
    ("controls/progress.rs", include_str!("controls/progress.rs")),
    ("controls/select.rs", include_str!("controls/select.rs")),
    (
        "controls/text_area.rs",
        include_str!("controls/text_area.rs"),
    ),
    (
        "controls/text_field.rs",
        include_str!("controls/text_field.rs"),
    ),
    ("document_tests.rs", include_str!("document_tests.rs")),
    ("editor.rs", include_str!("editor.rs")),
    ("i18n.rs", include_str!("i18n.rs")),
    ("lib.rs", include_str!("lib.rs")),
    ("pages/assistant.rs", include_str!("pages/assistant.rs")),
    ("pages/chrome.rs", include_str!("pages/chrome.rs")),
    ("pages/detail.rs", include_str!("pages/detail.rs")),
    ("pages/meetings.rs", include_str!("pages/meetings.rs")),
    ("pages/mod.rs", include_str!("pages/mod.rs")),
    ("pages/models.rs", include_str!("pages/models.rs")),
    ("pages/settings.rs", include_str!("pages/settings.rs")),
    ("route.rs", include_str!("route.rs")),
    ("source_files.rs", include_str!("source_files.rs")),
    ("speaker_tests.rs", include_str!("speaker_tests.rs")),
    ("speakers.rs", include_str!("speakers.rs")),
    ("state.rs", include_str!("state.rs")),
    ("test_support.rs", include_str!("test_support.rs")),
    ("theme/component.rs", include_str!("theme/component.rs")),
    ("theme/contrast.rs", include_str!("theme/contrast.rs")),
    ("theme/edge_fade.rs", include_str!("theme/edge_fade.rs")),
    ("theme/frost.rs", include_str!("theme/frost.rs")),
    ("theme/material.rs", include_str!("theme/material.rs")),
    ("theme/mod.rs", include_str!("theme/mod.rs")),
    ("theme/platform.rs", include_str!("theme/platform.rs")),
    ("theme/tokens.rs", include_str!("theme/tokens.rs")),
    ("workspace_tests.rs", include_str!("workspace_tests.rs")),
];

/// The modules a file declares with `mod name;`.
fn declared_modules(source: &str) -> Vec<&str> {
    source
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let line = ["pub(crate) ", "pub "]
                .iter()
                .find_map(|visibility| line.strip_prefix(visibility))
                .unwrap_or(line);
            line.strip_prefix("mod ")?.strip_suffix(';')
        })
        .collect()
}

/// The file of every module declared from `lib.rs` down. `controls`,
/// `pages` and `theme` are the directory modules.
fn declared_source_files() -> BTreeSet<String> {
    let directories = [
        ("controls", include_str!("controls/mod.rs")),
        ("pages", include_str!("pages/mod.rs")),
        ("theme", include_str!("theme/mod.rs")),
    ];
    let mut files = BTreeSet::from(["lib.rs".to_string()]);
    for module in declared_modules(include_str!("lib.rs")) {
        match directories
            .iter()
            .find(|(directory, _)| *directory == module)
        {
            Some((directory, source)) => {
                files.insert(format!("{directory}/mod.rs"));
                files.extend(
                    declared_modules(source)
                        .into_iter()
                        .map(|child| format!("{directory}/{child}.rs")),
                );
            }
            None => {
                files.insert(format!("{module}.rs"));
            }
        }
    }
    files
}

#[test]
fn sources_list_every_declared_module_file() {
    let listed = SOURCES
        .iter()
        .map(|(path, _)| path.to_string())
        .collect::<BTreeSet<_>>();
    assert_eq!(listed.len(), SOURCES.len(), "a file is listed twice");
    assert_eq!(listed, declared_source_files());
}
