# Diarization 三阶段模型、Rust 实现与 `diarization-native` 设计

## 探索原因

MeetNote 当前同时拥有 `diarization-sherpa-onnx` 和 `diarization-native` 两条离线路径。
两者都能输出最终 speaker segment；当前 contracts 还会传递 speaker embedding observation，
application 的共享 identity service 使用这些 observation 进行跨项目 profile matching。
`diarization-native` 已通过 `speakrs` 接入固定 revision 的完整 pipeline；本文件保留其阶段边界、
模型选择依据和可替换性约束，不再描述占位实现状态。

本次探索解决三个直接影响架构的问题：

- segmentation、embedding、clustering 各阶段有哪些当前效果较好的方案；
- 哪些方案能够在不启动 Python subprocess 的前提下由 Rust 直接实现或通过 Rust package 调用；
- 如何让 `diarization-native` 同时支持完整三阶段 pipeline、可观察中间结果和跨项目 speaker 自动标注。

## 探索目标

1. 只比较有论文、官方模型卡、官方仓库或可复现 benchmark 的模型和算法。
2. 区分 stage 指标、speaker verification EER 和端到端 diarization DER，禁止把三者混成一个排名。
3. 确定一个能在 MeetNote 当前 Rust workspace 中实际编译、测试和安装的默认组合。
4. 定义独立的 segmentation、embedding、clustering contract，同时允许成熟上游实现使用融合优化路径。
5. 定义不依赖 SQLite 的个人声纹 JSON store、模型空间隔离、匹配阈值和删除边界。

## 评价口径

### 指标不能混用

| 指标 | 衡量内容 | 可以证明什么 | 不能证明什么 |
|---|---|---|---|
| DER | missed speech + false alarm + speaker confusion | 完整 diarization pipeline 在指定数据集的效果 | 单独某个 embedding 模型一定更好 |
| WDER | 词级 speaker assignment error | diarization 与词时间戳结合后的 speaker 标注效果 | 纯时间段 DER 一定更低 |
| EER | speaker verification 等错误率 | 同一测试协议下 embedding 的身份区分能力 | 放进任意 segmentation/clustering 后的 DER |
| minDCF | 指定先验和代价下的 verification 检测成本 | verification 部署阈值质量 | speaker change 时间戳质量 |

不同论文的训练集、collar、overlap 计分、oracle VAD、speaker 数量和 domain 不同。本文只在同一来源、
同一测试协议内做数值比较；跨来源只做工程候选判断。

## 三阶段边界

```text
16 kHz mono audio
  -> segmentation
       输出局部 speaker activity、track/mask、overlap
  -> embedding
       对每个有效局部 track 提取固定维度 speaker vector
  -> clustering
       将跨窗口的局部 track 合并为全局匿名 speaker
  -> identity resolution
       已知 profile 匹配 + 未知 cluster 保留
  -> normalized DiarizationResult
```

identity resolution 不是 clustering 的同义词。clustering 只回答“这些片段是否来自同一个未知 speaker”；
profile matching 回答“这个 cluster 是否是已登记的某个人”。

## 第一阶段：segmentation

### 候选模型

| 模型/体系 | 结构和能力 | 公开效果证据 | Rust 路线 | 结论 |
|---|---|---|---|---|
| Pyannote `segmentation-3.0` | 10 秒滑窗、powerset、多 speaker activity、重叠语音 | Pyannote 3.0 相比 2.1 在 DIHARD 3 从 26.9% DER 降到 21.7%，AMI SDM 从 27.1% 降到 22.2%；这是完整 pipeline 变化，不是纯 segmentation ablation | `speakrs`、`diarization`、`pyannote-rs`、`sherpa-onnx` 均有 Rust/ONNX 路线 | 默认开源基线，模型、算子和后处理最成熟 |
| Pyannote `community-1` segmentation | 新 pipeline 的 powerset segmentation，与 WeSpeaker、PLDA、VBx 配套 | 官方 2025-09 表中，完整 pipeline 相比 3.1：AISHELL-4 12.2→11.7，AliMeeting 24.5→20.3，AMI SDM 22.7→19.9，DIHARD 3 21.4→20.2 | `speakrs 0.5` 已实现 community-1 风格完整 Rust pipeline | 当前默认实现的行为目标 |
| Rev `reverb-diarization-v1` | Pyannote-compatible segmentation，面向词级 speaker assignment 优化 | 官方模型卡称在 125 万以上 token、五套测试上相对 Pyannote 3.0 改善 16.5% WDER；earnings21 WDER 0.047、rev16 0.077 | sherpa-onnx 已提供 ONNX 转换和 Rust API | 技术可用，但许可证限制为非生产环境，不能作为 MeetNote 默认生产模型 |
| NeMo MarbleNet VAD + MSDD | VAD 后做多尺度 embedding，再由神经 diarizer 细化 speaker activity | NVIDIA 文档称双人数据相对单尺度 clustering diarizer 可最多降低 60% DER | 模型主要以 `.nemo`/PyTorch 体系提供；无完整官方 Rust package | 适合未来明确导出和逐算子验证后的独立 adapter，不进入当前默认实现 |
| Sortformer | Transformer end-to-end 直接输出 speaker activity，支持 offline/online | NVIDIA 将其作为当前端到端 diarization 主线 | 可导出 ONNX 的部分不能自动等价于完整 Rust adapter | 它不是可互换的“三阶段第一阶段模型”，应作为独立 end-to-end backend |

