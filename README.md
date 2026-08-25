# meetnote

Rust-native meeting intelligence platform：本地导入音频，使用可切换的 Rust/native ASR 与 speaker diarization 后端，生成可编辑、可追溯的会议纪要。

## 当前状态

当前仓库已完成旧版 Tauri、TypeScript、Python 实现的清理，并正在完成 v0.2 Rust-native desktop workspace：GPUI 项目列表、音频导入、Rust 音频解码、后台 processing service、JSON 项目持久化、真实 `whisper-rs` ASR 和 `sherpa-onnx` diarization 已接入。新的工程边界、后端管理层、模型管理和实施顺序见 [`docs/index.md`](docs/index.md)。

## 目标架构

```text
GPUI UI
  -> Rust application layer
  -> ASR / diarization / LLM / model managers
  -> Rust or native-library inference backends
```

项目不再依赖 Tauri、WebView、Python subprocess 或 Python 运行环境。

## 文档

- [架构总览](docs/architecture/overview.md)
- [ASR 与 diarization 后端管理层](docs/architecture/backend-system.md)
- [模型目录与下载协议](docs/architecture/model-management.md)
- [工程实施计划](docs/architecture/implementation-plan.md)
- [架构决策](docs/architecture/decisions.md)

## 后端方向

- ASR：`whisper-rs`、`sherpa-onnx`，后续可加入 Candle 或其他 native backend。
- Diarization：`sherpa-onnx`、`diarization` crate，后续可加入自定义 ONNX/CoreML pipeline。
- 后端通过统一 Rust trait 注册和选择，应用层不依赖具体推理实现。

## 本地处理示例

`meetnote` 默认启动 GPUI；传入 `process` 子命令可运行第一条真实 pipeline：

```bash
cargo run -p meetnote -- process \
  docs/example/2p_en_video.m4a \
  models/asr/whisper-large-v3-ggml/ggml-base.en.bin \
  models/diarization/sherpa-onnx-speaker-diarization-bundle \
  data/runs/2p_en_video
```

命令直接读取 M4A，并输出 JSON 和 Markdown；模型文件位于被 `.gitignore` 忽略的 `models/`，不会提交进仓库。

## 当前不执行

模型权重由用户按需放入本地 cache，不进入仓库。当前桌面入口使用 `data/` 保存项目 JSON 和 Markdown，GPUI 显示模型 readiness、支持 Whisper 与 sherpa-onnx bundle 下载、消费 ASR/diarization 异步状态事件，并支持基础 speaker 合并修正；SQLite、自由 speaker 命名和 LLM 去口语化仍是后续 hardening 工作。
