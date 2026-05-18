# 目录说明

## 当前初始化目录

- `frontend/`：Tauri 前端与 TypeScript 业务逻辑；当前包含 app shell、domain modules、Bun tests、dev server 和 build script
- `src-tauri/`：Tauri/Rust 桌面壳；当前包含最小 Tauri v2 壳层、配置和占位开发图标
- `python/`：后续 ASR、diarization 或模型工具链预留目录
- `tests/`：测试与契约验证预留目录
- `script/`：仓库脚本入口，当前包含 `dev.sh`
- `docs/`：决策、结构说明、模型下载与模型矩阵等设计文档
- `examples/`：示例数据或示例文档预留目录，当前不放真实音频

## 当前未实现内容

- 未创建真实 ASR、speaker diarization、LLM provider 或 Markdown 文件 IO
- 未创建 release 构建流程
- 未安装依赖
- 未下载模型
- 未实现真实模型下载器或 pyannote 外部配置 UI
- 未接入真实 ASR、speaker diarization 或 LLM provider
- 未添加 submodule
