# Agent Host

Agent Host 承载内置 Assistant：LLM 供应商连接、最小 Tool Loop，以及 Loop 与 Workspace capability 的绑定。内置 Assistant 是 MeetNote 唯一的 LLM 消费者；MeetNote 不向 Codex、Claude Code 等外部 Agent 暴露 CLI、MCP 或插件桥接，也不建设完整 Agent Runtime。Assistant 页面是其 GPUI 入口。

## 职责与依赖

| 代码 | 职责 |
|---|---|
| `crates/llm` | 三种协议 adapter、`ApiProvider` HTTP 客户端、`vendors` 供应商目录、模型列表、登录与额度读取 |
| `crates/settings` | `assistant` 设置（供应商、模型选择、写回策略、轮次上限）与 `credentials.json` 凭据存储 |
| `application::ProviderService` | 供应商视图、保存/移除、模型选择、登录与登出、模型列表、额度、连接测试，以及为 Loop 构造客户端 |
| `application::AgentRuntime`、`WorkspaceAgentScene` | 最小 Tool Loop；scene 负责 Skill 选择、预读与工具收窄 |
| `application::WorkspaceCapabilityPort` | 文档工具的执行、范围、revision、写回策略、receipt 与固定错误 |

Workspace 拥有文档工具的 typed contract、validator 与 `AgentErrorCode`；Agent Host 只把模型 tool call 映射为 port 调用，不执行文档规则。工具只开放 Workspace 文档能力，不开放 Audio 处理或 Settings 写入。Assistant 写入的 provenance 记录 actor `assistant` 与 mutation ID；`AgentActor::ExternalAgent` 只为读取已有 `state.json` 中的历史 provenance 保留，不产生新写入。

一次运行只需当前消息、工具结果、有限轮次、取消和可用 usage。运行进入 JobRuntime worker gate，进度、取消与终态复用 JobRuntime，不另建 worker 调度器。

## 最小 Tool Loop

循环为：请求模型、解析工具调用、执行已授权的 Workspace 工具、回传结果，直到模型完成、`maxToolRounds` 耗尽或取消；`maxValidationRetries` 限制可重试工具错误的修正次数。工具串行执行。取消阻止后续调用，已提交的 receipt 必须保留；失败、超时与解释生成失败都不能隐藏副作用。automatic 与 confirm_each 是同一服务的两种提交策略，confirm/reject/undo 的校验与执行归 Workspace。

不建设通用 Harness、长期记忆、自动压缩、规划器、多 Agent 或会话 resume/replay。非流式 HTTP 是当前实现，首版返回完整文本，不把完整文本切片伪装成流式。Skill/profile 描述语义目标，Workspace 提供投影、有界读取、语义结果提交和事实校验；Map/Reduce 等多步语义策略不进入 Loop。

Loop 的收敛方向以 pi-agent-core（badlogic/pi-mono）的最小 agent loop 为参照。

- //TODO - 逐项对照 pi-agent-core 与 `AgentRuntime` 的消息状态、工具执行与参数校验、事件和取消语义，列出差异并由 owner 确认取舍；收敛后以 application 层 fake provider 测试验证轮次上限、取消、错误修正次数与 receipt 保留不变。

## 供应商

### 协议与客户端

`ApiProvider` 支持 Chat Completions、Anthropic Messages 与 OpenAI Responses。一个供应商可同时提供多种 API，每种 API 有独立 base URL（`llm::vendors::endpoints::Endpoints`）；Messages 默认用 `x-api-key` 鉴权，Chat 与 Responses 用 Bearer，自定义供应商的鉴权头作用于其全部 API。base URL 包含版本前缀（如 `https://api.openai.com/v1`），客户端在其后拼接 `/chat/completions`、`/messages`、`/responses` 与 `/models`。base URL 必须为 HTTPS（本机 loopback 例外），且不含凭据、query 与 fragment。

`Backend` 决定请求去向：`Standard` 访问供应商自身 API；`ChatGpt { account_id }` 以 ChatGPT 登录的 access token 访问 Codex backend `https://chatgpt.com/backend-api/codex`，只使用 Responses 协议的 SSE 流。

### 目录与模型

`llm::vendors::catalog` 内置供应商目录：每项声明种类（`vendor` 模型厂商 / `relay` 聚合转售 / `local` 本机服务）、是否 featured、区域及每个区域各 API 的 base URL、Responses 偏好、密钥页面、可用登录方式与余额来源；第一个区域为默认。各 API 的端点按 magpie `internal/provider/presets.go` 录入（magpie 的 Anthropic 根地址加 `/v1`），例如 DeepSeek、xAI、Vercel、oMLX 与 magpie 网关提供三种 API，OpenRouter、Kimi、Qwen、MiniMax、Ollama 提供 Chat 与 Messages，Anthropic 只提供 Messages。magpie 的 Gemini `generateContent` 与 System One 不接入：目录中没有只说 Gemini 协议的供应商（Google 预设本身是 OpenAI 兼容 Chat），System One 是路由决策接口而非对话 API。目录外的 OpenAI/Anthropic 兼容端点以 `custom-` 前缀的自定义供应商配置。

