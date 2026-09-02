# 语音模型格式、Rust 推理后端与设备兼容性

## 探索原因

MeetNote 需要在不引入 Rust/Python 兼容层的前提下，支持可切换的 ASR 和 speaker diarization
后端。当前模型目录同时出现 PyTorch、GGML、ONNX 和模型仓库等不同来源，容易把“模型文件格式”、
“模型架构”和“推理后端”混为一谈。

本文件统一记录以下问题：

- `.pt`、`.safetensors`、`.onnx`、`.ggml`、`.gguf` 等格式的边界和用途。
- Candle 是什么，以及为什么需要独立的 Candle ASR backend。
- Rust、Candle、ONNX Runtime、MLX 和 Metal 之间的兼容性边界。
- 当前 MeetNote 模型目录声明了哪些模型，哪些模型实际上可以运行。
- whisper-rs 与 sherpa-onnx 接收什么模型，以及它们的算子是否统一。
- Candle 是否应该重写当前 backend，以及模型页面应该如何表达真实可用性。

## 与产品文档架构的关系

本研究只约束 ASR、speaker diarization、speaker identity 和模型管理。MeetNote 的项目结构仍以目标目录中的 Markdown 与文档 JSON 为唯一事实来源，并由应用实时派生最多三层的项目树。模型 catalog、模型 cache、speaker library 和 embedding observation 都是推理资产，不能成为项目树数据库。

Assistant 与外部 Agent 位于 backend 之上并使用同一 document service。Skill/Agent 决定复合录音的语义拆分边界；本研究涉及的模型只提供完整转录、speaker 轨道、身份匹配和可追溯证据，不直接生成项目父子关系。

## 核心结论

模型运行不是“读取一个权重文件”这么简单，而是多个层次共同匹配：

```text
模型包
  = 网络结构 + 权重 + 配置 + tokenizer/词表 + 前处理 + 后处理

文件格式
  = 上述资产如何序列化和存储

模型族适配器
  = 如何解释该模型的输入、输出、解码和状态

推理运行时
  = 如何执行 Tensor、计算图和算子

设备后端
  = 算子最终在 CPU、CUDA、Metal、Core ML 或其他设备上如何运行
```

因此，下列判断都不成立：

```text
文件扩展名是 .onnx，所以一定能运行
文件扩展名是 .safetensors，所以 Candle 可以直接加载
文件扩展名是 .pt，所以 Rust 不能使用
文件扩展名是 .gguf，所以所有 ggml runtime 都能使用
```

准确的判断必须同时包含：

```text
model family + artifact format + adapter + runtime + device provider + dtype
```

## 模型格式

### `.pt`、`.pth`、`.bin`

这些通常是 PyTorch 生态的文件扩展名，但不是严格统一的文件规范。同一个 `.pt` 可能表示：

- `state_dict`，即参数名到 Tensor 的映射。
- 完整 Python module 对象。
- TorchScript module。
- 其他由 `torch.save` 序列化的 Python 对象。

最推荐的 PyTorch 保存方式是 `state_dict`。它通常只保存参数，不保存完整模型结构，因此加载时仍然
需要对应的网络代码、配置、tokenizer 和输入输出逻辑。

传统 PyTorch 序列化使用 pickle。加载不可信的 checkpoint 存在执行任意代码的风险；模型管理器不能
把任意 `.pt` 当作安全的普通数据文件加载。

参考：

