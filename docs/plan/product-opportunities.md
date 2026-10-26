# 产品瓶颈与改进优先级

## 判断与边界

更新日期：2026-10-26；实施分支：`codex/workspace-discovery`。竞品调研基于 2026-10-24 至 2026-10-25 可访问的官方页面，页面更新时间不等于功能首次发布日期，未实际购买或逐项试用竞品。

MeetNote 已有原生转写、说话人分离、文档包、外部编辑、模型管理和受控 Agent 工具。下一步应把已有事实变成易查找、可核对、可带走的资料。产品优势在于本地文件、音频证据和外部 Agent 能力边界；继续增加 backend 或扩成通用 Agent Runtime，不能直接解决资料积累后的使用成本。

## 当前瓶颈

| 优先级 | 代码依据 | 影响与处理 |
|---|---|---|
| P0：全量读取缺少资源上限 | `crates/persistence/src/lib.rs` 的 `load_metadata` 有 16 MiB 限制，原 `DocumentPackage::load` 对 JSON/Markdown 直接全量读取 | 异常大文件影响扫描、编辑和搜索。本分支将每个完整输入文件限制为 16 MiB，同时在读取过程中执行上限；超限报告文件路径，保留资产 |
| P0：长音频全局聚类内存随观察数增长 | `crates/diarization-native/src/windowed.rs` 累积 `clustering_vectors` 与窗口 artifacts；`third_party/speakrs/src/clustering/ahc.rs` 分配 `n*(n-1)/2` 个距离 | 分窗解码不代表整条 pipeline 内存有界。仅距离数组在 5 万个 observation 时约 4.66 GiB，还未计其他对象。须先定义可接受的输入规模、质量门槛与资源策略，再修改聚类；本分支不改变聚类算法 |
| P1：检索后的原文核对成本 | `document_search.rs` 返回已保存文本命中；`meetings.rs` 将连续同 speaker 的段落归为一个回合 | 只打开文档不能解决长转写内的翻找。本分支让结果携带 segment ID、时间范围、speaker 来源和文档 revision，打开后定位并标记实际段落；不拆散现有发言分组 |
| P1：复用出口不足 | `crates/ui/src/pages/meetings.rs` 提供阅读、编辑和处理；`apps/meetnote/examples/audio_host_export.rs` 是评测导出入口 | 文件本身可读，但普通用户缺少可选择格式的桌面导出。优先补 TXT/Markdown/SRT 与剪贴板，沿用现有时间戳和 speaker label |
| P1：模型速度不是整条链路速度 | 独立 ASR benchmark 与 `diarization_benchmark`；native observation/AHC 路径 | 已测固定 FLEURS/AMI 的 ASR 质量、RTF 与 RSS，修复回调崩溃、子词与推理长度；CPU embedding 单线程是本机分离延迟主因；按设备可用并行度保留约三分之一余量后，本机使用六线程，同一 180 秒样本的分离中位耗时为 35.23 秒（单线程 164.91 秒，四线程 47.52 秒），输出时间线与 embedding 通过 baseline 一致性门。AHC 内存和精确时间对齐仍需分别治理。分段切口已修复 Unicode 词边界并保留真实文本 baseline；Silero VAD 裁剪虽将已知静音覆盖从 9.8 秒降至零，却导致英文和低音量中文识别退化，拒绝默认集成，见[边界回执](../quality/asr-boundary-evaluation.json)。模型选择见[研究记录](../reserch/asr-quality-and-runtime-evaluation.md) |
| P2：维护集中度偏高 | `crates/ui/src/controller.rs` 同时编排设置、文档、模型、Speaker Library 与 Assistant；已有 editor/audio 等独立模块 | 新能力继续在所属模块实现，controller 只保留装配和跨模块调用。本分支把搜索生命周期与展示集中于 `crates/ui/src/search.rs`，不做一次性全局重构 |

## 同类产品与可借鉴功能

