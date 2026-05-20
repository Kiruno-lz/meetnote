export const MARKDOWN_LIBRARY_SCHEMA_VERSION = "meetnote.markdown-library.v1" as const;

export interface ValidationResult {
  readonly ok: boolean;
  readonly errors: readonly string[];
}

export interface MarkdownFrontmatterInput {
  readonly title: string;
  readonly date: string;
  readonly audioHash: string;
  readonly speakers: readonly string[];
  readonly modelVersion: string;
  readonly llmProvider: "ollama" | "custom";
  readonly schemaVersion: typeof MARKDOWN_LIBRARY_SCHEMA_VERSION;
  readonly generatedAt: string;
}

export interface MarkdownFrontmatter extends MarkdownFrontmatterInput {
  readonly kind: "markdown.frontmatter";
}

export interface MarkdownMeetingInput {
  readonly frontmatter: MarkdownFrontmatter;
  readonly content: string;
}

export interface MarkdownMeeting extends MarkdownMeetingInput {
  readonly kind: "markdown.meeting";
}

export interface MarkdownLibraryManifestInput {
  readonly version: typeof MARKDOWN_LIBRARY_SCHEMA_VERSION;
  readonly baseDir: string;
  readonly indexedAt: string;
  readonly meetingCount: number;
}

export interface MarkdownLibraryManifest extends MarkdownLibraryManifestInput {
  readonly kind: "markdown.library.manifest";
}

export function createMarkdownFrontmatter(input: MarkdownFrontmatterInput): MarkdownFrontmatter {
  const frontmatter: MarkdownFrontmatter = {
    kind: "markdown.frontmatter",
    ...input
  };
  assertValidFrontmatter(frontmatter);
  return frontmatter;
}

export function createMarkdownMeeting(input: MarkdownMeetingInput): MarkdownMeeting {
  const meeting: MarkdownMeeting = {
    kind: "markdown.meeting",
    ...input
  };
  assertValidMeeting(meeting);
  return meeting;
}

export function createMarkdownLibraryManifest(input: MarkdownLibraryManifestInput): MarkdownLibraryManifest {
  const manifest: MarkdownLibraryManifest = {
    kind: "markdown.library.manifest",
    ...input
  };
  assertValidManifest(manifest);
  return manifest;
}

export function validateMarkdownFrontmatter(value: unknown): ValidationResult {
  const errors: string[] = [];

  if (!isRecord(value)) {
    return { ok: false, errors: ["MARKDOWN_FRONTMATTER_INVALID: frontmatter must be an object"] };
  }

  if (value.schemaVersion !== MARKDOWN_LIBRARY_SCHEMA_VERSION) {
    errors.push("MARKDOWN_SCHEMA_VERSION_INVALID: schemaVersion must be meetnote.markdown-library.v1");
  }

  requireNonEmptyString(value.title, "MARKDOWN_TITLE_REQUIRED: title is required", errors);
  validateDateFormat(value.date, "MARKDOWN_DATE_INVALID: date must be ISO 8601 format (YYYY-MM-DD)", errors);
  validateAudioHash(value.audioHash, errors);
  validateSpeakers(value.speakers, errors);
  requireNonEmptyString(value.modelVersion, "MARKDOWN_MODEL_VERSION_REQUIRED: modelVersion is required", errors);

  if (!["ollama", "custom"].includes(value.llmProvider as string)) {
    errors.push("MARKDOWN_LLM_PROVIDER_INVALID: llmProvider must be ollama or custom");
  }

  validateDateFormat(value.generatedAt, "MARKDOWN_GENERATED_AT_INVALID: generatedAt must be ISO 8601 format", errors);

  return { ok: errors.length === 0, errors };
}

export function validateMarkdownMeeting(value: unknown): ValidationResult {
  const errors: string[] = [];

  if (!isRecord(value)) {
    return { ok: false, errors: ["MARKDOWN_MEETING_INVALID: meeting must be an object"] };
  }

  if (!isRecord(value.frontmatter)) {
    errors.push("MARKDOWN_FRONTMATTER_MISSING: frontmatter must be an object");
  } else {
    const frontmatterResult = validateMarkdownFrontmatter(value.frontmatter);
    errors.push(...frontmatterResult.errors);
  }

  if (typeof value.content !== "string") {
    errors.push("MARKDOWN_CONTENT_INVALID: content must be a string");
  } else if (value.content.trim().length === 0) {
    errors.push("MARKDOWN_CONTENT_EMPTY: content must not be empty");
  }

  return { ok: errors.length === 0, errors };
}

export function validateMarkdownLibraryManifest(value: unknown): ValidationResult {
  const errors: string[] = [];

  if (!isRecord(value)) {
    return { ok: false, errors: ["MARKDOWN_MANIFEST_INVALID: manifest must be an object"] };
  }

  if (value.version !== MARKDOWN_LIBRARY_SCHEMA_VERSION) {
    errors.push("MARKDOWN_MANIFEST_VERSION_INVALID: version must be meetnote.markdown-library.v1");
  }

  requireNonEmptyString(value.baseDir, "MARKDOWN_BASE_DIR_REQUIRED: baseDir is required", errors);
  validateDateFormat(value.indexedAt, "MARKDOWN_INDEXED_AT_INVALID: indexedAt must be ISO 8601 format", errors);

  if (typeof value.meetingCount !== "number" || !Number.isInteger(value.meetingCount) || value.meetingCount < 0) {
    errors.push("MARKDOWN_MEETING_COUNT_INVALID: meetingCount must be a non-negative integer");
  }

  return { ok: errors.length === 0, errors };
}

