# 项目代码深度审查报告

> 基于 2026-06-06 的代码状态，对比 `docs/` 目录文档进行精确分析。
> 审查方法：将项目拆分为 5 个模块，由子代理逐文件分析源码、测试和文档，最后汇总。

---

## A. 功能偏移：文档有记录但未实现 / 已实现但文档未记录

### A1. 文档声称已实现但实际未完整实现

| # | 声称 | 实际 | 严重度 |
|---|------|------|--------|
| 1 | **Speaker 修正 UI 支持 4 种操作**（重命名、合并、拆分、批量替换） | UI 只实现了**重命名**和**合并**。拆分、区间分配、批量替换三个功能**在 speaker.ts 中已实现但 UI 组件 section-speaker-editor.ts 未接入**，也没有任何 UI 控件暴露这些操作。 | 🔴 高 |
| 2 | **Speaker 修正使用 `applySpeakerCorrectionCommands` 回放引擎** | UI **直接调用原始函数** (`renameSpeaker`, `mergeSpeakers`)，手动修改 projectSnapshot。重命名的修正记录未持久化到 `corrections` 数组——仅在本地 `profiles` 变量中存在，确认按钮只保存 profiles 列表，**不保存修正记录对象**。 | 🔴 高 |
| 3 | **LLM 去口语化支持 Ollama / custom provider** | LLM 管道在所有模式下（包括 Tauri 模式）**始终使用 `runDeterministicMockLlmProvider`**，没有真正的 Ollama API 调用或 custom API 集成。Provider 接口 (`llm-provider.ts`) 存在但从未连接到真实 API。 | 🔴 高 |
| 4 | **4 维搜索**（日期、speaker、标题、关键词） | 搜索 API (`searchMeetings`) 在 `stores/markdown-library-storage.ts` 中完整实现，但**该文件未在任何 UI 组件中引用**——它完全是死代码。用户界面没有搜索功能入口。 | 🟡 中 |
| 5 | **Markdown 保存到文件系统** | `saveMeetingMarkdown` 在 stores/ 中实现但**从未被 UI 调用**。当前 Markdown 仅作 `localStorage` 中的显示，没有文件写入。 | 🟡 中 |
| 6 | **音频重复导入检测**（基于 audioHash） | `planAudioImport` / `resolveImportedMeetingProject` 函数已实现并测试，但**UI 不使用它们**——UI 使用自己的简单 `findIndex` + upsert 逻辑。Tauri 导入路径使用 `tauri-${Date.now()}-${filename}` 伪哈希，**无法检测重复**。 | 🟡 中 |

### A2. 已实现但文档未记录

| # | 已实现 | 应补充文档 |
|---|--------|-----------|
| 1 | **`frontend/src/utils/audio-file-import.ts`** 的完整验证链（hash、时长、格式、大小） | 音频导入的验证规则列表 |
| 2 | **Speaker 修正冲突检测**：`resolveDiarizationRerun` 和 `resolveDiarizationRerunForProject` 的冲突策略 | 重新跑 diarization 时的人工修正保护机制 |
| 3 | **Rust 模型下载器支持断点续传、暂停、取消** | 下载状态机的完整说明 |
| 4 | **ASR 4 引擎的具体差异**——Paraformer 返回 `confidence=1.0`、MiMo 强制 CUDA、语言标签限于 `zh/en/unknown` | 引擎特性差异表 |
| 5 | **`scripts/dev.sh` 的 `--check`、`--frontend`、`--tauri` 模式** | 各模式的具体行为 |

---

## B. 功能实现不完整

### B1. ASR CLI JSON 输出与文档契约严重偏差

**文档规定** (`docs/reference/asr-cli.md`):
```json
{ "kind": "asr.result", "schemaVersion": "meetnote.asr.v1", "requestId": "asr-req-001", 
  "model": { "id": "...", "version": "..." }, "segments": [...] }
```

**实际输出** (`python/asr_cli.py:512-517`):
```json
{ "segments": [...], "language": "zh", "model": "whisper", "backend": "whisper" }
```

| 字段 | 文档要求 | 实际 | 影响 |
|------|---------|------|------|
| `kind` | `"asr.result"` | ❌ 缺失 | Rust 桥接无法通过 kind 区分类型 |
| `schemaVersion` | `"meetnote.asr.v1"` | ❌ 缺失 | 版本校验丢失 |
| `requestId` | 字符串 | ❌ 缺失 | 无法关联请求/响应 |
| `model` | `{id, version}` 对象 | ❌ 字符串 `"whisper"` | 前端只能看到短名称 |
| 错误格式 | `{kind, code, message, recoverable}` | ❌ `{"error": "..."}` | 结构化错误码缺失 |

