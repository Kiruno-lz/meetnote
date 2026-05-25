import { describe, expect, test } from "bun:test";
import {
  addEditHistoryEntry,
  attachDeverbalizedState,
  attachEditedMarkdown,
  attachSpeakerState,
  attachSpeakerStateFromDiarizationRerun,
  attachTranscript,
  createMeetingProject,
  createMeetingProjectManifest,
  getSupportedAudioExtension,
  planAudioImport,
  resolveImportedMeetingProject,
  restoreMeetingProjectManifest,
  validateMeetingProjectManifest
} from "../src/types/audio-project";
import { applySpeakerCorrectionCommands } from "../src/utils/speaker";

describe("audio project model", () => {
  test("accepts only the supported MVP audio formats", () => {
    expect(getSupportedAudioExtension("/tmp/meeting.wav")).toBe(".wav");
    expect(getSupportedAudioExtension("/tmp/meeting.MP3")).toBe(".mp3");
    expect(getSupportedAudioExtension("/tmp/meeting.m4a")).toBe(".m4a");
    expect(getSupportedAudioExtension("/tmp/meeting.flac")).toBeNull();
  });

  test("creates deterministic project state from an audio import", () => {
    const project = createMeetingProject({
      path: "/meetings/周会.m4a",
      hash: "abc123",
      durationSeconds: 1800,
      createdAt: "2026-05-17T09:00:00.000Z"
    });

    expect(project.id).toBe("audio-abc123");
    expect(project.title).toBe("周会");
    expect(project.format).toBe(".m4a");
    expect(project.editedMarkdown).toBeNull();
  });

  test("persists speaker corrections in project state", () => {
    const project = createMeetingProject({
      path: "/meetings/weekly.wav",
      hash: "weekly",
      durationSeconds: 120,
      createdAt: "2026-05-17T09:00:00.000Z"
    });

    const updated = attachSpeakerState(
      project,
      [{ id: "speaker-a", label: "主持人" }],
      [{ id: "seg-1", speakerId: "speaker-a", startMs: 0, endMs: 1000 }],
      [
        {
          id: "correction-1",
          targetSpeakerId: "speaker-b",
          replacementSpeakerId: "speaker-a",
          reason: "manual-merge",
          createdAt: "2026-05-17T09:01:00.000Z"
        }
      ]
    );

    expect(updated.corrections).toHaveLength(1);
    expect(updated.speakers[0]?.label).toBe("主持人");
  });

  test("persists structured manual speaker correction records through manifest restore", () => {
    const project = attachSpeakerState(
      attachTranscript(
        createMeetingProject({
          path: "/meetings/manual-speakers.wav",
          hash: "manual-speakers-hash",
          durationSeconds: 120,
          createdAt: "2026-05-20T09:00:00.000Z"
        }),
        [
          { id: "asr-1", startMs: 0, endMs: 1000, text: "今天看计划", language: "zh" },
          { id: "asr-2", startMs: 1000, endMs: 2000, text: "我来补充", language: "zh" }
        ]
      ),
      [
        { id: "speaker-a", label: "主持人" },
        { id: "speaker-b", label: "研发" }
      ],
      [
        { id: "spk-1", speakerId: "speaker-a", startMs: 0, endMs: 1000 },
        { id: "spk-2", speakerId: "speaker-b", startMs: 1000, endMs: 2000 }
      ],
      [
        {
          id: "cmd-assign-1",
          commandId: "cmd-assign-1",
          kind: "assign-range",
          targetSpeakerId: "speaker-b",
          replacementSpeakerId: "speaker-b",
          reason: "manual-range-assign",
          createdAt: "2026-05-20T09:01:00.000Z",
          beforeSegments: [{ id: "spk-2", speakerId: "speaker-a", startMs: 1000, endMs: 2000 }],
          afterSegments: [{ id: "spk-2", speakerId: "speaker-b", startMs: 1000, endMs: 2000 }]
        }
      ]
    );

    const manifest = createMeetingProjectManifest(project);
    const restored = restoreMeetingProjectManifest(JSON.parse(JSON.stringify(manifest)) as unknown);

    expect(validateMeetingProjectManifest(manifest)).toEqual({ ok: true, errors: [] });
    expect(restored).toEqual(project);
    expect(restored.corrections[0]?.beforeSegments).toEqual([
      { id: "spk-2", speakerId: "speaker-a", startMs: 1000, endMs: 2000 }
    ]);
    expect(restored.corrections[0]?.beforeSegments).not.toBe(project.corrections[0]?.beforeSegments);
  });

  test("rejects unsupported or invalid import inputs before project creation", () => {
    expect(() =>
      createMeetingProject({
        path: "/meetings/notes.txt",
        hash: "abc123",
        durationSeconds: 1800,
        createdAt: "2026-05-17T09:00:00.000Z"
      })
    ).toThrow("Unsupported audio format");

    expect(() =>
      createMeetingProject({
        path: "/meetings/empty.wav",
        hash: "abc123",
        durationSeconds: 0,
        createdAt: "2026-05-17T09:00:00.000Z"
      })
    ).toThrow("Audio duration must be positive");

    expect(() =>
      createMeetingProject({
        path: "/meetings/no-hash.wav",
        hash: " ",
        durationSeconds: 1,
        createdAt: "2026-05-17T09:00:00.000Z"
      })
    ).toThrow("Audio hash is required");

    expect(() =>
      createMeetingProject({
        path: "/meetings/no-date.wav",
        hash: "abc123",
        durationSeconds: 1,
        createdAt: " "
      })
    ).toThrow("Audio createdAt is required");
  });

  test("uses a stable duplicate import strategy keyed by audio hash", () => {
    const newer = createMeetingProject({
      path: "/meetings/newer.wav",
      hash: "same-audio",
      durationSeconds: 120,
      createdAt: "2026-05-18T09:00:00.000Z"
    });
    const older = createMeetingProject({
        path: "/meetings/weekly.wav",
        hash: "same-audio",
        durationSeconds: 120,
        createdAt: "2026-05-17T09:00:00.000Z"
    });
    const existing = [newer, older];

    const duplicatePlan = planAudioImport(existing, {
      path: "/downloads/copy.MP3",
      hash: "same-audio",
      durationSeconds: 120,
      createdAt: "2026-05-18T09:00:00.000Z"
    });

    const newPlan = planAudioImport(existing, {
      path: "/downloads/new.m4a",
      hash: "new-audio",
      durationSeconds: 300,
      createdAt: "2026-05-18T09:00:00.000Z"
    });

    expect(duplicatePlan).toEqual({
      action: "reuse-existing",
      projectId: "audio-same-audio",
      reason: "duplicate-audio-hash"
    });
    expect(
      planAudioImport([...existing].reverse(), {
        path: "/downloads/copy.MP3",
        hash: "same-audio",
        durationSeconds: 120,
        createdAt: "2026-05-18T09:00:00.000Z"
      })
    ).toEqual(duplicatePlan);
    expect(newPlan).toEqual({
      action: "create-new",
      projectId: "audio-new-audio",
      reason: "new-audio-hash"
    });
    expect(resolveImportedMeetingProject(existing, {
      path: "/downloads/copy.MP3",
      hash: "same-audio",
      durationSeconds: 120,
      createdAt: "2026-05-19T09:00:00.000Z"
    })).toEqual(older);
    expect(resolveImportedMeetingProject([...existing].reverse(), {
      path: "/downloads/copy.MP3",
      hash: "same-audio",
      durationSeconds: 120,
      createdAt: "2026-05-19T09:00:00.000Z"
    })).toEqual(older);
  });

  test("preserves original transcript while attaching downstream editable state", () => {
    const project = createMeetingProject({
      path: "/meetings/product-review.wav",
      hash: "product-review",
      durationSeconds: 600,
      createdAt: "2026-05-17T09:00:00.000Z"
    });

    const withTranscript = attachTranscript(project, [
      {
        id: "seg-1",
        startMs: 0,
        endMs: 2000,
        text: "呃我们今天看一下产品方案",
        language: "zh"
      }
    ]);
    const withSpeakers = attachSpeakerState(
      withTranscript,
      [{ id: "speaker-1", label: "产品负责人" }],
      [{ id: "spk-seg-1", speakerId: "speaker-1", startMs: 0, endMs: 2000 }],
      []
    );
    const withDeverbalized = attachDeverbalizedState(withSpeakers, [
      {
        id: "clean-1",
        sourceSegmentIds: ["seg-1"],
        speakerId: "speaker-1",
        text: "我们今天看一下产品方案。",
        createdAt: "2026-05-17T09:02:00.000Z",
        provider: "mock-local"
      }
    ]);
    const withMarkdown = attachEditedMarkdown(withDeverbalized, "# 产品评审\n\n我们今天看一下产品方案。");
    const withHistory = addEditHistoryEntry(withMarkdown, {
      id: "edit-1",
      kind: "markdown-edit",
      summary: "人工调整标题",
      createdAt: "2026-05-17T09:03:00.000Z",
      actor: "user"
    });

    expect(withHistory.originalTranscript).toEqual(withTranscript.originalTranscript);
    expect(withHistory.originalTranscript[0]?.text).toBe("呃我们今天看一下产品方案");
    expect(withHistory.deverbalizedSegments[0]?.text).toBe("我们今天看一下产品方案。");
    expect(withHistory.editedMarkdown).toContain("# 产品评审");
    expect(withHistory.editHistory).toEqual([
      {
        id: "edit-1",
        kind: "markdown-edit",
        summary: "人工调整标题",
        createdAt: "2026-05-17T09:03:00.000Z",
        actor: "user"
      }
    ]);
  });

  test("does not overwrite original transcript on later transcript attachments", () => {
    const project = attachTranscript(
      createMeetingProject({
        path: "/meetings/raw.wav",
        hash: "raw-hash",
        durationSeconds: 60,
        createdAt: "2026-05-17T09:00:00.000Z"
      }),
      [{ id: "seg-1", startMs: 0, endMs: 1000, text: "raw words", language: "en" }]
    );

    const updated = attachTranscript(project, [
      { id: "seg-1-cleaned", startMs: 0, endMs: 1000, text: "cleaned words", language: "en" }
    ]);

    expect(updated.originalTranscript).toEqual(project.originalTranscript);
    expect(updated.transcript).toEqual([
      { id: "seg-1-cleaned", startMs: 0, endMs: 1000, text: "cleaned words", language: "en" }
    ]);
    expect(updated.transcript).not.toBe(updated.originalTranscript);
  });

  test("validates the project manifest shape for persistence", () => {
    const project = addEditHistoryEntry(
      attachEditedMarkdown(
        attachTranscript(
          createMeetingProject({
            path: "/meetings/standup.mp3",
            hash: "standup-hash",
            durationSeconds: 90,
            createdAt: "2026-05-17T09:00:00.000Z",
            title: "Standup"
          }),
          [{ id: "seg-1", startMs: 0, endMs: 1000, text: "hello", language: "en" }]
        ),
        "## Standup"
      ),
      {
        id: "edit-1",
        kind: "title-change",
        summary: "Set meeting title",
        createdAt: "2026-05-17T09:01:00.000Z",
        actor: "user"
      }
    );

    expect(validateMeetingProjectManifest(project)).toEqual({ ok: true, errors: [] });
    expect(createMeetingProjectManifest(project)).toEqual(project);
    expect(restoreMeetingProjectManifest(JSON.parse(JSON.stringify(project)) as unknown)).toEqual(project);
    expect(() => restoreMeetingProjectManifest({ ...project, title: "" })).toThrow(
      "Invalid meeting project manifest: title is required"
    );

    expect(
      validateMeetingProjectManifest({
        ...project,
        schemaVersion: 2,
        id: "manual-id",
        originalTranscript: [{ id: "bad", startMs: 1000, endMs: 100, text: "", language: "en" }]
      })
    ).toEqual({
      ok: false,
      errors: [
        "schemaVersion must be 1",
        "id must be derived from audioHash",
        "originalTranscript[0].endMs must be greater than startMs",
        "originalTranscript[0].text is required"
      ]
    });
  });

  test("restores cloned nested state from persisted manifests", () => {
    const project = addEditHistoryEntry(
      attachSpeakerState(
        attachDeverbalizedState(
          attachTranscript(
            createMeetingProject({
              path: "/meetings/clone.m4a",
              hash: "clone-hash",
              durationSeconds: 120,
              createdAt: "2026-05-17T09:00:00.000Z"
            }),
            [{ id: "seg-1", startMs: 0, endMs: 1000, text: "hello", language: "en" }]
          ),
          [
            {
              id: "clean-1",
              sourceSegmentIds: ["seg-1"],
              speakerId: "speaker-1",
              text: "Hello.",
              createdAt: "2026-05-17T09:02:00.000Z",
              provider: "mock-local"
            }
          ]
        ),
        [{ id: "speaker-1", label: "Host" }],
        [{ id: "speaker-seg-1", speakerId: "speaker-1", startMs: 0, endMs: 1000 }],
        []
      ),
      {
        id: "edit-1",
        kind: "llm-deverbalized",
        summary: "Generate cleaned text",
        createdAt: "2026-05-17T09:03:00.000Z",
        actor: "system"
      }
    );

    const restored = restoreMeetingProjectManifest(JSON.parse(JSON.stringify(project)) as unknown);

    expect(restored).toEqual(project);
    expect(restored.originalTranscript).not.toBe(project.originalTranscript);
    expect(restored.deverbalizedSegments[0]?.sourceSegmentIds).not.toBe(project.deverbalizedSegments[0]?.sourceSegmentIds);
    expect(restored.editHistory).not.toBe(project.editHistory);
  });

  test("rejects invalid manifests before restore", () => {
    expect(validateMeetingProjectManifest(null)).toEqual({
      ok: false,
      errors: ["manifest must be an object"]
    });
    expect(() => restoreMeetingProjectManifest(null)).toThrow("Invalid meeting project manifest: manifest must be an object");

    const project = createMeetingProject({
      path: "/meetings/missing-fields.wav",
      hash: "missing-fields",
      durationSeconds: 90,
      createdAt: "2026-05-17T09:00:00.000Z"
    });

    const result = validateMeetingProjectManifest({
      ...project,
      id: "",
      title: "",
      audioHash: "",
      audioPath: "",
      format: ".flac",
      durationSeconds: -1,
      createdAt: "",
      originalTranscript: "bad",
      transcript: ["bad"],
      speakerSegments: "bad",
      speakers: "bad",
      corrections: "bad",
      deverbalizedSegments: "bad",
      editHistory: "bad"
    });

    expect(result.ok).toBe(false);
    expect(result.errors).toEqual([
      "audioHash is required",
      "id is required",
      "title is required",
      "audioPath is required",
      "format must be a supported audio extension",
      "durationSeconds must be positive",
      "createdAt is required",
      "originalTranscript must be an array",
      "transcript[0] must be an object",
      "speakerSegments must be an array",
      "speakers must be an array",
      "corrections must be an array",
      "deverbalizedSegments must be an array",
      "editHistory must be an array"
    ]);
  });

  test("validates speaker, deverbalized, markdown, and edit history state shapes", () => {
    const project = createMeetingProject({
      path: "/meetings/schema.wav",
      hash: "schema-hash",
      durationSeconds: 90,
      createdAt: "2026-05-17T09:00:00.000Z"
    });

    expect(
      validateMeetingProjectManifest({
        ...project,
        speakers: [{ id: "", label: "" }],
        speakerSegments: [{ id: "spk-1", speakerId: "", startMs: -1, endMs: 0 }],
        corrections: [{ id: "correction-1", targetSpeakerId: "", replacementSpeakerId: "", reason: "", createdAt: "" }],
        deverbalizedSegments: [{ id: "clean-1", sourceSegmentIds: [], speakerId: "", text: "", createdAt: "", provider: "" }],
        editedMarkdown: 42,
        editHistory: [{ id: "edit-1", kind: "summary", summary: "", createdAt: "", actor: "bot" }]
      })
    ).toEqual({
      ok: false,
      errors: [
        "speakerSegments[0].speakerId is required",
        "speakerSegments[0].startMs must be non-negative",
        "speakers[0].id is required",
        "speakers[0].label is required",
        "corrections[0].targetSpeakerId is required",
        "corrections[0].replacementSpeakerId is required",
        "corrections[0].reason is required",
        "corrections[0].createdAt is required",
        "deverbalizedSegments[0].sourceSegmentIds must not be empty",
        "deverbalizedSegments[0].speakerId is required",
        "deverbalizedSegments[0].text is required",
        "deverbalizedSegments[0].createdAt is required",
        "deverbalizedSegments[0].provider is required",
        "editHistory[0].kind is invalid",
        "editHistory[0].summary is required",
        "editHistory[0].createdAt is required",
        "editHistory[0].actor is invalid",
        "editedMarkdown must be a string or null"
      ]
    });
  });

  test("validates structured correction record shapes and speaker references", () => {
    const project = attachSpeakerState(
      createMeetingProject({
        path: "/meetings/structured-invalid.wav",
        hash: "structured-invalid-hash",
        durationSeconds: 90,
        createdAt: "2026-05-20T09:00:00.000Z"
      }),
      [{ id: "speaker-a", label: "主持人" }],
      [{ id: "spk-1", speakerId: "speaker-a", startMs: 0, endMs: 1000 }],
      [
        {
          id: "cmd-invalid",
          commandId: "",
          kind: "",
          targetSpeakerId: "speaker-a",
          replacementSpeakerId: "speaker-a",
          reason: "manual-range-assign",
          createdAt: "2026-05-20T09:01:00.000Z",
          beforeSegments: [
            { id: "before-1", speakerId: "speaker-missing", startMs: 0, endMs: 1000, confidence: 2 }
          ],
          afterSegments: [{ id: "after-1", speakerId: "speaker-missing", startMs: 1000, endMs: 1000 }]
        }
      ]
    );

    expect(validateMeetingProjectManifest(project).errors).toEqual([
      "corrections[0].commandId must be a non-empty string",
      "corrections[0].kind must be a non-empty string",
      "corrections[0].beforeSegments[0].confidence must be between 0 and 1",
      "corrections[0].afterSegments[0].endMs must be greater than startMs",
      "corrections[0].afterSegments[0].speakerId must reference a known speaker"
    ]);
  });

  test("requires callers to resolve rerun diarization conflicts before overwriting manual corrections", () => {
    const project = attachSpeakerState(
      createMeetingProject({
        path: "/meetings/rerun-guard.wav",
        hash: "rerun-guard-hash",
        durationSeconds: 90,
        createdAt: "2026-05-20T09:00:00.000Z"
      }),
      [
        { id: "s1", label: "Speaker 1" },
        { id: "s2", label: "Speaker 2" }
      ],
      [{ id: "spk-1", speakerId: "s2", startMs: 0, endMs: 1000 }],
      [
        {
          id: "cmd-range",
          commandId: "cmd-range",
          kind: "assign-range",
          targetSpeakerId: "s2",
          replacementSpeakerId: "s2",
          reason: "manual-range-assign",
          createdAt: "2026-05-20T09:01:00.000Z",
          beforeSegments: [{ id: "spk-1", speakerId: "s1", startMs: 0, endMs: 1000 }],
          afterSegments: [{ id: "spk-1", speakerId: "s2", startMs: 0, endMs: 1000 }]
        }
      ]
    );

    const resolution = attachSpeakerStateFromDiarizationRerun(
      project,
      project.speakers,
      [{ id: "spk-1", speakerId: "s1", startMs: 0, endMs: 1000 }]
    );

    expect(resolution.status).toBe("conflict");
    expect(resolution.project).toEqual(project);
    if (resolution.status === "conflict") {
      expect(resolution.conflicts).toEqual([
        {
          segmentId: "spk-1",
          startMs: 0,
          endMs: 1000,
          currentSpeakerId: "s2",
          generatedSpeakerId: "s1",
          reason: "manual-edit-would-change"
        }
      ]);
    }
  });

  test("preserves manual speaker rename labels when rerun diarization regenerates speaker profiles", () => {
    const project = attachSpeakerState(
      createMeetingProject({
        path: "/meetings/rerun-rename-guard.wav",
        hash: "rerun-rename-guard-hash",
        durationSeconds: 90,
        createdAt: "2026-05-20T09:00:00.000Z"
      }),
      [{ id: "speaker-1", label: "Speaker 1" }],
      [{ id: "spk-1", speakerId: "speaker-1", startMs: 0, endMs: 1000 }],
      []
    );
    const manualState = applySpeakerCorrectionCommands(project, [
      {
        id: "cmd-rename-speaker-1",
        kind: "rename-speaker",
        speakerId: "speaker-1",
        label: "主持人",
        createdAt: "2026-05-20T09:01:00.000Z"
      }
    ]);
    const renamedProject = attachSpeakerState(
      project,
      manualState.speakers,
      manualState.speakerSegments,
      manualState.corrections
    );

    const resolution = attachSpeakerStateFromDiarizationRerun(
      renamedProject,
      [{ id: "speaker-1", label: "Speaker 1" }],
      [{ id: "spk-1", speakerId: "speaker-1", startMs: 0, endMs: 1000 }]
    );

    expect(resolution.status).toBe("conflict");
    expect(resolution.project).toEqual(renamedProject);
    if (resolution.status === "conflict") {
      expect(resolution.conflicts).toEqual([
        {
          speakerId: "speaker-1",
          currentLabel: "主持人",
          generatedLabel: "Speaker 1",
          reason: "manual-rename-would-change"
        }
      ]);
    }
  });

  test("validates nested non-object entries and timing bounds", () => {
    const project = createMeetingProject({
      path: "/meetings/nested.wav",
      hash: "nested-hash",
      durationSeconds: 90,
      createdAt: "2026-05-17T09:00:00.000Z"
    });

    expect(
      validateMeetingProjectManifest({
        ...project,
        originalTranscript: [{ id: "", startMs: -1, endMs: 0, text: " ", language: "fr" }],
        transcript: [{ id: "seg-1", startMs: 1, endMs: 1, text: "ok", language: "zh" }],
        speakerSegments: [
          "bad",
          { id: "", speakerId: "speaker-1", startMs: 0, endMs: 0, confidence: 2 }
        ],
        speakers: ["bad"],
        corrections: ["bad", { id: "", targetSpeakerId: "a", replacementSpeakerId: "b", reason: "merge", createdAt: "now" }],
        deverbalizedSegments: ["bad", { id: "", sourceSegmentIds: ["seg-1"], speakerId: "speaker-1", text: "ok", createdAt: "now", provider: "mock" }],
        editHistory: ["bad", { id: "", kind: "markdown-edit", summary: "ok", createdAt: "now", actor: "user" }]
      }).errors
    ).toEqual([
      "originalTranscript[0].id is required",
      "originalTranscript[0].startMs must be non-negative",
      "originalTranscript[0].text is required",
      "originalTranscript[0].language is invalid",
      "transcript[0].endMs must be greater than startMs",
      "speakerSegments[0] must be an object",
      "speakerSegments[1].id is required",
      "speakerSegments[1].endMs must be greater than startMs",
      "speakerSegments[1].confidence must be between 0 and 1",
      "speakers[0] must be an object",
      "corrections[0] must be an object",
      "corrections[1].id is required",
      "deverbalizedSegments[0] must be an object",
      "deverbalizedSegments[1].id is required",
      "editHistory[0] must be an object",
      "editHistory[1].id is required",
      "speakerSegments[1].speakerId must reference a known speaker",
      "deverbalizedSegments[1].speakerId must reference a known speaker"
    ]);
  });

  test("validates source segment ids and traceable speaker/deverbalized references", () => {
    const project = attachSpeakerState(
      attachTranscript(
        createMeetingProject({
          path: "/meetings/references.wav",
          hash: "references-hash",
          durationSeconds: 90,
          createdAt: "2026-05-17T09:00:00.000Z"
        }),
        [{ id: "seg-1", startMs: 0, endMs: 1000, text: "hello", language: "en" }]
      ),
      [{ id: "speaker-1", label: "Host" }],
      [{ id: "speaker-seg-1", speakerId: "speaker-missing", startMs: 0, endMs: 1000 }],
      []
    );

    expect(
      validateMeetingProjectManifest({
        ...project,
        deverbalizedSegments: [
          {
            id: "clean-1",
            sourceSegmentIds: ["seg-missing", ""],
            speakerId: "speaker-missing",
            text: "Hello.",
            createdAt: "2026-05-17T09:01:00.000Z",
            provider: "mock"
          }
        ]
      }).errors
    ).toEqual([
      "deverbalizedSegments[0].sourceSegmentIds[1] must be a non-empty string",
      "speakerSegments[0].speakerId must reference a known speaker",
      "deverbalizedSegments[0].speakerId must reference a known speaker",
      "deverbalizedSegments[0].sourceSegmentIds[0] must reference a known transcript segment"
    ]);
  });
});
