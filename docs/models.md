# 模型下载与模型矩阵

本文对应 `PLAN.md` 第 3 节，定义模型管理的文档契约和当前 TypeScript schema 边界。当前仓库已提供前端侧 manifest/download state 校验，但尚未实现真实下载器、ASR CLI 或 pyannote CLI；不得把下述 schema 当作已可运行下载能力。

## 边界

- 模型权重不进入 app 包，也不提交进仓库。
- 首次使用真实 ASR 或 speaker diarization 时，引导用户下载模型或选择已有本地模型路径。
- mock ASR、mock diarization 与文档测试不需要模型。
- 下载器后续应支持断点续传、校验、版本记录、删除和切换，但 MVP 文档阶段不下载任何权重。
- pyannote 认证信息、Hugging Face token、云端 LLM token 只由用户配置，不写入明文项目文件。

## 模型矩阵

| 用途 | 候选 | MVP 角色 | 语言与质量预期 | 硬件与性能风险 | 推荐格式 | 来源 | checksum 策略 | 许可证与访问 | 下载尺寸 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| ASR | Whisper large-v3 | 质量优先候选，不等同于低配默认 | 多语言模型；中文会议优先评估，兼容少量英文片段；仍需用项目夹具评估幻觉、时间戳和中英切换 | 1550M 参数；本地 CPU 可能很慢，GPU/Metal 后续需单独验证 | `safetensors` 优先；避免默认使用 pickle 权重 | `openai/whisper-large-v3` on Hugging Face | manifest 固定每个文件 `sha256`；下载后逐文件校验；源端更新时生成新 revision 记录 | Apache-2.0，公开模型 | 推荐 transformers safetensors 路径约 3.1 GB 权重加配置文件；完整仓库约 24.7 GB，不应整库下载 |
| Speaker diarization | pyannote/speaker-diarization-3.1 | speaker 标注核心候选 | 面向说话人分离；不是 ASR；中文会议需独立评测 DER 和重叠语音表现 | CPU 可运行但慢；GPU 由 Python CLI 后续按设备配置；输入会被处理为 mono 16 kHz | pyannote pipeline 配置与依赖缓存 | `pyannote/speaker-diarization-3.1` on Hugging Face | gated 模型的具体文件在授权后下载；manifest 记录 revision、文件列表和校验结果 | MIT；需要接受模型条件和配置 Hugging Face access token | gated 仓库可预览配置文件，真实资产大小以授权后 manifest 为准 |
| LLM 去口语化 | Ollama 本地模型或 custom provider | 不属于第 3 节下载器的首批模型包 | 只做忠实去口语化，不做摘要或行动项 | 本地模型大小差异大；云端 provider 有隐私确认和 token 风险 | provider 配置，不由 meetnote 管理模型权重 | 用户本机 Ollama 或用户提供 endpoint | 不管理权重 checksum；只记录 provider id、model name 和 schema version | 取决于用户选择 | 不由 app 下载 |

## manifest schema

模型 manifest 是 app 与下载器、ASR CLI、diarization CLI 之间的稳定契约。它描述可下载资产，不保存 token，不保存用户音频内容。`frontend/src/model-manifest.ts` 中的 `MODEL_MATRIX` 是候选矩阵，允许使用未 pin 的 checksum 占位；真正进入下载器的 manifest 必须通过 `validateModelManifest`，并提供完整 `sha256:<hex>`。

```json
{
  "schemaVersion": "meetnote.model-manifest.v1",
  "models": [
    {
      "id": "whisper-large-v3",
      "kind": "asr",
      "defaultUseCase": "quality-first",
      "languageProfile": "zh-primary-en-mixed",
      "hardware": {
        "minimumMemoryGb": 10,
        "recommendedAcceleration": "apple-silicon"
      },
      "format": "external",
      "source": "openai/whisper-large-v3",
      "checksum": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
      "license": "apache-2.0",
      "downloadSizeBytes": 3100000000
    }
  ]
}
```

必填约束：

- `id` 必须稳定，不能包含本机绝对路径。
- `kind` 只允许 `asr`、`diarization`。
- `defaultUseCase` 只允许 `quality-first`、`speaker-diarization`。
- `languageProfile` 只允许 `zh-primary-en-mixed`、`speaker-agnostic`。
- `format` 只允许 `external`、`local-path`。
- `checksum` 在候选矩阵中可以是占位；正式 manifest 必须填入实际 `sha256:<64 hex>`。
- 需要认证的模型必须走外部配置流程，不能把 token 写入 manifest。

## 下载状态 JSON schema

下载状态用于 UI 进度、恢复、错误提示和测试夹具。状态文件可以放在 app 数据目录下；它不是模型 manifest。

