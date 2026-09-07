# Assistant 构建计划

## 目标

Assistant 是 MeetNote 内置的文档 Agent。它读取用户授权的文档事实，按 Skill 完成问答、内容整理、语义拆分和项目树调整，并通过与外部 Agent 相同的 document application 边界写回。

本计划描述目标架构，不以当前 `llm` trait、Assistant 占位页、`DocumentCommandService` 形态或 settings schema 为限制。实现可以替换这些内部接口，但必须保持以下产品要求：

- 文档和音频事实只有一个权威来源，Agent 不维护第二份项目状态。
- Agent 不能覆盖 MeetNote 管理的原始音频引用、processing provenance、transcript、diarization 和 speaker alignment。
- Agent 的所有文档操作经过同一组 schema、revision、来源、范围和项目树校验。
- Assistant 与外部 Agent 使用相同 tool contract、错误码和写回规则。
- 写回默认可以自动执行；用户可在 settings 中切换为逐次确认。
- provider 不可用时，本地文档读取、编辑、扫描和恢复能力不受影响。

## 产品场景

### 当前文档问答

用户针对一个明确选择的文档提问。Assistant 只获得该文档允许公开给模型的 Markdown、transcript、speaker-labelled segment 和必要 metadata，不创建持久化对话状态，也不触发写操作。

### 文档内容整理

Assistant 可以修改标题、标签、总结、Markdown 和 `document_status`。模型通过 typed tool call 表达修改意图；application 校验成功后，根据写回策略自动执行或进入确认队列。

### 语义拆分

Assistant 按 Skill 将一个父文档拆成多个连续的语义子文档。模型只选择真实 transcript segment ID，不直接生成权威毫秒边界。application 将 segment ID 映射为时间范围，完成覆盖、交叉、来源、层级和 revision 校验，再以一个事务创建全部子文档。

语义拆分分为两个明确模式：

- `partition`：完整分割父范围。子文档按序覆盖全部父范围，不重叠、不留空洞。
- `extract`：提取符合目标的部分范围。允许空洞，但必须在执行结果或确认界面中明确报告未覆盖范围。

同一子文档只表达一个连续音频范围。同一主题在录音中多次出现时应创建多个子文档，或由正文建立引用，不能把不连续范围伪装成一个连续来源。

每次拆分只创建下一层。用户或 Agent 可以对某个子文档再次执行拆分，从而形成最多三层的项目树；一次请求不同时生成两级新子树。

### 项目树调整

Assistant 可以移动或排序现有文档。操作必须携带当前 revision，并在写入前验证父节点、循环、深度和 `audio_asset_id`。多文档关系更新作为一个事务执行。

### 外部 Agent

外部 Agent 与 Assistant 是同一 Agent application boundary 的两个客户端，不是后续追加的第二套能力。两者获得完全相同的 read/write tools、argument schema、结构化结果、固定错误码、授权模型和写回策略；实际 document scope 都由用户授予。差别只在调用入口：Assistant 由内置 AgentLoop 调用，外部 Agent 通过稳定的 JSON CLI/adapter 调用。

外部 adapter 不直接暴露内部 Rust service ABI，也不能绕过 ToolRegistry、validator、write policy 或事务。Phase 0 起由 Codex 作为第一个外部 Agent，通过 CLI 黑盒调用每个已交付 tool；只通过内置 Assistant 验证的功能不算完成。

## 目标架构

```text
GPUI Assistant client       external Agent JSON adapter
          |                            |
          +------------+---------------+
                       v
                AgentGateway
        ├── ContextAssembler
        ├── SkillRegistry
        ├── AgentLoop
        ├── ToolRegistry
        ├── DocumentMutationValidator
        └── AgentWritePolicy
                |
        +-------+--------------------+
        |                            |
        v                            v
protocol-neutral LLM request   document application boundary
        |                            |
        v                            v
LLM protocol adapter           transactional persistence
├── OpenAI-compatible Chat Completions
├── Anthropic Messages
└── OpenAI Responses
        |
        v
configurable LiteLLM gateway
```

Assistant 的推理循环位于 application，不位于 GPUI、protocol adapter 或 persistence。外部 Agent 可以自己完成推理循环，但只能通过同一 AgentGateway 使用 tools。protocol adapter 只做 wire format 转换；Skill 只定义任务语义；tool contract 定义可请求的操作；validator 是所有写入是否成立的唯一裁决者。

## 约束机制与边界

完整实现只保留三项独立机制，不建立四套相互重复的规则系统。

### Skill 同时承载提示词

Skill 是版本化的任务定义和提示词包，不再单独维护另一套 prompt template registry。内置 Skill 使用以下结构：

```text
resources/assistant/skills/<skill-id>/SKILL.md
```

frontmatter 只声明机器需要读取的元数据：

```yaml
id: topic-partition
version: 1
allowed_tools:
  - read_document
  - create_semantic_children
```

正文直接包含完整 system instructions、语义判断原则、输出要求和必要示例。Skill 可以决定标题风格、切分粒度、标签约定、总结要求和 `partition`/`extract` 模式，但不能重新定义 document schema、写权限或 validator 行为。

application 只追加一段短小且固定的基础指令，用于说明可用工具、错误修正协议和当前写回模式。领域要求不在基础 prompt、Skill、tool description 三处重复书写。

首轮不增加 LLM intent router。UI 或 external `AgentToolRequest` 必须明确选择 `ask`、`organize`、`partition`、`extract` 或 `move` 场景，场景映射到一个 Skill 和最小 tool set；默认是只读 `ask`。自由文本只能影响所选 Skill 内的语义判断，不能自行切换到权限更高的 Skill。以后如增加自动路由，只能建议 Skill，不能直接扩大 tools 或写权限。

### Rust 类型是 Tool JSON Schema 的唯一来源

