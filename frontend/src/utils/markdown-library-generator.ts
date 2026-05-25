import type { DeverbalizationOutput, DeverbalizedParagraph } from "../types/llm-schema";
import type { MarkdownFrontmatter } from "../types/markdown-library-schema";
import { serializeMarkdownFrontmatter } from "../types/markdown-library-schema";

export interface SpeakerCorrection {
  readonly paragraphId: string;
  readonly originalSpeakerId: string;
  readonly correctedSpeakerId: string;
  readonly correctedSpeakerLabel: string;
  readonly correctedAt: string;
}

export interface GenerateMarkdownOptions {
  readonly frontmatter: MarkdownFrontmatter;
  readonly deverbalizationOutput: DeverbalizationOutput;
  readonly originalTranscript: string;
  readonly speakerCorrections: readonly SpeakerCorrection[];
}

export function generateMarkdownContent(options: GenerateMarkdownOptions): string {
  const frontmatterText = serializeMarkdownFrontmatter(options.frontmatter);
  const contentText = generateContent(
    options.deverbalizationOutput,
    options.speakerCorrections,
    options.frontmatter.speakers
  );
  const appendixText = generateAppendix(options.originalTranscript, options.speakerCorrections);

  return [frontmatterText, "", contentText, "", appendixText].join("\n");
}

function generateContent(
  output: DeverbalizationOutput,
  corrections: readonly SpeakerCorrection[],
  speakers: readonly string[]
): string {
  const correctionMap = new Map(corrections.map((c) => [c.paragraphId, c]));
  const speakerMap = new Map(speakers.map((s) => [s.toLowerCase(), s]));

  const paragraphs = output.paragraphs.map((para) => {
    const correction = correctionMap.get(para.id);
    let speakerLabel = correction ? correction.correctedSpeakerLabel : getSpeakerLabelFromId(para.speakerId);
    const normalized = speakerMap.get(speakerLabel.toLowerCase());
    if (normalized) {
      speakerLabel = normalized;
    }
    return `**${speakerLabel}**: ${para.text}`;
  });

  return paragraphs.join("\n\n");
}

function generateAppendix(originalTranscript: string, corrections: readonly SpeakerCorrection[]): string {
  const sections: string[] = [];

  sections.push("## Appendix");
  sections.push("");
  sections.push("### Original Transcript");
  sections.push("");
  sections.push("<details>");
  sections.push("<summary>Click to expand</summary>");
  sections.push("");
  sections.push(escapeMarkdownContent(originalTranscript));
  sections.push("");
  sections.push("</details>");

  if (corrections.length > 0) {
    sections.push("");
    sections.push("### Speaker Corrections");
    sections.push("");
    sections.push("```json");
    sections.push(JSON.stringify(corrections, null, 2));
    sections.push("```");
  }

  return sections.join("\n");
}

function getSpeakerLabelFromId(speakerId: string): string {
  return speakerId.replace(/^speaker-/, "").replace(/-/g, " ");
}

function escapeMarkdownContent(text: string): string {
  return text
    .replace(/\\/g, "\\\\")
    .replace(/\*/g, "\\*")
    .replace(/_/g, "\\_")
    .replace(/\[/g, "\\[")
    .replace(/\]/g, "\\]")
    .replace(/`/g, "\\`");
}
