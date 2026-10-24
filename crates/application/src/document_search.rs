use crate::DocumentService;
use contracts::DocumentRecord;
use domain::ProjectId;
use runtime::CancellationToken;

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
    /// Literal, case-insensitive search of persisted workspace facts.
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
        if query.chars().count() > 160 {
            return Err("Search text must contain at most 160 characters".into());
        }
        let query = query.to_lowercase();
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

fn find_hit(record: &DocumentRecord, query: &str) -> Option<DocumentSearchHit> {
    let state = &record.state;
    let hit = |text: &str, source, start_ms| {
        snippet(text, query).map(|snippet| DocumentSearchHit {
            document_id: state.id().clone(),
            title: state.title().to_string(),
            source,
            snippet,
            start_ms,
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
            })
        })
        .or_else(|| {
            state.results.transcript.iter().find_map(|segment| {
                hit(
                    &segment.text,
                    SearchSource::Transcript,
                    Some(segment.start_ms),
                )
            })
        })
        .or_else(|| hit(&record.markdown, SearchSource::Markdown, None))
}

fn snippet(text: &str, query: &str) -> Option<String> {
    let lowered = text.to_lowercase();
    let at = lowered.find(query)?;
    // Lowercasing can change UTF-8 lengths (for example İ -> i + dot).
    // Locate the original character rather than slicing by a folded offset.
    let mut folded_bytes = 0;
    let position = text
        .chars()
        .position(|character| {
            let contains =
                folded_bytes + character.to_lowercase().map(char::len_utf8).sum::<usize>() > at;
            folded_bytes += character.to_lowercase().map(char::len_utf8).sum::<usize>();
            contains
        })
        .unwrap_or_default();
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
