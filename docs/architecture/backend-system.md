# 模型后端系统

## 目标

ASR 与 diarization 分别注册、选择和执行。application 只依赖稳定 trait 和归一化 contracts，不知道模型族、推理库或文件布局；新增具体 backend 不修改 domain、application 或 GPUI 接口。

```text
application
├── AsrBackendProvider -> AsrBackendManager -> concrete ASR backend
└── DiarizationBackendProvider -> DiarizationBackendManager -> concrete DIA backend
```

模型来源、安装和 checksum 由 `models` crate 管理，见[模型管理](model-management.md)。

## ASR 兼容层

`crates/asr` 提供 registry 与 manager，`application` 定义 backend/provider 接口：

```rust
pub trait AsrBackend: Send + Sync {
    fn descriptor(&self) -> BackendDescriptor;
    fn probe(&self, model: &ModelRef) -> Result<(), BackendError>;
    fn transcribe(
        &self,
        request: AsrRequest,
        context: JobContext,
    ) -> BackendFuture<AsrResult>;
    fn supports_windowed_audio(&self) -> bool;
    fn create_window_session(
        &self,
        request: AsrRequest,
    ) -> Result<Box<dyn AsrWindowSession>, BackendError>;
}

pub trait AsrBackendProvider: Send + Sync {
    fn available_backends(&self) -> Vec<BackendDescriptor>;
    fn backend(&self, backend_id: &str)
        -> Result<Arc<dyn AsrBackend>, BackendError>;
}
```

`AsrBackendRegistry` 只按 ID 保存实现；`AsrBackendManager` 负责 descriptor 列表、选择、probe 与调用。`AsrResult` 必须包含 backend、model、语言和全局时间戳 transcript segments。

窗口 session 接收共享的 `Arc<AudioWindow>`，backend 自行处理模型族特有的 overlap 去重和时间恢复，最终只返回一个完整 `AsrResult`。

## Diarization 兼容层

`crates/diarization` 提供独立 registry 与 manager：

```rust
pub trait DiarizationBackend: Send + Sync {
    fn descriptor(&self) -> BackendDescriptor;
    fn probe(&self, model: &ModelRef) -> Result<(), BackendError>;
    fn validate_stage_configuration(
        &self,
        model: &ModelRef,
        configuration: &DiarizationStageConfiguration,
    ) -> Result<(), BackendError>;
    fn probe_with_configuration(
        &self,
        model: &ModelRef,
        configuration: &DiarizationStageConfiguration,
    ) -> Result<(), BackendError>;
    fn stage_provenance_with_configuration(
        &self,
        model: &ModelRef,
        configuration: &DiarizationStageConfiguration,
    ) -> DiarizationStageProvenance;
    fn diarize(
        &self,
        request: DiarizationRequest,
        context: JobContext,
    ) -> BackendFuture<DiarizationResult>;
    fn supports_windowed_audio(&self) -> bool;
    fn create_window_session(
        &self,
        request: DiarizationRequest,
    ) -> Result<Box<dyn DiarizationWindowSession>, BackendError>;
}
```

不声明 stage override 的 backend 只接受空配置。`DiarizationResult` 必须包含完整录音的 speaker segments、speaker count、embedding observations 和可选 profile candidates；窗口局部 label 不能直接成为最终 speaker ID。

## `asr-whisper-rs`

实现：whisper.cpp 的 Rust adapter。

- 模型：单文件 GGML/GGUF。
- 设备：`Cpu` 强制 CPU；其他选择允许 whisper.cpp 使用 GPU/Metal。
- whole-file：读取完整 16 kHz mono 音频，输出 Whisper 时间戳段。
- windowed：复用一个 `WhisperContext`，每个窗口创建 state；用全局 sample offset 恢复时间，以 valid-core midpoint 归属并去除重复段。
- probe：检查模型 artifact 是现存文件；真实 context 加载在推理 session 创建时完成。

当前 catalog 中 Whisper 条目都是 `candidate-source`，backend 可运行不等于 UI 可下载。

## `asr-sherpa-onnx`

实现：sherpa-onnx offline recognizer adapter。

