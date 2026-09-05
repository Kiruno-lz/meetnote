# 长音频分窗处理与语义子项目拆分研究

## 探索原因

MeetNote 的输入可以是一天的录音、长时间访谈或其他长音频资产，不以“会议”作为唯一内容类型。当前
audio crate 会把整个文件解码为 `Vec<f32>`，ASR 和 diarization backend 又分别从文件读取完整音频。
短音频可以这样处理，但长音频会产生三个实际问题：

- waveform 内存随音频时长增长；
- ASR 和 diarization 重复解码同一个文件；
- 取消和进度只能在较粗的任务阶段更新。

本研究确定一种用于长音频的 offline 分窗处理方式。分窗只改变内部处理方式，不改变最终产品形态：一次
处理仍然产出完整音频的 canonical transcript、diarization 和对齐结果，随后保存为文档 Markdown 与文档
JSON。

## 探索目标

1. 让单个推理窗口不再持有整段长音频，降低长音频的 waveform 内存峰值。
2. 让 ASR 和 diarization 共享解码后的窗口边界和全局时间基准，避免两个 backend 各自完整读取文件。
3. 保留窗口之间的上下文、时间戳和 speaker 一致性，不把窗口结果简单拼接。
4. 明确 VAD、diarization segmentation、embedding、clustering 和语义子项目拆分的职责边界。
5. 保持 Rust-native、backend 可替换、文档为事实来源和最多三层项目树等既有架构约束。

## 探索结果

### 当前范围

本阶段只采用 offline 分窗。模型级连续推理和处理中的中间结果协议不属于当前需求，
不进入实现计划、catalog 设计或 UI 设计。

### 确定的处理主线

```text
音频文件
  -> 有界解码与重采样
  -> 带 overlap 的处理窗口
  -> ASR 窗口推理 -> ASR stitcher
  -> diarization segmentation
  -> speaker embedding observation
  -> 全局 clustering
  -> identity matching
  -> ASR/diarization 对齐
  -> 完整 canonical document
  -> Agent/Skill 语义拆分
```

ASR 和 diarization 可以继续由两个独立 backend 并行处理，但二者应使用同一套全局 sample offset 规则。
窗口是 runtime/application 的技术对象，不是 domain 中的文档或项目节点。

### 方案选择

| 方案 | 适用性 | 主要问题 | 决定 |
|---|---|---|---|
| 完整文件 offline | 短音频、基线、fallback | 长音频持有完整 waveform，两个 backend 重复读取 | 保留 |
| ASR offline 重叠窗口 | 长音频转录 | 需要窗口边界处理、去重和时间拼接 | 采用 |
| 每个窗口独立完整 diarization 后直接拼接 | 仅能得到局部结果 | cluster ID 在窗口之间没有一致性 | 禁止作为正式结果 |
| 分窗 segmentation/embedding + 全局 clustering | 长音频最终 diarization | 需要保留 observation，并新增全局聚类编排 | 采用，作为主方案 |
| 按 VAD 或声学变化自动创建语义子项目 | 不属于声学处理职责 | 会把语义判断硬编码进 MeetNote | 禁止 |

“分窗 segmentation/embedding + 全局 clustering”是当前长音频 diarization 的主方案，但要准确理解为：
先在每个窗口提取局部轨迹和 embedding，再把所有窗口的 observation 放入同一个全局聚类过程。不能把每个
窗口自己的 `speaker-0`、`speaker-1` 直接写入最终文档。

分窗能限制单个 waveform buffer 的大小，不自动保证整个任务的内存恒定。全局 clustering 至少需要保存
或分批读取跨窗口的 embedding observation；需要在一小时和全天音频基准上测量其内存。若 observation
规模超过可接受范围，再设计 job scratch 的临时落盘，不能把临时数据变成文档事实或独立项目数据库。

## 术语和职责边界

