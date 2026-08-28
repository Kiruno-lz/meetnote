# 架构决策

## 已确认

1. 使用 GPUI 构建原生桌面 UI。
2. 使用 Rust application layer 承载业务编排。
3. 不使用 Tauri、WebView 或 Python subprocess。
4. ASR 与 diarization 分别通过独立 backend manager 管理。
5. 允许 `whisper-rs`、`sherpa-onnx`、`diarization` crate 和未来 native/ONNX backend 并存。
6. `crates/` 下使用语义目录名，去除重复的 `meetnote-` 前缀。
7. v0.2 及后续版本使用原子替换 JSON 保存可编辑项目状态，Markdown 作为可读产物；不引入 SQLite。
8. 模型清单和下载生命周期协议迁移到 Rust model manager。
9. 新增 backend 不得修改 domain、application 和 UI 的稳定接口。
10. GPUI 在 macOS 上启用 `runtime_shaders`，将 shader 编译从构建期移到运行期，降低本地构建工具链要求。
11. 产品核心对象是音频资产下的项目树，会议只是一个分类场景，不把领域模型锁定为 Meeting。
12. 每个项目节点使用 UUID、父项目、音频时间范围、标题和分类标签表达；层级最多三层。
13. 长音频按语义拆分是可选整理操作；单场景短音频保持单项目，不强制制造子项目。
14. 每个项目同时保留原始文和总结文，默认展示总结文；主项目使用内部链接引用子项目，不复制子项目正文。
15. 音频必须先完成完整转录，再进行语义分析和可选拆分；语义拆分不能替代或截断原始转录。
16. 默认首页使用无历史的单次 LLM 输入；查询结果不建立 conversation history，改变项目树或内容的 LLM 结果必须确认后写入。
17. 设置中的 UI locale 与 ASR transcription language 分离，为后续 i18n 保留独立字段。

## 明确不做

- 不把任意 ONNX 文件伪装成通用模型。
- 不在 GPUI View 中堆叠业务逻辑。
- 不在单个 manager 中实现所有模型族的预处理和后处理。
- 不把模型权重提交进仓库；模型按需下载到本地 cache，并经过 checksum 与 backend probe。

## 待验证但不阻塞分层设计

- whisper-rs 与 sherpa-onnx 在中文混合会议上的 WER/CER。
- sherpa-onnx 与 `diarization` crate 的 DER、重叠语音和 speaker 数量表现。
- Metal/CoreML provider 的内存和冷启动成本。
- 不同模型格式的转换与许可证边界。
