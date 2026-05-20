import { createMeetingProjectManifest, type MeetingProject } from "./audio-project";

export interface LibraryFile {
  readonly path: string;
  readonly contents: string;
}

export interface MarkdownLibraryRecord {
  readonly root: string;
  readonly projectId: string;
  readonly files: readonly LibraryFile[];
}

export function createMarkdownLibraryRecord(project: MeetingProject): MarkdownLibraryRecord {
  const root = `meetings/${project.createdAt.slice(0, 4)}/${project.createdAt.slice(5, 7)}/${slugify(project.title)}`;
  const manifest = {
    ...createMeetingProjectManifest(project)
  };

  return {
    root,
    projectId: project.id,
    files: [
      {
        path: `${root}/manifest.json`,
        contents: `${JSON.stringify(manifest, null, 2)}\n`
      },
      {
        path: `${root}/meeting.md`,
        contents: renderMeetingMarkdown(project)
      }
    ]
  };
}

export function rebuildLibraryIndex(records: readonly MarkdownLibraryRecord[]): { readonly projectIds: readonly string[] } {
  return {
    projectIds: records.map((record) => record.projectId).sort()
  };
}

function renderMeetingMarkdown(project: MeetingProject): string {
  return [
    "---",
    `title: ${escapeYaml(project.title)}`,
    `audio_hash: ${project.audioHash}`,
    `audio_format: ${project.format}`,
    `created_at: ${project.createdAt}`,
    "schema: meetnote.markdown.v1",
    "---",
    "",
    `# ${project.title}`,
    "",
    project.editedMarkdown ?? "_No edited meeting text yet._",
    ""
  ].join("\n");
}

function slugify(value: string): string {
  const slug = value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9\u4e00-\u9fa5]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return slug || "untitled-meeting";
}

function escapeYaml(value: string): string {
  return JSON.stringify(value);
}
