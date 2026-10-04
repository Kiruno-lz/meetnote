# Agent Host 与 Capability 接入计划

## 三项交付目标

1. **完整 capability 接口化**：Workspace、Audio、Settings 的约定操作经插件可访问的同一版本化接口调用，统一描述、授权、typed 参数/结果、固定错误与任务关联；不复制所属服务。
2. **Agent Host 得到真实回应**：Host 用最短输入实际调用 Agent/provider 接口，识别真实响应并正确解析字段；有效非 200 错误也可证明连通和错误解析，不要求其完成业务任务。超时、本地伪造错误或仅启动进程不算真实回应。
3. **Codex 与 Claude Code 经 Host 完成任务**：Codex（Luna medium）与 Claude Code 分别发现预置 Skill/tools，经 Agent Host 调用 capability，在错误反馈后修正调用并完成一次有回读的任务。直接调用服务、直接写文件或绕过 Host 的 CLI 不算验收。

MeetNote 不建设完整 Agent Runtime。外部 Agent 自管 Session、Context、Loop；Built-in 仅实现最小模型/工具调用循环。语义拆分、总结、多步规划等属于 Skill/插件扩展，不成为 Host 内置能力。Host 只做能力绑定、必要的协议规范化和调用控制。

## 范围与接口

- 本分支负责三服务 capability、Host 入口/桥接、连接配置、最小 Built-in 和协议 adapter。Workspace 持久化/恢复进度仅见 [Workspace Host](../architecture/workspace-host.md)；UI 更新见 [UI 计划](ui-plan.md)；分层质量测试见 [Benchmark 计划](audio-quality-plan.md)。
- 完整接口化覆盖当前约定操作：Workspace 列表/读取/创建/patch、语义结果提交、move/reorder、confirm/reject/undo；Audio 能力/probe、启动、查询/进度、取消/结果；Settings 字段级读取/patch、连接和写策略配置。不要求新增搜索、索引或底层算法。
- Rust 类型生成输入/输出 schema；服务负责事实 validator，插件负责语义，Host 不再维护一套模板或业务校验。各服务结果保持原类型，不把 Audio/Settings 伪装为文档结果。
- 用户/宿主授予的范围只能收窄；插件、Skill、请求和子进程不自行扩权。固定错误包含可定位 context 和 retryable；未知能力/版本、越权和非法参数明确拒绝。
- 自动写回默认开启，可在 Settings 关闭；确认前不写，失败/取消仍返回全部已提交 receipt，重试不重复提交。这些是服务调用保证，不要求本分支重建事务恢复或确认页面。
- connection profile 留在 AppSettings，保留 API key/API key file；目标/凭据/写策略分权并脱敏。保留 Chat Completions、Anthropic Messages、Responses 和 LiteLLM/自定义端点 adapter；不建设费用系统或完整模型矩阵。
- 长任务复用 JobRuntime/所属服务状态；外部传输只桥接，不创建平行调度器或任务事实存储。

## 当前进度与理解性验收

2026-09-24 两轮均先执行基线（`cargo fmt --all -- --check`、`cargo check --workspace`、`cargo test --workspace` 全绿：首轮 262 passed、次轮 264 passed，均 10 ignored），按“复现 → 测试红线 → 修复 → 转绿”闭环各缺口，再在最终二进制上执行 Host provider 探测、Codex 与 Claude Code 任务。三项目标均已验收；provider 探测只证明连通与解析，不证明模型质量；UI、音频质量与真实音频模型不在本计划验收范围。

| 目标 | 当前状态 |
|---|---|
| capability 接口化 | 已完成：三服务经 `AgentHost::dispatch` 同一版本化接口；Settings 只暴露 `snapshot/read/patch` 与一套服务级写规则；typed 参数拒绝非对象；三服务描述与调用 envelope 只由 `agent host discover` 从同一 Rust 类型生成，与实参实返一致 |
| Host 真实回应 | 已完成：`meetnote agent host probe` → `AgentHost::provider_probe` → 桌面同一凭据解析与 `ConnectionConfig` 映射 → OpenAI Responses adapter → 已配置连接 `https://api.deepseek.com`，最终二进制返回 `ok:true`、requested `deepseek-v4-flash`、resolved `deepseek-flash`、`stopReason: Complete`、text `ready`（0.77s）；probe 路径有 L1/L2 自动化回归 |
| Codex / Claude Code 经 Host 完成任务 | 已完成：Codex（`gpt-5.6-luna` medium）与 Claude Code（`claude-sonnet-5`）分别在隔离工作区、以不引用源码的同一任务提示完成 organize 任务，轨迹见下方验收记录 |

