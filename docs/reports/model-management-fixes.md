# 模型管理三个问题修复报告

> 日期：2026-06-07

---

## 问题 1：删除后再下载报 "Already completed" ✅

### 根因
`delete_model_file` 和 `remove_library_model` 删除文件/目录后，未清除 `DownloadManager.states` 中的状态记录。当用户再次点击下载时，`start_model_download` 检查到 `DownloadStatus::Completed`，返回 `"Already completed"`。

### 修复
`tauri/src/download.rs`：
- `delete_model_file`：新增 `model_id` + `state` 参数，删除文件后调用 `state.states.lock().unwrap().remove(&model_id)`
- `remove_library_model`：同上，删除目录后清除状态

`frontend/src/components/section-model-management.ts`：
- `deleteModel`：在两个 invoke 调用中传入 `modelId`

---

## 问题 2：Library 下载无法正确显示进度和速度 ✅

### 根因
Library-managed 下载的 Python 脚本仅输出 `{progress: 0.0-1.0}` 分数，Rust 将其映射为 0-100 "伪字节"，`speedBps` 始终为 0。

### 修复方案
**方案**：利用 manifest 中的已知模型大小 (`downloadSizeBytes`)，将 fraction 乘以 total 得到真实字节数，再计算速度。

### 修改
**Rust**（`tauri/src/download.rs`）：
- `start_model_download`：新增 `total_size_hint: Option<u64>` 参数
- `start_library_download`：接收 `total_size_hint`，在进度处理循环中：
  - `downloaded_bytes = progress * total_size`
  - `total_bytes = total_size`
  - 通过 delta_bytes / delta_time 计算 `speed_bps`

**前端**（`section-model-management.ts`）：
- `startDownload` 和 `resumeDownload`：对 library 下载传入 `totalSizeHint: model.sizeBytes`

### 效果
```
之前: downloaded=30/100, speed=0 B/s  (无意义)
之后: downloaded=132MB/440MB, speed=66.0 MB/s  (真实进度+速度)
```

---

## 问题 3：模型大小显示硬编码 ✅

### 根因
`renderModelManagement` 始终使用 `model.sizeBytes`（来自 `MODEL_MATRIX` 硬编码），不读取文件系统实际大小。

### 修复
`frontend/src/components/section-model-management.ts`：
- 从 `list_cached_models` 返回的 `installedModels` 中提取 `sizeBytes`（文件系统真实大小）
- 构建 `actualSizes: Map<manifestId, number>`
- 渲染时优先使用 `actualSizes.get(m.id)`，回退到 manifest 大小

### 效果
```
之前: "Whisper Large v3 · 3.0 GB" (硬编码)
之后: "Whisper Large v3 · 2.9 GB" (文件系统实际大小)
      "Paraformer 中文 · 848 MB" (实际目录大小)
```

---

## 修改文件

| 文件 | 变更 |
|------|------|
| `tauri/src/download.rs` | Fix 1: delete_model_file/remove_library_model 清除状态；Fix 2: total_size_hint 参数 + 真实字节速度计算 |
| `frontend/src/components/section-model-management.ts` | Fix 1: deleteModel 传 modelId；Fix 2: startDownload/resume 传 totalSizeHint；Fix 3: actualSizes 动态大小 |

## 验证

```
TypeScript: npx tsc --noEmit → 0 errors
单测: bun test → 309 pass, 0 fail
E2E: 下载→删除→重新下载 → success (no "Already completed")
      进度速度: fraction × totalSize → real bytes + speed
      动态大小: list_cached_models 返回实际文件/dir 大小
```
