use application::{DocumentRepository, DocumentService, SearchSource};
use domain::{DocumentState, ProjectId, TranscriptSegment};
use persistence::{FileDocumentRepository, InMemoryDocumentRepository};
use runtime::CancellationToken;
use std::sync::Arc;

fn document(id: &str, title: &str) -> DocumentState {
    DocumentState::new(ProjectId::new(id), title, "audio.wav")
}

#[test]
fn search_finds_all_persisted_fields_and_collapsed_children_once() {
    let repository = Arc::new(InMemoryDocumentRepository::default());
    let mut tagged = document("tagged", "Unrelated");
    tagged.content.tags.push("RoadMap".into());
    let mut summary = document("summary", "Unrelated");
    summary.content.summary = Some("The roadmap is ready".into());
    let mut child = document("child", "Hidden child");
    child.hierarchy.parent_id = Some(ProjectId::new("title"));
    child.hierarchy.depth = 1;
    child.results.transcript.push(TranscriptSegment {
        id: "segment-1".into(),
        start_ms: 62_000,
        end_ms: 65_000,
        text: "Discuss the roadmap tomorrow".into(),
    });
    for (state, markdown) in [
        (document("title", "Roadmap review"), "roadmap"),
        (tagged, ""),
        (summary, ""),
        (child, ""),
        (
            document("markdown", "Unrelated"),
            "# Notes\nThe ROADMAP is ready",
        ),
    ] {
        repository.save(state, Some(markdown.into())).unwrap();
    }
    let service = DocumentService::new(repository);
    let result = service
        .search("  roadmap  ", 50, &CancellationToken::new())
        .unwrap();
    assert_eq!(result.hits.len(), 5);
    for (id, source) in [
        ("title", SearchSource::Title),
        ("tagged", SearchSource::Tag),
        ("summary", SearchSource::Summary),
        ("child", SearchSource::Transcript),
        ("markdown", SearchSource::Markdown),
    ] {
        assert_eq!(
            result
                .hits
                .iter()
                .find(|hit| hit.document_id.as_str() == id)
                .unwrap()
                .source,
            source
        );
    }
    assert_eq!(
        result
            .hits
            .iter()
            .find(|hit| hit.document_id.as_str() == "child")
            .unwrap()
            .start_ms,
        Some(62_000)
    );
    assert!(!result.truncated);
}

#[test]
fn search_addresses_split_alignment_and_raw_paragraphs_without_using_row_indices() {
    let repository = Arc::new(InMemoryDocumentRepository::default());
    let mut state = document("meeting", "Review");
    state.schema.revision = 7;
    state.results.transcript = vec![TranscriptSegment {
        id: "asr-1".into(),
        start_ms: 0,
        end_ms: 8000,
        text: "Opening. Raw-only evidence.".into(),
    }];
    state.results.speaker_segments = vec![
        domain::SpeakerTaggedSegment {
            transcript_segment_id: "asr-1".into(),
            start_ms: 0,
            end_ms: 4000,
            text: "Opening.".into(),
            speaker_id: "speaker-0".into(),
            speaker_label: None,
            speaker_profile_id: None,
        },
        domain::SpeakerTaggedSegment {
            transcript_segment_id: "asr-1".into(),
            start_ms: 4000,
            end_ms: 8000,
            text: "Aligned evidence.".into(),
            speaker_id: "speaker-1".into(),
            speaker_label: None,
            speaker_profile_id: None,
        },
    ];
    repository.save(state, None).unwrap();
    let service = DocumentService::new(repository);
    for (query, start, end, speaker) in [
        ("Aligned evidence", 4000, 8000, Some("speaker-1")),
        ("Raw-only", 0, 8000, None),
    ] {
        let result = service
            .search(query, 50, &CancellationToken::new())
            .unwrap();
        let hit = &result.hits[0];
        assert_eq!(hit.document_revision, 7);
        assert_eq!(
            hit.transcript_target,
            Some(application::TranscriptSearchTarget {
                segment_id: "asr-1".into(),
                start_ms: start,
                end_ms: end,
                speaker_id: speaker.map(str::to_string),
            })
        );
    }
}

