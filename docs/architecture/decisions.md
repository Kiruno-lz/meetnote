# 架构决策

## 当前架构决策

1. MeetNote 是 Rust-native、本地优先的音频处理与 AI 可读文档工作区，不绑定会议场景。
2. 桌面主程序使用 Rust 与 GPUI，不使用 Tauri/WebView；provider 可按需求使用受管理的外部进程，不强制全部实现同进程。
3. `domain` 只包含领域模型、校验和纯算法；`application` 编排用例；UI 不承载领域算法、文件 IO 或推理。
4. ASR 与 diarization 使用独立 registry、manager 和 provider；具体模型族位于独立 backend crate。
5. 新增兼容现有能力的 backend 增加实现与注册，不改 domain 或 UI；真实新增能力才扩展 contract。
6. 任意 ONNX 文件不是可直接运行的通用模型；模型族 adapter 必须固定输入、输出、特征、tokenizer、解码、时间戳和后处理。
7. 模型下载、缓存、checksum、安装和下载后 probe 统一由 Rust `models` crate 管理；backend 不自行下载。
8. probe 分为下载后 artifact probe、处理前 preflight 和推理时真实加载，三层结果不得互相替代。
9. 音频处理不是实时产品：短音频保留整文件 baseline，长音频使用重叠的有界离线窗口。
10. ASR 窗口拼接属于具体 ASR backend；diarization 窗口必须汇总真实 observation 后进行录音级聚类，不能拼接局部 speaker label。
11. Speaker Library 只在匿名聚类完成后做身份匹配；embedding space、阈值、分布、歧义和重叠检查必须兼容。
12. 所有 backend 输出先归一化到 contracts，再进入对齐、人工修正和文档保存。
13. 每个项目由 `document.md + state.json` 表达；文档目录是项目结构的唯一事实来源，不以 SQLite 或独立全局 project store 保存权威事实；派生索引可重建。
14. 项目树固定最多三层，由文档关系扫描派生，必须可丢弃和重建。
15. MeetNote 校验 schema、revision、父子关系、循环、层级与音频来源；无效外部修改必须产生可定位错误。
16. 复合录音的语义边界、标题、标签、总结和父子关系由 Agent 按 Skill 决定；MeetNote 不内置自动语义拆分器。
17. Assistant 是内置 Agent 客户端，与外部 Agent 使用相同 document service、权限和写回规则，不拥有特权路径。
18. `DocumentCommandService` 是内部 application 边界，不是外部 Agent 的强制 ABI；CLI、IPC 和其他 adapter 必须复用同一服务。
19. 处理任务状态、处理完成记录和文档工作流状态分离：`JobRuntime` 管理运行状态，`processing` 保存当前处理来源，`document_status` 保存长期知识工作流。
20. 语义编辑不得覆盖原始音频引用、ASR、diarization、对齐和 processing provenance；重新处理只替换当前处理结果。
21. 文档与 Speaker Library 写入采用临时文件和原子替换；涉及多个文件或多个文档的操作必须在生产化阶段补齐事务与恢复保证。
22. UI locale 与 ASR transcription language 是独立设置。
23. GPUI、模型清单和第三方推理依赖必须锁定；模型 revision、checksum、来源与 license 必须可追溯。
24. MeetNote 的基础框架由 Workspace Host、Audio Host、Agent Host 和 GPUI UI 组成；现有 crate 按职责归入这四个层次，不要求新增同名 crate。
25. Agent Host 管理通用会话与 backend；语义场景由 profile/Skill 组合，文档与音频工具分别归 Workspace 和 Audio。
26. LLM 协议 adapter 不是通用 Agent backend；Built-in、Codex、Claude Code 的能力与会话协议分别验证。
27. GPUI UI 只负责四个基础层的交互、状态投影和任务反馈，不承载 Workspace、Audio 或 Agent 的领域实现。
28. Extension 是可选的注册与执行机制，不改变四个基础层的职责；先验证内置 Rust 服务，再考虑 manifest 和 Wasm Component。
29. 生命周期撤销注册不撤销已提交副作用；事务/undo/会话历史与可丢弃缓存分开管理。
30. Workspace 的源文件音频、Markdown、项目 JSON 和 Speaker Library 是不可删除资产；全文索引、embedding index、解析状态和搜索缓存只能作为可重建派生状态存在。
31. 插件通过 Workspace Host 的版本化 capability port 访问开放能力；port 执行 Workspace 的范围、revision、结构和资产保护，不提供任意文件系统删除权限。

详细理由见[Agent Host](agent-host.md)、[Workspace Host](workspace-host.md)和[Extension](extension-system.md)。

## 禁止偏离

- 不恢复旧桌面技术栈，不为可选进程 provider 改写现有原生音频链路。
- 不把 backend 特有字段放入通用领域模型。
- 不让模型、下载任务、UI 状态或 Speaker Library 成为项目树事实来源。
- 不移除三层上限，不把它描述为临时 MVP 约束。
- 不将 VAD、技术窗口、speaker 切换或匿名 cluster 直接解释为语义项目。
- 不用短音频执行、合成测试或进程退出码代替长音频精度与资源验收。
