# PLAN.md

## 0. 当前边界

本计划用于 meetnote 从空仓库推进到强落地 MVP。当前修订只更新文档，不创建代码、不安装依赖、不下载模型、不添加 submodule、不运行构建。

MVP 的首个交付目标是开发者可运行：通过 `script/dev.sh` 完成环境自检并启动 Tauri/TS app，能导入音频、运行或 mock ASR/speaker/LLM 流程、人工修正 speaker、保存 Markdown 历史库。dmg、签名、公证和 Homebrew cask 后置到 release 阶段。

**当前阶段验证命令**

```bash
git status --short
find . -maxdepth 2 -path './.git' -prune -o -type f -print | sort
find skills -type f -name 'SKILL.md' -print 2>/dev/null
```

当前阶段不运行 `cargo`、`bun`、`uv`、`brew`、Tauri 构建或模型下载命令；下文命令是对应阶段实施后的验收入口。

## 1. 需求决策与范围

**任务清单**

- 固化 MVP 范围：音频导入、项目化管理、ASR、speaker diarization、speaker 人工修正、LLM 去口语化、Markdown 纯文件历史库。
- 明确非 MVP：TTS、摘要、行动项、dmg、签名、公证、Homebrew cask、SQLite/Tantivy 默认索引。
- 记录 ASR 默认策略：质量优先，优先评估 Whisper large-v3，主要支持中文会议并兼容少量英文片段。
- 记录模型策略：app 包不内置模型，安装后提供按需下载、校验、配置和切换能力。

**产物**

- README 范围说明、`docs/decisions.md`、MVP/非 MVP 清单。

**验收标准**

- 新贡献者能明确知道 MVP 做什么、不做什么。
- 文档中不承诺开箱即用、不把模型放进 app 包、不把摘要/行动项作为 MVP 去口语化输出。

**验证命令示例**

```bash
rg -n "TTS|摘要|行动项|开箱即用|SQLite|Tantivy|dmg|cask" README.md docs AGENTS.md PLAN.md
git status --short
```

## 2. 仓库初始化

**任务清单**

- 建立基础目录规划：`frontend/`、`src-tauri/`、`python/`、`tests/`、`script/`、`docs/`、`examples/`。
- 编写 README：项目定位、隐私边界、模型按需下载策略、开发环境入口。
- 编写 `.gitignore`：排除模型权重、音频隐私数据、构建产物、缓存目录、provider token 和 secret。
- 创建 `script/dev.sh`，用于依赖检查、端口占用检测、服务启动和健康检查。

**产物**

- README、`.gitignore`、目录说明、`script/dev.sh` 初版。

**验收标准**

- 仓库不包含模型权重、真实音频隐私样本、构建产物或 secret。
- `script/dev.sh` 能报告缺失依赖，并在依赖齐全时启动开发环境。

**验证命令示例**

```bash
git status --short
find . -maxdepth 3 -path './.git' -prune -o -type f -print | sort
bash script/dev.sh --check
```

## 3. 模型下载与模型矩阵

**任务清单**

- 建立模型矩阵：Whisper large-v3 为质量优先候选，记录语言表现、硬件需求、格式、来源、checksum、许可证和下载尺寸。
- 设计 app 内模型下载流程：下载、断点续传、校验、缓存位置、版本记录、删除和切换。
- 设计 pyannote 外部配置：`pyannote/speaker-diarization-3.1` 的认证、下载、缓存、错误提示和隐私说明。
- 记录不内置模型的用户体验：首次使用时引导下载或选择本地模型路径。

**产物**

- `docs/models.md`、模型 manifest schema、模型缓存策略、下载状态 JSON schema。

**验收标准**

- 模型可独立于 app 包下载和管理。
- 缺模型、校验失败、权限不足、认证失败都有明确错误码和用户提示。

**验证命令示例**

```bash
cd frontend && bun test model-manifest
cd tests/api/ts && bun test model-download.contract.test.ts
```

## 4. Tauri/TS App 骨架

**任务清单**

- 初始化 Tauri 桌面壳和 TypeScript 前端；TS 优先实现业务逻辑，Rust 仅在权限、性能或进程编排需要时引入。
- 定义页面结构：项目列表、录音详情、转录进度、speaker 修正、去口语化预览、Markdown 历史库。
- 定义 app 与本地 CLI 的调用边界：命令、stdin/stdout、JSON schema、进度事件、错误码。
- 建立 mock pipeline，让无真实模型环境也能跑通端到端 UI。

**产物**

