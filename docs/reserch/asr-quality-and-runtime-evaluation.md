# 本地转录质量、运行稳定性与后端选型

## 探索原因

MeetNote 的默认中文路径使用 Paraformer，另有 Whisper 与 Qwen3-ASR adapter，但已有真实模型测试主要证明能加载并产生结果。没有同一硬件、官方参考文本和固定评分的比较，无法判断模型选择、线程、执行设备、量化或分窗是否改善了准确度和速度。

本轮发现五分钟输入的质量退化、Whisper 取消回调崩溃，以及英文子词拼接错误。这些问题分别影响 backend 输入边界、FFI 生命周期和 transcript 事实完整性；如果只比较模型宣传指标，可能把运行错误、格式错误或 CPU 回退当作模型能力差异。

## 探索目标

- 用固定官方音频与 gold 分别测中文 CER、英文 WER、执行失败、wall/RTF 和进程峰值 RSS。
- 比较 Paraformer、SenseVoice、Whisper large/base/Turbo 与 Qwen3-ASR，覆盖 CPU/Metal、线程、量化和模型驻留方式。
- 保留短音频 baseline，验证长输入的推理长度、窗口上下文和文本拼接，使用另一组未参与定位的句子检查改进。
- 在现有 Rust backend 中完成小范围接入；MLX 先以隔离 benchmark 验证，不增加产品 runtime。
- 区分 ASR 文本、时间戳、diarization、身份、语义与发布质量，不用单一总分代替各层验收。

## 探索结果

### 数据与口径

硬件为 Apple M2 Pro、10 核 CPU、32 GiB 统一内存。Rust benchmark 使用 release 构建，每种配置独立进程、顺序运行、重复两次。模型与音频均记录实际 SHA-256；大文件和推理输出保留在忽略目录，汇总与复现方法进入仓库。