### 计划内项目验收

- [x] **所有插件可访问的 Audio 方法执行相同所有权检查**：移除 `AudioCapabilityPort` 上绕过 registry 的公开 `snapshot(job_id)` 和 `cancel(job)`，统一使用带 `project_id` 的受控查询/取消接口；registry 对跨 project 的查询、取消、配置、结果和释放返回固定 `AccessDenied`。`audio_job_handles_cannot_cross_project_ownership_boundaries` 先覆盖跨 project 查询/取消红线，再验证 owner 能查询/取消且被拒绝任务保持 `Running`；修复后通过。
- [x] **撤销与启动注册具有一致生命周期边界**：用 `lifecycle` 锁保护撤销与最终授权检查/注册；启动在检查后若授权已撤销则取消并丢弃已创建任务，保留既有完成结果查询，不增加第二套调度器。`revoke_cannot_finish_before_a_checked_start_registers_a_job` 用屏障固定“检查通过 -> revoke 完成 -> 注册继续”的红线，修复后验证返回 `AccessDenied` 且 registry 为空。
- [x] **统一实际 wire 类型与描述**：Audio/Settings 输入输出使用共享 Rust wire 类型生成 schema；Audio snapshot 将服务原生 `job_id` 映射为协议 `jobId`，Settings snapshot 的各 section 保持 `{revision, value}` 包装，`read.section` 为四值枚举 `SettingsSection`。`capability_wire_schemas_match_their_host_outputs` 经实际 Agent Host 逐项执行 probe/start/query/cancel/result 和 snapshot/read/patch 并校验 discovery required 字段；`settings_read_descriptor_enumerates_the_only_readable_sections` 先复现 `section` 仅声明为 string 的红线（首轮 Codex 因此把 `assistant.writePolicy` 当作 section 被拒），修复后通过。
- [x] **Settings port 对窄授权执行收敛与脱敏**：`read_*` 与可信 grant 求交；writePolicy-only 不可读取目标/凭据或修改连接，原始凭据不进入任何投影。
- [x] **Host 的 Settings 操作使用 typed 参数**：Host snapshot/read/patch 对非对象、额外字段和未知 section 返回 `SETTINGS_ARGUMENTS_INVALID`。
- [x] **Settings revision 与提交保持一致**：`patch_if_equal` 校验 revision、应用 patch 后以同一 expected `AppSettings` 调用 `update_if_equal`，返回原子提交的 revision 与投影，不提交后重读其他请求结果。`patch_rejects_a_stale_snapshot_after_a_concurrent_commit` 用双屏障在检查与提交之间插入并发更新，返回 `RevisionConflict` 且保留新值。
- [x] **公开 Settings snapshot 从同一配置快照生成全部结果**：`SettingsCapabilityPort::public_snapshot` 只读取一次 `AppSettings`，从同一值生成顶层 revision、连接、目标、凭据状态和写策略 section，保持现有 wire 结构且不增加状态存储。`settings_host_snapshot_is_one_consistent_configuration_version` 经 Host snapshot 路径用屏障在读取期间插入配置更新，先在旧实现下复现 section revision 混版红线，修复后验证所有 section revision 与顶层 revision 相同。

既有 fake Audio feature 隔离、port/Loop 工具收窄、pending 首项暂停、重读 hash 修正、Host 可信 grants、typed control、可替换 binding 与持久 Host 不再重复列待办。

### 本轮项目验收

