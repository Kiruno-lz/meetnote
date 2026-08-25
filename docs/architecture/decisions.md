# 架构决策

## 已确认

1. 使用 GPUI 构建原生桌面 UI。
2. 使用 Rust application layer 承载业务编排。
3. 不使用 Tauri、WebView 或 Python subprocess。
4. ASR 与 diarization 分别通过独立 backend manager 管理。
5. 允许 `whisper-rs`、`sherpa-onnx`、`diarization` crate 和未来 native/ONNX backend 并存。
6. `crates/` 下使用语义目录名，去除重复的 `meetnote-` 前缀。
7. 使用 SQLite 保存可编辑项目状态，Markdown 作为可读产物。
8. 模型清单和下载生命周期协议迁移到 Rust model manager。
9. 新增 backend 不得修改 domain、application 和 UI 的稳定接口。
10. GPUI 在 macOS 上启用 `runtime_shaders`，将 shader 编译从构建期移到运行期，降低本地构建工具链要求。

## 明确不做

- 不把任意 ONNX 文件伪装成通用模型。
- 不在 GPUI View 中堆叠业务逻辑。
- 不在单个 manager 中实现所有模型族的预处理和后处理。
- 不在架构未验证前下载大模型或引入所有推理依赖。

## 待验证但不阻塞分层设计

- whisper-rs 与 sherpa-onnx 在中文混合会议上的 WER/CER。
- sherpa-onnx 与 `diarization` crate 的 DER、重叠语音和 speaker 数量表现。
- Metal/CoreML provider 的内存和冷启动成本。
- 不同模型格式的转换与许可证边界。