- [FLEURS](https://huggingface.co/datasets/google/fleurs/tree/70bb2e84b976b7e960aa89f1c648e09c59f894dd)：中文与英文 test 各取官方 TSV 顺序中的前 24 个不同句子 ID，保留官方参考文本；另取后续 24 个中文句子检查窗口改进。句子 ID 分离不证明 speaker 分离。
- 构造输入按固定顺序拼接原音频，每段后插入 400 ms 静音；噪声使用 seed `20261025`，按整段 RMS 加入全局 20 dB 高斯噪声。不同原片段音量不同，不能称为每句话都具有 20 dB SNR。这些输入检查边界与鲁棒性，不能代表自然长会议。
- [AMI](https://groups.inf.ed.ac.uk/ami/corpus/)：ES2002a Mix-Headset 与官方 manual 1.6.2 词标注；每位说话人最多两个原始片段，3–20 秒、至少六个词、前后各 100 ms、无其他说话人的词级重叠，共五个片段、37.29 秒。它是使用官方边界裁剪的 B1 会话样本，不测试自动 VAD 或重叠识别。[官方说明](https://groups.inf.ed.ac.uk/ami/corpus/transcription.shtml)区分人工校对文本与自动 forced-alignment 词时间；当前 manifest 的 `wordTiming` 保留 95 个词的 ID、原文与相对 sample 时间，明确不标成人工精确边界。投影使用十进制源时间与 sample 取整，另用有理数计算核对；原五个音频 hash、参考文本、词 ID 和裁剪偏移完全一致，见[参考回执](../quality/ami-word-reference.json)。现有 CER/WER 不使用这些词时间，不能据此宣称定位已达标。
- 中文按 NFKC/casefold 后的非空白文字计算 CER；英文标点分词、去除撇号后计算原样 WER，并单独给出固定 `whisper-normalizer 0.1.15` 的 normalized WER。两者均使用 RapidFuzz 精确 Levenshtein，不使用近似距离。原样错误率保留在报告中。

结果见[本机模型比较](../quality/model-comparison.md)与同目录机器 receipt。原生片段、构造片段、AMI 和第二组中文分别展示，不把重复或构造数据当作独立会议样本。

### 已证实的实现问题

| 问题 | 证据与处理 |
|---|---|
| Whisper 取消回调的 userdata 类型不一致 | 固定 `whisper-rs 0.16.0` 的 safe wrapper 将双层 Box 的地址交给按具体 closure 类型读取的 trampoline。五组 CPU/Metal 对照发生 SIGSEGV，macOS 栈落在 trampoline → `JobContext::is_cancelled`。adapter 改用类型明确的 C 回调和自有 `Box<JobContext>`，由参数对象持有到同步 `full` 返回后释放；不改 vendor，不泄漏 closure |
| Core ML 请求静默回退 CPU | 本机真实运行日志出现 `CoreML ... Fallback to cpu!`。当前 sherpa ASR/diarization adapter 明确拒绝不能保证执行的 Core ML/Metal 请求，返回 `DeviceUnavailable`；不把 CPU wall time 记录为 Core ML 加速 |
| SentencePiece 子词被插空格 | 原合并将 `ent`、`om`、`ologist`、`s` 拼成多个词。保留 SentencePiece 显式词边界和 Paraformer `@@` 连续标记，在词内直接连接，连续标记优先于 750 ms 停顿和标点分段，保留 `entomologists` 与 `U.S.`；单独的词边界标记只影响下一个词，不产生空段。正常整词行为保持独立，新增回归先失败再修复 |
| 单次推理长度与应用长音频路由混淆 | 原 application 在 30 分钟才进入长音频路径，但同一批中文拼成 295.42 秒后，Paraformer 整文件 CER 达 94.88%，短片段为 11.76%；加线程保持相同错误率。ASR backend 的模型推理粒度必须独立于 Audio Host 的长录音路由 |
| 长全零输入产生虚构转写 | 同一 31 秒全零 WAV，冻结 baseline 返回空识别错误，原分窗实现却生成四段中文。分窗只跳过严格全零的 valid core，不用能量阈值，保留非零弱声与上下文所有权；全段无结果统一返回 `OutputValidationFailed`。真实模型回归先失败再通过，这不是通用 VAD |
| 评测缺轮仍被视为完整 | 原评分器从已有行推断轮数；删除完整一轮后，门槛与汇总仍接受结果。现按 config 的 repeats 校验 case×repeat 精确覆盖，拒绝重复、缺项与越界；新预测绑定 SHA-256，重跑先作废旧成功回执，最终回执原子替换 |
| 中断遗留推理进程 | 原 wrapper 只处理超时，Ctrl-C 可留下独立 session 的推理进程。现 SIGINT/SIGTERM/超时统一结束整个进程组并 wait 回收，保留部分输出和取消/超时原因；损坏的最后一行不会阻止失败回执生成 |
| sherpa 默认线程隐式降为一线程 | 原 `Default` 产生零，运行时通过 `max(1)` 使用一线程。相同权重与输入下四线程改善 wall time；默认值改为可用逻辑 CPU 数与四的较小值，保持明确上限 |

窗口扫描比较了 Paraformer 的 6+2/8+2/10+2.5 秒（核心+单侧上下文），开发构造 clean 的 CER 分别为 16.88%/14.81%/15.58%，noise 为 21.35%/16.34%/18.19%。30 秒推理上限的初稿仍有 36.17% clean CER，未通过预先设定的 20% 门槛，不能作为最终修复。最终采用 8+2 秒；SenseVoice 采用 20+5 秒。30 秒以内 whole-file 输入保留 baseline；应用 60+15 秒窗口内部仍按模型粒度处理。子窗口从父 valid core 开始分配，而不是从重复 context 开始分配，避免嵌套窗口多解码与跨父窗口上下文顺序倒退。

原始目录保留的“五次失败”均为修复前 Whisper 回调 SIGSEGV（`runner exit -11`），不是五个仍未解决的独立问题。五份 macOS faulting-thread 栈与固定 binding 源码共同定位 userdata 布局错误。既有修复后完整/扩展对照共完成 370 次调用；此次 review 又用当前 release executable 对每组最先失败的原始病例各执行两次，10 次全部成功。下面的定点复测只证明这五条崩溃路径，不增加独立质量样本数。

| 原始失败配置 | 既有修复后调用 | 当前定点复测 |
|---|---:|---:|
| base.en CPU | 58 | 2/2 成功 |
| base.en Metal | 58 | 2/2 成功 |
| large-v3 CPU | 26 | 2/2 成功 |
| large-v3 Metal | 114 | 2/2 成功 |
| Turbo Q5 Metal | 114 | 2/2 成功 |

首尾各 32 秒全零、中间保留原始 FLEURS 中文音频的输入，整文件与 application-window 路径各执行两次，所有输出段均与中间语音范围相交，没有完全落在静音区的虚构文本。Paraformer 无可靠词时间时仍输出近似窗口范围，不把此检查当作词时间精度证明。复测 SHA、覆盖与质量门结果见[review 回归回执](../quality/review-regression.json)。

短音频 ASR→diarization 的顺序调度继续保留。此次回调栈解释本轮独立 Whisper 崩溃；它不能反向证明历史所有 native runtime 故障均只有同一个原因，也不构成恢复无界并行的依据。

### 模型、后端与执行载体的选择

| 路线与官方来源 | 本轮定位 |
|---|---|
| [sherpa-onnx SenseVoice](https://k2-fsa.github.io/sherpa/onnx/sense-voice/pretrained.html) | 复用已有 Rust binding 与模型管理，新增模型族 adapter，测中英文与窗口链路。模型权重采用其上游声明的模型许可，不能把 toolkit 的许可证当作权重许可 |
| [whisper.cpp](https://github.com/ggml-org/whisper.cpp) large-v3 / Turbo Q5 | 使用现有 `whisper-rs` 与 Metal；修复 FFI 后再评分。Turbo 提供更小的可安装候选，保留 Whisper 段时间戳路径；不把量化收益当作无损保证 |
| [Qwen3-ASR](https://github.com/QwenLM/Qwen3-ASR) + [MLX 实现](https://github.com/moona3k/mlx-qwen3-asr) | 测 0.6B 8-bit 与 1.7B 4-bit。独立 venv、固定权重 revision、离线加载；不启动 diarization、forced aligner 或服务。驻留 session 的 wall 与逐次加载后端分别标注，暂不作为桌面 provider |
| [Rust/Candle Qwen3-ASR](https://github.com/alan890104/qwen3-asr-rs/tree/c5ef09646af6278d2ba8b8ceaf543ffb32d1a5dc) | 有 Metal 与量化接口，但仍需独立验证输出一致性、取消、词时间、模型 checksum 与长音频。此次只核对固定源码，不以 README 的其他机器速度替代本机测量 |
| [Faster-Whisper / CTranslate2](https://opennmt.net/CTranslate2/hardware_support.html) | 官方预编译 GPU 路径列为 NVIDIA；在本机不能直接沿用 CUDA 排名。CPU INT8 可作为跨平台候选，本轮未运行，不作性能结论 |
| [FluidAudio](https://github.com/FluidInference/FluidAudio)、[WhisperKit](https://github.com/argmaxinc/argmax-oss-swift)、[SpeechAnalyzer](https://developer.apple.com/documentation/speech/speechanalyzer) | 可作为 Apple 平台执行载体候选，需另验系统版本、原生桥接、模型生命周期、取消与资源。此次没有测其速度；不据此更换 Rust/GPUI 或 Host contract |
| [Parakeet TDT v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3) | 官方语言范围是欧洲语言；可以补英文高吞吐路线，不能承担当前中文默认能力，本轮未测 |

### 当前机器的选择结论

相同 24 个中文/24 个英文原生句子、相同五个 AMI 裁剪、各执行两次；下表中文为 CER，英文为单独的 normalized WER，完整原样 WER 保留在自动报告中。

| 配置 | 中文原生 CER | 英文原生 normalized WER | AMI normalized WER | 原生 RTF（中/英） |
|---|---:|---:|---:|---:|
| Paraformer CPU 4 | 11.76% | 未测 | 未测 | 约 0.055 / — |
| SenseVoice CPU 4 | 10.35% | 7.89% | 20.00% | 约 0.072 / 0.079 |
| Whisper base.en Metal | 不支持 | 8.60% | 24.00% | — / 0.024 |
| Whisper large-v3 Metal | 10.13% | 3.68% | 12.00% | 0.248 / 0.268 |
| Whisper Turbo Q5 Metal | 11.00% | 5.26% | 15.00% | 0.115 / 0.134 |
| Qwen3-ASR 0.6B 8-bit MLX | 9.48% | 5.26% | 12.00% | 0.025 / 0.030 |
| Qwen3-ASR 1.7B 4-bit MLX | 9.04% | 2.81% | 13.00% | 0.035 / 0.045 |

原生 adapter 的 RTF 含每次模型加载，MLX 为驻留推理；此表用于分别判断质量和部署成本，不作跨加载策略的端到端倍速排名。AMI 仅五个裁剪，0.6B 此项优于 1.7B，说明模型大小不构成每种内容的质量保证。

- 现有中文轻量路径：保留 Paraformer，先使用本轮修复的推理粒度；不自动更换用户已选模型。开发/第二组构造 clean CER 为 14.81%/15.14%，应用窗口为 14.81%/14.62%；原生短片段两组共 96 次调用与旧 transcript（含时间）完全一致。
- 当前 Rust 多语言安装路线：优先试 Whisper Turbo Q5 Metal，权重约 574 MB、该配置峰值约 958 MiB；large-v3 在原生英文与 AMI 上更准，但加载和推理显著更慢。构造英文长输入的 Turbo/large 结果仍有高遗漏，不能以短样本分数承诺所有长录音效果。
- CPU 多语言备选：SenseVoice 复用现有 ONNX 生命周期，原生中文比 Paraformer 此小集略准；它在本机的调用速度没有超过修复后的 Paraformer，需要考虑上游权重许可。
- 更值得继续验证的 Apple 路线：Qwen MLX 1.7B 的这组中英文原生文本最好，0.6B 更轻、更快。实验权重固定为 [0.6B 8-bit](https://huggingface.co/moona3k/mlx-qwen3-asr-0.6b-8bit/tree/83ce2a8ef9a382d2ba2209aab383bc51fff366f5) 与 [1.7B 4-bit](https://huggingface.co/moona3k/mlx-qwen3-asr-1.7b-4bit/tree/b2315a2537123353af98e37fc93eae078f1b7cf3)。正式 provider 前仍需可靠词时间、取消/进程退出、驻留资源上限与完整会议验证；当前只交付隔离测试入口。

### 优化次序与边界

先修复崩溃、词拼接和设备标识，再依据同一输入的质量/速度选择模型。推理窗口属于 backend 技术状态，不能生成文档树节点或语义边界。输入 sample 连续性、context 与 valid core 分开验证；文本无可靠时间戳时不能承诺精确 speaker 归属。

模型驻留是后续可测方向：现有短音频每次加载 context，长音频 session 已复用一次模型。若增加跨任务缓存，必须由明确 owner 限制数量与内存，按 artifact/revision/device 失效，处理取消和工作区切换；当前结果不授权全局无限缓存或另建 worker 调度器。

[组件资源回执](../quality/processing-profile.json)记录同一段 AMI 前 180 秒的独立 release CPU 分离对照：native 两次 166.715/166.256 秒，峰值 RSS 292 MiB；sherpa 两次 62.228/62.000 秒，733 MiB。未指定预期人数，输出分别为 3 与 21 位 speaker；该计数不是 DER，不能据速度更换默认分离器。同一音频的 Whisper Turbo Metal 两次 ASR 为 17.587/10.196 秒（含各次模型加载；两轮差异保留，未进一步拆分加载、内核预热与解码成本）。上述 native 数值记录优化前执行；本轮 CPU embedding 对照已将同一 180 秒样本的分离中位耗时降至 35.233 秒。用新分离中位值与已有 ASR 两轮均值估算串行 ASR+DIA，ASR 约占 28.3%；ASR 时间减半预计再减少约 14.1% 总时间。该估算不含对齐/保存且不等于整链实测。完整处理速度应优先测量分离的 segmentation/embedding 和可验证的硬件执行路径，而不是只替换更大的 ASR 模型。

native diarization 仍保留全录音 observation 和 AHC 二次距离数组。ASR 分窗或更快模型不解决这一内存墙；小时级与 20 小时资源需要独立 B2/B3 验收。

### CPU embedding 的线程选择与空转对照

embedding 是固定 bundle 的主要耗时点。先前一/四线程的 180 秒对照中位耗时为 164.91/47.52 秒，详见[初次 CPU 回执](../quality/diarization-cpu-comparison.json)。四线程只是初始扫描点，当前默认依据更宽区间重新确定。

同一 M2 Pro、10 个可用逻辑 CPU 上，对相同 AMI 前 30 秒使用生产 adapter、固定权重与默认空转设置，每种线程数独立进程执行两轮。构建与推理依次运行，首轮保留，不剔除慢样本；CPU 列为整个双轮进程的 user+sys 时间，RSS 为进程高水位。12–20 线程是超额分配对照，不表示拥有对应核心数的设备。所有行均通过时间线与 embedding 的 baseline 等价检查。

| embedding 线程 | 30 秒音频中位耗时（秒） | 双轮总 CPU 时间（秒） | 进程峰值 MiB |
|---:|---:|---:|---:|
| 1 | 23.48 | 56.71 | 293.5 |
| 2 | 12.52 | 57.99 | 273.5 |
| 4 | 6.86 | 60.37 | 295.1 |
| 6 | 5.22 | 65.13 | 292.7 |
| 8 | 9.70 | 151.25 | 296.5 |
| 10 | 10.95 | 195.04 | 286.4 |
| 12 | 13.19 | 241.97 | 282.6 |
| 16 | 18.08 | 345.31 | 294.0 |
| 20 | 20.19 | 371.65 | 274.8 |

六线程比四线程的中位耗时下降约 24%，双轮 CPU 时间增加约 8%；更大线程数出现明显调度/等待开销。选择规则预设为：在最快合格结果的 15% 耗时区间内，优先 CPU 时间和线程开销更低者。当前机器选择六线程。

[可用并行度 API](https://doc.rust-lang.org/std/thread/fn.available_parallelism.html)提供跨平台的资源估计。默认采用简单整数规则：`N <= 2` 使用 `N`；其余使用 `N - ceil(N/3)`；查询失败使用一线程。例如 4/8/10/16/32 个可用线程分别使用 2/5/6/10/21 个，不写死四或六线程。纯预算测试覆盖 0–1024、极大整数、单调性、上界与余量，并编入 workspace suite。该 API 不识别 P/E 核能力、NUMA 或当前负载；比例是保守默认，不声称其他硬件也获得本机收益。没有运行时自动调参、全局模型缓存或第二套调度器。

完整 180 秒输入各两轮复核：旧四线程中位数 47.522 秒，设备规则所得六线程为 35.233 秒（34.674/35.792），再下降 25.86%；RSS 为 298.02 MiB，低于旧四线程的 301.47 MiB。每轮仍为 45 段、3 个匿名 speaker、303 个 observation；时间线等价，embedding 最大绝对差 4.32e-7、最小 cosine 0.9999999999994189。它证明固定输入与 baseline 一致，不替代 DER。

下一切口测试 [ONNX Runtime 的空转控制](https://onnxruntime.ai/docs/performance/tune-performance/threading.html)。在固定六线程下，事先要求 CPU 时间至少下降 10%、耗时增加不超过 5%、RSS 不超过 1.1 倍，并保持相同输出。短样本中双阶段关闭空转节省 14.6% CPU，但完整输入未通过延迟门；因此继续拆分并测试 1 ms 有界空转。运行库中存在对应配置键，三个方案均经真实推理核对，结果如下：

| 180 秒输入方案 | 中位耗时（秒） | 双轮 CPU 时间（秒） | 相对默认 CPU 时间 | 结论 |
|---|---:|---:|---:|---|
| 默认空转，六线程 | 35.23 | 431.15 | — | 保留 |
| segmentation + embedding 均关闭空转 | 37.14 | 371.24 | -13.9% | 耗时增加 5.4%，拒绝 |
| 仅 embedding 关闭空转 | 35.84 | 397.39 | -7.8% | CPU 收益不足 10%，拒绝 |
| 仅 embedding 空转限为 1 ms | 36.09 | 440.90 | +2.3% | CPU 开销增加，拒绝 |

三项完整对照的时间线和向量均与六线程默认完全相同；不因输出一致而忽略运行退化。空转相关生产改动已回退，保留失败方案与门禁记录。全部宽区间、资源对照、模型/输入/runner/预测 hash 见[线程扫描回执](../quality/diarization-thread-sweep.json)，本轮共 18 个新配置运行、36 次调用。没有测量能耗。

同时核对的 [speakrs 开发主线](https://github.com/avencera/speakrs/tree/beb30734c9067c1ed59af3eb8ec8baa8fb17f94a)引入 safetensors 原生 CPU 资产，manifest 标为 0.7.0、当时最新可见 tag 为 v0.5.0；它不属于本轮固定 ONNX bundle，未运行新引擎，不用其其他硬件成绩替代本机验证。当前更改保留 inter-op、图、权重、分窗与聚类参数。

### Whisper 线程范围与并发复核

Whisper 默认固定四线程与 native diarization 的预算是独立路径。上游 [whisper-bench](https://raw.githubusercontent.com/ggml-org/whisper.cpp/master/examples/bench/README.md) 只测随机输入的 encoder，不能代替真实解码。此轮用生产 adapter、固定 base.en、六个已知 FLEURS 英语输入共 65.68 秒，CPU/Metal 各测 1/2/4/6/8/10/12/16 线程，每个输入两轮，再反转设备和线程顺序；模型逐次加载，保留首轮异常，不剔除慢调用。

下表中位数汇总两个执行顺序；CPU 12/16 行均触发每配置 120 秒预算，部分预测不能算完整精度通过，RSS 缺失不记零。其余完成调用的归一化文本均与四线程控制一致，见[线程评测回执](../quality/whisper-thread-evaluation.json)。

| 每 session 线程 | CPU 中位 ms | Metal 中位 ms |
|---:|---:|---:|
| 1 | 796.40 | 247.15 |
| 2 | 535.69 | 243.19 |
| 4 | 399.23 | 238.19 |
| 6 | 347.79 | 239.46 |
| 8 | 389.30 | 243.23 |
| 10 | 452.17 | 237.16 |
| 12 | 两次超时 | 236.52 |
| 16 | 两次超时 | 235.07 |

运行前固定的候选为 `N <= 2` 使用 N，其余 `N - ceil(N/3)`，查询失败一线程；本机 N=10，对应六线程。它通过单任务门：CPU 中位耗时下降 12.89%，Metal 增加 0.54%，两设备比值几何平均 0.936，RSS 在 1.2 倍内。独立 review 要求补测 large-v3 与 Turbo Q5 的中英各一句、两轮，CPU/Metal 四组均保留文本、耗时与资源门通过。

但 per-session 默认影响同时运行的任务。进一步用两个独立 native 进程、每个两句英语两轮，按四→六、六→四顺序对照。反向批次总耗时为 3.330/2.300 秒，六线程比四线程增加 44.78%，超过预先定义的 1.1 倍上限；逐调用中位数也退化。正向四线程进程保留一次原因未明的 7.672 秒首调用，不能把两批平均后掩盖反向失败。该诊断共享主机 CPU，但不是同进程 JobRuntime 或长音频并发验收。

结论：拒绝将六线程预算推广为 Whisper 全局默认，保留既有默认与显式覆盖；没有根据单任务最优点增加总资源竞争。48 个 native 进程共完成 403 次调用，另有四个超时配置与缺失调用明确保留。//TODO - 先在现有 JobRuntime 明确多录音的总 CPU/模型 session 预算，再在同进程、不同模型和小时级输入上验证吞吐、取消与 RSS；不能只放大每 session 线程，也不另建一套调度器。其他设备的 P/E 核、当前负载和 NUMA 不能由 `available_parallelism` 的近似值推断。

### AMI 分离参考投影

在更改 AHC 或身份策略前，需要可追溯的 B2 参考。比较自行从 XML 造规则与复用 [BUT/pyannote AMI setup](https://github.com/pyannote/AMI-diarization-setup/tree/67c2d539286e89f68952d5dcf83912bd9f01dfae)，采用后者固定 commit 的 only_words RTTM、原 UEM 和训练分区清单；其规则保留词活动、合并相邻同 speaker 区间，不把该协议等同于人工声学静音标注。

`prepare_ami_diarization.py` 使用已有 pyannote.database loader，输出单个有 hash/sample 来源的参考文件；波形分块比较证明 `ami-first180.wav` 是官方 Mix-Headset 的精确开头，23 个投影区间、4 个参考 speaker 与独立 Decimal 运算逐项一致。测试覆盖重叠/身份、裁剪偏移、PCM 格式与内容、UEM 空隙、来源 pin、输入别名保护及完整 fixture 发布，全部 6 项通过。这个 ES2002a 裁剪明确属于 train，不能称为留出集；参考生成本身不执行模型；独立模型评分见后续基线。完整来源及范围见[参考回执](../quality/ami-diarization-reference.json)。

复现时为 `prepare_ami_diarization.py` 显式提供 `--source-audio`、`--audio`、`--rttm`、`--uem`、`--split-file`、`--start-ms` 与 `--root`。所有参考来源均验证固定 SHA；真实音频/标注留在忽略目录，失败不替换已有参考。单个 crop 必须被原 UEM 全覆盖；不通过拼接或填补 UEM 伪造有效评分范围。

### 独立 diarization 基线

在改变聚类前，以固定的 ES2002a 开头 180 秒、CPU native、自动说话人数、无 Speaker Library 运行生产 adapter 两轮；运行前冻结零 collar 主指标、500 ms 完整 collar 敏感性指标和包含重叠，不按结果改参考或阈值。`report_diarization.py` 复用现有 CPU run 的 hash/失败/重复校验和标准 DER，显式空输出作为漏检，不因缺少 embedding 而假称未执行；原 CPU 对照仍要求完整向量。两个入口分别检查，不削弱等价性门。

| 完整 collar | DER | 漏检率 | 误检率 | 混淆率 | 有效 speaker 时间 |
|---:|---:|---:|---:|---:|---:|
| 0 ms，主指标 | 27.98% | 4.90% | 19.41% | 3.67% | 113.49 秒 |
| 500 ms，仅敏感性 | 26.61% | 2.94% | 21.36% | 2.31% | 94.91 秒 |

两轮分项一致；全部四次评分与直接对原 RTTM/UEM 调用 pyannote 的结果一致。每轮 45 个时间段、3 个匿名 speaker、303 个 observation，中位执行 36.63 秒，进程峰值 RSS 约 305 MiB。与前次六线程输出逐段/向量等价；计时约慢 3.95%，本轮没有速度改动或提速声明。完整 hash、配置、原始秒数与方法见[分离基线回执](../quality/diarization-baseline-evaluation.json)。

这是真实模型在已知训练裁剪上的 word-activity 诊断，不是留出 DER。参考只含词活动，误检秒数可能包含协议未计的发声，不能全部叫作已知静音中的虚构语音；也不能仅按参考 4 人与预测 3 类强制聚类数。下一次参数/算法改变先检查参考语义，再用固定基线和隔离留出参考测误差，不能用更好看的 speaker count 代替 DER。

独立 review 先复现并修复合法 1001 ms 被浮点乘法误拒、backend/设备来源错配仍能评分，以及 bool wallSeconds 被当成一秒从而伪造提速。评分报告保留输入快照 hash，拒绝缺失/失败执行、错误音频与评分空隙，原子发布并保护原始音频、参考和 native 产物；无 ASR/Assistant 质量声明。


### AHC 深树遍历

固定 kodama 0.3.0 的公开 API 仅提供 linkage/dendrogram，没有平面切树接口；保留现有依赖，将私有切树独立为可直接测试的模块。旧实现的展开和整树归类都递归访问左右子树，10 万叶子的合法左深树在 2 MiB 线程栈中分别溢出，普通与优化构建均复现。新实现以显式 `(node, inherited_label)` 栈遍历，右后左先入栈，保持原标签次序；没有扩大线程栈。

独立 review 补测混合子树、阈值包含相等边界和子进程完成标记，4 项通过。旧/新源码对 0 至 128 叶、每种尺寸 8 棵树、9 个阈值共 9,288 次逐标签比较完全一致。生产 adapter 在固定 180 秒训练裁剪运行两轮，时间段/向量逐项等价，四个 DER 分项记录完全一致；中位 35.77 秒，不作提速声明。它只修复切树栈风险，不解决二次距离内存；非单调合并高度的距离语义见下一节，行为等价不能替代算法正确性。见[切树回执](../quality/ahc-tree-validation.json)。


### AHC 非单调高度的距离切分

原 centroid 树只比较当前根的合并距离；子节点高于阈值而父节点较低时，会将本不满足距离上限的 observation 合并。三个单位向量两两距离 `sqrt(3)`，最后一次 centroid 合并降为 1.5；阈值 1.6 时旧实现得到一类，标准 distance cut 得到三类。直接 kodama linkage 和多层反例先红后绿，修复对每个节点计算后代最大高度，再复用既有显式栈切树，不改归一化、linkage、0.6 默认阈值或 VBx。

核对 [SciPy distance 定义](https://docs.scipy.org/doc/scipy/reference/generated/scipy.cluster.hierarchy.fcluster.html)和 [pyannote 当前 VBx 初始化](https://github.com/pyannote/pyannote-audio/blob/623ab14d69a039b474fc1fb2551b657aa00c0ce4/src/pyannote/audio/pipelines/clustering.py#L596)：后者同样是归一化 embedding、centroid/euclidean 和 `fcluster(..., criterion="distance")`。复用标准语义比引入新模型/阈值或 Python 运行时更符合当前原生链路；并未据此改变二次距离算法或压缩 observation。

以相同 f32 距离树输入 Rust 与 SciPy 1.18.1，504 棵树、9 个阈值的 4,536 次分组比较中，旧实现 1,701 次不一致，其中归一化向量实际 centroid 树 104 次；修复后为零。比较允许标签置换，不把分组一致冒称 linkage 浮点完全一致。独立 review 检查子节点先于父节点、有效兄弟簇、包含相等边界，以及普通/优化构建的深树路径。完整工作区 941 项主 harness 与 3 项已有子 harness 通过，11 项 ignored 不作真实执行证据。

固定 180 秒训练裁剪的两轮 native 输出与基线逐段/向量等价，DER 全部分项不变；零 collar 27.98%，500 ms 完整 collar 26.61%。中位 37.83 秒，相对 36.63 秒基线约慢 3.28%，RSS 约 291 MiB；均通过运行前冻结的 1.1 倍耗时、1.2 倍 RSS 和 DER 不退化门，没有提速或留出验收声明。完整来源和门槛见[距离切分回执](../quality/ahc-distance-validation.json)。

验证时另修复 UI 模型夹具的相对共享 `models` 路径：从仓库根启动会读取真实安装，使四项引导测试失败。每个夹具改用独立绝对临时路径，路径/缺失状态回归先红后绿，四项原失败测试通过。首次完整并发回归另外出现两项 UI 失败及占满共享 worker 的测试停滞；两项各按 0–31 种子重跑及最终完整回归通过，未将停滞归因于 AHC，也不声称路径隔离已经修复它。共享资源干扰已按下节受控验证；历史失败的唯一根因没有据此全部闭合。


### UI 回归的共享 worker 隔离

全并行用例共用进程级 IO pool。队列搜索回归为验证取消后释放捕获引用，会占满所有 slot；只让“主动阻塞”的用例持有互斥锁，不能保护未拿锁的普通 UI IO。受控复现先确认全部 slot 已启动，再提交独立 heartbeat：200 ms 内不能完成，释放 slot 后完成并 join，隔离要求显式失败。这个机制能够造成跨测试干扰，不将它当作每一次历史超时的唯一根因。

对照[标准 Rust 测试组织](https://doc.rust-lang.org/book/ch11-03-test-organization.html)和当前私有 API 边界，保留原 GPUI 断言，在独立进程运行此资源耗尽场景，避免为 integration test 暴露 RootView。普通父测试显式用 `--exact --ignored` 运行私有 child；子进程占满自身 pool 后，握手暂停，父进程的真实 IO heartbeat 仍通过，随后释放 child。成功退出与唯一完成标记共同防止零匹配测试假绿；40 秒截止后 kill/wait，日志清理，未增产品配置或调度器。

独立 review 确认 clear/edit/resubmit、owner 数、文件字节和重新搜索断言均保留；父测试不是 GPUI 多种子循环，`ITERATIONS=32` 仅交给 child，避免平方重复。显式运行的 child 日志核对 32 个完成标记；连续三轮完整 UI 各 455 通过，随后完整工作区 941 项主 harness 与 3 项已有子 harness 通过。统计中的 12 项 ignored 包含由父测试显式执行的 1 项 UI child，其他 11 项不能据此声称执行。见[隔离回执](../quality/ui-worker-isolation-validation.json)。

另一次失败发生在编辑器草稿就绪等待，尚未进入详情开关断言；32 个单例种子及后续三轮完整并行均未复现。缩放点击受控场景在旧实现也通过，未按猜测修改点击 helper。等待截止仍为 20 秒，失败仅补充 route/mode/selection、shown/loading/read-error ID 等状态，不记录文档内容或凭据。//TODO - 若再次复现，按模式未切换、读取仍在队列和已产生错误三类状态捕获线程栈与 read 命令握手，先建立确定性红线再修复；当前通过次数不能替代根因证据。

### DER 评分器的正确性

评测代码 review 发现原 DER 同时把说话人替换计作 miss 与 false alarm、只把排序靠前的预测标签纳入候选映射，且同 speaker 重叠段结束时过早清除活动状态。三个手算反例先分别得到错误的 100%/198%/50%，正确答案应为 50%/1%/0%。这些数字是评分器反例，不是模型质量。

先核对已有 MeetEval：其 DER wrapper 需要额外 Perl scorer，缺少时会从未固定 master URL 下载，不适合当前离线、可追溯入口。改用独立环境中锁定的 [pyannote.metrics 4.0.0](https://github.com/pyannote/pyannote-metrics/tree/4.0.0) / core 6.1.0，复用 Hungarian mapping 与标准时域评分，不实现另一套匹配算法。它的 full-duration collar 在参考边界两侧各排除一半，不能与原来的语音膨胀混称；同 speaker 重叠段先合并，其他 speaker 重叠保留。该版本遗漏声明 `typing_extensions` 导致首次真实导入失败，已在可选 requirements 中显式锁定，完整重测通过。

独立 review 增补了参考/预测标签数量不同、重命名不变性、空预测、无参考语音、collar 排除全部参考、20 speaker、非法时间/精度范围，以及无依赖/无分离输出的未评分语义。评分命令另先复现并修复覆盖输入、重复/空/多余会议、旧成功报告残留和输入外部编辑时 hash 不对应；现按原始字节快照评分并原子发布，失败不保留旧成功回执。函数入口对显式携带的 meetingId 同样核对，先复现跨会议预测被接受后再拒绝混配。完整记录见[评分器回执](../quality/der-scorer-validation.json)。Rust export 在有/无可选依赖两种环境均明确验证，不将 unavailable 当作零。真实会议的 UEM、speaker gold、模型 DER 和 AHC 取舍仍未验收；旧手写 DER 不能作为模型质量证据。

### 共享 worker 的排队资源

复查 JobRuntime 发现，旧 gate 在新建 OS 线程后才等待名额；可取消入口的 future 即使未轮询或排队中被丢弃，该线程仍保留捕获的请求和资源。两个红线测试先复现 Arc owner 未释放，再复用已有依赖图中锁定的 [async-lock 3.4.2 semaphore](https://docs.rs/async-lock/3.4.2/async_lock/struct.Semaphore.html) 和 event-listener 5.4.2：先异步准入，再创建线程；取消监听先注册再检查标记，避免丢通知。实际 worker 持有 owned permit 到退出，运行中丢弃 receiver/取消 token 不能绕开并发预算。没有新增执行器、线程池或 native 线程默认值。

独立资源实验占住唯一名额后轮询 64 个排队 future，以 `ps -M -p <pid>` 读取进程线程数；两个执行顺序一致，见[回执](../quality/worker-admission-evaluation.json)。旧/新线程数分别从 3 增至 67/维持 3；丢弃队列后，捕获资源 owner 从 65 保持 65/降至唯一测试 owner。释放 blocker 后两者均回到唯一 owner。本构造只验证排队资源，不证明推理、RSS 或长音频提速。

独立 review 覆盖未轮询/已轮询丢弃、不可取消入口、32 个不同 task waker 的联动取消、运行中丢弃/取消仍持有名额，以及 panic 后下一个任务可执行。原长音频单名额、ASR/DIA 任一阶段排队取消和 panic 分类回归继续验证；worker 异常返回与用户取消仍由调用方检查 token 区分。临时计数 probe 已删除，复现核心保留于附录。

短音频复查发现 Whisper/Sherpa 整文件入口仍使用不可取消准入。两个真实 adapter 的单名额回归先失败：排队取消只能在 blocker 释放后返回。现通过 application `run_backend_worker` 统一复用可取消准入与错误转换，原 backend 仍持有解码/模型执行；6 项回归覆盖两种入口、未轮询取消释放捕获资源、运行中等待退出、错误原样和 panic 分类。显式设置本地模型/音频的 Whisper 与 Paraformer ignored 测试各一次通过，证明准入后的加载/输出链路继续工作，不作为精度或取消延迟的真实模型验收。证据融入[worker 回执](../quality/worker-admission-evaluation.json)。


整文件 native/Sherpa diarization 同样存在不可取消的准入，两个 adapter 各由真实 executor 首次轮询至 Pending 后取消，旧实现均等到 blocker 释放才返回，红线成立。现共用同一 helper，取消直接唤醒 executor，不依赖手动重轮询；window session 路径保持独立，不嵌套申请。完整工作区 943 项主 harness 与 3 项已有子 harness 通过，另有父测试显式执行的 UI child；原生 180 秒加载 smoke 与 Sherpa 8.67 秒共享身份 smoke 分别通过，后者两次真实分离，只证明加载/非空输出及该样本身份关联。

本轮首次尝试 native 8.67 秒已知语音样本时，实际返回零 segment/observation，非空输出断言失败；没有删除这条失败，也不把较长样本通过当作短音频通过。下一轮以固定旧 executable 和 segmentation 窗口规划验证是否遗漏短于 10 秒的输入。两者的输入/model hash、执行范围和回归结果纳入同一[worker 回执](../quality/worker-admission-evaluation.json)。

### 转写分段与时间定位：Unicode 接入和 VAD 试接

2026-10-26 只收窄到分段/时间定位，不更换 ASR 或分离默认模型。Review 的三个问题均先红后绿：中文被当作词内延续跨句/长停顿合并；CPU 比较失败留下旧通过文件；ASR 前置校验失败留下旧成功回执。CPU 输出同时拒绝覆盖输入回执/预测并原子发布；ASR 在读取输入前失效旧回执，前置失败不再能读取为本次成功。

[SentencePiece 官方说明](https://github.com/google/sentencepiece/blob/master/README.md)把 `▁` 定义为空白转义；没有 `▁` 不能证明仍在同一个自然语言词内。采用已锁定的 [unicode-segmentation 1.13.3](https://docs.rs/unicode-segmentation/1.13.3/unicode_segmentation/trait.UnicodeSegmentation.html)，按 [UAX #29](https://unicode.org/reports/tr29/) 的词起点，在完整还原文本上单次前向遍历 UTF-8 偏移；`@@` 明确连续性保留优先级。没有自行维护汉字范围、词典或语言分支。中文句末/长停顿回归恢复分段，英文子词、独立空白、缩写回归保持通过；UAX 词边界不是语义断句或所有语言的词典分词。

固定 FLEURS 中英各前 6 句、两轮，对 Paraformer/SenseVoice 的旧/新 executable 对照；另用前两句中文原音频插入 20 秒数字静音，形成 43.8 秒构造输入。SenseVoice 再反转 runner 顺序复测以检查进程 RSS 波动，共 76 对调用的完整 transcript 均一致。原生调用中位耗时为 SenseVoice 0.776→0.770 秒、Paraformer 0.614→0.617 秒；这组真实样本未触发 Unicode 修复，不据此宣称 CER/WER 或定位改善。SenseVoice 的 baseline 峰值 RSS 为 865–1044 MiB、candidate 为 1039–1043 MiB；不得把初次进程高水位差异直接归因为分词开销。完整输入、模型、预测和 runner hash 见[分段评测回执](../quality/asr-boundary-evaluation.json)。

时间定位仍有独立缺口：SenseVoice 在静音构造输入中保留的“明”覆盖 11.10–24.96 秒，其中 9.8 秒落入已知静音 `[15160, 35160]` ms。Unicode 调整不改变这组结果。没有可靠 duration/词对齐时，token 的 next timestamp 或离线窗口范围不能等同于真实发声边界。

#### Silero VAD 隔离试接

选择现有 sherpa-onnx 1.13.6 的原生 [Silero VAD 接口](https://raw.githubusercontent.com/k2-fsa/sherpa-onnx/v1.13.6/rust-api-examples/examples/silero_vad_remove_silence.rs)，不引入 Python 推理 runtime。模型来自 [sherpa 官方列表](https://k2-fsa.github.io/sherpa/onnx/vad/silero-vad.html)，MIT license；本次下载 643854 字节，观测 SHA-256 为 `9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6`，不是发布者签名声明。模型仅放在忽略目录，已真实加载；未加入桌面 catalog 或处理路径。

试验固定 CPU 一线程、threshold 0.5、min silence/speech 各 0.25 秒、max speech 30 秒、512 sample 窗、每侧 padding 200 ms；相邻 padding 重叠时合并，保持原时间偏移。每个语音片段复用当前生产 ASR adapter，按原文件重新汇总文本与时间，VAD 不决定 speaker 或语义。原生 12 句、静音构造 1 个、中英各 1 个降低 30 dB 的变体、纯静音分别记录；低音量变体不等于真实噪声场景。VAD 共 31 次执行，裁剪后的 ASR 共 50 次，两轮重复，保留每次重新加载模型的成本。

运行前冻结门槛：原生/低音量错误率不得增加超过 1 个百分点，不新增空预测；构造静音内覆盖最多 0.5 秒；中位耗时最多增加 10%；ASR 子进程 RSS 最多 1.2 倍。时间含各原文件的 VAD 加载/检测与全部 crop ASR 调用，不是常驻模型成绩。ASR RSS 单独记录，不代表 Audio Host 总内存。

| 场景 / 指标 | 当前原路径 | VAD 裁剪路径 | 判断 |
|---|---:|---:|---|
| 英文原生 WER，6 句 × 2 | 7.21% | 10.81% | 超出质量门 |
| 中文原生 CER，6 句 × 2 | 17.58% | 17.19% | 仅本小集改善 |
| 中文降低 30 dB CER，1 句 × 2 | 40.00% | 66.67% | 超出质量门 |
| 中文降低 30 dB 中位耗时 | 1.022 秒 | 1.510 秒 | 增加约 47.8%，超出延迟门 |
| 20 秒已知静音内覆盖，两轮一致 | 9.8 秒 | 0 秒 | 边界改善不能抵消识别退化 |

结论：拒绝把该裁剪方案作为默认集成；没有调阈值追逐本小集，也没有修改生产路由。VAD 的语音活动证据和 ASR 文本/词时间必须分别验收，不能用更整齐的时间段掩盖遗漏或识别退化。//TODO - 先从 AMI 的官方词时间建立配对参考，再验证自然会议、长静音、低音量、重叠与噪声；对任何可选活动检测方案同时检查文本、覆盖、词时间、取消和资源，保留原音频与可恢复的原路径。

现有 `asr_benchmark.py` 增加可选 `knownSilenceRangesMs` 与分组 `knownSilence`：参考区间与预测区间各自合并取并集后计算交集，重叠段不重复计量；失败标为未测，不能记作零覆盖。范围与有限数值先校验，不影响未提供标注的旧病例。它仅测已知静音覆盖，不是词对齐分数；真实插入静音已逐字节核对为零。临时 VAD probe/裁剪编排源码已删除，保留回执与附录核心方法。

### 粗时间诊断接入与 Whisper EOF 边界

为区分文本识别与粗时间合理性，隔离接入 [MeetEval 0.4.3 的 SISO 时间约束 WER](https://raw.githubusercontent.com/fgnt/meeteval/badcd3c7cf82f98d2ac1f292801fbe6e9093ee2f/meeteval/wer/wer/time_constrained.py)。`requirements-timing.txt` 仅属于离线评测，不进入桌面推理。`report_asr_timing.py` 复用生产 benchmark 的 hash/完整覆盖校验和原样 WER 归一口径；按语言、场景、repeat、collar 输出原样 WER 与 tcWER，同时保留逐 case 统计。旧 run 缺少 `wordTiming` 时需用新病例重跑，不修改旧快照。

[固定版本的官方说明](https://raw.githubusercontent.com/fgnt/meeteval/badcd3c7cf82f98d2ac1f292801fbe6e9093ee2f/doc/tcpwer.md)明确指出：hypothesis 的 `full_segment` 可用超长段绕过约束。试接采用其推荐的 `character_based_points`，参考使用既有官方词区间，不重新估计参考时间。运行前冻结 0.5/2 秒两个 collar；0.5 秒为敏感性探测，官方在 LibriCSS 推荐的 2–5 秒范围不能直接变成当前短 AMI 的精度门。近似点不能证明模型提供了词时间；本工具只做单人英语粗时间诊断，不评 DER、中文时间或词级精度，也不修改默认模型。

同一组五个 AMI 裁剪、95 个参考词，SenseVoice/Whisper 各两轮。下表为两轮一致的结果；Whisper 为修复源音频越界后的输出，原配置在诊断阶段被明确拒绝，见[完整回执](../quality/asr-temporal-evaluation.json)。

| 模型 / 配置 | 原样 WER | tcWER，0.5 秒 | tcWER，2 秒 |
|---|---:|---:|---:|
| SenseVoice small int8，CPU 四线程 | 26.32% | 71.58% | 29.47% |
| Whisper base.en，Metal 配置、四线程 | 29.47% | 64.21% | 32.63% |

collar 敏感性很大，差值同时包含粗段内近似与参考误差，不是“词时间错误率”，不得用于默认模型排名或质量通过声明。纯文本正确但整体平移、超长整段的正反例均经工具验证；未评估项与失败保持可见。报告原子发布，失败重跑清除旧报告、拒绝覆盖输入；EOF 比较回到 sample 域，已先复现并修复 `1.001 * 1000` 浮点误差导致合法 1001 ms 被拒绝的问题。

真实调用发现 Whisper 对 `ami-ES2002a-4` 的 8670 ms 源音频返回 9000 ms 结束点，两轮一致。显式设置 base.en 权重和第五裁剪路径的真实 ignored 测试先失败；随后整文件与窗口共用有界 sample 范围转换，裁去 decoder padding 的时间，纯 padding 段不进入结果。修复后真实测试通过，十次配对调用保留全部文本、ID、起点和分段，仅两次 EOF 从 9000 改为 8670 ms。SenseVoice 十次、Whisper 前后各十次，共三十次独立 ASR 调用均成功；没有放宽评分器范围检查。该修复只保证源音频范围，不将 EOF 当作真实发声结束点。

### 原生 Whisper 词级分段试接

在现有锁定的 whisper-rs 0.16.0 / whisper-rs-sys 0.15.0 上，隔离验证上游标为 experimental 的[词级时间戳](https://github.com/ggml-org/whisper.cpp#word-level-timestamp-experimental)。沿用同一解码参数、base.en 权重、Metal 配置、四线程和 sample 范围过滤，仅设置 `token_timestamps=true`、`max_len=1`、`split_on_word=true`。没有启用 DTW；binding 的 DTW token point 与普通 segment 起止时间是不同字段，不能混称。

先冻结文本原样/归一化均须保留、时间范围有效、tcWER 改善、耗时和 RSS 门槛，再按 coarse→words、words→coarse 两种顺序各执行两轮。五个 AMI 病例每种配置共二十次调用，控制组 transcript 与上一轮生产输出完全一致。两种顺序下 WER 均为 29.47%；词级分段的 tcWER 0.5 秒从 64.21% 降至 35.79%，2 秒从 32.63% 降至 29.47%。两次中位耗时分别为 215.2→226.0 ms、207.1→218.6 ms，过程 RSS 比值 0.991/0.983。首个 coarse 调用实际耗时 7.896 秒，原因未定位，原值保留，不把它解释成已证实的冷启动收益。每两轮输出段数从 14 增为 176。

独立 review 后补入六个固定 FLEURS 英语输入、42.94 秒的两句加二十秒全零间隔，以及原句缩小 30 dB 的输入，每种配置各两轮、共三十二次调用。词级模式在 `fleurs-en-1935` 两轮均被现有时间范围过滤丢弃 `there`，尽管原始 decoder 文本包含该词；六句 WER 从 17/136 增为 18/136。构造静音覆盖从 3340 降至 3160 ms，低音量文本一致，均不能抵消文本保留门失败。没有依据短样本调整过滤规则以迁就候选。

结论：拒绝默认接入，见[完整回执](../quality/whisper-word-timing-evaluation.json)。即使消除这次丢词，直接将 canonical segment 改成逐词仍会改变转录阅读的段落密度、逐段字面搜索以及持久化容量；需要独立的产品 contract 和跨段用例验证。此次七十二次调用仅验证局部候选，不代表词级精度、中文、小时级资源或生产验收。临时 probe、dev dependency 和 shim 已清理，生产默认参数不变。

## 复现说明

1. `uv venv --python 3.12 eval-data/asr-eval/venv`，在该环境安装 `scripts/audio-eval/requirements.txt`。MLX 实验安装 `scripts/audio-eval/requirements-mlx.txt` 的固定依赖，桌面不依赖这些包。
2. 从上述固定 FLEURS revision 下载 `data/cmn_hans_cn/audio/test.tar.gz`、`data/en_us/audio/test.tar.gz`，分别放入 `<root>/downloads/zh-test.tar.gz`、`en-test.tar.gz`。`prepare_fleurs.py --root <root>` 校验 archive SHA、读取官方 TSV 并产生病例；`--offset 24 --language zh` 构建第二组句子。
3. `prepare_ami_asr.py --audio <ES2002a.Mix-Headset.wav> --annotations <ami_public_manual_1.6.2.zip> --root <root>` 同时校验官方素材与标注 SHA，产生独立 AMI 病例与 `wordTiming`。词时间先校验有效性、顺序与完整包含关系，再以最近 sample、half-even 取整；保留词重叠，不裁短跨边界词，也不从间隙推导静音。
   可选时间诊断在独立 Python 3.12 环境安装 `scripts/audio-eval/requirements-timing.txt`；先以含 `wordTiming` 的病例执行原 ASR benchmark，再运行 `report_asr_timing.py --run <run-dir> --output <temporal.json> --collar 0.5 2`。保存环境版本、原始预测与 scorer/normalizer SHA；`status: complete` 仅表示诊断完成，`qualityAcceptance` 固定为 `not_assessed`。
4. `cargo build --release --locked -p asr --example asr_benchmark`；使用配置文件指定 `model`、`device`、`threads`、`repeats`、`windowed`。模型路径必须是已安装本地资产，backend 不下载。
5. `asr_benchmark.py --cases <cases.json> --config <configuration.json> --runner <asr_benchmark> --output <run-directory>` 执行与评分。模型加载、崩溃、超时、取消与缺失结果均保留；成功回执必须覆盖配置中的每个 case 与 repeat，读取时同时校验 manifest 和新回执中的预测 SHA-256；模型输出不掺入 gold。MLX runner 换为 `scripts/audio-eval/mlx_qwen_probe.py`。
6. `cargo build --release --locked -p diarization --example diarization_benchmark`；`diarization_benchmark <backend> <model-directory> <audio> <output.jsonl>` 独立运行两次，配合 `/usr/bin/time -l` 记录进程 RSS；未提供 speaker gold 时明确记为未评估精度。
7. `check_asr_windows.py --run <candidate-run> --baseline <original-run> --max-clean .2 --max-noise .25` 检查冻结质量门、时间范围和短路径逐段不变。旧 Paraformer clean 94.88% 触发失败；开发与第二组的 whole/window 路径均须通过。
8. `report_asr.py --root <root> --output <report-directory> --date 2026-10-25` 从逐样本预测重新计算统一口径的表格与 receipt；原始结果不因评分器更新而删除。

配置例：

```json
{
  "model": {
    "backendId": "sherpa-onnx",
    "modelId": "paraformer-zh",
    "revision": "2024-03-09",
    "artifactPath": "/path/to/models/asr/paraformer-zh"
  },
  "device": "Cpu",
  "threads": 4,
  "repeats": 2,
  "windowed": false
}
```

## 注意事项与补充

模型 checksum 会预热文件缓存，首次加载与重复执行均不代表磁盘冷启动。MLX 模型驻留时，加载耗时与调用耗时分开记录；跨加载策略不能直接据 RTF 宣称端到端倍速。RSS 是进程高水位，不是独立 GPU/ANE 显存。

FLEURS 与 AMI 使用 CC-BY-4.0；下载权重按其版本化来源保留许可，不提交音频、模型、用户数据或大运行目录。案例音频的制作不改变原始资产。

//TODO - 在更大的中文自然会议、远场、重叠与小时级数据上扩展 B0–B3；固定词时间与 speaker gold 后再评时间误差、DER、身份和端到端传播损失。两次执行、少量会话和构造噪声不证明生产准确度或发布质量。

### 验证记录

| 检查 | 结果 |
|---|---|
| `cargo fmt --all -- --check`、`git diff --check` | 通过 |
| `cargo check --workspace --locked --all-targets` | 通过，包含独立 benchmark example |
| `cargo test --workspace --locked` | 933 个主测试通过、0 failed、11 ignored；另有三次锁/读写子进程测试通过；Whisper EOF 的真实 ignored 回归另行显式通过 |
| `cargo build --locked -p meetnote` | 桌面开发构建通过 |
| Python 评分/基线测试 | timing + DER 环境 80 passed；基础环境 50 passed、30 项可选 timing/DER/参考投影测试跳过；涵盖 DIA 正反例、精确距离、覆盖、预测 hash、旧回执失效、取消/超时、AMI 词时间与粗时间诊断的正反例、输入保护和 sample 边界 |
| 窗口真实回归 | 旧整文件超过冻结门槛；新开发/第二组的 whole/application-window 四组均通过 clean ≤20%、noise ≤25% 与时间范围检查；两组短路径共 96 次调用逐段保持一致 |
| review 真实回归 | 当前 release 完成 12 组、128 次调用；五种 Whisper 配置各两次成功，四组 Paraformer 质量门与 96 次短路径逐段一致通过，SenseVoice 英文长输入两次成功；31 秒全零 ignored 回归已单独显式执行通过 |
| CPU 分离对照 | 180 秒同一自然录音，baseline/candidate 各两次；35.233 秒中位耗时；宽区间与三种空转实验均保留，通过设备预算的性能/输出一致性门；不作为 DER |
| 模型与资源 | 自动汇总选取 23 次配置执行、1016 次完成调用；另保留全部 43 次配置迭代、1768 次完成调用及失败记录。native/sherpa DIA 与同一音频 ASR 各两次独立 release 执行 |

ignored 资源测试未被常规 suite 自动执行；真实推理由独立 runner 显式提供模型与音频完成。此次不据单元测试或短样本宣称 DER、词时间、20 小时资源或发布质量通过。

### 附录：核心核对方法

Whisper 根因核对同时使用固定版本 binding 源码、macOS faulting-thread 栈和隔离真实推理，避免把进程退出码当作原因。窗口优化保留旧 executable 的 SHA，使用同一官方参考重跑；短输入与第二组中文分别核对，不能仅用文本长度接近原文件判断质量。

CPU 探针的核心方法为直接加载固定 `EmbeddingModel`，对同一输入的首、中、末三个 10 秒窗分别执行三轮 `embed` 并计时、保存向量；另用相同 segmentation 模型测整段处理。只在临时源码里改变 CPU intra-op 线程数为 1/2/4，顺序构建与执行，生产对照冻结完整 baseline executable。探针源码已移出构建目标并删除，保留此核心方法：

```rust
use speakrs::inference::EmbeddingModel;
let mut model = EmbeddingModel::with_mode(embedding_path, speakrs::ExecutionMode::Cpu)?;
assert!(samples.len() >= 160_000);
let mut measurements = Vec::new();
for _ in 0..3 {
    for start in [0, samples.len() / 2 - 80_000, samples.len() - 160_000] {
        let started = std::time::Instant::now();
        let vector = model.embed(&samples[start..start + 160_000])?;
        // Record elapsed seconds and the complete vector outside the timed call.
        measurements.push((start, started.elapsed().as_secs_f64(), vector));
    }
}
```

完整对照用相同 `diarization_benchmark` example，冻结基线后仅更改 vendor session 的线程数；两版分别运行 `/usr/bin/time -l <runner> diarization-native <model-dir> <audio> <prediction.jsonl>`。每个 run directory 的 `report.json` 使用 `meetnote.diarization-cpu-run.v1`，记录音频/全部模型文件/runner/预测 SHA、音频秒数、配置中的 repeats、embeddingThreads 及试验中的空转参数、user+sys CPU 时间、进程峰值 RSS 和 failure；结构示例与真实 hash 见 CPU 对照回执。按此 manifest 运行 `check_diarization_cpu.py --baseline <before-dir> --candidate <after-dir>`，检查完成覆盖、逐段时间线、每个 observation 的来源/向量以及资源门槛；`--criterion quality` 只测输出等价，`--criterion idle-cpu` 独立检查 CPU 效率门，默认 `speed` 保持至少 15% 提速和 1.2 倍 RSS 的原门槛。原始预测、向量与音频继续留在忽略目录。

#### Whisper 线程试验核心

冻结同一个 release `asr_benchmark` executable，输入和模型 checksum 预热后按固定配置运行 `asr_benchmark.py --cases <cases.json> --config <device-thread.json> --runner <frozen-runner> --output <run> --timeout 120`，配置只改 `device`/`threads`，保持 whole-file 与每句两轮。按升序 Cpu→Metal、降序 Metal→Cpu 执行，不并行构建。补充 large/Turbo 使用原生中英各一句；竞争诊断同时启动两个原 benchmark 子进程，并分别保留其报告、预测与整批 wall。两种顺序都检查质量与资源门；所有文本对照按原病例 language 归一，时间范围按源 sample 检查。源码无需新增实验开关，使用已有永久 benchmark；原始结果和冻结协议位于忽略目录，汇总回执记录 runner、输入、模型和预测 SHA。

#### 排队资源实验核心

固定 `with_max_concurrency(1)`，在独立 driver 中运行持有 release channel 的 blocker，并等待其 started 信号。为同一 runtime 创建 64 个 `spawn_blocking_cancellable` future，每个捕获同一个 `Arc<()>` 的副本，逐个用 noop waker 轮询至 Pending。由外部进程分别在 blocker 已开始、排队完成、丢弃所有 future 三个握手点执行 `ps -M -p <pid>`，记录线程数及 `Arc::strong_count`；最后关闭 release channel、等待 blocker 和捕获 owner 释放。旧源码以独立 module 加入同一个临时 release probe，运行 baseline→candidate、candidate→baseline，不并行编译/采样，不使用模型或用户数据。保存源码、二进制与协议 SHA，实验后删除 example。

#### 原生词级分段试接核心

临时 ignored probe 复用 backend 私有的 `whisper_params` 和 `bounded_sample_range`，每次重新加载同一模型；控制组逐段核对生产 runner 后，只对候选应用：

```rust
prepared.params.set_token_timestamps(true);
prepared.params.set_max_len(1);
prepared.params.set_split_on_word(true);
```

同时保存原始 decoder 字符流、过滤前的非空文本、过滤后 transcript 与耗时，按 case/repeat 比较归一化文本和去空白字符流；检查输出时间在源 sample 范围内。两组反转顺序，不并行编译或计时。使用同一 AMI `wordTiming` 与 MeetEval 0.4.3 评分；补充 FLEURS 六句和构造静音/音量病例只测文本与已知静音，不伪造词时间。回执保留临时源码、native executable、模型、输入、预测和冻结协议 SHA；失败后删除临时代码，不将试验开关加入产品配置。

#### 分段评测的复现核心

1. 从固定 FLEURS manifest 取每种语言前 6 个原生病例，原 ASR 配置四线程、每个病例两轮；冻结旧/新 runner SHA 后顺序执行。SenseVoice 再反转顺序复核；不得并行构建和计时。
2. 将首两句中文的 16 kHz mono PCM 按原采样拼接，中间插入 `20 * 16000` 个零；manifest 增加 `knownSilenceRangesMs: [[15160, 35160]]`。中英首句分别以 `10 ** (-30 / 20)` 缩放 PCM 并重新固定 hash，参考文本不变。只把构造变换本身标为 gold。
3. 用官方 VAD 模型和以下核心代码建立临时 example；执行后删除该源码。对 ranges 两侧加各 3200 sample padding、夹在源文件范围内并合并重叠区间；逐段通过相同 `asr_benchmark` executable 识别，恢复原 sample offset，再按原文件和 repeat 汇总评分。不能将各 crop 都赋予完整原文作为独立 gold。

```rust
use sherpa_onnx::{SileroVadModelConfig, VadModelConfig, VoiceActivityDetector};
let config = VadModelConfig {
    silero_vad: SileroVadModelConfig {
        model: Some(model_path.into()), threshold: 0.5,
        min_silence_duration: 0.25, min_speech_duration: 0.25,
        max_speech_duration: 30.0, window_size: 512,
    },
    ten_vad: Default::default(), sample_rate: 16000, num_threads: 1,
    provider: Some("cpu".into()), debug: false,
};
let vad = VoiceActivityDetector::create(&config, 60.0).expect("load VAD");
let mut ranges = Vec::new();
for chunk in samples.chunks(512) {
    vad.accept_waveform(chunk);
    while let Some(segment) = vad.front() {
        ranges.push((segment.start(), segment.start() + segment.n()));
        vad.pop();
    }
}
vad.flush();
while let Some(segment) = vad.front() {
    ranges.push((segment.start(), segment.start() + segment.n()));
    vad.pop();
}
```

4. 用当前 `asr_benchmark.py` 对带静音标注的 manifest 运行，`report.json.groups[].knownSilence` 应能复现原路径每轮 9800 ms 的交集；裁剪路径以相同 `summarize` 方法对恢复时间后的输出计算为 0。回执保存源音频、权重、runner、输入/预测 hash 与预先冻结的门槛，原始音频、裁剪和输出不提交。

#### 深树与切分等价性实验核心

从固定旧 commit 抽取原切树函数，与生产私有模块分别编译进临时纯 Rust driver，复用锁定 kodama；小树以固定种子 `0x86739` 从当前可用节点中逐次抽取两节点合并，逐阈值比较完整标签数组。深树先合并叶对，再将已有根作为左孩子与下一叶对合并，避免右尾调用优化；子进程在线程 `.stack_size(2 * 1024 * 1024)` 中核对 100,000 个标签并输出唯一完成标记。优化诊断使用 `rustc --test -O` 编译同一生产模块与测试；完整 cargo workspace 测试另行运行。实验后移除临时 driver，保留永久回归和来源 SHA。

#### AHC 距离规则交叉验证核心

用 NumPy seed 86739 对 2 至 64 个叶子分别构造四组归一化向量（2/3/8/32 维）的 SciPy centroid 树和四组合法随机合并树，将距离统一转为 f32 后同时送给两个 Rust 私有切树实现与 SciPy `fcluster(criterion="distance")`。阈值为 0/0.2/0.4/0.6/0.8/1/1.2/1.6/2；按每个标签首次出现的次序重编号，比较完整分组。临时 driver 从 stdin 读取 Step，避免另造 inference provider；实验后删除。native 复核另用已有永久 benchmark，固定六线程、相同模型/音频 hash、自动人数和无 Library 两次调用，标准 DER 与向量等价性分别核对；先等待原始回执完整写入，再评分。

#### 共享 worker 干扰的受控复现核心

旧布局中，`OccupiedUiWorkers::new()` 逐 slot 等待 started 信号，直到当前进程全部 IO slot 被占用；另起 driver 提交普通 `runtime::spawn_blocking` heartbeat，等待 200 ms，先释放占用并 join driver，再断言 heartbeat 必须提前完成，得到红线。永久隔离测试在 child 执行同一占用和完整搜索断言，stdout 发布 `UI_WORKERS_HELD`，stdin 等待父进程释放；父进程实际执行独立 IO，校验 5 秒内完成，再关闭 stdin。child 正常退出和完成标记均必须存在。临时同进程探针已删除，保留探针 SHA 和永久父/子实现。