export function serializeMarkdownFrontmatter(frontmatter: MarkdownFrontmatter): string {
  const speakersYaml = frontmatter.speakers
    .map((s) => (s.includes(",") || s.includes(":") ? `"${s.replace(/"/g, '\\"')}"` : s))
    .join(", ");

  const lines = [
    "---",
    `title: ${escapeYamlValue(frontmatter.title)}`,
    `date: ${frontmatter.date}`,
    `audioHash: ${frontmatter.audioHash}`,
    `speakers: [${speakersYaml}]`,
    `modelVersion: ${frontmatter.modelVersion}`,
    `llmProvider: ${frontmatter.llmProvider}`,
    `schemaVersion: ${frontmatter.schemaVersion}`,
    `generatedAt: ${frontmatter.generatedAt}`,
    "---"
  ];
  return lines.join("\n");
}

export function parseMarkdownFrontmatter(text: string): ValidationResult & { readonly frontmatter?: MarkdownFrontmatter } {
  const match = text.match(/^---\n([\s\S]*?)\n---/);
  if (!match || !match[1]) {
    return { ok: false, errors: ["MARKDOWN_FRONTMATTER_MISSING: frontmatter block not found"] };
  }

  const frontmatterText = match[1];
  const parsed: Record<string, unknown> = {};

  const lines = frontmatterText.split("\n");
  for (const line of lines) {
    const colonIndex = line.indexOf(":");
    if (colonIndex === -1) continue;

    const key = line.substring(0, colonIndex).trim();
    const value = line.substring(colonIndex + 1).trim();

    if (key === "speakers") {
      const speakersMatch = value.match(/^\[(.*)\]$/);
      if (speakersMatch && speakersMatch[1]) {
        parsed[key] = parseSpeakersArray(speakersMatch[1]);
      } else {
        parsed[key] = [];
      }
    } else {
      parsed[key] = value.replace(/^["']|["']$/g, "");
    }
  }

  parsed.schemaVersion = MARKDOWN_LIBRARY_SCHEMA_VERSION;
  parsed.kind = "markdown.frontmatter";

  const result = validateMarkdownFrontmatter(parsed);
  if (!result.ok) {
    return result;
  }

  return { ok: true, errors: [], frontmatter: parsed as unknown as MarkdownFrontmatter };
}

function parseSpeakersArray(speakersStr: string): string[] {
  const speakers: string[] = [];
  let current = "";
  let inQuotes = false;

  for (let i = 0; i < speakersStr.length; i++) {
    const char = speakersStr[i];

    if (char === '"' && (i === 0 || speakersStr[i - 1] !== "\\")) {
      inQuotes = !inQuotes;
    } else if (char === "," && !inQuotes) {
      const trimmed = current.trim().replace(/^["']|["']$/g, "");
      if (trimmed) speakers.push(trimmed);
      current = "";
    } else {
      current += char;
    }
  }

  const trimmed = current.trim().replace(/^["']|["']$/g, "");
  if (trimmed) speakers.push(trimmed);

  return speakers;
}

function validateDateFormat(value: unknown, errorMessage: string, errors: string[]): void {
  if (typeof value !== "string") {
    errors.push(errorMessage);
    return;
  }

  const isoDateRegex = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d{3})?Z?$/;
  const dateOnlyRegex = /^\d{4}-\d{2}-\d{2}$/;

  if (!isoDateRegex.test(value) && !dateOnlyRegex.test(value)) {
    errors.push(errorMessage);
    return;
  }

  const dateStr = value.split("T")[0];
  if (dateStr) {
    const date = new Date(dateStr);
    if (Number.isNaN(date.getTime())) {
      errors.push(errorMessage);
    }
  }
}

function validateAudioHash(value: unknown, errors: string[]): void {
  if (typeof value !== "string") {
    errors.push("MARKDOWN_AUDIO_HASH_REQUIRED: audioHash is required");
    return;
  }

  if (!value || value.length === 0) {
    errors.push("MARKDOWN_AUDIO_HASH_REQUIRED: audioHash is required");
  }
}

function validateSpeakers(value: unknown, errors: string[]): void {
  if (!Array.isArray(value)) {
    errors.push("MARKDOWN_SPEAKERS_INVALID: speakers must be an array");
    return;
  }

  if (value.length === 0) {
    errors.push("MARKDOWN_SPEAKERS_EMPTY: speakers must not be empty");
    return;
  }

  value.forEach((speaker, index) => {
    if (!isNonEmptyString(speaker)) {
      errors.push(`MARKDOWN_SPEAKER_INVALID: speakers[${index}] must be a non-empty string`);
    }
  });
}

function escapeYamlValue(value: string): string {
  if (value.includes(",") || value.includes(":") || value.includes("[") || value.includes("]")) {
    return `"${value.replace(/"/g, '\\"')}"`;
  }
  return value;
}

function assertValidFrontmatter(frontmatter: MarkdownFrontmatter): void {
  const result = validateMarkdownFrontmatter(frontmatter);
  if (!result.ok) {
    throw new Error(result.errors.join("; "));
  }
}

function assertValidMeeting(meeting: MarkdownMeeting): void {
  const result = validateMarkdownMeeting(meeting);
  if (!result.ok) {
    throw new Error(result.errors.join("; "));
  }
}

function assertValidManifest(manifest: MarkdownLibraryManifest): void {
  const result = validateMarkdownLibraryManifest(manifest);
  if (!result.ok) {
    throw new Error(result.errors.join("; "));
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

function requireNonEmptyString(value: unknown, code: string, errors: string[]): void {
  if (!isNonEmptyString(value)) {
    errors.push(code);
  }
}
