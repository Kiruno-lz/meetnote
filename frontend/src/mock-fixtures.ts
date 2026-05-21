import type { TranscriptSegment, SpeakerSegment, SpeakerProfile } from "./audio-project.ts";

export const DEMO_TRANSCRIPT_SEGMENTS: TranscriptSegment[] = [
  {
    id: "seg-001",
    startMs: 0,
    endMs: 4200,
    text: "好的，那我们今天的会议主要讨论一下项目的进度情况。",
    language: "zh"
  },
  {
    id: "seg-002",
    startMs: 4500,
    endMs: 9800,
    text: "嗯，对，我们上周完成了后端接口的开发，那个，基本上已经可以联调了。",
    language: "zh"
  },
  {
    id: "seg-003",
    startMs: 10200,
    endMs: 15600,
    text: "前端这边呢，设计稿刚刚确认，我们预计这周内可以完成主要页面的开发。",
    language: "zh"
  },
  {
    id: "seg-004",
    startMs: 16000,
    endMs: 22400,
    text: "那测试计划怎么安排？我觉得我们需要提前规划一下，避免到时候时间太紧。",
    language: "zh"
  },
  {
    id: "seg-005",
    startMs: 23000,
    endMs: 29500,
    text: "测试的话，那个，我们计划下周一开始，然后，给大概三天时间，应该够的。",
    language: "zh"
  },
  {
    id: "seg-006",
    startMs: 30000,
    endMs: 36000,
    text: "好，那就这样定了，大家按计划推进，有问题随时在群里沟通。",
    language: "zh"
  }
];

export const DEMO_SPEAKER_SEGMENTS: SpeakerSegment[] = [
  {
    id: "diar-001",
    speakerId: "speaker-0",
    startMs: 0,
    endMs: 4200,
    confidence: 0.92
  },
  {
    id: "diar-002",
    speakerId: "speaker-1",
    startMs: 4500,
    endMs: 9800,
    confidence: 0.88
  },
  {
    id: "diar-003",
    speakerId: "speaker-0",
    startMs: 10200,
    endMs: 15600,
    confidence: 0.95
  },
  {
    id: "diar-004",
    speakerId: "speaker-1",
    startMs: 16000,
    endMs: 22400,
    confidence: 0.91
  },
  {
    id: "diar-005",
    speakerId: "speaker-0",
    startMs: 23000,
    endMs: 29500,
    confidence: 0.87
  },
  {
    id: "diar-006",
    speakerId: "speaker-1",
    startMs: 30000,
    endMs: 36000,
    confidence: 0.94
  }
];

export const DEMO_SPEAKER_PROFILES: SpeakerProfile[] = [
  { id: "speaker-0", label: "发言人A" },
  { id: "speaker-1", label: "发言人B" }
];