> ✅ **已修复**：ASR CLI 输出已增加 `kind: "asr.result"`、`schemaVersion: "meetnote.asr.v1"`；错误输出已增加结构化 `kind/code/message/recoverable` 字段（同时保留 `error` 字段兼容 Rust 桥接）。`requestId` 可通过 `--request-id` 参数传入。

### B2. Rust 进度事件解析错误

```rust
// tauri/src/asr.rs:67-76（同样逻辑在 diarization.rs）
let parts: Vec<&str> = rest.splitn(2, ':').collect();
let stage = parts.get(1).copied().unwrap_or("processing");
```

Python CLI 发送 `PROGRESS:0/4:loading-model`，但 Rust 解析时 `splitn(2, ':')` 产生 `["0/4", "loading-model"]`，`parts.get(1)` 返回 `"loading-model"` 对应前端期待的 stage 名——**仔细看这个逻辑实际上是正确的**。`parts.get(1)` 正确获取到了 stage 名称。

等等，让我重新检查。Python 发送格式是 `PROGRESS:0/4:loading-model`。
`rest = "0/4:loading-model"`
`rest.splitn(2, ':')` → `["0/4", "loading-model"]`
`parts.get(1)` → `Some("loading-model")`

所以实际上 stage 解析是正确的！之前的分析有误。但 `message` 字段:
```rust
let message = stage;  // "loading-model"
```

而 `processedMs` 和 `totalMs` 从未从 `0/4` 部分解析出来，导致进度条无法正确显示百分比。这是一个**真实 bug**——进度百分比不会更新。

### B3. 库管理下载的进度报告使用假字节数

```rust
// tauri/src/download.rs
let total_bytes = 100_000_000u64;  // 硬编码 100 MB
let downloaded_bytes = (progress * total_bytes as f64) as u64;
```

Paraformer (440 MB) → 显示 "100 MB / 100 MB"（完成时正确但过程中比例不对）
Qwen3-ASR (3.4 GB) → 显示 "100 MB / 100 MB"（全程固定）
MiMo (16 GB) → 同上

**UI 进度条与模型实际大小完全脱节**。

### B4. Whisper 置信度计算错误

```python
# python/asr_cli.py:132
abs(float(seg.get("avg_logprob", -0.1)))
```

`avg_logprob` 在 whisper 输出中为**负值**（如 -0.05 = 高置信度，-0.5 = 低置信度）。`abs()` 取绝对值反转了尺度：**低置信度的 segment 得到更高的置信度分数**。

### B5. Diarization CLI 的 stdout 污染保护不完整

ASR CLI 使用**文件描述符级别**的 stdout 重定向（`os.dup2(2, 1)`），但 diarization CLI 仅使用 Python 层面的 `contextlib.redirect_stdout`。如果 pyannote 或任何 C 扩展直接写入 fd 1，JSON 输出会被污染。

> ✅ **已修复**：Diarization CLI 现使用与 ASR CLI 相同的 `redirect_stdout_to_stderr()` fd 级别重定向模式。

### B6. section-speaker-editor.ts 的事件监听泄漏

```typescript
// section-speaker-editor.ts:163
document.addEventListener("click", closeDropdown);
```

每次渲染组件时添加全局点击监听器，但**从未移除**。用户导航离开再返回时，监听器重复累积。同样问题存在于 `section-recording-detail.ts`。

---

## C. 架构偏离

### C1. 两条并行的 `rebuildLibraryIndex` 实现（签名不同）

| 文件 | 签名 | 状态 |
|------|------|------|
| `frontend/src/stores/markdown-library-storage.ts` | `async (baseDir: string): Promise<LibraryIndex>` | **死代码**，从未被引用 |
| `frontend/src/utils/markdown-library.ts` | `(meetings: MeetingRecord[], storage: ...): { projectIds: string[] }` | **死代码**，从未被引用 |

两条路径同名但不同签名的函数，两者均未连接到 UI。违反了 "一条事实来源" 的架构原则。

### C2. `getAudioDuration` 重复实现

| 位置 | 实现 | 问题 |
|------|------|------|
| `frontend/src/utils/audio-file-import.ts:55` | `extractDurationFromAudioElement` — 带超时、mock 友好 | ❌ **从未被调用** |
| `frontend/src/components/section-project-list.ts:13` | 内联 `getAudioDuration` — 无超时处理、无 mock 支持 | ✅ **实际被使用** |

**应为**：复用 `audio-file-import.ts` 中的函数。

### C3. `resolveDiarizationRerun` 重复实现

| 文件 | 函数 | 功能集 |
|------|------|--------|
| `frontend/src/utils/speaker.ts:319` | `resolveDiarizationRerun` | 仅检查 segment 级别冲突 |
| `frontend/src/types/audio-project.ts:642` | `resolveDiarizationRerunForProject` | 检查 segment + speaker 标签双级别冲突 |

