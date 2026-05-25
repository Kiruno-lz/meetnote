import { describe, expect, test } from "bun:test";
import {
  validateAudioFile,
  extractDefaultTitle,
  computeFileHash,
  extractDurationFromAudioElement
} from "../src/utils/audio-file-import";
import type { MockableAudioElement } from "../src/utils/audio-file-import";

// ---------------------------------------------------------------------------
// validateAudioFile
// ---------------------------------------------------------------------------

describe("validateAudioFile", () => {
  test("accepts .wav files", () => {
    const result = validateAudioFile("meeting.wav", 1000);
    expect(result.ok).toBe(true);
    expect(result.errors).toHaveLength(0);
    expect(result.extension).toBe(".wav");
  });

  test("accepts .mp3 files", () => {
    const result = validateAudioFile("record.mp3", 1000);
    expect(result.ok).toBe(true);
    expect(result.extension).toBe(".mp3");
  });

  test("accepts .m4a files", () => {
    const result = validateAudioFile("audio.m4a", 1000);
    expect(result.ok).toBe(true);
    expect(result.extension).toBe(".m4a");
  });

  test("accepts uppercase extension .WAV", () => {
    const result = validateAudioFile("meeting.WAV", 1000);
    expect(result.ok).toBe(true);
    expect(result.extension).toBe(".wav");
  });

  test("accepts uppercase extension .MP3", () => {
    const result = validateAudioFile("record.MP3", 1000);
    expect(result.ok).toBe(true);
    expect(result.extension).toBe(".mp3");
  });

  test("rejects .txt files", () => {
    const result = validateAudioFile("notes.txt", 1000);
    expect(result.ok).toBe(false);
    expect(result.errors.length).toBeGreaterThan(0);
    expect(result.extension).toBeNull();
  });

  test("rejects .pdf files", () => {
    const result = validateAudioFile("document.pdf", 1000);
    expect(result.ok).toBe(false);
    expect(result.extension).toBeNull();
  });

  test("rejects .mp4 files", () => {
    const result = validateAudioFile("video.mp4", 1000);
    expect(result.ok).toBe(false);
    expect(result.extension).toBeNull();
  });

  test("rejects .avi files", () => {
    const result = validateAudioFile("video.avi", 1000);
    expect(result.ok).toBe(false);
    expect(result.extension).toBeNull();
  });

  test("rejects empty file (sizeBytes === 0)", () => {
    const result = validateAudioFile("meeting.wav", 0);
    expect(result.ok).toBe(false);
    expect(result.errors.some((e) => e.includes("empty"))).toBe(true);
  });

  test("rejects negative size", () => {
    const result = validateAudioFile("meeting.wav", -1);
    expect(result.ok).toBe(false);
  });

  test("rejects file smaller than 100 bytes", () => {
    const result = validateAudioFile("meeting.wav", 50);
    expect(result.ok).toBe(false);
    expect(result.errors.some((e) => e.includes("small"))).toBe(true);
  });

  test("accepts file exactly at 100 bytes threshold", () => {
    const result = validateAudioFile("meeting.wav", 100);
    expect(result.ok).toBe(true);
  });

  test("returns correct extension for valid files", () => {
    expect(validateAudioFile("a.wav", 200).extension).toBe(".wav");
    expect(validateAudioFile("a.mp3", 200).extension).toBe(".mp3");
    expect(validateAudioFile("a.m4a", 200).extension).toBe(".m4a");
  });

  test("accepts valid audio file when sizeBytes is omitted (Tauri path)", () => {
    const result = validateAudioFile("audio.m4a");
    expect(result.ok).toBe(true);
    expect(result.errors).toHaveLength(0);
    expect(result.extension).toBe(".m4a");
  });

  test("accepts valid audio file when sizeBytes is explicitly undefined", () => {
    const result = validateAudioFile("audio.m4a", undefined);
    expect(result.ok).toBe(true);
    expect(result.errors).toHaveLength(0);
    expect(result.extension).toBe(".m4a");
  });
});

// ---------------------------------------------------------------------------
// extractDefaultTitle
// ---------------------------------------------------------------------------

describe("extractDefaultTitle", () => {
  test("strips .wav extension", () => {
    expect(extractDefaultTitle("meeting-2024.wav")).toBe("meeting-2024");
  });

  test("strips .m4a extension with unicode filename", () => {
    expect(extractDefaultTitle("Q2_产品规划.m4a")).toBe("Q2_产品规划");
  });

  test("strips .mp3 extension", () => {
    expect(extractDefaultTitle("record.mp3")).toBe("record");
  });

  test("truncates titles longer than 50 characters", () => {
    const longName = "a".repeat(60) + ".wav";
    const result = extractDefaultTitle(longName);
    expect(result.length).toBeLessThanOrEqual(50);
  });

  test("truncates filename without extension that exceeds 50 characters", () => {
    const longName = "b".repeat(60);
    const result = extractDefaultTitle(longName);
    expect(result.length).toBeLessThanOrEqual(50);
  });

  test("returns original filename (truncated) when no extension is present", () => {
    expect(extractDefaultTitle("README")).toBe("README");
    expect(extractDefaultTitle("noextension")).toBe("noextension");
  });

  test("does not strip leading dot (hidden files)", () => {
    // ".hidden" — lastDot is at index 0, so we treat as no extension
    expect(extractDefaultTitle(".hidden")).toBe(".hidden");
  });

  test("truncates to exactly 50 chars for exactly-50-char name", () => {
    const name = "c".repeat(50) + ".wav";
    expect(extractDefaultTitle(name).length).toBe(50);
  });
});

