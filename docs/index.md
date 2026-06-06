# meetnote 文档地图

> 所有项目文档已整理到此目录。
> **根目录 README.md** 保留为项目简介入口，指向此页。

```
docs/
├── index.md                     ← 本文档：文档地图与导航
│
├── guide/                       # 入门与产品
│   ├── quickstart.md            ← 开发者快速上手（环境、启动、MVP）
│   └── product.md               ← 产品定位、用户、设计原则
│
├── architecture/                # 架构与决策
│   ├── overview.md              ← 系统架构、目录结构、数据流
│   └── decisions.md             ← 关键决策记录与边界
│
├── models/                      # AI 模型管理
│   ├── model-management.md      ← 模型矩阵、下载流程、缓存策略
│   ├── asr-models-guide.md      ← ASR 4 引擎使用指南
│   └── mlx-analysis.md          ← MLX Metal 后端可行性分析
│
├── markdown-library/            # Markdown 历史库
│   ├── schema.md                ← Schema 规范、API 参考
│   └── quickref.md              ← 快速参考（API 速查表）
│
├── reference/                   # 接口契约与规格
│   ├── asr-cli.md               ← ASR CLI JSON 契约
│   ├── audio-projects.md        ← 音频项目管理与 manifest
│   └── fixture-spec.md          ← 测试夹具数据规范
│
├── project/                     # 项目状态与规划
│   ├── roadmap.md               ← 路线图与开发阶段（从 PLAN.md 提纯）
│   ├── progress.md              ← 当前实现进度（含测试结果）
│   ├── gap-analysis.md          ← 文档与实现差距分析
│   └── next-steps.md            ← 下一步行动计划
│
└── archive/                     # 已归档的历史文档
    ├── structure.md             ← (过时) 目录说明
    ├── project-completion.md    ← (过时) Markdown 历史库交付报告
    └── delivery-report.md       ← (过时) Markdown 历史库交付报告（重叠副本）
```

## 快速导航

| 你想了解… | 看这里 |
|-----------|--------|
| 项目是做什么的？ | `guide/product.md` |
| 怎么跑起来？ | `guide/quickstart.md` |
| 整体架构怎么设计的？ | `architecture/overview.md` |
| 为什么这样做？ | `architecture/decisions.md` |
| 支持哪些 ASR 模型？ | `models/asr-models-guide.md` |
| 模型怎么下载和管理的？ | `models/model-management.md` |
| Markdown 历史库怎么用？ | `markdown-library/schema.md` |
| ASR CLI 返回什么格式？ | `reference/asr-cli.md` |
| 项目当前进度如何？ | `project/progress.md` |
| 还有哪些坑要填？ | `project/gap-analysis.md` |
| 接下来做什么？ | `project/next-steps.md` |

## 文档维护原则

1. **docs/ 根目录仅保留 index.md** — 所有详细内容放入子目录
2. **不保留重复信息** — 整合冗余，仅存一份权威版本
3. **子目录职责明确** — guide / architecture / models / markdown-library / reference / project / archive
4. **过时文档移入 archive/** — 不删除，但不再作为权威参考
5. **根目录的 .md 已迁移至此** — 8 份根目录 .md 均已归入对应子目录