- [x] **移除或收敛旧 Audio CLI 的平行执行路径**。删除 `agent audio call start/query/cancel`、`agent audio worker`、`AudioCliJob`、专用任务文件和 cancel 文件路径；Audio 的启动、查询、取消统一经 `agent host call` 使用同一 port、授权和 JobRuntime 生命周期。`external_agent_host_rejects_cross_document_audio_requests_without_side_effects` 先复现 A grant/B 请求的越权红线，再验证 start/query/cancel 均返回 `AUDIO_ACCESS_DENIED`、不创建任务状态；Host 正常生命周期与跨进程持久 Host 回归通过。
- [x] **扩展 capability 同样拒绝不支持的版本**。`AgentHost::dispatch` 在调用任意 binding 前读取其 descriptor 并统一比较 `capabilityVersion`；不匹配时返回 `CAPABILITY_VERSION_UNSUPPORTED`，不进入 binding，业务参数仍由所属 capability 校验。`capability_binding_is_replaceable_without_changing_builtin_dispatch` 覆盖 EchoBinding 版本 7 成功、版本 8 固定拒绝且 dispatch 计数不增加，并保留内置版本拒绝回归。
- [x] **全部 Assistant 字段授权不等于整个 AppSettings 读取授权**。删除 `settings_mutation` 在四个 assistant grant 齐备时序列化完整 `AppSettings` 的分支；所有 patch 结果只含 `{"assistant": …}` 已授予 section 与 `credentialStatus`。复现：旧二进制经 Host 四字段 patch 返回 `SENTINEL_ASR_MODEL`/`SENTINEL_LANGUAGE`/`SENTINEL_LIBRARY`。`assistant_grants_never_project_unrelated_settings_or_credentials` 以四字段和 `assistant` 两种可信 grant 执行 snapshot、四个 read、patch，断言结果无哨兵值、持久配置仅 `maxToolRounds` 变化且凭据保留。
- [x] **Settings 只保留一个公开入口与已声明契约**。删除 `agent settings describe`、`agent settings update`、`agent call settings <op>` 及其专用请求/凭据恢复/错误映射代码；Settings 只经 `agent host call`、`agent host serve` 与进程内 `AgentHost::dispatch` 访问，同一 descriptor、参数拒绝、字段授权与固定错误。复现：旧 `agent call settings snapshot` 接受 `arguments: []` 并返回 `{revision, settings}`。`removed_settings_cli_entries_cannot_bypass_the_host` 验证三个旧入口退出码 3、无 stdout、配置不变；`settings_discovery_declares_only_the_callable_typed_operations` 验证 discovery 只声明 `snapshot/read/patch` 且 snapshot required 字段与实返一致。
- [x] **脱敏读取后的写回不隐式清除凭据**。删除整份 `AppSettings` 的 `update` 操作（请求返回 `SETTINGS_OPERATION_UNKNOWN`），公开写入只剩 typed `patch`：未提供字段保持原值，凭据只经显式 `CredentialUpdate::{Direct, File, Clear}` 变更；MoonBit 插件的 `settings-update` 改为 `settings-snapshot`。复现：旧入口“读取 → 只改 `maxToolRounds` → 写回”后持久 `apiKey` 被清空。`settings_contract_is_identical_through_every_retained_host_entry` 在带 apiKey 与 apiKeyFile 及 ASR/language/speakerLibraryPath 哨兵的独立工作区中，分别经 `host call` 与 `host serve` 执行同一场景：writePolicy 读取 → patch → 回读、stale revision 返回可重试 `SETTINGS_REVISION_CONFLICT`、writePolicy grant 改凭据被拒、显式轮换 direct 凭据与清除 file 凭据；每步断言输出无哨兵/新密钥，并断言持久配置与两入口结果完全一致。
- [x] **patch 字段归属与凭据目标校验为一套服务级规则**。`apply_patch` 按读取投影确定唯一归属：name 归 connections；protocol/baseUrl/model/authMode 及 contextWindowTokens/timeoutMs/maxOutputTokens 归 connectionTargets；credential 归 credentials；只有端点字段变更要求同一 patch 显式处理已配置凭据，预算字段不触发。`patch_field_ownership_matches_read_projection_and_binds_credentials_to_targets` 先复现预算字段可由 connections 写、targets 不可写的红线，修复后验证仅获目标授权无法把旧凭据带到新端点、拒绝后配置不变、targets+credentials 显式 Clear 可以提交；上述两入口场景覆盖 `SETTINGS_CREDENTIAL_TARGET_CONFLICT` 与 `SETTINGS_FIELD_NOT_GRANTED` 的相同结果。
- [x] **typed 参数拒绝非对象输入**。serde 结构体会按位置接受 JSON 数组（旧 Host 接受 `read_document` 参数 `["allowed","metadata"]`）；共享 `decode_object_arguments` 在 Workspace、control、Audio、Settings 解码前拒绝非对象，分别返回 `TOOL_ARGUMENTS_INVALID`、`AUDIO_ARGUMENTS_INVALID`、`SETTINGS_ARGUMENTS_INVALID` 并保留 service/operation context；Audio typed 参数错误不再使用通用 `CAPABILITY_ARGUMENTS_INVALID`。`builtin_capabilities_reject_non_object_arguments_with_service_codes` 覆盖并验证配置不变。
- [x] **Host 得到真实回应**。最终二进制在仓库工作区执行 `meetnote agent host probe`，路径与结果见上表；0.85s 内返回，`resolvedModel` 取自 provider 响应。
- [x] **Codex 经 Host 完成一次有纠错与回读的任务**。Codex CLI 0.155.0-alpha.9.2（`/Applications/ChatGPT.app/Contents/Resources/codex`）、`gpt-5.6-luna` medium、workspace-write 沙箱、`--ignore-user-config`。夹具由 Host `create_document`/`update_document` 在隔离工作区生成（revision 1），提示给出过期交接值（revision 0 与旧 hash），协议只来自 Host discovery。最终二进制上的轨迹：读取 `./skills/*/SKILL.md` 选定 `organize` → `agent host discover` → 唯一的持久 `agent host serve`；`settings.read` 把授权字段写成 `writePolicy` 得到 `SETTINGS_FIELD_NOT_GRANTED` 后改为 `assistant.writePolicy`（automatic）→ `read_document`（revision 1）→ 以交接值 `update_document` 得到 `DOCUMENT_REVISION_CONFLICT`（context 含实际 `markdownSha256`）→ 按实际 revision/hash 重写 title/summary（mutation `req-005:call-005`，revision 1→2，actor `external_agent`）→ `read_document` 回读新值且 markdown hash 不变。线级轨迹取自 Codex rollout 中 Host 进程的实际输出；无其他 MeetNote CLI、源码读取、`./data` 访问或文件编辑；工作区仅 `state.json` 与一条 mutation journal 变化，独立 Host 回读一致。
- [x] **Claude Code 经 Host 完成一次有纠错与回读的任务**。Claude Code 2.1.260（`/opt/homebrew/bin/claude`，本机登录态）以 `--safe-mode --setting-sources "" --strict-mcp-config` 运行，工具限 Bash/Read/Glob/Grep；debug 日志确认插件、skill 目录、claude.ai MCP 与 hooks 均未加载，询问全局 CLAUDE.md 中独有的字符串得到 NO。夹具与任务同 Codex，提示不引用源码。轨迹：读取 `./skills/*/SKILL.md` 选定 `organize` → `agent host discover` → 以 FIFO 与 keeper 进程保持单个 `agent host serve`（首个启动尝试在收到任何请求前被其自行终止并重启，全部 capability 请求由同一 Host 进程处理）；`settings.snapshot` 仅带 writePolicy 授权得到 `SETTINGS_FIELD_NOT_GRANTED`（context.field `assistant.connections`）后改用 `read writePolicy`（automatic）→ `read_document`（revision 1）→ 以交接值写入得到 `DOCUMENT_REVISION_CONFLICT` → 按实际 revision/hash 写入（mutation `r5:c5`，revision 1→2，actor `external_agent`）→ 回读新 title/summary 且 hash 不变。线级轨迹取自 Host 进程自身写出的 `host-io/out.log` 与 `~/.claude/projects/<cwd>/<session>.jsonl`；无其他 MeetNote CLI、源码读取、`./data` 访问、文件编辑或 `/tmp` 写入；工作区仅 `state.json` 与一条 mutation journal 变化，独立 Host 回读一致。
- [x] **旧 discovery 入口已删除**。`agent discover --json` 曾以 `transport: one-shot-cli` 声明只能经 Host 调用的 `settings.app`/`audio.processing`，且 Settings descriptor 固定按 `assistant` 全字段生成；现已删除，三服务描述只由 `agent host discover` 按可信 grant 提供。`legacy_one_shot_discovery_is_removed_in_favor_of_host_discovery` 先复现旧入口仍返回 0，修复后验证退出码 3、无 stdout；`external_agent_discovers_audio_capability_through_the_same_adapter` 与 `external_agent_discovers_and_calls_all_services_with_fixed_schema_and_auth_errors` 改用 `agent host discover`。
- [x] **Host discovery 描述调用 envelope**。`CapabilityCallRequest`/`CapabilityCallResponse` 及其授权、错误类型派生 `JsonSchema`，`agent host discover` 输出 `envelope.request`/`envelope.response`，与 descriptor 同源生成。`host_keeps_trusted_grants_and_exposes_exact_descriptors` 先复现 discovery 缺少 envelope，修复后断言 request required 恰为 `arguments/callId/capabilityId/capabilityVersion/operation/requestId/schemaVersion`、两者 `additionalProperties:false`，且 `wire_call` 实发字段与 Host 成功/错误实返字段均已声明；两个外部 Agent 的验收提示均不引用源码。
- [x] **Host probe 复用桌面凭据解析并有自动化回归**。`run_host_probe` 删除自有 `ConnectionConfig` 映射与 `.ok()` 读 key，改用 `DesktopAssistantService::active_provider`（profile 校验、`resolve_api_key`、`connection_config`）；凭据/校验错误在创建 provider 前以 `{hostCallId, ok:false, error}` 与退出码 2 返回，provider 错误保持原结构并脱敏。复现：旧实现在 apiKeyFile 不可读时仍无凭据请求 provider，stub 返回 200 即显示 `ok:true`。`host_provider_probe_sends_one_minimal_toolless_request` 断言单次请求无 tools、`tool_choice` 为 None、`max_output_tokens` 为 16；`host_probe.rs` 以本地 HTTP stub 断言 200 时 `resolvedModel` 取自响应、401 返回 `PROVIDER_AUTHENTICATION_FAILED`（context.status 401），不可读 apiKeyFile 返回 `ASSISTANT_API_KEY_FILE_READ_FAILED` 且 stub 未收到请求。