#[test]
fn search_unicode_snippet_keeps_the_actual_match_and_bounds_its_length() {
    let repository = Arc::new(InMemoryDocumentRepository::default());
    repository
        .save(
            document("chinese", "Meeting"),
            Some(format!(
                "{}İ\n项目预算🎉{}",
                "前文".repeat(300),
                "后文".repeat(300)
            )),
        )
        .unwrap();
    let service = DocumentService::new(repository);
    let result = service
        .search("项目预算🎉", 50, &CancellationToken::new())
        .unwrap();
    assert_eq!(result.hits.len(), 1);
    let snippet = &result.hits[0].snippet;
    assert!(snippet.contains("项目预算🎉"));
    assert!(snippet.chars().count() <= 182);
    assert!(snippet.starts_with('…') && snippet.ends_with('…'));
}

#[test]
fn search_limits_matches_and_reads_external_changes_again() {
    let repository = Arc::new(InMemoryDocumentRepository::default());
    for id in ["a", "b", "c"] {
        repository
            .save(document(id, "Meeting"), Some("budget".into()))
            .unwrap();
    }
    let service = DocumentService::new(repository.clone());
    let result = service
        .search("budget", 2, &CancellationToken::new())
        .unwrap();
    assert_eq!(result.hits.len(), 2);
    assert!(result.truncated);
    repository
        .save(document("a", "Meeting"), Some("changed".into()))
        .unwrap();
    let result = service
        .search("budget", 2, &CancellationToken::new())
        .unwrap();
    assert_eq!(result.hits.len(), 2);
    assert!(!result.truncated);
    assert!(result
        .hits
        .iter()
        .all(|hit| hit.document_id.as_str() != "a"));
    assert!(service
        .search(" ", 50, &CancellationToken::new())
        .unwrap()
        .hits
        .is_empty());
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(service
        .search("budget", 50, &cancelled)
        .unwrap()
        .hits
        .is_empty());
}

#[test]
fn search_reports_unreadable_files_without_hiding_other_matches() {
    let root = std::env::temp_dir().join(format!("meetnote-search-{}", ProjectId::new_uuid()));
    let repository = Arc::new(FileDocumentRepository::open(&root).unwrap());
    repository
        .save(document("good", "Budget"), Some("Notes".into()))
        .unwrap();
    for index in 0..12 {
        let id = format!("bad-{index}");
        repository
            .save(document(&id, "Budget"), Some("Notes".into()))
            .unwrap();
        std::fs::write(root.join(&id).join("document.md"), [0xff]).unwrap();
    }
    let service = DocumentService::new(repository);
    let found = service
        .search("budget", 50, &CancellationToken::new())
        .unwrap();
    assert_eq!(found.hits.len(), 1);
    assert_eq!(found.hits[0].document_id.as_str(), "good");
    assert_eq!(found.unreadable_count, 12);
    assert_eq!(found.unreadable.len(), 10);
    assert!(found
        .unreadable
        .iter()
        .all(|(_, error)| error.contains("document.md")));
    std::fs::write(root.join("good/state.json"), "invalid JSON").unwrap();
    let error = service
        .search("budget", 50, &CancellationToken::new())
        .unwrap_err();
    assert!(
        error.contains("state.json"),
        "metadata scan failures remain visible"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn search_hard_caps_results_and_treats_query_as_literal_text() {
    let repository = Arc::new(InMemoryDocumentRepository::default());
    for index in 0..51 {
        repository
            .save(document(&format!("doc-{index:02}"), "Budget review"), None)
            .unwrap();
    }
    let service = DocumentService::new(repository);
    let found = service
        .search("budget", usize::MAX, &CancellationToken::new())
        .unwrap();
    assert_eq!(found.hits.len(), 50);
    assert!(found.truncated);
    assert!(service
        .search(".*", 50, &CancellationToken::new())
        .unwrap()
        .hits
        .is_empty());
    assert!(service
        .search(&"预".repeat(161), 50, &CancellationToken::new())
        .is_err());
}
