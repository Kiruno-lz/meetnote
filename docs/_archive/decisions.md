# 决策记录

## 已确认范围

- 项目定位为本地桌面会议纪要工作台
- 目标优先覆盖中文会议，并兼容少量英文片段
- ASR 质量优先，优先评估 Whisper large-v3
- 模型不内置进应用包，改为按需下载、配置、校验和切换
- speaker diarization 与 speaker 人工修正是 MVP 核心能力
- 去口语化由本地 LLM 或云端 custom provider 完成，且必须受严格 schema 约束
- Markdown 纯文件历史库是核心数据资产，不默认引入 SQLite 或 Tantivy
- TTS 不属于 MVP
- 摘要、行动项、概括性结论不属于 MVP
- dmg、签名、公证、Homebrew cask 归入 release 阶段
- Rust 是可选能力层，TS 优先承载 UI、状态管理和可测试业务逻辑
- `script/dev.sh` 是仓库约定的开发入口，支持 `--check`、`--frontend` 和 `--tauri`；脚本不自动安装依赖、不下载模型、不写入 secret

## 当前用户最新要求

- 扫描项目内 `skills/**/SKILL.md`，列出 name/description；若不存在则记录“未发现项目内 skills”
- 读取并遵守 `AGENTS.md` 和 `PLAN.md`
- 仅执行 `PLAN.md` 中的仓库初始化相关工作
- 不做真实代码构建，不安装依赖，不下载模型，不添加 submodule，不运行 Tauri/bun/cargo/uv/brew 构建或测试
- 不提交 secret，不提交模型权重，不提交真实音频，不修改 `node_modules/`
- 创建基础目录和初始化文档、`.gitignore`、`script/dev.sh`
- Tauri/TS app 骨架已进入 TS-first 路线：前端 domain modules 先行，Tauri/Rust 只承载最小桌面壳