后者更完整但为私有函数。前者为公开导出但功能较弱。

### C4. 纯文件历史库完全脱钩

架构文档声明 Markdown 历史库为核心数据资产，但：
- `saveMeetingMarkdown` 和 `rebuildLibraryIndex` 在 `stores/markdown-library-storage.ts` 中存在但**完全未连接**
- 当前 `section-markdown-library.ts` **仅显示 localStorage 中的 `MeetingRecord` 列表**
- 没有文件系统 I/O，没有搜索功能入口
- 整个历史库模块是"挂在墙上但没接管道"的状态

### C5. 音频项目存储架构偏离

文档 (`docs/reference/audio-projects.md`) 描述文件系统布局 `meetings/YYYY/MM/<slug>/manifest.json`，但实际存储是：
- **localStorage 键 `meetnote-mvp-state`** 中的 JSON 序列化字符串
- 每个项目是 `MeetingRecord` 对象（包含 `projectSnapshot` 字段存储 `MeetingProject` JSON 字符串）
- **没有文件系统目录结构、没有 manifest.json、没有 meeting.md**

---

## D. 冗余/重复代码

### D1. 死代码模块（总计 ~24 KB）

| 文件 | 大小 | 状态 | 说明 |
|------|------|------|------|
| `frontend/src/stores/markdown-library-storage.ts` | 13,673 B | 🗑️ 死代码 | 从未被任何文件导入；使用 Node.js `fs` 模块 |
| `frontend/src/utils/markdown-library.ts` | 2,059 B | 🗑️ 死代码 | 从未被导入；与上面同名函数冲突 |
| `frontend/src/utils/audio-file-import.ts:100` | `importAudioFile` | 🗑️ 死代码 | 完整实现但从未被调用 |
| `frontend/src/types/audio-project.ts` 中的 `planAudioImport`、`resolveImportedMeetingProject` | ~150行 | 🗑️ 死代码 | 完整测试但 UI 不使用 |

### D2. 函数级重复

| 重复函数 | 位置 1 | 位置 2 | 差异 |
|---------|--------|--------|------|
| `getOverlapMs` | `speaker.ts:550` | `speaker-output.ts:164` | 完全相同 |
| `resolveDiarizationRerun` | `speaker.ts:319` | `audio-project.ts:642` | 后者更完整 |
| `getAudioDuration` | `audio-file-import.ts:55` | `section-project-list.ts:13` | 前者更完整但死 |

### D3. 类型重复

`audio-project.ts:102` 和 `speaker.ts:75` 各自定义了几乎相同的 `SpeakerDiarizationConflict` 类型，但 `audio-project.ts` 的版本多了一个 `reason: "manual-rename-would-change"` 变体。

---

## E. 已知 Bug

### E1. 测试层

| # | Bug | 文件 | 说明 |
|---|-----|------|------|
| E1-1 | 🟢 **L2 测试全部 8 个因路径错误失败** | `tests/api/ts/*.test.ts` | ✅ 已修复：导入路径补全子目录，65/65 测试通过 |
| E1-2 | 🔴 **E2E file-import spec 失败** | `tests/e2e/specs/file-import.spec.ts` | fixture `test-audio.wav` 在 git worktree 中丢失（二进制未跟踪） |
| E1-3 | 🟢 **E2E mock-meeting-flow spec 失败** | `tests/e2e/specs/mock-meeting-flow.spec.ts` | ✅ 已修复：CSS 选择器 `.meeting-library-item` → `.meeting-item` |
| E1-4 | 🟢 **Python 测试文档误标为 "pytest 未安装"** | 文档声称 | ✅ 已纠正：实际是 `uv run python test_*.py` 独立脚本 |

### E2. 实现层

