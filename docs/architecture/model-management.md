# 模型管理

## 资产边界

`resources/models/catalog.json` 是模型来源、格式、revision、候选 backend、大小、显存预估、license 与 artifact 清单的版本化声明。`models/` 是本地缓存，`.download-tasks.json` 是可恢复的下载任务状态；两者都不保存文档或项目树。

模型权重不进入仓库。backend 不自行联网下载，UI、Assistant 和外部 Agent 也不能直接修改 catalog、缓存或下载任务内部状态。

```text
resources/models/catalog.json
models/
├── asr/<model-id>/
├── diarization/<model-id>/
└── .download-tasks.json
```

## Catalog 字段

每个条目必须声明：

- `id`、`task`、`modelFamily`、`candidateBackends`
- `revision`、`sourceFormat`、`source`
- `sizeBytes`、`estimatedVramBytes`
- `sha256` 或逐项 `artifacts[].sha256`
- `license`、`requiresAuth`、`status`
- bundle 的 `artifactFiles` 与 `artifacts`

状态含义：

| 状态 | 含义 |
|---|---|
| `ready` | Rust model manager 可以下载、安装并交给候选 backend probe |
| `candidate-source` | 来源已记录，但下载 manifest 或 backend 适配尚不完整，不可安装 |
| `requires-native-adapter` | 只有模型来源，没有可用 Rust backend |

`candidateBackends` 只表示格式和 adapter 的候选关系，不代表模型已经下载、probe 成功或完成真实推理验收。

## 当前模型

大小和显存均来自 catalog；`—` 表示 catalog 尚未给出可靠值。

