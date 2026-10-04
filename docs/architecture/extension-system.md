# Extension 与能力注册

## 两个独立问题

Extension 定义实现如何参与 MeetNote：提供服务、声明依赖、注册与撤销。执行载体定义代码在哪里运行：内置 Rust、外部进程/MCP/RPC、未来的 Wasm Component，或不执行代码的声明式 Skill/config。插件参与 Workspace 时，必须通过 Workspace Host 提供的 capability port。

当前没有通用 Extension Host、manifest、动态依赖解析或卸载机制；ASR/diarization registry 是内置 provider 注册，不等于插件运行时。
核心服务不要求成为插件。当前验收明确要求 MoonBit 真实测试插件，经 Agent Host 桥接访问三类 capability；执行载体只实现该测试所需的最小机制，不据此建设通用动态插件平台。

## 最小逻辑契约

先在内置 Rust 装配验证下列职责，不立即实现通用框架。当前 WorkspaceCapabilityPort、AudioCapabilityPort 和 SettingsCapabilityPort 是内置消费者/提供者的验证载体：

| 概念 | 约束 |
|---|---|
| service/capability | 稳定名字与类型化 contract；名字用于标识/诊断，Rust 调用继续依赖 trait |
| host port | 由 Host 持有的受控访问端口；声明能力、范围、版本和生命周期，不暴露任意文件系统 |
| provider/consumer | provider 实现能力；consumer 显式依赖接口；多实现由 Host 明确选择，不依赖注册顺序 |
| requires/provides | 激活前检查必需能力与版本；缺失、冲突或循环明确失败；不先建设通用自动求解器 |
| registration | 返回归属明确的注册凭据；初始化部分失败时撤销已注册内容 |
| disposal | 停止接收新调用，等待或取消在途任务，释放 session/订阅，再撤销注册和释放载体 |
| typed events | 服务拥有事件 schema；标明来源、顺序和生命周期，不通过全局隐式事件驱动文档写入 |
| profile/bundle | 组合已有 backend、Skill、能力与配置；声明需求不等于获得授权 |

“可逆插件效果”只指撤销注册、订阅与资源。已经提交的文档或外部副作用不能因卸载自动消失；撤销必须由 Workspace 的显式 undo/补偿操作完成。长推理持有的 provider/session 仍在使用时不能强行卸载动态库。

服务注册元数据与模型 catalog 分离：catalog 管理模型资产与校验，不承担扩展依赖图。服务 ID 与版本先由具体契约定义，不建设任意字符串到 `Any` 的全局 service locator。

Workspace port 的初始能力可以包括 `workspace.read_document`、`workspace.patch_document`、`workspace.create_semantic_children`、`workspace.search`、`workspace.build_index` 和事件订阅。根文档创建/音频导入暂留 Workspace 内部命令，具体 port 必须复用 Workspace 的 revision、资产保护、结构校验和错误语义；插件不能通过 port 删除源音频、Markdown、项目 JSON 或 Speaker Library，也不能把自己的索引提升为事实来源。

## 演进门槛

1. 内置 Rust 服务先实现一个实际消费者与替换 provider 的完整闭环，复用当前 registry 与装配入口。
2. 确有可选扩展需求后，增加最小 manifest（身份、版本、requires/provides、入口）和生命周期；验证缺依赖、重复注册、部分失败回滚、在途取消与注销。
3. 先完成已明确要求的 MoonBit 测试插件及最小 Host 桥接，覆盖正常调用与越权/版本/参数红线；若采用 Wasm Component，仅定义该桥接必需的 ABI/WIT。通用组件升级/动态加载仍需独立需求，不迁移 Rust domain 或整个 Agent。

执行载体可以混合存在。音频解码、buffer、GPU/FFI 和模型 session 留在原生高性能层；不将大块音频跨 Wasm ABI 反复复制。外部进程与云端能力由 adapter 管理退出、取消、超时和数据传输。

## 未来 ABI 的资源所有权

若引入组件边界，workspace、document snapshot、audio/model session 等 host-owned 状态可通过不透明 handle 访问。按需定义资源，不先创建 Recording/Transcript/AudioStream 全套对象系统。

handle 必须关联 workspace、权限与生命周期；文档 handle 还需明确快照 revision。定义失效、重复释放、取消、组件退出与升级后的访问错误。小型不可变 metadata 可直接传值；大对象采用分页/有界读取。handle 本身既不是持久 ID，也不是授权凭据，不等于自动实现零拷贝。