| 产品与官方依据 | 当前可确认功能 | 对 MeetNote 的取舍 |
|---|---|---|
| [Granola 产品入门](https://docs.granola.ai/help-center/getting-started/granola-101)、[跨会议对话](https://help.granola.ai/article/chatting-with-your-meetings) | 文件夹组织、选择多份会议作为上下文、单会议/文件夹/全库问答、模板与可复用 recipes | 借鉴明确的上下文范围、场景模板与重复操作入口；模板放入 profile/Skill，不能把语义规则写入 Workspace |
| [Otter Advanced Chat](https://help.otter.ai/hc/en-us/articles/35234602608919-Advanced-Otter-AI-Chat-Modes)，更新于 2026-03-09 | 跨转写、截图和历史会议搜索；行动项、邮件/日历与录音控制；页面还列有 MCP 接入 | 优先借鉴“结果可追溯到会议证据”和现有 capability 对外复用。邮件、日历与云端自动化需要独立授权和外部适配，本轮不接入 |
| [Fathom AI Search](https://help.fathom.video/en/articles/7374465)，更新于 2026-08-17 | 普通搜索与 AI 搜索并列；可按日期、团队、会议类型和发言者筛选；支持保存搜索订阅 | 先交付可预测的本地文本检索，再依据使用情况补日期/标签筛选。语义检索独立增强，结果须带原文证据；不复制团队权限体系 |
| [Notta 导出与分享](https://support.notta.ai/hc/en-us/articles/18646448413083-Intro-to-data-export-sharing)，更新于 2026-09-21 | 复制选段、工作区权限分享、TXT/DOCX/SRT 等文件导出 | 便携导出价值高、与本地优先方向一致；先 TXT/Markdown/SRT。多人协作与云分享不是当前闭环前置条件 |
| [MacWhisper](https://www.macwhisper.com/)，官网未提供对应功能发布日期 | 本地文件转写、无机器人会议录制、听写、转写内搜索、多格式导出 | 借鉴原文检索和低摩擦导出；录音采集具有独立的设备、权限和音频路由成本，必须单独完成生命周期与真实设备验证 |

这些能力已在官方页面展示，不能统一称为“本月新发布”。本轮不比较订阅价格，也不以营销页面证明识别准确率或隐私效果。

## 本分支交付

### 本地全文搜索

- 在项目侧栏输入文字后按 Enter 或点击“搜索”；`⌘⇧F` 从任意页面打开项目并聚焦搜索。
- 对已保存内容进行大小写不敏感的字面子串匹配，覆盖标题、标签、摘要、带 speaker 的转写、原始转写与 Markdown；不搜索未保存草稿，不执行语义推断或上传。
- 每个匹配文档保留一个结果，命中字段按上述顺序选取；最多 50 个结果，发现第 51 个匹配时明确提示收窄查询。按文档 ID 稳定遍历，不声称相关性排序。
- 片段最长 180 个原始字符，另加省略号；查询最多 160 字符。UTF-8 与 Unicode 大小写长度变化不用于直接切片原文。
- 转写结果带时间，点击打开 Original；没有处理记录或只有部分说话人对齐时仍展示原始转写，未对齐内容明确标为“未标注说话人”。Markdown 结果打开独立的“笔记”视图，后台读取最新保存正文；切回编辑时只刷新干净草稿，保留未保存修改。其他结果打开 Summary。结果保留在侧栏，可继续查阅；清除搜索后展开所选文档的祖先。
- 输入变化或清除取消旧查询，结果按请求代数校验；外部变更后重新查询并刷新已保存笔记，切换工作区清空搜索并拒绝迟到的查询和正文读取。笔记读取失败显示原因并可重试。
- 读取走后台 JobRuntime worker gate，固定当前 repository；一次只读取一个文档，不建立权威索引。读取失败显示文档 ID/路径与原因；metadata 扫描失败则显示搜索失败，不能将其当作零结果。

转写命中打开后，按文档 revision、segment ID、起止时间、speaker 来源和当前查询核对段落，再滚动到它并标记背景。同一 ASR 段被拆为多个 speaker 段、原始转写补全和时间排序都不依赖显示行号；命中段落在顶部渐隐区之外。定位只执行一次，之后正常滚动；重复打开结果可重新定位。查询变化、清除、切换工作区或离开对应文档/视图后清除标记。打开时 revision、身份或匹配文字已失效则提示重新搜索，不按时间猜测其他段落；全程不写入文档。

该切口借鉴 [Fathom 的搜索结果直达证据](https://help.fathom.video/en/articles/449856)（官方页面标注 2025-09-08 编辑，2026-10-25 核对）。比较后采用现有 GPUI 布局与 ScrollHandle 定位嵌套段落，无新增依赖：持久索引改善查询耗时，不能代替命中后的阅读定位；播放跳转还需要音频播放生命周期，因此当前只交付段落定位，不做段内词级定位或音频 seek。搜索仍需遍历文档，不能据此承诺大型资料库的查询延迟；持久派生索引应在真实规模测量后引入，并单独验证 stale、重建和外部编辑一致性。

### 文档包读写上限

`state.json` 和 `document.md` 各自沿用 16 MiB 上限。读取前检查文件大小，读取时最多消费上限加一字节，阻止外部编辑在检查后扩文件绕过限制。普通保存和事务暂存先检查 Markdown 与序列化 JSON 的实际字节数，再写入任一文件；超限不删除、不截断或覆盖原文件，多文档暂存失败保留整个原工作区。现有文档包加载失败时，repository 保留错误，不将其当作不存在而覆盖。该限制不能替代解析后对象内存、树规模或 AHC 内存的整体预算。

## 后续顺序与验证方法

1. **导出与复制**：//TODO - 在 Workspace/application 生成 TXT/Markdown/SRT，桌面只负责选择目标和反馈；测试中英文、多 speaker、跨小时时间、空转写与覆盖取消，回读导出文件。
2. **原文定位扩展**：//TODO - 独立验证段内词级定位与真实音频播放/seek；覆盖长段落、时间边界、取消与文档切换后迟到回调，不以段落定位测试替代播放验收。
3. **模板与有依据的行动项**：//TODO - 复用现有 Skill/profile 与 capability；输出保留 document/segment 来源，区分 AI 推断、人工确认与已执行状态，使用固定真实 provider 数据集验证遗漏和无依据生成。
4. **长音频与真实模型质量**：已有[ASR 模型比较](../quality/model-comparison.md)；//TODO - 扩展自然中文会议与重叠场景，补 DER、身份误匹配、词时间、取消/故障及小时级 RSS；资源约束与精度取舍达标后再承诺长录音生产支持。
5. **资料规模优化**：//TODO - 测量 100/1000/10000 文档的冷、热查询、取消延迟与峰值 RSS；达到明确交互预算后再决定是否建立可重建全文索引。未来外部 Agent 搜索必须经版本化 capability 与 scope 校验，不能直接暴露内部 DocumentService。

## 验证记录

回归先建立红线，再实现修复：搜索入口不存在、三个搜索用例返回空结果、超限完整读取没有返回 `TooLarge` 均已记录失败；最小窗口的 Tab 滚动测试也先复现越界，再修正列表视口约束。Review 回归复现了超限保存破坏可读性、收起侧栏无法通过快捷键展开、Markdown 命中显示旧草稿、原始转写不可见，以及切回干净编辑器仍显示旧正文，再完成修复。原文定位回归先复现长回合无定位与定位落入顶部渐隐区，再修复并转绿；覆盖 100 段同 speaker、最小窗口、原始/拆分转写、键盘打开、重复打开、手动滚动、跨文档切换与过期目标。最终验证如下：

| 检查 | 结果 |
|---|---|
| `cargo fmt --all -- --check`、`git diff --check` | 通过 |
| `cargo check --workspace --all-targets --locked` | 通过 |
| `cargo build --locked -p meetnote` | 开发构建通过 |
| `cargo test --workspace --locked` | 主测试 909 passed、0 failed、11 ignored；另有 3 次锁与读写子进程测试通过；UI 单元/交互 450 passed，route 47 passed |
| 搜索与读写回归 | 搜索服务 6 项、桌面搜索 12 项回归通过；覆盖真实文件/异步 GPUI、段落定位与失效保护；16 MiB 边界与超限保存、无效现有包保护、多文档暂存失败、原始转写补全通过；route Tab 顺序覆盖搜索和笔记入口 |
| 全量离屏图像 | 27 个场景 × 深/浅 × 玻璃/不透明共 108 张复比均为 0 像素差异；新增原文命中场景的 4 张原图已逐张查看，确认标记与文字清晰 |
| 修改文档的相对链接 | 通过 |

全量截图可用 `MEETNOTE_VISUAL=1 MEETNOTE_VISUAL_UPDATE=1 cargo test --locked -p ui --test visual` 重建，去掉 `MEETNOTE_VISUAL_UPDATE=1` 后执行复比；`MEETNOTE_VISUAL_FILTER=meetings-search` 仅验证搜索场景。静态截图固定使用“减少动态效果”模式并在 baseline 元数据中记录，防止循环动画影响稳定帧；常规动画时序由交互测试覆盖，资源探测前恢复常规动态效果。图像与 baseline 是本地派生产物，不提交到仓库。

上表验证搜索与文档包能力，真实模型执行和精度由独立[ASR 比较](../quality/model-comparison.md)记录。常规 workspace suite 不自动执行需要本地模型的 ignored 测试；20 小时资源、音频播放、原生无障碍手验、签名与 notarization 尚未验收。