- [PyTorch Saving and Loading Models](https://docs.pytorch.org/tutorials/beginner/saving_loading_models.html)
- [PyTorch torch.load](https://docs.pytorch.org/docs/stable/generated/torch.load.html)

### `safetensors`

`safetensors` 是专门保存 Tensor 的安全格式，通常包含：

- Tensor 名称。
- shape。
- dtype。
- 文件偏移。
- Tensor 二进制数据。

它不一定包含完整的模型结构、tokenizer 或推理流程。Hugging Face 模型通常需要与以下文件一起使用：

```text
config.json
model.safetensors
tokenizer.json
tokenizer_config.json
preprocessor_config.json
```

主要特点：

- 不依赖 pickle 执行模型代码。
- 可以进行内存映射和部分 Tensor 读取。
- 适合大模型分片。
- Rust 生态和 Candle、MLX 都可以使用。

`safetensors` 是权重格式，不是可执行模型。读取成功只说明 Tensor 能被读取，不说明模型架构已经在
目标 runtime 中实现。

参考：[Hugging Face Safetensors](https://huggingface.co/docs/safetensors/index)

### `ONNX`

ONNX 是标准化的模型计算图表示。它可以描述：

- 输入和输出。
- 计算节点。
- Tensor 权重。
- 数据类型。
- shape。
- opset 版本。
- 部分元数据。

ONNX 更接近“可部署的推理图”，而不仅仅是权重文件。一个 ONNX 模型可能由多个文件组成，权重也
可能以 external data 形式存放。

但是 ONNX 不是推理 runtime。能否运行仍取决于：

- ONNX IR 版本。
- opset 版本。
- 模型使用的算子。
- 算子的 dtype 支持。
- 动态 shape 支持。
- 是否包含 custom/contrib operator。
- 当前 Execution Provider 是否实现这些算子。

ONNX Runtime 会把计算图分成子图，交给 CPU、CUDA、Core ML 等 Execution Provider 执行；某些算子
可能回退 CPU，某些算子则会导致加载或执行失败。

参考：

- [ONNX IR Specification](https://onnx.ai/onnx/repo-docs/IR.html)
- [ONNX Runtime Execution Providers](https://onnxruntime.ai/docs/execution-providers/)
- [ONNX Runtime High-level Design](https://onnxruntime.ai/docs/reference/high-level-design.html)

### `GGML` 和 `GGUF`

`GGML` 是 ggml 生态早期使用的模型二进制格式。以 Whisper 为例，OpenAI 原始 PyTorch 权重需要
转换为 whisper.cpp 可以读取的格式，转换后文件可以包含：

- 模型参数。
- Mel filter。
- 词表。
- runtime 所需的其他模型信息。

`GGUF` 是后续更通用的 ggml 容器，能够保存：

- Tensor。
- Tensor dtype 和 offset。
- 模型架构元数据。
- tokenizer 信息。
- 量化信息。

它们适合轻量本地推理，但与 ggml 系列 runtime 强绑定。不同模型族、不同 runtime 和不同版本之间仍然
可能不兼容。

对于 Whisper，典型流程是：

```text
OpenAI/Hugging Face PyTorch checkpoint
  -> whisper.cpp conversion
  -> GGML/GGUF artifact
  -> whisper.cpp / whisper-rs
```

参考：

- [whisper.cpp Model Format](https://github.com/ggml-org/whisper.cpp/blob/master/models/README.md)
- [GGUF File Structure](https://github.com/ggml-org/llama.cpp/blob/master/ggml/include/gguf.h)

### 其他常见格式

| 格式 | 典型用途 | 主要特点 |
|---|---|---|
| `.mlmodel` / `.mlpackage` | Core ML | Apple 专用图和权重，可面向 CPU、GPU、ANE |
| `.tflite` | TensorFlow Lite | 移动端和嵌入式推理 |
| `.engine` / `.plan` | TensorRT | 面向特定 NVIDIA GPU 的编译产物 |
| `.xml` + `.bin` | OpenVINO IR | Intel/OpenVINO 推理图和权重 |
| `.npz` | 数组权重 | 简单但缺少统一模型语义 |
| `.h5` / `.ckpt` | TensorFlow/Keras | 训练检查点或权重 |
| 模型仓库目录 | Hugging Face 等 | 权重、配置、tokenizer 和前后处理文件的集合 |

扩展名只是入口信息。模型管理器必须把模型包作为一个 artifact bundle 管理，而不能只下载单个权重
文件。

## Candle

### Candle 是什么

Candle 是 Hugging Face 的 Rust 原生机器学习框架，定位类似于一个面向 Rust 的轻量 Tensor/神经网络
运行时。它提供：

- Tensor 和 dtype。
- CPU、CUDA、Metal 等设备抽象。
- 神经网络层和基础算子。
- 模型加载工具。
- 部分 Transformer 和音频模型实现。
- safetensors、PyTorch、GGML、NPZ 等文件读取路径。
- 自定义 operation/kernel 扩展能力。

Candle 不是模型格式，也不是“任意模型播放器”。它只提供计算基础和一部分模型实现。

参考：[Candle 官方仓库](https://github.com/huggingface/candle)

### 为什么需要 Candle ASR backend

如果使用 Candle 运行 Whisper、Paraformer 或 Qwen ASR，仍然需要实现完整的模型族适配器。例如 Whisper
至少需要：

```text
音频读取
  -> 重采样
  -> Mel 频谱
  -> Whisper encoder
  -> Whisper decoder
  -> attention/cache
  -> token 解码
  -> 时间戳恢复
  -> 文本后处理
  -> MeetNote TranscriptSegment
```

所以 `asr-candle` 的职责是：

```text
Candle Tensor runtime
  + 模型族结构
  + 权重 key 映射
  + tokenizer
  + 音频前处理
  + 解码策略
  + 时间戳逻辑
  + 统一 ASR contract
```

只添加 `candle-core` 并不能让 `.pt` 或 `.safetensors` 自动变成可运行的 ASR。

### Candle 与 `.pt` / `safetensors`

对于 Candle，通常更推荐使用：

```text
safetensors + config.json + tokenizer + preprocessor
```

而不是依赖任意 `.pt`。原因是：

- 简单的 `state_dict` 可能可以读取，但仍需模型结构和 key 映射。
- 完整 Python pickle module 不能直接在 Rust 中恢复。
- TorchScript 和其他 PyTorch 导出物需要专门解析路径。
- 同一个模型族的不同实现可能有不同 Tensor 名称和权重布局。

因此，MeetNote catalog 中的 `candle` 表示“未来由 Candle 模型适配器承接”，不表示该模型已经可以
直接运行。

## Rust、算子和设备兼容性

### Rust 本身不是限制

Rust 可以调用：

- Rust 原生模型实现。
- C/C++ FFI。
- ONNX Runtime C API。
- CUDA。
- Metal。
- Core ML。
- MLX C++ API。

真正的限制来自具体 runtime 和模型适配器，而不是 Rust 语言本身。

### 算子兼容性

语音模型可能使用：

- Conv1D。
- MatMul。
- LayerNorm/RMSNorm。
- Attention。
- Rotary Position Embedding。
- STFT 和 Mel filter。
- Gather、Scatter、TopK、Where。
- 动态 shape。
- 融合算子。
- 量化算子。
- custom/contrib operator。

一个模型可能出现以下情况：

```text
CPU：全部支持
CUDA：全部或大部分支持
Metal：部分支持
Core ML：需要转换
```

结果可能是：

- 完全在目标设备上运行。
- 部分算子回退 CPU。
- CPU/GPU 之间发生数据复制。
- 模型加载成功但性能较差。
- 因缺少 kernel 或 custom operator 直接失败。

### Candle 的限制

Candle 的适配成本主要来自模型实现和 kernel：

1. 需要手动实现模型结构和权重映射。
2. 某些算子需要使用基础算子重写。
3. 某些算子需要增加 Candle custom op。
4. CUDA、Metal 和 CPU 的算子覆盖与性能不一定一致。
5. FlashAttention、fused attention、特殊 int8/int4 kernel 可能需要单独实现。
6. GPU/CPU 之间频繁同步会抵消 Tensor runtime 的性能优势。

### MLX 的限制

MLX 是 Apple 面向 Apple Silicon 设计的数组和机器学习框架，特点包括：

- CPU 和 GPU 共享统一内存。
- 延迟执行计算图。
- CPU/GPU 可以操作同一组数组。
- GPU 侧使用 Metal。
- 官方主要提供 Python 和 C++ API。

MLX 的统一内存可以减少显式数据复制，但不代表内存无限，也不代表所有算子都在 GPU 上高效执行。
模型权重、KV cache、音频特征和系统内存会共同竞争统一内存。

MLX 的延迟执行还要求谨慎处理同步：频繁把 GPU 结果读取到 CPU 做控制流，可能产生额外同步。

如果 Rust 调用 MLX，通常需要：

```text
Rust
  -> C/C++ FFI 或社区 binding
  -> MLX C++ API
  -> Metal
```

这不是 Python 兼容层，但会引入 C++ ABI、Xcode/Metal 构建、unsafe FFI 和 macOS 专属打包边界。

MLX 也允许通过 C++/Metal 编写自定义 operation，因此缺失算子仍然需要工程实现，而不是自动解决。

参考：

- [MLX 官方介绍](https://ml-explore.github.io/mlx/)
- [MLX Unified Memory](https://github.com/ml-explore/mlx/blob/main/docs/src/usage/unified_memory.rst)
- [MLX Lazy Evaluation](https://github.com/ml-explore/mlx/blob/main/docs/src/usage/lazy_evaluation.rst)
- [MLX Custom Extensions](https://ml-explore.github.io/mlx/build/html/dev/extensions.html)

## Candle 是否应该重写现有 backend

### 不建议直接替换

当前 `whisper-rs` 使用 whisper.cpp 的 Whisper 专用实现。whisper.cpp 已针对 CPU、Apple Silicon、
Metal 和量化路径做了长期优化。仅仅换成 Candle 不会自动获得性能提升。

当前 `sherpa-onnx` 则是 ONNX Runtime 之上的多模型族语音 runtime，包含模型配置、解码和 streaming
等逻辑。用 Candle 重写意味着要重新实现这些能力。

因此不建议：

```text
删除 whisper-rs
删除 sherpa-onnx
用一个 Candle backend 替代全部能力
```

### 建议并行增加

更稳妥的结构是：

```text
ASR Backend Manager
├── whisper-rs
│   └── Whisper GGML/GGUF
├── sherpa-onnx
│   ├── Paraformer ONNX
│   ├── Zipformer ONNX
│   ├── Whisper ONNX
│   └── Qwen3 ASR ONNX
└── candle
    ├── Whisper safetensors
    ├── Paraformer safetensors
    └── 其他 Candle 原生模型
```

所有 backend 统一输出 MeetNote 的 `AsrResult`，然后用相同测试集比较：

- 模型加载时间。
- 单位音频处理时间。
- 实时率 RTF。
- 峰值内存。
- CPU/GPU 利用率。
- WER/CER。
- 时间戳误差。
- 长音频稳定性。
- 中英文混合语音表现。

只有在相同模型、相同音频、相同设备和相同质量指标下完成测试，才能宣称 Candle 性能更好。

## 当前 MeetNote 模型和 backend 矩阵

模型目录位于：

```text
resources/models/catalog.json
```

### 当前真正可运行的模型

| 任务 | 模型 | 格式 | backend | 当前状态 |
|---|---|---|---|---|
| ASR | `whisper-base-en-ggml` | GGML | `whisper-rs` | 已实现 |
| ASR | `whisper-large-v3-ggml` | GGML | `whisper-rs` | 已实现 |
| Diarization | `sherpa-onnx-speaker-diarization-bundle` | ONNX bundle | `sherpa-onnx` | 已实现 |

当前应用实际注册的 backend 位于 [apps/meetnote/src/main.rs](../../apps/meetnote/src/main.rs)：

- ASR：`whisper-rs`、`sherpa-onnx`
- Diarization：`sherpa-onnx`、`diarization-native`

### 目录中的 ASR 候选模型

#### Whisper PyTorch 模型

OpenAI Whisper PyTorch 条目和没有 Rust backend 的 Mimo 条目已从当前 catalog 移除；当前 Whisper
catalog 只保留可被 `whisper-rs` 识别的 GGML 来源。PyTorch `.pt` 文件不能直接交给 `whisper-rs`，
如果未来重新引入，必须先经过明确的转换或独立 native adapter。

```text
.pt -> 转换为 GGML/GGUF -> whisper-rs
.pt/.safetensors -> Candle Whisper adapter -> Candle
```

#### `paraformer-zh`

- 规划 backend：`sherpa-onnx`
- 目标格式：ONNX 模型族 bundle
- 当前状态：已实现 Paraformer adapter，并保留 Qwen3 ASR adapter 的目录校验和配置路径；Qwen3
  catalog 条目仍是 candidate source，尚未形成可直接安装的嵌套目录 manifest。

#### `qwen3-asr-0.6b`

- 规划 backend：`sherpa-onnx`
- 当前状态：`asr-sherpa-onnx` 已实现模型族配置，但 catalog archive 仍需目录成员下载支持。

#### `mimo-v2.5-asr`

- 已从当前 catalog 移除，因为没有已注册的 Rust-native backend；未来重新加入前必须先完成明确的
  模型族 adapter，不能假设它可以直接作为任意 ONNX 模型运行。

### 目录中的 diarization 候选模型

#### `sherpa-onnx-speaker-diarization-bundle`

当前已实现，至少包含：

```text
segmentation.onnx
embedding.onnx
```

执行流程是：

```text
speaker segmentation
  -> speaker embedding
  -> clustering
  -> diarization segments
```

#### `pyannote-speaker-diarization-3.1`

- 规划 backend：`diarization-native`
- gated Hugging Face 模型，需要授权
- 当前 `diarization-native` 已接入 `speakrs` 的 community-1 风格完整 pipeline，并通过固定
  revision 的 CPU bundle 完成真实模型验收；Pyannote 3.1 仍是 gated、无直接 Rust adapter 的来源。

## Speaker embedding、重叠语音与个人 speaker profile

### Embedding 不能直接完成声音分离

需要区分：

```text
diarization
  = 判断谁在什么时候说话

source separation
  = 把混合声音真正拆成多路音频
```

当两个人同时说话时，普通 speaker embedding 模型对混合音频得到的通常是：

```text
embedding(A + B)
```

它不能可靠地从一个混合向量还原出：

```text
embedding(A) + embedding(B)
```

要真正拆出两路音频，需要 speech separation 或 target-speaker extraction 模型：

```text
混合音频
  -> speech separation
  -> 音频 A、音频 B
  -> 分别提取 embedding
```

但是，支持重叠说话的 segmentation 可以输出多个局部 speaker track 或 mask：

```text
时间窗口
  ├── track 0 / mask 0
  └── track 1 / mask 1
```

如果 embedding adapter 支持 masked/weighted 输入，就可以分别得到：

```text
mask 0 + 音频 -> embedding(A)
mask 1 + 音频 -> embedding(B)
```

这属于“按照 segmentation 提供的局部轨道提取身份特征”，不是从一个普通混合 embedding 中分离声音。

### Embedding 与 clustering 的持久化

embedding 可以保存为个人 speaker profile，用于跨项目自动标注：

```text
已知身份的语音样本
  -> speaker embedding
  -> SpeakerProfile

新音频
  -> segmentation
  -> 每个 track 提取 embedding
  -> 与已知 profile 比较
  -> 匹配成功则自动标注姓名
  -> 未匹配片段再进行未知 speaker clustering
```

因此，pipeline 中的 `clustering` 需要拆成两个职责：

```text
SpeakerIdentityResolver
├── KnownSpeakerMatcher
│   └── embedding 与已知 profile 匹配
└── UnknownSpeakerClusterer
    └── 对未匹配片段聚类
```

`speaker-0`、`speaker-1` 只应被视为当前音频中的匿名 cluster ID，不能作为跨项目稳定身份。跨项目身份
应使用独立的 speaker UUID 和用户标签。

建议使用不依赖 SQLite 的 JSON speaker library：

```text
data/speakers/library.json
```

核心数据结构：

```text
SpeakerProfile
├── uuid
├── display_name
├── aliases
├── embeddings[]
├── embedding_model_id
├── embedding_model_revision
├── backend_id
├── sample_rate
├── feature_config
├── match_threshold
├── created_at
└── updated_at
```

项目内还应保留中间观测值，以便重新聚类和诊断：

```text
SpeakerEmbeddingObservation
├── uuid
├── project_id
├── source_segment_id
├── start_ms
├── end_ms
├── track_id
├── vector
├── quality
├── model_id
├── model_revision
└── identity_match
```

不同 embedding 模型不在同一个向量空间，不能直接比较：

```text
3D-Speaker vector != WeSpeaker vector != ECAPA vector
```

更换 embedding 模型、特征提取参数或采样率后，需要重新生成 profile，或明确执行一次 profile migration。
embedding 属于可识别个人声音的敏感数据，持久化时必须保留来源和模型版本，并在产品层提供删除和重新
生成能力；不能把它当作普通缓存无限期保留。

### 当前可用的 speaker embedding 框架

| 框架/库 | embedding 能力 | pipeline 能力 | 适合作为 |
|---|---|---|---|
| `sherpa-onnx` Rust API | 独立 `SpeakerEmbeddingExtractor`，并提供命名 speaker manager | 高层离线 diarization | `diarization-sherpa-onnx` 的个人 speaker 匹配补充 |
| `diarization` crate | WeSpeaker ResNet34-LM，支持普通、masked、weighted 和 batch embedding | Pyannote 风格 offline/streaming、AHC、PLDA、VBx | `diarization-native` 的高质量基础 |
| `speakrs` | 完整 Pyannote community-1 风格 pipeline 的 embedding 阶段 | powerset、overlap-add、PLDA、VBx、CoreML/ONNX Runtime | `diarization-native` 的完整 pipeline 候选 |
| `pyannote-rs` | WeSpeaker embedding | 轻量 segmentation + embedding + cosine clustering | 轻量 native profile |

`diarization` crate 的公开 API 将 WeSpeaker embedding 定义为可持久化/比较的 speaker fingerprint，并提供
`embed_masked`、`embed_weighted` 和 batch 路径。[diarization embedding API](https://docs.rs/diarization/latest/diarization/embed/)

`speakrs` 则将 segmentation、重叠语音处理、embedding、PLDA 和 VBx clustering 组合成完整的 Rust pipeline，
推理可以使用 ONNX Runtime 或 native CoreML，不需要 Python runtime。[speakrs 文档](https://docs.rs/speakrs/latest/speakrs/)

### `diarization-sherpa-onnx` 是否可以实现个人 speaker 功能

可以实现一部分，而且不需要 Python。

当前 sherpa-onnx 1.13.6 的 Rust API 暴露了：

```text
SpeakerEmbeddingExtractor
  -> create_stream
  -> is_ready
  -> compute
  -> Vec<f32>

SpeakerEmbeddingManager
  -> add(name, embedding)
  -> add_list(name, embeddings)
  -> search(embedding, threshold)
  -> verify(name, embedding, threshold)
```

这意味着 `diarization-sherpa-onnx` 可以增加：

```text
已知 speaker 样本
  -> SpeakerEmbeddingExtractor
  -> 保存 SpeakerProfile

新片段
  -> SpeakerEmbeddingExtractor
  -> SpeakerEmbeddingManager::search
  -> 已知 speaker 自动标注
```

但是，当前高层 `OfflineSpeakerDiarization::process()` 返回的结果只提供：

- diarization segments
- speaker index
- speaker count

它不提供：

- 每个 segment 的 embedding
- segmentation mask
- 局部 speaker track
- 内部 clustering 输入
- cluster center

因此有三个实现层次：

#### 方案 A：独立 speaker embedding service

在当前 sherpa backend 旁边独立创建 embedding extractor，对人工选择的干净语音片段提取 profile，也可以
对最终 diarization segment 重新提取 embedding。

优点：

- 改动小。
- 可以使用现有 3D-Speaker embedding ONNX。
- 可以使用 `SpeakerEmbeddingManager` 做已知 speaker 匹配。

限制：

- 不能获得 sherpa 内部真正用于 clustering 的 embedding。
- 如果最终 segment 已经合并了重叠语音，重新提取的 embedding 可能是混合向量。
- 不能恢复被高层 API 隐藏的 mask 和局部 track。

#### 方案 B：拆开 sherpa pipeline

分别调用 segmentation、embedding 和 clustering，把中间结果纳入 MeetNote contract。

优点：

- 可以保留 `SpeakerEmbeddingObservation`。
- 可以先匹配已知 profile，再对未知片段聚类。
- 可以更准确地处理 track 和重叠语音。

限制：

- 当前高层 Rust wrapper 没有直接提供完整的中间结果接口。
- 需要使用更底层 sherpa API、扩展上游 wrapper，或者在 MeetNote 中重新编排 pipeline。
- 需要严格复现 sherpa 的窗口、特征和后处理行为。

#### 方案 C：由 `diarization-native` 承担可观察 pipeline

将 segmentation、embedding、identity matching 和 clustering 全部拆成 MeetNote 自己的 adapter contract，
使用 `diarization` crate、`speakrs` 或其他 Rust/native 实现作为具体 provider。

优点：

- 中间 embedding、mask、track 和 cluster 都可以进入统一 contract。
- 更容易实现个人 profile、跨项目 speaker 标注和重新聚类。
- 可以同时支持 offline 和 streaming。

这是长期推荐方案。`diarization-sherpa-onnx` 继续作为稳定的快速 pipeline，`diarization-native` 负责更完整、
可观察、可扩展的 speaker intelligence 能力。

## Diarization-native 的正式能力边界

`diarization-native` 不应只是另一个“调用 diarization crate 的占位 backend”，而应提供以下能力：

```text
Native Diarization Pipeline
├── SegmentationAdapter
│   ├── Pyannote segmentation
│   ├── Reverb segmentation
│   ├── overlap-aware tracks/masks
│   └── offline/streaming windows
├── EmbeddingAdapter
│   ├── WeSpeaker
│   ├── 3D-Speaker
│   ├── ECAPA-TDNN
│   ├── x-vector
│   └── masked/weighted/batch embedding
├── SpeakerIdentityResolver
│   ├── known speaker profile matching
│   └── unknown speaker clustering
├── ClusteringAdapter
│   ├── cosine
│   ├── AHC
│   ├── spectral
│   ├── PLDA
│   └── VBx
└── ArtifactStore
    ├── SpeakerProfile
    └── SpeakerEmbeddingObservation
```

最终输出不应只返回匿名 speaker segments，还应允许调用方选择是否获取：

```text
DiarizationResult
  + SpeakerEmbeddingObservation[]
  + ClusterResult
  + IdentityMatch[]
  + QualityDiagnostics
```

其中 embedding 输出必须携带：

- embedding model id
- model revision
- backend/runtime
- provider/device
- sample rate
- feature configuration
- vector dimension
- normalization method
- source time range
- track/mask identity

这样才能安全地进行跨项目复用、阈值校准和模型迁移。

## whisper-rs 与 sherpa-onnx 的实际接收范围

### whisper-rs

`whisper-rs` 是 whisper.cpp 的 Rust binding，因此是 Whisper 专用 backend，不是通用 ASR runtime。

当前 catalog 只声明以下两类实际模型：

```text
Whisper GGML/GGUF artifact
  -> whisper-rs
```

不能直接接收：

- Paraformer ONNX。
- Qwen3 ASR ONNX 或 safetensors。
- 普通 PyTorch `.pt`。
- 任意 ONNX ASR。

当前仓库的 `probe()` 主要检查 artifact path 是否为文件，真正的模型兼容性由 whisper.cpp 在加载阶段
判断。因此：

```text
文件存在 != 模型兼容
```

下载完成后的 backend probe 只做快速 artifact 与模型入口校验；完整模型加载仍在实际推理时执行。
文件存在、快速 probe 通过和真实推理成功是三个不同的状态，不能合并为一个 readiness 结论。

### sherpa-onnx ASR

当前仓库的 `asr-sherpa-onnx` 已实现模型族适配、目录校验、音频解码和离线推理入口；
Paraformer 路径可直接使用已安装目录，Qwen3 路径等待 catalog v2 的嵌套目录安装协议。

上游 sherpa-onnx runtime 本身支持多个 ASR 模型族，例如：

- Zipformer Transducer。
- Zipformer CTC。
- Whisper。
- SenseVoice。
- Paraformer。
- Moonshine。
- FireRedAsr。
- WeNet。
- Omnilingual。
- Qwen3。
- Parakeet TDT。
- NeMo CTC。

但这些模型需要不同的模型配置、文件组合、输入输出定义和解码器，不能仅凭“都是 ONNX”直接互换。

参考：[sherpa-onnx Offline ASR Models](https://github.com/k2-fsa/sherpa-onnx/blob/master/sherpa-onnx/c-api/docs/offline-asr.dox)

## 算子是否统一

### whisper-rs

whisper-rs 使用 Whisper 专用 runtime：

```text
Whisper model
  -> whisper.cpp model representation
  -> ggml tensor and operators
  -> Whisper decoder
```

它内部当然也会使用 MatMul、Conv、Softmax 和 LayerNorm 等基础计算，但这些算子属于 Whisper runtime
实现的一部分，不是面向任意模型的统一图接口。

### sherpa-onnx

sherpa-onnx 使用 ONNX Runtime 的统一计算图执行层，但模型族的图接口不同：

```text
CTC:
  encoder -> logits -> CTC decode

Transducer:
  encoder + decoder + joiner -> decode

Paraformer:
  encoder + predictor + decoder -> decode

Whisper:
  encoder + decoder -> token decode
```

它们可能共享 ONNX 基础算子，但不共享：

- 输入 Tensor。
- 输出 Tensor。
- shape。
- 模型组件。
- tokenizer。
- 解码算法。
- streaming 状态。

所以正确的结论是：

```text
ONNX 基础算子层面：部分统一
模型图层面：不统一
模型族适配层面：不统一
MeetNote application contract：统一
```

## 转换规则

### 不需要转换

满足以下条件时可以直接使用：

```text
已有对应模型族 adapter
  + artifact format 兼容
  + 所需算子存在
  + dtype 和 device provider 兼容
```

例如：

```text
预转换 Whisper GGML -> whisper-rs
```

### 需要转换或重新适配

常见路径包括：

```text
PyTorch Whisper .pt
  -> GGML/GGUF
  -> whisper-rs
```

```text
PyTorch/safetensors
  -> ONNX graph
  -> sherpa-onnx / ONNX Runtime
```

```text
PyTorch/safetensors
  -> Candle model implementation
  -> Candle backend
```

```text
ONNX
  -> Core ML
  -> Core ML/ANE backend
```

转换不是修改文件扩展名，而是重新表达模型结构和权重布局，可能改变：

- Tensor 名称。
- 权重排列。
- dtype。
- 量化方式。
- 算子表达。
- 动态 shape。
- tokenizer 关联。
- 时间戳行为。

每次转换都必须经过：

```text
结构检查
  -> 权重加载检查
  -> 小样本推理
  -> 端到端音频测试
  -> WER/CER/DER
  -> 内存和实时率测试
```

## 模型页面应表达的状态

模型页面不能只显示“下载”或“已安装”，至少需要分别展示：

| 维度 | 示例 |
|---|---|
| 模型族 | Whisper、Paraformer、Zipformer、Pyannote |
| 任务 | ASR、Diarization |
| 来源格式 | PyTorch、safetensors、ONNX、GGML |
| 运行格式 | GGML/GGUF、ONNX bundle、Candle weights |
| 目标 backend | whisper-rs、sherpa-onnx、Candle |
| backend 状态 | 已实现、candidate source、未注册 |
| 安装状态 | 缺失、部分安装、已安装 |
| 转换要求 | 不需要、需要转换、转换工具缺失 |
| 授权要求 | 无需授权、需要 token |
| 设备支持 | CPU、Metal、CUDA、Core ML |
| 验证状态 | 未验证、probe 通过、端到端验证通过 |

推荐的 catalog 语义是：

```json
{
  "modelFamily": "whisper",
  "sourceFormat": "pytorch",
  "runtimeFormat": "ggml",
  "candidateBackends": ["whisper-rs"],
  "requiresConversion": true,
  "conversionTool": "whisper.cpp conversion script",
  "requiredArtifacts": ["model.bin"],
  "supportedDevices": ["cpu", "metal"]
}
```

`candidateBackends` 只能表示候选关系，不能直接表示“当前可运行”。模型管理器需要结合 backend registry、
artifact probe 和设备 probe 计算最终 readiness。

## MeetNote 的工程建议

ASR 和 diarization 继续保持独立 manager，application 只依赖统一 contract：

```text
ASR Backend Manager
  -> selected backend
  -> model-family adapter
  -> normalized AsrResult

Diarization Backend Manager
  -> selected backend
  -> model-family adapter
  -> segmentation / embedding / identity resolution / clustering
  -> normalized DiarizationResult
```

标准化结果写入目标目录文档后，document service 才刷新三层项目树。模型 backend 不读取项目树，不因 Assistant 或外部 Agent 的语义修改而覆盖原始转录、speaker 段或 embedding observation。

后续新增 backend 时：

- 不修改 domain contract。
- 不修改 application pipeline 的选择接口。
- 不把模型族差异塞入一个巨型 manager。
- 不把任意 ONNX 视为通用模型。
- 不在运行时启动 Python subprocess。
- 每个模型条目都记录来源、license、格式、转换和验证方式。
- 已知 speaker 匹配和未知 speaker 聚类必须分开建模。
- embedding observation 必须记录模型、版本、采样率、特征配置和来源时间段。

推荐的 backend 演进顺序：

```text
1. 完善 whisper-rs 和 sherpa-onnx 的 artifact/runtime probe
2. 为 sherpa-onnx 增加独立 speaker embedding extractor 和 profile matching
3. 定义 segmentation、embedding、identity resolution 和 clustering 的公共 contract
4. 由 diarization-native 接入可观察的 Pyannote 风格 pipeline
5. 增加 asr-candle 的 Whisper 或其他明确模型族
6. 用统一基准比较不同 backend
7. 再决定是否为 Apple Silicon 增加 MLX 专用 backend
```

MLX 更适合作为 Apple Silicon 专用可选 backend，而不是跨平台基础 backend。Candle 更适合作为可控的
Rust-native 模型实现路径；whisper-rs 和 sherpa-onnx 则分别保留其已经成熟的 Whisper 专用和多模型族
ONNX runtime 优势。

## 探索结果

当前 MeetNote 中最重要的边界如下：

```text
whisper-rs
  = Whisper 专用，当前使用 GGML/GGUF 路线，已实现

sherpa-onnx diarization
  = ONNX speaker segmentation + embedding + FastClustering，已实现
    高层结果不暴露 embedding、mask 或内部 cluster 输入
    独立 SpeakerEmbeddingExtractor/Manager 可补充 profile matching

sherpa-onnx ASR
  = 已实现 Paraformer/Qwen3 模型族 adapter，catalog 仍需区分可安装与 candidate source

Candle
  = Rust Tensor/NN framework，需要独立模型族 adapter，尚未接入

MLX
  = Apple Silicon 专用数组/ML framework，适合单独的 Apple backend

speaker profile
  = 独立的 UUID、标签和 model-versioned embedding 集合
    用于跨项目已知 speaker 自动标注

diarization-native
  = 可观察、可重用的 segmentation/embedding/identity/clustering pipeline
    负责重叠语音、中间 embedding、profile matching 和 unknown clustering
```

因此当前最合理的策略不是寻找一个“统一格式”或“统一算子”的 runtime，而是在统一的 MeetNote
backend contract 下，允许不同模型族使用不同 artifact 和不同推理实现。

## 注意事项与补充

- “Rust-native”表示 application 和 backend 不依赖 Python subprocess，不表示不能使用 Rust 绑定的 C/C++ runtime。
- whisper-rs 底层包含 whisper.cpp，不应把“Rust crate”理解成所有推理代码都由 Rust 实现。
- sherpa-onnx 的 Rust wrapper 可以是 Rust-native API，但底层仍然依赖其 native/ONNX Runtime 能力。
- `safetensors` 的安全性只解决序列化加载风险，不解决模型架构和算子兼容性。
- ONNX 的可移植性只覆盖计算图表示，不保证所有硬件 provider 具有相同性能或完整算子覆盖。
- Apple Silicon 的统一内存减少了 CPU/GPU 显式复制，但不能替代内存容量、带宽和 kernel 优化。
- 模型页面必须把“可下载”“已安装”“backend 已实现”“probe 通过”和“端到端验证通过”分开表示。
- speaker embedding 不是临时中间变量；如果启用跨项目标注，必须作为带模型版本的独立 artifact 管理。
- 当前 sherpa 高层 diarization 结果不包含 embedding；要实现精确的 embedding 复用，需要独立 extractor 或可观察 pipeline。
- speaker library 的 JSON 只保存跨文档说话人身份资产；目标目录文档 JSON 才保存项目关系。两者不得合并为不可重建的全局项目数据库。
- Assistant 和外部 Agent 对文档的修改必须进入同一文档接口，并接受固定三层项目树约束；模型 backend 不参与语义拆分决策。
