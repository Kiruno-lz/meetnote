# Markdown 历史库 - 快速参考

## 10 秒快速开始

```typescript
import { 
  generateMarkdownContent, 
  saveMeetingMarkdown, 
  rebuildLibraryIndex, 
  searchMeetings,
  createMarkdownFrontmatter 
} from "frontend/src/markdown-library-storage";

// 1. 生成 Markdown
const markdown = generateMarkdownContent({
  frontmatter: createMarkdownFrontmatter({
    title: "Sprint Planning",
    date: "2026-05-21",
    audioHash: "a1b2c3d4e5f6g7h8",
    speakers: ["Alice", "Bob"],
    modelVersion: "1.0",
    llmProvider: "ollama",
    schemaVersion: "meetnote.markdown.v1",
    generatedAt: new Date().toISOString()
  }),
  deverbalization: deverbOutputFromLlm,
  originalTranscript: asrTranscript,
  correctionHistory: speakerCorrections
});

// 2. 保存
const result = await saveMeetingMarkdown({
  baseDir: "~/.meetnote/history",
  markdown,
  audioHash: "a1b2c3d4e5f6g7h8",
  title: "Sprint Planning",
  date: "2026-05-21"
});

// 3. 搜索
const index = await rebuildLibraryIndex("~/.meetnote/history");
const results = searchMeetings(index, { speakers: ["Alice"] });
```

## 文件位置

| 文件 | 用途 | 路径 |
|------|------|------|
| 主要 API | 所有导出函数 | `frontend/src/markdown-library-storage.ts` |
| Schema | 类型定义 | `frontend/src/markdown-library-schema.ts` |
| Generator | Markdown 生成 | `frontend/src/markdown-library-generator.ts` |
| 文档 | 完整说明 | `docs/markdown-library.md` |
| 测试 | 全部测试用例 | `frontend/tests/markdown-library-*.test.ts` |
| 集成测试 | E2E 测试 | `tests/api/ts/markdown-library-*.test.ts` |

## 核心函数

### `generateMarkdownContent(options)`

从去口语化结果生成完整 Markdown。

```typescript
interface GenerateMarkdownOptions {
  frontmatter: MarkdownFrontmatter;
  deverbalization: DeverbalizationOutput;
  originalTranscript: string;
  correctionHistory?: readonly SpeakerCorrection[];
}

// 返回值: 完整的 Markdown 字符串
const markdown = generateMarkdownContent(options);
```

**输出格式**:
```markdown
---
title: ...
date: ...
[其他 frontmatter]
---

**Alice**: First paragraph text...

**Bob**: Second paragraph text...

## 原始转录 (附录)
...

## Speaker 修正记录 (附录)
...
```

### `saveMeetingMarkdown(options)`

保存 Markdown 到文件系统。

```typescript
interface SaveMeetingMarkdownOptions {
  baseDir: string;           // e.g., "~/.meetnote/history"
  markdown: string;          // 完整 Markdown 内容
  audioHash: string;         // SHA256 前 16 字符
  title: string;            // 会议标题
  date: string;             // ISO 8601 格式: YYYY-MM-DD
}

// 返回值
interface SaveResult {
  success: boolean;
  path?: string;            // e.g., "meetings/2026/05/2026-05-21-sprint-planning/"
  error?: string;
  reason?: string;
}

const result = await saveMeetingMarkdown(options);
```

**生成的结构**:
```
baseDir/
└── meetings/
    └── 2026/
        └── 05/
            └── 2026-05-21-sprint-planning/
                ├── meeting.md
                └── manifest.json
```

### `rebuildLibraryIndex(baseDir)`

从文件系统重建完整索引。

```typescript
// 输入: 历史库根目录
const index = await rebuildLibraryIndex("~/.meetnote/history");

// 返回值
interface LibraryIndex {
  meetings: readonly LibraryMeeting[];  // 所有会议的元数据
  stats: {
    totalMeetings: number;
    duplicateCount: number;
    corruptedCount: number;
  };
}
```

