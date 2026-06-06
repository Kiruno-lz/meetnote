# meetnote 文档与实现差距分析

> 最后更新：2026-06-06

## 差距总览

| 严重程度 | 数量 | 定义 |
|---------|------|------|
| 🔴 严重 | 3 | 文档声称已实现的测试/功能实际不可运行 |
| 🟡 中等 | 6 | 文档过时，声称"未实现"的内容已实现 |
| 🔵 轻微 | 3 | 文档路径/命名与实际不一致 |
| ⚪ 冗余 | 4 | 多文档内容高度重叠，需合并/归档 |

## 🔴 严重差距

### G-01: L2 契约测试全部失败但文档声称通过

| 项目 | 说明 |
|------|------|
| **涉及文档** | `PROJECT_COMPLETION_SUMMARY.md` 声称"46/46 contract tests passing, 100%"；`DELIVERY_MARKDOWN_LIBRARY.md` 类似数据 |
| **当前实际** | 8 个 L2 测试文件全部因模块路径解析失败 (0 pass / 8 fail) |
| **根因** | 测试文件在 `tests/api/ts/`，导入路径 `../../../frontend/src/` 在 Bun 运行时解析失败 |
| **影响** | Markdown 历史库的 E2E 流程、索引重建、灾难恢复均未实际验证 |
| **修复** | 配置 bun 的 `root` 或修正测试运行工作目录 |

### G-02: L3 E2E 测试失败但无对应文档

| 项目 | 说明 |
|------|------|
| **涉及文档** | `PLAN.md` 第 11 节要求"L3 关键路径无确定性失败"，但无 E2E 结果记录 |
| **当前实际** | 上次 Playwright 运行 2 个 spec 失败（file-import, mock-meeting-flow） |
| **影响** | 用户导入音频和 mock 流程路径未经端到端验证 |
| **修复** | 修复 E2E 失败，更新运行配置 |

### G-03: Python 测试不可运行

| 项目 | 说明 |
|------|------|
| **涉及文档** | 无明确 Python 测试文档，但 `PLAN.md` 第 7 节要求 `uv run pytest python/tests` |
| **当前实际** | `pytest` 未安装在 venv 中，3 个 Python 测试文件无法运行 |
| **影响** | ASR CLI 和 Diarization CLI 的单元验证缺失 |

## 🟡 中等差距（文档过时 — 声称未实现但已实现）

### G-04: docs/structure.md — 几乎全文过时

| 条目 | 文档原文 | 当前实际 |
|------|---------|---------|
| 目录名 | `src-tauri/` | 已重命名为 `tauri/` |
| ASR 状态 | "未创建真实 ASR subprocess" | `asr_cli.py` (4 引擎) + `asr.rs` 已实现 |
| Diarization 状态 | "未创建 speaker diarization" | `diarization_cli.py` + `diarization.rs` 已实现 |
| 模型下载 | "未实现真实模型下载器" | `model_downloader.py` + `download.rs` 已实现 |
| LLM Provider | "未接入真实 LLM provider" | `llm-provider.ts` 已实现 |
| 音频项目存储 | "未实现真实音频项目文件 IO" | `audio-project.ts` + 测试通过 |

### G-05: AGENTS.md — "仓库基本为空"

| 条目 | 文档原文 | 当前实际 |
|------|---------|---------|
| 仓库状态 | "仓库基本为空，仅有 `.git` 与系统文件" | 仓库有 26 个模块、300+ 测试、完整 Python/Rust/TS 代码 |
| 最后更新 | 未注明日期 | 明显是项目初始化时创建，未更新 |

### G-06: PLAN.md — 各节"当前阶段"描述过时

| 条目 | 问题 |
|------|------|
| 第 0 节 | "当前修订只更新文档，不创建代码" — 实际已实现大量代码 |
| 第 2 节 | 仓库初始化已完成并演进到 Tauri+Python 实现 |
| 第 3-11 节 | 大部分实现已完成，但 PLAN 以"待实现"语气撰写 |

### G-07: docs/asr-cli.md — 首段声明过时

| 条目 | 原文 | 实际 |
|------|------|------|
| ASR CLI 状态 | "当前仓库只实现 TypeScript schema…尚未接入真实 Whisper 运行器" | `asr_cli.py` 已实现 4 引擎 |

### G-08: docs/models.md — 首段声明过时

