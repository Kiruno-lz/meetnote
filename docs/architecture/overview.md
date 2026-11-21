# 系统架构总览

## 产品与事实基线

MeetNote 把音频转换成带时间戳、说话人与处理来源的文档，以用户可读文件维护最多三层项目树。会议、日记和课程是使用场景，不是内核类型。

Workspace Host、Audio Host、Agent Host 和 Rune UI 是 MeetNote 的四个基础层。它们是现有 crate 的职责归类和依赖方向，不要求在代码中拆成同名 crate，也不表示每一层都已经有完整独立 API。当前实现进度和缺口见[架构对齐计划](../plan/architecture-alignment.md)。

## 四个基础层

```text
                 Rune UI
          交互、状态投影、任务反馈
                    │
      ┌─────────────┼─────────────┐
      ▼             ▼             ▼
Workspace Host  Audio Host   Agent Host
文档、文件、     解码、模型、   内置 Assistant、
结构校验、恢复   provider、处理  LLM 供应商、最小 loop
      ▲                           │
      └── WorkspaceCapabilityPort ┘

应用入口和 CLI 负责装配四个基础层。
```

Workspace Host 维护用户工作区和文档事实；Audio Host 维护音频事实、处理编排、模型资源和音频 provider；Agent Host 承载内置 Assistant 的 LLM 供应商与最小 Tool Loop；Rune UI 负责交互和状态投影。Assistant 只通过 `WorkspaceCapabilityPort` 读写文档，不把 Workspace 或 Audio 的领域逻辑复制到 Agent Host。

四个基础层是责任边界，不是必须拆出的 crate 或进程，均为内置 Rust 实现，由入口显式装配；不建设插件或 Extension 运行时。Rune UI 只做客户端和状态投影。

## 现有 crate 的基础层归属

### Workspace Host

| crate / 代码区域 | 职责 |
|---|---|
| `crates/application` 的 `WorkspaceService`、`DocumentService`、`document_commands` | 工作区用例、文档命令、revision、树关系和资产关联 |
| `crates/persistence` | 文档包、repository、scanner、watcher 和文件恢复 |
| `crates/domain` | 文档状态、关系模型、结构校验和跨层共享的纯领域算法 |
| Workspace 派生索引服务（待实现） | 全文索引、embedding index、解析状态和搜索缓存；不能成为事实来源 |

### Audio Host

| crate / 代码区域 | 职责 |
|---|---|
| `crates/audio` | 解码、downmix、重采样、chunk 和离线窗口 |
| `crates/application` 的 `ProcessingService`、`long_audio`、`speaker_identity` | 音频处理编排、窗口 pipeline 和身份匹配 |
| `crates/asr`、`crates/diarization` | ASR/diarization registry、manager 和 provider contract |
| `crates/asr-*`、`crates/diarization-*` | 具体模型族和 native backend |
| `crates/models` | 模型 catalog、下载、安装、校验和 probe |

### Agent Host

| crate / 代码区域 | 职责 |
|---|---|
| `crates/llm` | 三种协议 adapter、`ApiProvider`、供应商目录、模型列表、OAuth 登录与额度读取 |
| `crates/application` 的 `AgentRuntime`、`WorkspaceAgentScene`、Skill | 最小 Tool Loop、场景装配与工具收窄 |
| `crates/application` 的 `ProviderService` | 供应商配置、凭据、登录、模型选择与客户端构造 |
| `WorkspaceCapabilityPort` | Assistant 可调用的文档工具；执行规则归 Workspace，不复制进 Agent Host |

### Rune UI 与应用入口

| crate / 代码区域 | 职责 |
|---|---|
| `apps/meetnote` | Rune 桌面页面、交互和状态投影；CLI 入口、配置装配和四个基础层的 composition root |
| `crates/ui-text` | 界面文案与 locale |

### 跨层基础设施

| crate / 代码区域 | 职责 |
|---|---|
| `crates/contracts` | 跨层请求、结果、能力、模型引用和统一错误；具体 contract 按服务归属 |
| `crates/runtime` | JobRuntime、任务状态、进度、取消和 worker gate |
| `crates/settings` | 设置、选择和本地配置持久化；供应商凭据存于独立 `credentials.json` |

全局 `runtime::spawn_blocking` 承载匿名后台读取，复用共享 worker gate；其 context 由 future/worker 持有，不向私有 registry 累积任务历史，未轮询即丢弃、正常完成和 worker panic 都不会留下注册记录。显式 `JobRuntime::create_job` 仍保留可查询的任务状态，按调用方生命周期移除。

共享 gate 复用锁定的 `async-lock` semaphore，先异步取得名额，再创建阻塞 worker；排队不创建等待线程。未取得名额就丢弃 future 时，释放其捕获资源；可取消入口用 `event-listener` 等待 token，不进行定时轮询。名额移交真实 worker 持有直至退出，不能因调用方 future 被丢弃或 token 已取消而提前释放仍在执行的工作预算。该限制是 worker 数量，不能替代各 native session 的线程预算或运行中合作取消。

## 数据与依赖

```text
音频 -> Audio Host providers -> 匿名聚类 -> 身份匹配 -> 对齐
     -> Workspace Host 写入 document.md + state.json
     -> 扫描/监听 -> 三层树与 Rune UI
Assistant + profile/Skill -> WorkspaceCapabilityPort -> 同一服务校验
```

短音频保留整文件 pipeline，长音频使用有上下文重叠的离线窗口，具体参数与模型族协议见[Audio Host 与模型后端系统](audio-host.md)。语义边界、标题、标签、总结由 Assistant 按 Skill 决定；Workspace 负责验证与执行。

`domain` 不依赖 IO/UI/推理库；`application` 依赖能力 trait，具体实现由入口装配。Workspace 不依赖 Assistant 才能读写；Audio 不依赖 Assistant 才能处理；Rune UI 不反向承载 Host 的领域实现。新增与现有 contract 兼容的 backend 不修改领域与 UI；新增真实能力才演进 contract。

## 详细边界

- [Workspace Host](workspace-host.md)：文件、外部编辑、窄操作与恢复。
- [Audio Host 与模型后端](audio-host.md)：音频处理、provider、backend 和模型生命周期。
- [Agent Host](agent-host.md)：最小 Tool Loop、LLM 供应商与凭据。
- [Rune UI 计划](../plan/ui-plan.md)：四个基础层之上的交互客户端和状态投影。
- [文档状态](document-state.md)与[架构决策](decisions.md)。
- [架构对齐](../plan/architecture-alignment.md)负责迁移顺序；[工程计划](../plan/implementation-plan.md)负责生产验收。
