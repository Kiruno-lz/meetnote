export interface DeverbalizationOutput {
  readonly kind: "deverbalization";
  readonly schemaVersion: "meetnote.deverbalization.v1";
  readonly provider: "ollama" | "custom";
  readonly segments: readonly {
    readonly sourceSegmentIds: readonly string[];
    readonly speakerId: string;
    readonly text: string;
  }[];
  readonly summary?: never;
  readonly actionItems?: never;
  readonly conclusions?: never;
}

export interface ValidationResult {
  readonly ok: boolean;
  readonly errors: readonly string[];
}

const FORBIDDEN_TOP_LEVEL_KEYS = ["summary", "actionItems", "actions", "todos", "conclusions"] as const;

export function validateDeverbalizationOutput(value: unknown): ValidationResult {
  const errors: string[] = [];

  if (!isRecord(value)) {
    return { ok: false, errors: ["Output must be an object"] };
  }

  if (value.kind !== "deverbalization") {
    errors.push("kind must be deverbalization");
  }

  if (value.schemaVersion !== "meetnote.deverbalization.v1") {
    errors.push("schemaVersion must be meetnote.deverbalization.v1");
  }

  if (value.provider !== "ollama" && value.provider !== "custom") {
    errors.push("provider must be ollama or custom");
  }

  for (const key of FORBIDDEN_TOP_LEVEL_KEYS) {
    if (key in value) {
      errors.push(`Forbidden top-level key: ${key}`);
    }
  }

  if (!Array.isArray(value.segments) || value.segments.length === 0) {
    errors.push("segments must be a non-empty array");
  } else {
    value.segments.forEach((segment, index) => {
      if (!isRecord(segment)) {
        errors.push(`segments[${index}] must be an object`);
        return;
      }
      if (!Array.isArray(segment.sourceSegmentIds) || segment.sourceSegmentIds.length === 0) {
        errors.push(`segments[${index}].sourceSegmentIds must be a non-empty array`);
      }
      if (typeof segment.speakerId !== "string" || segment.speakerId.trim() === "") {
        errors.push(`segments[${index}].speakerId must be a non-empty string`);
      }
      if (typeof segment.text !== "string" || segment.text.trim() === "") {
        errors.push(`segments[${index}].text must be a non-empty string`);
      }
    });
  }

  return { ok: errors.length === 0, errors };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