Pyannote 官方 2025-09 完整 pipeline DER（越低越好）：

| 数据集 | legacy 3.1 | community-1 | precision-2 |
|---|---:|---:|---:|
| AISHELL-4 | 12.2 | 11.7 | 11.4 |
| AliMeeting channel 1 | 24.5 | 20.3 | 15.2 |
| AMI IHM | 18.8 | 17.0 | 12.9 |
| AMI SDM | 22.7 | 19.9 | 15.6 |
| DIHARD 3 full | 21.4 | 20.2 | 14.7 |
| VoxConverse v0.3 | 11.2 | 11.2 | 8.5 |

`precision-2` 是商业服务/自托管产品，不是可随应用分发的开源 Rust 模型，因此只作为质量上界，不是
`diarization-native` 候选。

### segmentation 后端需求

- 固定记录 sample rate、window、step、frame duration 和 frame step。
- 保留 powerset 解码和 overlap 信息，不能提前压成单 speaker VAD。
- 长音频按滑窗推理；窗口可以并行，但 overlap-add、PIT/track 对齐必须在确定顺序下聚合。
- 输出必须携带 local track ID，供 embedding 和后续诊断关联。
- Core ML/ONNX Runtime 结果允许浮点微差，但 shape、frame 数和非有限值必须严格校验。

## 第二阶段：speaker embedding

### 同一 3D-Speaker benchmark 内的比较

以下是 3D-Speaker 官方仓库给出的 fully-supervised speaker verification EER。数值只用于比较 embedding
身份区分能力，不直接等于 diarization DER。

| 模型 | 参数量 | VoxCeleb1-O EER | CNCeleb EER | 3D-Speaker EER |
|---|---:|---:|---:|---:|
| ResNet34 | 6.34M | 1.05% | 6.92% | 7.29% |
| ECAPA-TDNN | 20.8M | 0.86% | 8.01% | 8.87% |
| ERes2Net-base | 6.61M | 0.84% | 6.69% | 7.21% |
| CAM++ | 7.2M | 0.65% | 6.78% | 7.75% |
| ERes2NetV2 | 17.8M | 0.61% | 6.14% | 6.52% |
| ERes2Net-large | 22.46M | 0.52% | 6.17% | 6.34% |

ERes2NetV2 论文还报告 VoxCeleb1-O 在完整、3 秒、2 秒 utterance 上分别为 0.61%、0.98%、1.48% EER，
说明它对 diarization 中常见的短片段有明确优势。

### Rust 可实现性

| 模型族 | 官方/成熟 Rust package | ONNX 路线 | masked/weighted embedding | 与完整 clustering 参数配套 | 当前选择 |
|---|---|---|---|---|---|
| WeSpeaker ResNet34 / ResNet34-LM | `speakrs`、`diarization`、`pyannote-rs`；sherpa-onnx 官方 Rust API | 完整 | `speakrs`/`diarization` 支持 | community-1 PLDA/VBx 参数完整 | 默认 |
| WeSpeaker ResNet293-LM、CAM++ | sherpa-onnx 官方 Rust API | 完整 | sherpa 高层 pipeline 内部处理 | 没有与每个模型一一校准的 MeetNote PLDA/VBx bundle | 可选 sherpa backend |
| 3D-Speaker ERes2Net/ERes2NetV2 | sherpa-onnx 官方 Rust API 支持多种已导出 ONNX | 完整 | sherpa pipeline 支持 | verification 指标强，但当前 `speakrs` community-1 参数针对 WeSpeaker | 后续 embedding adapter + 重新校准 |
| ECAPA-TDNN | sherpa-onnx 支持 3D-Speaker/NeMo 模型 | 完整 | 依 runtime adapter | 需要对应 domain 阈值/PLDA | 后续候选 |
| NeMo TitaNet | sherpa-onnx 官方部署模型；NeMo 原生无 Rust package | sherpa 转换可用 | sherpa pipeline 支持 | NeMo MSDD 配套但 MSDD 本体不是当前 Rust pipeline | sherpa 候选 |
| WavLM/Whisper-PMFA embedding | 当前主实现和训练 recipe 以 PyTorch 为主 | 可导出不等于已验证 | 需要新增特征和 mask 适配 | 无当前本地 bundle | 研究候选 |

