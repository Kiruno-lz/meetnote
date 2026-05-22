import { promises as fs } from "fs";
import { join, resolve } from "path";
import type { MarkdownFrontmatter } from "./markdown-library-schema";
import { parseMarkdownFrontmatter, MARKDOWN_LIBRARY_SCHEMA_VERSION } from "./markdown-library-schema";

export interface SaveMeetingMarkdownOptions {
  readonly basePath: string;
  readonly markdown: string;
  readonly overwrite?: boolean;
}

export interface SaveResult {
  readonly success: boolean;
  readonly error?: string;
  readonly errorCode?: string;
  readonly savedPath?: string;
  readonly indexEntry?: MeetingSummary;
}

export interface MeetingSummary {
  readonly title: string;
  readonly date: string;
  readonly audioHash: string;
  readonly speakers: readonly string[];
  readonly generatedAt: string;
  readonly path: string;
  readonly slug: string;
}

export interface LibraryIndex {
  readonly version: string;
  readonly baseDir: string;
  readonly indexedAt: string;
  readonly meetings: readonly MeetingSummary[];
}

export interface SearchQuery {
  readonly dateRange?: { readonly startDate: string; readonly endDate: string };
  readonly speakers?: readonly string[];
  readonly keywords?: readonly string[];
  readonly title?: string;
}

// Core Functions

export async function saveMeetingMarkdown(options: SaveMeetingMarkdownOptions): Promise<SaveResult> {
  try {
    // Parse markdown to extract frontmatter
    const parseResult = parseMarkdownFrontmatter(options.markdown);
    if (!parseResult.ok || !parseResult.frontmatter) {
      return {
        success: false,
        error: `Invalid markdown: ${parseResult.errors.join("; ")}`,
        errorCode: "INVALID_MARKDOWN"
      };
    }

    const frontmatter = parseResult.frontmatter;
    const dateYear = frontmatter.date.slice(0, 4);
    const dateMonth = frontmatter.date.slice(5, 7);

    // Build the directory structure: meetings/YYYY/MM/<slug>/
    // Start with basic slug, then check for collisions and add hash suffix if needed
    let slug = generateSlug(frontmatter.title, frontmatter.date);
    let relativePath = `meetings/${dateYear}/${dateMonth}/${slug}`;

    // Validate path to prevent traversal attacks
    if (!validatePath(options.basePath, relativePath)) {
      return {
        success: false,
        error: "Path traversal detected",
        errorCode: "PATH_TRAVERSAL"
      };
    }

    let fullPath = resolve(options.basePath, relativePath);
    let meetingFilePath = join(fullPath, "meeting.md");
    const indexFilePath = join(options.basePath, "index.json");

    // Check for duplicate (same audioHash + generatedAt)
    if (!options.overwrite) {
      const dupKey = deduplicationKey(frontmatter);
      try {
        const existingIndex = await loadIndex(indexFilePath);
        for (const meeting of existingIndex.meetings) {
          const existingKey = `${meeting.audioHash}|${meeting.generatedAt}`;
          if (existingKey === dupKey) {
            return {
              success: false,
              error: `Duplicate import: audioHash="${frontmatter.audioHash}" and generatedAt="${frontmatter.generatedAt}" already exists`,
              errorCode: "DUPLICATE_IMPORT"
            };
          }
        }
      } catch {
        // Index file doesn't exist yet, continue
      }
    }

    // Check if slug directory already exists with different content
    let fullPathCheck = resolve(options.basePath, relativePath);
    try {
      const stat = await fs.stat(fullPathCheck);
      if (stat.isDirectory()) {
        // Directory exists, need to handle collision
        if (!options.overwrite) {
          // Add hash suffix to avoid collision
          const hashSuffix = generateHashSuffix(frontmatter.audioHash);
          slug = `${slug}-${hashSuffix}`;
          relativePath = `meetings/${dateYear}/${dateMonth}/${slug}`;

          if (!validatePath(options.basePath, relativePath)) {
            return {
              success: false,
              error: "Path traversal detected after slug collision handling",
              errorCode: "PATH_TRAVERSAL"
            };
          }

          // Update the paths with the new slug
          fullPath = resolve(options.basePath, relativePath);
          meetingFilePath = join(fullPath, "meeting.md");
        }
      }
    } catch {
      // Directory doesn't exist, continue
    }

    // Create directory structure
    try {
      await fs.mkdir(fullPath, { recursive: true });
    } catch (err) {
      if ((err as NodeJS.ErrnoException).code === "EACCES") {
        return {
          success: false,
          error: "Permission denied creating directory",
          errorCode: "PERMISSION_DENIED"
        };
      }
      throw err;
    }

    // Write markdown file
    try {
      await fs.writeFile(meetingFilePath, options.markdown, "utf-8");
    } catch (err) {
      if ((err as NodeJS.ErrnoException).code === "EACCES") {
        return {
          success: false,
          error: "Permission denied writing file",
          errorCode: "PERMISSION_DENIED"
        };
      }
      throw err;
    }

    // Create the index entry
    const indexEntry: MeetingSummary = {
      title: frontmatter.title,
      date: frontmatter.date,
      audioHash: frontmatter.audioHash,
      speakers: frontmatter.speakers,
      generatedAt: frontmatter.generatedAt,
      path: relativePath,
      slug
    };

    // Update index.json
    try {
      const index = await rebuildLibraryIndex(options.basePath);
      // rebuildLibraryIndex already saves the index, so we just return success
    } catch (err) {
      // Index update failed, but file was saved
      console.warn("Failed to update index:", err);
    }

    return {
      success: true,
      savedPath: relativePath,
      indexEntry
    };
  } catch (err) {
    return {
      success: false,
      error: `Unexpected error: ${err instanceof Error ? err.message : String(err)}`,
      errorCode: "IO_ERROR"
    };
  }
}

