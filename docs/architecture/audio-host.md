# Audio Host 与模型后端系统

Audio Host 是 MeetNote 四个基础层之一，负责音频事实、处理编排、模型资源和 ASR/diarization provider；它通过 Workspace 保存结果，通过 Agent capability 接受显式调用。

## 目标

ASR 与 diarization 分别注册、选择和执行。application 只依赖稳定 trait 和归一化 contracts，不知道模型族、推理库或文件布局；新增具体 backend 不修改 domain、application 或 GPUI 接口。

```text
application
├── AsrBackendProvider -> AsrBackendManager -> concrete ASR backend
└── DiarizationBackendProvider -> DiarizationBackendManager -> concrete DIA backend
```

模型来源、安装和 checksum 由 `models` crate 管理，见[模型管理](model-management.md)。

## Audio Host 与扩展边界

Audio Host 是现有 application processing、audio、模型管理与专用 provider 的逻辑集合，不要求新增同名 crate。保留现有 adapter/backend 链路；provider contract 决定能做什么，Extension lifecycle 决定实现如何注册、启动、停止与分发，见[扩展系统](extension-system.md)。

`application::AudioCapabilityPort` 是面向 Assistant、外部 Agent 和插件的最小受控入口。它只接受授予的项目 ID，复用 `WorkspaceService` 的 probe 和 `ProjectProcessingJob`；模型、配置、任务进度和结果仍由 Audio/Workspace 服务负责，port 不开放任意音频路径或文件系统。

处理失败保留 `BackendErrorCode`，application pipeline 在不改变 code 的前提下补充 ASR/diarization stage 与 backend ID；Audio CLI 将已接受请求的失败映射为版本化 envelope 中的固定 `AUDIO_*` code 和 exit 2，参数/适配器错误仍使用 exit 3。这样外部 Agent 可以按阶段和稳定 code 修正，不依赖解析 stderr 文本。

当前公开替换点是 ASR 与 diarization；native segmentation/embedding/clustering stage 配置不是独立通用 provider。只有出现独立消费者、替换需求与明确的输出/模型兼容规则后，才提取 VadProvider、EmbeddingProvider 或 enhancement contract，不一次拆开全部 pipeline。

原生 Rust、C/C++ FFI、GPU runtime 是现有实现基础；未来 provider 可以使用受管理的外部 Python/native 进程或云 API。它们须归一化输出、传播取消与错误，说明资源和数据传输；不恢复旧混合主程序。模型下载仍由 models 管理，云 provider 需明确服务/模型来源，不能伪造本地 artifact checksum。

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
- 设备：`Cpu` 强制 CPU，`Auto`/`Metal` 使用 Metal；当前构建不启用 Core ML，显式请求返回 `DeviceUnavailable`。
- whole-file：读取完整 16 kHz mono 音频，输出 Whisper 时间戳段；与 windowed 共用 sample 范围标准化，将模型 padding 内的结束时间裁到实际解码边界，纯 padding 段不进入结果。该约束保证时间属于源音频，不等于词级对齐。
- windowed：复用一个 `WhisperContext`，每个窗口创建 state；用全局 sample offset 恢复时间，以 valid-core midpoint 归属并去除重复段。
- probe：检查模型 artifact 是现存文件；真实 context 加载在推理 session 创建时完成。
- 取消：参数对象持有稳定的 `Box<JobContext>`，类型明确的 C 回调只在同步 `full` 调用内读取，返回后释放；避免固定 binding safe wrapper 的 userdata 类型不一致。

当前 catalog 的 large-v3、base.en 与 large-v3-turbo Q5 已固定来源 revision、大小、SHA-256 与 MIT 许可并标为 `ready`；可安装与质量验收分开。

## `asr-sherpa-onnx`

实现：sherpa-onnx offline recognizer adapter。

- 模型族：Paraformer、SenseVoice 与 Qwen3-ASR。
- 设备：CPU；默认线程数为可用 CPU 数与四的较小值。固定发行包不能保证 Core ML provider，显式 Core ML/Metal 请求返回 `DeviceUnavailable`，避免静默 CPU 回退。
- Paraformer/SenseVoice：要求 `model.int8.onnx` 或 `model.onnx`，以及 `tokens.txt`；SenseVoice 传递请求语言并启用 ITN。
- Qwen3-ASR：要求 `conv_frontend.onnx`、encoder、decoder、`tokenizer/` 和 `tokens.txt`。
- whole-file：30 秒以内保留完整文本 baseline；更长的 Paraformer/SenseVoice 输入改用同一个 recognizer 的有界推理与拼接。
- windowed：复用 recognizer；优先按 token timestamp 与 valid core 拼接，SentencePiece 的显式词边界、独立边界 token 和 `@@` 词内连续标记分别处理；`▁` 只表示空白，不等于所有词边界；复用锁定的 `unicode-segmentation` 1.13.3 按完整还原文本的 UAX #29 词起点识别延续，单次前向遍历保留 UTF-8 偏移。明确的词内连续优先于停顿、句末标点和段长限制，中文词起点仍受这些时间与句界约束，避免把单词或缩写拆开，也不把跨长静音的中文整段吞并；没有时间戳时使用近似文本边界去重，无法可靠匹配时保留文本。
- probe：检查 backend ID、模型目录和模型族必需文件；真实 recognizer 创建属于加载验证。