```json
{
  "schemaVersion": "meetnote.model-download-state.v1",
  "modelId": "whisper-large-v3",
  "version": "large-v3",
  "cachePath": "<app-data-dir>/models/ready/whisper-large-v3/large-v3",
  "expectedChecksum": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  "downloadedBytes": 1024,
  "totalBytes": 3090000000,
  "progress": 0.0000003313915858,
  "status": "downloading",
  "error": null
}
```

`status` 枚举：

- `idle`：尚未开始。
- `downloading`：正在下载或续传。
- `verified`：校验通过，模型可用。
- `failed`：确定性失败，需要用户处理或重试。

错误码与用户提示：

| code | 场景 | 用户提示 |
| --- | --- | --- |
| `MODEL_MISSING` | manifest 存在但本地文件缺失 | 下载模型或选择本地模型路径。 |
| `MODEL_CHECKSUM_FAILED` | sha256 不一致 | 删除损坏缓存并重新下载。 |
| `MODEL_CACHE_PERMISSION_DENIED` | app 数据目录不可写 | 更换缓存位置或修复目录权限。 |
| `PYANNOTE_AUTH_REQUIRED` | pyannote gated 模型需要认证 | 打开外部配置，配置 Hugging Face token。 |

## 缓存策略

逻辑缓存根目录：

```text
<app-data-dir>/models/
├── manifests/
├── downloads/
│   └── <model-id>/<revision>/*.part
├── ready/
│   └── <model-id>/<revision>/
└── states/
    └── <model-id>.json
```

策略：

- `<app-data-dir>` 由桌面壳在运行时解析；macOS 通常对应用户级 Application Support 目录，但文档不硬编码绝对路径。
- 所有下载先进入 `downloads/`，只有校验通过后才原子移动到 `ready/`。
- `ready/` 下的目录必须包含 pinned manifest、下载状态、校验摘要和模型文件。
- 删除模型只删除 app 管理的缓存，不删除用户手动选择的外部路径。
- 切换模型只改变当前项目或全局设置中的 `modelId` 和 `revision`，不得改写已有转录原文。
- 会议项目和 Markdown 历史库记录实际使用的模型 id、revision、provider 和 schema version，确保后续可追溯。

## app 内下载流程

1. 用户进入真实 ASR 或 speaker diarization 流程。
2. app 检查项目配置和全局模型选择。
3. 若模型缺失，展示模型矩阵中的大小、来源、许可证、认证要求和隐私边界。
4. 用户选择“下载”、“选择本地路径”或“继续 mock 流程”。
5. 下载器解析 manifest，创建下载状态，执行断点续传。
6. 下载完成后按 manifest 校验 checksum。
7. 校验通过后标记 `ready`，记录 revision 和缓存路径。
8. ASR 或 diarization CLI 只接收已验证的本地路径和必要参数。
9. 用户可在设置中删除缓存或切换模型；删除前提示会影响后续真实运行，但不影响已保存的原始转录和 Markdown。

## pyannote 外部配置

`pyannote/speaker-diarization-3.1` 是 gated Hugging Face 模型。后续实现必须把它当作外部配置能力，而不是内置资产。

配置要求：

- 用户自行登录 Hugging Face。
- 用户接受 `pyannote/segmentation-3.0` 和 `pyannote/speaker-diarization-3.1` 的访问条件。
- 用户创建 access token，并通过 app 的安全配置入口提供给 Python CLI。
- token 不写入仓库、不写入 manifest、不出现在日志、错误消息或 Markdown 历史库。
- Python 侧依赖按后续 `python/` 工具链用 `uv` 管理；当前仓库尚未实现该工具链。

运行契约：

- CLI 接收音频路径、认证状态、模型缓存路径和 speaker 数量可选参数。
- CLI 输出稳定 JSON，并保留 pyannote revision、运行参数和认证状态摘要。
- pyannote 模型输出只负责 speaker segments；它不得覆盖 ASR 原始转录或人工 speaker 修正。
- 认证缺失返回 `PYANNOTE_AUTH_REQUIRED`，不把 token 内容写进错误；token 无效或未接受模型条件会在后续契约中扩展独立错误码。
- 隐私提示必须说明：音频在本地 Python CLI 处理；模型下载与认证会访问 Hugging Face；云端 LLM provider 是另一条单独确认链路。

## 资料来源

- `openai/whisper-large-v3` Hugging Face model card：用于确认 ASR 任务、99 languages、Apache-2.0、large-v3 参数量、safetensors 支持与 Transformers 用法。
- `pyannote/speaker-diarization-3.1` Hugging Face model card：用于确认 MIT license、gated 访问、pyannote.audio 3.1 要求、token 配置、16 kHz mono 输入和 GPU 可选运行方式。

## 验证入口

当前可运行：

```bash
cd frontend && bun test model-docs
cd frontend && bun test model-manifest
cd tests/api/ts && bun test model-download.contract.test.ts
```

完整前端 L1 与类型检查：

```bash
cd frontend && bun test
cd frontend && bun run typecheck
```