| 阶段 | 负责什么 | 不负责什么 |
|---|---|---|
| 技术窗口 | 划定带上下文的音频处理范围 | 不创建文档或语义项目 |
| VAD | 判断帧或区间是否有语音，提供可选预筛选 | 不识别 speaker，不判断语义边界 |
| diarization segmentation | 估计局部 speaker activity、track 和 overlap | 不产生跨窗口全局身份 |
| embedding | 为局部语音轨迹生成固定维度声纹向量 | 不单独决定 speaker 分组 |
| clustering | 合并跨窗口 observation，生成匿名 cluster | 不直接等同于已登记人员 |
| identity matching | 将匿名 cluster 与已登记 profile 匹配 | 不替代 clustering |
| Agent/Skill | 决定标题、标签、总结、语义边界和父子关系 | 不修改原始音频或绕过文档校验 |

## VAD 与 diarization segmentation

### VAD

VAD（Voice Activity Detection，语音活动检测）回答的是“这一帧有没有语音”。常见输出是每帧的 speech
probability 或二值结果，之后合并为 speech interval。它可以作为分窗前的便宜预筛选，减少静音区间的
ASR/embedding 工作量，也可以帮助定位可处理的语音区间。

VAD 不回答“是谁在说话”，也不能表达两个人同时说话。噪声、音乐、键盘声和掌声是否被判定为活动，取决
于模型训练目标和阈值。因此 VAD 区间不是 speaker 区间，也不是语义子项目边界。

### diarization segmentation（文档中的 seg）

这里的 seg 是 diarization segmentation，不是 ASR 的文字 segment，也不是语义拆分。它在一个有限窗口内
估计：

- 语音活动的时间范围；
- 一个或多个局部 speaker track；
- speaker overlap 及其 frame mask；
- 后续 embedding 应从哪些局部轨迹提取。

seg 可以包含类似 VAD 的“无语音/有语音”信息，但比单一 VAD 丰富。Pyannote powerset 风格 segmentation
可以保留“无 speaker、一个 speaker、多个 speaker”的状态；若先压成单一 VAD，会丢失 overlap 信息。

```text
VAD：       [有人说]       [静音]       [有人说]
seg：       [track A]      [静音]       [track B]
             [A+B overlap]
```

当前 catalog 中的 `segmentation-3.0.onnx` 是 diarization segmentation artifact，不是独立 VAD 模型；
当前代码也没有独立 VAD stage。若以后增加 VAD，只能把它作为预处理组件，不能把它写成 speaker
segmentation 或语义拆分器。

## 当前项目的真实状况

### 当前端到端流程

当前代码保留短音频整文件基线，并为长音频启用有界 offline 分窗：

```text
ProcessingService
  -> AudioChunkReader 读取源信息
  -> 时长 < 30 分钟：既有 whole-file run_pipeline
  -> 时长 >= 30 分钟：run_windowed_pipeline
       -> 单次有界解码和连续 16 kHz mono 重采样
       -> 60 秒 valid core + 前后 15 秒 context
       -> 同一个 Arc<AudioWindow> 分发给 ASR 和 diarization
       -> ASR backend 自己完成窗口拼接
       -> native diarization 汇总真实 inference artifacts 后只做一次全局聚类
       -> Speaker Library identity matching
       -> transcript/speaker 对齐
  -> state.json + document.md，并记录 windowing provenance
```

`AsrRequest` 和 `DiarizationRequest` 继续使用 `audio_path`，没有为现有 backend 制造第二套公共请求模型。
窗口是 application/backend session 内部 contract；不支持有界窗口的 backend 在长音频入口明确失败，不会
静默退回整文件读取。短音频仍使用原有路径，因此改动没有扩大到 domain、文档仓储或 UI。

### 当前 backend

| backend | 当前实现 | 与分窗方案的关系 |
|---|---|---|
| `whisper-rs` | 保留 whole-file offline 识别；窗口 session 对每个窗口执行同一模型并按 valid core 去重、重编号、恢复全局时间 | 支持长音频分窗 |
| `asr-sherpa-onnx` | 保留 whole-file offline 识别；Paraformer 窗口 session 使用边界文本重叠去重并恢复全局时间 | 支持长音频分窗；Qwen3 仍取决于其模型 adapter 可用性 |
| `diarization-native` | whole-file 使用 speakrs 完整 pipeline；窗口 session 只做 segmentation/embedding，汇总 observation 后执行一次 recording-wide clustering，再逐窗重建和裁剪 valid core | 支持长音频分窗，是当前正式 diarization 路径 |
| `diarization-sherpa-onnx` | whole-file diarization 与 embedding observation 可用 | 尚未暴露未聚类窗口中间结果，因此长音频入口明确拒绝该组合，绝不拼接局部 cluster label |