### 为什么默认不是 EER 最低的 ERes2Net-large

默认组合必须整体闭合：

```text
模型权重
  + 精确 fbank 配置
  + overlap-aware mask
  + embedding normalization
  + PLDA 参数
  + AHC/VBx 阈值
  + 可复现 DER
```

WeSpeaker ResNet34 的单项 EER 不是表中最低，但 `speakrs` 提供与 community-1 pipeline 配套的 PLDA/VBx、
masked embedding、中间结果和端到端 benchmark。直接把 ERes2NetV2 换进去而沿用 WeSpeaker 的 PLDA 与阈值
会混用向量空间，是错误实现。

### embedding 后端需求

- 输入为 16 kHz mono 和 segmentation mask，而不是任意裁切音频。
- 输出必须校验 dimension、finite、L2 norm 和最短有效语音长度。
- 每个 observation 记录 model ID、revision、feature config、sample rate、normalization、track 和时间范围。
- 不同模型或 revision 默认视为不同向量空间；未经过显式 migration 不允许跨空间 cosine 比较。
- batch 和跨窗口并行只改变执行顺序，不改变 observation 与 track 的确定映射。

## 第三阶段：clustering

clustering 通常不是单个神经模型，而是一组距离、统计模型和时间序列推断算法。

| 算法 | 优点 | 主要限制 | Rust 状态 | 使用位置 |
|---|---|---|---|---|
| Cosine threshold / online centroid | 简单、低延迟、适合 streaming | speaker 数和阈值敏感，缺少全局时间建模 | 容易纯 Rust 实现；`pyannote-rs`、sherpa manager 已有 | profile matching 或低延迟路径，不作为默认离线 clustering |
| AHC | 确定性、易解释，可用 threshold 或已知 cluster 数 | 对 domain、linkage、embedding calibration 敏感 | `speakrs`、`diarization/diaric`、sherpa 均有 | 默认 pipeline 的初始化 |
| Spectral / NME-SC | speaker 数估计和多尺度 embedding 上常用 | O(n²) 相似度矩阵、长音频内存高、参数复杂 | 可纯 Rust 实现；NeMo 官方实现主要在 Python | 后续长音频分块/稀疏实现候选 |
| PLDA + VBx | 同时利用 speaker 分布与时间连续性；CALLHOME、AMI、DIHARD 上有强基线 | 必须使用匹配 embedding 空间的 PLDA 参数 | `speakrs` 和 `diarization/diaric` 已有 Rust 实现 | 默认离线路径 |
| UMAP + HDBSCAN | 对非球状 cluster 和未知 speaker 数灵活 | 结果稳定性、超参数、时间信息和小 cluster 处理更难 | WeSpeaker 有 recipe；无项目内验证过的完整 Rust 配置 | 不作为默认 |
| MSDD | 多尺度、overlap-aware、可在 domain 上训练 | 它是 clustering 初始化后的神经 diarizer，不是简单可替换 clusterer | 无完整官方 Rust package | 未来独立 adapter |

Pyannote 4.0 将 `community-1` 从传统 AHC 主线切换到 VBx，官方说明目的就是改善 speaker assignment 和
speaker counting。VBx 原始论文在 CALLHOME、AMI、DIHARD II 上相对常见 clustering 方法表现更好；
这也是默认组合使用 `AHC -> PLDA -> VBx` 而不是单独 cosine threshold 的原因。

### clustering 后端需求

- 输入只接受同一 embedding space 的有限向量。
- AHC 距离、linkage、threshold 和 PLDA/VBx 参数必须版本化。
- `expected_speakers` 是 clustering constraint；为零直接拒绝。融合 provider 不支持精确 constraint 时不得
  伪造结果，应在能力声明中标明为 speaker-count hint。
