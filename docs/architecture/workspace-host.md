# Workspace Host

Workspace Host 是 MeetNote 四个基础层之一，负责用户工作区和文档事实；它不依赖 Agent 会话才能工作，也不承担 Audio provider 的实现。

## 现有 Workspace 抽象

`application/src/workspace.rs` 已有 `WorkspaceService` 与 `ProcessingService`。前者同时提供列表、导入、树编辑、summary/speaker 修改与启动音频处理，`DocumentCommandService` 又提供 revision-aware 文档命令；未来 Agent 工具写入也必须复用这一边界。目标是收敛这些入口的写入保证和所有权，不是第一次引入 Workspace 名称。`ProcessingService` 保持 Audio 编排，Workspace 负责资产关联与结果提交。

## 文件是事实来源

Workspace Host 负责工作区关联、文档读写、结构校验、资产定位、扫描与恢复。它是现有 `DocumentService`、document commands、repository、scanner/watcher 的职责收敛，不是用新 Storage 框架替换它们。

`document.md + state.json` 共同表达权威文档；JSON 不是可丢弃的 parsed metadata，其中包含来源、关系、revision 与处理事实。源文件音频、Markdown、项目 JSON 和 Speaker Library 都是 Workspace 的不可删除资产，不能被索引清理、插件卸载或普通 patch 当作缓存处理。普通附件仍是用户文件。具体格式保持[文档状态](document-state.md)。

| 数据 | 所有权与可重建性 |
|---|---|
| 源文件音频、Markdown、state JSON、Speaker Library | 用户资产，Workspace 负责引用、校验、备份和迁移；插件端口不能删除或静默覆盖 |
| 普通附件 | 用户文件，是否属于项目资产由工作区清单和导入策略决定 |
| 项目树、全文索引、embedding index、解析元数据、搜索缓存 | 派生状态，可删除并从不可删除资产重建；版本或模型不匹配时必须标记 stale |
| 未完成事务 journal | 恢复所需，完成恢复前不能删除 |
| undo、幂等记录、会话历史 | 操作历史，不能从最新文档完整重建；有独立保留/清理语义 |
| 模型缓存、settings | 独立运行资源，不是项目事实 |

索引、embedding index 和搜索缓存可以使用适合查询的本地存储，但只能承载可重建派生状态，不改变文件所有权。派生状态损坏、过期或被删除时，Workspace 必须仍能读取项目资产，并能通过扫描和索引任务恢复查询能力。

当前导入流程主要保存音频外部路径，尚未保证音频与项目目录一起迁移，也不能阻止外部程序删除该路径指向的文件。//TODO - 为源音频增加明确的 asset manifest、checksum 和托管/外部引用状态；用移动、缺失、恢复和重建测试验证“不可删除资产”的生命周期语义。

## 索引与搜索

当前代码已有目录扫描和 watcher，但还没有独立的全文、embedding 或搜索索引服务。后续 Workspace Host 可以维护以下派生能力：

- 基于 Markdown、transcript、tags 和结构元数据的全文索引。
- 基于明确 embedding model、revision、维度和来源字段的 embedding index。
- 为最近查询、排序、分页和索引构建过程服务的搜索缓存。

这些索引不能成为项目树、文档内容、音频来源或 Speaker Library 的第二事实来源。watcher、受控写入和外部编辑刷新后，相关索引必须进入更新或 stale 状态；更新失败只能降低搜索能力，不能阻止读取原始资产或伪造搜索结果。索引重建必须从项目目录、源音频资产和 `speakers/library.json` 读取，不依赖旧索引自身。

## 窄操作边界与插件端口

Workspace 提供读取、创建、patch、列表、关系更新、语义子文档创建和搜索等明确操作。`workspace.read_document/create_document/patch_document/search` 是逻辑能力名；当前内部命令继续复用，不新增同义接口。索引构建和 embedding provider 也通过明确的 Workspace capability 暴露，而不是让插件直接打开文件目录。

文档授权、typed arguments、schema/revision、音频来源、segment 范围、三层树和写策略由服务执行；业务实现不能留在通用 Agent Host。自动写入与逐次确认只是不同提交策略，都不能绕过校验。确认时重新核对源版本；undo 也不能覆盖后续修改。

UI、内置 Agent、外部工具 adapter 和插件端口经过同一读写服务。音频结果有专用受控写入入口；语义 patch 不可覆盖 processing/results。插件只能获得 manifest 声明且由 Workspace 授予的 capability port，例如文档读取、受限 patch、搜索、索引构建或事件订阅；port 不提供任意文件系统访问，也不提供删除不可删除资产的操作。最初使用工作区/文档范围与读写能力即可，不建设复杂权限语言。

每个 port 必须有稳定名称、版本、输入输出 schema、错误语义、取消和生命周期。Workspace 在 port 内执行 workspace/document scope、revision、来源、范围、三层关系和资产保护；插件卸载只撤销 port、订阅和派生任务，不撤销已经提交的文档或资产变更。

## 外部编辑与并发

编辑器和同步工具修改文件是正常输入。watcher 触发重新读取与校验；无效 JSON、关系或来源需返回路径和原因，不能静默修复成另一套事实。

revision 只能保护遵守版本协议的客户端。直接编辑 Markdown 不一定递增 JSON revision，故提交前还需检测所读文件内容是否变化；不能把 watcher 通知或实例内锁当作跨进程互斥。目标是受管理写入具备冲突与恢复保证，外部编辑经过扫描暴露冲突；不能承诺任意同步软件参与同一个原子事务。

外部修改不能保证经过 Agent tool 审计。记录观测到的变化即可，不伪造 actor、撤销链或音频 processing provenance。备份/迁移应说明外部音频引用和历史数据的范围。

## 当前恢复能力与缺口

文档包已有逐文件原子保存；多文档 Agent mutation 所需的 batch、journal、pending、幂等与 undo 仍需收敛进 Workspace 写入协议。仍需验证跨进程恢复与附件保留，不能直接宣称所有文件写入已具备事务性。

//TODO - 统一 Workspace 写入路径：桌面编辑、CLI、Agent 和音频结果写入共用提交与冲突协议；用两个 repository/进程和不递增 revision 的外部 Markdown 编辑验证不会丢失更新。

//TODO - 验证 journal 恢复所有权与普通附件保留：在提交各阶段中断，重新打开工作区并检查文档、附件及 undo；确认恢复不会回滚另一个仍在运行的事务。

以上验收在[架构对齐计划](../plan/architecture-alignment.md)安排，不在本次文档工作中实现。
