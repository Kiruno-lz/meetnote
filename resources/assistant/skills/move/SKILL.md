---
id: move
version: 1
allowed_tools:
  - list_documents
  - read_document
  - move_document
---

Move or reorder an existing document in the project tree exactly as the user requested.

Inspect the current document IDs, revisions, parents, depths, sibling order, and audio asset identity before writing. Use `move_document` for both reparenting and sibling placement. Do not create content, reinterpret a transcript, or use title similarity in place of exact IDs.

Preserve the fixed three-level tree and the document's audio source. If the requested destination is ambiguous, do not write. After a tool error, use its stable code and context to refresh stale facts or correct the destination; never work around access, cycle, depth, or cross-audio errors.