- 长音频的 pairwise 矩阵需要内存上限或 spill 策略。
- cluster ID 只在当前音频内稳定；不能拿 `speaker-0` 当跨项目身份。

## Rust 实现候选的完整比较

| package/runtime | 三阶段可观察性 | 重叠语音 | clustering | 中间 embedding | CPU/Core ML | 端到端证据 | 结论 |
|---|---|---|---|---|---|---|---|
| `speakrs 0.5` | 完整 result 暴露 segmentation、embedding、hard cluster、segments | powerset + overlap-add + masked embedding | AHC + PLDA + VBx | 有 | CPU；可选 native Core ML | 官方仓库报告 VoxConverse dev 7.1% DER、test 11.1% DER；需在 MeetNote 数据集复验 | 默认融合 engine |
| `diarization 0.1` + `diaric` | stage API 和 backend-free core 清晰 | masked/weighted embedding | AHC + PLDA + VBx | 有 | ONNX Runtime 多 EP | 上游 README 明确 Owned pipeline 仍有 PIT 聚合差异和 ≤5% 目标，不应当作已完成指标 | 可作为后续 stage adapter 来源 |
| `sherpa-onnx 1.13.6` | 高层 pipeline 不暴露内部中间结果；独立 embedding API 可用 | Pyannote/Reverb segmentation | FastClustering | 高层无，独立 extractor 有 | CPU/Core ML provider | 工程成熟、模型面广 | 保留快速稳定 backend；不承担完整可观察身份链 |
| `pyannote-rs 0.3.4` | segmentation + embedding + cosine manager | 简化 | cosine threshold | 有 | CPU/Core ML | `speakrs` 同项目 benchmark 显示其在所测 VoxConverse 子集输出/DER 明显不稳定 | 不作为默认 |
| NeMo Python pipeline | 完整研究体系 | MSDD/Sortformer | NME-SC/MSDD | 有 | CUDA 主线 | 官方模型和训练证据充分 | 禁止 Python subprocess；只能在完成明确 Rust/native adapter 后接入 |

## `diarization-native` 架构

### 模块边界

```text
src/lib.rs
  只声明模块和稳定 re-export

src/backend.rs
  DiarizationBackend adapter
  音频读取、JobRuntime、错误归一化、application result

src/engine.rs
  SegmentationAdapter
  EmbeddingAdapter
  ClusteringAdapter
  StagePipelineEngine<S, E, C>
  SpeakrsEngine（融合优化 provider）

src/model.rs
  模型 bundle layout 和 artifact probe

src/types.rs
  EmbeddingSpace、Observation、Profile、IdentityMatch、rich output

src/identity.rs
  cluster centroid、cosine、threshold、ambiguity margin、one-to-one assignment

src/library.rs
  enrollment、追加样本、rename、delete、模型空间校验、样本上限

src/store.rs
  SpeakerProfileStore trait
  atomic JSON store
  in-memory test store

src/error.rs
  typed native/library error -> BackendErrorCode
```

### 两种 engine 路径

```text
StagePipelineEngine
  -> 三个 adapter 可独立替换
  -> 适合新模型、单阶段 benchmark 和 fake test

SpeakrsEngine
  -> 使用上游已经校准的 powerset/overlap-add/masked embedding/PLDA/VBx
  -> 避免在 MeetNote 重写成熟算法并产生 parity 漂移
  -> 仍把 observations、hard cluster 和 segment 转为 MeetNote 类型
```

这种结构没有把成熟的融合 pipeline 强行拆开导致性能和准确率退化，同时也没有把未来所有模型族塞进
单个 `lib.rs` 或 manager。

### 默认模型 bundle

`speakrs-community-1-cpu` 固定到 upstream revision
`5d24ffee75f13fb061fa6d10944a64e2dc1d5e6f`，由 MeetNote model manager 下载并逐文件校验 SHA-256。

```text
segmentation-3.0.onnx
wespeaker-voxceleb-resnet34.onnx
wespeaker-voxceleb-resnet34.onnx.data
plda_lda.npy
plda_tr.npy
plda_mu.npy
plda_psi.npy
plda_mean1.npy
plda_mean2.npy
wespeaker-voxceleb-resnet34.min_num_samples.txt
```

backend 不调用 `speakrs` 自带在线下载器；依赖关闭 `online` feature，模型下载、checksum、安装路径仍由
MeetNote `models` crate 统一管理。