最终验证：`cargo fmt --all -- --check`、`cargo check --workspace`（0 warning）、`cargo test --workspace`（269 passed / 0 failed / 10 ignored，ignored 为需显式模型/音频或本地 LiteLLM fixture 的测试，未运行）与 MoonBit native 构建通过。

## 验收硬要求

三项目标分别通过，不互相替代：

1. **接口完整与可替换**：无 GPUI，真实 MoonBit 插件经 Host 调用三类真实服务 port 的全部约定操作；Audio 可用 fake backend。新增测试 capability 只改装配，不改 Loop。描述/schema 与实参实返一致。
2. **边界正确**：覆盖文档/job/字段越权、非法参数、过期 revision、处理事实只读、服务不可用；返回预期固定错误，既无旁路也无隐式扩权。两种写策略、幂等、取消和 receipt 保留正确。
3. **Host 连通**：短请求产生来自实际 Agent/provider 的可解析回应，包括有效错误响应；记录实际调用路径，不要求业务内容质量。
4. **Codex 与 Claude Code 任务完成**：Codex（Luna medium）与 Claude Code 分别经 **外部 Agent → Agent Host → 插件可访问 capability → 服务 → Host → 外部 Agent** 完成一次合法写回与回读；轨迹包含 Skill/tools 发现、错误反馈、调用修正和最终结果。直接 capability CLI 调用只算接口测试。
5. **最小实现与回归**：有限 Loop 可结束、可取消、不丢副作用；相关测试及 fmt/check/test 通过。只保留必要验收记录，不要求 Host 实现语义拆分、不要求 UI 更新或质量跑分。
