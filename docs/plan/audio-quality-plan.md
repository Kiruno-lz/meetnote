# 渐进式自动化 Benchmark 计划

## 目标与原则

建立 MeetNote 专属、版本固定、参考答案确定的 benchmark 集，自动生成测试音频、运行、评分并产出可读模型比较文档；不需要用户标注、干预或人工评审。测试分原子功能、组合链路和 Agent 插件语义三层，精准定位误差来源。

“确定性”指素材、构造规则、gold、参数与评分固定，不要求推理输出逐字恒定。参考答案来自官方标注和可验证的变换，不能来自待测模型。原子测试先于组合，低成本先于高成本：离线检查 → 本地 ASR/diarization → 对齐/窗口/资源 → Agent 语义 → 端到端。当地计算仍消耗 CPU/GPU、时间和内存，但不产生远程调用费用。

Agent Host 是否正常只按 [Assistant 计划](assistant-plan.md)验收；本计划中的语义质量测试通过 Host 调用语义插件，不能把语义策略写入 Host，也不阻塞其三项基础目标。

## 固定 Benchmark 集

B1 已有固定 FLEURS revision、archive/audio SHA、官方参考文本、句子 ID 与构造配方的 manifest；AMI 固定 ES2002a 官方音频/词标注并保存裁剪来源。`prepare_fleurs.py`、`prepare_ami_asr.py` 自动生成并拒绝 checksum 不匹配。音频、模型和逐样本大产物留在忽略目录，汇总 receipt 可提交。

- //TODO - 扩展 manifest 的会议/speaker split、词/speaker gold 投影、适用任务与资源档位；现有 FLEURS 第二组仅句子 ID 分离，不能标为 speaker-disjoint holdout。
- //TODO - 自动从 AMI、AliMeeting、AISHELL-4、CHiME-6 等已标注数据提取原生片段和构造片段；固定解码/重采样工具版本。拼接、插入静音、叠加与噪声变换同步变换词时间、speaker 身份、重叠区与有效评分区，边界跨词时自动调整到合法标注点，无法可靠投影的样本自动拒绝。
- //TODO - 原生片段测真实分布；构造片段测精确边界/极端情况，分开统计。语义 gold 来自明确 topic 标注或预先选定的不同主题组合；不能将任意拼接点、静音或 speaker 切换自动当作语义 gold。
- //TODO - 固定开发与留出集，按原始会议/speaker 隔离，构造变体沿用源 split；禁止同一素材的裁剪/拼接副本泄漏到另一 split。参考标注不足时标记 unsupported，不临时让用户补答案。
- //TODO - 优先完成 AMI 音频与官方词级时间/speaker 标注的成对构造，再扩充其他场景；已有 AMI 手工文本 topic 评测不等于音频 benchmark 已就绪。

| 维度 | 初始必备覆盖 |
|---|---|
| 内容/语言 | 中文、英文；单人叙述、双人访谈、多方会议；中英混合仅选参考可靠的素材 |
| 声学/说话人 | 近场/远场、安静/噪声、单人/多人、重叠、静音、短发言、同一 speaker 跨片段出现 |
| 时长/边界 | 30–60 秒 smoke、3 分钟短样本；60 秒处理 core 和 30 分钟路由阈值前后；长档逐步扩至小时级/20 小时资源测试 |
| 语义/项目 | 单主题不拆分、多主题连续切分、重复主题提取、未覆盖范围、二次拆分/三层树、来源回读 |
| 运行 | 冷/热启动、顺序/受控并发、取消、模型/解码/写入失败；功能通过率与模型质量分别计分 |

采用分层代表集与逐级扩容，不立即跑所有维度的笛卡尔积。重复拼接的超长音频只证明资源/边界行为，不证明真实长录音精度。

## 分级执行与自动门禁

| 阶段 | 独立输入与输出 | 自动验收及推进条件 |
|---|---|---|
| B0 素材/评分器 | manifest、音频、官方 gold、固定构造配方 | 校验 checksum、时间轴、speaker 映射、评分器正反例；通过才允许运行模型 |
| B1 ASR | 音频 → transcript；不启动 diarization/Agent | 分语言 WER/CER、遗漏/重复、可用时间戳误差，wall/RTF/峰值内存；从 smoke 通过后扩短/长档 |
| B2 diarization | 音频 → speaker 时间段；不依赖 ASR 文本或 Agent | 固定 UEM/collar/overlap 的 DER 及 miss/false alarm/confusion、speaker count、边界误差；AMI 优先 |
| B3 对齐/窗口/身份/运行 | 先用 gold transcript+gold speaker 测纯对齐，再只替换一个预测；窗口与整文件对比 | 分离对齐损失、窗口重复/缺失、跨窗口身份漂移；独立测身份匹配、取消/并发/恢复和资源 |
| B4 Agent 语义插件 | 先用 gold transcript/speaker 投影，经 Host 调用插件 → 语义范围/工具结果 | 固定 boundary F1、Pk/WindowDiff、coverage/evidence、结构/写回成功率；不让 ASR 误差混入语义基线 |
| B5 端到端归因 | 相同 case 逐步换为 ASR prediction、diarization prediction，再运行完整链路 | 与 B1–B4 对比质量降幅、耗时/内存/调用量，定位传播损失；不得仅给一个总分 |

