# Assistant 与 Agent 接入计划

## 目标与边界

Assistant 是 Agent Host 的内置 UI 客户端，可选择 Built-in Agent、Codex 或 Claude Code backend。文档问答、整理、语义拆分与树调整是 profile/Skill 和 Workspace 工具组合，不是 Host 内建领域职责。

运行时设计见[Agent Host](../architecture/agent-host.md)，实现迁移以[架构对齐计划](architecture-alignment.md)为准。此计划保留文档场景和验收，不另建一个包含全部文档逻辑的 AgentGateway。

## 实现基线

当前 Assistant 仍是占位客户端。五种 typed tools、静态 Skill、JSON CLI、LLM loop、长文档 Map/Reduce、三种非流式 HTTP adapter、连接/凭据设置、确认/undo UI 与 mutation persistence 属于已确定的设计与待实现范围。实现后仍需按当前验收要求重新验证，不能仅凭设计文档宣称完成。

保存实现缺少通用 session/resume、多 Agent backend、实时事件 UI 与统一 JobRuntime 接入。其文档工具、场景与循环需分离，不重写已完成的能力。

## 文档场景与不变量

| 场景 | 输入与允许操作 | 保证 |
|---|---|---|
| ask | 用户选定文档的投影，读取工具 | 不写文档；可保留独立会话历史 |
| organize | 标题、标签、总结、Markdown、document_status patch | revision 校验；processing/results 只读 |
| partition | 父文档真实 segment ID，连续子范围 | 完整覆盖、无重叠与空洞 |
| extract | 父文档真实 segment ID，选定连续范围 | 可留空洞，必须报告未覆盖范围 |
| move | reparent 或同级 reorder | 来源、循环、三层上限与受影响 revision 校验 |

语义边界由 Skill/模型选择，Workspace 将真实 segment ID 映射成时间并验证。一个子文档只表示一个连续范围；一次拆分只创建下一层。子文档事实由父文档权威处理结果裁剪，不接受模型生成的 ASR/diarization 事实。

现有 `resources/assistant/skills/*/SKILL.md` 保留为文档 profile 内容。默认只读 ask；profile 声明工具需求不能扩大用户授予的文档或读写范围。不建设通用 intent router 或为每个场景新增 Agent runtime。

## 工具、提交与外部客户端

`list_documents`、`read_document`、`update_document`、`create_semantic_children`、`move_document` 归 Workspace capability。Rust argument 类型产生 schema，解析检查结构，领域 validator 检查状态不变量；不手写第二份等价 schema。

保留 schema version、固定错误码、结构化 context、scope、幂等键、revision、provenance、pending 和 undo。automatic 默认可直接提交，confirm_each 在确认前不写入并在确认时重新校验。取消只阻止后续动作，不自动撤销已提交修改。多次写入不能因最后一次模型请求失败而隐藏已完成结果。

外部 CLI adapter 与内置 backend 使用同一服务；未来 MCP/RPC 可以有不同 envelope，但不复制 validator。工具桥接与 backend 的原始文件权限应明确区分，不能声称 CLI 限制了外部 Agent 的全部文件行为。

## Built-in 模型层与长文档

保留 canonical LLM request/output、Chat Completions、Anthropic Messages、OpenAI Responses adapter 及 LiteLLM 连接实现。LiteLLM 是 Built-in 的现有接入路径，不是 Codex/Claude Code 的强制依赖。凭据通过 credential store，配置只存引用；日志、远程文档投影与错误必须避免泄漏凭据及不必要的本机 locator。

文档 context assembly、segment/speaker turn 分块与 Map/Reduce 属于文档场景层。保留短文档路径、相邻上下文、来源 ID、统一轮次与资源预算；不能把 chunk 当语义边界。未知模型上下文上限须显式处理，不静默截断。真实 token/成本与内存仍需测量，字节预算与 fixture 不构成真实模型质量证据。

Host 统一事件可以承载当前完整响应和阶段进度。真正的 token 流、backend session 恢复与会话持久化按能力逐项实现，不再把非流式或禁止对话持久化冻结成产品约束。

## 验收

1. **共享 Workspace 工具**：内置与 CLI 使用同一 fixture；检查投影、固定错误、权限、revision、范围、三层树、幂等、confirm/reject、undo 和附件保留；故障时无无法恢复的半成品。
2. **会话与调度**：创建、输入、取消、事件顺序、失败与终止；写后模型失败仍显示 receipt；resume 不支持时明确返回能力错误，支持时不会重放写副作用。
3. **协议与 backend**：保留三种 HTTP fixture 的文本/tool round-trip、认证、限流、timeout、畸形响应与 redirect 隔离测试。真实 LiteLLM 代理搭配 fixture upstream 只证明代理兼容；Codex/Claude Code 分别需要真实会话集成证据。
4. **语义质量**：复用 48 个合成 fixture 与 QMSum/AMI 转换、评测脚本，但区分 gold 工具回放和模型独立预测。先保证每个 fixture 有独立可判定任务意图，再评估边界、覆盖、引用和总结。公开数据需固定来源、license 与 checksum。
5. **长文档与发布**：跨块主题、重复/遗漏、prompt injection、取消、失败、预算与资源；真实模型问答、两种写回策略和拆分矩阵仍需验证。结构正确与模型质量分别报告，不把测试代码或无 ground truth 输出写成生产验收。

实施顺序统一引用[架构对齐计划](architecture-alignment.md)，不再要求从空白重做已有的工具和协议实现。
