# 架构对齐实施计划

## 当前进度

MeetNote 已完成第一轮四层职责收敛。当前按所属模块分支推进：Assistant 收口最小 Loop 与供应商接入；Workspace 可靠性、UI 与音频质量分别验收，不建设完整 Agent Runtime、外部 Agent 接入或插件框架。现阶段的实现状态如下：

| 区域 | 当前进度 | 下一步 |
|---|---|---|
| Workspace | `WorkspaceCapabilityPort` 已承载 typed tools、scope、revision、Markdown hash、分页快照、batch/journal/pending/undo、repository lock 和 512 KiB result cap；UI 与 Assistant 共用服务 | Workspace 分支补锁内内容检查、普通保存 CAS/恢复与读取上限；不归 Assistant 分支建设 |
| Audio | 解码、重采样、离线分窗、ASR/diarization provider、模型管理和多个 backend 已存在；处理由 UI 与 `meetnote process` 发起 | 真实模型质量、speaker identity 阈值和更多 backend 矩阵仍待验收 |
| Assistant | 最小 Tool Loop、三协议 adapter、ChatGPT backend、Skill、JobRuntime worker gate、progress、warning 与多 receipt 有测试；供应商目录、凭据、OAuth 登录、模型列表与额度已实现，HTTP 测试均为 loopback stub | 真实供应商验证与桌面端到端是发布门，见 [Assistant 计划](assistant-plan.md)；Loop 向 pi-agent-core 收敛，不增加第二套 runtime |
| 文档场景 | ask、organize、partition、extract、move 的 Skill 与工具校验已分离；Workspace 负责事实、结构和写回 | 跨块语义质量依赖真实模型评测（B4） |
| Rune UI | 六个 route（含设备页与供应商页）在 Rune 上实现（`apps/meetnote`），具备文档编辑/恢复反馈、模型管理、Assistant review、键盘与快捷键流程，并有 Runtime 交互测试 | 原生窗口、Accessibility Inspector 与真实 provider 桌面验收见 UI 计划 |

这些状态描述当前代码、已确定的设计和待实现能力，不等同于生产验收。测试存在、编译通过、真实模型执行、质量验收和发布就绪必须分别记录。

## 目标与保留项

保留以下已有基础：

- `document.md + state.json` 文件事实、目录扫描、三层树和外部修改报告。
- `WorkspaceService`、`DocumentCommandService`、revision 检查和文档包原子写入。
- Audio 的解码、窗口规划、ASR/diarization registry、manager、provider 和具体 backend。
- 模型 catalog、下载校验、probe、`JobRuntime` 和 Speaker Library 身份匹配。
- 文档命令的 schema、scope、revision 和结构校验；Assistant 工具 schema、Skill、幂等、pending、provenance、undo 和 LLM 协议 adapter 复用这些边界。

需要调整的是职责，而不是这些能力本身：

- Agent Host 只承载内置 Assistant 的 LLM 供应商与最小 Tool Loop；不实现完整 Agent Runtime。
- Workspace 负责文档读取、创建、patch、树关系、范围校验、revision、事务和恢复。
- Audio 负责音频事实、处理编排和 provider；Assistant 不调用音频处理。
- profile/Skill、文档 context 和 Map/Reduce 属于场景层，不进入 Loop。

## 需要修正的规划判断

- 文档工具执行、写入校验和事务归 Workspace；Agent Host 复用 capability 的小型 envelope，不建立第二套格式；Skill 负责语义。
- MeetNote 不提供跨 backend 会话 create/resume/close。Assistant 只保存本次运行必需消息，持久会话不是当前前置能力。
- 非流式 HTTP 是当前通用协议路径的实现选择，ChatGPT backend 已使用 SSE；不是所有供应商的长期限制。统一事件可以承载完整消息，也可以承载真实流式事件，但不能把完整文本切片伪装成流式。
- 主程序保持 Rust/Rune 和原生音频链路；外部进程或云端音频 provider 是可选执行载体，必须单独管理取消、退出、超时、资源和数据传输。
- 不使用权威 SQLite 或全局 project store；这不意味着删除 state、事务 journal、undo 或会话历史。它们分别属于事实、恢复和操作历史，不能都当作缓存。

## 分支职责与依赖

- Assistant：最小 Loop、供应商接入与真实供应商验证，详见 [Assistant 计划](assistant-plan.md)。
- Workspace 基础服务：资产、CAS、恢复与读取上限的唯一进度/TODO 归 [Workspace Host](../architecture/workspace-host.md)；Settings 基础服务由工程计划协调。
- Audio 基础服务：任务执行与跨进程生命周期归工程计划；AMI ASR/diarization 边界优先由[音频质量计划](audio-quality-plan.md)执行。
- Rune UI：所有页面、设置界面、结果/错误渲染和交互验收归[UI 计划](ui-plan.md)。
- 质量按固定 benchmark 的本地原子测试、组合测试、Assistant 语义与端到端逐级执行；三协议完整语义矩阵不是 Assistant 分支验收门。

## 当前缺口

- 外部 Markdown 编辑不会参与受管理事务；owner/liveness 的发布级证明和更广泛的外部编辑边界仍需独立验收。
- 真实上游模型语义质量、长音频 AHC 的二次内存、speaker identity 阈值、质量基准和 macOS 发布验收仍是独立工作。
- 所有供应商 HTTP 测试均为 loopback stub，尚无真实供应商验证；桌面异步交互归 UI 计划，完整语义质量不作为当前 Assistant 门槛。

## 最小闭环的验收归属

1. 六个文档工具的执行、范围、validator、写回策略和固定错误码归 Workspace capability；Skill 不进入 capability wire。
2. Assistant 有限工具循环进入 JobRuntime worker gate；阶段进度、取消、终态和 receipt 不另造调度器。
3. ask、organize、partition、extract、move 的读取、写入、错误修正、confirm_each、source hash、receipt 和 undo 均有 application 测试。
4. 供应商连接、登录、额度与凭据存储以 loopback stub 测试，真实供应商结果以回执单独记录。

底层可靠性、UI、音频质量和发布分别由所属计划负责。

## 后续验证门槛

- Workspace：继续验证外部 Markdown/owner/liveness 边界；现有跨进程 lock、journal、附件和 undo 回归不重复建设。
- Assistant：真实供应商连接、ChatGPT/OpenRouter 登录与额度、桌面端到端任务，方法见 [Assistant 计划](assistant-plan.md)。LLM HTTP stub 测试与真实供应商结果分别记录。

工程资源、模型质量和发布继续按[工程计划](implementation-plan.md)验收，文档 Assistant 场景继续按[Assistant 计划](assistant-plan.md)验收。
