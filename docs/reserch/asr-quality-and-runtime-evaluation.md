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
- [AMI](https://groups.inf.ed.ac.uk/ami/corpus/)：ES2002a Mix-Headset 与官方 manual 1.6.2 词标注；每位说话人最多两个原始片段，3–20 秒、至少六个词、前后各 100 ms、无其他说话人的词级重叠，共五个片段、37.29 秒。它是使用官方边界裁剪的 B1 会话样本，不测试自动 VAD 或重叠识别。
- 中文按 NFKC/casefold 后的非空白文字计算 CER；英文标点分词、去除撇号后计算原样 WER，并单独给出固定 `whisper-normalizer 0.1.15` 的 normalized WER。两者均使用 RapidFuzz 精确 Levenshtein，不使用近似距离。原样错误率保留在报告中。

结果见[本机模型比较](../quality/model-comparison.md)与同目录机器 receipt。原生片段、构造片段、AMI 和第二组中文分别展示，不把重复或构造数据当作独立会议样本。

### 已证实的实现问题

| 问题 | 证据与处理 |
|---|---|
| Whisper 取消回调的 userdata 类型不一致 | 固定 `whisper-rs 0.16.0` 的 safe wrapper 将双层 Box 的地址交给按具体 closure 类型读取的 trampoline。五组 CPU/Metal 对照发生 SIGSEGV，macOS 栈落在 trampoline → `JobContext::is_cancelled`。adapter 改用类型明确的 C 回调和自有 `Box<JobContext>`，由参数对象持有到同步 `full` 返回后释放；不改 vendor，不泄漏 closure |
| Core ML 请求静默回退 CPU | 本机真实运行日志出现 `CoreML ... Fallback to cpu!`。当前 sherpa ASR/diarization adapter 明确拒绝不能保证执行的 Core ML/Metal 请求，返回 `DeviceUnavailable`；不把 CPU wall time 记录为 Core ML 加速 |
| SentencePiece 子词被插空格 | 原合并将 `ent`、`om`、`ologist`、`s` 拼成多个词。保留 SentencePiece 显式词边界和 Paraformer `@@` 连续标记，在词内直接连接；单独的词边界标记只影响下一个词，不产生空段。正常整词行为保持独立，新增回归先失败再修复 |
| 单次推理长度与应用长音频路由混淆 | 原 application 在 30 分钟才进入长音频路径，但同一批中文拼成 295.42 秒后，Paraformer 整文件 CER 达 94.88%，短片段为 11.76%；加线程保持相同错误率。ASR backend 的模型推理粒度必须独立于 Audio Host 的长录音路由 |
| sherpa 默认线程隐式降为一线程 | 原 `Default` 产生零，运行时通过 `max(1)` 使用一线程。相同权重与输入下四线程改善 wall time；默认值改为可用逻辑 CPU 数与四的较小值，保持明确上限 |

窗口扫描比较了 Paraformer 的 6+2/8+2/10+2.5 秒（核心+单侧上下文），开发构造 clean 的 CER 分别为 16.88%/14.81%/15.58%，noise 为 21.35%/16.34%/18.19%。30 秒推理上限的初稿仍有 36.17% clean CER，未通过预先设定的 20% 门槛，不能作为最终修复。最终采用 8+2 秒；SenseVoice 采用 20+5 秒。30 秒以内 whole-file 输入保留 baseline；应用 60+15 秒窗口内部仍按模型粒度处理。子窗口从父 valid core 开始分配，而不是从重复 context 开始分配，避免嵌套窗口多解码与跨父窗口上下文顺序倒退。

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

[组件资源回执](../quality/processing-profile.json)记录同一段 AMI 前 180 秒的独立 release CPU 分离对照：native 两次 166.715/166.256 秒，峰值 RSS 292 MiB；sherpa 两次 62.228/62.000 秒，733 MiB。未指定预期人数，输出分别为 3 与 21 位 speaker；该计数不是 DER，不能据速度更换默认分离器。同一音频的 Whisper Turbo Metal 两次 ASR 为 17.587/10.196 秒（含各次模型加载；两轮差异保留，未进一步拆分加载、内核预热与解码成本）。按独立组件均值估算串行 ASR+DIA，ASR 仅占约 7.7%；即使 ASR 时间减半，总时间预计也仅减少约 3.9%，此估算不含对齐/保存且不等于整链实测。完整处理速度应优先测量分离的 segmentation/embedding 和可验证的硬件执行路径，而不是只替换更大的 ASR 模型。

native diarization 仍保留全录音 observation 和 AHC 二次距离数组。ASR 分窗或更快模型不解决这一内存墙；小时级与 20 小时资源需要独立 B2/B3 验收。

## 复现说明

1. `uv venv --python 3.12 eval-data/asr-eval/venv`，在该环境安装 `scripts/audio-eval/requirements.txt`。MLX 实验安装 `scripts/audio-eval/requirements-mlx.txt` 的固定依赖，桌面不依赖这些包。
2. 从上述固定 FLEURS revision 下载 `data/cmn_hans_cn/audio/test.tar.gz`、`data/en_us/audio/test.tar.gz`，分别放入 `<root>/downloads/zh-test.tar.gz`、`en-test.tar.gz`。`prepare_fleurs.py --root <root>` 校验 archive SHA、读取官方 TSV 并产生病例；`--offset 24 --language zh` 构建第二组句子。
3. `prepare_ami_asr.py --audio <ES2002a.Mix-Headset.wav> --annotations <ami_public_manual_1.6.2.zip> --root <root>` 同时校验官方素材与标注 SHA，产生独立 AMI 病例。
4. `cargo build --release --locked -p asr --example asr_benchmark`；使用配置文件指定 `model`、`device`、`threads`、`repeats`、`windowed`。模型路径必须是已安装本地资产，backend 不下载。
5. `asr_benchmark.py --cases <cases.json> --config <configuration.json> --runner <asr_benchmark> --output <run-directory>` 执行与评分。模型加载、崩溃、超时与缺失结果均保留；模型输出不掺入 gold。MLX runner 换为 `scripts/audio-eval/mlx_qwen_probe.py`。
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
| `cargo test --workspace --locked` | 900 个主测试通过、0 failed、10 ignored；另有三次锁/读写子进程测试通过 |
| `cargo build --locked -p meetnote` | 桌面开发构建通过 |
| Python 评分/基线测试 | 15 passed；精确距离、标点、归一口径、加权、失败/缺项与输入 hash 校验 |
| 窗口真实回归 | 旧整文件超过冻结门槛；新开发/第二组的 whole/application-window 四组均通过 clean ≤20%、noise ≤25% 与时间范围检查；两组短路径共 96 次调用逐段保持一致 |
| 模型与资源 | 自动汇总选取 23 次配置执行、1016 次完成调用；另保留全部 43 次配置迭代、1768 次完成调用及失败记录。native/sherpa DIA 与同一音频 ASR 各两次独立 release 执行 |

ignored 资源测试未被常规 suite 自动执行；真实推理由独立 runner 显式提供模型与音频完成。此次不据单元测试或短样本宣称 DER、词时间、20 小时资源或发布质量通过。

### 附录：核心核对方法

Whisper 根因核对同时使用固定版本 binding 源码、macOS faulting-thread 栈和隔离真实推理，避免把进程退出码当作原因。窗口优化保留旧 executable 的 SHA，使用同一官方参考重跑；短输入与第二组中文分别核对，不能仅用文本长度接近原文件判断质量。
