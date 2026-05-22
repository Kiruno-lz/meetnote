# Markdown Library Schema

## Overview

The Markdown Library stores deverbalised meeting transcripts as a pure-file system without relying on SQLite or Tantivy. Indexes are rebuilt from the file system on demand.

## Directory Structure

```
meetings/
├── YYYY/
│   ├── MM/
│   │   ├── <slug>/
│   │   │   └── meeting.md
│   │   └── <slug>/
│   │       └── meeting.md
│   └── MM/
│       └── <slug>/
│           └── meeting.md
└── library.manifest.json
```

### Path Convention

- `YYYY`: 4-digit year (e.g., 2026)
- `MM`: 2-digit month (01-12)
- `<slug>`: URL-safe identifier derived from meeting title (lowercase, hyphens, no spaces)
  - Example: "Q1 Planning Meeting" → `q1-planning-meeting`

### Example Path

```
meetings/2026/05/team-standup/meeting.md
```

## Frontmatter Format

Each markdown file starts with YAML frontmatter delimited by `---`:

```yaml
---
title: Team Standup
date: 2026-05-20
audioHash: abc123def456
speakers: [Alice, Bob]
modelVersion: claude-3-haiku
llmProvider: ollama
schemaVersion: meetnote.markdown-library.v1
generatedAt: 2026-05-20T12:31:21.565Z
---

## Meeting Content

Content goes here...
```

### Frontmatter Fields

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `title` | string | ✓ | Meeting name, non-empty |
| `date` | string | ✓ | ISO 8601 or YYYY-MM-DD format |
| `audioHash` | string | ✓ | SHA256 or other hash of source audio |
| `speakers` | string[] | ✓ | Non-empty array of speaker names |
| `modelVersion` | string | ✓ | LLM model identifier (e.g., `claude-3-haiku`) |
| `llmProvider` | string | ✓ | Must be `ollama` or `custom` |
| `schemaVersion` | string | ✓ | Must be `meetnote.markdown-library.v1` |
| `generatedAt` | string | ✓ | ISO 8601 timestamp when deverbalisation occurred |

## File Organization Principles

### 1. Immutability

Once created, markdown files are immutable. Corrections require a new file with an updated `generatedAt` timestamp and filename (e.g., `meeting-v2.md`).

### 2. Traceability

Frontmatter preserves:
- `audioHash`: Links to original audio
- `modelVersion`: Tracks which model performed deverbalisation
- `llmProvider`: Documents the LLM provider used
- `generatedAt`: Timestamp for audit trails

### 3. Content Integrity

Content must not include:
- Summary sections
- Action items
- Conclusions
- Inferred content beyond faithful transcript rewriting

These are explicitly validated against and rejected.

## Meeting Content Format

Content follows the transcribed meeting body:

```markdown
## Opening Remarks

Alice: We're here to discuss Q1 priorities...

Bob: I agree. Let's start with...

## Action Items Review

Alice: From last meeting, we had three items...

## Next Steps

Bob: Let's reconvene next week...
```

### Content Rules

- Must be non-empty after trimming whitespace
- Preserves paragraph boundaries and speaker labels
- Links to original transcript segments via source segment IDs (in generated metadata)
- No programmatic manipulation of content; treated as raw text

## Index Rebuild Process

The library manifest tracks metadata for index rebuilding:

```json
{
  "kind": "markdown.library.manifest",
  "version": "meetnote.markdown-library.v1",
  "baseDir": "/meetings",
  "indexedAt": "2026-05-20T12:31:21.565Z",
  "meetingCount": 42
}
```

### Rebuild Steps

1. Walk the filesystem tree: `baseDir/YYYY/MM/<slug>/meeting.md`
2. Parse frontmatter from each file
3. Extract: title, date, speakers, modelVersion, llmProvider
4. Build in-memory index for search/query
5. Update manifest `indexedAt` and `meetingCount`

### Filesystem Scan Strategy

- Scan happens at startup or on-demand
- Concurrent file I/O is safe (read-only)
- Malformed files are logged and skipped
- Index is ephemeral (rebuilt each session)

## Validation Rules

### Frontmatter Validation

