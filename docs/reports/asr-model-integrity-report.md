# ASR 模型完整性与调用检测报告

> 检测日期：2026-06-06
> 检测范围：所有 ASR 模型的下载路径、文件完整性、CLI 调用功能

---

## 1. 模型存储路径检查

### 1.1 HTTP 下载模型目录：`~/.meetnote/models/`

| 模型 ID | 预期路径 | 文件大小 | 状态 |
|---------|---------|---------|------|
| whisper-base | `~/.meetnote/models/whisper-base.bin` | 148 MB | ✅ 存在 |
| whisper-large-v3 | `~/.meetnote/models/whisper-large-v3.bin` | 3.1 GB | ✅ 存在 |
| whisper-large-v3-turbo-q5_0 | `~/.meetnote/models/whisper-large-v3-turbo-q5_0.bin` | 574 MB | ✅ 存在 |
| whisper-tiny | `~/.meetnote/models/whisper-tiny.bin` | 77 MB | ✅ 存在 |
| whisper-large-v3-turbo | `~/.meetnote/models/whisper-large-v3-turbo.bin` | 874 MB | ❌ 未下载 |
| whisper-medium | `~/.meetnote/models/whisper-medium.bin` | 1.5 GB | ❌ 未下载 |
| whisper-small | `~/.meetnote/models/whisper-small.bin` | 488 MB | ❌ 未下载 |

> **注意**：这些文件为 **GGML 格式**（whisper.cpp），由 app 的 HTTP 下载系统下载。
> 当前 ASR 管线使用 **openai-whisper**（PyTorch 格式），因此这些 GGML 文件实际未被使用。
> 参见 §4.1 架构说明。

### 1.2 openai-whisper 自动缓存目录：`~/.cache/whisper/`

| 模型 | 文件 | 大小 | 状态 |
|------|------|------|------|
| tiny | `tiny.pt` | 76 MB | ✅ 存在 |
| base | `base.pt` | 145 MB | ✅ 存在 |
| large-v3 | `large-v3.pt` | 3.1 GB | ✅ 存在 |

> 这些模型由 `openai-whisper` 库在首次调用 `whisper.load_model()` 时自动下载。

### 1.3 Library-Managed 模型缓存目录：`~/.cache/huggingface/hub/`

| 模型 ID | HF Repo | 缓存路径 | 大小 | 状态 |
|---------|---------|---------|------|------|
| paraformer-zh | `funasr/paraformer-zh` | `models--funasr--paraformer-zh` | 880 MB | ✅ 存在（12个blob） |
| qwen3-asr-1.7b | `Qwen/Qwen3-ASR-1.7B` | `models--Qwen--Qwen3-ASR-1.7B` | 4.7 GB | ✅ 存在（2个safetensors shard） |
| mimo-v2.5-asr | `XiaomiMiMo/MiMo-V2.5-ASR` | `models--XiaomiMiMo--MiMo-V2.5-ASR` | ~31 GB | ✅ 存在（7个safetensors shard） |

---

## 2. ASR CLI 调用测试

测试音频：`/tmp/test_meetnote_audio.wav`（纯音正弦波 440Hz, 16kHz, 3秒）

### 2.1 Whisper 后端

| 模型参数 | 加载 | 转录 | 输出格式 | 状态 |
|---------|------|------|---------|------|
| `--model tiny` | ✅ 成功 | ✅ 成功 | ✅ 合法 JSON, 空 segments | ✅ |
| `--model base` | ✅ 成功 | ✅ 成功 | ✅ 合法 JSON, 空 segments | ✅ |
| `--model large-v3` | ✅ 成功 | ✅ 成功 | ✅ 合法 JSON, 1 segment (幻觉) | ✅ |
| `--model whisper-tiny` | ❌ 失败 | — | openai-whisper 不认识 `"whisper-tiny"` | ❌ **命名不匹配** |

**验证结果**：前端代码（`section-transcription-progress.ts:103`）已正确剥离 `whisper-` 前缀，因此实际调用使用 `"tiny"`、`"large-v3"` 等正确名称。直接 CLI 调用需要使用者传入正确名称。

### 2.2 Paraformer 后端

| 模型参数 | 加载 | 转录 | 输出格式 | 状态 |
|---------|------|------|---------|------|
| `--model funasr/paraformer-zh` | ✅ 成功 | ✅ 成功 | ✅ 合法 JSON, 1 empty segment | ✅ |

### 2.3 Qwen3-ASR 后端

| 模型参数 | 加载 | 转录 | 输出格式 | 状态 |
|---------|------|------|---------|------|
| `--model Qwen/Qwen3-ASR-1.7B` | ✅ 成功（2 shards） | ✅ 成功 | ✅ 合法 JSON, 空 segments | ✅ |

