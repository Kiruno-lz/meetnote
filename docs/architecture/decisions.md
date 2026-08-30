# 架构决策

## 已确认

1. 使用 GPUI 构建原生桌面 UI。
2. 使用 Rust application layer 承载业务编排。
3. 不使用 Tauri、WebView 或 Python subprocess。
4. ASR 与 diarization 分别通过独立 backend manager 管理。
5. 允许 `whisper-rs`、`sherpa-onnx`、`diarization` crate 和未来 native/ONNX backend 并存。
6. `crates/` 下使用语义目录名，去除重复的 `meetnote-` 前缀。
7. 文档内容与文档自身 JSON 状态是事实来源；MeetNote 不维护独立的权威项目数据库，索引只能从文档扫描重建；不引入 SQLite。
8. 模型清单和下载生命周期协议迁移到 Rust model manager。
9. 新增 backend 不得修改 domain、application 和 UI 的稳定接口。
10. GPUI 在 macOS 上启用 `runtime_shaders`，将 shader 编译从构建期移到运行期，降低本地构建工具链要求。
11. 产品核心对象是 AI 可读文档，文档可以代表一天、一次会议或其他任意音频资料；不把领域模型锁定为 Meeting。
12. 音频资产、原始转录、说话人对齐结果和知识文档分层保存；每份文档自身的 JSON 状态保存其管理信息、语义项目树和来源引用。
13. MeetNote 提供文档扫描、读取、创建、编辑、状态更新和保存，并从目标目录的文档 JSON 实时构建和维护语义项目树；项目树固定最多三层，以控制复杂度；不维护独立的权威数据库，所有索引都必须能从文档自身 JSON 重建。
14. MeetNote 不实现语义边界判断、自动总结、自动标签或自动拆分；但必须解析和维护由文档信息表达的语义项目树派生视图。
15. 原始音频、原始 ASR、原始 diarization 和对齐结果不可被文档编辑覆盖；编辑结果以新的文档版本、修正记录或子文档保存。
16. MeetNote 必须向任意 Agent 提供稳定的机器可读文档读写边界；Agent 可以读取、编辑、更新文档状态以及按 Skill 写入拆分结果，MeetNote 必须校验三层约束、实时发现文档变化并更新项目树视图，但不能让派生视图取代文档 JSON。
17. 系统处理状态与文档工作流状态分离：前者由 MeetNote 写入文档 JSON，后者由 Agent 或用户按 Skill 写入同一文档 JSON 的独立字段；任一方不得覆盖另一方。
18. Assistant 页面作为内置 Agent 客户端长期保留，与任意外部 Agent 处于同一层级、使用同一文档管理服务和相同权限；Assistant 不拥有独立项目状态或特权写入路径。
19. 设置中的 UI locale 与 ASR transcription language 分离，为后续 i18n 保留独立字段。

## 明确不做

- 不把任意 ONNX 文件伪装成通用模型。
- 不在 GPUI View 中堆叠业务逻辑。
- 不在单个 manager 中实现所有模型族的预处理和后处理。
- 不把模型权重提交进仓库；模型按需下载到本地 cache，并经过 checksum 与 backend probe。
- 不把 LLM provider、语义拆分器或 Agent Skill 作为 MeetNote 的核心运行时依赖。
- 不把所有文档锁定为“会议项目”；项目树固定最多三层，且必须从文档自身 JSON 派生并可实时重建。
- 不移除三层项目树约束，也不把它描述为过渡性 MVP 限制。
- 不把“Agent 能够拆分文档”实现为 MeetNote 的语义判断功能；拆分规则、边界和子文档状态由 Skill 指导 Agent 写入文档，MeetNote 负责发现变化并更新派生树。

## 待验证但不阻塞分层设计

- whisper-rs 与 sherpa-onnx 在中文混合会议上的 WER/CER。
- sherpa-onnx 与 `diarization` crate 的 DER、重叠语音和 speaker 数量表现。
- Metal/CoreML provider 的内存和冷启动成本。
- 不同模型格式的转换与许可证边界。