- Schema version must match `meetnote.markdown-library.v1`
- Title: non-empty string
- Date: ISO 8601 (`YYYY-MM-DDTHH:mm:ss.SSSZ`) or date-only (`YYYY-MM-DD`)
- Audio hash: non-empty string
- Speakers: non-empty array with non-empty strings
- Model version: non-empty string
- LLM provider: `ollama` or `custom`
- Generated at: valid ISO 8601 timestamp

### Meeting Validation

- Frontmatter must be valid (see above)
- Content must be non-empty after whitespace trim
- Content must be a string

### Manifest Validation

- Version must match schema version
- Base dir: non-empty string (absolute or relative path)
- Indexed at: valid ISO 8601 timestamp
- Meeting count: non-negative integer

## Serialization Format

Frontmatter is serialized as YAML with automatic escaping:

```typescript
// Special characters trigger quoting
title: "Q1 Planning: Strategy & Goals"
speakers: ["Alice Smith", "Bob, Jr."]
```

### Escaping Rules

- Values containing `:`, `[`, `]`, or `,` are double-quoted
- Internal quotes are backslash-escaped
- Speaker arrays are always formatted as `[name1, name2, ...]`

## Error Codes

| Code | Message |
|------|---------|
| `MARKDOWN_SCHEMA_VERSION_INVALID` | Schema version mismatch |
| `MARKDOWN_TITLE_REQUIRED` | Title is empty |
| `MARKDOWN_DATE_INVALID` | Date format is invalid |
| `MARKDOWN_AUDIO_HASH_REQUIRED` | Audio hash is missing |
| `MARKDOWN_SPEAKERS_EMPTY` | Speakers array is empty |
| `MARKDOWN_SPEAKER_INVALID` | Speaker name is invalid |
| `MARKDOWN_MODEL_VERSION_REQUIRED` | Model version is missing |
| `MARKDOWN_LLM_PROVIDER_INVALID` | LLM provider is not `ollama` or `custom` |
| `MARKDOWN_GENERATED_AT_INVALID` | Generated at timestamp is invalid |
| `MARKDOWN_CONTENT_EMPTY` | Meeting content is empty |
| `MARKDOWN_FRONTMATTER_MISSING` | Frontmatter block not found |
| `MARKDOWN_MEETING_COUNT_INVALID` | Meeting count is negative or not an integer |

## API Usage

### Creating a Frontmatter

```typescript
import {
  createMarkdownFrontmatter,
  MARKDOWN_LIBRARY_SCHEMA_VERSION
} from "./markdown-library-schema";

const frontmatter = createMarkdownFrontmatter({
  title: "Team Standup",
  date: "2026-05-20",
  audioHash: "abc123def456",
  speakers: ["Alice", "Bob"],
  modelVersion: "claude-3-haiku",
  llmProvider: "ollama",
  schemaVersion: MARKDOWN_LIBRARY_SCHEMA_VERSION,
  generatedAt: new Date().toISOString()
});
```

### Creating a Meeting

```typescript
import { createMarkdownMeeting } from "./markdown-library-schema";

const meeting = createMarkdownMeeting({
  frontmatter,
  content: "## Meeting Content\n\nDiscussion here..."
});
```

### Serializing Frontmatter

```typescript
import { serializeMarkdownFrontmatter } from "./markdown-library-schema";

const yaml = serializeMarkdownFrontmatter(frontmatter);
const fileContent = `${yaml}\n\n## Meeting Content\n\n...`;
```

### Parsing Frontmatter

```typescript
import { parseMarkdownFrontmatter } from "./markdown-library-schema";

const result = parseMarkdownFrontmatter(fileContent);
if (result.ok) {
  console.log(result.frontmatter);
} else {
  console.error(result.errors);
}
```

### Validation

```typescript
import { validateMarkdownMeeting } from "./markdown-library-schema";

const validation = validateMarkdownMeeting(data);
if (!validation.ok) {
  console.error("Validation errors:", validation.errors);
}
```

## Constraints

- No external dependencies (TypeScript, validation only)
- All types are `readonly` for immutability enforcement
- Validation is deterministic (seeded random numbers if needed)
- Performance: validation completes in <100ms
- No database, no file I/O (external to caller)