`NativeDiarizationEngine` 返回 native 自己的 `NativeDiarizationWindowSession`。只有
`diarization-native/backend.rs` 的 `ApplicationWindowSessionAdapter` 知道 application trait，engine 和
windowed 实现不依赖 application 的 session 接口。

`SpeakerLibraryService` 只负责将最终匿名 cluster 与已有 speaker profile 匹配，不能替代跨窗口 clustering。
不同 embedding model、revision、维度、前端或 normalization 不在同一个 `EmbeddingSpace` 时不能直接混聚。

### 当前 catalog

| 条目 | 状态 | 当前真实能力 |
|---|---|---|
| `paraformer-zh` | `ready` | sherpa-onnx offline Paraformer，包含 `model.int8.onnx` 和 `tokens.txt` |
| `qwen3-asr-0.6b` | `candidate-source` | 官方 sherpa-onnx ONNX bundle 来源，当前 adapter 方向未达到 ready |
| `whisper-large-v3-ggml` | `candidate-source` | whisper-rs GGML 来源，当前 catalog 未标记为 ready |
| `whisper-base-en-ggml` | `candidate-source` | whisper-rs GGML 来源，当前 catalog 未标记为 ready |
| `speakrs-community-1-cpu` | `ready` | diarization-native 的 segmentation、WeSpeaker embedding 和 PLDA/VBx bundle |
| `sherpa-onnx-speaker-diarization-bundle` | `candidate-source` | sherpa-onnx offline segmentation/embedding bundle 来源，尚未作为 ready 模型使用 |
| `pyannote-speaker-diarization-3.1` | `requires-native-adapter` | gated 来源，当前没有可用 candidate backend |

catalog 的 `ready` 只说明模型目录、安装流程和 backend probe 可以进入可用路径。长音频能力还要求 ASR
和 diarization backend 都实现窗口 session；模型可加载、分窗可执行、20 小时资源验收和带标注的精度验收
是四个不同层级，不能相互替代。窗口参数属于 application runtime 配置，不进入 model catalog。

## 确定的分窗架构

### 1. AudioChunkReader

在 `audio` crate 增加有界读取边界：

```text
AudioDecoder
  -> AudioChunk {
       sequence,
       start_sample,
       sample_rate,
       samples,
       is_final
     }
```

要求：

- demux、decoder 和 resampler 只保留当前 chunk 以及必要的跨 chunk 状态；
- 统一输出 16 kHz mono，同时保留原始 source metadata；
- 时间全部由 sample index 计算，不用浮点累计时间；
- chunk 之间不能丢样本、重复样本或破坏 resampler 连续性；
- 在 chunk 边界提供取消检查和可定位的 decode error；
- 保留现有 `read_audio_mono_16k`，作为短音频 baseline 和兼容入口。

### 2. ProcessingWindowPlanner

planner 根据模型需要把连续 chunk 组合为带上下文的窗口。每个窗口至少需要：

```text
AudioWindow {
  audio_samples,
  global_start_sample,
  global_end_sample,
  valid_core_start_sample,
  valid_core_end_sample
}
```

overlap 用于减少窗口边界的信息损失，但只有 valid core 应作为最终结果的主要归属区间。planner 只负责
技术窗口，不创建文档节点，也不决定语义边界。

当前默认值是 60 秒 valid core、前后各 15 秒 context；分窗激活阈值是 30 分钟。所有边界以 sample index
计算。20 小时 planner 测试生成 1200 个连续 valid core，验证从 sample 0 到末尾恰好覆盖一次，单窗口
waveform 上限为 90 秒；它证明边界和缓冲上限，不证明 20 小时模型质量。

### 3. ASR window stitcher

