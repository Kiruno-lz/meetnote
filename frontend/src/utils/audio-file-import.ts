import { getSupportedAudioExtension } from "../types/audio-project";
import type { AudioImportInput, SupportedAudioExtension } from "../types/audio-project";

export interface AudioFileValidationResult {
  ok: boolean;
  errors: string[];
  extension: SupportedAudioExtension | null;
}

const MIN_FILE_SIZE_BYTES = 100;

export function validateAudioFile(filename: string, sizeBytes?: number): AudioFileValidationResult {
  const errors: string[] = [];
  const extension = getSupportedAudioExtension(filename);

  if (!extension) {
    errors.push(`Unsupported audio format: ${filename}`);
  }

  if (sizeBytes !== undefined) {
    if (sizeBytes <= 0) {
      errors.push("File is empty");
    } else if (sizeBytes < MIN_FILE_SIZE_BYTES) {
      errors.push(`File is too small (${sizeBytes} bytes, minimum ${MIN_FILE_SIZE_BYTES})`);
    }
  }

  return {
    ok: errors.length === 0,
    errors,
    extension: errors.length === 0 ? extension : null
  };
}

export function extractDefaultTitle(filename: string): string {
  const lastDot = filename.lastIndexOf(".");
  const nameWithoutExt = lastDot > 0 ? filename.slice(0, lastDot) : filename;
  return nameWithoutExt.slice(0, 50);
}

export async function computeFileHash(buffer: ArrayBuffer): Promise<string> {
  const hashBuffer = await globalThis.crypto.subtle.digest("SHA-256", buffer);
  const hashArray = Array.from(new Uint8Array(hashBuffer));
  return hashArray.map((b) => b.toString(16).padStart(2, "0")).join("");
}

export interface MockableAudioElement {
  src: string;
  load(): void;
  duration: number;
  onloadedmetadata: ((this: unknown, ev: unknown) => unknown) | null;
  onerror: ((this: unknown, ev: unknown) => unknown) | null;
}

export function extractDurationFromAudioElement(
  createAudioFn: () => MockableAudioElement,
  objectUrl: string,
  timeoutMs = 30000
): Promise<number> {
  return new Promise<number>((resolve, reject) => {
    const audio = createAudioFn();
    let settled = false;
    let timer: ReturnType<typeof setTimeout> | null = null;

    function cleanup() {
      if (timer !== null) {
        clearTimeout(timer);
        timer = null;
      }
    }

    timer = setTimeout(() => {
      if (!settled) {
        settled = true;
        reject(new Error(`Timed out loading audio metadata after ${timeoutMs}ms`));
      }
    }, timeoutMs);

    audio.onloadedmetadata = function () {
      if (!settled) {
        settled = true;
        cleanup();
        resolve(audio.duration);
      }
    };

    audio.onerror = function (_ev) {
      if (!settled) {
        settled = true;
        cleanup();
        reject(new Error("Failed to load audio metadata"));
      }
    };

    audio.src = objectUrl;
    audio.load();
  });
}

export async function importAudioFile(file: File): Promise<AudioImportInput> {
  const validation = validateAudioFile(file.name, file.size);
  if (!validation.ok) {
    throw new Error(`Invalid audio file: ${validation.errors.join("; ")}`);
  }

  const buffer = await file.arrayBuffer();
  const hash = await computeFileHash(buffer);

  let objectUrl: string | null = null;
  let durationSeconds: number;
  try {
    objectUrl = URL.createObjectURL(file);
    durationSeconds = await extractDurationFromAudioElement(
      () => new Audio() as unknown as MockableAudioElement,
      objectUrl
    );
  } finally {
    if (objectUrl !== null) {
      URL.revokeObjectURL(objectUrl);
    }
  }

  return {
    path: file.name,
    hash,
    durationSeconds,
    createdAt: new Date().toISOString(),
    title: extractDefaultTitle(file.name)
  };
}
