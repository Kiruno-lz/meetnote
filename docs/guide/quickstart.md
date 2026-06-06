# 开发者快速上手

> 从 README.md 提炼，合并了产品定位、技术栈、MVP 范围、开发环境。

## 一句话

meetnote 是一个本地桌面会议纪要工作台：**导入音频 → ASR 转录 → Speaker 标注 → 人工修正 → LLM 去口语化 → Markdown 历史库**。

## 技术栈

| 层 | 技术 | 用途 |
|---|---|---|
| 桌面壳 | Tauri 2.x (Rust) | 原生窗口、文件系统、IPC |
| 前端 | TypeScript + Bun + Web Components | UI、状态管理、业务逻辑 |
| ASR | Python (openai-whisper / funasr / qwen-asr / transformers) | 4 引擎语音转文字 |
| 发言人区分 | Python (pyannote.audio 4.x) | 说话人分割与聚类 |
| LLM 去口语化 | TypeScript (llm-provider) | Ollama / custom provider |
| 包管理 | uv / Cargo / Bun | 各语言依赖管理 |
| E2E 测试 | Playwright | 浏览器自动化 |

## 数据流

```
音频文件 (.wav/.mp3/.m4a)
  → Tauri IPC (前端 → Rust 桥接)
  → Python CLI subprocess (stdout JSON / stderr PROGRESS)
  → 前端 TranscriptionResult (segments[] 时间轴)
  → Speaker Diarization (pyannote, 可选)
  → LLM 去口语化 (忠实改写, 不做摘要)
  → Markdown 历史库 (纯文件, 可追溯)
```

## MVP 范围

### ✅ 包含

1. 导入 `.wav` / `.mp3` / `.m4a`
2. 项目化管理录音
3. ASR 多引擎转录 (Whisper / Paraformer / Qwen3-ASR / MiMo)
4. Speaker diarization (pyannote)
5. Speaker 人工修正
6. LLM 忠实去口语化 (Ollama / custom)
7. Markdown 历史库保存与重建 (4 维搜索)
8. `scripts/dev.sh` 环境自检

### ❌ 不包含

- TTS、摘要、行动项、概括性结论
- dmg / 签名 / 公证 / Homebrew cask（Release 阶段）
- 默认 SQLite / Tantivy 索引
- 模型权重打包进应用

## 开发入口

```bash
bash scripts/dev.sh            # 启动完整环境
bash scripts/dev.sh --check    # 依赖检查
bash scripts/dev.sh --frontend # 仅前端 dev server
bash scripts/dev.sh --tauri    # 启动 Tauri dev mode
```

脚本不会自动安装依赖、下载模型或写入 secret。

## 运行测试

```bash
# L1 单元测试
cd frontend && bun test

# L2 契约测试（当前因模块路径问题全部失败，见 project/gap-analysis.md）
cd tests/api/ts && bun test

# L3 E2E 测试
cd tests/e2e && npx playwright test

# 类型检查
cd frontend && bun run typecheck
```