| 模型 | 后端 / 状态 | 下载来源 | 权重大小 | 预期显存 | 说明 |
|---|---|---|---:|---:|---|
| `paraformer-zh` | `sherpa-onnx` / `ready` | [sherpa-onnx Paraformer archive](https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-paraformer-zh-small-2024-03-09.tar.bz2) | 81,904,027 B | 1 GiB | 中文离线 ASR；安装 `model.int8.onnx` 与 `tokens.txt`，支持整文件和离线分窗 |
| `sensevoice-small-int8` | `sherpa-onnx` / `ready` | [固定 revision 资产](https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/tree/2365baeacb507f821a0c8120fcee3d484dba7a07) | 239,549,806 B | 2 GiB | 多语言 INT8；model/tokens/license bundle，遵循上游 FunASR 模型许可，不能按 toolkit 的 MIT 许可解释权重 |
| `qwen3-asr-0.6b` | `sherpa-onnx` / `candidate-source` | [sherpa-onnx Qwen3-ASR archive](https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25.tar.bz2) | — | — | backend 已有 Qwen3 文件布局适配；catalog 尚不能安装含 tokenizer 目录的完整 bundle |
| `whisper-large-v3-ggml` | `whisper-rs` / `ready` | [whisper.cpp GGML large-v3](https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-large-v3.bin) | 3,095,033,483 B | 4 GiB | 多语言 Whisper GGML；已固定 revision/checksum，MIT 许可，CPU/Metal 真实执行 |
| `whisper-base-en-ggml` | `whisper-rs` / `ready` | [whisper.cpp GGML base.en](https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-base.en.bin) | 147,964,211 B | 512 MiB | 英文 Whisper GGML；已固定 revision/checksum，MIT 许可，CPU/Metal 真实执行 |
| `whisper-large-v3-turbo-q5` | `whisper-rs` / `ready` | [固定 revision GGML](https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-large-v3-turbo-q5_0.bin) | 574,041,195 B | 2 GiB | 多语言 Turbo Q5；MIT 许可，Metal 真实执行；量化质量按场景评测 |
| `speakrs-community-1-cpu` | `diarization-native` / `ready` | [固定 revision 资产目录](https://huggingface.co/avencera/speakrs-models/tree/5d24ffee75f13fb061fa6d10944a64e2dc1d5e6f) | 59,751,799 B | 1 GiB | Pyannote segmentation、WeSpeaker embedding 与 AHC/PLDA/VBx 参数，共 10 个固定 artifact；支持整文件和离线分窗 |
| `sherpa-onnx-speaker-diarization-bundle` | `sherpa-onnx` / `candidate-source` | [segmentation archive](https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2) + [3D-Speaker embedding](https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx) | 45,586,674 B | 512 MiB | sherpa-onnx 离线 diarization；当前只支持整文件处理 |
| `pyannote-speaker-diarization-3.1` | 无 backend / `requires-native-adapter` | [Pyannote 3.1](https://huggingface.co/pyannote/speaker-diarization-3.1) | — | 1 GiB | gated 参考来源，不能进入当前处理链路 |

Qwen3-ASR 0.6B/1.7B 的 MLX 量化模型仅在隔离 benchmark 中验证；不增加桌面 Python 依赖，不将驻留 session 的性能与逐次加载混为同一条件。`ready` 不代表已通过所有语言、噪声、时间戳或长会议质量门。

具体 artifact URL 与 SHA-256 不在本文重复维护，以 [`catalog.json`](../../resources/models/catalog.json) 为唯一清单。

## 下载与安装协议

```text
读取并校验 catalog
  -> 创建下载任务
  -> <artifact>.part
  -> HTTP Range 续传
  -> 流式写入、速度与 ETA
  -> 大小与 SHA-256 校验
  -> archive member 提取（如有）
  -> 原子 rename 到最终 artifact
  -> 对全部候选 backend 执行快速 probe
  -> completed
```

下载状态：

```text
queued -> downloading <-> paused -> verifying -> completed
                    └-> cancelled      └-> failed
```

- 暂停、取消和进程中断保留 `.part`；重启后活动任务恢复为 `paused`。
- 继续下载时发送 HTTP `Range`；服务器不返回 `206` 时从零覆盖临时文件。
- 网络或 IO 错误最多自动重试三次。
- 终态（`completed`、`failed`、`cancelled`）在任务最后一次写入（安装文件、probe 记录、任务文件）之后才发布，此后状态不再变化；取消运行中的下载时由 worker 在下一个检查点（下一数据块、backend probe 结束或重试等待）发布 `cancelled`。
- 校验失败删除临时文件；backend probe 失败删除本次模型安装目录。
- `requiresAuth` 且没有认证接入的模型拒绝下载，token 不进入 catalog、日志或命令行。
- bundle 按 artifact 下载；`archiveMember` 先校验来源 archive，再提取目标成员。

当前 catalog v1 的 `artifacts[].sha256` 对直接文件表示文件 checksum；对 `archiveMember` 表示下载 archive 的 checksum。需要同时校验 archive 与成员时，应升级 catalog schema 后分别建模。

## 三层 Probe

### 1. Catalog 与安装状态

加载 catalog 时校验 schema、ID 唯一性、来源和 license。安装状态只根据声明的 artifact 是否齐全划分为 `missing`、`partial` 或 `installed`，不加载模型。

### 2. 下载后快速 probe

下载完成并通过大小/checksum 后，`ModelManager` 为条目中的每个 `candidateBackend` 构造 `ModelRef` 并调用 backend probe。失败时模型不能进入 `completed`。

快速 probe 至少检查：

- backend 与 model ID/模型族兼容。
- 文件或目录存在，bundle 必需 artifact 齐全。
- native diarization stage 组合、revision 和固定参数文件兼容。

每次 probe 的结果写入 `<model_root>/<task>/<model-id>.probe.json`（schema `meetnote.model-probe.v1`，原子替换），位于安装目录旁而不在其中：处理 provenance 对安装目录逐文件计算 checksum，probe 失败又会删除安装目录。记录包含：

- probe 类型（`download` 或 `reprobe`）、时间与 catalog revision；
- 逐 artifact checksum 结果（`matched`、`archive-matched`、`archive-not-rechecked`、`not-declared`、`mismatched`、`unreadable`）与逐 backend 结果；
- 最近一次失败的操作与类型化原因（`network`、`io`、`size-mismatch`、`checksum-mismatch`、`archive`、`unsupported-source`、`backend-probe`），之后一次成功即清除。

`ModelAvailability` 携带该记录、失败原因与记录读取错误（无法解析、schema 未知或属于其他模型时报告路径和原因，下次写入覆盖）。`ModelManager::reprobe` 在独立线程上对已安装模型重新校验 checksum 并执行 backend probe；archive 成员因 archive 未保留而记为 `archive-not-rechecked`。重新 probe 失败只更新记录与失败原因，不删除已安装文件，模型仍可选择，处理前 preflight 照常把关。

### 3. 处理前 preflight 与真实加载

`ProcessingService::probe` 在任务进入 runtime 前再次检查选中的 ASR、diarization 模型和 stage configuration。真正的 ONNX graph、GGML/GGUF context、provider/ABI 与推理 session 在 backend 执行时加载；快速 probe 不能替代这一层。

```text
下载后 artifact probe
  -> 处理前 model/stage preflight
  -> backend session 真实加载
  -> 输出结构校验
```

## Provenance

处理成功后，`state.json` 的 `processing.latest` 记录：

- backend、model ID 与 revision
- artifact 路径、聚合 checksum 和逐 artifact 期望/实际 checksum
- diarization segmentation、embedding 与 clustering 来源
- 设备、语言、采样率、expected speakers 与窗口参数
- 输入音频 checksum 和对齐算法

模型 license 与来源保留在版本化 catalog 中；将其快照写入每次处理记录属于当前生产化计划，见[工程实施计划](../plan/implementation-plan.md)。
