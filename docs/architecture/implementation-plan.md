# Rust-native 工程实施计划

## 阶段 0：架构冻结

产物：

- Cargo workspace 结构
- domain 类型边界
- contracts 版本
- ASR/Diarization trait
- JobRuntime 事件模型
- 模型 catalog schema

验收：新增 backend 不需要修改 application 和 UI 接口。

## 阶段 1：纯 Rust 基础层

实现：

- domain
- contracts
- application interfaces
- runtime
- fake backend
- fake repository

验收：不用真实模型即可测试导入、转录、分离、对齐、修正和保存完整流程。

## 阶段 2：模型管理

实现：

- catalog 解析
- cache layout
- HTTP Range 下载
- pause/resume/cancel
- SHA-256 校验
- atomic install
- backend probe

验收：应用重启后能恢复下载状态；错误模型不会进入 installed。

## 阶段 3：第一个真实纵向流程

建议组合：

```text
whisper-rs
+
sherpa-onnx diarization
```

流程：

```text
导入音频
  -> ASR
  -> diarization
  -> 对齐
  -> 保存 canonical project
```

验收：不依赖 Python、不依赖 Tauri、不经过跨进程 JSON。

## 阶段 4：替换后端验证

加入：

- sherpa-onnx ASR
- `diarization` crate backend

验收：同一个 application pipeline 可以切换 backend；UI 不修改。

## 阶段 5：GPUI UI

顺序：

1. 应用壳和导航
2. 项目列表
3. 录音详情
4. 转录进度
5. speaker 编辑器
6. Markdown 预览
7. 模型管理
8. 设置

GPUI View 不得直接执行领域算法、文件 IO 或推理。

## 阶段 6：生产化

- SQLite migration
- 崩溃恢复
- 模型安装恢复
- Metal/CoreML 性能评测
- WER/CER/DER 基准
- macOS 打包、签名和第三方 license

## 完成标准

- `cargo test --workspace` 通过。
- fake backend 能覆盖 application 主流程。
- 每个真实 backend 有独立 integration test。
- ASR 和 diarization 可独立选择和组合。
- 无 Python、Tauri、WebView 依赖。
- 模型来源、checksum、license 和运行版本可追溯。
