# ASR 与 Diarization 后端管理层

## 设计目标

应用层可以自由组合不同后端：

```text
whisper-rs + sherpa-onnx diarization
sherpa-onnx ASR + diarization crate
Candle ASR + custom ONNX diarization
```

新增后端只增加实现 crate、模型 catalog 条目和注册 factory，不修改 domain、application 或 GPUI UI。

后端只产生完整音频的标准化转录、speaker 段和可追溯元数据。输出先由 application 对齐并写入目标目录文档，再由 MeetNote 从文档 JSON 实时更新最多三层的项目树。Assistant、外部 Agent 和 Skill 不进入 ASR/diarization backend，也不能改变后端原始结果。

## ASR 接口

```rust
pub trait AsrBackend: Send + Sync {
    fn descriptor(&self) -> BackendDescriptor;
    fn probe(&self, model: &ModelRef) -> Result<(), BackendError>;
    fn transcribe(
        &self,
        request: AsrRequest,
        context: JobContext,
    ) -> BackendFuture<AsrResult>;
}
```

核心对象：

- `AsrBackendRegistry`：注册和发现 backend。
- `AsrBackendManager`：维护 registry，按 `backend_id` 返回实现并执行 probe。
- `AsrBackendProvider`：application 使用的稳定选择边界，不依赖具体实现 crate。
- session 缓存和能力解析在具体 backend 需要时独立增加，不进入 application pipeline。

第一批实现：

- `asr-whisper-rs`：whisper.cpp、GGML/GGUF、Metal/CPU。
- `asr-sherpa-onnx`：sherpa-onnx 支持的 ONNX ASR 模型族。

运行时选择流程：

```text
AppSettings.asr_backend_id / ModelRef.backend_id
  -> AsrBackendProvider
  -> AsrBackendManager
  -> selected AsrBackend
```

## Diarization 接口

```rust
pub trait DiarizationBackend: Send + Sync {
    fn descriptor(&self) -> BackendDescriptor;
    fn probe(&self, model: &ModelRef) -> Result<(), BackendError>;
    fn diarize(
        &self,
        request: DiarizationRequest,
        context: JobContext,
    ) -> BackendFuture<DiarizationResult>;
}
```

第一批实现：

- `diarization-sherpa-onnx`：segmentation、speaker embedding 和 clustering。
- `diarization-native`：默认接入 `speakrs` 的 Pyannote powerset segmentation、
  WeSpeaker embedding、AHC/PLDA/VBx 完整 pipeline，并通过独立 stage trait 保留模型替换能力。
  该 backend 暴露中间 embedding observation，使用原子 JSON speaker library 完成跨项目
  profile matching；最终结果同时保留项目内 cluster ID、稳定 profile ID 和可读 speaker label。

`diarization-native` 的模块边界固定为：

```text
backend
  -> engine
       -> SegmentationAdapter
       -> EmbeddingAdapter
       -> ClusteringAdapter
  -> identity resolver
  -> speaker profile store
```

融合 provider 可以在 `engine` 内复用已经校准的 overlap-add、masked embedding 和 VBx，
但不得绕过统一 observation、identity 和 persistence contract。模型下载仍由 `models` crate 负责，
backend 不调用上游 package 的在线下载器。

`DiarizationBackendManager` 以同样方式实现 `DiarizationBackendProvider`。ASR 和
diarization 可以独立选择，application 只接收两个统一的 `ModelRef`。

## 不把任意 ONNX 当作通用 backend

裸 ONNX 文件不足以描述完整模型。每个模型族还需要声明：

- 输入 tensor 和采样率
- 输出 tensor
- tokenizer 或特征提取器
- 解码策略
- 时间戳恢复方式
- embedding 与 clustering 方式

因此运行时分两层：

```text
InferenceRuntime
  -> ModelFamilyAdapter
  -> normalized result
```

这样可以复用 ONNX Runtime，但不会把模型族差异塞进一个巨型 manager。

## 后端错误

所有后端错误统一转换为：

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

当前后端统一返回 `BackendError { code, message }`；GPUI 只消费错误码和用户可读消息。

文档项目树错误不属于推理后端错误。第四层、循环、缺失父节点和跨音频来源关系由 document/domain 层处理；backend 不读取或维护项目结构。

## 任务事件

```rust
pub enum JobEvent {
    Started { job_id: JobId },
    Progress { job_id: JobId, progress: Progress },
    Completed { job_id: JobId },
    Failed { job_id: JobId, message: String },
    Cancelled { job_id: JobId },
}
```

不使用字符串事件，不让具体 backend 直接更新 GPUI Entity、Assistant、外部 Agent 或项目树。处理完成后由 application 写入文档事实，再触发文档扫描/项目树刷新事件。
