# 音频项目管理

> 从 docs/audio-projects.md 迁移。定义音频项目模型、重复导入策略和持久化边界。

## 范围

- 支持导入 `.wav` / `.mp3` / `.m4a`
- 以音频 hash 作为项目去重锚点
- 项目对象保存：原始转录、speaker、人工修正、去口语化文本、编辑后的 Markdown 和编辑历史
- **原始转录必须保留**，不可被后续修正覆盖

## 项目标识

- `id` 由 `audioHash` 派生，格式 `audio-<hash>`
- 同一音频重复导入时优先复用已有项目
- 冲突处理按稳定排序选择 canonical 项目

## 项目 Manifest

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

| 字段 | 用途 | 不可覆盖 |
|------|------|---------|
| `originalTranscript` | 原始逐字转录 | ✅ |
| `transcript` | 当前展示层 | 不覆盖原文 |
| `speakerSegments` | speaker 分段 | |
| `speakers` | speaker 列表 | |
| `corrections` | 修正历史 | |
| `deverbalizedSegments` | 去口语化结果 | |
| `editedMarkdown` | 最终人工编辑稿 | |
| `editHistory` | 变更记录 | |

## 项目目录

```
meetings/YYYY/MM/<slug>/
├── manifest.json
├── meeting.md
└── history/        # 预留细粒度历史文件
```