模型推理窗口独立于 application 的录音路由：Paraformer 核心 8 秒、两侧各 2 秒；SenseVoice 核心 20 秒、两侧各 5 秒。子窗口以父窗口 valid core 为原点，context 限制在父样本内，连续覆盖 core；不让重复 context 产生新的所有权区间。Qwen3-ASR 不沿用该模型特定策略。窗口仅在 valid core 的所有 sample 严格为零时跳过解码；context 中的语音不改变 core 所有权，非零弱声仍交给模型。该检查不是 VAD，不过滤低能量噪声；整段无识别结果返回 `OutputValidationFailed`。

`paraformer-zh` 与 `sensevoice-small-int8` 是可安装路径；Qwen3-ASR ONNX bundle 尚未达到 `ready`。MLX Qwen 仅通过隔离评测 runner 接入，不属于桌面 provider。模型选择依据与实测边界见[转录评测](../reserch/asr-quality-and-runtime-evaluation.md)。

## `diarization-sherpa-onnx`

实现：sherpa-onnx offline speaker diarization。

- 模型：包含 `segmentation.onnx` 与 `embedding.onnx` 的目录。
- 设备：CPU；固定 sherpa runtime 不提供有效 provider 的验证回执，Core ML/Metal 请求返回 `DeviceUnavailable`，不静默回退。
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
- embedding：`wespeaker-voxceleb-resnet34`，含 external data 和最短样本配置；ONNX CPU session 的 intra-op 预算按 `available_parallelism` 估计值计算：一/二线程设备使用全部可用量，其余使用 `N - ceil(N/3)`，查询失败回退一线程；不设固定线程上限。该预算作用于每个 embedding session，不是全应用总预算；accelerator session 保持一个 host 线程。
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
ASR future -> diarization future -> identity -> alignment -> normalized result
```

短音频 ASR 完成后才启动 diarization，包括 ASR future 暂停时；ASR 失败保留错误 code，并取消未启动的 diarization job。Whisper 与 Sherpa 的整文件 ASR 通过 application 的 `run_backend_worker` 复用所属 JobRuntime 可取消准入；排队取消无需等待其他录音结束，不加载音频或模型。运行中继续合作取消并等待真实 worker 退出；正常 backend 错误保留 code/message，无取消标记的异常退出报告 `InferenceFailed`。JobRuntime 继续负责状态、取消与资源记录。长音频保留 window session 的独立调度，短音频顺序测试不证明进程级 native runtime 的隔离。

已用独立真实 Whisper 推理与 macOS faulting-thread 栈定位并修复取消回调 userdata 故障。

//TODO - 用固定模型、音频验证 ASR/diarization 的并行与顺序、短音频与长音频 worker 的进程级稳定性；独立 Whisper 回调修复不能解释所有历史 runtime 故障，也不改变当前顺序调度。

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

整条录音通过所属 ASR `JobContext` 的共享 worker gate 异步取得一个名额后，才创建阻塞 worker、两种模型 session 和解码器；排队不占等待线程，丢弃未获准的 future 会释放捕获资源。ASR/diarization 取消 token 互联，任一阶段取消都能终止排队且不加载模型；运行中仍由 decoder/backend 合作取消，并等待 scoped worker 退出。录音内两条 worker 不重复申请名额，单名额 runtime 不会因嵌套申请死锁。该限制控制录音任务数，不等于 native 推理库的线程总数；worker 异常退出报告推理失败，不能伪装成用户取消。

当前 native session 保留全部窗口 artifact 与 observation，AHC 使用二次规模的 pairwise distance；20 小时资源上限和多录音真实模型资源验收仍属于生产化阻塞项，见[工程实施计划](../plan/implementation-plan.md)。

### 转写与说话人归属

`domain::align_transcript_and_speakers` 按单个说话人段的最大时间交集确定归属；交集相同时选择开始更早的段，仍相同时保留输入中最后一个段的规则，无交集标为 unknown。不拆分转写段，不改变文本、时间、顺序或匹配身份，也不将多个片段的交集累加为新的归属策略。

每次调用使用锁定的 [intervaltree 0.2.7](https://docs.rs/intervaltree/0.2.7/intervaltree/struct.IntervalTree.html) 构建只读区间索引，仅查询重叠候选；索引携带原始位置以固定同分规则，支持乱序和嵌套区间。空转写直接返回；零长/反向区间不能产生归属。索引是本次调用的派生状态，不缓存到文档或引入 IO；密集重叠仍需比较全部命中候选，不保证任意输入都提速。资源对照与复现入口见[音频质量计划](../plan/audio-quality-plan.md)。

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
