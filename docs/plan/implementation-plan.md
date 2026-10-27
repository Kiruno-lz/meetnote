# 工程实施计划

## 当前阶段

MeetNote 已有 Rust-native 核心分层和主要音频处理链路，当前进行职责对齐与生产化收敛；架构迁移顺序见[架构对齐计划](architecture-alignment.md)。

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
| GPUI 文档、模型、设置和处理界面 | zui/gpui-base 页面与键盘流程已实现，自动与离屏验收见 UI 计划；原生窗口和无障碍发布检查待手验 |
| Assistant / external Agent | 工作树已有工具、Skill、adapter 和 loop；本分支收口三服务 capability 与无 UI 调用验收，UI/音频质量分别归对应计划 |
| Capability / Extension 生命周期 | 插件共享 port 与生命周期待收敛；不建设完整 Agent Runtime |
| 长音频资源、精度、故障与发布验收 | 音频质量矩阵计划已建立；已有数据下载/转换、Audio Host export、WER/CER/DER 和复合边界评分工具；导出到评分器的协议回归已通过，冻结矩阵与真实模型质量/资源验收仍待执行 |

基础验证：

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
```

评测数据集、下载 receipt 与评测输出位于仓库根 `eval-data/`（gitignored）：音频评测在 `eval-data/audio-quality/`，Assistant 评测在 `eval-data/assistant/`。`target/` 只含可重建的构建产物，不是 MeetNote 的运行时数据源，也没有 `.db`、SQLite 或 SQL 数据库文件；确认没有正在运行的测试或构建任务后可直接 `cargo clean`。

真实模型验收由显式 `#[ignore]` 测试执行。真实数据的下载、reference 转换和矩阵定义见[音频质量矩阵计划](audio-quality-plan.md)。协议回归使用固定 fixture 验证导出、评分与语义质量边界；真实模型执行、WER/CER/DER 质量、speaker 数与长音频资源必须由对应音频、模型和运行 receipt 单独证明。

## P0：生产正确性与资源边界

### 长音频资源

- 长音频已通过所属 `JobContext` 共享 worker gate 按录音取得名额；单名额回归覆盖排队、不提前加载模型、任一阶段排队取消和模型创建 panic 的错误分类。//TODO - 用固定真实模型验证多录音并行时的耗时、峰值 RSS 和 native runtime 稳定性；名额数不能作为推理线程总预算。
- 限制 recording-wide clustering 输入规模；保留原始 observation 供重建与验证，但不能让 AHC 的二次距离数组随整段录音无限增长。
- 使用真实 20 小时代表性音频记录耗时、峰值 RSS、observation 数和临时存储。
- 执行取消、解码失败、模型失败、磁盘失败和进程中断注入，验证错误可见性与重新处理安全。

### 文档一致性

- Workspace 资产生命周期、CAS、journal/恢复与读取上限的唯一进度/TODO 见 [Workspace Host](../architecture/workspace-host.md)；由 Workspace 分支交付，不重复列入 Assistant 接入计划。
- 为源音频、Markdown、项目 JSON 和 Speaker Library 固化不可删除资产边界；为全文索引、embedding index 和搜索缓存定义 stale、删除与重建测试。
- 所有 UI 写回统一使用 revision-aware document service。
- 在正常文档工作区展示 scanner issue 的路径和原因。
- 校验子文档时间范围包含关系、稳定 ID 格式和结构化内部链接。

P0 完成条件：长音频受控并发和资源上限可证明；任意写入失败不会留下无法解释的文档树状态。

## P1：产品闭环

- 将文档根目录加入 settings，支持选择并恢复目标工作区。
- 资产生命周期依赖 [Workspace Host](../architecture/workspace-host.md)；本计划只安排其产品集成，不重复维护托管/恢复任务。
- 完成标题、标签、总结、Markdown、`document_status`、revision 和 provenance 的查看/编辑 UI。
- 完成 Speaker Library 候选确认、enrollment、样本追加、profile merge/split 与证据查看。
- 提供首批 Workspace capability port，让插件通过受控接口访问读取、搜索、索引构建和订阅能力；不开放任意文件系统操作。
- 按[Assistant 计划](assistant-plan.md)收口三服务 capability、外部薄入口与最小 Built-in loop；不要求该分支完成 UI、底层恢复、音频质量或完整模型语义矩阵。
- //TODO - Audio 跨进程执行基础设施复用统一 JobRuntime：明确状态所有者、配置快照、幂等、进度/结果、取消竞争、worker 崩溃与清理；避免 CLI 文件状态与内存 registry 平行演进。接口越权修复仍由 Assistant 接入计划负责。
- Settings 基础服务已实现跨进程 CAS、owner-only 设置/凭据文件、原子替换与失败恢复测试；Agent 复用配置/授权契约。
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
