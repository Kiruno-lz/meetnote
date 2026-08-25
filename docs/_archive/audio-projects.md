# 音频项目管理

本文对应 `PLAN.md` 第 5 节，定义 meetnote 的音频项目模型、重复导入策略和项目持久化边界。

## 范围

- 支持导入 `.wav`、`.mp3`、`.m4a`
- 以音频 hash 作为项目去重锚点
- 项目对象保存原始转录、speaker、人工修正、去口语化文本、编辑后的 Markdown 和编辑历史
- 原始转录必须保留，不可被后续修正覆盖

## 项目标识

- `id` 由 `audioHash` 派生，格式为 `audio-<hash>`
- 同一音频重复导入时，优先复用已存在的项目
- 冲突处理按稳定排序选择 canonical 项目，避免重复导入产生不确定结果

## 项目 manifest

项目 manifest 采用单文件 JSON 形式，字段与 `frontend/src/audio-project.ts` 保持一致。

```json
{
  "schemaVersion": 1,
  "id": "audio-standup-hash",
  "title": "Standup",
  "audioPath": "/meetings/standup.mp3",
  "audioHash": "standup-hash",
  "durationSeconds": 90,
  "format": ".mp3",
  "createdAt": "2026-05-17T09:00:00.000Z",
  "originalTranscript": [],
  "transcript": [],
  "speakerSegments": [],
  "speakers": [],
  "corrections": [],
  "deverbalizedSegments": [],
  "editedMarkdown": null,
  "editHistory": []
}
```

## 持久化边界

- `originalTranscript` 保存原始逐字转录
- `transcript` 可用于当前展示层，但不得覆盖原文
- `speakerSegments`、`speakers`、`corrections` 保存 speaker 修正历史
- `deverbalizedSegments` 保存忠实去口语化结果
- `editedMarkdown` 保存最终人工编辑稿
- `editHistory` 记录每次可追溯变更

## 项目目录

```text
meetings/YYYY/MM/<slug>/
├── manifest.json
├── meeting.md
└── history/
```

`manifest.json` 保存结构化状态，`meeting.md` 保存可读输出，`history/` 预留后续细粒度历史文件。

## 验证入口

```bash
cd frontend && bun test audio-project
cd tests/api/ts && bun test project-storage.contract.test.ts
```