ASR 先对每个窗口使用现有 offline 模型族推理，再将窗口内时间戳加上 `global_start_sample` 对应的全局
offset。stitcher 负责：

1. 处理窗口 overlap 中的重复 segment；
2. 保证最终 segment 按全局时间单调排列；
3. 处理窗口边界的重复、漏词和冲突文本；
4. 保留模型、窗口和拼接 provenance；
5. 只输出完整 transcript，不把窗口结果写成独立文档。

Whisper 和 sherpa offline ASR 的拼接规则可能不同，不能把去重算法硬编码到通用 backend manager。

### 4. Diarization window extraction

每个窗口的 diarization 只产生真实模型中间结果：

```text
窗口音频
  -> segmentation
  -> local speaker tracks / overlap masks
  -> embeddings
  -> InferenceArtifacts {
       segmentation,
       embeddings,
       chunk layout
     }
```

窗口阶段不执行匿名 speaker clustering，也不生成可写入文档的 `speaker_id`。每个 artifact 通过 speakrs
正式的 clean-mask/filtering 规则导出 `EmbeddingObservations` 供全局聚类；原始 artifact 保留到全局 cluster
形成后，用于逐窗口 constrained assignment 和 reconstruction。这样不伪造 tensor layout，也不会把
`InferenceArtifacts` 滥用为无时序 embedding 容器。

### 5. Global clustering

所有窗口 observation 进入同一个 clustering 阶段：

```text
all window observations
  -> validate same EmbeddingSpace
  -> cluster_embedding_observations
  -> stable anonymous cluster IDs
  -> 每个窗口用全局 centroid 重建
  -> valid-core 裁剪和 boundary reconciliation
  -> identity matching
  -> normalized DiarizationResult
```

vendored speakrs 的职责接口明确拆开为：

```text
run_inference(audio) -> InferenceArtifacts
cluster_inference_artifacts(artifacts) -> DiarizationResult
cluster_embedding_observations(observations) -> EmbeddingClustering
finish_post_inference_with_clustering(artifacts, clustering) -> DiarizationResult
```

`windowed.rs` 只调用第一、第三和第四个能力。`InferenceArtifacts::from_arrays` 已删除；不再先做窗口局部
AHC/PLDA/VBx，再为整段录音重复做第二次 clustering。全局 centroid 仍使用模型 bundle 内配套的
AHC/PLDA/VBx 参数，窗口重建使用真实 segmentation、embedding 和 layout。

当前 `speakrs-community-1-cpu` 的默认组合是 Pyannote community-1 风格 segmentation、WeSpeaker VoxCeleb
ResNet34 embedding 和 AHC/PLDA/VBx。其参数和 embedding space 是配套的，不能拿 3D-Speaker、ECAPA 或
其他 revision 的向量直接复用 PLDA/阈值。

### 6. Speaker Library identity matching

Speaker Library 只在当前录音的匿名 speaker 全局聚类完成后参与：

```text
recording-wide anonymous clusters
  -> 计算 cluster centroid、最大分布距离和高置信 observation 时间范围
  -> 与兼容 EmbeddingSpace 的 profile 比较
  -> 中心相似度 + 分布下限 + cluster/合并后离散度 + 第一/第二候选差距
  -> 已有 profile match 或 unknown profile candidate
```

允许同一录音的多个匿名 cluster 指向同一个 profile，以修复 diarization 对同一人的过拆分，但只有同时满足
以下条件才接受第二个 cluster：

- 两个 cluster 的高置信 observation 重叠不超过 100 ms；
- 每个 cluster 单独超过 profile 绝对阈值、分布下限和竞争差距；
- 两个 cluster centroid 相似度不低于跨 cluster 下限；
- profile 样本和所有待合并 cluster 的最大余弦距离不超过合并后离散度上限。

未匹配 cluster 只有在 observation 数、累计语音时长、平均质量和离散度均达标时才生成
`SpeakerProfileCandidate`。候选只引用最接近 cluster centroid 且质量高的 observation，不自动写入 library。
用户确认 enrollment 时，每个 `SpeakerEmbeddingSample` 必须保留 `(project_id, source_segment_id, time range)`
逻辑引用，或 `audio_path` / 独立 `backup_path`；可附带源音频 SHA-256。向已有 profile 追加样本时必须再次
通过绝对相似度和合并后离散度检查，防止长期污染。