### `searchMeetings(index, query)`

多维度搜索。

```typescript
interface SearchQuery {
  dateRange?: {
    from: string;  // YYYY-MM-DD
    to: string;
  };
  speakers?: readonly string[];  // 模糊匹配
  titlePrefix?: string;          // 前缀匹配
  keywords?: readonly string[];  // 全文搜索（会议正文）
}

const results = searchMeetings(index, {
  dateRange: { from: "2026-05-01", to: "2026-05-31" },
  speakers: ["Alice"],
  keywords: ["roadmap"]
});

// 返回值: readonly MeetingSummary[]
// 每个 summary 包含: title, date, speakers, path, hash
```

## 常见场景

### 场景 1: 完整工作流（导入→修正→保存）

```typescript
// 用户导入音频后，经过 ASR、Diarization、LLM 处理
async function handleMeetingCompletion(
  audioFile: File,
  asrResult: AsrOutput,
  diarizationResult: DiarizationOutput,
  llmResult: DeverbalizationOutput,
  speakerCorrections: SpeakerCorrections
) {
  // 1. 生成 Frontmatter
  const frontmatter = createMarkdownFrontmatter({
    title: audioFile.name.replace(/\.[^.]+$/, ""),
    date: new Date().toISOString().split("T")[0],
    audioHash: computeHash(audioFile).slice(0, 16),
    speakers: extractSpeakerNames(speakerCorrections),
    modelVersion: "whisper-v3",
    llmProvider: "ollama",
    schemaVersion: "meetnote.markdown.v1",
    generatedAt: new Date().toISOString()
  });

  // 2. 生成 Markdown
  const markdown = generateMarkdownContent({
    frontmatter,
    deverbalization: llmResult,
    originalTranscript: asrResult.fullText,
    correctionHistory: speakerCorrections
  });

  // 3. 保存
  const result = await saveMeetingMarkdown({
    baseDir: getUserHistoryPath(),
    markdown,
    audioHash: frontmatter.audioHash,
    title: frontmatter.title,
    date: frontmatter.date
  });

  if (result.success) {
    showNotification(`✓ 已保存到 ${result.path}`);
  }
}
```

### 场景 2: 搜索历史会议

```typescript
async function searchHistoryByDate(startDate: string, endDate: string) {
  // 1. 重建索引（或使用缓存）
  const index = await rebuildLibraryIndex(getUserHistoryPath());

  // 2. 执行搜索
  const results = searchMeetings(index, {
    dateRange: { from: startDate, to: endDate }
  });

  // 3. 显示结果
  return results.map(meeting => ({
    title: meeting.title,
    date: meeting.date,
    speakers: meeting.speakers.join(", "),
    link: `file://${meeting.path}/meeting.md`
  }));
}
```

### 场景 3: 灾难恢复（缓存丢失）

```typescript
async function recoverHistoryAfterCacheLoss() {
  // 1. 直接从文件系统重建（无需外部索引）
  const index = await rebuildLibraryIndex("~/.meetnote/history");

  // 2. 重新加载会议列表
  const meetings = index.meetings;

  // 3. 检查完整性
  console.log(`恢复 ${meetings.length} 个会议`);
  console.log(`检测到 ${index.stats.duplicateCount} 个重复`);
  
  return index;
}
```

## 数据结构

### MarkdownFrontmatter

```typescript
interface MarkdownFrontmatter {
  readonly title: string;              // 会议标题
  readonly date: string;               // ISO 8601: YYYY-MM-DD
  readonly audioHash: string;          // SHA256 前 16 字符
  readonly speakers: readonly string[]; // 发言人列表
  readonly modelVersion: string;       // 模型版本标识
  readonly llmProvider: "ollama" | "custom";
  readonly schemaVersion: string;      // 当前: "meetnote.markdown.v1"
  readonly generatedAt: string;        // ISO 8601 时间戳
}
```

### LibraryMeeting (索引中)

```typescript
interface LibraryMeeting {
  readonly path: string;               // 相对路径
  readonly title: string;
  readonly date: string;               // YYYY-MM-DD
  readonly speakers: readonly string[];
  readonly audioHash: string;
  readonly generatedAt: string;
  readonly schemaVersion: string;
}
```

## 错误处理

### 常见错误码

| 错误 | 原因 | 处理方式 |
|------|------|--------|
| `INVALID_MARKDOWN` | Markdown 格式不正确 | 检查生成逻辑 |
| `DUPLICATE_IMPORT` | 同音频同时间重复导入 | 提示用户已存在，询问是否覆盖 |
| `PATH_TRAVERSAL` | 路径安全检查失败 | 记录日志，拒绝保存 |
| `IO_ERROR` | 文件系统操作失败 | 检查权限和磁盘空间 |
| `PERMISSION_DENIED` | 权限不足 | 提示用户权限问题 |

### 错误处理示例

```typescript
const result = await saveMeetingMarkdown(options);

