# Agent Host

Agent Host 的核心是 Capability API 的绑定与薄适配，不是完整 Agent 系统。MeetNote 不管理 Codex / Claude Code 的 Session、Context 和 Loop；外部 Agent 使用自己的运行能力经 Agent Host 调用 MeetNote capability；验收必须穿过 Host 分发层，不能用直调服务或独立 capability CLI 代替。Built-in 只提供有限 Tool Loop，Assistant 是其 GPUI 入口。

## 职责与依赖

Workspace、Audio、Settings 分别拥有具体操作、typed contract、validator 和结果。Agent Host 为消费者绑定已经授予的 capability，将模型 tool call 或外部 wire 请求映射到服务调用；不执行文档规则、音频算法或配置写入。插件通过同一 port 访问这些服务，核心服务不需要成为插件才能被调用。

一次 Built-in 输入只需当前运行消息、工具结果、有限轮次、取消和可用 usage。JobRuntime 负责长任务调度、进度、取消与受控并发；Host 不另建 worker 调度器。外部 Agent 不需要 MeetNote session 才能发现和调用能力。

## 最小规范化层

各 capability 服务拥有输入、输出、错误与事件 payload，复用小型公共关联 envelope：版本、request/call ID、能力标识、结果或错误；长任务包含 job ID。事件关联同一次调用，顺序与终止明确。

服务错误保持所属 contract 的固定 code：Workspace 使用 Rust `AgentErrorCode`，Audio/Settings adapter 分别使用 `AUDIO_*`/`SETTINGS_*` code（包括 Audio 授权拒绝 `AUDIO_ACCESS_DENIED`），并只在明确可重试的 revision、锁或运行时失败上标记 retryable；Host 不通过解析自然语言 message 判断修正策略。Audio/Settings descriptor 暴露 operation、side effect、long-running 和输入 schema 元数据。

Workspace port 已归一化的事件直接复用，Agent Host 不再转换成第二份业务事件格式。CLI/MCP 等 adapter 只做 wire 映射；LLM adapter 只归一化模型消息与 tool call。首版可以返回完整文本，不伪造 token streaming，也不为接工具而解析外部 Agent 的完整事件流。

授权由 Host 授予，Skill 和请求只能收窄能力。取消阻止后续调用，已经提交的 receipt 必须独立保留；失败、超时、解释生成失败都不能隐藏副作用。confirm/reject/undo 的校验与执行归所属服务。

## Built-in 与场景

Built-in 循环为：请求模型、解析工具调用、执行已授权 capability、回传结果，直到模型完成、轮次耗尽或取消。工具串行，错误修正有界，不建设通用 Harness、长期记忆、自动压缩、规划器、多 Agent 或跨 backend session/resume/replay。

LLM 模型层支持 Chat Completions、Anthropic Messages、OpenAI Responses；LiteLLM 是兼容连接路径，不是外部 Agent 的依赖。连接配置和 API key/API key file 归 Settings；Capability 对外 snapshot 脱敏 API key/API key file，不使用操作系统 credential store。

Skill/profile 描述语义目标；Workspace 提供投影、有界读取、语义结果提交和事实校验。Map/Reduce 等多步语义策略归外部 Agent 或可选场景插件，不进入核心 loop。文档、Audio、Settings 能力不依赖固定 AgentScene。

## 当前验收目标

三个目标分别验收：完整 capability 接口化、Host 最短输入得到真实可解析回应、Codex（Luna medium）经 Host/真实 MoonBit 测试插件调用 capability 并纠错完成任务。有效非 200 响应可证明连通/解析，但不能证明完整任务成功。语义拆分属于后续插件扩展；质量由渐进 benchmark 独立验收，不加入 Host 内核。

## 当前代码与调整

具体阶段、代码处置与验收见 [Assistant 计划](../plan/assistant-plan.md)，注册和载体边界见 [Extension](extension-system.md)。
