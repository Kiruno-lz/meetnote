# ASR 与 Diarization 后端管理层

## 设计目标

应用层可以自由组合不同后端：

```text
whisper-rs + sherpa-onnx diarization
sherpa-onnx ASR + diarization crate
Candle ASR + custom ONNX diarization
```

新增后端只增加实现 crate、模型 catalog 条目和注册 factory，不修改 domain、application 或 GPUI UI。

后端只产生完整音频的标准化转录、speaker 段、embedding observation 和运行时元数据。输出由 application 对齐，再由 application/persistence 将当前结果与 processing provenance 写入目标目录文档；重新处理直接替换当前结果。当前 pipeline 已写入 catalog revision、逐 artifact checksum 和 segmentation/embedding/clustering stage metadata。Assistant、外部 Agent 和 Skill 不进入 ASR/diarization backend。

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
    fn validate_stage_configuration(
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
}
```

第一批实现：

- `diarization-sherpa-onnx`：segmentation、speaker embedding 和 clustering。
- `diarization-native`：默认接入 `speakrs` 的 Pyannote powerset segmentation、
  WeSpeaker embedding、AHC/PLDA/VBx 完整 pipeline，并通过独立 stage trait 提供替换边界。
  当前生产配置可分别选择兼容的 segmentation、embedding 和 clustering stage；native backend
  会按配置组装临时模型目录，再交给 `speakrs` 加载。stage 模型必须满足该 pipeline 的输入、
  输出、关联 sidecar 和 PLDA 兼容约束，不能把任意 ONNX 当作替代品。
- speaker profile matching 位于 application 的共享 identity service，可处理 native 和
  sherpa-onnx 输出；backend 只负责输出 embedding observation。

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
backend 不调用上游 package 的在线下载器。stage 配置由 descriptor 的结构化 capability 声明；
不支持 stage override 的 backend 不显示这些设置，并在 application preflight 拒绝非空配置。

`DiarizationBackendManager` 以同样方式实现 `DiarizationBackendProvider`。ASR 和
diarization 可以独立选择，application 只接收两个统一的 `ModelRef`。

`DiarizationStageProvenance` 是可选后端元数据边界。后端声明实际使用的 segmentation、embedding
和 clustering 来源，application 将其写入当前 `ProcessingRecord`；未声明的 stage 不得由
application 猜测为某个具体模型。

## Backend probe 生命周期

probe 分为快速 artifact probe、处理前 preflight 和实际加载三层，触发时机固定如下：

| 时机 | 是否触发 | 作用 |
| --- | --- | --- |
| 浏览 catalog、启动 UI、读取设置 | 否 | 只读取 manifest 和文件安装状态，不为所有模型拖慢启动 |
| 单个模型下载完成后 | 是 | `ModelManager` 进入 `verifying`，校验尺寸/checksum 后，对该模型声明的每个 candidate backend 执行快速 probe；失败则删除本次安装并标记失败 |
| 开始处理项目之前 | 是 | `ProcessingService::probe` 对选中的 ASR、diarization 模型和 stage configuration 执行 preflight，阻止无效任务进入 JobRuntime |
| backend 开始推理 | 是 | backend 再次检查请求，并创建真正的推理 session；ONNX graph、GGML header、provider/ABI 等只有在这里完成真实加载验证 |

因此下载后的 probe 不等价于模型已经完成加载；设置页切换也不触发全 catalog probe。处理前 probe
是用户可见任务的硬门槛，实际推理加载失败仍会以 `ModelLoadFailed` 或 `InferenceFailed` 返回。

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

不使用字符串事件，不让具体 backend 直接更新 GPUI Entity、Assistant、外部 Agent 或项目树。处理完成后由 application 写入文档事实和当前 `processing.latest`，再触发文档扫描/项目树刷新事件。重新处理直接替换当前记录；外部 Agent 通过 Skill 写回语义内容，不需要绑定内部 command enum。