模型列表实时读取供应商每个 API 的 `/models`（相同 base 只读一次；Messages 列表使用 Anthropic 头与 `after_id` 分页），聚合为 `ListedModel { id, protocols }`：顺序取第一个成功列表的顺序，新 ID 追加在后；某个 API 的列表失败时视为该 API 提供全部模型，全部失败才报错；magpie 网关 `/v1/models` 返回的 `native_endpoints` 优先于各列表的结论。ChatGPT 账户只读取 Codex backend 的模型列表。

每个模型的请求 API 由 `endpoints::native` 决定（移植 magpie `Provider.Native`）：候选为供应商提供且列表指明服务该模型的 API；偏好 Responses 的供应商（ModelScope 对全部模型，OpenAI 与 magpie 网关对 GPT/Codex/o 系列）且候选含 Responses 时用 Responses；Claude 模型且候选含 Messages 时用 Messages；否则按 Chat、Responses、Messages 顺序取第一个候选。选择模型时解析出的 API 存入 `ModelChoice.protocol`；运行时该 API 仍可用则沿用，否则按规则重新决定。`ModelSelection.mode` 决定选择器提供的模型：`vendor` 为供应商列表，`picked` 为手选子集，`latestPerFamily` 为每个模型族的最新版本；`added` 中手填的模型 ID 在所有模式下提供。模型族规则移植自 magpie 的 family 规则：从 ID 去除版本、快照与 preview 后得到族名，尺寸与变体词保留在族名中。

### 设置与凭据

`settings.json`（`meetnote.settings.v2`）的 `assistant` 段：

```text
assistant
├── enabled、writeMode、maxToolRounds、maxValidationRetries
├── model?: { provider, model, protocol? }     # Assistant 当前使用的供应商、模型与 API
└── providers: [ { id, enabled, region?, custom?: { name, authMode },
                   endpoints { chat?, responses?, messages? },  # 内置为覆盖，自定义为定义（至少一项）
                   models { mode, picked, added },
                   contextWindowTokens?, timeoutMs, maxOutputTokens } ]
```

`settings.json` 不含任何 secret。API key 与 ChatGPT OAuth token 只保存在同目录的 `credentials.json`（`meetnote.credentials.v1`）：以 0600 创建、原子替换、独立文件锁；对 group/other 可读时拒绝加载；解析错误与 `Debug` 输出不引用 secret。不使用操作系统 keychain。v1 设置在加载时于内存迁移为 v2，旧 connection profile 及其 key 被丢弃，其余字段保留，下次保存写回 v2。

`ProviderService::client` 为 Loop 构造客户端：按 `ModelChoice` 解析 API 并取当前区域该 API 的 base URL（有覆盖时用覆盖）；未知区域报错；供应商被禁用时拒绝；ChatGPT access token 距过期不足 5 分钟时先刷新，在凭据锁内持久化轮换后的 token，再使用新 token。返回给 UI 的错误按该供应商已存凭据脱敏。

### 登录与额度

- ChatGPT：使用 Codex CLI 的 OAuth client 执行 PKCE，回调监听 `127.0.0.1:1455`；也可只读导入 Codex CLI 的 `auth.json`（`$CODEX_HOME`，否则 `~/.codex`）。
- OpenRouter：官方 OAuth PKCE，回调监听本机随机端口，换得的 API key 按普通 API key 保存。
- xAI 没有面向第三方的公开 OAuth，只支持 API key。
- Anthropic 订阅 OAuth 不支持：Anthropic 条款不允许第三方使用 Claude 订阅；Anthropic 只支持 API key。

额度只做展示：ChatGPT 读取 `wham/usage` 的用量窗口、credits 与可用 reset card；OpenRouter、DeepSeek、Moonshot 读取 API key 的预付余额；其余供应商不显示额度。不建设计费、预算或成本核算。`ProviderService::test` 在 `client` 解析出的同一 API 上用一次文本请求和一次工具调用请求检查所选模型的连通性与工具支持，不证明任务质量。

## 当前状态与验证

上述供应商、凭据、登录、额度与 Loop 已实现。`crates/llm`、`crates/settings` 与 `crates/application/tests/providers.rs` 的协议、登录、刷新、额度、余额、凭据与设置迁移测试均使用本机 loopback stub；尚未对任何真实供应商端点执行验证。具体任务与验收见 [Assistant 计划](../plan/assistant-plan.md)。