B1 的可选 `knownSilenceRangesMs` 标注以区间并集计算转写与已知静音的交集，分组输出 `knownSilence`；失败标为未测，不能当作零覆盖或词对齐通过。B1 的 `asr_benchmark` 直接运行生产 adapter；`asr_benchmark.py` 在独立进程记录失败、超时、wall 与 RSS，并采用精确 Levenshtein。B2 的 `diarization_benchmark` 保留独立执行/资源及完整 speaker timeline/embedding 输出；`check_diarization_cpu.py` 验证固定输入下的提速、资源与 baseline 一致性，speaker count 和一致性均不替代 DER。

- //TODO - 给 B2 补固定 UEM/collar/overlap 的精度 runner；完整 Audio export 不能代替独立 ASR/diarization 测试。
- //TODO - 各阶段在运行前冻结场景阈值与评分口径。低质量是 fail，不是已完成执行；不要临时放宽阈值。近似 WER/CER 不作为正式通过门，需精确分块/流式评分或明确不具备评分条件。
- //TODO - 自动发现可用模型、校验配置，按 case/model/配置哈希复用缓存；只重跑受改动影响的阶段。固定重复次数，保留波动范围与失败，支持断点续跑。
- //TODO - B4 前先满足 B0–B3 所需基线与 Host 功能门；远程调用使用预置调用数/token/时长上限，有限重试，超限或缺凭据自动标记 blocked 并退出，不等待用户评审、不悄悄切换模型。相关本地失败不触发更昂贵的下游运行。
- //TODO - 变更时先运行本地小集；定期完整本地集与昂贵任务由显式配置的执行策略触发，不在本次文档工作创建后台监控。新增模型先过 smoke 再进入可比较矩阵。

## 模型比较报告

`report_asr.py` 已从逐样本原始预测自动生成[模型比较](../quality/model-comparison.md)与机器 receipt：按语言和原生/构造/AMI 场景统计，保留失败。评分器统一重算原样 CER/WER 与单独列出的英文 normalized WER，不混入时间戳、DER 或语义分数。

- //TODO - 按 B2–B5 的独立参考与门槛扩展报告，未测项明确标为未评估。
- 每行至少包含 benchmark/case 版本、场景、语言、时长/样本数、模型/backend/revision、设备/线程、冷/热、质量指标、wall/RTF、峰值 RAM/VRAM、失败率；Agent 另列调用数/token 与可获得的费用，未提供费用标 unknown。
- 同时输出逐 case 明细、分组均值/中位数/尾部与最差样本，说明加权方法、参考/评分版本和重复波动；不同硬件、输入或评分口径不得直接排名。speaker ID 允许置换，不能要求预测标签字符串等于 gold。
- 分析回答“哪个模型适合哪类内容/时长、质量与速度取舍、哪个原子环节退化”；不只给全局平均或跨任务单一总分。失败/unsupported/blocked 同样展示，链接可复现命令与脱敏结果。
- 确定性比较不需要人工签收；已有结果不足时报告缺项，不用历史 smoke 替代新 benchmark 分数。

## 当前进度与 TODO

2026-10-25 至 2026-10-26 本机真实执行，范围与结果见[研究记录](../reserch/asr-quality-and-runtime-evaluation.md)。

| 交付物 | 当前状态 | 下一步 |
|---|---|---|
| B0 素材/评分器 | FLEURS 中英文各 24 个原生句子、构造 clean/noise、后续 24 个中文句子；AMI 五个非重叠会话裁剪；固定 hash、版本与精确评分正反例 | 增加自然中文会议、远场与重叠 gold，按会议/speaker 隔离 |
| B1 模型比较 | Paraformer、SenseVoice、Whisper large/base/Turbo、Qwen MLX 已真实比较；CPU/Metal、线程与加载策略分别记录；Unicode 分段接入保留这组真实输出，Silero VAD 裁剪因英文/低音量退化拒绝默认集成，见[边界回执](../quality/asr-boundary-evaluation.json) | 扩大场景，添加可靠词时间参考与时间误差 |
| B2 diarization | 独立 release runner 记录执行、完整输出与进程资源；CPU 线程对照通过 baseline 输出等价和资源门，计数与等价不替代 DER | 固定 UEM/collar/overlap，计算 DER 分项，验证模型与聚类取舍 |
| B3 窗口/运行 | 已复现并修复 Whisper 回调、子词边界、设备回退与推理粒度问题；短路径和应用窗口分别核对 | 验证小时级资源、取消延迟、恢复和身份漂移；AHC 的二次内存仍需单独治理 |
| B4/B5 | 已有 Agent runner 与历史基础；本轮未开展语义和完整误差传播验收 | 使用固定 gold 经 Host 调用，逐层替换预测定位损失 |

现有脚本位于 `scripts/audio-eval/` 和 `scripts/assistant-eval/`；重用而不另造产品 runtime。历史下载、smoke 和复合样本评分是已有基础，不表示专属 benchmark 或任何阶段已全部通过。
