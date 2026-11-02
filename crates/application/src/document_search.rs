use crate::DocumentService;
use contracts::DocumentRecord;
use domain::ProjectId;
use regex::{Regex, RegexBuilder};
use runtime::CancellationToken;

/// A bounded literal query shared by search and evidence validation.
#[derive(Debug, Clone)]
pub struct DocumentSearchQuery(Regex);

impl DocumentSearchQuery {
    pub fn new(query: &str) -> Result<Self, String> {
        let query = query.trim();
        if query.is_empty() {
            return Err("Search text must not be empty".into());
        }
        if query.chars().count() > 160 {
            return Err("Search text must contain at most 160 characters".into());
        }
        RegexBuilder::new(&regex::escape(query))
            .case_insensitive(true)
            .build()
            .map(Self)
            .map_err(|error| format!("Cannot prepare search text: {error}"))
    }

    pub fn is_match(&self, text: &str) -> bool {
        self.0.is_match(text)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchSource {
    Title,
    Tag,
    Summary,
    Transcript,
    Markdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentSearchHit {
    pub document_id: ProjectId,
    pub title: String,
    pub source: SearchSource,
    pub snippet: String,
    pub start_ms: Option<u64>,
    pub document_revision: u64,
    pub transcript_target: Option<TranscriptSearchTarget>,
}

/// Identity of a rendered raw or speaker-aligned paragraph. Alignment may
/// split one ASR segment, so an ID alone is not a unique paragraph address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptSearchTarget {
    pub segment_id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker_id: Option<String>,
}

impl From<&domain::TranscriptSegment> for TranscriptSearchTarget {
    fn from(segment: &domain::TranscriptSegment) -> Self {
        Self {
            segment_id: segment.id.clone(),
            start_ms: segment.start_ms,
            end_ms: segment.end_ms,
            speaker_id: None,
        }
    }
}

impl From<&domain::SpeakerTaggedSegment> for TranscriptSearchTarget {
    fn from(segment: &domain::SpeakerTaggedSegment) -> Self {
        Self {
            segment_id: segment.transcript_segment_id.clone(),
            start_ms: segment.start_ms,
            end_ms: segment.end_ms,
            speaker_id: Some(segment.speaker_id.clone()),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DocumentSearchResults {
    pub hits: Vec<DocumentSearchHit>,
    /// More matches exist than the requested result limit.
    pub truncated: bool,
    /// Documents that could not be read, with their repository error.
    pub unreadable: Vec<(ProjectId, String)>,
    pub unreadable_count: usize,
}

impl DocumentService {
    /// Literal search of persisted facts using Unicode simple case folding.
    /// Call off the UI thread; no index or model is required.
    pub fn search(
        &self,
        query: &str,
        limit: usize,
        cancellation: &CancellationToken,
    ) -> Result<DocumentSearchResults, String> {
        let mut result = DocumentSearchResults::default();
        let query = query.trim();
        if query.is_empty() || limit == 0 || cancellation.is_cancelled() {
            return Ok(result);
        }
        let query = DocumentSearchQuery::new(query)?;
        let documents = self.frozen();
        // Metadata scanning holds repository locks. Read records only after
        // the scan returns, never recursively inside its visitor.
        let mut ids = Vec::new();
        documents.scan_metadata(&mut |metadata| {
            if cancellation.is_cancelled() {
                return Ok(false);
            }
            ids.push(metadata.document_id);
            Ok(true)
        })?;
        ids.sort();
        for id in ids {
            if cancellation.is_cancelled() {
                break;
            }
            let record = match documents.read(&id) {
                Ok(Some(record)) => record,
                Ok(None) => continue, // Removed by an external editor since scanning.
                Err(error) => {
                    result.unreadable_count += 1;
                    if result.unreadable.len() < 10 {
                        result.unreadable.push((id, error));
                    }
                    continue;
                }
            };
            if let Some(hit) = find_hit(&record, &query) {
                if result.hits.len() == limit.min(50) {
                    result.truncated = true;
                    break;
                }
                result.hits.push(hit);
            }
        }
        Ok(result)
    }
}

fn find_hit(record: &DocumentRecord, query: &DocumentSearchQuery) -> Option<DocumentSearchHit> {
    let state = &record.state;
    let hit = |text: &str, source, start_ms| {
        snippet(text, query).map(|snippet| DocumentSearchHit {
            document_id: state.id().clone(),
            title: state.title().to_string(),
            source,
            snippet,
            start_ms,
            document_revision: state.revision(),
            transcript_target: None,
        })
    };
    hit(state.title(), SearchSource::Title, None)
        .or_else(|| {
            state
                .content
                .tags
                .iter()
                .find_map(|tag| hit(tag, SearchSource::Tag, None))
        })
        .or_else(|| {
            state
                .content
                .summary
                .as_deref()
                .and_then(|text| hit(text, SearchSource::Summary, None))
        })
        .or_else(|| {
            state.results.speaker_segments.iter().find_map(|segment| {
                hit(
                    &segment.text,
                    SearchSource::Transcript,
                    Some(segment.start_ms),
                )
                .map(|mut hit| {
                    hit.transcript_target = Some(segment.into());
                    hit
                })
            })
        })
        .or_else(|| {
            state.results.transcript.iter().find_map(|segment| {
                hit(
                    &segment.text,
                    SearchSource::Transcript,
                    Some(segment.start_ms),
                )
                .map(|mut hit| {
                    hit.transcript_target = Some(segment.into());
                    hit
                })
            })
        })
        .or_else(|| hit(&record.markdown, SearchSource::Markdown, None))
}

fn snippet(text: &str, query: &DocumentSearchQuery) -> Option<String> {
    // The matcher returns a UTF-8 boundary in the original text, so the
    // excerpt does not need a second representation or folded-offset mapping.
    let at = query.0.find(text)?.start();
    let position = text[..at].chars().count();
    let start = position.saturating_sub(20);
    let mut characters = text.chars().skip(start);
    let excerpt: String = characters
        .by_ref()
        .take(180)
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    Some(format!(
        "{}{}{}",
        if start > 0 { "…" } else { "" },
        excerpt,
        if characters.next().is_some() {
            "…"
        } else {
            ""
        }
    ))
}
