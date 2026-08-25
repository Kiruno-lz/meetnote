# ASR 多模型支持指南

meetnote 现在支持四类 ASR 引擎，可在「模型管理」页面下载、切换并用于转录。

## 支持的模型

| 模型 | Backend | 大小 | 框架 | 硬件 | 下载方式 | 适用场景 |
|------|---------|------|------|------|----------|----------|
| Whisper 系列（7 个） | `whisper` | 73 MB ~ 2.9 GB | openai-whisper | CPU / MPS / CUDA | HTTP | 多语言通用 |
| Paraformer 中文 | `paraformer` | 419 MB | FunASR | CPU 可运行 | 库管理 | 中文会议，120x 实时速度 |
| Qwen3 ASR 1.7B | `qwen3-asr` | 3.17 GB | qwen-asr | CPU / CUDA | 库管理 | 30 种语言 + 22 种中文方言 |
| MiMo V2.5 ASR | `mimo` | 14.9 GB | transformers | **需要 CUDA GPU** | 库管理 | 方言、歌词识别 |

## 下载方式说明

- **HTTP 下载**（Whisper）：直接从 HuggingFace 下载 `.bin` 文件到 `~/.meetnote/models/`，支持断点续传。
- **库管理下载**（Paraformer / Qwen3 / MiMo）：通过 Python 库（FunASR / qwen-asr / transformers）下载到各自的缓存目录
  （`~/.cache/modelscope/` 或 `~/.cache/huggingface/`）。这些模型由库自身管理版本，无法在 UI 中单独删除——
  如需清理请手动删除对应缓存目录。

## 使用步骤

1. 打开「模型管理」页面，每个模型卡片显示引擎徽章（whisper / paraformer / qwen3-asr / mimo）、大小和依赖。
2. 点击「下载」。库管理模型首次下载会触发 Python 库拉取权重。
3. 下载完成后点击「切换使用」将其设为当前 ASR 模型。
4. 导入音频，转录流程会自动使用所选模型的 backend 调用对应引擎。

## 前置依赖

```bash
cd python && uv sync
```

这会安装 `openai-whisper`、`funasr`、`qwen-asr`、`transformers`、`librosa` 等。

### GPU 加速（可选）

`flash-attn` 仅在 CUDA 环境可编译，因此被设为可选依赖。在有 NVIDIA GPU 的机器上：

```bash
cd python && uv sync --extra cuda
```

macOS（Apple Silicon）无需也无法安装 flash-attn——Qwen3 和 MiMo 会使用标准 attention 路径。
**MiMo 强制要求 CUDA GPU**，在无 GPU 环境下载/推理时会返回明确的 `GPU_REQUIRED` 错误。

## 架构

```
前端 (TypeScript)
  │  invoke("run_asr", { backend, modelId, ... })
  │  invoke("start_model_download", { downloadMethod: "library-managed", backend, ... })
  ▼
Rust 桥接 (src-tauri)
  │  asr.rs        → uv run python asr_cli.py --backend <X> --model <id>
  │  download.rs   → uv run python model_downloader.py --backend <X> --model-id <repo>
  ▼
Python CLI (python/)
  ├─ asr_cli.py          统一入口，4 个引擎类，统一 JSON 输出
  └─ model_downloader.py 库管理模型预下载，JSON 进度事件
```

### 关键设计：stdout 纯净性

FunASR / qwen-asr / transformers 会向 stdout 打印版本横幅和进度条，会污染 Rust 解析的 JSON。
`asr_cli.py` 在模型加载和推理期间于**文件描述符层面**将 stdout 重定向到 stderr，仅在最后输出一行纯 JSON 结果。
进度事件统一通过 stderr 的 `PROGRESS:x/4:stage` 前缀传递。

## 统一输出格式

所有引擎输出相同的 segment schema，供前端直接消费：

```json
{
  "segments": [
    { "id": "asr-seg-0000", "startMs": 0, "endMs": 2480, "text": "...", "language": "zh", "confidence": 0.94 }
  ],
  "language": "zh",
  "model": "funasr/paraformer-zh",
  "backend": "paraformer"
}
```

## 故障排除

| 现象 | 原因 | 解决 |
|------|------|------|
| `funasr not installed` | 依赖未安装 | `cd python && uv sync` |
| MiMo 报 `GPU_REQUIRED` | 无 CUDA GPU | 仅在有 NVIDIA GPU 的机器使用 MiMo |
| 转录结果解析失败 | （已修复）库日志污染 stdout | 确认运行的是最新 `asr_cli.py` |
| Paraformer 在 Mac 报 cuda 错误 | （已修复）MPS→cuda 误映射 | 确认最新代码，MPS 现回退至 CPU |
