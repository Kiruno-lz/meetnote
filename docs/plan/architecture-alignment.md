# 架构对齐实施计划

## 当前进度

MeetNote 已完成第一轮四层职责收敛和 capability 初步闭环。当前按所属模块分支推进：Assistant 只收口 capability 与最小 Loop；Workspace 可靠性、UI 与音频质量分别验收，不建设完整 Agent Runtime 或通用插件框架。现阶段的实现状态如下：

| 区域 | 当前进度 | 下一步 |
|---|---|---|
| Workspace | `WorkspaceCapabilityPort` 已承载 typed tools、scope、revision、Markdown hash、分页快照、batch/journal/pending/undo、repository lock 和 512 KiB result cap；CLI/Built-in 共用服务 | Workspace 分支补锁内内容检查、普通保存 CAS/恢复与读取上限；不归 Agent 接入建设 |
| Audio | 解码、重采样、离线分窗、ASR/diarization provider、模型管理和多个 backend 已存在；`AudioCapabilityPort` 已接入 grant、job、snapshot、cancel 和 envelope | 真实模型质量、speaker identity 阈值和更多 backend 矩阵仍待验收 |
| Built-in Agent | 已收缩为最小 Tool Loop，三协议 adapter、Skill、JobRuntime worker gate、progress、warning、多 receipt 和 UI 终态均有测试 | 真实 provider/GPUI host 集成仍是发布门，不增加第二套 runtime |
| 文档场景 | ask、organize、partition、extract、move 的 Skill 与工具校验已分离；Workspace 负责事实、结构和写回 | 跨块语义质量依赖外部 Agent/场景插件和真实模型评测 |
| 外部 Agent | JSON capability CLI 已覆盖读取、分页、写回、source hash、confirm_each、undo、Settings、Audio discovery/error envelope；Codex CLI 已真实完成 discovery、`read_document` metadata 和临时 workspace 无预览自动写回/mutation receipt bridge | 外部工具桥接按能力验收；不要求 MeetNote 托管 session/context/loop |
| GPUI UI | 四个 route 已迁移到 zui/gpui-base，具备玻璃外壳、文档编辑/恢复反馈、模型管理、Assistant 状态、键盘遍历与带 label 控件 | 原生窗口、Accessibility Inspector 与真实 provider 桌面验收见 UI 计划 |
| Extension | 当前采用显式 Rust 装配，不建设通用 manifest、依赖解析或 lifecycle runtime | 只有出现真实动态 provider 才验证注册撤销、依赖失败回滚和在途任务停止 |

这些状态描述当前代码、已确定的设计和待实现能力，不等同于生产验收。测试存在、编译通过、真实模型执行、质量验收和发布就绪必须分别记录。

## 目标与保留项

保留以下已有基础：

- `document.md + state.json` 文件事实、目录扫描、三层树和外部修改报告。
- `WorkspaceService`、`DocumentCommandService`、revision 检查和文档包原子写入。
- Audio 的解码、窗口规划、ASR/diarization registry、manager、provider 和具体 backend。
- 模型 catalog、下载校验、probe、`JobRuntime` 和 Speaker Library 身份匹配。
- 文档命令的 schema、scope、revision 和结构校验；未来 Agent tool schema、Skill、幂等、pending、provenance、undo 和 LLM 协议 adapter 应复用这些边界。

需要调整的是职责，而不是这些能力本身：

- Agent Host 以 Capability API 为核心，只负责能力绑定、薄适配和 Built-in 最小 Tool Loop；不实现完整 Agent Runtime。
- Workspace 负责文档读取、创建、patch、树关系、范围校验、revision、事务和恢复。
- Audio 负责音频事实、处理编排和 provider；Agent 只能通过显式能力调用它。
- profile/Skill、文档 context 和 Map/Reduce 属于场景层，不进入通用 Host。
- Extension 负责注册、依赖、生命周期和撤销，不拥有文档事实或音频算法。

## 需要修正的规划判断

