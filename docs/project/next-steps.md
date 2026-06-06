# meetnote 下一步行动计划

> 基于差距分析生成的优先级排序清单
> 最后更新：2026-06-06

## 优先级说明

- **P0** — 阻塞性：修复测试/构建，否则无法验证 MVP 闭环
- **P1** — 高优先级：核心功能验证或文档对齐
- **P2** — 中优先级：文档整理和优化
- **P3** — 低优先级：可延后处理的改进

---

## P0 — 当前阻塞项

### 0-01. 修复 L2 契约测试路径错误 🚨

| 项目 | 内容 |
|------|------|
| **问题** | 8 个 L2 测试全部因 `Cannot find module '../../../frontend/src/...'` 失败 |
| **根因** | Bun 从 `tests/api/ts/` 运行，模块相对路径在 Bun 下解析方式与预期不同 |
| **修复方案** | 方案 A：在 `tests/api/ts/` 添加 `bunfig.toml` 设置 `root: "../../../"`<br>方案 B：修改测试导入路径为绝对路径或使用 alias<br>方案 C：在根目录统一运行 `cd tests/api/ts && bun test --preload ../../../frontend/src/...` |
| **验证** | `cd tests/api/ts && bun test` → 8 pass |
| **预估** | 0.5 天 |

### 0-02. 修复 L3 E2E 测试失败 🚨

| 项目 | 内容 |
|------|------|
| **问题** | file-import 和 mock-meeting-flow 两个 spec 失败 |
| **根因** | 需要查看 error-context.md 确定 (上次运行有错误上下文) |
| **修复** | 检查 Playwright 测试与当前组件接口是否一致，修复断言或流程 |
| **验证** | `cd tests/e2e && npx playwright test` → 4 pass |
| **预估** | 1-2 天 |

### 0-03. 安装 pytest 并运行 Python 测试 🚨

| 项目 | 内容 |
|------|------|
| **问题** | `pytest` 未安装在 Python venv 中 |
| **修复** | `cd python && uv add --dev pytest && uv run pytest` |
| **验证** | Python 3 个测试文件通过 |
| **预估** | 0.5 天 |

---

## P1 — 高优先级

### 1-01. 更新过时的文档声明 📝 — ✅ 已在本轮文档整理中完成

| 文件 | 操作 | 状态 |
|------|------|------|
| `docs/structure.md` | → `docs/archive/structure.md`（标记过时） | ✅ |
| `docs/asr-cli.md` | 内容移至 `docs/reference/asr-cli.md`，更新首段 | ✅ |
| `docs/models.md` | 内容移至 `docs/models/model-management.md`，更新首段 | ✅ |
| `AGENTS.md` | 更新"仓库状态"节 | ✅ |
| `PLAN.md` | → `docs/archive/plan.md`，提纯为 `docs/project/roadmap.md` | ✅ |
| `docs/decisions.md` | → `docs/archive/decisions.md`，提纯为 `docs/architecture/decisions.md` | ✅ |

### 1-02. 验证 Rust 编译 ✅

| 项目 | 内容 |
|------|------|
| **当前** | 未运行 `cargo check` 或 `cargo build` |
| **操作** | `cd tauri && cargo check` → 确认 Rust 桥接无编译错误 |
| **注意** | 首次构建可能较慢，因为需下载 Tauri 依赖 |
| **验证** | `cargo build` 无 error |
| **预估** | 首次 15-30 分钟，后续增量 1-2 分钟 |

### 1-03. 统一路径引用 `tauri/` 📝 — ✅ 已在本轮文档整理中完成

| 项目 | 内容 |
|------|------|
| **操作** | 已归档含 `src-tauri` 引用的旧文档，新文档统一使用 `tauri/` | ✅ |

---

## P2 — 中优先级

### 2-01. 合并 Markdown 历史库文档 📝 — ✅ 已在本轮文档整理中完成

