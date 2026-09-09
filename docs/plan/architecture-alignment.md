# 架构对齐实施计划

## 当前进度

MeetNote 已有 Workspace 和 Audio 的主要基础能力，GPUI UI 已提供桌面交互骨架，Agent Host 仍处于设计和早期实现规划阶段。当前工作重点是收敛四个基础层的职责边界，而不是重新搭建一套框架。现阶段的实现状态如下：

| 区域 | 当前进度 | 下一步 |
|---|---|---|
| Workspace | `WorkspaceService`、文档命令、文件包、扫描和监听已经存在 | 统一资产生命周期、UI/CLI/Agent/音频写入、可重建索引和插件 capability port |
| Audio | 解码、重采样、离线分窗、ASR/diarization provider、模型管理和多个 backend 已存在 | 保持原生 provider 结构，补齐长音频资源与故障验收 |
| Built-in Agent | Assistant 仍是占位客户端；Agent runtime、文档 context、工具、Skill 和 LLM adapter 已完成方向设计 | 先实现最小内置会话，再接入统一 JobRuntime 与类型化事件 |
| 文档场景 | ask、organize、partition、extract、move 等场景和工具校验已定义，当前由文档命令提供基础能力 | 将工具执行与校验明确归 Workspace，Skill 只负责场景语义 |
| Agent backend | 当前没有统一 Agent backend；Codex、Claude Code 和 Built-in 均待接入 | 先定义最小 create/input/cancel/event 契约，再验证外部 backend |
| GPUI UI | 桌面入口、页面、controller 和基础状态投影已经存在 | 作为三个 Host 的客户端，接入统一事件、错误和任务状态 |
| Extension | 尚无通用 manifest、依赖解析或生命周期运行时 | 先用内置 Rust 装配验证服务注册与撤销，再决定是否需要扩展载体 |

这些状态描述当前代码、已确定的设计和待实现能力，不等同于生产验收。测试存在、编译通过、真实模型执行、质量验收和发布就绪必须分别记录。

## 目标与保留项

保留以下已有基础：

- `document.md + state.json` 文件事实、目录扫描、三层树和外部修改报告。
- `WorkspaceService`、`DocumentCommandService`、revision 检查和文档包原子写入。
- Audio 的解码、窗口规划、ASR/diarization registry、manager、provider 和具体 backend。
- 模型 catalog、下载校验、probe、`JobRuntime` 和 Speaker Library 身份匹配。
- 文档命令的 schema、scope、revision 和结构校验；未来 Agent tool schema、Skill、幂等、pending、provenance、undo 和 LLM 协议 adapter 应复用这些边界。

需要调整的是职责，而不是这些能力本身：

- Agent Host 只负责 session、输入、取消、backend 能力和归一化事件。
- Workspace 负责文档读取、创建、patch、树关系、范围校验、revision、事务和恢复。
- Audio 负责音频事实、处理编排和 provider；Agent 只能通过显式能力调用它。
- profile/Skill、文档 context 和 Map/Reduce 属于场景层，不进入通用 Host。
- Extension 负责注册、依赖、生命周期和撤销，不拥有文档事实或音频算法。

## 需要修正的规划判断

- 文档 Agent 的网关不能同时成为通用 Agent Host。工具执行、写入校验和事务属于 Workspace；会话循环和 backend 连接属于 Agent Host；Skill 只描述场景。
- 只读问答不产生文档事实，但会话是否持久化应由 backend 能力和产品策略决定，不能用“只读”推导出永不恢复。
- 非流式 HTTP 是当前 Built-in 路径的实现选择，不是所有 Agent backend 的长期限制。统一事件可以承载完整消息，也可以承载真实流式事件，但不能把完整文本切片伪装成流式。
- 主程序保持 Rust/GPUI 和原生音频链路；外部进程或云 provider 是可选执行载体，必须单独管理取消、退出、超时、资源和数据传输。
- 不使用权威 SQLite 或全局 project store；这不意味着删除 state、事务 journal、undo 或会话历史。它们分别属于事实、恢复和操作历史，不能都当作缓存。

## 已知缺口

- `WorkspaceService`、`DocumentCommandService` 和 Agent 工具目前存在多个写入入口，需要统一校验、提交和冲突处理，不新增同义 facade。
- Agent 写入后，如果后续模型请求失败，UI 必须保留 mutation receipt、usage、diagnostics 和 undo，而不能把整个任务表现为无副作用失败。
- Agent runtime 尚未完整接入统一 JobRuntime；阶段进度、取消和 worker gate 需要与音频任务采用同一调度语义。
- 文档事务的跨进程互斥、journal 恢复所有权、普通附件保留和外部编辑冲突仍需验证，实例内锁不能被当作工作区级互斥。
- 真实模型语义质量、长音频 AHC 的二次内存、质量基准和 macOS 发布验收仍是独立工作。

## 最小连贯的第一步

后续实现先完成一个内置 Agent 到 Workspace 的闭环：

1. 将五种文档工具的执行、授权、validator 和提交策略收敛为 Workspace capability；保持现有参数、错误码和结果语义。
2. 为 Built-in runtime 提供最小 session、input、cancel 和 normalized event 接口；仅声明已经真实支持的 resume 和 streaming 能力。
3. 将一次会话接入 `JobRuntime`，让桌面和 CLI 复用同一 Workspace 服务，不新增第二套任务状态机。
4. 用 ask 和 organize 验证读取、写入、取消、错误修正、receipt 与 undo；写入后的模型失败不得隐藏已产生的副作用。

完成条件：fake backend 能验证 session/run 关联、取消和单一终止事件；内置与 CLI 对同一文档产生一致 revision 和错误；Workspace 不启动 Agent 也能独立读写；已有工具、LLM、文档和恢复测试通过。

## 后续验证门槛

- Workspace：验证跨进程冲突、直接编辑 Markdown、普通附件、各提交阶段故障恢复和 undo。
- Agent backend：验证 Codex 或 Claude Code 的真实 create/input/cancel、工具桥接、能力缺失和会话恢复行为；LLM HTTP adapter 不能代替该证据。
- Extension：用一个真实 provider 验证注册所有权、依赖失败回滚、在途任务停止和注销；没有实际需求时不建设通用插件框架。
- Component：只有在扩展契约稳定且存在适合隔离的真实能力后，再验证 WIT、资源 handle、版本、错误和升级语义。

工程资源、模型质量和发布继续按[工程计划](implementation-plan.md)验收，文档 Agent 场景继续按[Assistant 计划](assistant-plan.md)验收。
