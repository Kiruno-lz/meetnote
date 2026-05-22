# Mock Fixture Specifications for Integration Testing

## Overview
Create 5 comprehensive mock meeting fixtures in `tests/fixtures/` directory.
Each fixture is a single JSON file containing complete data for one meeting scenario.

## General Structure for Each Fixture

```typescript
{
  // Metadata
  audioMetadata: {
    filename: string;
    durationMs: number;
    audioHash: string; // sha256 first 16 chars
    uploadedAt: string; // ISO 8601
  };
  
  // ASR Pipeline Output
  asrResult: {
    requestId: string;
    model: { id: string; version: string; };
    segments: AsrTranscriptSegment[];
  };
  
  // Diarization Pipeline Output
  diarizationResult: {
    requestId: string;
    model: { id: string; version: string; };
    speakers: DiarizationSpeakerProfile[];
    segments: DiarizationSegment[];
  };
  
  // Optional: Speaker Corrections (only in corrected-meeting.json)
  speakerCorrections?: {
    timestamp: string;
    corrections: Array<{
      originalLabel: string;
      newLabel: string;
      speakerId: string;
    }>;
  };
  
  // LLM Deverbalization Output
  deverbalizationOutput: {
    requestId: string;
    provider: "ollama" | "custom";
    model: { id: string; version: string; };
    paragraphs: DeverbalizedParagraph[];
  };
  
  // Expected Output
  expectedMarkdownFrontmatter: {
    title: string;
    date: string; // YYYY-MM-DD
    audioHash: string;
    speakers: string[];
    modelVersion: string;
    llmProvider: "ollama" | "custom";
  };
}
```

## Fixture 1: simple-meeting.json

**Scenario**: Single speaker, short product discussion (~8 minutes)

**Speakers**: 
- 1 speaker: "Alice" (Product Manager)

**Content**:
- 12-15 ASR segments
- Simple dialog, minimal cross-talk
- Chinese with common colloquial: 嗯、然后、就是、其实

**Example segments**:
```
[0-2000ms] Alice: "嗯，今天我们看一下Q2产品规划。"
[2000-5000ms] Alice: "然后呢，我们需要完成三个核心功能。"
[5000-8000ms] Alice: "其实这些功能已经在backlog里面了。"
```

**Time span**: 8 minutes
**Total segments**: 14
**Paragraphs**: 7-8

---

## Fixture 2: multi-speaker-meeting.json

**Scenario**: 4 speakers, technical discussion (~45 minutes)

**Speakers**:
- Speaker 1: "Alice" (Engineer)
- Speaker 2: "Bob" (Product Manager)
- Speaker 3: "Charlie" (Designer)
- Speaker 4: "Diana" (QA Lead)

**Content**:
- 60-70 ASR segments
- Multiple speakers, frequent turn-taking
- Some overlaps/cross-talk
- Mix of questions, answers, discussions

**Time span**: 45 minutes
**Total segments**: 65
**Paragraphs**: 20-25
**Colloquial terms**: 对吧、你看、我觉得、就这样、嗯这个

---

## Fixture 3: mixed-language-meeting.json

**Scenario**: Chinese-English mixed discussion (~25 minutes)

**Speakers**:
- Speaker 1: "Alice" (中文)
- Speaker 2: "Bob" (English name, code-switch)
- Speaker 3: "Charlie" (中英混合)

**Content**:
- 40-50 segments mixing Chinese and English
- Code-switching: "我们需要optimize一下API latency"
- Technical terms in English: API, database, query, etc.
- Chinese colloquial preserved in English segments

**Example**:
```
Alice: "我们今天讨论一下API的性能问题"
Bob: "Yeah, we need to optimize the database queries"
Charlie: "其实这个和缓存设计有关，我们应该implement Redis"
```

**Language tags**: "zh", "en", "mixed"
**Time span**: 25 minutes
**Total segments**: 45
**Paragraphs**: 15-18

---

## Fixture 4: corrected-meeting.json

**Scenario**: Meeting with speaker correction history (~20 minutes)

**Speakers** (before correction):
- Speaker 1: "Speaker A"
- Speaker 2: "Speaker B"
- Speaker 3: "Speaker C"

**Speakers** (after correction):
- "Speaker A" → "Alice"
- "Speaker B" → "Bob"
- "Speaker C" → "Charlie"

