import { describe, expect, test } from "bun:test";
import { attachSpeakerState, attachTranscript, createMeetingProject } from "../src/audio-project";
import {
  createCorrectedSpeakerLlmInput,
  projectMeetingCorrectedSpeakerOutput,
  renderCorrectedSpeakerLlmInput,
  renderCorrectedSpeakerMarkdown
} from "../src/speaker-output";

describe("corrected speaker output projection", () => {
  test("projects corrected speakers into deterministic LLM and markdown text", () => {
    const project = attachSpeakerState(
      attachTranscript(
        createMeetingProject({
          title: "Speaker review",
          path: "/meetings/speaker-review.wav",
          hash: "speaker-review-hash",
          durationSeconds: 12,
          createdAt: "2026-05-20T09:00:00.000Z"
        }),
        [
          { id: "asr-2", startMs: 2000, endMs: 3000, text: "我来补充风险。", language: "zh" },
          { id: "asr-1", startMs: 0, endMs: 1000, text: "今天先看进度。", language: "zh" },
          { id: "asr-3", startMs: 3200, endMs: 4000, text: "下周完成验证。", language: "zh" }
        ]
      ),
      [
        { id: "speaker-a", label: "主持人" },
        { id: "speaker-b", label: "研发" }
      ],
      [
        { id: "spk-2", speakerId: "speaker-b", startMs: 1800, endMs: 3100 },
        { id: "spk-1", speakerId: "speaker-a", startMs: 0, endMs: 1200 },
        { id: "spk-3", speakerId: "speaker-b", startMs: 3100, endMs: 4200 }
      ],
      []
    );

    const output = projectMeetingCorrectedSpeakerOutput(project);

    expect(output.lines).toEqual([
      {
        transcriptSegmentId: "asr-1",
        speakerId: "speaker-a",
        speakerLabel: "主持人",
        startMs: 0,
        endMs: 1000,
        text: "今天先看进度。"
      },
      {
        transcriptSegmentId: "asr-2",
        speakerId: "speaker-b",
        speakerLabel: "研发",
        startMs: 2000,
        endMs: 3000,
        text: "我来补充风险。"
      },
      {
        transcriptSegmentId: "asr-3",
        speakerId: "speaker-b",
        speakerLabel: "研发",
        startMs: 3200,
        endMs: 4000,
        text: "下周完成验证。"
      }
    ]);
    expect(output.blocks).toEqual([
      {
        speakerId: "speaker-a",
        speakerLabel: "主持人",
        startMs: 0,
        endMs: 1000,
        transcriptSegmentIds: ["asr-1"],
        text: "今天先看进度。"
      },
      {
        speakerId: "speaker-b",
        speakerLabel: "研发",
        startMs: 2000,
        endMs: 4000,
        transcriptSegmentIds: ["asr-2", "asr-3"],
        text: "我来补充风险。\n下周完成验证。"
      }
    ]);
    expect(renderCorrectedSpeakerLlmInput(output)).toBe("主持人: 今天先看进度。\n研发: 我来补充风险。\n研发: 下周完成验证。");
    expect(renderCorrectedSpeakerMarkdown(output)).toBe("**主持人:** 今天先看进度。\n\n**研发:** 我来补充风险。\n下周完成验证。");
    expect(createCorrectedSpeakerLlmInput("ollama", output)).toEqual({
      kind: "corrected-speaker-deverbalization-input",
      schemaVersion: "meetnote.corrected-speaker-deverbalization-input.v1",
      provider: "ollama",
      text: "主持人: 今天先看进度。\n研发: 我来补充风险。\n研发: 下周完成验证。",
      segments: [
        {
          sourceSegmentIds: ["asr-1"],
          speakerId: "speaker-a",
          speakerLabel: "主持人",
          text: "今天先看进度。"
        },
        {
          sourceSegmentIds: ["asr-2"],
          speakerId: "speaker-b",
          speakerLabel: "研发",
          text: "我来补充风险。"
        },
        {
          sourceSegmentIds: ["asr-3"],
          speakerId: "speaker-b",
          speakerLabel: "研发",
          text: "下周完成验证。"
        }
      ]
    });
  });

  test("keeps projection JSON-safe when no speaker segment overlaps", () => {
    const project = attachTranscript(
      createMeetingProject({
        path: "/meetings/no-speaker.wav",
        hash: "no-speaker-hash",
        durationSeconds: 3,
        createdAt: "2026-05-20T09:00:00.000Z"
      }),
      [{ id: "asr-1", startMs: 0, endMs: 1000, text: "未匹配说话人。", language: "zh" }]
    );

    expect(projectMeetingCorrectedSpeakerOutput(project)).toEqual({
      lines: [
        {
          transcriptSegmentId: "asr-1",
          speakerId: "unknown",
          speakerLabel: "unknown",
          startMs: 0,
          endMs: 1000,
          text: "未匹配说话人。"
        }
      ],
      blocks: [
        {
          speakerId: "unknown",
          speakerLabel: "unknown",
          startMs: 0,
          endMs: 1000,
          transcriptSegmentIds: ["asr-1"],
          text: "未匹配说话人。"
        }
      ]
    });
  });
});