if (!result.success) {
  switch (result.reason) {
    case "DUPLICATE_IMPORT":
      showDialog("该会议已存在，是否更新？");
      break;
    case "PERMISSION_DENIED":
      showError("无权限写入历史库，请检查目录权限");
      break;
    case "IO_ERROR":
      showError(`保存失败: ${result.error}`);
      break;
    default:
      showError("保存失败，请重试");
  }
  return;
}

showSuccess(`已保存到 ${result.path}`);
```

## 性能特征

| 操作 | 数据量 | 时间 | 备注 |
|------|--------|------|------|
| 生成 Markdown | 100 段 | <50ms | 纯字符串操作 |
| 保存到文件 | 1MB | <100ms | 单次 I/O |
| 重建索引 | 100 会议 | <100ms | 递归扫描 |
| 重建索引 | 1000 会议 | <500ms | 线性增长 |
| 搜索 | 1000 会议 | <50ms | 内存搜索 |

**当 > 10k 会议时**，建议升级到完整文本索引（非 MVP）。

## 与其他组件的接口

### 输入: DeverbalizationOutput (来自 LLM Provider)

```typescript
interface DeverbalizationOutput {
  kind: "llm.deverbalization.result";
  requestId: string;
  provider: "ollama" | "custom";
  model: LlmProviderModel;
  paragraphs: readonly DeverbalizedParagraph[];
  // 其他字段...
}

interface DeverbalizedParagraph {
  id: string;
  sourceSegmentIds: readonly string[];
  speakerId: string;
  text: string;
}
```

### 输入: Speaker Corrections

```typescript
interface SpeakerCorrection {
  readonly timestamp: string;          // 修正时间
  readonly action: "rename" | "merge" | "split" | "replace";
  readonly from: string;               // 原始 speaker ID
  readonly to: string;                 // 修正后的 speaker label
}
```

## 集成检查清单

- [ ] 导入所有必要的函数和类型
- [ ] 在 App 初始化时调用 `rebuildLibraryIndex()`
- [ ] 在会议完成后调用 `saveMeetingMarkdown()`
- [ ] 在搜索界面中使用 `searchMeetings()`
- [ ] 配置历史库根目录（如: `~/.meetnote/history`)
- [ ] 处理所有错误场景
- [ ] 测试灾难恢复流程
- [ ] 更新 App 帮助文档

## 测试验证

运行测试验证实现：

```bash
# 所有单元测试
cd frontend && bun test markdown-library

# 所有集成测试
cd tests/api/ts && bun test markdown-library

# 查看覆盖率
cd frontend && bun test --coverage
```

**预期**: 所有测试通过，无类型错误，覆盖率 ≥90%

---

**更多信息**: 查看 `docs/markdown-library.md` 了解完整系统设计。
