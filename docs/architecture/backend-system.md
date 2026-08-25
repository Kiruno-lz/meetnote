# ASR 与 Diarization 后端管理层

## 设计目标

应用层可以自由组合不同后端：

```text
whisper-rs + sherpa-onnx diarization
sherpa-onnx ASR + diarization crate
Candle ASR + custom ONNX diarization
```

新增后端只增加实现 crate、模型 catalog 条目和注册 factory，不修改 domain、application 或 GPUI UI。

## ASR 接口

```rust
pub trait AsrBackend: Send + Sync {
    fn descriptor(&self) -> AsrBackendDescriptor;
    fn probe(&self, model: &ResolvedModel) -> Result<BackendHealth, BackendError>;
    fn transcribe(
        &self,
        request: AsrRequest,
        context: JobContext,
    ) -> BackendFuture<AsrResult>;
}
```

核心对象：

- `AsrBackendRegistry`：注册和发现 backend。
- `AsrBackendSelector`：根据配置、模型和设备选择 backend。
- `AsrSessionManager`：缓存已加载模型和推理 session。
- `AsrCapabilityResolver`：判断语言、时间戳、流式、设备和模型兼容性。

第一批实现：

- `asr-whisper-rs`：whisper.cpp、GGML/GGUF、Metal/CPU。
- `asr-sherpa-onnx`：sherpa-onnx 支持的 ONNX ASR 模型族。

## Diarization 接口

```rust
pub trait DiarizationBackend: Send + Sync {
    fn descriptor(&self) -> DiarizationBackendDescriptor;
    fn probe(&self, model: &ResolvedModel) -> Result<BackendHealth, BackendError>;
    fn diarize(
        &self,
        request: DiarizationRequest,
        context: JobContext,
    ) -> BackendFuture<DiarizationResult>;
}
```

第一批实现：

- `diarization-sherpa-onnx`：segmentation、speaker embedding 和 clustering。
- `diarization-native`：接入 `diarization` crate，保留 ONNX/CoreML provider 选择能力。

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

后端私有错误保留在 `BackendError::details`，不直接暴露给 GPUI。

## 任务事件

```rust
pub enum JobEvent {
    Started { job_id: JobId },
    Progress { job_id: JobId, progress: Progress },
    Completed { job_id: JobId },
    Failed { job_id: JobId, error: AppError },
    Cancelled { job_id: JobId },
}
```

不使用字符串事件，不让具体 backend 直接更新 GPUI Entity。
