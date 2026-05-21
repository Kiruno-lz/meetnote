// ——— Pure functions (L1 testable) ———

/**
 * Parsed parts of a Markdown document: optional YAML frontmatter and body.
 */
export interface MarkdownParts {
  /** The complete frontmatter block including both --- delimiters, or "" if absent. */
  frontmatter: string;
  /** The body text after the frontmatter (leading blank lines stripped), or the full text if no frontmatter. */
  body: string;
}

/**
 * Parse a raw Markdown string into frontmatter and body parts.
 *
 * Frontmatter format: a block that starts with "---\n" at the very beginning
 * of the string and closes with a "\n---\n" or "\n---" at the end.
 * If no valid frontmatter is found, frontmatter is "" and body is the full text.
 */
export function parseMarkdownParts(raw: string): MarkdownParts {
  if (!raw.startsWith("---\n") && raw !== "---") {
    return { frontmatter: "", body: raw };
  }

  // Find the closing --- after the opening one.
  // The closing delimiter is a line that is exactly "---".
  const afterOpen = raw.slice(4); // skip the leading "---\n"
  const closingIndex = findClosingFrontmatter(afterOpen);
  if (closingIndex === -1) {
    // No closing delimiter found — treat as plain body
    return { frontmatter: "", body: raw };
  }

  // closingIndex points to the start of "---" in afterOpen
  const frontmatterContent = afterOpen.slice(0, closingIndex);
  const frontmatter = `---\n${frontmatterContent}---`;

  // Everything after the closing "---"
  const afterClose = afterOpen.slice(closingIndex + 3); // skip "---"
  // Strip one leading newline (the one right after the closing ---)
  const bodyRaw = afterClose.startsWith("\n") ? afterClose.slice(1) : afterClose;
  // Strip additional leading blank lines
  const body = bodyRaw.replace(/^\n+/, "");

  return { frontmatter, body };
}

/** Find the index of the closing "---" line within the text after the opening delimiter. */
function findClosingFrontmatter(text: string): number {
  let searchStart = 0;
  while (searchStart < text.length) {
    const idx = text.indexOf("---", searchStart);
    if (idx === -1) return -1;
    // Must be at start of line (idx === 0 or preceded by \n)
    const atLineStart = idx === 0 || text[idx - 1] === "\n";
    // Must be followed by \n or end of string
    const afterDashes = text[idx + 3];
    const atLineEnd = afterDashes === "\n" || afterDashes === undefined;
    if (atLineStart && atLineEnd) {
      return idx;
    }
    searchStart = idx + 3;
  }
  return -1;
}

/**
 * Reassemble a Markdown document from its parts.
 *
 * If frontmatter is "", returns only body.
 * Otherwise returns frontmatter + "\n" + body.
 */
export function reassembleMarkdown(parts: MarkdownParts): string {
  if (parts.frontmatter === "") {
    return parts.body;
  }
  return `${parts.frontmatter}\n${parts.body}`;
}

/**
 * Detect whether the body content has changed (trim-compared).
 */
export function isMarkdownBodyChanged(original: string, current: string): boolean {
  return original.trim() !== current.trim();
}

/**
 * Split content into display lines (split on "\n").
 * An empty string yields [""] (one empty line).
 */
export function splitMarkdownLines(content: string): string[] {
  return content.split("\n");
}

// ——— DOM component (UI layer — not L1 tested) ———

export interface MarkdownEditorOptions {
  initialContent: string;
  readonly?: boolean;
  onchange?: (newContent: string) => void;
}

/**
 * Create a Markdown editor/preview HTMLElement.
 *
 * - readonly === false (default): returns a <textarea class="markdown-editor-textarea">
 * - readonly === true: returns a <div class="markdown-preview"> with one <div class="md-line"> per line
 */
export function createMarkdownEditor(options: MarkdownEditorOptions): HTMLElement {
  const { initialContent, readonly = false, onchange } = options;

  if (readonly) {
    const container = document.createElement("div");
    container.className = "markdown-preview";
    const lines = splitMarkdownLines(initialContent);
    for (const line of lines) {
      const div = document.createElement("div");
      div.className = "md-line";
      div.textContent = line;
      container.appendChild(div);
    }
    return container;
  }

  const textarea = document.createElement("textarea");
  textarea.className = "markdown-editor-textarea";
  textarea.value = initialContent;

  if (onchange) {
    textarea.addEventListener("input", () => {
      onchange(textarea.value);
    });
  }

  return textarea;
}