export async function rebuildLibraryIndex(
  baseDir: string
): Promise<{ success: boolean; index: LibraryIndex; errors: readonly string[] }> {
  const errors: string[] = [];
  const meetingMap = new Map<string, MeetingSummary>();

  try {
    // Recursively scan meetings directory
    const meetings = await scanMeetingsDirectory(baseDir, meetingMap, errors);

    // Remove duplicates: keep the first one by (audioHash, generatedAt)
    const seenDupKeys = new Set<string>();
    const uniqueMeetings: MeetingSummary[] = [];

    for (const meeting of meetings) {
      const dupKey = `${meeting.audioHash}|${meeting.generatedAt}`;
      if (!seenDupKeys.has(dupKey)) {
        seenDupKeys.add(dupKey);
        uniqueMeetings.push(meeting);
      } else {
        errors.push(`Duplicate meeting found: ${meeting.slug} (audioHash="${meeting.audioHash}", generatedAt="${meeting.generatedAt}")`);
      }
    }

    // Sort by date descending (newest first)
    uniqueMeetings.sort((a, b) => new Date(b.date).getTime() - new Date(a.date).getTime());

    const index: LibraryIndex = {
      version: MARKDOWN_LIBRARY_SCHEMA_VERSION,
      baseDir,
      indexedAt: new Date().toISOString(),
      meetings: uniqueMeetings
    };

    // Save index.json
    const indexFilePath = join(baseDir, "index.json");
    await fs.mkdir(baseDir, { recursive: true });
    await fs.writeFile(indexFilePath, JSON.stringify(index, null, 2), "utf-8");

    return {
      success: errors.length === 0,
      index,
      errors
    };
  } catch (err) {
    const errorMsg = err instanceof Error ? err.message : String(err);
    return {
      success: false,
      index: {
        version: MARKDOWN_LIBRARY_SCHEMA_VERSION,
        baseDir,
        indexedAt: new Date().toISOString(),
        meetings: []
      },
      errors: [errorMsg]
    };
  }
}

export async function searchMeetings(baseDir: string, query: SearchQuery): Promise<MeetingSummary[]> {
  try {
    const indexFilePath = join(baseDir, "index.json");
    const index = await loadIndex(indexFilePath);

    let results = [...index.meetings];

    // Apply date range filter
    if (query.dateRange) {
      const { startDate, endDate } = query.dateRange;
      results = results.filter((m) => {
        const date = m.date;
        return date >= startDate && date <= endDate;
      });
    }

    // Apply speaker filter (any of the speakers)
    if (query.speakers && query.speakers.length > 0) {
      const speakersLower = query.speakers.map((s) => s.toLowerCase());
      results = results.filter((m) =>
        m.speakers.some((speaker) => speakersLower.includes(speaker.toLowerCase()))
      );
    }

    // Apply title filter (prefix match, case-insensitive)
    if (query.title) {
      const titleLower = query.title.toLowerCase();
      results = results.filter((m) => m.title.toLowerCase().startsWith(titleLower));
    }

    // Apply keywords filter (fulltext search)
    if (query.keywords && query.keywords.length > 0) {
      const keywordsLower = query.keywords.map((k) => k.toLowerCase());

      // Load meeting files and search content
      const filtered: MeetingSummary[] = [];
      for (const meeting of results) {
        try {
          const meetingPath = join(baseDir, meeting.path, "meeting.md");
          const content = await fs.readFile(meetingPath, "utf-8");
          const contentLower = content.toLowerCase();

          // Check if all keywords are present
          const allKeywordsFound = keywordsLower.every((keyword) => contentLower.includes(keyword));
          if (allKeywordsFound) {
            filtered.push(meeting);
          }
        } catch {
          // File not found or can't be read, skip it
        }
      }
      results = filtered;
    }

    // Results are already sorted by date DESC from index
    return results;
  } catch {
    return [];
  }
}

