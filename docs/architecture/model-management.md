# 模型目录与下载协议

## 资产边界

旧实现中可保留的主要资产是模型清单、来源信息和下载生命周期协议。新实现不保留 Python library-managed 下载路径；所有下载、校验和安装由 Rust `models` crate 负责。

模型目录和安装状态属于 MeetNote 运行时资产，不属于文档项目结构。目标目录中的文档 JSON 仍是三层项目树的唯一事实来源；模型 manifest、下载任务和 cache 不得保存项目父子关系，也不得被 Assistant 或外部 Agent 当作文档数据库。

模型目录位于：

```text
resources/models/catalog.json
```

模型条目必须包含：

- `id`
- `task`
- `candidateBackends`
- `modelFamily`
- `sourceFormat`
- `source`
- `artifacts`（多文件 bundle 的逐文件来源、SHA-256 和可选 archive member）
- `sha256`
- `sizeBytes`
- `license`
- `requiresAuth`
- `estimatedVramBytes`
- `status`

`source` 是单文件模型的直接下载地址，或 bundle 的主来源地址；它不是必然可下载的网页说明。多来源 bundle 必须把每个文件写入 `artifacts`。`candidateBackends` 只允许填写当前已注册且能 probe 该格式的 backend；没有 Rust-native adapter 的模型必须使用 `reference-only` 或 `requires-native-adapter`，不能伪装为可下载模型。

当前目录只保留当前 Rust backend 可识别的 Whisper GGML 条目；OpenAI Whisper PyTorch 条目和没有 Rust-native backend 的 Mimo ASR 已被清除。gated Pyannote 仍作为明确标记为 `requires-native-adapter` 的后续来源保留。

Qwen 条目已改为官方 sherpa-onnx 的 `0.6B int8` ONNX archive 直接地址；这不是原先的 Qwen 1.7B Transformers 仓库。该 archive 内含 `conv_frontend.onnx`、`encoder.int8.onnx`、`decoder.int8.onnx`、`tokenizer` 目录和 `tokens.txt`。catalog v1 不能表达嵌套目录 manifest，因此在实现目录成员下载前只能保持 `candidate-source`，不得显示为 ready 或把 archive 当作单一可 probe 模型。

sherpa-onnx speaker bundle 是由 segmentation archive 和 embedding 文件组成的多来源 bundle，主 `source` 现在指向有效的 segmentation archive；真正的逐文件来源仍以 `artifacts` 为准。上游 release 的 `speaker-recongition-models` 拼写虽然看起来像错误，但该地址是有效的官方路径，不能擅自改成其他 URL。

## 下载状态

```text
idle
queued
downloading
paused
verifying
installed
failed
cancelled
```

运行时任务状态由 Rust `ModelManager` 管理；取消或进程中断会保留 `.part`，下一次下载
自动通过 HTTP Range 继续。`.download-tasks.json` 保存可重建的下载进度，应用重启时将中断
任务恢复为 `Paused`，调用 `resume` 后重新创建 worker。项目不引入 SQLite，下载状态不与文档项目树混用。

## HTTP 下载流程

```text
解析 catalog
  -> 创建 .part 文件
  -> 根据已有长度发送 Range
  -> 流式写入
  -> 发送进度事件
  -> 校验文件大小
  -> 校验 archive 或 artifact SHA-256
  -> 原子 rename
  -> backend probe
  -> 写入 installed 状态
```

下载必须支持：

- 断点续传
- 暂停
- 取消
- 失败重试
- 速度和 ETA
- 校验失败后清理
- 临时文件与最终文件隔离

v0.2 已落地 `models::download`：单文件支持 HTTP Range 续传、`.part` 临时文件、流式 SHA-256 校验和原子 rename；bundle 支持逐 artifact 下载，并从 sherpa-onnx tar.bz2 中提取指定 ONNX member 后再安装。GPUI 会在两个 bundle artifact 都存在后显示 DIA ready。

v0.2.1 已落地 `models::manager::ModelManager`：提供 catalog 按 task/backend 浏览、模型安装状态查询、
下载任务事件订阅，以及 `pause`、`resume`、`cancel` 控制。下载控制与文件写入解耦，
不会要求 UI 参与模型协议。

Assistant 与外部 Agent 都只能通过 application 暴露的稳定能力读取模型 readiness；它们不能直接修改 catalog、安装目录或下载任务内部状态。

## library-managed 的新含义

旧版 `library-managed` 表示启动 Python 模型下载器。新版本中它只表示：

> 模型由 meetnote 的 Rust model manager 根据 catalog 解析、下载、校验和安装。

不同后端的模型文件可以有不同目录，但安装协议统一：

```text
<model-cache>/<task>/<model-id>/
├── artifact files
└── .part files during download

<model-cache>/.download-tasks.json
```

## 模型兼容性

模型源格式不等于 backend 可直接加载的格式。例如 PyTorch `.pt` 不能直接视为 whisper-rs 的 GGML/GGUF 模型。catalog 必须显式记录：

- 原始来源格式
- 目标 backend 支持格式
- 是否需要转换
- 转换工具和版本
- 转换后 checksum

没有完成转换和校验的条目不能显示为已安装。

### Archive checksum 与成员 checksum

`artifacts[].sha256` 的含义必须由来源类型决定：

- 直接文件：校验下载文件本身。
- `archiveMember`：优先校验整个 archive 的 `archiveSha256`，再提取成员；如果上游提供成员 checksum，再额外填写 `artifactSha256`。

当前 catalog v1 只有一个 `sha256` 字段，旧的 archive bundle 条目可能将 archive checksum 重复写在每个 artifact 上。迁移到 catalog v2 时必须拆为 `archiveSha256` 与 `artifactSha256`，并增加目录成员/嵌套文件的 manifest 表达能力；在此之前不得把 archive checksum 误标成提取后文件 checksum。

v0.1 的 backend artifact 约定：

- `whisper-rs` 接收一个已解析的 GGML/GGUF 文件路径，例如 `ggml-large-v3.bin`；OpenAI Whisper 的 PyTorch `.pt` 文件不能直接加载。
- `sherpa-onnx` diarization 接收一个目录路径，目录至少包含 catalog `artifactFiles` 声明的 `segmentation.onnx` 和 `embedding.onnx`。
- `diarization-native` CPU bundle 接收一个目录，包含固定 revision 的 Pyannote segmentation ONNX、
  WeSpeaker ONNX/external data、六个 PLDA/VBx `.npy` 参数文件和最短样本配置。catalog 对十个 artifact
  分别固定 URL 与 SHA-256；backend 不调用上游在线下载器。
- `ModelRef.artifact_path` 是安装完成后的解析路径，不是下载 URL；文件模型指向文件，bundle 模型指向目录。

## 安全与可追溯

- SHA-256 必须来自可信的固定 manifest。
- 下载完成前不得覆盖已安装版本。
- gated 模型的 token 不写入 catalog、日志或命令行。
- 每次推理记录 backend、模型 id、revision、artifact checksum 和运行参数；当前 `ModelRef` 的 artifact 路径由 catalog 和设置解析，不由 UI 或入口硬编码。处理 provenance 的完整写回由 document model 迁移任务完成，见[文档模型与处理记录](document-model.md)。
- 模型 license 和原始来源必须随安装记录保留。