| # | Bug | 文件/行 | 说明 |
|---|-----|---------|------|
| E2-1 | 🟢 **Whisper 置信度反转** | `python/asr_cli.py` | ✅ 已修复：`abs(avg_logprob)` → `1.0 - abs(avg_logprob)` |
| E2-2 | 🟢 **ASR CLI 输出缺少必需字段** | `python/asr_cli.py` | ✅ 已修复：增加 `kind`、`schemaVersion`；错误输出增加结构化字段 |
| E2-3 | 🔴 **UI 事件监听泄漏** | `section-speaker-editor.ts:163`、`section-recording-detail.ts:163` | 多次导航后重复绑定 |
| E2-4 | 🟡 **Mock 流水线写入固定 `createdAt`** | `mock-pipeline.ts:36` | `"2024-01-01T00:00:00.000Z"` — 所有会议显示同一日期 |
| E2-5 | 🟡 **Tauri 导入使用伪哈希** | `section-project-list.ts:335` | `tauri-${Date.now()}-${filename}` 无法检测重复 |
| E2-6 | 🟡 **`activeMeetingId` 删除后悬空** | `ui-state.ts` | 删除当前打开的会议后 ID 未重置 |
| E2-7 | 🟡 **Hardcoded `createdAt`** | `mock-pipeline.ts:36` | 所有会议创建时间相同 |
| E2-8 | 🟢 **`transformer` 未列为 Python 依赖** | `python/pyproject.toml` | ✅ 已修复：添加 `transformers` 和 `torch` 为显式依赖 |
| E2-9 | 🟢 **`pyannote.audio>=3.1` 版本下限错误** | `python/pyproject.toml` | ✅ 已修复：`>=3.1` → `>=4.0` |
| E2-10 | 🟢 **Diarization CLI stdout 保护不足** | `python/diarization_cli.py` | ✅ 已修复：应用 fd 级别 stdout 重定向 |
| E2-11 | 🟡 **库管理下载进度使用假 100 MB 总量** | `tauri/src/download.rs` | 不同大小的模型显示错误进度 |
| E2-12 | 🟡 **重命名 speaker 的修正记录未持久化** | `section-speaker-editor.ts` | 仅在本地 state 中存在，保存时丢失 |
| E2-13 | 🟡 **`get_device("auto")` 不检测 CUDA** | `python/meetnote/device.py:11-13` | 直接跳过 `torch.cuda.is_available()` |

### E3. 安全/配置

| # | 问题 | 文件 | 说明 |
|---|------|------|------|
| E3-1 | 🔴 **CSP 禁用** | `tauri/tauri.conf.json` | `"csp": null` — 无内容安全策略 |
| E3-2 | 🟡 **无 Tauri 2 capabilities 配置** | `tauri/tauri.conf.json` | 生产构建可能 IPC 调用失败 |
| E3-3 | 🟡 **HF Token 通过命令行参数暴露** | `tauri/src/download.rs` | 对同一机器的其他进程可见 |

---

## F. 文档对不齐项

| # | 项目 | 文档位置 | 实际状态 | 差异 |
|---|------|---------|---------|------|
| 1 | `section-transcription-progress.ts` 大小 | 文档声称 29 KB | **实际 15 KB** | 数字属于 `section-model-management.ts` |
| 2 | 音频项目持久化方式 | `docs/reference/audio-projects.md` 描述文件系统 | **实际 localStorage** | 架构偏离 |
| 3 | Manifest 字段列表 | 边界表仅列 8 个字段 | **实际有 16 个字段** | 文档缺少 8 个必有字段 |
| 4 | ASR 输出 schema | `docs/reference/asr-cli.md` 版本 | 实际输出不同（缺少 kind/schemaVersion/requestId） | 功能偏离 |
| 5 | Markdown 保存路径 | 文档说保存到 `meetings/YYYY/MM/<slug>/` | 实际未保存到文件系统 | 未实现 |
| 6 | 🟢 **AGENTS.md 断链** | AGENTS.md | ✅ 已修复：引用指向 `docs/project/gap-analysis.md` |

---

## G. 总结：关键指标

| 指标 | 数据 |
|------|------|
| **L1 测试（前端）** | ✅ 312/312 通过 |
| **L2 测试** | ✅ 65/65 通过（E1-1 已修复） |
| **L3 E2E 测试** | ⚠️ 2/4 通过（CSS 已修复；fixture 需要在 worktree 中创建） |
| **Python 测试** | ✅ 可运行（需 `uv run python test_*.py`，文档误标为 pytest） |
| **死代码总量** | ~24 KB（3 个完整模块 + 5 个未使用函数） |
| **已知实现层 Bug** | 13 个（2 个 🔴 严重，11 个 🟡 中等） |
| **架构偏离** | 5 项（纯文件库脱钩、存储架构不一致、重复实现、事件泄漏等） |
| **文档偏差** | 6 项（数据不准确、契约不匹配、链接断链） |

### 最重要的 5 个行动

1. 🟢 **已修复：L2 测试路径** ✅
2. **🔴 修复 ASR CLI JSON 输出** — 增加 `kind`、`schemaVersion`、`requestId`、结构化 `model` 字段
3. **🔴 连接 LLM Provider** — 实现真实 Ollama/custom API 调用或用接口将 mock 替换为真实调用
4. **🔴 连接 Markdown 保存与搜索** — 调用 `saveMeetingMarkdown` + 实现搜索 UI
5. **🔴 修复状态泄漏** — 移除全局 `document.addEventListener` 和死代码模块
