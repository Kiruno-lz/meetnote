# Markdown 历史库 Schema 与 API

> 合并自 docs/markdown-library.md（Schema 细节）+ MARKDOWN_LIBRARY_QUICKREF.md（API 速查）。
> 交付报告类内容已归档到 docs/archive/。

## 概述

Markdown Library 以纯文件系统存储去口语化的会议转录，不依赖 SQLite 或 Tantivy。索引按需从文件系统重建。

## 目录结构

```
meetings/
└── YYYY/
    └── MM/
        └── <slug>/
            ├── meeting.md        # 完整 Markdown 文档
            └── manifest.json     # 索引元数据
```

Slug 格式：`YYYY-MM-DD-kebab-case-title`，碰撞时附加 hash 后缀。

## Frontmatter 格式

每份 meeting.md 以 YAML frontmatter 开头：

```yaml
---
title: Team Standup
date: 2026-05-20
audioHash: abc123def456
speakers: [Alice, Bob]
modelVersion: whisper-large-v3
llmProvider: ollama
schemaVersion: meetnote.markdown.v1
generatedAt: 2026-05-20T12:31:21.565Z
---
```

### 字段约束

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `title` | string | ✓ | 非空 |
| `date` | string | ✓ | YYYY-MM-DD 或 ISO 8601 |
| `audioHash` | string | ✓ | SHA256 前 16 字符 |
| `speakers` | string[] | ✓ | 非空数组 |
| `modelVersion` | string | ✓ | 非空 |
| `llmProvider` | string | ✓ | 必须为 `ollama` 或 `custom` |
| `schemaVersion` | string | ✓ | 必须为 `meetnote.markdown.v1` |
| `generatedAt` | string | ✓ | 有效 ISO 8601 时间戳 |

### 内容约束

- 不得包含摘要、行动项、结论或推断内容
- 保留段落边界和 speaker 标签
- 链接到原始 transcript segment ID（在生成元数据中）

## 核心 API

所有导出位于 `frontend/src/markdown-library-storage.ts`。

### `generateMarkdownContent(options)`

从去口语化结果生成完整 Markdown。

```typescript
interface GenerateMarkdownOptions {
  frontmatter: MarkdownFrontmatter;
  deverbalization: DeverbalizationOutput;
  originalTranscript: string;
  correctionHistory?: readonly SpeakerCorrection[];
}
```

### `saveMeetingMarkdown(options)`

保存到文件系统。

```typescript
interface SaveMeetingMarkdownOptions {
  baseDir: string;    // 如 "~/.meetnote/history"
  markdown: string;   // 完整 Markdown 内容
  audioHash: string;  // SHA256 前 16 字符
  title: string;
  date: string;       // YYYY-MM-DD
}
// 返回 { success, path?, error?, reason? }
```

### `rebuildLibraryIndex(baseDir)`

从文件系统重建索引。

```typescript
const index = await rebuildLibraryIndex("~/.meetnote/history");
// index.meetings: LibraryMeeting[]
// index.stats: { totalMeetings, duplicateCount, corruptedCount }
```

### `searchMeetings(index, query)`

4 维度搜索：

```typescript
interface SearchQuery {
  dateRange?: { from: string; to: string };   // 日期范围
  speakers?: readonly string[];                // 模糊匹配
  titlePrefix?: string;                        // 前缀匹配
  keywords?: readonly string[];                // 全文搜索
}
const results = searchMeetings(index, { speakers: ["Alice"], keywords: ["roadmap"] });
```

## 错误码

| 错误码 | 场景 |
|--------|------|
| `MARKDOWN_SCHEMA_VERSION_INVALID` | Schema 版本不匹配 |
| `MARKDOWN_TITLE_REQUIRED` | 标题为空 |
| `MARKDOWN_DATE_INVALID` | 日期格式无效 |
| `MARKDOWN_AUDIO_HASH_REQUIRED` | 音频 hash 缺失 |
| `MARKDOWN_SPEAKERS_EMPTY` | speakers 数组为空 |
| `MARKDOWN_SPEAKER_INVALID` | speaker 名称无效 |
| `MARKDOWN_MODEL_VERSION_REQUIRED` | 模型版本缺失 |
| `MARKDOWN_LLM_PROVIDER_INVALID` | provider 不是 ollama/custom |
| `MARKDOWN_GENERATED_AT_INVALID` | 时间戳无效 |
| `MARKDOWN_CONTENT_EMPTY` | 内容为空 |
| `MARKDOWN_FRONTMATTER_MISSING` | 未找到 frontmatter |
| `DUPLICATE_IMPORT` | 同音频同时间重复导入 |
| `PATH_TRAVERSAL` | 路径安全检查失败 |
| `IO_ERROR` | 文件系统操作失败 |
| `PERMISSION_DENIED` | 权限不足 |

## 性能特征

| 操作 | 100 会议 | 1000 会议 | 10k 会议 |
|------|----------|-----------|----------|
| 生成 Markdown | <50ms | <50ms | <50ms |
| 保存文件 | <100ms | <100ms | <100ms |
| 重建索引 | <100ms | <500ms | ~4s (⚠️) |
| 搜索 | <50ms | <50ms | <50ms |

> 超过 10k 会议建议升级到 SQLite FTS5 或 Tantivy（非 MVP）。

## 设计决策

- **纯文件存储**：数据完全本地，便于备份/迁移，可脱离 app 使用
- **Slug 格式**：`YYYY-MM-DD-kebab-case-title`，自排序、跨平台兼容
- **去重策略**：`(audioHash, generatedAt)` 作为复合主键
- **索引无状态**：完全从文件系统重建，无需独立 .index 文件