- 模型族：Paraformer 与 Qwen3-ASR。
- 设备：CPU/CoreML；显式拒绝 Metal provider。
- Paraformer：要求 `model.int8.onnx` 或 `model.onnx`，以及 `tokens.txt`。
- Qwen3-ASR：要求 `conv_frontend.onnx`、encoder、decoder、`tokenizer/` 和 `tokens.txt`。
- whole-file：输出完整文本段。
- windowed：复用 recognizer；优先按 token timestamp 与 valid core 拼接，没有时间戳时使用近似文本边界去重，无法可靠匹配时保留文本。
- probe：检查 backend ID、模型目录和模型族必需文件；真实 recognizer 创建属于加载验证。

`paraformer-zh` 已是可安装路径；Qwen3-ASR adapter 存在，但 catalog bundle 尚未达到 `ready`。

## `diarization-sherpa-onnx`

实现：sherpa-onnx offline speaker diarization。

- 模型：包含 `segmentation.onnx` 与 `embedding.onnx` 的目录。
- 设备：CPU/CoreML。
- pipeline：Pyannote segmentation、speaker embedding、sherpa fast clustering。
- stage：不支持用户覆盖 segmentation、embedding 或 clustering。
- provenance：分别记录 segmentation、embedding 与 `sherpa-fast-clustering`。
- 长音频：尚未暴露未聚类的窗口中间结果，因此 `supports_windowed_audio == false`；application 对长音频明确拒绝该组合。

## `diarization-native`

实现：固定版本 `speakrs` 的完整三阶段 pipeline。

```text
NativeDiarizationBackend
  -> NativeDiarizationEngine
       -> segmentation
       -> embedding
       -> AHC / PLDA / VBx clustering
  -> normalized DiarizationResult
  -> application Speaker Library service
```

- segmentation：`pyannote-segmentation-3.0`。
- embedding：`wespeaker-voxceleb-resnet34`，含 external data 和最短样本配置。
- clustering：`speakrs-ahc-plda-vbx` 与配套参数。
- 设备：CPU；可选 Cargo `coreml` feature 提供 CoreML 执行路径。
- stage override：只接受 descriptor 声明且与 `speakrs-community-1-cpu` bundle 兼容的三阶段配置。
- whole-file：speakrs 完整 inference、聚类和重建。
- windowed：每个窗口只做 inference 和 observation 提取；全录音聚类一次，再用全局结果重建每个窗口并裁剪 valid core。

native engine 返回自己的窗口 session，只有 backend adapter 实现 application 的 `DiarizationWindowSession`，避免 engine 依赖应用层 session 细节。

Speaker Library 位于 application，不属于具体 diarization backend。它只在匿名聚类完成后匹配兼容 `EmbeddingSpace` 的 profile，并生成 label 或待确认 candidate；不能替代跨窗口聚类。

## Application Pipeline

### 短音频

```text
ASR future ──────────┐
                     ├-> join -> identity -> alignment -> normalized result
Diarization future ──┘
```

### 长音频

```text
AudioChunkReader
  -> ProcessingWindowPlanner
  -> shared Arc<AudioWindow>
       ├── ASR window session -> backend stitcher
       └── native DIA window session -> recording-wide clustering
  -> Speaker Library matching
  -> alignment
  -> windowing provenance
```

默认窗口阈值为 30 分钟，core 为 60 秒，前后 context 各 15 秒，采样率为 16 kHz。后端必须同时声明并实现窗口 session；不允许静默退回整文件或拼接窗口局部 speaker label。

当前 native session 保留全部窗口 artifact 与 observation，AHC 使用二次规模的 pairwise distance；20 小时资源上限和受控并发属于生产化阻塞项，见[工程实施计划](../plan/implementation-plan.md)。

## Probe 与错误

固定生命周期：

```text
下载完成 -> 快速 artifact probe
处理开始 -> model + stage preflight
backend 执行 -> session/graph/context 真实加载
推理完成 -> normalized output validation
```

统一错误码：

- `BackendUnavailable`
- `ModelMissing`
- `ModelIncompatible`
- `ModelLoadFailed`
- `DeviceUnavailable`
- `InvalidAudio`
- `InferenceFailed`
- `Cancelled`
- `OutputValidationFailed`
- `PersistenceFailed`
- `NotImplemented`

backend 不读取文档树，不更新 GPUI，不保存项目，也不解释 Agent/Skill 语义。处理结果由 application 写入当前文档和 processing provenance。