- 文档工具执行、写入校验和事务归 Workspace；Agent Host 复用 capability 的小型事件 envelope，不建立第二套格式；Skill/可选场景插件负责语义。
- 外部 Agent 自行管理会话与上下文；MeetNote 不提供跨 backend create/resume/close。Built-in 只保存本次运行必需消息，持久会话不是当前前置能力。
- 非流式 HTTP 是当前 Built-in 路径的实现选择，不是所有 Agent backend 的长期限制。统一事件可以承载完整消息，也可以承载真实流式事件，但不能把完整文本切片伪装成流式。
- 主程序保持 Rust/GPUI 和原生音频链路；外部进程或云 provider 是可选执行载体，必须单独管理取消、退出、超时、资源和数据传输。
- 不使用权威 SQLite 或全局 project store；这不意味着删除 state、事务 journal、undo 或会话历史。它们分别属于事实、恢复和操作历史，不能都当作缓存。

## 分支职责与依赖

- Assistant：完整 capability 接口化、Host 真实回应、Codex（Luna medium）经 Host/测试插件完成任务；MoonBit 真实插件与红线测试是当前验收要求，详见 [Assistant 计划](assistant-plan.md)。
- Workspace 基础服务：资产、CAS、恢复与读取上限的唯一进度/TODO 归 [Workspace Host](../architecture/workspace-host.md)；Settings 基础服务由工程计划协调。
- Audio 基础服务：任务执行与跨进程生命周期归工程计划；AMI ASR/diarization 边界优先由[音频质量计划](audio-quality-plan.md)执行。
- GPUI：所有页面、设置界面、结果/错误渲染和交互验收归[UI 计划](ui-plan.md)。
- 质量按固定 benchmark 的本地原子测试、组合测试、Agent 语义与端到端逐级执行；三协议完整语义矩阵不是 Agent Host 分支验收门。

## 当前缺口

- 外部 Markdown 编辑不会参与受管理事务；owner/liveness 的发布级证明和更广泛的外部编辑边界仍需独立验收。
- 真实上游模型语义质量、长音频 AHC 的二次内存、speaker identity 阈值、质量基准和 macOS 发布验收仍是独立工作。
- 外部 Agent 工具调用与 fake 测试分层验证；GPUI 异步交互归 UI 计划，完整语义质量不作为当前 Agent 接入门槛。
- Extension 当前仍以 Rust 装配为主；先实现明确要求的 MoonBit 测试插件与 Host 桥接，不扩成通用动态插件平台。

## 最小闭环的验收归属

以下为当前已有基础与目标边界，不代表三服务实际接入全部完成：

1. 五种文档工具的执行、授权、validator、写回策略和固定错误码归 Workspace capability；Skill 不进入 capability wire。
2. Workspace/Audio/Settings 有版本化描述与 envelope；外部 CLI、Rust consumer 和 Built-in 使用同一服务 port。
3. Built-in 有限工具循环进入 JobRuntime worker gate；阶段进度、取消、终态和 receipt 不另造调度器。
4. ask、organize、partition、extract、move 的读取、写入、错误修正、confirm_each、source hash、receipt 和 undo 均有 application/CLI 测试。

Workspace 的既有闭环不能替代真实 Audio/Settings payload 接入；按 Assistant 计划完成无 UI 三服务、授权与 Loop 验收。底层可靠性、UI、音频质量和发布分别由所属计划负责。

## 后续验证门槛

- Workspace：继续验证外部 Markdown/owner/liveness 边界；现有跨进程 lock、journal、附件和 undo 回归不重复建设。
- 外部 Agent：验证 Codex/Claude Code 的真实工具桥接、授权、错误修正和副作用回读；不要求 MeetNote 托管其 session。LLM HTTP 测试与外部工具集成分别记录。
- Extension：只有真实 provider 需求出现时，才验证注册所有权、依赖失败回滚、在途任务停止和注销。
- Component：MoonBit 测试插件先验证最小可执行桥接；仅在选定载体确有需要时定义相应 ABI/WIT，不一次建设通用组件升级系统。

工程资源、模型质量和发布继续按[工程计划](implementation-plan.md)验收，文档 Agent 场景继续按[Assistant 计划](assistant-plan.md)验收。