写入类输出不再同时维护“响应 JSON Schema”和“Agent Tool Schema”。每个 tool 的 Rust argument struct 使用 `serde` 和受约束 newtype 解析，并从同一类型与约束 annotation 生成提交给模型的 JSON Schema。实现不得手写第二份等价 schema。

JSON Schema 只负责：

- 字段名称、类型、必填项和 enum。
- 拒绝未知字段。
- 字符串、数组和数字的基本边界。

provider 侧的 schema enforcement 不是可信边界。typed deserialization 和受约束 newtype 必须再次执行同一 Rust 类型声明中的结构约束，再把 tool arguments 转成 application command；它们不依赖模型是否遵守 schema。第一阶段不引入独立通用 JSON Schema validator，也不要求 provider 原生支持 Structured Outputs。

### Rust validator 只负责领域不变量

validator 不重复检查已经由 typed deserialization 保证的 JSON 类型。它只处理无法可靠写入 JSON Schema 的状态相关规则：

- document 是否存在、revision 是否一致。
- 字段是否属于 Agent 可写范围。
- segment ID 是否存在及顺序是否有效。
- 子范围是否位于父范围内。
- `partition` 的完整覆盖和无重叠。
- `extract` 的范围报告。
- 父节点、循环、来源和三层上限。
- 批次操作能否作为一个事务整体提交。

validator 只能返回固定错误码和结构化 context。人类可读 `message` 不参与程序分支。

## Agent Tool 设计

目标 tool surface 如下；各 tool 按 Phase 逐步开放：

| Tool | 作用 | 副作用 |
|---|---|---|
| `list_documents` | 获取授权范围内的文档摘要、ID 和 revision | 无 |
| `read_document` | 按 projection 读取一个文档 | 无 |
| `update_document` | 修改允许的语义字段 | 有 |
| `create_semantic_children` | 批量创建下一层语义子文档 | 有 |
| `move_document` | 一次设置父节点和同级位置，同时覆盖 reparent/reorder | 有 |

`read_document.projection` 使用 enum 限定为 `metadata`、`markdown`、`transcript`、`speaker_segments` 或 `complete`。写入 tool 的 schema 不包含 `processing`、`results`、音频路径和模型 provenance，因此模型不能构造这些字段的修改请求。

`create_semantic_children` 的核心参数为：

```text
parent_document_id
expected_parent_revision
mode: partition | extract
children[]
  ├── start_segment_id
  ├── end_segment_id
  ├── title
  ├── tags
  ├── summary
  └── evidence_segment_ids
```

application 从 segment ID 派生权威时间范围，并根据父文档结果生成子文档的原始 transcript Markdown。Agent 不需要复制 transcript，也不在拆分 tool 中重写 processing/results。

写入类 tool 不等同于直接磁盘操作。ToolRegistry 先生成 `DocumentMutationBatch`，依次经过 validator 和 `AgentWritePolicy`，最后才调用 document application boundary。

## Assistant 与外部 Agent 的统一接口

AgentGateway 是内置 Assistant 和外部 Agent 的唯一 tool 入口，公开版本化 `AgentToolRequest` / `AgentToolResponse`。内置 Assistant 直接调用 Rust interface；外部 adapter 只负责 JSON 编解码，不能重新实现命令、校验或权限。

初期外部 adapter 使用可组合、非交互的 CLI：

```text
meetnote agent describe --json
meetnote agent call <tool-name> --json-stdin
```

- request 从 stdin 读取，避免大型 transcript、secret 或 JSON 被 shell history 记录。
- stdout 只输出一个 versioned JSON response；诊断日志只写 stderr。
- exit `0` 表示成功；exit `2` 表示已返回结构化 tool/validator error；exit `3` 表示 adapter 或进程级失败。
- 每个 request 必须包含 `schemaVersion`、`requestId`、scope selector 和 arguments；AgentGateway 将 selector 与调用入口授予的 `AgentAccessPolicy` 求交，客户端不能自行扩大授权。
- side-effect request 使用 `requestId` 作为幂等键的一部分；重复调用返回第一次提交结果。
- `describe` 一次返回 contract version、当前授权、write mode、可用 tools 和 schema；schema 与内置 Assistant 实际使用的定义来自同一 Rust 类型。

