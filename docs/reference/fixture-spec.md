# 测试夹具规范

> 从 FIXTURE_SPEC.md 迁移。定义 5 个 mock meeting 夹具的数据结构。
> 夹具文件位于 `tests/fixtures/`。

## 通用结构

```typescript
{
  audioMetadata: { filename, durationMs, audioHash, uploadedAt };
  asrResult: { requestId, model, segments: AsrTranscriptSegment[] };
  diarizationResult: { requestId, model, speakers, segments: DiarizationSegment[] };
  speakerCorrections?: { timestamp, corrections: Array<{originalLabel, newLabel, speakerId}> };
  deverbalizationOutput: { requestId, provider, model, paragraphs: DeverbalizedParagraph[] };
  expectedMarkdownFrontmatter: { title, date, audioHash, speakers, modelVersion, llmProvider };
}
```

## 5 个场景

| 夹具 | 场景 | Speaker 数 | 时长 | 特点 |
|------|------|------------|------|------|
| `simple-meeting.json` | 单 speaker 产品讨论 | 1 | ~8 分钟 | 简单对话，中文口语词 |
| `multi-speaker-meeting.json` | 4 人技术讨论 | 4 | ~45 分钟 | 多人轮替、少量重叠 |
| `mixed-language-meeting.json` | 中英混合 | 3 | ~25 分钟 | 代码切换、英文技术术语 |
| `corrected-meeting.json` | 含 speaker 修正 | 3 | ~20 分钟 | 从通用标签重命名为真实姓名 |
| `edge-case-meeting.json` | 边界情况 | 3 | ~30 分钟 | 特殊字符、URL、emoji、超长段落 |

## 规格约束

- ID 格式：`asr-seg-0001`, `dia-seg-0001`, `para-0001`
- 时间戳：毫秒，无重叠，gap 100-500ms
- 置信度：ASR 0.85-0.99, Diarization 0.75-0.95
- 音频 hash：16 字符小写 hex（SHA256 前 16 位）
- 文件大小：<500KB 每个