`SpeakerLibraryService::merge_profiles` 在 embedding space、centroid 阈值和合并后分布均合格时合并，并
保留所有来源证据、别名及最优样本；`split_profile` 按 sample id 移动证据，要求两边非空且各自形成紧致
分布。schema 保持版本 1，新证据字段使用向后兼容的可选字段，不建立双写或旧/新两套 service。已有
schema-1 profile 即使没有新增字段仍可参与 identity matching；新 enrollment/append 以及 merge/split
结果必须满足可恢复音频证据约束。

## 语义子项目拆分边界

MeetNote 不根据 VAD、声学变化、关键词或 speaker 切换自动创建语义子项目。正式流程是：

```text
完整 ASR + diarization + alignment
  -> 当前文档的 Markdown 和 JSON
  -> Assistant 或外部 Agent 读取 Skill
  -> Agent 决定语义边界、标题、标签、总结和父子关系
  -> MeetNote document service 解析、校验并保存
  -> domain 从文档派生最多三层项目树
```

文档 Markdown 与同目录 JSON 是事实来源；项目树和索引必须可以从目标目录重新扫描生成。窗口、VAD 区间、
局部 track 和 embedding observation 都不能成为独立的项目树事实来源。Assistant 与外部 Agent 使用相同的
document service、权限、版本校验和写入规则。

## 对现有 crate 的影响

| crate | 变更方向 | 当前状态 |
|---|---|---|
| `audio` | packet reader、stateful resampler、bounded chunk iterator、sample-based window planner | 已实现并有 packet/连续重采样/20 小时边界测试 |
| `application` | 长短音频选择、单次解码双 backend 分发、取消、进度、identity matching | 已实现；backend 特有 stitcher 不进入 application |
| `contracts` | Speaker Library 分布指标、unknown candidate、样本音频证据 | 已实现为 additive schema-1 字段；没有新增窗口领域模型 |
| `asr-whisper-rs` | offline window session、全局 offset、valid-core 去重 | 已实现 |
| `asr-sherpa-onnx` | Paraformer offline window session、边界文字重叠去重 | 已实现 |
| `diarization-native` | native session contract、真实 inference artifact 汇总、单次全局 clustering、全局重建 | 已实现 |
| `diarization-sherpa-onnx` | 必须暴露未聚类窗口中间结果后才能用于长音频 | 未实现；当前明确拒绝，不增加错误兼容层 |
| `third_party/speakrs` | 正式拆分 inference-artifact clustering 与 observation clustering API | 已实现；伪造 `from_arrays` 已删除 |
| `runtime` | 复用 `JobContext` 做窗口取消和按 valid-core sample 进度 | 已实现，不保存文档事实 |
| `domain` | 继续接收最终 normalized result，不放窗口算法 | 不需要修改职责 |
| `persistence` | 继续保存最终 Markdown/JSON；临时窗口数据不进入文档事实 | 不需要修改职责 |

新增 backend 仍实现既有 backend contract；若声明 `supports_windowed_audio`，还必须提供自己的窗口 session。
application 负责窗口调度，ASR 拼接和 diarization 聚合留在各自 backend 内；GPUI View 不负责音频 IO、
推理或聚类。

## 复现说明

当前事实可从以下实现和测试位置核对：

- 有界解码与连续重采样：`crates/audio/src/stream.rs`；
- sample-based planner 与 20 小时覆盖：`crates/audio/src/window.rs`；
- 压缩音频 whole-file/streaming 样本等价：`crates/audio/tests/streaming_real.rs`；
- 长音频选择、单次解码、共享窗口、取消和进度：`crates/application/src/long_audio.rs`、`workspace.rs`；
- Whisper/Paraformer backend-specific stitcher：`crates/asr-whisper-rs/src/lib.rs`、`crates/asr-sherpa-onnx/src/lib.rs`；
- native session 适配边界：`crates/diarization-native/src/engine.rs`、`backend.rs`；
- 真实 inference、全局 clustering 和逐窗重建：`crates/diarization-native/src/windowed.rs`、
  `third_party/speakrs/src/pipeline.rs`；
