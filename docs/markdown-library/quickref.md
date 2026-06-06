# Markdown 历史库 — 快速参考

> 从 MARKDOWN_LIBRARY_QUICKREF.md 提纯。API 速查表，详细说明见 schema.md。

## 10 秒快速开始

```typescript
import {
  generateMarkdownContent, saveMeetingMarkdown,
  rebuildLibraryIndex, searchMeetings,
  createMarkdownFrontmatter
} from "frontend/src/markdown-library-storage";

// 1. 生成
const markdown = generateMarkdownContent({
  frontmatter: createMarkdownFrontmatter({
    title: "Sprint Planning", date: "2026-05-21",
    audioHash: "a1b2c3d4e5f6g7h8", speakers: ["Alice", "Bob"],
    modelVersion: "1.0", llmProvider: "ollama",
    schemaVersion: "meetnote.markdown.v1",
    generatedAt: new Date().toISOString()
  }),
  deverbalization: llmOutput,
  originalTranscript: asrTranscript,
  correctionHistory: corrections
});

// 2. 保存
const result = await saveMeetingMarkdown({
  baseDir: "~/.meetnote/history", markdown,
  audioHash: "a1b2c3d4e5f6g7h8", title: "Sprint Planning", date: "2026-05-21"
});

// 3. 搜索
const index = await rebuildLibraryIndex("~/.meetnote/history");
const results = searchMeetings(index, { speakers: ["Alice"] });
```

## 核心函数一览

| 函数 | 输入 | 输出 | 用途 |
|------|------|------|------|
| `createMarkdownFrontmatter()` | MarkdownFrontmatter 字段 | MarkdownFrontmatter 对象 | 构造 frontmatter |
| `generateMarkdownContent()` | GenerateMarkdownOptions | 完整 Markdown 字符串 | 生成会议文档 |
| `saveMeetingMarkdown()` | SaveMeetingMarkdownOptions | `{ success, path?, error?, reason? }` | 写入文件系统 |
| `rebuildLibraryIndex()` | baseDir string | LibraryIndex | 从文件重建索引 |
| `searchMeetings()` | LibraryIndex + SearchQuery | MeetingSummary[] | 多维度搜索 |

输出 Markdown 格式：

```markdown
---
title: ...
date: ...
---
**Alice**: First paragraph...

**Bob**: Second paragraph...

## 原始转录 (附录)

## Speaker 修正记录 (附录)
```

## 搜索语法

```typescript
// 日期范围
searchMeetings(index, { dateRange: { from: "2026-05-01", to: "2026-05-31" } })
// 发言人 (模糊)
searchMeetings(index, { speakers: ["Alice"] })
// 标题前缀
searchMeetings(index, { titlePrefix: "Product" })
// 正文关键词
searchMeetings(index, { keywords: ["roadmap"] })
// 组合
searchMeetings(index, { dateRange: { from: "2026-05-01", to: "2026-05-31" }, speakers: ["Alice"] })
```

## 常见场景

```typescript
// 完整工作流
async function handleMeetingCompletion(audioFile, asrResult, diarizationResult, llmResult, corrections) {
  const frontmatter = createMarkdownFrontmatter({
    title: audioFile.name.replace(/\.[^.]+$/, ""),
    date: new Date().toISOString().split("T")[0],
    audioHash: computeHash(audioFile).slice(0, 16),
    speakers: extractSpeakerNames(corrections),
    modelVersion: "whisper-v3", llmProvider: "ollama",
    schemaVersion: "meetnote.markdown.v1",
    generatedAt: new Date().toISOString()
  });
  const markdown = generateMarkdownContent({ frontmatter, deverbalization: llmResult, originalTranscript: asrResult.fullText, correctionHistory: corrections });
  const result = await saveMeetingMarkdown({ baseDir: getUserHistoryPath(), markdown, audioHash: frontmatter.audioHash, title: frontmatter.title, date: frontmatter.date });
  if (result.success) showNotification(`✓ 已保存到 ${result.path}`);
}

// 灾难恢复
async function recoverAfterCacheLoss() {
  const index = await rebuildLibraryIndex("~/.meetnote/history");
  return index;
}
```

## 测试验证

```bash
cd frontend && bun test markdown-library     # L1
cd tests/api/ts && bun test markdown-library # L2（当前因路径问题失败）
```
