# 路线图与开发阶段

> 从 PLAN.md 提纯。原 PLAN.md 是项目初始化时写的"待办清单"风格计划，现整理为阶段回顾 + 实际状态的对照。
> 原始 PLAN.md 已归档至 docs/archive/。

## 阶段对照

| # | 阶段 | 原始目标 | 当前状态 |
|---|------|---------|---------|
| 1 | 需求决策与范围 | 固化 MVP 范围 | ✅ 完成 |
| 2 | 仓库初始化 | 基础目录、README、.gitignore、dev.sh | ✅ 完成 |
| 3 | 模型下载与矩阵 | 模型矩阵、下载流程、缓存策略 | ✅ 已实现 |
| 4 | Tauri/TS App 骨架 | 桌面壳+前端骨架、mock pipeline | ✅ 已实现 |
| 5 | 音频项目管理 | 项目 manifest、导入校验、持久化 | ✅ 已实现 |
| 6 | ASR CLI | ASR subprocess/CLI 封装 | ✅ 已实现 4 引擎 |
| 7 | Speaker Diarization CLI | pyannote 集成、合并算法 | ✅ 已实现 |
| 8 | Speaker 人工修正 | UI 操作、修正记录 schema | ✅ 已实现 |
| 9 | LLM 去口语化 Schema | Provider + schema 校验 | ✅ 已实现 |
| 10 | Markdown 纯文件历史库 | 目录规范、索引、搜索 | ✅ 已实现 |
| 11 | 开发者可运行验收 | 开发环境、mock fixtures、测试验证 | ⚠️ L2/E2E 仍需修复 |
| 12 | Release Packaging | dmg、签名、公证、cask | ⏳ 后置 |

## 当前焦点：阶段 11（开发者可运行验收）

**剩余工作**：
1. 修复 L2 契约测试模块路径问题（全部 8 个测试失败）
2. 修复 E2E 测试（file-import 和 mock-meeting-flow 失败）
3. 安装 pytest 使 Python 测试可运行

## 后续阶段：Release

- 建立 `packaging/` 目录
- dmg 构建脚本、cask 模板、release checklist
- app 包与模型包分离
- 优先 macOS，其他平台后置