- identity 组合判定、候选、merge/split 与合成测试数据：`crates/application/src/speaker_identity.rs`；
- 当前模型来源、状态和 artifact：`resources/models/catalog.json`；
- 真实 ASR + native diarization 分窗验收：`apps/meetnote/tests/long_audio_real.rs`，为显式 `#[ignore]`
  的受控测试，必须提供本机模型和真实音频路径。

设计依据：

- [Symphonia crate documentation](https://docs.rs/symphonia/latest/symphonia/)：FormatReader 按 packet 读取媒体数据；
- [whisper-rs FullParams](https://docs.rs/whisper-rs/0.16.0/whisper_rs/struct.FullParams.html)：Whisper 的完整推理参数和窗口相关参数；
- [speakrs source](https://github.com/avencera/speakrs)：完整 diarization pipeline 和中间结果边界；
- [pyannote SpeakerDiarization pipeline](https://github.com/pyannote/pyannote-audio/blob/main/src/pyannote/audio/pipelines/speaker_diarization.py)：segmentation、embedding 和 clustering 的阶段关系。

## 验收状态和后续顺序

已完成：

1. `AudioChunkReader` 的 packet 边界、全局 sample offset、EOF、取消和连续 resampler 单元测试。
2. 20 小时、60 秒 core 的 1200 窗口规划覆盖测试；valid core 无洞、无重复，窗口 waveform 有上限。
3. 同一 `Arc<AudioWindow>` 向 ASR/diarization 双路分发、取消、进度和 provenance 测试。
4. Paraformer 与 Whisper 窗口 session；native diarization 真实 artifacts 的单次全局聚类接口。
5. Speaker Library 的同 profile 多 cluster、重叠拒绝、宽分布拒绝、竞争差距拒绝、unknown candidate、
   样本污染拒绝、merge/split 和音频证据测试。
6. 约 3 分钟真实中文音频的当前对照结果：Paraformer whole-file 为 1 segment / 922 字符，windowed 为
   4 segments / 924 字符；native diarization whole-file 与 windowed 均为 34 segments / 303 observations /
   3 个匿名 speaker。端到端 alignment 非空。该音频没有 transcript ground truth 或 RTTM，因此这些数字
   证明执行完整性和基线一致性，不证明 WER/CER/DER 或真实 speaker 数正确。

仍需完成后才能声称“20 小时完整质量验收”：

1. 使用真实 20 小时代表性音频执行完整模型链路，记录耗时、峰值 RSS、observation 数和 scratch 占用。
2. 用带人工 transcript/RTTM 和已知 speaker identity 的长音频测量 WER/CER、DER、speaker count、
   跨窗口 identity 一致性和误合并率。
3. 执行中途取消和故障注入，确认响应延迟、失败可见性以及重新处理不会覆盖原始音频或原始结果。
4. 若 20 小时 observation 常驻内存超过预算，再把 `WindowInference` 与 observations 原子写入 job scratch；
   scratch 必须可丢弃、可重建，不能成为文档事实来源。
5. sherpa diarization 只有在提供真实 segmentation/embedding window contract 和同等级验收后才可声明长音频支持。

## 注意事项与补充

- 技术窗口、VAD 区间、ASR segment、diarization segment、embedding observation 和文档子项目不能共用同一个 `DocumentState` 字段。
- `speaker-0` 只是某次 clustering 结果中的匿名标识；不能把窗口内的同名 label 当作跨窗口同一 speaker。
- 全局 clustering 需要同一 `EmbeddingSpace` 和兼容的校准参数；不同模型族、revision、前端或 normalization 不能直接混聚。
- 分窗中间数据可以服务于当前 job，但不能替代 Markdown/JSON 事实来源，也不能引入 SQLite 或全局 project store。
- 不提交模型权重、音频样本、secret、`.env`、构建产物或本机缓存。
- 当前实现仍属于 offline 分窗，不引入实时流式识别协议；任何 realtime/online 方案都不作为长音频精度问题的替代品。
