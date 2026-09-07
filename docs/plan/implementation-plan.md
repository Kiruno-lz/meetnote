# 工程实施计划

## 当前阶段

MeetNote 已完成 Rust-native 核心架构和主要纵向处理链路，当前处于生产化前的功能收敛阶段。

| 能力 | 状态 |
|---|---|
| Rust workspace、分层与 GPUI 入口 | 已完成 |
| 文档包、目录扫描、三层树和文件监听 | 已完成基础实现 |
| ASR/diarization registry、manager 与 provider | 已完成 |
| Whisper、Paraformer、sherpa diarization、native diarization | 已实现 |
| 模型 catalog、下载恢复、校验和三层 probe | 已完成基础实现 |
| Speaker Library 匹配、候选、enroll、merge、split | application 已实现 |
| 短音频整文件 pipeline | 已完成 |
| 长音频离线分窗与 native 全局聚类 | 已完成技术链路 |
| GPUI 文档、模型、设置和处理界面 | 功能脚手架 |
| Assistant / external Agent | 只有内部边界和占位客户端 |
| 长音频资源、精度、故障与发布验收 | 未完成 |

基础验证：

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
```

真实模型验收由显式 `#[ignore]` 测试执行。当前 Paraformer + native diarization 分窗链路已在约 3 分钟中文音频上运行，证明模型与结构可执行；无人工 transcript/RTTM，不能据此声称 WER/CER、DER 或 speaker 数正确。

## P0：生产正确性与资源边界

### 长音频资源

- 将长音频阻塞工作纳入 `JobContext` 共享 worker gate，覆盖并发任务测试。
- 限制 recording-wide clustering 输入规模；保留原始 observation 供重建与验证，但不能让 AHC 的二次距离数组随整段录音无限增长。
- 使用真实 20 小时代表性音频记录耗时、峰值 RSS、observation 数和临时存储。
- 执行取消、解码失败、模型失败、磁盘失败和进程中断注入，验证错误可见性与重新处理安全。

### 文档一致性

- 为 `state.json + document.md` 提供文档包级提交或恢复协议。
- 为 reparent/reorder 等多文档写回提供原子批次、journal 或可验证回滚。
- 所有 UI 写回统一使用 revision-aware document service。
- 在正常文档工作区展示 scanner issue 的路径和原因。
- 校验子文档时间范围包含关系、稳定 ID 格式和结构化内部链接。

P0 完成条件：长音频受控并发和资源上限可证明；任意写入失败不会留下无法解释的文档树状态。

## P1：产品闭环

- 将文档根目录加入 settings，支持选择并恢复目标工作区。
- 明确音频资产是外部引用还是项目托管，并实现缺失文件检测与恢复入口。
- 完成标题、标签、总结、Markdown、`document_status`、revision 和 provenance 的查看/编辑 UI。
- 完成 Speaker Library 候选确认、enrollment、样本追加、profile merge/split 与证据查看。
- 按[Assistant 构建计划](assistant-plan.md)接入 LiteLLM、三类协议 adapter 和外部 Agent adapter；内置与外部 Agent 复用同一 tool、validator 和 document application 边界。
- Agent 写回默认自动执行，并可在 settings 切换为逐次确认；两种模式都不能修改 processing/results 所有权字段。
- 将 identity matching 实际阈值、模型 license 和来源快照写入处理 provenance。

P1 完成条件：用户可以从选择工作区、导入音频、安装模型、处理、语义整理、人工校正到重启恢复，完成一个不依赖开发工具的闭环。

## P2：质量与发布

- 建立中文、英文、混合语音的 WER/CER 数据集。
- 建立含重叠语音、已知 speaker 数和身份标注的 DER/identity 数据集。
- 校准 Speaker Library 阈值、分布下限、歧义 margin 和误合并率。
- 测量 CPU、Metal/CoreML 的冷启动、实时率、峰值内存和能耗。
- 完成 macOS bundle、签名、notarization、模型与第三方 license notice。
- 在全新用户环境验证安装、模型下载、失败恢复、升级和卸载。

P2 完成条件：质量、资源、安全恢复和发布证据同时满足，才能标记生产就绪。

## 完成标准

- 文档目录可在无缓存、无数据库条件下重建完整三层项目树。
- Assistant、外部 Agent 与 UI 使用相同的版本、权限和结构校验。
- 所有可选 backend 具有独立 probe、合成测试和真实模型测试。
- 长音频 20 小时处理有可复现的资源、取消、故障和精度结果。
- 处理 provenance 足以定位输入、模型、artifact、stage、算法和运行参数。
- 发布产物经过签名、notarization、license 与全新环境验收。