| 项目 | 内容 |
|------|------|
| **结果** | - `docs/markdown-library/schema.md` 保留为规范文档（合并了 docs/markdown-library.md 的 schema 细节 + MARKDOWN_LIBRARY_QUICKREF 的 API）<br>- `docs/markdown-library/quickref.md` 保留为快速参考<br>- `PROJECT_COMPLETION_SUMMARY.md` → `docs/archive/project-completion.md`<br>- `DELIVERY_MARKDOWN_LIBRARY.md` → `docs/archive/delivery-report.md` | ✅ |

### 2-02. 归档/删除冗余文档 📝 — ✅ 已在本轮文档整理中完成

| 文件 | 操作 | 状态 |
|------|------|------|
| `PROJECT_COMPLETION_SUMMARY.md` | → `docs/archive/project-completion.md` | ✅ |
| `DELIVERY_MARKDOWN_LIBRARY.md` | → `docs/archive/delivery-report.md` | ✅ |
| `FIXTURE_SPEC.md` | → `docs/reference/fixture-spec.md` | ✅ |
| `MARKDOWN_LIBRARY_QUICKREF.md` | → `docs/markdown-library/quickref.md`（提纯） | ✅ |
| `docs/structure.md` | → `docs/archive/structure.md` | ✅ |
| `docs/overview.md` | → `docs/archive/overview.md`（内容已分散到各子目录） | ✅ |

### 2-03. 补充 ASR CLI 文档以匹配多引擎实现 📝 — ✅ 已在本轮文档整理中完成

| 项目 | 内容 |
|------|------|
| **操作** | `docs/models/asr-models-guide.md` 详细记录了 4 引擎信息；`docs/reference/asr-cli.md` 保持了接口契约 | ✅ |

### 2-04. 确保 Python 依赖锁定与测试兼容 🔧

| 项目 | 内容 |
|------|------|
| **操作** | 验证 `uv.lock` 包含 `pytest` 等 dev 依赖，确认测试可运行 |
| **验证** | `cd python && uv sync && uv run pytest` → 通过 |

---

## P3 — 低优先级

### 3-01. 完善 `scripts/dev.sh` 脚本 🔧

| 项目 | 内容 |
|------|------|
| **操作** | 验证脚本在 macOS 上端到端可用，处理 Edge case |
| **当前** | 脚本存在但未在本次分析中运行验证 |

### 3-02. 补充组件级注释 📝

| 项目 | 内容 |
|------|------|
| **操作** | 为 section-* 组件添加 JSDoc，标注数据流和依赖关系 |

### 3-03. 建立文档更新检查清单 📝

| 项目 | 内容 |
|------|------|
| **操作** | 在 AGENTS.md 或 CONTRIBUTING.md 中添加"提交代码时同步更新文档"规则 |

---

## 执行路线图

```
Week 1 (P0)
├── 0-01 修复 L2 测试路径  [0.5天]
├── 0-02 修复 E2E 测试     [1-2天]
└── 0-03 安装 pytest 运行   [0.5天]

Week 2 (P1)
├── 1-01 ✅ 已完成（文档整理）
├── 1-02 验证 Rust 编译     [0.5天]
├── 1-03 ✅ 已完成（路径统一）
└── (新增) 清理 docs/archive/ 中残留的旧路径引用 [0.5天]

后续 (P2)
├── 2-01 ✅ 已完成（合并文档）
├── 2-02 ✅ 已完成（归档冗余）
├── 2-03 ✅ 已完成（多引擎补充）
└── 2-04 验证 Python 依赖   [0.5天]

后续 (P3)
├── 完善 dev.sh             [1天]
├── 组件注释                [1天]
└── 文档更新检查清单        [0.5天]
```

## 最重要 3 个行动

1. **🔴 修复 L2 契约测试路径错误** — 所有 8 个测试失败，直接影响 Markdown 历史库集成验收
2. **🔴 修复 E2E 测试失败** — 文件导入和 mock 流程是 MVP 核心路径
3. **🟢 验证 Rust 编译** — Rust 桥接代码未经验证，确认 `cargo check` 通过
