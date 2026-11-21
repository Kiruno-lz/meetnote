# Assistant 与供应商计划

## 目标

1. **内置 Assistant 完成文档任务**：以最小 Tool Loop 执行 ask、organize、partition、extract、move 五个 Skill 场景，经 `WorkspaceCapabilityPort` 读取与写回；写回默认 automatic，可在 Settings 切换为 confirm_each。失败、取消和轮次耗尽都保留已提交 receipt。
2. **供应商可直接接入**：用户从内置目录选择供应商或配置自定义端点，以 API key 或供应商允许的浏览器登录接入，选择模型并查看账户额度；secret 只保存在 `credentials.json`。
3. **Loop 保持最小**：以 pi-agent-core（badlogic/pi-mono）的最小 agent loop 为收敛参照，不建设完整 Agent Runtime。

不做：外部 Agent（Codex、Claude Code）接入、Agent CLI/MCP、插件运行时、Audio/Settings 的 Agent 工具、计费/预算/成本核算。架构边界见 [Agent Host](../architecture/agent-host.md)。

## 范围与接口

- Loop：`AgentRuntime` + `WorkspaceAgentScene`；工具为 Workspace 六个文档工具与 confirm/reject/undo 控制，校验与事务归 [Workspace Host](../architecture/workspace-host.md)。长任务复用 JobRuntime worker gate，不另建调度器。
- 供应商：`llm::ApiProvider`（`Backend::{Standard, ChatGpt}`）与 `llm::vendors::{catalog, family, models, oauth, quota}`；`application::ProviderService` 编排视图、保存/移除、模型选择、登录（ChatGPT、OpenRouter、Codex 登录导入）、登出、模型列表、额度、连接测试与 `client()`。
- 配置：`settings.json` 的 `assistant` 段（`meetnote.settings.v2`）保存供应商、模型选择、写回策略与轮次上限；`credentials.json` 保存 API key 与 ChatGPT token。v1 设置在内存迁移，旧 connection profile 丢弃，用户需在供应商页面重新接入。
- UI：桌面在 Models 与 Settings 之间提供 Providers route；页面与交互验收归 [UI 计划](ui-plan.md)。
- 评测：`scripts/assistant-eval` 的 runner 直接请求供应商端点，不经过 Rust Loop；`run_matrix_litellm.py` 与 `litellm_deepseek_smoke.py` 经 `eval_settings.py` 读取 v2 settings 与 `credentials.json` 中的 API key。语义质量矩阵归[渐进式 Benchmark 计划](audio-quality-plan.md)的 B4。

## 当前状态

“已实现”指代码存在；“自动测试”指 `cargo test --workspace` 覆盖的测试；“真实验证”指对真实供应商端点执行并记录结果。

| 能力 | 已实现 | 自动测试 | 真实验证 |
|---|---|---|---|
| Tool Loop、Skill 场景、Workspace 工具、两种写回策略、receipt/undo | 是 | `assistant_runtime` 与 `assistant_service` 单元测试（fake/stub provider）覆盖取消、错误修正、confirm_each 暂停与 receipt | 未在当前供应商实现上执行 |
| 三协议 adapter 与 `ApiProvider` | 是 | `crates/llm/tests/protocol_and_http.rs`，loopback stub | 未执行；`real_upstream_three_protocols_text_and_tools` 为 ignored，需 `MEETNOTE_RUN_REAL_LLM=1` 与端点/模型/key |
| ChatGPT backend（Codex SSE Responses）与模型列表 | 是 | `crates/llm/tests/vendors_http.rs`，loopback stub | 未执行 |
| 供应商目录、latest per family | 是 | `crates/llm/src/vendors` 单元测试 | 目录 URL 与真实 `/models` 列表未核对 |
| 多 API 端点、跨 API 模型聚合与 native 规则 | 是 | `crates/llm/src/vendors` 单元测试；`vendors_http.rs` 的 DeepSeek 式聚合与 `native_endpoints`；`crates/application/tests/providers.rs` 的 magpie 式 stub 分别到达 `/v1/messages`、`/v1/responses`、`/v1/chat/completions` | 未对真实 magpie 网关或供应商执行 |
| ChatGPT/OpenRouter PKCE、token 刷新与轮换持久化、Codex 登录导入 | 是 | `vendors_http.rs` 与 `crates/application/tests/providers.rs`，loopback stub | 未执行 |
| 额度：ChatGPT `wham/usage` 与 reset card；OpenRouter/DeepSeek/Moonshot 余额 | 是 | loopback stub | 未执行 |
| `credentials.json`（0600、原子、加锁、拒绝宽权限）与 v1→v2 迁移 | 是 | `crates/settings` 单元测试 | 不适用 |
| Providers 页面 | 进行中 | 归 UI 计划 | 未执行 |

## 待办与验证方法

- //TODO - 真实供应商连接：对 featured 供应商（Anthropic、OpenAI、Google、OpenRouter、DeepSeek、Moonshot、xAI）各用真实 API key 执行 `ProviderService::test` 与一次 `/models` 读取，记录 base URL、请求路径、requested/resolved model、文本与工具调用结果及失败 code；结果写入 `docs/quality/` 回执，key 不入库。
- //TODO - ChatGPT 订阅：在本机浏览器完成 PKCE 登录（回调 `127.0.0.1:1455`），验证 Codex backend 文本与工具调用、模型列表、`wham/usage` 窗口与 reset card；把存储的 `expiresAt` 改到 5 分钟内，验证 `client()` 先刷新并在使用前持久化轮换后的 token，旧 token 不再使用。
- //TODO - OpenRouter PKCE：真实登录后验证换得的 key 写入 `credentials.json`、供应商被启用、`/credits` 余额可读；DeepSeek 与 Moonshot（`.cn` 与 `.ai` 两个区域）分别验证余额币种与数值。
- //TODO - latest per family：对上述供应商的真实 `/models` 列表输出 `latestPerFamily` 结果，人工核对每个族只保留最新版本且未误合并尺寸/变体；误判以单元测试固定后再修规则。
- //TODO - 桌面端到端：用一个已验证的真实供应商，在 GPUI 中分别以 automatic 与 confirm_each 执行 organize 与 partition，回读文档 revision、receipt 与 undo；取消运行时确认已提交 receipt 保留。
- //TODO - Loop 对照 pi-agent-core：方法见 [Agent Host](../architecture/agent-host.md#最小-tool-loop)。
- //TODO - Assistant 评测：用 v2 settings 指定真实供应商运行 `scripts/assistant-eval/run_matrix_litellm.py`，确认从 `credentials.json` 读取 key 且输出与回执不含 key；语义质量门槛按 B4 冻结后再执行。

## 验收

- 基础检查：`cargo fmt --all -- --check`、`cargo check --workspace`、`cargo test --workspace`。
- 每个供应商的“真实验证”只以对应回执为准；loopback stub 测试、编译通过与有效非 200 响应都不能替代成功的文本与工具调用。
- 连接测试只证明连通与工具支持，不证明任务质量；语义质量由 B4 单独验收。
