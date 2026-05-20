# ASR CLI 契约

本文对应 `PLAN.md` 第 6 节，定义 meetnote app 与后续 ASR subprocess/CLI 之间的 JSON 边界。当前仓库只实现 TypeScript schema、mock contract test 和文档；尚未接入真实 Whisper 运行器，未下载模型，未启动 Python 工具链。

## 边界

- ASR 默认质量优先，首个评估候选是 Whisper large-v3。
- 主要评测中文会议，夹具需要包含少量英文混杂片段。
- CLI 只通过 stdin/stdout 或等价进程边界交换 JSON，不读取 app 内部对象。
- CLI 输出原始逐字转录。speaker 合并、去口语化和 Markdown 编辑不得覆盖 ASR 原文。
- 模型路径必须来自已校验模型缓存或用户选择的本地路径；模型权重不进入仓库或 app 包。

## request JSON

```json
{
  "kind": "asr.run.request",
  "schemaVersion": "meetnote.asr.v1",
  "requestId": "asr-req-001",
  "audio": {
    "path": "/meetings/product-review.zh-en.m4a",
    "format": ".m4a",
    "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
  },
  "model": {
    "id": "whisper-large-v3",
    "version": "large-v3",
    "path": "/models/whisper-large-v3"
  },
  "languageHint": "zh-primary-en-mixed",
  "options": {
    "wordTimestamps": true,
    "temperature": 0,
    "task": "transcribe"
  }
}
```

约束：

- `schemaVersion` 固定为 `meetnote.asr.v1`。
- `audio.format` 只允许 `.wav`、`.mp3`、`.m4a`。
- `audio.sha256` 是 64 位小写 hex，用于把请求与项目音频指纹对齐。
- `languageHint` 只允许 `zh-primary`、`zh-primary-en-mixed`、`auto`。
- `temperature` 固定在 `0..1`；mock 和默认质量优先流程应使用 `0` 保持确定性。

## progress JSON

```json
{
  "kind": "asr.progress",
  "schemaVersion": "meetnote.asr.v1",
  "requestId": "asr-req-001",
  "stage": "transcribing",
  "processedMs": 30000,
  "totalMs": 120000,
  "progress": 0.25,
  "message": "Transcribing audio"
}
```

`stage` 只允许：

- `queued`
- `loading-model`
- `transcribing`
- `finalizing`
- `cancelled`
- `failed`

`processedMs`、`totalMs` 和 `progress` 必须一致表达同一进度；`processedMs` 不得超过 `totalMs`。

## result JSON

```json
{
  "kind": "asr.result",
  "schemaVersion": "meetnote.asr.v1",
  "requestId": "asr-req-001",
  "modelVersion": "large-v3",
  "parameters": {
    "language": "zh",
    "temperature": 0
  },
  "model": {
    "id": "whisper-large-v3",
    "version": "large-v3",
    "parameters": {
      "language": "zh",
      "temperature": 0
    }
  },
  "segments": [
    {
      "id": "seg-0001",
      "startMs": 0,
      "endMs": 2400,
      "text": "我们先看一下 roadmap。",
      "language": "mixed",
      "confidence": 0.94
    }
  ]
}
```

segment 约束：

- `id` 在单次转录内稳定唯一。
- `startMs` 非负，`endMs` 必须大于 `startMs`。
- `text` 保留原始转录文本，不做总结、去口语化或 speaker 改写。
- `language` 只允许 `zh`、`en`、`mixed`、`unknown`。
- `confidence` 必填，必须在 `0..1`。

## error JSON

```json
{
  "kind": "asr.error",
  "schemaVersion": "meetnote.asr.v1",
  "requestId": "asr-req-001",
  "code": "ASR_MODEL_MISSING",
  "message": "Select or download an ASR model before transcription.",
  "recoverable": true
}
```

稳定错误码：

| code | 场景 | recoverable |
| --- | --- | --- |
| `ASR_MODEL_MISSING` | 未选择或未下载 ASR 模型 | `true` |
| `ASR_UNSUPPORTED_AUDIO_FORMAT` | 音频不是 `.wav`、`.mp3`、`.m4a` | `true` |
| `ASR_PROCESS_FAILED` | subprocess 非零退出或输出无效 | `false` |
| `ASR_USER_CANCELLED` | 用户主动取消 | `true` |

错误消息不得包含 secret、token、完整环境变量或用户音频内容。

## mock 评测夹具

后续可在 `tests/fixtures/asr/` 增加固定 mock 输入和期望输出：

- 中文会议：固定 `.wav` 或无音频 JSON fixture，覆盖中文时间戳。
- 中英混合：包含英文术语片段，segment `language` 可为 `mixed`。
- 错误路径：缺模型、格式不支持、进程失败、用户取消。

夹具不得提交真实隐私音频。需要音频时只能使用可公开分发的极短样本或生成数据，并记录来源。

## 验证入口

```bash
cd frontend && bun test asr-schema
cd tests/api/ts && bun test asr.contract.test.ts
```

完整验证：

```bash
cd frontend && bun test
cd frontend && bun run typecheck
cd tests/api/ts && bun test
```
