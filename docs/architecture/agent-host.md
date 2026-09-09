# Agent Host

Agent Host 是 MeetNote 四个基础层之一；它通过 Workspace 和 Audio 的显式 capability 工作，不能替代这两个 Host。

## 职责

Agent Host 是通用 Agent backend 的会话运行边界。初始目标 backend 为 Built-in Agent、Codex 和 Claude Code；不按日记、会议或课程创建专用 runtime。

Host 管理 session ID、backend session 映射、workspace 关联、创建/恢复/关闭、发送输入、取消、运行状态、能力描述与归一化事件。backend 负责自身协议与进程/连接生命周期。JobRuntime 负责一次运行的调度、进度和受控 worker；会话可以跨多次运行存在，两者不能混为一个 ID 或互相复制调度逻辑。

## 与现有代码的关系

当前代码提供 `llm` provider 描述边界、文档 application 服务和 Assistant 占位客户端；通用 `AgentRuntime`、AgentGateway、JSON 工具 CLI、三种 LLM adapter 与桌面 Agent 会话仍属于待实现能力。本节规定它们未来的职责，不能当作当前 API。

- LLM provider 和协议 adapter 属于 Agent Host 的模型连接层；调用 Claude 模型不等于运行 Claude Code，外部 Codex 调用 MeetNote CLI 也不等于 MeetNote 已托管 Codex session。
- Agent runtime 需要承担 loop、取消、usage 和阶段进度，但不应把 `AgentScene` 固定成所有 backend 的通用领域枚举；context 与 Map/Reduce 属于文档场景层。
- Agent gateway 未来可以汇集文档工具、Skill、scope 与调用协调，但文档执行与 validator 必须归 Workspace；Skill 选择与场景上下文归 profile/场景层。
- 当前没有通用 create/resume session 或跨 backend 流式事件协议；统一事件只能声明 backend 真实支持的能力。

## 目标契约

以下为待实现的语义要求，不固定 Rust 名称或外部 wire schema：

| 操作/数据 | 要求 |
|---|---|
| backend descriptor | backend/model 标识、可用性、恢复、输入、流式与工具能力；不支持的能力明确报告 |
| create / resume / close | host session 与 backend session 分离；恢复失败可定位，不能伪装为成功续接 |
| send input / cancel run | 每次输入关联 run；取消终止继续生成与新工具调用，已经提交的文档修改仍需返回 receipt |
| context / capability binding | 显式 workspace ID、profile/Skill 与服务授权；选择 Skill 不扩大授权 |
| normalized events | session/run ID、递增序号、状态、输出、工具请求/结果、usage、错误与终止；无法归一化的 backend 信息留明确 metadata |

先支持内置 backend 能真实提供的事件；不支持 token streaming 时可以发送完整消息事件。不得通过切分完整文本伪造实时流。消费端慢、取消与终止后不再发事件的行为属于契约验收。

如需重启恢复，记录可版本化的会话元数据和追加事件历史；最初只持久化实际支持的恢复信息。日志不是确定性重放承诺，重放不能再次提交写工具；模型输出、远程 session 和副作用不能仅靠事件重新生成。凭据不进入事件正文，历史保留策略需与用户数据一起定义。

## 能力与场景组合

```text
backend + profile/config + Skills + granted tools + output schema
  -> 文档问答 / 整理 / 日记 / 课程等场景
  -> Workspace 或 Audio 服务
```

文档 context projection、segment 选取、语义 Map/Reduce、摘要与拆分策略不进入 Host。保留现有五种 Skill 与工具作为文档场景，固定 `AgentScene` 不成为所有 backend 和扩展的通用枚举。Rust argument 类型继续作为工具 schema 来源；领域 validator 继续裁决范围、来源和 revision。

内置与外部客户端复用 Workspace 操作与错误语义，wire adapter 可以不同。现有 JSON CLI 先保留原有工具名；`workspace.patch_document` 是能力归属示意，不要求立即重命名 `update_document`。

Codex/Claude Code 接入前验证其真实会话、取消与工具桥接协议，再决定 adapter。若 backend 具有直接文件权限，服务授权不能限制其操作系统访问；集成时必须区分工具受控模式与直接文件编辑模式，后者走 Workspace 外部编辑校验。

最小实施与验证见[架构对齐计划](../plan/architecture-alignment.md)，文档场景要求见[Assistant 计划](../plan/assistant-plan.md)。