// Helper Functions (internal)

function generateSlug(title: string, date: string): string {
  // Extract YYYY-MM-DD from date
  const datePrefix = date.slice(0, 10);

  // Slugify title
  const slugTitle = title
    .trim()
    .toLowerCase()
    // Allow CJK, alphanumeric, and hyphens
    .replace(/[^\p{L}\p{N}\-一-龥]+/gu, "-")
    // Remove leading/trailing hyphens
    .replace(/^-+|-+$/g, "");

  const finalSlug = slugTitle || "untitled";
  return `${datePrefix}-${finalSlug}`;
}

function generateHashSuffix(audioHash: string): string {
  // Generate a short hash suffix from audioHash for collision handling
  // Take first 8 characters of the hash
  return audioHash.slice(0, 8);
}

function validatePath(baseDir: string, relativePath: string): boolean {
  // Prevent path traversal
  if (relativePath.includes("..") || relativePath.includes("~") || relativePath.startsWith("/")) {
    return false;
  }

  const fullPath = resolve(baseDir, relativePath);
  const baseDirResolved = resolve(baseDir);

  // Check that the resolved path starts with the base directory
  return fullPath.startsWith(baseDirResolved + "/") || fullPath === baseDirResolved;
}

function deduplicationKey(frontmatter: MarkdownFrontmatter): string {
  return `${frontmatter.audioHash}|${frontmatter.generatedAt}`;
}

async function loadIndex(indexFilePath: string): Promise<LibraryIndex> {
  const content = await fs.readFile(indexFilePath, "utf-8");
  const data = JSON.parse(content) as LibraryIndex;
  return data;
}

async function scanMeetingsDirectory(
  baseDir: string,
  meetingMap: Map<string, MeetingSummary>,
  errors: string[]
): Promise<MeetingSummary[]> {
  const meetings: MeetingSummary[] = [];

  try {
    const meetingsDir = join(baseDir, "meetings");
    await fs.access(meetingsDir);

    // Recursively scan for meeting.md files
    await scanDirRecursive(meetingsDir, baseDir, meetings, errors);
  } catch {
    // meetings directory doesn't exist yet
  }

  return meetings;
}

async function scanDirRecursive(
  currentDir: string,
  baseDir: string,
  meetings: MeetingSummary[],
  errors: string[]
): Promise<void> {
  try {
    const entries = await fs.readdir(currentDir, { withFileTypes: true });

    for (const entry of entries) {
      const fullPath = join(currentDir, entry.name);

      if (entry.isDirectory()) {
        await scanDirRecursive(fullPath, baseDir, meetings, errors);
      } else if (entry.name === "meeting.md") {
        try {
          const content = await fs.readFile(fullPath, "utf-8");
          const parseResult = parseMarkdownFrontmatter(content);

          if (parseResult.ok && parseResult.frontmatter) {
            const frontmatter = parseResult.frontmatter;
            const relativePath = fullPath.slice(baseDir.length + 1); // Remove baseDir + /
            const pathParts = relativePath.split("/");

            // Extract slug from path: meetings/YYYY/MM/<slug>/meeting.md
            const slug = pathParts.length >= 4 ? pathParts[3] : "unknown";

            const summary: MeetingSummary = {
              title: frontmatter.title,
              date: frontmatter.date,
              audioHash: frontmatter.audioHash,
              speakers: frontmatter.speakers,
              generatedAt: frontmatter.generatedAt,
              path: `meetings/${pathParts[1]}/${pathParts[2]}/${slug}`,
              slug: slug || "unknown"
            };

            meetings.push(summary);
          } else {
            errors.push(`Failed to parse ${fullPath}: ${parseResult.errors.join("; ")}`);
          }
        } catch (err) {
          errors.push(`Error reading ${fullPath}: ${err instanceof Error ? err.message : String(err)}`);
        }
      }
    }
  } catch (err) {
    errors.push(`Error scanning directory ${currentDir}: ${err instanceof Error ? err.message : String(err)}`);
  }
}
