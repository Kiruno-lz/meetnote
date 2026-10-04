# 系统架构总览

## 产品与事实基线

MeetNote 把音频转换成带时间戳、说话人与处理来源的文档，以用户可读文件维护最多三层项目树。会议、日记和课程是使用场景，不是内核类型。

Workspace Host、Audio Host、Agent Host 和 GPUI UI 是 MeetNote 的四个基础层。它们是现有 crate 的职责归类和依赖方向，不要求在代码中拆成同名 crate，也不表示每一层都已经有完整独立 API。当前实现进度和缺口见[架构对齐计划](../plan/architecture-alignment.md)。

## 四个基础层

```text
                 GPUI UI
          交互、状态投影、任务反馈
                    │
      ┌─────────────┼─────────────┐
      ▼             ▼             ▼
Workspace Host  Audio Host   Agent Host
文档、文件、     解码、模型、   capability 绑定、
结构校验、恢复   provider、处理  薄适配、最小 loop
      │             │             │
      └─────── 显式 capability ───┘

应用入口和 CLI 负责装配四个基础层。
```

Workspace Host 维护用户工作区和文档事实；Audio Host 维护音频事实、处理编排、模型资源和音频 provider；Agent Host 提供 capability 绑定、薄适配与 Built-in 最小 Tool Loop；GPUI UI 负责交互和状态投影。Agent 通过显式 capability 调用 Workspace 或 Audio，不把这两个 Host 的领域逻辑复制到 Agent Host。

四个基础层是责任边界，不是必须拆出的 crate 或进程。核心 Workspace、Audio、Agent 可以内置 Rust 实现；GPUI UI 只做客户端和状态投影。Extension lifecycle 是可选的注册和执行机制，不改变四层归属。

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
| `crates/llm` | LLM provider 描述和协议适配边界 |
| Agent application、Agent contracts | 能力绑定、薄适配、Built-in 最小 Tool Loop；工作树已有实现，职责收敛见 Assistant 计划 |
| Workspace / Audio capabilities | Agent 可调用的显式服务；不把文档或音频领域逻辑复制进 Host |

### GPUI UI 与应用入口

| crate / 代码区域 | 职责 |
|---|---|
| `crates/ui` | GPUI 页面、controller、交互和状态投影 |
| `apps/meetnote` | GPUI/CLI 入口、配置装配和四个基础层的 composition root |

### 跨层基础设施

| crate / 代码区域 | 职责 |
|---|---|
| `crates/contracts` | 跨层请求、结果、能力、模型引用和统一错误；具体 contract 按服务归属 |
| `crates/runtime` | JobRuntime、任务状态、进度、取消和 worker gate |
| `crates/settings` | 设置、选择和本地配置持久化 |

## 数据与依赖

```text
音频 -> Audio Host providers -> 匿名聚类 -> 身份匹配 -> 对齐
     -> Workspace Host 写入 document.md + state.json
     -> 扫描/监听 -> 三层树与 GPUI UI
Agent Host + profile/Skill -> Workspace/Audio capability -> 同一服务校验
```

短音频保留整文件 pipeline，长音频使用有上下文重叠的离线窗口，具体参数与模型族协议见[Audio Host 与模型后端系统](audio-host.md)。语义边界、标题、标签、总结由 Agent 场景层决定；Workspace 负责验证与执行。

`domain` 不依赖 IO/UI/推理库；`application` 依赖能力 trait，具体实现由入口装配。Workspace 不依赖 Agent session 才能读写；Audio 不依赖 Agent 或插件运行时才能处理；GPUI UI 不反向承载 Host 的领域实现。新增与现有 contract 兼容的 backend 不修改领域与 UI；新增真实能力才演进 contract。

## 详细边界

- [Workspace Host](workspace-host.md)：文件、外部编辑、窄操作与恢复。
- [Audio Host 与模型后端](audio-host.md)：音频处理、provider、backend 和模型生命周期。
- [Agent Host](agent-host.md)：Capability API、最小 Tool Loop 与外部 Agent 边界。
- [GPUI UI 计划](../plan/ui-plan.md)：四个基础层之上的交互客户端和状态投影。
- [Extension](extension-system.md)：逻辑契约先于执行载体。
- [文档状态](document-state.md)与[架构决策](decisions.md)。
- [架构对齐](../plan/architecture-alignment.md)负责迁移顺序；[工程计划](../plan/implementation-plan.md)负责生产验收。