- Tauri/TS 项目骨架、核心 TypeScript 数据模型、CLI 契约文档、mock pipeline。

**验收标准**

- 开发者可通过 `script/dev.sh` 启动 app。
- 不下载模型也能用 mock 数据验证主要 UI 流程。

**验证命令示例**

```bash
bash script/dev.sh --check
bash script/dev.sh
cd frontend && bun test
```

## 5. 音频项目管理

**任务清单**

- 支持导入 `.wav`、`.mp3`、`.m4a`，记录原始路径、文件 hash、时长、格式、创建时间和项目标题。
- 每个录音文件持久化保存转录、speaker、人工修正、LLM 去口语化结果和编辑后的会议内容。
- 设计项目数据目录，确保后期可在 app 内继续编辑会议内容。
- 保留原始转录和人工编辑历史，禁止不可逆覆盖。

**产物**

- 项目 manifest schema、音频导入校验、项目目录规范、编辑状态持久化测试。

**验收标准**

- 同一音频重复导入有确定性处理策略。
- 关闭并重启 app 后，标题、speaker、修正文稿和 Markdown 草稿可恢复。

**验证命令示例**

```bash
cd frontend && bun test audio-project
cd tests/api/ts && bun test project-storage.contract.test.ts
```

## 6. ASR CLI

**任务清单**

- 封装 ASR subprocess/CLI，支持模型路径配置、音频格式校验、进度事件、取消和失败恢复。
- 默认质量优先，优先评估 Whisper large-v3；中文会议为主要评测集，并包含少量英文夹杂片段。
- 定义转录输出 JSON：segment id、起止时间、原文文本、语言、置信度、模型版本、参数。
- 保留原始逐字转录，后续 speaker 合并和去口语化不得覆盖原文。

**产物**

- ASR CLI 契约、transcript JSON schema、mock ASR 输出、中文/中英混合评测夹具说明。

**验收标准**

- 固定 mock 输入能稳定生成带时间戳的 transcript。
- 缺模型、格式不支持、进程失败、用户取消都有稳定错误码。

**验证命令示例**

```bash
cd frontend && bun test asr-schema
cd tests/api/ts && bun test asr.contract.test.ts
```

## 7. Speaker Diarization CLI

**任务清单**

- 通过 subprocess/CLI 集成 `pyannote/speaker-diarization-3.1`，记录模型版本、运行参数和认证状态。
- 定义 diarization 输出 JSON：speaker id、起止时间、置信度、重叠标记、模型版本。
- 合并 ASR segment 与 speaker segment，处理重叠、静音、未知发言人和边界误差。
- 将 speaker 标注作为 MVP 核心链路，不后置到 release 或后续版本。

**产物**

- diarization CLI 契约、speaker segments schema、合并算法、mock diarization 输出。

**验收标准**

- mock diarization 输出能与 ASR transcript 稳定合并。
- speaker 标签能进入后续人工修正、LLM 去口语化和 Markdown 输出。

**验证命令示例**

```bash
uv run pytest python/tests
cd frontend && bun test speaker-merge
cd tests/api/ts && bun test diarization.contract.test.ts
```

## 8. Speaker 人工修正

**任务清单**

- 提供转录后 speaker 操作：重命名、合并、拆分、批量替换、区间改标、未知 speaker 处理。
- 支持按时间轴和文本段落查看 speaker，编辑后即时更新会议内容预览。
- 保存人工修正记录，后续重新生成去口语化文本或 Markdown 时必须尊重修正结果。
- 设计冲突策略：重新跑 diarization 时不得静默覆盖人工修正。

**产物**

- speaker edit model、UI 交互流程、修正记录 schema、回归测试夹具。

**验收标准**

- 转录完成后用户必须能修正 speaker，并持久化到录音项目。
- speaker 修正能同步反映到 LLM 输入和 Markdown 历史库。

**验证命令示例**

```bash
cd frontend && bun test speaker-edit
cd tests/e2e && npx playwright test specs/speaker-edit.spec.ts
```

## 9. LLM 去口语化 Schema

**任务清单**

- 将去口语化拆成 LLM provider 层和严格 schema 校验层；本地支持 Ollama，云端支持 custom provider。
- 输入包含原始转录、时间戳、speaker 修正结果和段落边界。
- 输出只做忠实去口语化：去除填充词、重复、明显口误，修正常见断句；不得总结、摘要、添加行动项或扩写含义。
- 设计失败处理：schema 校验失败、provider 超时、token 不足、云端隐私确认、用户取消。

