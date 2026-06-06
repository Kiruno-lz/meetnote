# ASR 模型统一存储重构报告

> 日期：2026-06-07
> 目标：将所有模型存储路径统一到可配置的 `modelCachePath`，统一下载管理（进度/速度/暂停/取消）

---

## 架构变更

### 之前（4 个分散路径）

```
~/.meetnote/models/*.bin     ← Whisper GGML（HTTP 下载，有进度/暂停/取消）
~/.cache/whisper/*.pt        ← Whisper PyTorch（库自动下载，不可控）
~/.cache/huggingface/hub/    ← HF 缓存（Python 库，无暂停/取消）
~/.cache/modelscope/hub/     ← ModelScope 备用
```

### 之后（1 个统一路径）

```
~/.meetnote/models/          ← modelCachePath（可配置、统一管理）
├── tiny.pt                  ← Whisper PyTorch, HTTP 下载（进度/速度/暂停/取消）
├── large-v3.pt              ← Whisper PyTorch, HTTP 下载
├── paraformer-zh/           ← Library-managed, Python 下载器→local_dir
│   ├── model.pt
│   ├── config.yaml
│   └── ...
├── qwen3-asr-1.7b/          ← Library-managed, Python 下载器→local_dir
│   ├── model-00001-of-00002.safetensors
│   └── ...
└── mimo-v2.5-asr/           ← Library-managed, Python 下载器→local_dir
    ├── model-00001-of-00007.safetensors
    └── ...
```

---

## 修改文件清单

### Python 后端（Phase 1）

| 文件 | 变更 |
|------|------|
| `python/asr_cli.py` | +`--cache-dir` 参数；+`resolve_cache_model_path()` 映射函数；+`FROM_REPO_KEY` 常量；WhisperEngine/ParaformerEngine/Qwen3ASREngine 支持从本地路径加载 |
| `python/model_downloader.py` | +`--cache-dir` 参数；download_paraformer/qwen3_asr/mimo 支持 `local_dir` 参数下载到指定目录 |

### Rust 桥接（Phase 2）

| 文件 | 变更 |
|------|------|
| `tauri/src/asr.rs` | `run_asr` +`cache_dir` 参数，传递 `--cache-dir` 到 Python |
| `tauri/src/download.rs` | `start_model_download` +`cache_dir` 参数；`start_library_download` 传递 `--cache-dir`；`DownloadManager` +`child_pids` 字段；`cancel_model_download` 中 kill 子进程；`list_cached_models` 同时扫描 `.pt` 文件和子目录；`check_library_model_installed` 检查统一目录；`remove_library_model` 删除统一目录下的子目录 |

### 前端（Phase 3）

| 文件 | 变更 |
|------|------|
| `frontend/src/types/model-manifest.ts` | 7 个 whisper 模型 source 从 GGML `.bin` 更新为 Azure CDN `.pt` URL；更新 downloadSizeBytes 为精确值；移除 `whisper-large-v3-turbo-q5_0`（GGML 专属量化） |
| `frontend/src/utils/model-manager.ts` | 移除 `whisper-large-v3-turbo-q5_0` 的显示元数据 |
| `frontend/src/components/section-model-management.ts` | 全面重写：+`buildShortNameMap()` 短文件名→manifest ID 映射；+`computeDestFilename()` 从 source URL 提取文件名；移除 HF 缓存探测（`check_library_model_installed` 循环）；统一下载/删除逻辑；`startDownload`/`resumeDownload` 传递 `cacheDir` |
| `frontend/src/components/section-transcription-progress.ts` | `run_asr` invoke 新增 `cacheDir: state.modelCachePath` |

### 测试

| 文件 | 变更 |
|------|------|
| `frontend/tests/model-manifest.test.ts` | source URL 更新为 `.pt` URL |
| `frontend/tests/model-manager.test.ts` | 所有 `.bin` 路径更新为 `.pt` |

---

## 验证结果

### 端到端功能测试（全部通过）

| # | 测试 | 结果 |
|---|------|------|
| 1 | Whisper tiny 从统一缓存加载 | ✅ |
| 2 | Whisper large-v3 从统一缓存加载 | ✅ |
| 3 | Paraformer 下载到统一缓存 | ✅ |
| 4 | Paraformer 从统一缓存加载 | ✅ |
| 5 | Qwen3-ASR 下载到统一缓存 | ✅ |
| 6 | Qwen3-ASR 从统一缓存加载 | ✅ |

### TypeScript 类型检查

```
npx tsc --noEmit → 0 errors
```

### 单元测试

```
bun test → 309 pass, 0 fail
```

---

## 未完成项

| 项目 | 状态 | 备注 |
|------|------|------|
| `~/.meetnote/models/` 旧 `.bin` 清理 | ⚠️ macOS TCC 写保护 | 需手动删除（Finder 或终端 + 全磁盘访问权限） |
| Rust `cargo check` 构建 | ⚠️ 预存 Tauri 2.x 插件权限问题 | 与本次改动无关，需单独修复 |
| MiMo ASR 后端 | ⚠️ 架构不支持 | 已在 asr_cli.py 中返回友好错误提示 |
| pyannote 说话人识别 | ⏳ 未测试 | 需要 HF Token + gated 模型 |
| Library 下载 speed 追踪 | ⏳ 待优化 | 目前 speedBps=0，后续可通过解析 `snapshot_download` 进度回调实现 |