| 条目 | 原文 | 实际 |
|------|------|------|
| 下载器状态 | "尚未实现真实下载器、ASR CLI 或 pyannote CLI" | `download.rs` + `model_downloader.py` 已实现 |

## 🔵 轻微差距

### G-09: docs/asr-cli.md — CLI 参数与实现不匹配

| 条目 | 文档 | 实际实现 |
|------|------|---------|
| ASR 引擎 | 仅提及 Whisper | 支持 4 引擎 (Whisper/Paraformer/Qwen3-ASR/MiMo) |
| `languageHint` 枚举 | 只允许 3 种 | 实际 `asr_cli.py` 接受更多格式 |

### G-10: docs/audio-projects.md — 路径规范不匹配

| 条目 | 文档 | 实际实现 |
|------|------|---------|
| 项目目录 | `meetings/YYYY/MM/<slug>/` | Markdown 历史库使用 `meetings/YYYY/MM/<slug>/` 但音频项目不同 |
| slug 格式 | 文档未完全明确 | 实际使用 `YYYY-MM-DD-kebab-case-title-hash` 格式 |

### G-11: 路径引用 — `src-tauri` vs `tauri`

| 文件 | 问题 |
|------|------|
| `docs/structure.md` | 仍引用 `src-tauri/` |
| `docs/ASR_MODELS_GUIDE.md` | 使用 `src-tauri` 术语 |
| `README.md` | 已更新为 `tauri/` |
| 实际目录 | 已重命名为 `tauri/` |

## ⚪ 冗余文档

### R-01: Markdown 历史库三份文档高度重叠

| 文件 | 行数 | 独特内容 | 重叠度 |
|------|------|---------|--------|
| `PROJECT_COMPLETION_SUMMARY.md` | ~500 | 交付报告、质量指标、集成步骤 | 与 DELIVERY 重叠 ~70% |
| `DELIVERY_MARKDOWN_LIBRARY.md` | ~400 | 交付报告、代码清单、集成指南 | 与 PROJECT_COMPLETION 重叠 ~70% |
| `MARKDOWN_LIBRARY_QUICKREF.md` | ~400 | 快速参考、API 示例 | 部分独立 |
| `docs/markdown-library.md` | ~280 | Schema 详细说明、验证规则 | 基础文档 |

### R-02: docs/decisions.md 内容分散

决策记录的内容在 `AGENTS.md`、`PLAN.md`、`PRODUCT.md` 中均有散见，决策记录本身无独特信息。

### R-03: FIXTURE_SPEC.md 与夹具数据分离

Fixture 规范单独成文，但夹具数据在 `tests/fixtures/` 中。规范可与夹具一起维护或移入测试文档。

## 差距影响评分

| 差距 | 严重度 | 影响范围 | 修复工作量 |
|------|--------|---------|-----------|
| G-01 L2 测试全部失败 | 🔴 | 测试报告、文档可信度 | 小（Bun 配置修复） |
| G-02 E2E 测试失败 | 🔴 | 端到端验证 | 中（需调试 UI 流程） |
| G-03 Python 测试不可运行 | 🔴 | 后端验证 | 小（`uv add --dev pytest`） |
| G-04 structure.md 过时 | 🟡 | 文档准确性 | 小（重写或归档） |
| G-05 AGENTS.md 空仓库声明 | 🟡 | Agent 上下文 | 中（重写状态节） |
| G-06 PLAN.md 待实现语气 | 🟡 | 计划文档 | 中（更新状态标记） |
| G-07 asr-cli.md 首段过时 | 🟡 | 文档准确性 | 小（更新首段） |
| G-08 models.md 首段过时 | 🟡 | 文档准确性 | 小（更新首段） |
| G-09 asr-cli.md 参数不匹配 | 🔵 | 文档准确性 | 小（补充引擎信息） |
| G-10 audio-projects.md 路径 | 🔵 | 文档准确性 | 小（对齐实际实现） |
| G-11 src-tauri 引用 | 🔵 | 路径一致性 | 小（批量替换） |
| R-01 三份文档重叠 | ⚪ | 维护负担 | 中（合并到 1-2 份） |
| R-02 决策记录分散 | ⚪ | 维护负担 | 小（移除冗余） |
| R-03 Fixture 规范分离 | ⚪ | 维护负担 | 小（移动或引用） |