模型 bundle 的权利来源不是单一 Apache-2.0：runtime crate 使用 Apache-2.0，Pyannote segmentation 使用
MIT，WeSpeaker/VoxCeleb 与 community-1 PLDA 资产还需要保留对应数据和 CC-BY-4.0 attribution。catalog
明确标记 `model-notices-required`；正式打包前必须把 upstream 模型卡和 attribution 汇总到应用的第三方
许可页，不能只展示 Rust crate 的 license。

## 个人声纹持久化和自动标注

### 数据模型

```text
SpeakerLibraryDocument schema v1
└── SpeakerProfile[]
    ├── UUID
    ├── display_name / aliases
    ├── EmbeddingSpace
    │   ├── backend/model/revision
    │   ├── dimension/sample rate
    │   └── feature config/normalization
    ├── SpeakerEmbeddingSample[]
    │   ├── vector/quality
    │   ├── project/segment/time source
    │   └── created_at
    ├── match_threshold
    └── created_at/updated_at
```

默认路径为 `data/speakers/library.json`。写入流程是同目录临时文件、`sync_all`、原子 rename；不存在时
只返回空库，不会隐式创建文件。

### 匹配流程

```text
当前音频每个 cluster 的多条 observation
  -> 每条 L2 normalize
  -> 按 quality 加权 centroid
  -> 只筛选完全相同 EmbeddingSpace 的 profile
  -> profile 多样本加权 centroid
  -> cosine score
  -> max(profile threshold, backend default threshold)
  -> ambiguity margin
  -> cluster/profile 一对一贪心分配
```

命中后：

- `speaker_id` 仍保留当前音频的 `speaker-N`；
- `speaker_profile_id` 保存跨项目稳定 UUID；
- `speaker_label` 保存当前可读姓名；
- ASR 对齐结果和 Markdown/UI 优先显示 `speaker_label`，但不会丢失匿名 cluster ID。

未命中时保持匿名，不会自动创建永久 profile。新的个人声纹只能通过显式 enrollment 写入，避免错误匹配
污染全局身份库。

### 隐私和生命周期

- embedding 是可识别个人的敏感数据，不写入日志和模型 catalog。
- 每个 profile 最多保留 32 条高质量样本，避免无上限积累。
- 提供 profile 删除；删除同时移除其持久 embedding。
- 模型 revision、特征或维度改变后默认禁止比较，必须重新 enrollment 或执行显式 migration。
- JSON speaker library 只保存身份资产，不承担项目文档数据库职责。

## 并行和长任务设计

可并行关系：

```text
ASR ------------------------------------------┐
                                             ├── final alignment
segmentation windows -> embedding batches -> clustering -> identity
```

- ASR 与整条 diarization 由 application 并行运行。
- segmentation 的独立窗口可并行推理；完成的窗口按 index 确定性聚合。
- embedding 依赖对应 segmentation mask，但不同窗口/track 可 batch 或并行。
- 全局 AHC/PLDA/VBx 依赖有效 embedding 集合，不能在输入尚未稳定时提前宣称最终 cluster。
- identity matching 依赖 cluster centroid，放在 clustering 后。
- backend 当前在音频读取、融合 inference、identity/output 三个边界报告 JobRuntime 进度，并在每个边界检查取消。
  上游同步 inference 暂不支持中途强制取消，不能伪造为可即时取消。

## 探索结果

默认实现确定为：

```text
Pyannote segmentation-3.0/community-1 style powerset
  + WeSpeaker VoxCeleb ResNet34 embedding
  + AHC initialization
  + matched PLDA transform
  + VBx temporal refinement
  + persistent known-speaker identity resolver
```

选择依据：

1. Rust package 能完整编译，不依赖 Python subprocess。
2. stage 中间结果可观察，能实现声纹 enrollment 和诊断。
3. segmentation、embedding、PLDA、VBx 参数来自同一套已校准 bundle。
4. 上游有同协议端到端 DER 和性能 benchmark，不只是在模型文件层面“可能可运行”。
5. CPU 是默认跨平台路径；Core ML 通过 crate feature 和完整 Core ML bundle 单独启用，不把 macOS 资产强加给其他平台。

Reverb v1 由于非生产许可证不进入默认 catalog。ERes2NetV2、ERes2Net-large 和 CAM++ 保留为后续 adapter
候选，接入前必须同时提供匹配的特征、clustering calibration、profile migration 和 DER benchmark。

## 复现说明

### 无模型的分层验证

