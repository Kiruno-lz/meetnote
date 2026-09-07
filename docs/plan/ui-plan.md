# GPUI UI 计划

## UI 边界

GPUI 是 MeetNote application/document service 的客户端，只负责交互与状态投影。它不执行音频 IO、模型下载协议、推理、聚类、语义拆分或 Agent 决策。

```text
UI action
  -> controller
  -> application/document/model/settings service
  -> typed result or event
  -> UI state projection
```

文档目录是项目树事实来源。UI 的树、内容和状态在启动及文件变化后重新读取，不维护独立项目副本。

## 信息架构

```text
MeetNote
├── Assistant       内置 Agent 客户端
├── Documents       三层项目树、文档内容与处理
│   └── Detail      悬浮详情，不切换 route
├── Models          catalog、安装与下载任务
└── Settings        backend、模型、设备、语言与路径
```

导航文案可以暂时保留 `Meetings`，但代码和内容模型必须使用中性的 document/audio project。

## 当前状态

| 区域 | 已实现 | 当前缺口 |
|---|---|---|
| Application shell | 四个入口、暗色主题、基础路由 | 最终视觉、键盘导航与完整无障碍 |
| Assistant | 输入框、通过 document service 读取列表 | provider、查询结果、自动写回与可选确认 |
| Documents | 目录数据源、三层树、展开、拖动重排、Summary/Original、处理与取消 | 文档编辑、scanner issue、revision/provenance、通用 speaker 校正 |
| Detail | overlay 打开/关闭、基础 transcript | 完整 JSON、来源、状态、处理记录和 Agent 变更来源 |
| Models | catalog 分组、下载、暂停、继续、取消和进度 | license/source 展示、probe 结果和安装问题诊断 |
| Settings | locale、ASR/DIA backend/model、语言、设备、speaker 数、stage、Speaker Library 路径 | 文档工作区路径、能力联动说明和完整校验反馈 |

当前 UI 只允许一个前台音频处理任务；下载任务由 `ModelManager` 独立持续。Assistant 仍是占位客户端。

## Documents

左侧树最多三层：

```text
root document
└── semantic child
    └── semantic child
```

- 拖入节点内部：reparent。
- 拖到节点前后：同级 reorder。
- 非法来源、循环和第四层必须拒绝。
- watcher 发现无效外部文件时，在工作区显示路径、原因和刷新状态。
- 内容区默认显示 summary，可切换 speaker-labelled original。
- 编辑标题、标签、总结、正文或状态时携带当前 revision；冲突不得覆盖磁盘内容。
- processing/results 只读，语义字段可编辑。

Detail 作为右侧悬浮面板，展示 identity、audio source、hierarchy、document status、revision、processing provenance、结果统计和 Markdown；关闭面板不得取消处理或下载任务。

## Assistant

```text
输入请求
  -> Agent/provider + Skill
  -> 读取文档
  -> 查询结果，或 typed write tool
  -> settings write policy：自动执行或用户确认
  -> document service
  -> watcher 刷新
```

- 纯查询不创建独立项目状态。
- 修改标题、标签、总结、正文、状态或树关系时，automatic 模式直接执行并显示结果与撤销入口；confirm_each 模式在执行前显示明确 diff/proposal。
- Assistant 与外部 Agent 使用相同 document service，不允许直接编辑 processing/results。
- provider 不可用时保留文档管理能力，并显示可操作的配置错误。
- connection profile、协议、模型和写回策略统一保存在 Settings；完整行为与验收见[Assistant 构建计划](assistant-plan.md)。

## Models

- 按 model family 展示任务、状态、大小、显存、license、来源和候选 backend。
- 只有 `ready` 且缺失/部分安装的模型提供下载按钮。
- 下载任务展示 artifact、字节进度、速度、ETA 和 queued/downloading/paused/verifying 状态。
- 完成后显示 checksum 与 backend probe 结果；失败时显示原因和重试入口。
- Settings 的模型选择器只允许选择已安装且与当前 backend 兼容的模型。

## Settings

设置项分为：

- Workspace：文档目录、Speaker Library 路径、音频资产策略。
- Interface：UI locale。
- ASR：backend、model、transcription language。
- Diarization：backend、model、segmentation、embedding、clustering、expected speakers。
- Runtime：Auto/CPU/Metal/CoreML 与 backend 能力提示。
- Assistant：LiteLLM connection profile、协议、模型、credential reference、写回策略与 Agent loop 上限。

UI locale 与 transcription language 独立。backend 不支持的设备或 stage 不显示为有效选择；修改后立即保存时必须给出成功或失败反馈。

## 实施顺序

1. 将 scanner issue、revision 和完整文档状态接入 Documents/Detail。
2. 将全部树和文档写回切换到 revision-aware document service，并补齐冲突 UI。
3. 完成通用文档编辑、speaker 校正和 Speaker Library 管理。
4. 增加可选择的文档工作区与音频缺失恢复。
5. 按 Assistant 构建计划接入 LiteLLM、查询结果、自动写回与可选确认。
6. 补齐 Models 的 license、来源、probe 与失败诊断。
7. 完成键盘、焦点、无障碍、视觉系统和端到端 GPUI 验收。

UI 完成标准：完整产品流程不需要 CLI 或手工编辑 JSON；外部文件变化、版本冲突、模型失败和任务取消都有明确且可恢复的反馈。
