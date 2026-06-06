# ASR CLI 契约

> 从 docs/asr-cli.md 迁移。定义 meetnote app 与 ASR subprocess/CLI 之间的 JSON 边界。
> 当前已实现 4 引擎 ASR CLI (Whisper / Paraformer / Qwen3-ASR / MiMo)，参见 `docs/models/asr-models-guide.md`。

## 边界

- ASR 默认质量优先，首选 Whisper large-v3
- 主要评测中文会议，夹具需包含英文混杂片段
- CLI 只通过 stdin/stdout 交换 JSON
- CLI 输出**原始逐字转录**，后续处理不得覆盖原文
- 模型路径来自已校验模型缓存或用户选择的本地路径

## Request JSON

```json
{
  "kind": "asr.run.request",
  "schemaVersion": "meetnote.asr.v1",
  "requestId": "asr-req-001",
  "audio": {
    "path": "/meetings/product-review.m4a",
    "format": ".m4a",
    "sha256": "0123456789abcdef..."
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
- `audio.format` 只允许 `.wav` / `.mp3` / `.m4a`
- `languageHint` 只允许 `zh-primary` / `zh-primary-en-mixed` / `auto`
- `temperature` 固定 `0..1`，默认质量优先流程用 `0`

## Progress JSON

```json
{
  "kind": "asr.progress",
  "schemaVersion": "meetnote.asr.v1",
  "requestId": "asr-req-001",
  "stage": "transcribing",
  "processedMs": 30000,
  "totalMs": 120000,
  "progress": 0.25
}
```

`stage` 枚举：`queued` / `loading-model` / `transcribing` / `finalizing` / `cancelled` / `failed`

## Result JSON

```json
{
  "kind": "asr.result",
  "schemaVersion": "meetnote.asr.v1",
  "requestId": "asr-req-001",
  "model": { "id": "whisper-large-v3", "version": "large-v3" },
  "segments": [
    {
      "id": "seg-0001",
      "startMs": 0, "endMs": 2400,
      "text": "我们先看一下 roadmap。",
      "language": "mixed",
      "confidence": 0.94
    }
  ]
}
```

Segment 约束：
- `id` 在单次转录内稳定唯一
- `startMs` 非负，`endMs > startMs`
- `text` 保留原始转录，不做任何改写
- `language` 只允许 `zh` / `en` / `mixed` / `unknown`
- `confidence` 必填，`0..1`

## Error JSON

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

### 稳定错误码

| code | 场景 | recoverable |
|------|------|-------------|
| `ASR_MODEL_MISSING` | 未选择或下载 ASR 模型 | true |
| `ASR_UNSUPPORTED_AUDIO_FORMAT` | 格式不是 .wav/.mp3/.m4a | true |
| `ASR_PROCESS_FAILED` | subprocess 非零退出 | false |
| `ASR_USER_CANCELLED` | 用户主动取消 | true |

错误消息不得包含 secret、token 或用户音频内容。