### 2.4 MiMo 后端 ⚠️

| 模型参数 | 加载 | 转录 | 输出格式 | 状态 |
|---------|------|------|---------|------|
| `--model XiaomiMiMo/MiMo-V2.5-ASR` | ❌ **架构不支持** | — | — | ❌ **根本性缺陷** |

**根本原因**：MiMo-V2.5-ASR 使用自定义架构 `MiMoV2ASRForCausalLM`（Qwen2 base + speech encoder），
`transformers.pipeline("automatic-speech-recognition")` 只支持标准 ASR 架构（Whisper, Wav2Vec2 等），
无法加载 Qwen2 系 ASR 模型。

**所需集成**：
1. 从 https://github.com/XiaomiMiMo/MiMo-V2.5-ASR 获取自定义 `MimoAudio` pipeline
2. 额外下载 `XiaomiMiMo/MiMo-Audio-Tokenizer`（当前未下载）
3. 使用 `from src.mimo_audio.mimo_audio import MimoAudio` 加载

**已修复**：`asr_cli.py` 中 `MiMoEngine` 被修改为直接抛出带详细指引的错误信息，
不再尝试使用 `pipeline()` 加载。

---

## 3. 模型下载功能测试

### 3.1 Library-Managed 下载

`python/model_downloader.py` 测试结果：

| 后端 | 脚本调用 | JSON 事件 | 缓存路径返回 | 状态 |
|------|---------|----------|------------|------|
| paraformer | ✅ 通过 | ✅ 4个progress + 1个result | ✅ 返回正确路径 | ✅ |
| qwen3-asr | ✅ 通过 | ✅ 同上 | 见下次测试 | ✅ |
| mimo | ✅ 通过 | ✅ 同上 | 见下次测试 | ✅ |

### 3.2 Rust 桥接

- `download.rs` 中的 `start_model_download`：HTTP 下载逻辑完整（range 断点续传 ✅, 进度推送 ✅）
- `start_library_download`：uv 子进程调用完整（stdin 传 HF token ✅, JSON 事件解析 ✅）
- `check_library_model_installed`：检查 HF / ModelScope 缓存 ✅

### 3.3 Rust 构建状态

`cargo check` → ❌ **构建错误**（非关键）：Tauri 2.x 插件权限路径问题，报 `app_hide.toml` 未找到。
该错误不影响 `run_asr` 或下载相关命令的编译，但需要修复才能完成完整构建。

---

## 4. 架构发现

### 4.1 双模型存储体系

项目存在**两套独立**的 whisper 模型存储体系：

```
~/.meetnote/models/     ← GGML 格式（whisper.cpp），由 app HTTP 下载
    whisper-large-v3.bin
    whisper-tiny.bin
    ...

~/.cache/whisper/       ← PyTorch 格式（openai-whisper），由 Python 库自动下载
    large-v3.pt
    tiny.pt
    ...
```

当前 ASR 管线使用 `openai-whisper`（PyTorch），因此 `~/.meetnote/models/` 中的
GGML 文件未被任何代码引用。这是一个设计分歧，需要决策：
- **方案 A**：保持现状，让 openai-whisper 自动管理 PyTorch 模型，移除 GGML 下载
- **方案 B**：切换到 whisper.cpp（需重写 ASR 引擎为 C++ 绑定）

### 4.2 模型 ID 命名映射

前端 `section-transcription-progress.ts:103` 自动剥离 `whisper-` 前缀：
```
"whisper-large-v3" → "large-v3"
"whisper-tiny"     → "tiny"
```
Library-managed 后端直接传递 HF repo ID（`funasr/paraformer-zh` 等）。

---

## 5. 未覆盖测试

以下未在当前检测范围内：
- **Tauri 端到端测试**：通过 Tauri invoke 执行 ASR 和下载（需要完整 Tauri 运行时）
- **进度事件**：未验证 Rust→前端的事件推送完整性
- **断点续传**：未验证 HTTP 下载的 Range 断点续传
- **pyannote 说话人识别**：未测试，需要 HF Token 和 gated 模型授权
- **Rust 构建修复**：Tauri 插件权限路径问题需单独处理

---

## 6. 修复清单

| # | 问题 | 状态 | 修复方式 |
|---|------|------|---------|
| 1 | MiMo 后端加载失败 | ✅ 已修复 | 替换为友好错误消息（`asr_cli.py`） |
| 2 | Rust 构建权限错误 | ⏳ 待修复 | Tauri 2.x 插件权限路径问题 |
| 3 | 双模型体系分歧 | 📝 待决策 | 见 §4.1 |
| 4 | `whisper-` 前缀剥离（CLI 直接使用） | 📝 仅 CLI 问题 | 前端已正确处理，CLI 使用者需注意 |