**产物**

- LLM provider interface、de-verbalization JSON schema、prompt 模板、schema 校验测试。

**验收标准**

- 同一 mock provider 输入输出可确定性复现。
- 输出每个段落都能追溯到原始 transcript segment 和 speaker。
- MVP 输出不包含摘要、行动项或概括性结论。

**验证命令示例**

```bash
cd frontend && bun test llm-schema
cd tests/api/ts && bun test llm-provider.contract.test.ts
```

## 10. Markdown 纯文件历史库

**任务清单**

- 设计纯文件历史库：`meetings/YYYY/MM/<slug>/meeting.md`、`manifest.json`、原始 transcript、speaker 修正和生成记录。
- Markdown frontmatter 保存标题、日期、音频 hash、speaker、模型版本、LLM provider、schema 版本和生成时间。
- 从文件系统重建索引，MVP 不默认依赖 SQLite/Tantivy。
- 支持按标题、日期、speaker 和正文进行基础搜索；后续再评估全文索引引擎。

**产物**

- 历史库目录规范、Markdown schema、重建索引逻辑、迁移策略。

**验收标准**

- Markdown 文件可脱离 app 独立阅读。
- 删除 app 缓存后可从纯文件历史库重建项目列表和搜索索引。

**验证命令示例**

```bash
cd frontend && bun test markdown-library
cd tests/api/ts && bun test library-rebuild.contract.test.ts
```

## 11. 开发者可运行验收

**任务清单**

- 完善 `script/dev.sh`：依赖检查、端口检测、服务启动、健康检查、清晰失败提示。
- 准备 mock fixtures，使无模型环境可完成导入、转录、speaker、去口语化、Markdown 保存闭环。
- 执行 L1/L2/L3 分层验证，并记录未覆盖项和原因。
- 补充开发者 README：环境要求、启动命令、模型下载入口、mock 模式说明。

**产物**

- 可运行开发环境、mock fixtures、测试报告、开发者 README。

**验收标准**

- 新开发者能按 README 启动 app 并跑通 mock 会议流程。
- L1 通过率 100%，L2 契约测试通过，L3 关键路径无确定性失败。

**验证命令示例**

```bash
bash script/dev.sh --check
cd frontend && bun test
cd tests/api/ts && bun test
cd tests/e2e && npx playwright test
```

## 12. Release Packaging 后置

**任务清单**

- 在 MVP 开发者可运行后，再规划 `packaging/`、dmg、签名、公证、checksum 和 Homebrew cask。
- 明确 app 包与模型包分离：release app 不内置 Whisper、pyannote 或 LLM 模型权重。
- 设计 release checklist：版本号、构建、签名、公证、checksum、下载链接、回滚和 cask 更新。
- 确保 secret 只通过环境变量或安全存储注入。

**产物**

- `packaging/README.md`、dmg build script、cask template、release checklist。

**验收标准**

- 在具备证书的机器上可复现生成 dmg。
- cask 模板包含版本、url、sha256、name、desc、homepage 和卸载策略。
- release 流程不改变 MVP 开发启动方式。

**验证命令示例**

```bash
bash packaging/build-dmg.sh --dry-run
brew audit --cask packaging/Casks/meetnote.rb
```

## 13. 风险与决策点

**主要风险**

- Whisper large-v3 的本地性能、内存占用、模型格式和硬件兼容性。
- 中文会议夹杂英文时的识别质量、语言切换和时间戳稳定性。
- pyannote 模型授权、认证流程、离线可用性和用户隐私预期。
- ASR 时间戳与 speaker segments 合并误差导致发言人错标。
- LLM 去口语化可能过度改写，必须用 schema、prompt 和回归样例约束忠实原文。
- 长音频处理的性能、内存、取消恢复和进度准确性。

**关键决策点**

- Whisper large-v3 是否作为默认推荐模型，是否提供更小模型作为低配备选。
- pyannote 是否长期保留 Python CLI 边界，还是后续迁移到更紧的本地集成。
- 云端 custom provider 的隐私确认、token 存储和错误处理边界。
- 纯文件历史库何时需要升级到 SQLite/Tantivy 等索引引擎。
- release 阶段是否优先 macOS，其他平台是否后置。

**验证命令示例**

```bash
git status --short
rg -n "TODO|FIXME|secret|token|password" .
rg -n "摘要|行动项|TTS|SQLite|Tantivy|开箱即用" AGENTS.md PLAN.md README.md docs 2>/dev/null
```