**Content**:
- 30-40 ASR segments
- Diarization initial output with generic labels
- Speaker corrections applied after initial processing
- Verify corrections persist through generation

**Time span**: 20 minutes
**Total segments**: 35
**Paragraphs**: 12-15
**Correction timestamp**: ISO 8601, within meeting time range

---

## Fixture 5: edge-case-meeting.json

**Scenario**: Stress test - special characters, long paragraphs, format boundaries (~30 minutes)

**Content**:
1. Special characters: 「」『』、，。；：？！…… 
2. URLs and code snippets in text
3. Very long paragraph (500+ chars without break)
4. Numbered lists: 1. 第一点 2. 第二点
5. Emotional expressions: 😊 (may test escaping)
6. Multiple punctuation sequences
7. Inconsistent spacing

**Example text**:
```
Alice: "我们看看这个：https://example.com/api?param=value&other=test"
Bob: "好的，核心步骤是：
  1. 首先检查数据库连接
  2. 然后执行查询优化……等等，这个有点复杂
  3. 最后输出结果"
Charlie: "嗯……我觉得这个……问题可能需要用到一些更复杂的技术，比如说……"
```

**Speakers**: 3
**Time span**: 30 minutes
**Total segments**: 50-60
**Paragraphs**: 18-22
**Edge cases tested**:
- Markdown special chars: `*`, `_`, `[`, `]`, `#`
- YAML special chars in frontmatter
- Very long speaker names (20+ chars)
- Paragraph with no line breaks (one long segment)

---

## Data Generation Rules

### IDs
- ASR segments: `asr-seg-{4-digit zero-padded index}` (asr-seg-0001, asr-seg-0002, ...)
- Diarization segments: `dia-seg-{4-digit}` 
- Speakers: `speaker-{name-lowercase-no-spaces}` (speaker-alice, speaker-bob)
- Paragraphs: `para-{4-digit}`

### Timestamps
- All in milliseconds
- No overlap between consecutive segments (endMs of segment N ≤ startMs of segment N+1)
- Realistic gaps: 100-500ms between segments
- Total duration must match audioMetadata.durationMs

### Confidence Scores
- ASR confidence: 0.85-0.99 (realistic range)
- Diarization confidence: 0.75-0.95
- No overlaps for single-speaker stretches
- Overlaps only for multi-speaker passages (overlap=true)

### Audio Hash
- Format: lowercase hex, exactly 16 characters (first 16 of sha256)
- Must be unique per fixture
- Example: "a1b2c3d4e5f6g7h8" (note: g-h invalid in hex, use 0-9a-f only)
- Corrected example: "a1b2c3d4e5f6a7b8"

### Generated At
- ISO 8601 UTC format
- Timezone: Z
- Should be after audioMetadata.uploadedAt
- Examples: "2024-05-20T15:30:45.123Z"

### Model Versions
- ASR model: "whisper-v1.0"
- Diarization model: "pyannote-v2.1"
- LLM model: "qwen-7b-v1.0" or similar

### Temperature/Parameters
- ASR temperature: 0.3-0.5 (low, for consistency)
- LLM temperature: 0.7 (medium, for natural deverbalization)
- maxOutputTokens: 2000-4000

---

## Validation Checklist

For each fixture:
- [ ] All IDs properly formatted and unique
- [ ] All timestamps in correct range (0 ≤ t ≤ totalMs)
- [ ] No timestamp overlaps within same segment sequence
- [ ] All speaker IDs in diarization referenced in deverbalization
- [ ] All paragraph sourceSegmentIds reference valid ASR segments
- [ ] Confidence scores in valid range [0, 1]
- [ ] No forbidden MVP fields in deverbalization output
- [ ] Frontmatter speakers match paragraph speaker IDs (translated)
- [ ] File is valid JSON (parseable without errors)
- [ ] File size < 500KB

---

## File Naming Convention
- `/tests/fixtures/simple-meeting.json`
- `/tests/fixtures/multi-speaker-meeting.json`
- `/tests/fixtures/mixed-language-meeting.json`
- `/tests/fixtures/corrected-meeting.json`
- `/tests/fixtures/edge-case-meeting.json`

All fixtures in single directory, one JSON file per scenario.