```bash
cargo fmt --all -- --check
cargo test -p diarization-native
cargo test -p domain -p application -p diarization-sherpa-onnx
cargo check -p meetnote
cargo check -p diarization-native --features coreml
cargo test --workspace
```

覆盖：

- 模型 bundle 缺失 artifact；
- 原子 JSON roundtrip 和 missing-store 行为；
- enrollment、revision 隔离、追加、删除；
- threshold、ambiguity margin、one-to-one identity；
- fake engine 的 audio -> diarization -> persistent identity -> label 纵向流程；
- 旧 JSON 新 identity 字段的向后兼容。

### 真实模型验收

```bash
MEETNOTE_DIARIZATION_MODEL_DIR="$PWD/models/diarization/speakrs-community-1-cpu" \
MEETNOTE_DIARIZATION_AUDIO="$PWD/docs/example/2p_zh_video.m4a" \
cargo test -p diarization-native --test native_pipeline \
  real_model_produces_valid_segments_and_observations -- --ignored --nocapture
```

真实验收检查：model probe、实际音频解码、非空 speaker segments、合法时间范围、非空 256 维 embedding
observations。DER 仍需带 RTTM ground truth 的 corpus；“成功输出两个 speaker”不等于完成 DER 验证。

本机实际结果（M4，debug test profile，`2p_zh_video.m4a` 约 196.6 秒）：

```text
10/10 model artifact SHA-256 passed
34 diarization segments
3 detected speaker clusters
303 overlap-aware 256-dimensional embedding observations
0 identity matches (empty speaker library)
173.02 seconds wall time in an unoptimized test build
```

文件名中的 `2p` 不是 ground truth；模型输出 3 个 cluster 可能来自第三个实际声源、片头/片尾语音或
speaker over-splitting。在没有 RTTM 标注前不把它改写成“正确应为 2”，也不据此宣称 DER。release 性能
必须用固定 corpus 单独测量，debug wall time 只证明真实路径执行完成。

## 注意事项与补充

- `expected_speakers` 为零会被拒绝。当前 `speakrs` 融合 provider 不暴露精确固定 cluster 数的公开接口，
  因此正数只作为 hint；不能为了满足数字强行拆分或合并 speaker。需要严格固定数量的任务可继续选择
  `diarization-sherpa-onnx`，或为 StagePipelineEngine 接入支持 fixed-K 的 cluster adapter。
- `speakrs` 上游 benchmark 是选择依据，不替代 MeetNote 自己在目标语言、麦克风和重叠比例上的验收。
- Core ML 模式需要额外 `.mlmodelc` 和分批模型文件；CPU bundle 完整不能推导 Core ML bundle 完整。
- ONNX Runtime 与 Core ML 即使 FP32 也可能因 reduction 顺序产生细微差异，阈值需要分别校准。
- profile 匹配阈值不得照搬 speaker verification EER 的最佳阈值，必须用 MeetNote enrollment/test protocol 校准。

## 主要来源

- [Pyannote audio README and 2025 benchmark](https://github.com/pyannote/pyannote-audio/blob/main/README.md)
- [Pyannote 4.0 community-1 / VBx changelog](https://github.com/pyannote/pyannote-audio/blob/main/CHANGELOG.md)
- [Pyannote 3.0 release comparison](https://github.com/pyannote/pyannote-audio/discussions/1471)
- [Speakrs Rust pipeline, API and benchmark](https://github.com/avencera/speakrs)
- [FinDIT diarization and diaric Rust pipeline](https://github.com/findit-studio/diarization)
- [Sherpa-onnx speaker diarization documentation](https://k2-fsa.github.io/sherpa/onnx/speaker-diarization/index.html)
- [Sherpa-onnx speaker identification model list](https://k2-fsa.github.io/sherpa/onnx/speaker-identification/apk-cn.html)
- [3D-Speaker verification and diarization benchmark](https://github.com/modelscope/3D-Speaker)
- [ERes2NetV2 official recipe](https://github.com/modelscope/3D-Speaker/tree/main/egs/voxceleb/sv-eres2netv2)
- [WeSpeaker pretrained models and ONNX artifacts](https://github.com/wenet-e2e/wespeaker/blob/master/docs/pretrained.md)
- [NVIDIA NeMo diarization models](https://docs.nvidia.com/nemo-framework/user-guide/26.02/nemotoolkit/asr/speaker_diarization/models.html)
- [VBx paper](https://arxiv.org/abs/2012.14952)
- [Reverb diarization v1 model card and license](https://huggingface.co/Revai/reverb-diarization-v1)