Codex 在 Phase 0 起作为外部 Agent 执行黑盒验收：只运行公开 CLI、解析 JSON、检查文档 readback 和 revision，不链接内部 crate，也不直接调用 `DocumentCommandService`。OpenAI 的 Codex 使用指南也将可组合命令、可预测 JSON、精确 ID 读取和安全写入作为 Agent-friendly CLI 的适用场景。[Codex 可组合 CLI](https://learn.chatgpt.com/use-cases/agent-friendly-clis)

## 写回策略

settings 保存：

```text
assistant.write_mode = automatic | confirm_each
```

默认值为 `automatic`。

### `automatic`

合法的写入 tool call 在 validator 通过后立即事务提交。执行结果作为 tool result 返回 Agent，使其继续任务或向用户汇报。UI 显示已完成的变更摘要和可恢复操作，但不阻塞等待预览或确认。

### `confirm_each`

合法的写入 tool call 转换为内存中的 pending mutation，UI 展示影响文档、字段差异、树结构变化和未覆盖范围。用户确认后使用原 expected revision 执行；revision 已变化时必须拒绝，不能静默 rebase。拒绝结果作为 tool result 返回 Agent。

两种模式共享相同 ToolRegistry、validator、事务和错误码。确认只是执行策略，不形成另一套 proposal schema 或写入通道。

任何已提交操作都必须返回新的 revision、受影响文档 ID 和恢复标识。自动写回的安全基础是原子提交、固定错误码、可定位 provenance 和可恢复变更，不是隐藏式 best-effort 写入。

## Validator 错误协议

所有 Assistant 和外部 Agent tool error 使用同一 envelope：

```json
{
  "schemaVersion": 1,
  "ok": false,
  "error": {
    "code": "DOCUMENT_REVISION_CONFLICT",
    "message": "document revision changed",
    "retryable": true,
    "toolCallId": "call-123",
    "field": "expectedRevision",
    "documentId": "document-123",
    "expectedRevision": 8,
    "actualRevision": 9
  }
}
```

错误码是跨 Assistant、外部 adapter 和协议实现的稳定 contract。增加错误码允许向后兼容；修改既有错误码语义必须提升 error schema version。

### 固定错误码

| Code | retryable | 含义与修正方式 |
|---|---:|---|
| `TOOL_UNKNOWN` | false | tool 不存在 |
| `TOOL_NOT_ALLOWED` | false | tool 未被当前 Skill 或授权范围开放 |
| `TOOL_ARGUMENTS_INVALID` | true | arguments 无法反序列化；根据 field/context 重建调用 |
| `TOOL_LOOP_LIMIT_REACHED` | false | 达到本次 Agent loop 上限 |
| `SKILL_NOT_FOUND` | false | 请求的内置 Skill ID/version 不存在 |
| `DOCUMENT_NOT_FOUND` | true | 重新 list/read 后决定是否继续 |
| `DOCUMENT_ACCESS_DENIED` | false | document 不在本次 Agent 授权范围内 |
| `DOCUMENT_REVISION_CONFLICT` | true | 必须重新 read，不能复用旧内容静默重试 |
| `DOCUMENT_FIELD_READ_ONLY` | true | 删除禁止字段并重新调用 |
| `SEGMENT_NOT_FOUND` | true | 重新读取 transcript 并使用真实 segment ID |
| `RANGE_INVALID_ORDER` | true | 修正开始、结束 segment 顺序 |
| `RANGE_OUTSIDE_PARENT` | true | 将范围限制在父文档内 |
| `RANGE_OVERLAP` | true | 消除兄弟范围交叉 |
| `RANGE_COVERAGE_GAP` | true | `partition` 模式补齐未覆盖 segment |
| `TREE_PARENT_NOT_FOUND` | true | 重新读取项目树 |
| `TREE_CYCLE` | true | 选择非后代父节点 |
| `TREE_MAX_DEPTH_EXCEEDED` | true | 停止创建第四层或选择更高层父节点 |
| `TREE_CROSS_AUDIO_ASSET` | true | 选择相同 `audio_asset_id` 的父节点 |
| `WRITE_POLICY_REJECTED` | false | 用户在 `confirm_each` 模式拒绝写回；本次 loop 结束 |
| `WRITE_CONFIRMATION_EXPIRED` | true | pending mutation 已过期；重新读取并生成新调用 |
| `IDEMPOTENCY_CONFLICT` | false | 同一幂等键对应了不同 arguments |
| `WRITE_TRANSACTION_FAILED` | false | 事务未提交；由用户或应用诊断存储错误 |
| `CONNECTION_NOT_CONFIGURED` | false | 没有 active connection 或配置不完整 |
| `PROVIDER_CAPABILITY_UNSUPPORTED` | false | 当前 connection/model 不支持任务所需能力 |
| `PROVIDER_AUTHENTICATION_FAILED` | false | 修正 connection credential |
| `PROVIDER_RATE_LIMITED` | true | 遵守服务端 retry hint 后重试 |
| `PROVIDER_TIMEOUT` | true | 在 loop 预算内重试或切换 connection |
| `PROVIDER_RESPONSE_INVALID` | true | provider 返回无法解释的协议响应 |
| `CONTEXT_LIMIT_UNKNOWN` | false | 配置 context limit 或使用可报告限制的 connection |
| `CONTEXT_LIMIT_EXCEEDED` | true | 缩小 projection 或进入长文档分块流程 |
| `TASK_CANCELLED` | false | 用户取消任务；不继续 loop |

validator 错误不会产生任何文档副作用。Agent 可以根据 `retryable`、`code` 和 context 修正 tool call；默认最多允许 3 次 validator 修正和 8 个总 tool round。达到上限后停止任务并向 UI 返回最后一个错误，不进行无限 loop。

## LLM 协议架构

### 内部统一模型

application 只使用 protocol-neutral 类型：

```text
LlmRequest
├── instructions
├── messages: LlmMessage[]
├── tools: LlmToolDefinition[]
├── tool_choice
└── max_output_tokens

LlmOutputBlock
├── Text
└── ToolCall { id, name, arguments }

LlmInputBlock
├── Text
└── ToolResult { tool_call_id, payload, is_error }
```

AgentLoop 保存本地 canonical conversation，并负责执行 tool、追加 tool result 和进入下一轮。第一阶段不依赖 provider 端持久 conversation，也不使用 Responses 的 `previous_response_id`；这保证切换 LiteLLM route 后不会丢失 Agent 状态。

protocol adapter 只负责 canonical block 与 wire payload/event 的双向转换。禁止让 application 分支处理 `tool_calls`、`tool_use`、`function_call_output` 等协议专有字段，也禁止以任意 `serde_json::Value` 作为跨层公共消息模型。

### OpenAI-compatible Chat Completions adapter

- 使用 `/v1/chat/completions`。
- 将 instructions 映射为 system/developer message。
- 将 ToolCall 映射为 assistant `tool_calls`。
- 将 ToolResult 映射为 `role=tool` message。
- 处理 finish reason、usage 和不完整 arguments。

### Anthropic Messages adapter

- 使用 `/v1/messages`。
- 将 instructions 映射为顶层 `system`。
- 将 ToolCall 映射为 assistant `tool_use` content block。
- 将 ToolResult 映射为紧随其后的 user `tool_result` content block。
- 维护 `anthropic-version`、stop reason 和 content block 顺序。

Anthropic Messages 的 tool result 必须紧跟对应 tool use，且 tool result block 位于下一条 user content 的前部；adapter 负责此顺序，AgentLoop 不感知该差异。Tool definition 使用 `input_schema` 表达参数结构，仍由相同 Rust 类型生成。[Anthropic tool use](https://platform.claude.com/docs/en/agents-and-tools/tool-use/define-tools)

### OpenAI Responses adapter

- 使用 `/v1/responses`。
- 将 instructions、input items、function call 和 function call output 映射为 Responses item。
- 默认发送 `store=false`，由 MeetNote 本地保存 canonical conversation。
- 解析 output item 数组、response status、incomplete details 和 usage。
- 第一阶段不接入 provider-hosted tool、conversation 或 background response。

OpenAI Responses 同时支持 instructions、结构化 text format、custom function tools 和多种 output item；adapter 只实现 Assistant 当前需要的文本与客户端 function tools，不把所有 provider 功能泄漏到 application。[OpenAI Responses API](https://developers.openai.com/api/reference/cli/resources/responses/methods/create)

### 能力模型

connection probe 产生运行时能力：

```text
text
client_tools
```

只读问答只要求 `text`。任何写入任务必须要求 `client_tools`；没有 tool calling 的 model/endpoint 不使用自由文本 JSON fallback 执行写入。provider 是否支持 strict schema、usage 和 system/developer message 作为 adapter 响应与诊断字段处理，不扩张为 application capability 分支；本地 typed deserialization 与 validator 始终运行。

一个任务开始后固定 connection、protocol 和 model route。LiteLLM fallback 若返回不同实际 model，必须记录 requested/resolved model；fallback model 不满足 `client_tools` 时任务失败，不能静默降级为无约束文本。

## LiteLLM 接入

LiteLLM 是可配置 gateway，不是 application 的消息模型，也不需要独立的 vendor-specific Agent runtime。三个 protocol adapter 都可以指向同一个 LiteLLM base URL；LiteLLM-specific 代码只负责 health、model discovery、认证默认值和诊断方式。

LiteLLM Proxy 提供 OpenAI-compatible Chat Completions 和 Responses 调用，并提供 Anthropic-compatible Messages endpoint；不同模型支持的 tool/structured parameters 并不一致，因此必须以 connection/model probe 为准，不能因 gateway 可达就宣称模型能力可用。[LiteLLM 客户端接口](https://docs.litellm.ai/docs/proxy/user_keys)

MeetNote 不内嵌 LiteLLM Python SDK，不启动 Python subprocess，也不管理 LiteLLM 的 provider credential。OpenAI、Anthropic、Gemini、Azure、Ollama 和自定义 provider 都在 LiteLLM 侧配置；MeetNote 只保存 gateway connection 和 gateway credential reference。LiteLLM 可通过 `api_base` 接入自定义 OpenAI-compatible endpoint。[LiteLLM OpenAI-compatible endpoint](https://docs.litellm.ai/docs/providers/openai_compatible)

不得启用会静默删除 `tools`、`tool_choice` 或结构化参数的 `drop_params` 行为。写入任务所需参数不受支持时必须返回 `PROVIDER_CAPABILITY_UNSUPPORTED`。

## Settings

所有 connection profile 与 Assistant 行为配置都进入现有 `AppSettings`，不创建独立配置文件：

```json
{
  "assistant": {
    "enabled": true,
    "activeConnectionId": "local-litellm",
    "writeMode": "automatic",
    "maxToolRounds": 8,
    "maxValidationRetries": 3,
    "connections": [
      {
        "id": "local-litellm",
        "name": "Local LiteLLM",
        "protocol": "openaiChatCompletions",
        "baseUrl": "http://127.0.0.1:4000",
        "model": "assistant-default",
        "authMode": "bearer",
        "credentialRef": "meetnote/assistant/local-litellm",
        "contextWindowTokens": 128000,
        "timeoutMs": 120000,
        "maxOutputTokens": 4096
      }
    ]
  }
}
```

`protocol` 固定为：

- `openaiChatCompletions`
- `anthropicMessages`
- `openaiResponses`

`authMode` 固定为 `bearer` 或 `xApiKey`。`contextWindowTokens` 是可选的显式上限；未配置且 probe 不能确定时，不运行需要自动分块的长文档任务。

profile 中保存 connection metadata；当前只实现 LiteLLM gateway，因此不增加只有一个取值的 `gatewayKind`。实际 secret 通过 `credentialRef` 从系统 credential store 读取，不写入 `settings.json`，也不产生新的 secret 文件。LiteLLM gateway 使用 Bearer 或 Anthropic-compatible header 的差异由 protocol adapter 和 profile auth mode 处理。

`baseUrl` 支持自定义主机、路径和端口。localhost 允许显式 HTTP；非本机地址默认要求 HTTPS。Authorization 或 API key header 不得随跨 origin redirect 转发。Settings 页面必须提供保存前格式校验、connection test、model probe、错误诊断和 credential 更新入口。

## 上下文与隐私

ContextAssembler 默认只提供用户当前选择的文档。它可以发送：

- 文档 ID、revision、层级和当前语义字段。
- Markdown。
- 带 ID 和时间戳的 transcript。
- speaker-labelled segment。
- 当前 Skill 和可用 tool definition。

默认不发送：

- 原始音频字节和本机绝对路径。
- Speaker Library embedding 或 enrollment 样本。
- 模型缓存、catalog 内部状态和未选择的文档。
- credential、环境变量和文件系统目录。

UI 必须显示当前 connection 是本机还是远程，以及本次任务会发送哪些文档。原始 prompt、完整 transcript 和响应默认不写应用日志。

文档正文属于不可信输入。adapter 和 AgentLoop 必须保持 instructions、Skill、tool definition 与 document content 的结构边界；文档中的文本不能声明新 tool、扩大授权范围或改变写回策略。

## 文档事务与恢复

语义拆分、move 和 reorder 在发布前依赖统一的 `DocumentMutationBatch`：

1. 读取所有受影响文档及 revision。
2. 在内存中应用全部候选变更。
3. 对变更后的完整树运行 validator。
4. 将新文档包写入 staging directory，并记录可恢复 journal。
5. 全部落盘成功后统一发布。
6. 任一步失败时恢复提交前状态，watcher 不得观察到半成品树。

成功结果包含 mutation ID、旧/新 revision 和受影响文档。每个 side-effect tool 使用 task ID 与 tool call ID 形成幂等键；重复提交返回第一次提交的结果，不能再次执行。结构操作必须推进全部受影响文档的 revision，创建子文档时也必须推进父文档 revision。

automatic 模式必须提供可恢复的 undo 入口；undo 作为新的 revision-aware mutation 执行，不能用不可恢复删除或覆盖实现。

语义修改 provenance 与音频 processing provenance 分离，至少记录：

```text
actor: assistant | external_agent | user
mutation_id
skill_id / skill_version
connection_id
protocol
requested_model / resolved_model
source_revisions
applied_at_ms
```

不默认保存模型隐藏推理、完整 prompt 或完整响应。

## 代码分区

```text
crates/contracts
├── assistant settings DTO
├── canonical tool arguments/results
├── AgentErrorCode / AgentToolError
└── AgentToolRequest / AgentToolResponse

crates/llm
├── canonical request/output types
├── LlmProtocolAdapter trait
├── openai_chat adapter
├── anthropic_messages adapter
├── openai_responses adapter
└── LiteLLM health/model probe

crates/application/src/assistant
├── gateway
├── context
├── skills
├── loop
├── tools
├── validator
└── write_policy

crates/persistence
└── document mutation batch / journal / recovery

crates/settings
└── AssistantSettings persistence and validation

crates/ui
└── connection settings / task progress / result / optional confirmation

apps/meetnote
└── GPUI composition / external Agent JSON CLI
```

protocol crate 不依赖 GPUI 或 persistence。application 不解析 provider wire JSON。persistence 不知道 Skill、LLM 或 Agent。UI 不执行 Agent loop、validator 或文档 IO。具体 LLM protocol adapter 和外部 JSON adapter 由 `apps/meetnote` 组装；它们共享 AgentGateway，但互不依赖。

## 过度设计边界

以下能力不进入首轮实现，除非前一阶段的真实验证证明必要：

- 不建立独立 proposal domain。写入 tool 经过 policy 后直接执行或暂存在内存，proposal 只是 `confirm_each` 的 UI projection。
- 不同时维护手写 JSON Schema、响应 schema 和 Rust struct。Rust tool argument 类型是唯一声明来源。
- 不引入通用 Agent framework、MCP runtime、vector database、RAG index 或长期 memory。
- 不建立 Markdown 模板检测器或文档类型分类 schema。会议、访谈、全天记录等差异由用户选择的 Skill 表达；文档排版由 application renderer 生成，模板匹配不成为写入前置条件。
- 不为每个模型厂商建立 provider crate。所有模型由 LiteLLM 管理，MeetNote 只实现协议 adapter。
- 不允许任意 provider-specific JSON 参数、header passthrough 或自动降级；只有明确进入 canonical contract 的能力才开放。
- 不在 Phase 0 同时实现三种 wire protocol。先用 OpenAI-compatible Chat Completions 完成端到端闭环，再在相同 contract 上增加 Anthropic Messages 和 OpenAI Responses。
- 首轮不实现 streaming。问答和 tool loop 先使用完整响应；流式 UI 只有在非流式闭环通过且真实延迟证明需要时再立项，不进入当前完成条件。
- 不持久化 provider conversation、隐藏推理或完整 prompt/response。任务状态由本地 AgentLoop 管理。
- 不要求所有 connection 支持 model discovery；允许用户直接填写 LiteLLM model alias，实际 probe 才是可用性依据。
- AgentGateway、ToolRegistry、SkillRegistry、validator 和 write policy 是 `application::assistant` 内职责明确的模块，不是独立进程或可插拔服务。ToolRegistry 使用静态 tool table，SkillRegistry 只加载随应用发布的 Skill；首轮不实现动态插件、热更新或依赖注入框架。

原子批次、revision、幂等键、固定错误码、credential 隔离和 undo 不属于可删除的复杂度。automatic 写回会直接改变用户文档，这些能力是自动执行成立的最低安全边界。

## 语义评测数据

语义拆分不能等实现完成后再临时寻找样本。Phase 0 先建立不依赖真实音频推理的 text-level corpus；Phase 3 再接入公开标注数据。数据只验证 Assistant 的语义工具链，不替代 ASR、diarization 或真实音频质量验收。

### Codex 独立生成的合成集

Codex 作为外部 Agent 创建可重复生成的 scenario blueprint 和 gold mutation，不让被测 Assistant 模型同时生成输入与答案。generator 使用固定 seed，从 blueprint 生成 MeetNote `DocumentRecord`、segment ID、speaker、时间戳和期望 tool call。

完整合成集至少包含 48 个文档：会议、访谈、全天记录、自由对话四类场景，各覆盖中文、英文、中英混合三类语言，每个组合覆盖 `partition`、`extract`、无需拆分和二级拆分四种结构。Phase 0 只生成 8 个 smoke fixture 验证格式和外部接口，Phase 3 再补齐完整矩阵，避免数据工程阻塞第一个可运行闭环。整个集合必须覆盖：

- 没有静音的主题切换，以及有长静音但主题不变。
- speaker 切换但主题不变，以及同一 speaker 连续切换主题。
- 主题中断后再次出现、闲聊、开场、设备问题和无意义片段。
- 极短 segment、缺失 speaker label、ASR 标点/同音错误和重叠语音文本。
- 文档正文中的 prompt injection、伪造 document ID 和伪造 tool 指令。
- 合法三层结构、非法第四层、范围交叉、空洞和 revision conflict。

gold data 必须包含允许的拆分模式、边界 segment ID、父子关系、必需 evidence segment 和禁止操作。标题、标签和总结允许声明多个可接受答案；结构、范围和 evidence ID 使用确定性期望值。

小型合成 fixture 与 generator 存放在 `crates/application/tests/fixtures/assistant/` 和对应 test support module，可以提交仓库。每个 fixture 保存 generator version、seed 和 scenario ID，确保失败可复现。

### 下载并转换的公开数据

公开语料下载到默认 `target/assistant-eval-data/` 或显式测试缓存，不提交大体积 transcript、annotation 或音频。仓库只在 `scripts/assistant-eval/` 保存 fetch/convert 脚本、来源 manifest、license/checksum 和允许提交的最小派生 fixture。

- QMSum：人类标注的多领域会议数据，包含 transcript、`topic_list`、相关 span 和 query-summary，可验证 `extract`、主题复现和 evidence span。其同一 topic 可以对应多个不连续 span，转换时必须拆成多个连续 MeetNote child，不能改变 source model。QMSum 只提供 turn/span 时，转换器生成稳定 segment ID 和仅用于 text-level fixture 的单调时间，不据此声称真实音频边界质量。[QMSum](https://github.com/Yale-LILY/QMSum)
- AMI manual annotations：场景会议与自然会议，包含 transcript、浅层 topic hierarchy、抽取/生成式总结；其两级子主题与 MeetNote 三层树直接对应。仅下载约 22 MB 的 manual annotations 即可完成 text-level 验证，不要求下载音视频。AMI 官方未提供 topic segmentation reliability 结果，因此它必须与合成 exact-match、QMSum 和人工抽查组合使用，不能独自作为真值。[AMI download](https://groups.inf.ed.ac.uk/ami/download/)、[AMI topic annotations](https://groups.inf.ed.ac.uk/ami/corpus/annotation.shtml)
- SuperDialseg：包含 9,478 个 document-grounded dialogue，可作为 topic-boundary 补充压力集；它不是会议树或总结 benchmark，不能单独承担产品验收。数据由外部下载入口提供，使用前必须核对派生数据的上游 license。[SuperDialseg](https://aclanthology.org/2023.emnlp-main.249/)

fetch 脚本必须支持 checksum、断点重试、license snapshot、只下载 annotations/text 和无网络时跳过。convert 脚本将来源 speaker/utterance/span 映射为稳定 segment ID，并输出转换报告：输入/输出数量、被舍弃项、断续 span 拆分、无法映射边界和 license。

Codex 负责执行首次下载、转换和黑盒评测，并在结果中明确区分 synthetic exact-match、公开人工标注和人工抽查三种证据。公开数据不能被称为合成数据，合成结果也不能冒充人工 ground truth。

## 需求覆盖矩阵

| 需求 | 实现位置 | 首次完成 Phase |
|---|---|---:|
| Assistant 与外部 Agent 同定位、同接口 | AgentGateway、同源 tool schema、JSON CLI | 0 |
| Codex 作为初期外部 Agent 验证 | 每阶段 CLI black-box 验收 | 0，并持续执行 |
| connection profile 只进入 settings | `AppSettings.assistant.connections` | 1 |
| LiteLLM、自定义 host/path/port | connection profile + LiteLLM probe | 1 |
| OpenAI-compatible Chat Completions | protocol adapter | 1 |
| 自动写回与可关闭选项 | `automatic` / `confirm_each` write policy | 2 |
| 固定 validator 错误码与修正 loop | `AgentToolError` + ToolResult | 2 |
| Skill/提示词、Schema、Tools、validator 去重 | Skill 包、Rust 单源 schema、领域 validator | 2 |
| `partition` / `extract` 语义拆分 | segment-ID tool + batch mutation | 3 |
| 项目树移动和排序 | 单一 `move_document` tool | 3 |
| 长文档上下文与资源边界 | Map/Reduce + 显式 context limit | 4 |
| Anthropic Messages | protocol adapter | 5 |
| OpenAI Responses | protocol adapter | 5 |
| 合成与公开语义数据 | generator + QMSum/AMI/SuperDialseg converter | 0/3/6 |
| 事务、幂等、provenance、undo | document mutation boundary | 2/3 |
| 质量、安全和发布证据 | 黑盒套件与分层验收 | 6 |

## 分阶段实施

每个 Phase 必须形成可独立验证、可独立回退的场景闭环。不得一次提交跨越多个 Phase 的混合实现。

### Phase 0：Assistant 与外部 Agent 读取同一文档

目标：不接入 LLM，先证明内置 Assistant application client 和 Codex 外部 Agent 可以通过同一 AgentGateway 发现工具、读取相同文档并获得相同错误；GPUI 在 Phase 1 接入该 client。

功能要求：

- 定义 versioned `AgentToolRequest`、`AgentToolResponse`、显式 scene/Skill selection、成功结果和错误 envelope。
- 实现 AgentGateway、静态 ToolRegistry、授权 scope，以及 `describe`、`list_documents`、`read_document`。
- Rust tool argument 类型生成 JSON Schema；typed decoding 拒绝未知字段和非法 enum。
- 在 `apps/meetnote` 增加非交互 JSON CLI，不复制 application 逻辑。
- `AppSettings` 预留 Assistant 节点、write mode、connection profile 和 loop budget；此阶段不连接模型。
- 建立合成语义文档的 blueprint、generator、manifest 和 8 个固定 smoke fixture。
- 实现 `TOOL_UNKNOWN`、`TOOL_NOT_ALLOWED`、`TOOL_ARGUMENTS_INVALID`、`SKILL_NOT_FOUND`、`DOCUMENT_NOT_FOUND`、`DOCUMENT_ACCESS_DENIED` 的稳定序列化。

验证要求：

- application contract test 与 CLI black-box test 使用同一 fixture，响应除 transport metadata 外必须一致。
- Codex 通过 `meetnote agent describe/call` 完成 schema discovery、list、五种 read projection、越权读取和 malformed arguments 测试。
- stdout 只有 JSON，stderr 只有诊断；退出码和 `ok/error` 一致。
- 同一 Rust 类型生成的 schema 同时出现在内置 client 与 CLI `describe` 结果中。
- 合成 generator 相同 seed 产生 byte-stable fixture，gold segment ID 全部存在。

完成条件：在没有 LiteLLM 和 GPUI 推理实现的情况下，Codex 已能作为外部 Agent 稳定读取文档；内置与外部入口不存在两套 tool contract。

### Phase 1：通过 LiteLLM 对当前文档问答

目标：用户在 Settings 创建 connection，选择模型并通过 OpenAI-compatible Chat Completions 完成当前文档的只读问答。

功能要求：

- 完整持久化 connection profile、active connection、write mode 和 loop budget；不创建额外配置文件。
- 实现 credential store abstraction 和 macOS credential backend，settings 只保存 `credentialRef`。
- 建立最小 canonical LLM request/output、AgentLoop 和 OpenAI-compatible Chat Completions adapter。
- 实现 LiteLLM readiness、可选 model discovery 和实际 text/tool probe；model discovery 不可用时允许手工填写 model alias。
- 实现 ContextAssembler 和只读问答 Skill，默认只授权当前文档，扩大范围必须来自明确用户操作。
- Assistant UI 默认选择只读 `ask`；外部 request 必须声明相同 scene，不能通过 prompt 自动升级权限。
- 支持取消、timeout、usage 和 requested/resolved model；当前计划统一使用非流式响应。
- 映射认证、限流、timeout、畸形响应和 capability error，不把 provider 错误伪装成 validator error。
- Settings 和 Assistant UI 展示连接测试、模型能力、任务状态、回答和来源 segment。

验证要求：

- fixture server 覆盖 request path、header、payload、文本、tool call 和 tool result round-trip。
- 覆盖 401/403、404、429、5xx、timeout、unknown model、畸形 JSON 和不完整 tool arguments。
- 验证 localhost 自定义端口、远程 HTTPS 和 redirect credential 隔离。
- provider 中断或取消不创建文件、不改变 revision。
- Codex 通过外部 CLI 读取与内置 Assistant 相同的文档和 segment，再核对 Assistant 回答引用的 ID。
- 一个真实 LiteLLM connection 完成直接问答、需要 `read_document` 的问答和不存在信息的问答。

完成条件：用户可以配置并测试 LiteLLM、询问当前文档且获得可追溯回答；外部 Agent 对相同文档事实具有对称读取能力。

### Phase 2：自动或确认后修改文档语义字段

目标：Assistant 和 Codex 外部 Agent 使用同一 `update_document` tool，在 automatic 与 confirm_each 两种策略下完成标题、标签、总结、Markdown 和状态修改。

功能要求：

- 实现 `DocumentMutationValidator` 和完整固定错误 envelope。
- 实现 automatic/confirm_each write policy；默认 automatic。
- 实现单文档原子 mutation、semantic provenance、revision 推进和 undo。
- side-effect request 使用幂等键；重复调用返回第一次结果。
- validator error 作为 tool result 返回内置 AgentLoop；外部 Agent 直接收到相同 JSON error 并自行修正调用。
- 禁止并行执行写 tool；同一轮多个写入先组成 batch，或顺序执行并刷新 revision。
- automatic UI 显示完成摘要和 undo；confirm_each UI 显示 pending diff、接受、拒绝和过期状态。
- Assistant UI 与外部 request 使用同一个 `organize` Skill ID 和允许工具集合。
- 完成其余 document、policy、transaction 和 loop error code 的序列化与兼容性测试。

验证要求：

- revision conflict、只读字段、非法 status、越权 document 和 malformed arguments 均不产生副作用。
- automatic 无需确认即可写入并可撤销；confirm_each 确认前不落盘，拒绝后不自动重试。
- 进程在 state、Markdown、journal 各写入点失败后，文档包只有提交前或提交后状态。
- 内置 AgentLoop 完成一次 validator error -> 修正 -> 成功写入。
- Codex 通过 CLI 分别触发同一错误、读取固定 code/context、修正 arguments 并成功写入；不得直接改文件完成测试。
- 相同幂等键重复提交不会增加 revision 或重复内容。

完成条件：两种客户端和两种 write mode 只在 client/policy 层分叉，使用同一 tool、validator、事务、错误码和持久化结果。

### Phase 3：按语义拆分并调整项目树

目标：Assistant 和 Codex 外部 Agent 对一个父文档执行 `partition` 或 `extract`，原子创建下一层语义子文档，并能移动和排序文档。

功能要求：

- 实现 `create_semantic_children` 和 `move_document` typed tools；`move_document` 同时表达 reparent 与同级 reorder，不创建重复工具。
- 将合成语义集从 8 个 smoke fixture 扩展为完整 48 个文档矩阵。
- 模型只提交 start/end/evidence segment ID；application 派生权威时间范围。
- 实现覆盖、空洞、交叉、父范围、来源、循环和三层 validator。
- application 从权威 processing results 裁剪 transcript、diarization 和 speaker segment，并生成含 summary 与 speaker-labelled transcript 的 Markdown。
- 一个 tool call 的全部子文档及父 revision 作为单个 `DocumentMutationBatch` 提交。
- automatic 直接提交；confirm_each 展示树变化、范围、空洞和受影响文档。
- 完成 QMSum、AMI annotations 的 fetch/convert；SuperDialseg 通过 license 后作为补充集。
- Skill 分别定义 `partition`、`extract` 和项目树调整场景，不重复 tool/validator 规则。
- Assistant UI 必须显式选择拆分模式；外部 request 使用同一个 scene/Skill ID，模型不能在执行中自行切换模式。

验证要求：

- 48 个合成 fixture 全部通过结构 exact-match；`partition` 完整覆盖，`extract` 精确报告空洞。
- QMSum 不连续 topic span 被转换为多个连续 child；AMI 浅层 topic hierarchy 不产生第四层。
- 伪造 ID、反向范围、跨父范围、重叠、覆盖空洞、循环、第四层和跨音频来源触发对应固定错误码。
- 内置 AgentLoop 对每类 retryable validator error 至少完成一个修正样例。
- Codex 通过 CLI 对同一 fixture 完成创建、readback、跨父节点 move、同级 move、错误修正和 undo。
- 每个文件写入阶段的故障注入均不能留下半成品树或孤儿文档。

完成条件：两种客户端都能在不手工编辑 JSON 的情况下完成可恢复、可追溯的语义拆分与树调整，并得到一致结果。

### Phase 4：处理超出单次上下文的长文档

目标：长录音在明确的 token、轮次和成本上限内完成问答与语义拆分，不改变短文档路径。

功能要求：

- 根据 connection probe 与显式模型上下文设置计算输入预算；未知限制时拒绝自动截断。
- 使用 transcript segment 与 speaker turn 组成上下文块；技术窗口不成为语义边界。
- Map 输出带 segment ID 的主题候选和 evidence，Reduce 只生成既有 typed tool call。
- 保留相邻块上下文，检测重复、遗漏、跨块主题和输入顺序变化。
- provider 超限时显式进入长文档流程或返回错误，不允许静默丢弃开头内容。
- 支持取消、阶段进度、usage、成本、失败恢复和整个任务的统一 loop budget。
- 长文档中间结果仅为任务临时状态，成功或失败后不成为文档事实来源。

验证要求：

- 使用跨 chunk 延续主题、长静音、多 speaker、重复话题和 prompt injection 合成 fixture。
- `partition` 最终覆盖 100%，segment 不重复，输出顺序稳定。
- 取消、任一 Map 失败或 Reduce 失败均不产生文档写入。
- Codex 通过 CLI 对同一长 fixture 读取分块诊断并核对最终 mutation，不访问内部临时结构。
- 对代表性长文档记录输入 segment、token、round、耗时、临时存储和峰值内存。

完成条件：输入规模增长不会破坏短文档行为、事实覆盖、事务性、客户端对称性或资源上限。

### Phase 5：补齐 Anthropic Messages 与 OpenAI Responses

目标：在不修改 AgentGateway、tools、validator、Skill 或 write policy 的前提下，让同一 Assistant 场景运行于三种主流协议。

功能要求：

- 实现 Anthropic Messages adapter 的 system、content block、tool use/result 和 stop reason 映射。
- 实现 OpenAI Responses adapter 的 instructions、input/output item、function call/output 和 status 映射。
- 保持本地 canonical conversation；Responses 默认 `store=false`，不依赖 provider conversation ID。
- 为三个 adapter 实现相同的 text、client_tools、cancel、usage 和 resolved model contract。
- Settings 可以选择三种协议；切换协议不修改 connection 之外的 Assistant 设置。

验证要求：

- 三套 golden fixture 覆盖普通文本、单/多 tool call、tool error、截断和 malformed response。
- 同一个 fake Agent scenario 通过三个 adapter 得到等价 canonical output 和 tool invocation。
- 三种协议分别完成只读问答、automatic write、confirm_each、validator 修正和 semantic split。
- 每种协议至少通过一个真实 LiteLLM route，并验证自定义端口。
- Codex 继续通过同一外部 CLI 执行对照测试，证明新增协议没有改变外部 Agent contract。

完成条件：三种协议差异完全留在 adapter，应用场景、工具、错误码、文档结果和外部接口没有分叉。

### Phase 6：质量、安全与发布验收

目标：结合独立合成数据、公开标注数据和小规模人工抽查，证明语义质量、协议兼容、失败恢复和隐私边界达到可发布标准。

功能要求：

- 固化 synthetic generator、QMSum/AMI/SuperDialseg converter、来源 manifest、license 和 checksum。
- 固化 Skill 版本与评测结果；修改 Skill 必须重新执行受影响 fixture 和 held-out corpus。
- 对 LiteLLM route/fallback 记录 resolved model，并验证 capability 不降级。
- 完成 credential redaction、日志审计、prompt injection、恶意 tool arguments 和未授权 document 测试。
- 由 Codex 运行公开 CLI 端到端套件；内部单元测试不能替代该黑盒证据。
- 从四类场景各抽取至少 6 个文档进行人工复核，仅用于校准标题、总结可用性和公开数据边界歧义，不重新手工构造整套数据集。

验证要求：

- 结构错误写入为 0；所有失败均无部分文档状态。
- synthetic gold 的范围、层级、error code 和 evidence ID exact-match 为 100%。
- `partition` 事实覆盖率为 100%，无重复 segment；`extract` 的未覆盖范围报告为 100%。
- 在 QMSum/AMI 上同时报告 Boundary F1、Pk 和 WindowDiff，并优于 no-split 与 equal-length 两个确定性 baseline；不以单一指标代替样例审查。
- evidence segment 全部存在且落在对应子范围内，summary 中的关键事实能定位到 evidence。
- 人工抽查中至少 80% 结果无需调整边界即可使用，且不能出现关键事实被修改或 processing/results 被覆盖。
- 三种协议、自定义端口和一个 LiteLLM 管理的自定义 OpenAI-compatible provider 全部通过真实连接测试。
- API key、Authorization、Speaker Library embedding、未授权文档和本机绝对路径不出现在日志或远程请求中。

完成条件：协议、领域正确性、语义质量、事务恢复、客户端对称性和隐私证据同时通过，Assistant 才能标记为生产可用。

## 实施顺序与提交边界

严格按 Phase 0 -> 1 -> 2 -> 3 -> 4 -> 5 -> 6 实施。Phase 0-3 的每个完成条件都必须包含一次由 Codex 作为外部 Agent 执行的 CLI 黑盒验收。

提交数量不预设。每个提交只承载一个可独立验证和回退的场景或功能边界；contract、application、adapter、persistence、UI、测试与文档可以在同一场景无法独立工作时一起提交，也可以在能够独立成立时拆开。不能把多个 Phase 混进一次提交，不能在 protocol adapter 提交中顺带修改语义规则，也不能用后续 Phase 的未完成抽象替代当前 Phase 的端到端闭环。