// ---------------------------------------------------------------------------
// computeFileHash
// ---------------------------------------------------------------------------

describe("computeFileHash", () => {
  test("returns a 64-character lowercase hex string", async () => {
    const hash = await computeFileHash(new ArrayBuffer(0));
    expect(hash).toHaveLength(64);
    expect(/^[0-9a-f]{64}$/.test(hash)).toBe(true);
  });

  test("is deterministic — same input returns same hash", async () => {
    const buf = new Uint8Array([1, 2, 3]).buffer;
    const hash1 = await computeFileHash(buf);
    const hash2 = await computeFileHash(buf);
    expect(hash1).toBe(hash2);
  });

  test("different content returns different hash", async () => {
    const hash1 = await computeFileHash(new Uint8Array([1, 2, 3]).buffer);
    const hash2 = await computeFileHash(new Uint8Array([4, 5, 6]).buffer);
    expect(hash1).not.toBe(hash2);
  });

  test("empty and non-empty buffer produce different hashes", async () => {
    const empty = await computeFileHash(new ArrayBuffer(0));
    const nonEmpty = await computeFileHash(new Uint8Array([1]).buffer);
    expect(empty).not.toBe(nonEmpty);
  });

  test("single byte difference produces different hash", async () => {
    const hash1 = await computeFileHash(new Uint8Array([0, 0, 0]).buffer);
    const hash2 = await computeFileHash(new Uint8Array([0, 0, 1]).buffer);
    expect(hash1).not.toBe(hash2);
  });
});

// ---------------------------------------------------------------------------
// extractDurationFromAudioElement
// ---------------------------------------------------------------------------

function makeMockAudio(): MockableAudioElement {
  const el: MockableAudioElement = {
    src: "",
    duration: 0,
    onloadedmetadata: null,
    onerror: null,
    load() {}
  };
  return el;
}

describe("extractDurationFromAudioElement", () => {
  test("resolves with duration when onloadedmetadata fires", async () => {
    let audioRef: MockableAudioElement | null = null;

    const createFn = () => {
      const el = makeMockAudio();
      audioRef = el;
      return el;
    };

    const promise = extractDurationFromAudioElement(createFn, "blob:fake-url");

    // Simulate metadata loaded
    if (audioRef === null) throw new Error("audio element not created");
    const el = audioRef as MockableAudioElement;
    el.duration = 123.45;
    el.onloadedmetadata?.call(el, new Event("loadedmetadata"));

    const duration = await promise;
    expect(duration).toBe(123.45);
  });

  test("rejects when onerror fires", async () => {
    let audioRef: MockableAudioElement | null = null;

    const createFn = () => {
      const el = makeMockAudio();
      audioRef = el;
      return el;
    };

    const promise = extractDurationFromAudioElement(createFn, "blob:fake-url");

    if (audioRef === null) throw new Error("audio element not created");
    const el = audioRef as MockableAudioElement;
    el.onerror?.call(el, new Event("error"));

    await expect(promise).rejects.toThrow();
  });

  test("rejects on timeout when no event fires", async () => {
    const createFn = () => makeMockAudio();
    // Use a very short timeout — 30ms
    const promise = extractDurationFromAudioElement(createFn, "blob:fake-url", 30);
    await expect(promise).rejects.toThrow(/timed out/i);
  });

  test("sets src and calls load on the audio element", () => {
    let loadCalled = false;
    let srcSet = "";

    const createFn = () => {
      const el = makeMockAudio();
      const originalLoad = el.load.bind(el);
      el.load = () => {
        loadCalled = true;
        originalLoad();
      };
      // Intercept src via Object.defineProperty to track assignment
      let _src = "";
      Object.defineProperty(el, "src", {
        get: () => _src,
        set: (v: string) => {
          _src = v;
          srcSet = v;
        }
      });
      // fire metadata immediately after src is set
      setTimeout(() => {
        el.duration = 5;
        el.onloadedmetadata?.call(el, {});
      }, 0);
      return el;
    };

    const url = "blob:test-url";
    return extractDurationFromAudioElement(createFn, url).then((dur) => {
      expect(loadCalled).toBe(true);
      expect(srcSet).toBe(url);
      expect(dur).toBe(5);
    });
  });
});
