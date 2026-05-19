# meetnote

meetnote 是一个面向本地桌面的会议纪要工作台，核心目标是把会议音频转录、发言人区分、忠实去口语化整理和 Markdown 历史库沉淀为可维护、可追溯的本地项目。

## 定位

- 中文会议优先，兼顾少量英文片段
- ASR 以质量优先，默认优先评估 Whisper large-v3
- 模型不内置进应用包，安装后按需下载、配置、校验和切换
- speaker 标注与人工修正是核心能力，不是后置增强
- 去口语化由本地 LLM 或云端 custom provider 完成，输出必须受严格 schema 约束
- Markdown 纯文件历史库是核心数据资产，原始转录和修正记录必须可追溯
- 音频项目以纯 TS manifest 契约管理，数据目录和重复导入规则见 `docs/audio-projects.md`

模型矩阵、下载流程、缓存策略和 pyannote 外部配置见 `docs/models.md`。当前仓库只包含文档契约与 mock 骨架，尚未实现真实模型下载器，也不会自动下载模型。

音频项目管理、项目 manifest 和重复导入策略见 `docs/audio-projects.md`。音频项目对象会保留原始转录、speaker 修正、去口语化文本和编辑历史，避免不可逆覆盖原文。

ASR CLI JSON 边界、转录结果 schema、进度事件和稳定错误码见 `docs/asr-cli.md`。当前仓库已提供 TypeScript schema 与 L2 contract test，尚未接入真实 Whisper subprocess。

## MVP

MVP 只覆盖开发者可运行的最小闭环：

1. 导入 `.wav`、`.mp3`、`.m4a`
2. 项目化管理录音
3. ASR
4. speaker diarization
5. speaker 人工修正
6. LLM 忠实去口语化
7. Markdown 历史库保存与重建
8. `script/dev.sh` 环境自检

## 非 MVP

- TTS
- 摘要、行动项、概括性结论
- dmg、签名、公证、Homebrew cask
- 默认引入 SQLite 或 Tantivy
- 将模型权重打包进应用

## 开发入口

仓库约定的开发入口是 `script/dev.sh`：

```bash
bash script/dev.sh --check
bash script/dev.sh --frontend
bash script/dev.sh --tauri
```

`--frontend` 启动 Bun 前端开发服务器；`--tauri` 启动 Tauri dev mode，要求本机已有 Tauri CLI。脚本不会自动安装依赖、下载模型或写入 secret。

前端骨架位于 `frontend/`，包含纯 TypeScript domain modules、Bun 测试、类型检查、dev server 和 build script。Tauri 壳层位于 `src-tauri/`，当前只暴露最小桌面壳和健康命令。
