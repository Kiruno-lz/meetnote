#![allow(clippy::result_large_err)]

use crate::{
    DocumentMetadata, DocumentMutationBatch, DocumentMutationCommit, DocumentMutationStoreError,
    DocumentMutationWrite, DocumentService, PendingDocumentMutation,
};
use contracts::{
    schema_for_arguments, AgentActor, AgentDocumentPatch, AgentDocumentProjection,
    AgentDocumentRevision, AgentDocumentSummary, AgentErrorCode, AgentFieldDiff,
    AgentMutationProvenance, AgentMutationReceipt, AgentPendingMutation, AgentRangePreview,
    AgentScopeSelector, AgentSourceRevision, AgentSpeakerSegment, AgentToolDefinition,
    AgentToolError, AgentToolResult, AgentTranscriptSegment, AgentTreeChange, AgentWriteMode,
    CapabilityRequest, CapabilityResponse, CreateDocumentArguments, CreateDocumentSource,
    CreateSemanticChildrenArguments, DocumentProjection, ListDocumentsArguments,
    MoveDocumentArguments, MovePlacement, ReadDocumentArguments, SemanticSplitMode,
    UpdateDocumentArguments, WorkspaceCapabilityDescription, AGENT_TOOL_SCHEMA_VERSION,
    WORKSPACE_CAPABILITY_ID, WORKSPACE_CAPABILITY_VERSION,
};
use domain::{reparent_project, DocumentId, DocumentState, ProjectTreeError};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use uuid::Uuid;

pub const LIST_DOCUMENTS_TOOL: &str = "list_documents";
pub const READ_DOCUMENT_TOOL: &str = "read_document";
pub const UPDATE_DOCUMENT_TOOL: &str = "update_document";
pub const CREATE_DOCUMENT_TOOL: &str = "create_document";
pub const CREATE_SEMANTIC_CHILDREN_TOOL: &str = "create_semantic_children";
pub const MOVE_DOCUMENT_TOOL: &str = "move_document";
pub const CONFIRM_CONTROL: &str = "confirm_mutation";
pub const REJECT_CONTROL: &str = "reject_mutation";
pub const UNDO_CONTROL: &str = "undo_mutation";
/// Hard upper bound for a single capability result on the typed wire.
/// Segment pagination remains available for large transcript projections.
pub const MAX_CAPABILITY_RESULT_BYTES: usize = 512 * 1024;

/// Projects Workspace results before they cross the capability boundary.
///
/// The port is shared by Built-in, CLI, and external capability consumers, so
/// sensitive local locators and secret-bearing error context must be removed
/// here rather than only while preparing a particular model request.
#[derive(Debug, Clone, Copy, Default)]
pub struct CapabilityProjectionPolicy;

impl CapabilityProjectionPolicy {
    pub fn project_response(mut response: CapabilityResponse) -> CapabilityResponse {
        if let Some(result) = response.result.as_mut() {
            Self::project_result(result);
        }
        if let Some(error) = response.error.as_mut() {
            redact_error(error);
        }
        response
    }

    pub fn project_result(result: &mut AgentToolResult) {
        match result {
            AgentToolResult::Documents { documents } => {
                for document in documents {
                    redact_local_locator(&mut document.audio_asset_id);
                }
            }
            AgentToolResult::Document(document) => {
                redact_document_summary(&mut document.metadata);
                if let Some(markdown) = document.markdown.as_mut() {
                    redact_audio_locator_markdown(markdown);
                }
            }
            AgentToolResult::Mutation(_) => {}
            AgentToolResult::Pending(pending) => {
                for diff in &mut pending.field_diffs {
                    if diff.field == "markdown" {
                        if let Some(before) = diff.before.as_mut() {
                            redact_audio_locator_markdown(before);
                        }
                        if let Some(after) = diff.after.as_mut() {
                            redact_audio_locator_markdown(after);
                        }
                    }
                }
            }
        }
    }
}

fn redact_document_summary(summary: &mut AgentDocumentSummary) {
    redact_local_locator(&mut summary.audio_asset_id);
}

fn redact_audio_locator_markdown(markdown: &mut String) {
    *markdown = redact_audio_locator_markdown_owned(markdown);
}

fn redact_audio_locator_markdown_owned(markdown: &str) -> String {
    let mut result = String::new();
    for (index, line) in markdown.lines().enumerate() {
        if index > 0 {
            result.push('\n');
        }
        if let Some(locator) = line
            .strip_prefix("Audio: `")
            .and_then(|value| value.strip_suffix('`'))
        {
            if locator.starts_with('/') || locator.contains(':') || locator.starts_with('~') {
                result.push_str("Audio: `[local audio asset]`");
                continue;
            }
        }
        result.push_str(line);
    }
    result
}

fn redact_local_locator(value: &mut String) {
    let bytes = value.as_bytes();
    let windows_absolute = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\');
    if value.starts_with('/') || value.starts_with("~/") || windows_absolute {
        *value = "[redacted-local-path]".into();
    }
}

fn redact_local_locator_owned(value: &str) -> String {
    let mut result = value.to_string();
    redact_local_locator(&mut result);
    result
}

fn redact_error(error: &mut AgentToolError) {
    error.message = redact_local_locator_text(&error.message);
    for (key, value) in error.context.iter_mut() {
        if is_secret_context_key(key) {
            *value = serde_json::Value::String("[redacted-secret]".into());
        } else {
            redact_json_locators(value);
        }
    }
}

fn redact_json_locators(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(value) => *value = redact_local_locator_text(value),
        serde_json::Value::Array(values) => {
            for value in values {
                redact_json_locators(value);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values_mut() {
                redact_json_locators(value);
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

fn is_secret_context_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("secret")
        || key.contains("api_key")
        || key.contains("apikey")
        || key.contains("password")
        || key.contains("authorization")
        || key.ends_with("token")
}

fn redact_local_locator_text(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut result = String::with_capacity(text.len());
    let mut index = 0;
    while index < bytes.len() {
        let windows_absolute = bytes.len().saturating_sub(index) >= 3
            && bytes[index].is_ascii_alphabetic()
            && bytes[index + 1] == b':'
            && matches!(bytes[index + 2], b'/' | b'\\');
        let path_boundary = |position: usize| {
            position == 0
                || bytes[position - 1].is_ascii_whitespace()
                || matches!(
                    bytes[position - 1],
                    b'=' | b'(' | b'[' | b'{' | b'"' | b'\'' | b'`'
                )
        };
        let posix_absolute = bytes[index] == b'/' && path_boundary(index);
        let home_absolute =
            bytes[index] == b'~' && bytes.get(index + 1) == Some(&b'/') && path_boundary(index);
        if windows_absolute || posix_absolute || home_absolute {
            let mut end = index;
            while end < bytes.len()
                && !bytes[end].is_ascii_whitespace()
                && !matches!(bytes[end], b'"' | b'\'' | b'`' | b')' | b']' | b'}' | b',')
            {
                end += 1;
            }
            result.push_str("[redacted-local-path]");
            index = end;
        } else {
            let character = text[index..]
                .chars()
                .next()
                .expect("index is inside UTF-8 text");
            result.push(character);
            index += character.len_utf8();
        }
    }
    result
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceAccessPolicy {
    AllDocuments,
    Documents(BTreeSet<DocumentId>),
}

impl WorkspaceAccessPolicy {
    pub fn documents(document_ids: impl IntoIterator<Item = DocumentId>) -> Self {
        Self::Documents(document_ids.into_iter().collect())
    }

    fn grants(&self, document_id: &DocumentId) -> bool {
        match self {
            Self::AllDocuments => true,
            Self::Documents(document_ids) => document_ids.contains(document_id),
        }
    }

    fn describe(&self) -> AgentScopeSelector {
        AgentScopeSelector {
            document_ids: match self {
                Self::AllDocuments => Vec::new(),
                Self::Documents(document_ids) => document_ids
                    .iter()
                    .map(|document_id| document_id.as_str().to_string())
                    .collect(),
            },
            all_documents: matches!(self, Self::AllDocuments),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ToolRegistry;

impl ToolRegistry {
    pub fn definitions(&self) -> Vec<AgentToolDefinition> {
        vec![
            AgentToolDefinition {
                name: LIST_DOCUMENTS_TOOL.into(),
                description: "List metadata for documents within the granted scope.".into(),
                side_effect: false,
                input_schema: schema_for_arguments::<ListDocumentsArguments>(),
            },
            AgentToolDefinition {
                name: READ_DOCUMENT_TOOL.into(),
                description: "Read an authorized document through a fixed projection.".into(),
                side_effect: false,
                input_schema: schema_for_arguments::<ReadDocumentArguments>(),
            },
            AgentToolDefinition {
                name: UPDATE_DOCUMENT_TOOL.into(),
                description: "Update only Agent-writable semantic document fields.".into(),
                side_effect: true,
                input_schema: schema_for_arguments::<UpdateDocumentArguments>(),
            },
            AgentToolDefinition {
                name: CREATE_DOCUMENT_TOOL.into(),
                description:
                    "Create a normal document from an authorized audio asset or explicit no-audio source."
                        .into(),
                side_effect: true,
                input_schema: schema_for_arguments::<CreateDocumentArguments>(),
            },
            AgentToolDefinition {
                name: CREATE_SEMANTIC_CHILDREN_TOOL.into(),
                description:
                    "Create a validated batch of continuous semantic children from segment IDs."
                        .into(),
                side_effect: true,
                input_schema: schema_for_arguments::<CreateSemanticChildrenArguments>(),
            },
            AgentToolDefinition {
                name: MOVE_DOCUMENT_TOOL.into(),
                description: "Move a document and set its position among the new siblings.".into(),
                side_effect: true,
                input_schema: schema_for_arguments::<MoveDocumentArguments>(),
            },
        ]
    }

    fn definition(&self, name: &str) -> Option<AgentToolDefinition> {
        self.definitions()
            .into_iter()
            .find(|definition| definition.name == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentLlmProvenance {
    connection_id: String,
    protocol: String,
    requested_model: String,
    resolved_model: String,
}

#[derive(Clone)]
pub struct WorkspaceCapabilityPort {
    documents: DocumentService,
    tools: ToolRegistry,
    granted_tools: BTreeSet<String>,
    granted_controls: BTreeSet<String>,
    access: WorkspaceAccessPolicy,
    actor: AgentActor,
    write_mode: AgentWriteMode,
    confirmation_ttl_ms: u64,
    llm_provenance: Option<AgentLlmProvenance>,
}

impl WorkspaceCapabilityPort {
    pub fn new(documents: DocumentService, access: WorkspaceAccessPolicy) -> Self {
        Self {
            documents,
            tools: ToolRegistry,
            granted_tools: ToolRegistry
                .definitions()
                .into_iter()
                .map(|tool| tool.name)
                .collect(),
            granted_controls: [CONFIRM_CONTROL, REJECT_CONTROL, UNDO_CONTROL]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            access,
            actor: AgentActor::Assistant,
            write_mode: AgentWriteMode::Automatic,
            confirmation_ttl_ms: 15 * 60 * 1000,
            llm_provenance: None,
        }
    }

    /// Restrict an already granted port; consumers cannot broaden it.
    pub fn restrict_tools(mut self, names: impl IntoIterator<Item = String>) -> Self {
        let requested: BTreeSet<String> = names.into_iter().collect();
        self.granted_tools.retain(|name| requested.contains(name));
        self
    }

    pub fn restrict_controls(mut self, names: impl IntoIterator<Item = String>) -> Self {
        let requested: BTreeSet<String> = names.into_iter().collect();
        self.granted_controls
            .retain(|name| requested.contains(name));
        self
    }

    pub fn read_only(self) -> Self {
        self.restrict_tools([LIST_DOCUMENTS_TOOL.into(), READ_DOCUMENT_TOOL.into()])
            .restrict_controls(std::iter::empty())
    }

    pub fn with_write_mode(mut self, write_mode: AgentWriteMode) -> Self {
        self.write_mode = write_mode;
        self
    }

    pub fn with_actor(mut self, actor: AgentActor) -> Self {
        self.actor = actor;
        self
    }

    pub fn with_confirmation_ttl_ms(mut self, confirmation_ttl_ms: u64) -> Self {
        self.confirmation_ttl_ms = confirmation_ttl_ms;
        self
    }

    pub fn with_llm_provenance(
        mut self,
        connection_id: impl Into<String>,
        protocol: impl Into<String>,
        requested_model: impl Into<String>,
        resolved_model: impl Into<String>,
    ) -> Self {
        self.llm_provenance = Some(AgentLlmProvenance {
            connection_id: connection_id.into(),
            protocol: protocol.into(),
            requested_model: requested_model.into(),
            resolved_model: resolved_model.into(),
        });
        self
    }

    pub fn describe(&self) -> WorkspaceCapabilityDescription {
        let description = WorkspaceCapabilityDescription {
            schema_version: AGENT_TOOL_SCHEMA_VERSION,
            capability_id: WORKSPACE_CAPABILITY_ID.into(),
            capability_version: WORKSPACE_CAPABILITY_VERSION,
            write_mode: self.write_mode,
            authorization: self.access.describe(),
            tools: self
                .tools
                .definitions()
                .into_iter()
                .filter(|tool| self.granted_tools.contains(&tool.name))
                .collect(),
        };
        debug_assert!(serialized_json_size(&description) <= MAX_CAPABILITY_RESULT_BYTES);
        description
    }

    pub fn execute(&self, tool_name: &str, request: CapabilityRequest) -> CapabilityResponse {
        let request_id = request.request_id.clone();
        let response = match self.execute_inner(tool_name, &request) {
            Ok(result) => CapabilityResponse::success(request_id, result),
            Err(error) => CapabilityResponse::failure(
                request_id,
                error.with_tool_call(request.tool_call_id.clone()),
            ),
        };
        CapabilityProjectionPolicy::project_response(response)
    }

    fn execute_inner(
        &self,
        tool_name: &str,
        request: &CapabilityRequest,
    ) -> Result<AgentToolResult, AgentToolError> {
        if request.schema_version != AGENT_TOOL_SCHEMA_VERSION {
            return Err(invalid_arguments(
                "schemaVersion",
                format!(
                    "unsupported agent tool schema {}, expected {}",
                    request.schema_version, AGENT_TOOL_SCHEMA_VERSION
                ),
            ));
        }
        if request.request_id.trim().is_empty() {
            return Err(invalid_arguments("requestId", "requestId cannot be empty"));
        }
        if request.tool_call_id.trim().is_empty() {
            return Err(invalid_arguments(
                "toolCallId",
                "toolCallId cannot be empty",
            ));
        }
        if request.scope.all_documents && !request.scope.document_ids.is_empty() {
            return Err(invalid_arguments(
                "scope",
                "allDocuments cannot be combined with documentIds",
            ));
        }
        if request
            .scope
            .document_ids
            .iter()
            .any(|document_id| document_id.trim().is_empty())
        {
            return Err(invalid_arguments(
                "scope.documentIds",
                "scope document IDs cannot be empty",
            ));
        }
        let _definition = self.tools.definition(tool_name).ok_or_else(|| {
            AgentToolError::new(
                AgentErrorCode::ToolUnknown,
                format!("unknown agent tool: {tool_name}"),
                false,
            )
        })?;
        if !self.granted_tools.contains(tool_name) {
            return Err(AgentToolError::new(
                AgentErrorCode::ToolNotAllowed,
                "capability was not granted",
                false,
            ));
        }
        match tool_name {
            LIST_DOCUMENTS_TOOL => {
                let _: ListDocumentsArguments = decode_arguments(&request.arguments)?;
                self.list_documents(&request.scope)
            }
            READ_DOCUMENT_TOOL => {
                let arguments: ReadDocumentArguments = decode_arguments(&request.arguments)?;
                validate_document_id_argument(&arguments.document_id, "documentId")?;
                self.read_document(&request.scope, arguments)
            }
            UPDATE_DOCUMENT_TOOL => {
                self.reject_read_only_fields(&request.arguments)?;
                let arguments: UpdateDocumentArguments = decode_arguments(&request.arguments)?;
                validate_document_id_argument(&arguments.document_id, "documentId")?;
                self.update_document(request, arguments)
            }
            CREATE_DOCUMENT_TOOL => {
                let arguments: CreateDocumentArguments = decode_arguments(&request.arguments)?;
                validate_create_document_arguments(&arguments)?;
                if let Some(parent_id) = arguments.parent_document_id.as_deref() {
                    validate_document_id_argument(parent_id, "parentDocumentId")?;
                }
                self.create_document(request, arguments)
            }
            CREATE_SEMANTIC_CHILDREN_TOOL => {
                let arguments: CreateSemanticChildrenArguments =
                    decode_arguments(&request.arguments)?;
                validate_document_id_argument(&arguments.parent_document_id, "parentDocumentId")?;
                if arguments.children.is_empty() {
                    return Err(invalid_arguments(
                        "children",
                        "semantic split must contain at least one child",
                    ));
                }
                self.create_semantic_children(request, arguments)
            }
            MOVE_DOCUMENT_TOOL => {
                let arguments: MoveDocumentArguments = decode_arguments(&request.arguments)?;
                validate_document_id_argument(&arguments.document_id, "documentId")?;
                if let Some(parent_id) = arguments.new_parent_document_id.as_deref() {
                    validate_document_id_argument(parent_id, "newParentDocumentId")?;
                }
                if let Some(target_id) = arguments.target_sibling_document_id.as_deref() {
                    validate_document_id_argument(target_id, "targetSiblingDocumentId")?;
                }
                self.move_document(request, arguments)
            }
            _ => unreachable!("tool registry and dispatcher must remain exhaustive"),
        }
    }

    fn list_documents(
        &self,
        scope: &AgentScopeSelector,
    ) -> Result<AgentToolResult, AgentToolError> {
        let mut documents = Vec::new();
        self.documents
            .scan_metadata(&mut |metadata| {
                if !self.is_in_effective_scope(&metadata.document_id, scope) {
                    return Ok(true);
                }
                documents.push(summarize_metadata(&metadata));
                let result = AgentToolResult::Documents {
                    documents: documents.clone(),
                };
                enforce_result_size(&result, None)
                    .map(|_| true)
                    .map_err(|error| {
                        // Returning the error from the visitor stops the repository scan before
                        // another metadata row is loaded, keeping peak list memory bounded.
                        serde_json::to_string(&error).expect("capability errors must serialize")
                    })
            })
            .map_err(|message| {
                if let Ok(error) = serde_json::from_str::<AgentToolError>(&message) {
                    error
                } else {
                    transaction_error(message)
                }
            })?;
        let result = AgentToolResult::Documents { documents };
        enforce_result_size(&result, None)?;
        Ok(result)
    }

    fn read_document(
        &self,
        scope: &AgentScopeSelector,
        arguments: ReadDocumentArguments,
    ) -> Result<AgentToolResult, AgentToolError> {
        let document_id = DocumentId::new(arguments.document_id);
        self.ensure_access(&document_id, scope)?;
        let record = self
            .documents
            .read(&document_id)
            .map_err(transaction_error)?
            .ok_or_else(|| {
                let mut error = AgentToolError::new(
                    AgentErrorCode::DocumentNotFound,
                    "document was not found",
                    true,
                );
                error.document_id = Some(document_id.as_str().into());
                error
            })?;
        let include_markdown = matches!(
            arguments.projection,
            DocumentProjection::Markdown | DocumentProjection::Complete
        );
        let include_transcript = matches!(
            arguments.projection,
            DocumentProjection::Transcript | DocumentProjection::Complete
        );
        let include_speakers = matches!(
            arguments.projection,
            DocumentProjection::SpeakerSegments | DocumentProjection::Complete
        );
        if let Some(page) = arguments.page {
            if page.limit == 0
                || page.limit > 256
                || !matches!(
                    arguments.projection,
                    DocumentProjection::Transcript | DocumentProjection::SpeakerSegments
                )
            {
                return Err(invalid_arguments(
                    "page",
                    "page requires transcript or speaker_segments and limit 1..=256",
                ));
            }
            let snapshot =
                document_projection_snapshot(&record, include_transcript, include_speakers)?;
            if (page.offset > 0 && page.snapshot.is_none())
                || page
                    .snapshot
                    .as_ref()
                    .is_some_and(|value| value != &snapshot)
            {
                return Err(document_error(
                    AgentErrorCode::DocumentRevisionConflict,
                    "document snapshot changed or is missing; restart pagination",
                    &document_id,
                    true,
                ));
            }
            let total = if include_transcript {
                record.state.results.transcript.len()
            } else {
                record.state.results.speaker_segments.len()
            };
            if page.offset > total {
                return Err(invalid_arguments(
                    "page.offset",
                    "offset exceeds segment count",
                ));
            }
            let offset = page.offset;
            let end = offset.saturating_add(page.limit).min(total);
            let page = contracts::DocumentPage {
                snapshot,
                total,
                next_offset: (end < total).then_some(end),
            };
            let estimated_size = document_result_size(
                &record,
                include_markdown,
                include_transcript,
                include_speakers,
                Some((&page, offset..end)),
            );
            enforce_result_size_bytes(estimated_size, Some(&document_id))?;
            let projection = AgentDocumentProjection {
                metadata: summarize_with_markdown(&record.state, &record.markdown),
                page: Some(page),
                markdown: None,
                transcript: include_transcript.then(|| {
                    record.state.results.transcript[offset..end]
                        .iter()
                        .map(|segment| AgentTranscriptSegment {
                            id: segment.id.clone(),
                            start_ms: segment.start_ms,
                            end_ms: segment.end_ms,
                            text: segment.text.clone(),
                        })
                        .collect()
                }),
                speaker_segments: include_speakers.then(|| {
                    record.state.results.speaker_segments[offset..end]
                        .iter()
                        .map(|segment| AgentSpeakerSegment {
                            transcript_segment_id: segment.transcript_segment_id.clone(),
                            speaker_id: segment.speaker_id.clone(),
                            speaker_profile_id: segment.speaker_profile_id.clone(),
                            speaker_label: segment.speaker_label.clone(),
                            start_ms: segment.start_ms,
                            end_ms: segment.end_ms,
                            text: segment.text.clone(),
                        })
                        .collect()
                }),
            };
            let result = AgentToolResult::Document(projection);
            enforce_result_size(&result, Some(&document_id))?;
            return Ok(result);
        }
        let estimated_size = document_result_size(
            &record,
            include_markdown,
            include_transcript,
            include_speakers,
            None,
        );
        enforce_result_size_bytes(estimated_size, Some(&document_id))?;
        let projection = AgentDocumentProjection {
            metadata: summarize_with_markdown(&record.state, &record.markdown),
            page: None,
            markdown: include_markdown
                .then(|| redact_audio_locator_markdown_owned(&record.markdown)),
            transcript: include_transcript.then(|| {
                record
                    .state
                    .results
                    .transcript
                    .iter()
                    .map(|segment| AgentTranscriptSegment {
                        id: segment.id.clone(),
                        start_ms: segment.start_ms,
                        end_ms: segment.end_ms,
                        text: segment.text.clone(),
                    })
                    .collect()
            }),
            speaker_segments: include_speakers.then(|| {
                record
                    .state
                    .results
                    .speaker_segments
                    .iter()
                    .map(|segment| AgentSpeakerSegment {
                        transcript_segment_id: segment.transcript_segment_id.clone(),
                        speaker_id: segment.speaker_id.clone(),
                        speaker_profile_id: segment.speaker_profile_id.clone(),
                        speaker_label: segment.speaker_label.clone(),
                        start_ms: segment.start_ms,
                        end_ms: segment.end_ms,
                        text: segment.text.clone(),
                    })
                    .collect()
            }),
        };
        let result = AgentToolResult::Document(projection);
        enforce_result_size(&result, Some(&document_id))?;
        Ok(result)
    }

    fn update_document(
        &self,
        request: &CapabilityRequest,
        arguments: UpdateDocumentArguments,
    ) -> Result<AgentToolResult, AgentToolError> {
        let mutation_id = format!("{}:{}", request.request_id, request.tool_call_id);
        let fingerprint = mutation_fingerprint(UPDATE_DOCUMENT_TOOL, request)?;
        let document_id = DocumentId::new(arguments.document_id.clone());
        self.ensure_access(&document_id, &request.scope)?;
        if let Some(commit) = self
            .documents
            .find_agent_mutation(&mutation_id, &fingerprint)
            .map_err(map_store_error)?
        {
            return mutation_result(commit, Vec::new());
        }
        if arguments.source_markdown_sha256.is_none() {
            let mut error = invalid_arguments(
                "sourceMarkdownSha256",
                "document updates must include the Markdown hash from read_document",
            );
            error.retryable = true;
            return Err(error);
        }
        let mut record = self.read_required(&document_id)?;
        let source_markdown_sha256 = arguments.source_markdown_sha256.clone();
        ensure_markdown_hash(
            &document_id,
            &record.markdown,
            source_markdown_sha256.as_deref(),
        )?;
        validate_patch(&arguments.patch)?;
        if let Some(title) = arguments.patch.title {
            record.state.content.title = title;
        }
        if let Some(tags) = arguments.patch.tags {
            record.state.content.tags = tags;
        }
        if let Some(summary) = arguments.patch.summary {
            record.state.content.summary = summary;
        }
        if let Some(status) = arguments.patch.document_status {
            record.state.document_status.value = status;
        }
        if let Some(markdown) = arguments.patch.markdown {
            record.markdown = markdown;
        }
        record.state.schema.revision = arguments.expected_revision.saturating_add(1);
        record.state.schema.updated_at_ms = now_ms();
        let batch = self.make_batch(
            request,
            UPDATE_DOCUMENT_TOOL,
            vec![DocumentMutationWrite {
                record,
                expected_revision: Some(arguments.expected_revision),
                source_markdown_sha256,
            }],
        )?;
        self.execute_or_defer(batch, Vec::new(), Vec::new())
    }

    fn create_document(
        &self,
        request: &CapabilityRequest,
        arguments: CreateDocumentArguments,
    ) -> Result<AgentToolResult, AgentToolError> {
        let mutation_id = format!("{}:{}", request.request_id, request.tool_call_id);
        let fingerprint = mutation_fingerprint(CREATE_DOCUMENT_TOOL, request)?;

        let parent_id = arguments.parent_document_id.as_deref().map(DocumentId::new);
        if let CreateDocumentSource::AudioAsset { asset_id } = &arguments.source {
            self.ensure_access(&DocumentId::new(asset_id.clone()), &request.scope)?;
        }
        if let Some(parent_id) = &parent_id {
            self.ensure_access(parent_id, &request.scope)?;
        }
        if let Some(commit) = self
            .documents
            .find_agent_mutation(&mutation_id, &fingerprint)
            .map_err(map_store_error)?
        {
            return mutation_result(commit, Vec::new());
        }

        let source_record = match &arguments.source {
            CreateDocumentSource::AudioAsset { asset_id } => {
                Some(self.read_required(&DocumentId::new(asset_id.clone()))?)
            }
            CreateDocumentSource::NoAudio => None,
        };
        let parent = if let Some(parent_id) = &parent_id {
            self.ensure_access(parent_id, &request.scope)?;
            let expected = arguments.expected_parent_revision.ok_or_else(|| {
                invalid_arguments(
                    "expectedParentRevision",
                    "a parent document requires expectedParentRevision",
                )
            })?;
            let source_markdown_sha256 =
                arguments.source_markdown_sha256.as_deref().ok_or_else(|| {
                    let mut error = invalid_arguments(
                        "sourceMarkdownSha256",
                        "a parent document requires the Markdown hash from read_document",
                    );
                    error.retryable = true;
                    error
                })?;
            let parent = self.read_required(parent_id)?;
            if parent.state.revision() != expected {
                let mut error = document_error(
                    AgentErrorCode::DocumentRevisionConflict,
                    "parent document changed before creation",
                    parent_id,
                    true,
                );
                error.expected_revision = Some(expected);
                error.actual_revision = Some(parent.state.revision());
                return Err(error);
            }
            ensure_markdown_hash(parent_id, &parent.markdown, Some(source_markdown_sha256))?;
            Some(parent)
        } else {
            None
        };

        let audio_path = source_record
            .as_ref()
            .map(|record| record.state.audio_path().to_owned())
            .unwrap_or_default();
        let root_asset = source_record.as_ref().map(|record| {
            record
                .state
                .source
                .audio_asset_id
                .clone()
                .unwrap_or_else(|| record.state.id().clone())
        });
        if let Some(parent) = &parent {
            if parent.state.hierarchy.depth >= 2 {
                return Err(document_error(
                    AgentErrorCode::TreeMaxDepthExceeded,
                    "created document would exceed the three-level tree",
                    parent.state.id(),
                    true,
                ));
            }
            let parent_root = if parent.state.audio_path().is_empty() {
                None
            } else {
                Some(
                    parent
                        .state
                        .source
                        .audio_asset_id
                        .clone()
                        .unwrap_or_else(|| parent.state.id().clone()),
                )
            };
            if parent_root != root_asset {
                return Err(AgentToolError::new(
                    AgentErrorCode::TreeCrossAudioAsset,
                    "created document must use the parent audio asset",
                    true,
                ));
            }
        }

        let sibling_order = if let Some(parent) = &parent {
            self.documents
                .list()
                .map_err(transaction_error)?
                .into_iter()
                .filter(|document| document.hierarchy.parent_id.as_ref() == Some(parent.state.id()))
                .map(|document| document.hierarchy.sibling_order)
                .max()
                .map_or(0, |order| order.saturating_add(1))
        } else {
            0
        };
        let mut state = DocumentState::new(DocumentId::new_uuid(), arguments.title, audio_path);
        state.content.tags = arguments.tags;
        state.content.summary = arguments.summary;
        if let Some(status) = arguments.document_status {
            state.document_status.value = status;
        }
        state.hierarchy.parent_id = parent.as_ref().map(|record| record.state.id().clone());
        state.hierarchy.depth = parent
            .as_ref()
            .map_or(0, |record| record.state.hierarchy.depth.saturating_add(1));
        state.hierarchy.sibling_order = sibling_order;
        state.source.audio_asset_id = root_asset;
        let new_record = contracts::DocumentRecord {
            state,
            markdown: arguments.markdown,
        };

        let mut writes = Vec::with_capacity(2);
        if let Some(mut parent) = parent {
            let expected = arguments
                .expected_parent_revision
                .expect("validated parent revision");
            parent.state.schema.revision = expected.saturating_add(1);
            parent.state.schema.updated_at_ms = now_ms();
            writes.push(DocumentMutationWrite {
                source_markdown_sha256: arguments.source_markdown_sha256.clone(),
                record: parent,
                expected_revision: Some(expected),
            });
        }
        writes.push(DocumentMutationWrite {
            record: new_record,
            expected_revision: None,
            source_markdown_sha256: None,
        });
        let batch = self.make_batch(request, CREATE_DOCUMENT_TOOL, writes)?;
        self.execute_or_defer(batch, Vec::new(), Vec::new())
    }

    fn create_semantic_children(
        &self,
        request: &CapabilityRequest,
        arguments: CreateSemanticChildrenArguments,
    ) -> Result<AgentToolResult, AgentToolError> {
        let mutation_id = format!("{}:{}", request.request_id, request.tool_call_id);
        let fingerprint = mutation_fingerprint(CREATE_SEMANTIC_CHILDREN_TOOL, request)?;
        let parent_id = DocumentId::new(arguments.parent_document_id.clone());
        self.ensure_access(&parent_id, &request.scope)?;
        if let Some(commit) = self
            .documents
            .find_agent_mutation(&mutation_id, &fingerprint)
            .map_err(map_store_error)?
        {
            return mutation_result(commit, Vec::new());
        }
        let mut parent = self.read_required(&parent_id)?;
        if arguments.source_markdown_sha256.is_none() {
            let mut error = invalid_arguments(
                "sourceMarkdownSha256",
                "semantic writes must include the parent Markdown hash from read_document",
            );
            error.retryable = true;
            return Err(error);
        }
        let source_markdown_sha256 = arguments.source_markdown_sha256.clone();
        ensure_markdown_hash(
            &parent_id,
            &parent.markdown,
            source_markdown_sha256.as_deref(),
        )?;
        if parent.state.hierarchy.depth >= 2 {
            return Err(document_error(
                AgentErrorCode::TreeMaxDepthExceeded,
                "semantic children would exceed the three-level tree",
                &parent_id,
                true,
            ));
        }
        if parent.state.results.transcript.is_empty() {
            return Err(document_error(
                AgentErrorCode::SegmentNotFound,
                "parent document has no transcript segments",
                &parent_id,
                true,
            ));
        }

        let positions: BTreeMap<&str, usize> = parent
            .state
            .results
            .transcript
            .iter()
            .enumerate()
            .map(|(index, segment)| (segment.id.as_str(), index))
            .collect();
        let mut resolved = Vec::with_capacity(arguments.children.len());
        for child in arguments.children {
            if child.start_segment_id.trim().is_empty() {
                return Err(invalid_arguments(
                    "children.startSegmentId",
                    "child start segment ID cannot be empty",
                ));
            }
            if child.end_segment_id.trim().is_empty() {
                return Err(invalid_arguments(
                    "children.endSegmentId",
                    "child end segment ID cannot be empty",
                ));
            }
            let start = segment_position(&positions, &child.start_segment_id, &parent_id)?;
            let end = segment_position(&positions, &child.end_segment_id, &parent_id)?;
            if start > end {
                return Err(document_error(
                    AgentErrorCode::RangeInvalidOrder,
                    "child start segment follows its end segment",
                    &parent_id,
                    true,
                ));
            }
            if child.title.trim().is_empty() {
                return Err(invalid_arguments(
                    "children.title",
                    "child title cannot be empty",
                ));
            }
            if child.tags.iter().any(|tag| tag.trim().is_empty()) {
                return Err(invalid_arguments(
                    "children.tags",
                    "child tags cannot contain empty values",
                ));
            }
            for evidence in &child.evidence_segment_ids {
                if evidence.trim().is_empty() {
                    return Err(invalid_arguments(
                        "children.evidenceSegmentIds",
                        "evidence segment IDs cannot be empty",
                    ));
                }
                let evidence_position = segment_position(&positions, evidence, &parent_id)?;
                if evidence_position < start || evidence_position > end {
                    let mut error = document_error(
                        AgentErrorCode::RangeOutsideParent,
                        "evidence segment is outside its child range",
                        &parent_id,
                        true,
                    );
                    error.field = Some("children.evidenceSegmentIds".into());
                    return Err(error);
                }
            }
            resolved.push((start, end, child));
        }
        for pair in resolved.windows(2) {
            if pair[0].0 > pair[1].0 {
                return Err(document_error(
                    AgentErrorCode::RangeInvalidOrder,
                    "children must be ordered by transcript position",
                    &parent_id,
                    true,
                ));
            }
            if pair[0].1 >= pair[1].0 {
                return Err(document_error(
                    AgentErrorCode::RangeOverlap,
                    "semantic child ranges overlap",
                    &parent_id,
                    true,
                ));
            }
        }

        let mut covered = vec![false; parent.state.results.transcript.len()];
        for (start, end, _) in &resolved {
            for value in &mut covered[*start..=*end] {
                *value = true;
            }
        }
        let uncovered_segment_ids: Vec<String> = covered
            .iter()
            .enumerate()
            .filter(|(_, covered)| !**covered)
            .map(|(index, _)| parent.state.results.transcript[index].id.clone())
            .collect();
        if arguments.mode == SemanticSplitMode::Partition && !uncovered_segment_ids.is_empty() {
            let mut error = document_error(
                AgentErrorCode::RangeCoverageGap,
                "partition must cover every parent transcript segment",
                &parent_id,
                true,
            );
            error.context.insert(
                "uncoveredSegmentIds".into(),
                serde_json::json!(uncovered_segment_ids),
            );
            return Err(error);
        }

        let snapshot = self.documents.list().map_err(transaction_error)?;
        let base_order = snapshot
            .iter()
            .filter(|document| document.hierarchy.parent_id.as_ref() == Some(&parent_id))
            .map(|document| document.hierarchy.sibling_order)
            .max()
            .map_or(0, |order| order.saturating_add(1));
        let mut writes = Vec::with_capacity(resolved.len() + 1);
        let mut range_previews = Vec::with_capacity(resolved.len());
        parent.state.schema.revision = arguments.expected_parent_revision.saturating_add(1);
        parent.state.schema.updated_at_ms = now_ms();
        writes.push(DocumentMutationWrite {
            record: parent.clone(),
            expected_revision: Some(arguments.expected_parent_revision),
            source_markdown_sha256,
        });
        for (offset, (start, end, child_arguments)) in resolved.into_iter().enumerate() {
            let start_ms = parent.state.results.transcript[start].start_ms;
            let end_ms = parent.state.results.transcript[end].end_ms;
            if start_ms >= end_ms {
                return Err(document_error(
                    AgentErrorCode::RangeInvalidOrder,
                    "semantic child has a non-positive time range",
                    &parent_id,
                    true,
                ));
            }
            if start_ms < parent.state.source.audio_start_ms
                || parent
                    .state
                    .source
                    .audio_end_ms
                    .is_some_and(|parent_end| end_ms > parent_end)
            {
                return Err(document_error(
                    AgentErrorCode::RangeOutsideParent,
                    "semantic child time range is outside its parent",
                    &parent_id,
                    true,
                ));
            }
            let mut child = parent.state.semantic_child(
                DocumentId::new_uuid(),
                child_arguments.title,
                start_ms,
                end_ms,
                child_arguments.tags,
            );
            child.content.summary = child_arguments.summary;
            child.hierarchy.sibling_order = base_order.saturating_add(offset as u32);
            child.processing = parent.state.processing.clone();
            child.results.transcript = parent.state.results.transcript[start..=end].to_vec();
            child.results.diarization = parent
                .state
                .results
                .diarization
                .iter()
                .filter_map(|segment| {
                    let mut segment = segment.clone();
                    segment.start_ms = segment.start_ms.max(start_ms);
                    segment.end_ms = segment.end_ms.min(end_ms);
                    (segment.start_ms < segment.end_ms).then_some(segment)
                })
                .collect();
            child.results.speaker_segments = parent
                .state
                .results
                .speaker_segments
                .iter()
                .filter_map(|segment| {
                    if !child
                        .results
                        .transcript
                        .iter()
                        .any(|transcript| transcript.id == segment.transcript_segment_id)
                    {
                        return None;
                    }
                    let mut segment = segment.clone();
                    segment.start_ms = segment.start_ms.max(start_ms);
                    segment.end_ms = segment.end_ms.min(end_ms);
                    (segment.start_ms < segment.end_ms).then_some(segment)
                })
                .collect();
            range_previews.push(AgentRangePreview {
                document_id: Some(child.id().as_str().to_string()),
                start_segment_id: child_arguments.start_segment_id.clone(),
                end_segment_id: child_arguments.end_segment_id.clone(),
                evidence_segment_ids: child_arguments.evidence_segment_ids.clone(),
            });
            let markdown =
                render_semantic_child_markdown(&child, &child_arguments.evidence_segment_ids);
            writes.push(DocumentMutationWrite {
                record: contracts::DocumentRecord {
                    state: child,
                    markdown,
                },
                expected_revision: None,
                source_markdown_sha256: None,
            });
        }
        let batch = self.make_batch(request, CREATE_SEMANTIC_CHILDREN_TOOL, writes)?;
        self.execute_or_defer(batch, uncovered_segment_ids, range_previews)
    }

    fn move_document(
        &self,
        request: &CapabilityRequest,
        arguments: MoveDocumentArguments,
    ) -> Result<AgentToolResult, AgentToolError> {
        let document_id = DocumentId::new(arguments.document_id);
        let mutation_id = format!("{}:{}", request.request_id, request.tool_call_id);
        let fingerprint = mutation_fingerprint(MOVE_DOCUMENT_TOOL, request)?;
        self.ensure_access(&document_id, &request.scope)?;
        let new_parent_id = arguments.new_parent_document_id.map(DocumentId::new);
        if let Some(parent_id) = &new_parent_id {
            self.ensure_access(parent_id, &request.scope)?;
        }
        let target_id = arguments.target_sibling_document_id.map(DocumentId::new);
        if let Some(target_id) = &target_id {
            self.ensure_access(target_id, &request.scope)?;
        }
        if let Some(commit) = self
            .documents
            .find_agent_mutation(&mutation_id, &fingerprint)
            .map_err(map_store_error)?
        {
            return mutation_result(commit, Vec::new());
        }
        let records = self.read_all_records()?;
        let mut documents: Vec<DocumentState> =
            records.iter().map(|record| record.state.clone()).collect();
        let current = documents
            .iter()
            .find(|document| document.id() == &document_id)
            .ok_or_else(|| not_found(&document_id))?;
        let old_parent_id = current.hierarchy.parent_id.clone();

        reparent_project(&mut documents, &document_id, new_parent_id.as_ref(), 0)
            .map_err(|error| tree_error(error, &document_id))?;
        position_document(
            &mut documents,
            &document_id,
            new_parent_id.as_ref(),
            arguments.placement,
            target_id.as_ref(),
        )?;

        let before: BTreeMap<String, &contracts::DocumentRecord> = records
            .iter()
            .map(|record| (record.state.id().as_str().to_string(), record))
            .collect();
        let parents_to_advance = [old_parent_id.as_ref(), new_parent_id.as_ref()]
            .into_iter()
            .flatten()
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut writes = Vec::new();
        for mut document in documents {
            let original = before
                .get(document.id().as_str())
                .expect("snapshot state and records must match");
            if document.id() == &document_id
                || document.hierarchy != original.state.hierarchy
                || parents_to_advance.contains(document.id())
            {
                self.ensure_access(document.id(), &request.scope)?;
                let expected_revision = if document.id() == &document_id {
                    arguments.expected_revision
                } else {
                    original.state.revision()
                };
                document.schema.revision = expected_revision.saturating_add(1);
                document.schema.updated_at_ms = now_ms();
                writes.push(DocumentMutationWrite {
                    record: contracts::DocumentRecord {
                        state: document,
                        markdown: original.markdown.clone(),
                    },
                    expected_revision: Some(expected_revision),
                    source_markdown_sha256: Some(markdown_sha256(&original.markdown)),
                });
            }
        }
        let batch = self.make_batch(request, MOVE_DOCUMENT_TOOL, writes)?;
        self.execute_or_defer(batch, Vec::new(), Vec::new())
    }

    fn reject_read_only_fields(&self, arguments: &serde_json::Value) -> Result<(), AgentToolError> {
        let Some(patch) = arguments
            .get("patch")
            .and_then(serde_json::Value::as_object)
        else {
            return Ok(());
        };
        for field in [
            "schema",
            "identity",
            "source",
            "hierarchy",
            "processing",
            "results",
            "transcript",
            "diarization",
            "speakerSegments",
        ] {
            if patch.contains_key(field) {
                let mut error = AgentToolError::new(
                    AgentErrorCode::DocumentFieldReadOnly,
                    format!("document field is read-only: {field}"),
                    true,
                );
                error.field = Some(format!("patch.{field}"));
                return Err(error);
            }
        }
        Ok(())
    }

    fn read_required(
        &self,
        document_id: &DocumentId,
    ) -> Result<contracts::DocumentRecord, AgentToolError> {
        self.documents
            .read(document_id)
            .map_err(transaction_error)?
            .ok_or_else(|| not_found(document_id))
    }

    fn read_all_records(&self) -> Result<Vec<contracts::DocumentRecord>, AgentToolError> {
        let states = self.documents.list().map_err(transaction_error)?;
        states
            .into_iter()
            .map(|state| self.read_required(state.id()))
            .collect()
    }

    fn make_batch(
        &self,
        request: &CapabilityRequest,
        tool_name: &str,
        writes: Vec<DocumentMutationWrite>,
    ) -> Result<DocumentMutationBatch, AgentToolError> {
        if writes.is_empty() {
            return Err(invalid_arguments(
                "arguments",
                "mutation does not change any document",
            ));
        }
        let mutation_id = format!("{}:{}", request.request_id, request.tool_call_id);
        let fingerprint = mutation_fingerprint(tool_name, request)?;
        Ok(DocumentMutationBatch {
            mutation_id: mutation_id.clone(),
            fingerprint,
            undo_token: Uuid::new_v4().to_string(),
            tool_name: tool_name.into(),
            provenance: AgentMutationProvenance {
                actor: self.actor,
                mutation_id,
                skill: None,
                connection_id: self
                    .llm_provenance
                    .as_ref()
                    .map(|provenance| provenance.connection_id.clone()),
                protocol: self
                    .llm_provenance
                    .as_ref()
                    .map(|provenance| provenance.protocol.clone()),
                requested_model: self
                    .llm_provenance
                    .as_ref()
                    .map(|provenance| provenance.requested_model.clone()),
                resolved_model: self
                    .llm_provenance
                    .as_ref()
                    .map(|provenance| provenance.resolved_model.clone()),
                source_revisions: writes
                    .iter()
                    .map(|write| AgentSourceRevision {
                        document_id: write.record.state.id().as_str().into(),
                        revision: write.expected_revision,
                    })
                    .collect(),
                applied_at_ms: 0,
            },
            writes,
        })
    }

    fn execute_or_defer(
        &self,
        batch: DocumentMutationBatch,
        uncovered_segment_ids: Vec<String>,
        ranges: Vec<AgentRangePreview>,
    ) -> Result<AgentToolResult, AgentToolError> {
        if self.write_mode == AgentWriteMode::Automatic {
            let commit = self
                .documents
                .commit_agent_mutation(batch)
                .map_err(map_store_error)?;
            return mutation_result(commit, uncovered_segment_ids);
        }
        if let Some(existing) = self
            .documents
            .find_pending_agent_mutation(&batch.mutation_id, &batch.fingerprint)
            .map_err(map_store_error)?
        {
            self.ensure_pending_result_size(
                &existing.batch,
                &existing.uncovered_segment_ids,
                &existing.ranges,
                &existing.confirmation_id,
                existing.expires_at_ms,
            )?;
            let (field_diffs, tree_changes) = self.preview_batch(&existing.batch)?;
            let affected_document_ids = existing
                .batch
                .writes
                .iter()
                .map(|write| write.record.state.id().as_str().to_string())
                .collect();
            return Ok(AgentToolResult::Pending(AgentPendingMutation {
                confirmation_id: existing.confirmation_id,
                expires_at_ms: existing.expires_at_ms,
                affected_document_ids,
                field_diffs,
                tree_changes,
                ranges: existing.ranges,
                uncovered_segment_ids: existing.uncovered_segment_ids,
            }));
        }
        let (field_diffs, tree_changes) = self.preview_batch(&batch)?;
        let confirmation_id = Uuid::new_v4().to_string();
        let expires_at_ms = now_ms().saturating_add(self.confirmation_ttl_ms);
        self.ensure_pending_result_size(
            &batch,
            &uncovered_segment_ids,
            &ranges,
            &confirmation_id,
            expires_at_ms,
        )?;
        let affected_document_ids = batch
            .writes
            .iter()
            .map(|write| write.record.state.id().as_str().to_string())
            .collect();
        self.documents
            .save_pending_agent_mutation(PendingDocumentMutation {
                confirmation_id: confirmation_id.clone(),
                batch,
                uncovered_segment_ids: uncovered_segment_ids.clone(),
                expires_at_ms,
                ranges: ranges.clone(),
            })
            .map_err(map_store_error)?;
        Ok(AgentToolResult::Pending(AgentPendingMutation {
            confirmation_id,
            expires_at_ms,
            affected_document_ids,
            field_diffs,
            tree_changes,
            ranges,
            uncovered_segment_ids,
        }))
    }

    fn preview_batch(
        &self,
        batch: &DocumentMutationBatch,
    ) -> Result<(Vec<AgentFieldDiff>, Vec<AgentTreeChange>), AgentToolError> {
        let mut field_diffs = Vec::new();
        let mut tree_changes = Vec::new();
        for write in &batch.writes {
            let document_id = write.record.state.id().as_str().to_string();
            let before = self
                .documents
                .read(write.record.state.id())
                .map_err(transaction_error)?;
            let after = &write.record;
            let before_state = before.as_ref().map(|record| &record.state);
            push_field_diff(
                &mut field_diffs,
                &document_id,
                "title",
                before_state.map(|state| state.content.title.clone()),
                Some(after.state.content.title.clone()),
            );
            push_field_diff(
                &mut field_diffs,
                &document_id,
                "tags",
                before_state.map(|state| format_string_list(&state.content.tags)),
                Some(format_string_list(&after.state.content.tags)),
            );
            push_field_diff(
                &mut field_diffs,
                &document_id,
                "summary",
                before_state.and_then(|state| state.content.summary.clone()),
                after.state.content.summary.clone(),
            );
            push_field_diff(
                &mut field_diffs,
                &document_id,
                "document_status",
                before_state.map(|state| state.document_status.value.clone()),
                Some(after.state.document_status.value.clone()),
            );
            push_field_diff(
                &mut field_diffs,
                &document_id,
                "markdown",
                before.as_ref().map(|record| record.markdown.clone()),
                Some(after.markdown.clone()),
            );

            let previous_parent_id = before_state
                .and_then(|state| state.hierarchy.parent_id.as_ref())
                .map(|id| id.as_str().to_string());
            let new_parent_id = after
                .state
                .hierarchy
                .parent_id
                .as_ref()
                .map(|id| id.as_str().to_string());
            let previous_sibling_order = before_state.map(|state| state.hierarchy.sibling_order);
            let new_sibling_order = Some(after.state.hierarchy.sibling_order);
            if before.is_none()
                || previous_parent_id != new_parent_id
                || previous_sibling_order != new_sibling_order
            {
                tree_changes.push(AgentTreeChange {
                    document_id,
                    previous_parent_id,
                    new_parent_id,
                    previous_sibling_order,
                    new_sibling_order,
                });
            }
        }
        Ok((field_diffs, tree_changes))
    }

    fn ensure_pending_result_size(
        &self,
        batch: &DocumentMutationBatch,
        uncovered_segment_ids: &[String],
        ranges: &[AgentRangePreview],
        confirmation_id: &str,
        expires_at_ms: u64,
    ) -> Result<(), AgentToolError> {
        let mut field_diff_sizes = Vec::new();
        let mut tree_change_sizes = Vec::new();
        for write in &batch.writes {
            let before = self
                .documents
                .read(write.record.state.id())
                .map_err(transaction_error)?;
            let before_state = before.as_ref().map(|record| &record.state);
            let document_id = write.record.state.id().as_str();
            let field = |name: &'static str, before: Option<usize>, after: usize| {
                json_object_size([
                    ("documentId", json_string_size(document_id)),
                    ("field", json_string_size(name)),
                    ("before", before.unwrap_or(4)),
                    ("after", after),
                ])
            };
            // This is deliberately an upper bound: it includes unchanged
            // fields, so rejecting here cannot allow an oversized pending
            // result to be constructed later.
            field_diff_sizes.extend([
                field(
                    "title",
                    before_state.map(|state| json_string_size(&state.content.title)),
                    json_string_size(&write.record.state.content.title),
                ),
                field(
                    "tags",
                    before_state.map(|state| 2 + 2 * serialized_json_size(&state.content.tags)),
                    2 + 2 * serialized_json_size(&write.record.state.content.tags),
                ),
                field(
                    "summary",
                    before_state
                        .and_then(|state| state.content.summary.as_deref().map(json_string_size)),
                    serialized_json_size(&write.record.state.content.summary),
                ),
                field(
                    "document_status",
                    before_state.map(|state| json_string_size(&state.document_status.value)),
                    json_string_size(&write.record.state.document_status.value),
                ),
                field(
                    "markdown",
                    before
                        .as_ref()
                        .map(|record| projected_markdown_size(&record.markdown)),
                    projected_markdown_size(&write.record.markdown),
                ),
            ]);
            tree_change_sizes.push(json_object_size([
                ("documentId", json_string_size(document_id)),
                (
                    "previousParentId",
                    before_state
                        .map_or(4, |state| serialized_json_size(&state.hierarchy.parent_id)),
                ),
                (
                    "newParentId",
                    serialized_json_size(&write.record.state.hierarchy.parent_id),
                ),
                (
                    "previousSiblingOrder",
                    before_state.map_or(4, |state| {
                        serialized_json_size(&Some(state.hierarchy.sibling_order))
                    }),
                ),
                (
                    "newSiblingOrder",
                    serialized_json_size(&Some(write.record.state.hierarchy.sibling_order)),
                ),
            ]));
        }
        let affected = json_array_size(
            batch
                .writes
                .iter()
                .map(|write| json_string_size(write.record.state.id().as_str())),
        );
        let payload = json_object_size([
            ("confirmationId", json_string_size(confirmation_id)),
            ("expiresAtMs", serialized_json_size(&expires_at_ms)),
            ("affectedDocumentIds", affected),
            ("fieldDiffs", json_array_size(field_diff_sizes)),
            ("treeChanges", json_array_size(tree_change_sizes)),
            ("ranges", serialized_json_size(&ranges)),
            (
                "uncoveredSegmentIds",
                serialized_json_size(&uncovered_segment_ids),
            ),
        ]);
        enforce_result_size_bytes(tagged_result_size("pending", payload), None)
    }

    pub fn confirm(&self, confirmation_id: &str, request_id: &str) -> CapabilityResponse {
        CapabilityProjectionPolicy::project_response(self.confirm_inner(
            confirmation_id,
            request_id,
            None,
        ))
    }

    /// Confirm a pending mutation while applying the request's narrower scope.
    /// The port's grant remains authoritative; this selector can only reduce it.
    pub fn confirm_scoped(
        &self,
        confirmation_id: &str,
        request_id: &str,
        scope: &AgentScopeSelector,
    ) -> CapabilityResponse {
        CapabilityProjectionPolicy::project_response(self.confirm_inner(
            confirmation_id,
            request_id,
            Some(scope),
        ))
    }

    fn confirm_inner(
        &self,
        confirmation_id: &str,
        request_id: &str,
        scope: Option<&AgentScopeSelector>,
    ) -> CapabilityResponse {
        if !self.granted_controls.contains(CONFIRM_CONTROL) {
            return CapabilityResponse::failure(
                request_id,
                AgentToolError::new(
                    AgentErrorCode::ToolNotAllowed,
                    "confirm control was not granted",
                    false,
                ),
            );
        }
        let pending = match self.documents.take_pending_agent_mutation(confirmation_id) {
            Ok(pending) => pending,
            Err(error) => {
                return CapabilityResponse::failure(request_id, map_store_error(error));
            }
        };
        let result = match pending {
            None => Err(AgentToolError::new(
                AgentErrorCode::WriteConfirmationExpired,
                "pending mutation was not found or already consumed",
                true,
            )),
            Some(pending) if pending.expires_at_ms < now_ms() => Err(AgentToolError::new(
                AgentErrorCode::WriteConfirmationExpired,
                "pending mutation expired",
                true,
            )),
            Some(pending) => {
                if let Err(error) = self.ensure_pending_access_scoped(&pending.batch, scope) {
                    match self.documents.save_pending_agent_mutation(pending) {
                        Ok(()) => Err(error),
                        Err(store_error) => Err(map_store_error(store_error)),
                    }
                } else {
                    let validation = self
                        .documents
                        .validate_agent_mutation_sources(&pending.batch)
                        .map_err(map_store_error);
                    match validation {
                        Ok(()) => self
                            .documents
                            .commit_agent_mutation(pending.batch)
                            .map_err(map_store_error)
                            .and_then(|commit| {
                                mutation_result(commit, pending.uncovered_segment_ids)
                            }),
                        Err(error) => match self.documents.save_pending_agent_mutation(pending) {
                            Ok(()) => Err(error),
                            Err(store_error) => Err(map_store_error(store_error)),
                        },
                    }
                }
            }
        };
        match result {
            Ok(result) => CapabilityResponse::success(request_id, result),
            Err(error) => CapabilityResponse::failure(request_id, error),
        }
    }

    pub fn reject(&self, confirmation_id: &str, request_id: &str) -> CapabilityResponse {
        CapabilityProjectionPolicy::project_response(self.reject_inner(
            confirmation_id,
            request_id,
            None,
        ))
    }

    /// Reject a pending mutation while applying the request's narrower scope.
    pub fn reject_scoped(
        &self,
        confirmation_id: &str,
        request_id: &str,
        scope: &AgentScopeSelector,
    ) -> CapabilityResponse {
        CapabilityProjectionPolicy::project_response(self.reject_inner(
            confirmation_id,
            request_id,
            Some(scope),
        ))
    }

    fn reject_inner(
        &self,
        confirmation_id: &str,
        request_id: &str,
        scope: Option<&AgentScopeSelector>,
    ) -> CapabilityResponse {
        if !self.granted_controls.contains(REJECT_CONTROL) {
            return CapabilityResponse::failure(
                request_id,
                AgentToolError::new(
                    AgentErrorCode::ToolNotAllowed,
                    "reject control was not granted",
                    false,
                ),
            );
        }
        match self.documents.take_pending_agent_mutation(confirmation_id) {
            Ok(Some(pending)) => {
                if let Err(error) = self.ensure_pending_access_scoped(&pending.batch, scope) {
                    if let Err(store_error) = self.documents.save_pending_agent_mutation(pending) {
                        return CapabilityResponse::failure(
                            request_id,
                            map_store_error(store_error),
                        );
                    }
                    return CapabilityResponse::failure(request_id, error);
                }
                if pending.expires_at_ms < now_ms() {
                    return CapabilityResponse::failure(
                        request_id,
                        AgentToolError::new(
                            AgentErrorCode::WriteConfirmationExpired,
                            "pending mutation expired",
                            true,
                        ),
                    );
                }
                CapabilityResponse::failure(
                    request_id,
                    AgentToolError::new(
                        AgentErrorCode::WritePolicyRejected,
                        "pending mutation was rejected",
                        false,
                    ),
                )
            }
            Ok(None) => CapabilityResponse::failure(
                request_id,
                AgentToolError::new(
                    AgentErrorCode::WriteConfirmationExpired,
                    "pending mutation was not found or already consumed",
                    true,
                ),
            ),
            Err(error) => CapabilityResponse::failure(request_id, map_store_error(error)),
        }
    }

    pub fn undo(&self, undo_token: &str, request_id: &str) -> CapabilityResponse {
        CapabilityProjectionPolicy::project_response(self.undo_inner(undo_token, request_id, None))
    }

    /// Undo a mutation while applying the request's narrower scope.
    pub fn undo_scoped(
        &self,
        undo_token: &str,
        request_id: &str,
        scope: &AgentScopeSelector,
    ) -> CapabilityResponse {
        CapabilityProjectionPolicy::project_response(self.undo_inner(
            undo_token,
            request_id,
            Some(scope),
        ))
    }

    fn undo_inner(
        &self,
        undo_token: &str,
        request_id: &str,
        scope: Option<&AgentScopeSelector>,
    ) -> CapabilityResponse {
        if !self.granted_controls.contains(UNDO_CONTROL) {
            return CapabilityResponse::failure(
                request_id,
                AgentToolError::new(
                    AgentErrorCode::ToolNotAllowed,
                    "undo control was not granted",
                    false,
                ),
            );
        }
        let ids = match self.documents.agent_mutation_document_ids(undo_token) {
            Ok(ids) => ids,
            Err(error) => return CapabilityResponse::failure(request_id, map_store_error(error)),
        };
        for id in ids {
            let id = DocumentId::new(id);
            let in_request_scope = scope.map_or(true, |scope| {
                scope.all_documents
                    || scope.document_ids.is_empty()
                    || scope
                        .document_ids
                        .iter()
                        .any(|candidate| candidate == id.as_str())
            });
            if !self.access.grants(&id) || !in_request_scope {
                return CapabilityResponse::failure(
                    request_id,
                    document_error(
                        AgentErrorCode::DocumentAccessDenied,
                        "undo mutation is outside the current caller grant",
                        &id,
                        false,
                    ),
                );
            }
        }
        match self.documents.undo_agent_mutation(undo_token) {
            Ok(commit) => match mutation_result(commit, Vec::new()) {
                Ok(result) => CapabilityResponse::success(request_id, result),
                Err(error) => CapabilityResponse::failure(request_id, error),
            },
            Err(error) => CapabilityResponse::failure(request_id, map_store_error(error)),
        }
    }

    fn ensure_access(
        &self,
        document_id: &DocumentId,
        scope: &AgentScopeSelector,
    ) -> Result<(), AgentToolError> {
        if self.is_in_effective_scope(document_id, scope) {
            return Ok(());
        }
        let mut error = AgentToolError::new(
            AgentErrorCode::DocumentAccessDenied,
            "document is outside the granted scope",
            false,
        );
        error.document_id = Some(document_id.as_str().into());
        Err(error)
    }

    fn ensure_pending_access_scoped(
        &self,
        batch: &DocumentMutationBatch,
        scope: Option<&AgentScopeSelector>,
    ) -> Result<(), AgentToolError> {
        for write in &batch.writes {
            let in_request_scope = scope.map_or(true, |scope| {
                scope.all_documents
                    || scope.document_ids.is_empty()
                    || scope
                        .document_ids
                        .iter()
                        .any(|candidate| candidate == write.record.state.id().as_str())
            });
            let requires_port_access = scope.is_some() || write.expected_revision.is_some();
            if requires_port_access
                && (!self.access.grants(write.record.state.id()) || !in_request_scope)
            {
                return Err(document_error(
                    AgentErrorCode::DocumentAccessDenied,
                    "pending mutation is outside the current caller grant",
                    write.record.state.id(),
                    false,
                ));
            }
        }
        Ok(())
    }

    fn is_in_effective_scope(&self, document_id: &DocumentId, scope: &AgentScopeSelector) -> bool {
        self.access.grants(document_id)
            && (scope.all_documents
                || scope.document_ids.is_empty()
                || scope
                    .document_ids
                    .iter()
                    .any(|candidate| candidate == document_id.as_str()))
    }
}

fn mutation_fingerprint(
    tool_name: &str,
    request: &CapabilityRequest,
) -> Result<String, AgentToolError> {
    let mut hasher = Sha256::new();
    hasher.update(tool_name.as_bytes());
    hasher.update(
        serde_json::to_vec(request).map_err(|error| {
            transaction_error(format!("cannot fingerprint tool request: {error}"))
        })?,
    );
    Ok(format!("{:x}", hasher.finalize()))
}

fn validate_create_document_arguments(
    arguments: &CreateDocumentArguments,
) -> Result<(), AgentToolError> {
    if arguments.title.trim().is_empty() {
        return Err(invalid_arguments("title", "document title cannot be empty"));
    }
    if arguments.tags.iter().any(|tag| tag.trim().is_empty()) {
        return Err(invalid_arguments(
            "tags",
            "document tags cannot contain empty values",
        ));
    }
    if arguments.parent_document_id.is_none() && arguments.expected_parent_revision.is_some() {
        return Err(invalid_arguments(
            "expectedParentRevision",
            "expectedParentRevision requires parentDocumentId",
        ));
    }
    if arguments.parent_document_id.is_none() && arguments.source_markdown_sha256.is_some() {
        return Err(invalid_arguments(
            "sourceMarkdownSha256",
            "sourceMarkdownSha256 requires parentDocumentId",
        ));
    }
    if let CreateDocumentSource::AudioAsset { asset_id } = &arguments.source {
        validate_document_id_argument(asset_id, "source.assetId")?;
    }
    Ok(())
}

fn push_field_diff(
    diffs: &mut Vec<AgentFieldDiff>,
    document_id: &str,
    field: &str,
    before: Option<String>,
    after: Option<String>,
) {
    if before != after {
        diffs.push(AgentFieldDiff {
            document_id: document_id.to_string(),
            field: field.to_string(),
            before,
            after,
        });
    }
}

fn format_string_list(values: &[String]) -> String {
    serde_json::to_string(values).expect("string list serialization cannot fail")
}

fn decode_arguments<T: DeserializeOwned>(
    arguments: &serde_json::Value,
) -> Result<T, AgentToolError> {
    crate::capabilities::decode_object_arguments(arguments)
        .map_err(|error| invalid_arguments("arguments", format!("invalid tool arguments: {error}")))
}

struct CountingWriter {
    bytes: usize,
}

impl Write for CountingWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.bytes = self.bytes.saturating_add(bytes.len());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn serialized_json_size<T: Serialize>(value: &T) -> usize {
    let mut writer = CountingWriter { bytes: 0 };
    serde_json::to_writer(&mut writer, value).expect("capability values must serialize");
    writer.bytes
}

fn json_string_size(value: &str) -> usize {
    serialized_json_size(&value)
}

fn json_array_size<I>(values: I) -> usize
where
    I: IntoIterator<Item = usize>,
{
    let mut size: usize = 2;
    let mut first = true;
    for value in values {
        if !first {
            size += 1;
        }
        first = false;
        size = size.saturating_add(value);
    }
    size
}

fn json_object_size(fields: impl IntoIterator<Item = (&'static str, usize)>) -> usize {
    let mut size: usize = 2;
    let mut first = true;
    for (name, value) in fields {
        if !first {
            size += 1;
        }
        first = false;
        size = size
            .saturating_add(json_string_size(name))
            .saturating_add(1)
            .saturating_add(value);
    }
    size
}

fn tagged_result_size(kind: &str, payload: usize) -> usize {
    json_object_size([("type", json_string_size(kind)), ("payload", payload)])
}

fn projected_locator_size(value: &str) -> usize {
    if value.starts_with('/') || value.starts_with("~/") {
        json_string_size("[redacted-local-path]")
    } else {
        json_string_size(value)
    }
}

fn projected_markdown_size(markdown: &str) -> usize {
    let replacement = "Audio: `[local audio asset]`";
    let mut content = 2;
    let mut first = true;
    for line in markdown.lines() {
        if !first {
            content += 2; // the JSON escaped newline
        }
        first = false;
        let redacted = line
            .strip_prefix("Audio: `")
            .and_then(|value| value.strip_suffix('`'))
            .is_some_and(|locator| {
                locator.starts_with('/') || locator.contains(':') || locator.starts_with('~')
            });
        content += if redacted {
            json_string_size(replacement) - 2
        } else {
            json_string_size(line) - 2
        };
    }
    content
}

fn summary_size(document: &DocumentState, markdown_sha256: &str) -> usize {
    let document_id = document.id().as_str();
    let audio_asset_id = document
        .source
        .audio_asset_id
        .as_ref()
        .map_or(document_id, DocumentId::as_str);
    json_object_size([
        ("documentId", json_string_size(document_id)),
        ("revision", serialized_json_size(&document.revision())),
        ("title", json_string_size(&document.content.title)),
        ("tags", serialized_json_size(&document.content.tags)),
        ("summary", serialized_json_size(&document.content.summary)),
        (
            "documentStatus",
            json_string_size(&document.document_status.value),
        ),
        (
            "parentDocumentId",
            serialized_json_size(
                &document
                    .hierarchy
                    .parent_id
                    .as_ref()
                    .map(DocumentId::as_str),
            ),
        ),
        ("depth", serialized_json_size(&document.hierarchy.depth)),
        (
            "siblingOrder",
            serialized_json_size(&document.hierarchy.sibling_order),
        ),
        ("audioAssetId", projected_locator_size(audio_asset_id)),
        ("markdownSha256", json_string_size(markdown_sha256)),
    ])
}

fn transcript_segment_size(segment: &domain::TranscriptSegment) -> usize {
    json_object_size([
        ("id", json_string_size(&segment.id)),
        ("startMs", serialized_json_size(&segment.start_ms)),
        ("endMs", serialized_json_size(&segment.end_ms)),
        ("text", json_string_size(&segment.text)),
    ])
}

fn speaker_segment_size(segment: &domain::SpeakerTaggedSegment) -> usize {
    json_object_size([
        (
            "transcriptSegmentId",
            json_string_size(&segment.transcript_segment_id),
        ),
        ("speakerId", json_string_size(&segment.speaker_id)),
        (
            "speakerProfileId",
            serialized_json_size(&segment.speaker_profile_id),
        ),
        ("speakerLabel", serialized_json_size(&segment.speaker_label)),
        ("startMs", serialized_json_size(&segment.start_ms)),
        ("endMs", serialized_json_size(&segment.end_ms)),
        ("text", json_string_size(&segment.text)),
    ])
}

fn page_size(page: &contracts::DocumentPage) -> usize {
    json_object_size([
        ("snapshot", json_string_size(&page.snapshot)),
        ("total", serialized_json_size(&page.total)),
        ("nextOffset", serialized_json_size(&page.next_offset)),
    ])
}

fn document_result_size(
    record: &contracts::DocumentRecord,
    include_markdown: bool,
    include_transcript: bool,
    include_speakers: bool,
    page: Option<(&contracts::DocumentPage, std::ops::Range<usize>)>,
) -> usize {
    let mut fields = vec![(
        "metadata",
        summary_size(&record.state, &markdown_sha256(&record.markdown)),
    )];
    if let Some((page, range)) = page {
        fields.push(("page", page_size(page)));
        if include_transcript {
            fields.push((
                "transcript",
                json_array_size(
                    record.state.results.transcript[range.clone()]
                        .iter()
                        .map(transcript_segment_size),
                ),
            ));
        }
        if include_speakers {
            fields.push((
                "speakerSegments",
                json_array_size(
                    record.state.results.speaker_segments[range]
                        .iter()
                        .map(speaker_segment_size),
                ),
            ));
        }
    } else {
        if include_markdown {
            fields.push(("markdown", projected_markdown_size(&record.markdown)));
        }
        if include_transcript {
            fields.push((
                "transcript",
                json_array_size(
                    record
                        .state
                        .results
                        .transcript
                        .iter()
                        .map(transcript_segment_size),
                ),
            ));
        }
        if include_speakers {
            fields.push((
                "speakerSegments",
                json_array_size(
                    record
                        .state
                        .results
                        .speaker_segments
                        .iter()
                        .map(speaker_segment_size),
                ),
            ));
        }
    }
    tagged_result_size("document", json_object_size(fields))
}

fn document_projection_snapshot(
    record: &contracts::DocumentRecord,
    include_transcript: bool,
    include_speakers: bool,
) -> Result<String, AgentToolError> {
    // The snapshot is the hash of the unpaged projection. Stream each field
    // directly into SHA-256 so no full JSON buffer or full segment projection
    // is materialized merely to establish pagination identity.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"{\"metadata\":");
    bytes.extend_from_slice(
        serde_json::to_string(&summarize_with_markdown(&record.state, &record.markdown))
            .map_err(|error| transaction_error(error.to_string()))?
            .as_bytes(),
    );
    if include_transcript {
        bytes.extend_from_slice(b",\"transcript\":");
        bytes.extend_from_slice(
            serde_json::to_vec(
                &record
                    .state
                    .results
                    .transcript
                    .iter()
                    .map(|segment| AgentTranscriptSegment {
                        id: segment.id.clone(),
                        start_ms: segment.start_ms,
                        end_ms: segment.end_ms,
                        text: segment.text.clone(),
                    })
                    .collect::<Vec<_>>(),
            )
            .map_err(|error| transaction_error(error.to_string()))?
            .as_slice(),
        );
    }
    if include_speakers {
        bytes.extend_from_slice(b",\"speakerSegments\":");
        bytes.extend_from_slice(
            serde_json::to_vec(
                &record
                    .state
                    .results
                    .speaker_segments
                    .iter()
                    .map(|segment| AgentSpeakerSegment {
                        transcript_segment_id: segment.transcript_segment_id.clone(),
                        speaker_id: segment.speaker_id.clone(),
                        speaker_profile_id: segment.speaker_profile_id.clone(),
                        speaker_label: segment.speaker_label.clone(),
                        start_ms: segment.start_ms,
                        end_ms: segment.end_ms,
                        text: segment.text.clone(),
                    })
                    .collect::<Vec<_>>(),
            )
            .map_err(|error| transaction_error(error.to_string()))?
            .as_slice(),
        );
    }
    bytes.push(b'}');
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn enforce_result_size_bytes(
    actual_bytes: usize,
    document_id: Option<&DocumentId>,
) -> Result<(), AgentToolError> {
    if actual_bytes <= MAX_CAPABILITY_RESULT_BYTES {
        return Ok(());
    }
    let mut error = AgentToolError::new(
        AgentErrorCode::ContextLimitExceeded,
        format!(
            "capability result exceeds the {} byte limit; use bounded projections or segment pages",
            MAX_CAPABILITY_RESULT_BYTES
        ),
        true,
    );
    error.field = Some("result".into());
    if let Some(document_id) = document_id {
        error.document_id = Some(document_id.as_str().into());
    }
    error.context.insert(
        "maxBytes".into(),
        serde_json::json!(MAX_CAPABILITY_RESULT_BYTES),
    );
    error
        .context
        .insert("actualBytes".into(), serde_json::json!(actual_bytes));
    Err(error)
}

fn invalid_arguments(field: &str, message: impl Into<String>) -> AgentToolError {
    let mut error = AgentToolError::new(AgentErrorCode::ToolArgumentsInvalid, message, true);
    error.field = Some(field.into());
    error
}

fn enforce_result_size(
    result: &AgentToolResult,
    document_id: Option<&DocumentId>,
) -> Result<(), AgentToolError> {
    enforce_result_size_bytes(serialized_json_size(result), document_id)
}

fn validate_document_id_argument(id: &str, field: &str) -> Result<(), AgentToolError> {
    if id.trim().is_empty()
        || id == "."
        || id == ".."
        || id.starts_with('.')
        || id.contains('/')
        || id.contains('\\')
    {
        return Err(invalid_arguments(
            field,
            "document ID must be a non-empty safe package ID",
        ));
    }
    Ok(())
}

fn transaction_error(message: String) -> AgentToolError {
    AgentToolError::new(AgentErrorCode::WriteTransactionFailed, message, false)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn validate_patch(patch: &AgentDocumentPatch) -> Result<(), AgentToolError> {
    if patch
        .title
        .as_ref()
        .is_some_and(|title| title.trim().is_empty())
    {
        return Err(invalid_arguments("patch.title", "title cannot be empty"));
    }
    if patch
        .document_status
        .as_ref()
        .is_some_and(|status| status.trim().is_empty())
    {
        return Err(invalid_arguments(
            "patch.documentStatus",
            "document status cannot be empty",
        ));
    }
    if patch
        .tags
        .as_ref()
        .is_some_and(|tags| tags.iter().any(|tag| tag.trim().is_empty()))
    {
        return Err(invalid_arguments(
            "patch.tags",
            "document tags cannot contain empty values",
        ));
    }
    if patch.title.is_none()
        && patch.tags.is_none()
        && patch.summary.is_none()
        && patch.document_status.is_none()
        && patch.markdown.is_none()
    {
        return Err(invalid_arguments(
            "patch",
            "document patch must change at least one field",
        ));
    }
    Ok(())
}

fn document_error(
    code: AgentErrorCode,
    message: impl Into<String>,
    document_id: &DocumentId,
    retryable: bool,
) -> AgentToolError {
    let mut error = AgentToolError::new(code, message, retryable);
    error.document_id = Some(document_id.as_str().into());
    error
}

fn not_found(document_id: &DocumentId) -> AgentToolError {
    document_error(
        AgentErrorCode::DocumentNotFound,
        "document was not found",
        document_id,
        true,
    )
}

fn segment_position(
    positions: &BTreeMap<&str, usize>,
    segment_id: &str,
    document_id: &DocumentId,
) -> Result<usize, AgentToolError> {
    positions.get(segment_id).copied().ok_or_else(|| {
        let mut error = document_error(
            AgentErrorCode::SegmentNotFound,
            format!("transcript segment was not found: {segment_id}"),
            document_id,
            true,
        );
        error.context.insert(
            "segmentId".into(),
            serde_json::Value::String(segment_id.into()),
        );
        error
    })
}

fn tree_error(error: ProjectTreeError, document_id: &DocumentId) -> AgentToolError {
    let (code, message) = match error {
        ProjectTreeError::ProjectNotFound => {
            (AgentErrorCode::DocumentNotFound, "document was not found")
        }
        ProjectTreeError::ParentNotFound => (
            AgentErrorCode::TreeParentNotFound,
            "tree parent or sibling target was not found",
        ),
        ProjectTreeError::Cycle => (AgentErrorCode::TreeCycle, "tree move would create a cycle"),
        ProjectTreeError::MaximumDepth => (
            AgentErrorCode::TreeMaxDepthExceeded,
            "tree move would exceed the three-level limit",
        ),
        ProjectTreeError::CrossAudioAsset => (
            AgentErrorCode::TreeCrossAudioAsset,
            "tree move would cross audio assets or sibling groups",
        ),
    };
    document_error(code, message, document_id, true)
}

fn position_document(
    documents: &mut [DocumentState],
    document_id: &DocumentId,
    parent_id: Option<&DocumentId>,
    placement: MovePlacement,
    target_id: Option<&DocumentId>,
) -> Result<(), AgentToolError> {
    match (placement, target_id) {
        (MovePlacement::First | MovePlacement::Last, Some(_)) => {
            return Err(invalid_arguments(
                "targetSiblingDocumentId",
                "first/last placement cannot include a sibling target",
            ));
        }
        (MovePlacement::Before | MovePlacement::After, None) => {
            return Err(invalid_arguments(
                "targetSiblingDocumentId",
                "before/after placement requires a sibling target",
            ));
        }
        _ => {}
    }
    if target_id == Some(document_id) {
        return Err(document_error(
            AgentErrorCode::TreeCycle,
            "a document cannot be positioned relative to itself",
            document_id,
            true,
        ));
    }

    let mut siblings: Vec<(u32, DocumentId)> = documents
        .iter()
        .filter(|candidate| candidate.hierarchy.parent_id.as_ref() == parent_id)
        .map(|candidate| (candidate.hierarchy.sibling_order, candidate.id().clone()))
        .collect();
    siblings.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    siblings.retain(|(_, id)| id != document_id);
    let insert_at = match placement {
        MovePlacement::First => 0,
        MovePlacement::Last => siblings.len(),
        MovePlacement::Before | MovePlacement::After => {
            let target_id = target_id.expect("validated target must exist");
            let target_index = siblings
                .iter()
                .position(|(_, id)| id == target_id)
                .ok_or_else(|| {
                    document_error(
                        AgentErrorCode::TreeParentNotFound,
                        "target document is not a sibling under the selected parent",
                        target_id,
                        true,
                    )
                })?;
            target_index + usize::from(placement == MovePlacement::After)
        }
    };
    siblings.insert(insert_at, (0, document_id.clone()));
    for (order, (_, id)) in siblings.into_iter().enumerate() {
        if let Some(document) = documents.iter_mut().find(|candidate| candidate.id() == &id) {
            document.hierarchy.sibling_order = order as u32;
        }
    }
    Ok(())
}

fn render_semantic_child_markdown(
    child: &DocumentState,
    evidence_segment_ids: &[String],
) -> String {
    let mut markdown = format!("# {}\n\n", child.content.title);
    if let Some(summary) = child.content.summary.as_deref() {
        markdown.push_str(summary);
        markdown.push_str("\n\n");
    }
    markdown.push_str("## Transcript\n\n");
    for segment in &child.results.speaker_segments {
        let speaker = segment
            .speaker_label
            .as_deref()
            .unwrap_or(segment.speaker_id.as_str());
        markdown.push_str(&format!(
            "- [{}] {}: {}\n",
            segment.transcript_segment_id, speaker, segment.text
        ));
    }
    if child.results.speaker_segments.is_empty() {
        for segment in &child.results.transcript {
            markdown.push_str(&format!("- [{}] {}\n", segment.id, segment.text));
        }
    }
    if !evidence_segment_ids.is_empty() {
        markdown.push_str("\n## Evidence\n\n");
        for segment_id in evidence_segment_ids {
            markdown.push_str(&format!("- {segment_id}\n"));
        }
    }
    markdown
}

fn map_store_error(error: DocumentMutationStoreError) -> AgentToolError {
    match error {
        DocumentMutationStoreError::NotFound { document_id } => document_error(
            AgentErrorCode::DocumentNotFound,
            "document was not found during transaction",
            &DocumentId::new(document_id),
            true,
        ),
        DocumentMutationStoreError::AlreadyExists { document_id } => document_error(
            AgentErrorCode::WriteTransactionFailed,
            "a document with the generated ID already exists",
            &DocumentId::new(document_id),
            true,
        ),
        DocumentMutationStoreError::RevisionConflict {
            document_id,
            expected,
            actual,
        } => {
            let mut error = document_error(
                AgentErrorCode::DocumentRevisionConflict,
                "document changed before the transaction was committed",
                &DocumentId::new(document_id),
                true,
            );
            error.expected_revision = Some(expected);
            error.actual_revision = Some(actual);
            error
        }
        DocumentMutationStoreError::ContentConflict {
            document_id,
            expected,
            actual,
        } => {
            let mut error = document_error(
                AgentErrorCode::DocumentRevisionConflict,
                "document Markdown changed before the transaction was committed",
                &DocumentId::new(document_id),
                true,
            );
            error
                .context
                .insert("expectedMarkdownSha256".into(), serde_json::json!(expected));
            error
                .context
                .insert("actualMarkdownSha256".into(), serde_json::json!(actual));
            error
        }
        DocumentMutationStoreError::IdempotencyConflict => AgentToolError::new(
            AgentErrorCode::IdempotencyConflict,
            "request ID was reused with different arguments",
            false,
        ),
        DocumentMutationStoreError::UndoNotFound => AgentToolError::new(
            AgentErrorCode::WriteTransactionFailed,
            "undo token was not found or already consumed",
            false,
        ),
        DocumentMutationStoreError::Transaction(message) => {
            AgentToolError::new(AgentErrorCode::WriteTransactionFailed, message, false)
        }
    }
}

fn mutation_result(
    commit: DocumentMutationCommit,
    uncovered_segment_ids: Vec<String>,
) -> Result<AgentToolResult, AgentToolError> {
    let affected_size = json_array_size(commit.revisions.iter().map(|revision| {
        json_object_size([
            ("documentId", json_string_size(&revision.document_id)),
            (
                "previousRevision",
                serialized_json_size(&revision.previous_revision),
            ),
            ("revision", serialized_json_size(&revision.revision)),
            (
                "deleted",
                serialized_json_size(&revision.revision.is_none()),
            ),
        ])
    }));
    let payload = json_object_size([
        ("mutationId", json_string_size(&commit.mutation_id)),
        ("affectedDocuments", affected_size),
        ("undoToken", json_string_size(&commit.undo_token)),
        ("provenance", serialized_json_size(&commit.provenance)),
        (
            "uncoveredSegmentIds",
            serialized_json_size(&uncovered_segment_ids),
        ),
    ]);
    enforce_result_size_bytes(tagged_result_size("mutation", payload), None)?;
    Ok(AgentToolResult::Mutation(AgentMutationReceipt {
        mutation_id: commit.mutation_id,
        affected_documents: commit
            .revisions
            .into_iter()
            .map(|revision| AgentDocumentRevision {
                document_id: revision.document_id,
                previous_revision: revision.previous_revision,
                revision: revision.revision,
                deleted: revision.revision.is_none(),
            })
            .collect(),
        undo_token: commit.undo_token,
        provenance: commit.provenance,
        uncovered_segment_ids,
    }))
}

fn summarize(document: &DocumentState) -> AgentDocumentSummary {
    AgentDocumentSummary {
        document_id: document.id().as_str().into(),
        revision: document.revision(),
        title: document.content.title.clone(),
        tags: document.content.tags.clone(),
        summary: document.content.summary.clone(),
        document_status: document.document_status.value.clone(),
        parent_document_id: document
            .hierarchy
            .parent_id
            .as_ref()
            .map(|id| id.as_str().to_string()),
        depth: document.hierarchy.depth,
        sibling_order: document.hierarchy.sibling_order,
        audio_asset_id: document.source.audio_asset_id.as_ref().map_or_else(
            || redact_local_locator_owned(document.id().as_str()),
            |id| redact_local_locator_owned(id.as_str()),
        ),
        markdown_sha256: String::new(),
    }
}

fn summarize_metadata(metadata: &DocumentMetadata) -> AgentDocumentSummary {
    AgentDocumentSummary {
        document_id: metadata.document_id.as_str().into(),
        revision: metadata.revision,
        title: metadata.title.clone(),
        tags: metadata.tags.clone(),
        summary: metadata.summary.clone(),
        document_status: metadata.document_status.clone(),
        parent_document_id: metadata
            .parent_id
            .as_ref()
            .map(|id| id.as_str().to_string()),
        depth: metadata.depth,
        sibling_order: metadata.sibling_order,
        audio_asset_id: metadata.audio_asset_id.as_ref().map_or_else(
            || redact_local_locator_owned(metadata.document_id.as_str()),
            |id| redact_local_locator_owned(id.as_str()),
        ),
        markdown_sha256: String::new(),
    }
}

fn summarize_with_markdown(document: &DocumentState, markdown: &str) -> AgentDocumentSummary {
    let mut summary = summarize(document);
    summary.markdown_sha256 = markdown_sha256(markdown);
    summary
}

fn markdown_sha256(markdown: &str) -> String {
    Sha256::digest(markdown.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn ensure_markdown_hash(
    document_id: &DocumentId,
    markdown: &str,
    expected: Option<&str>,
) -> Result<(), AgentToolError> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let actual = markdown_sha256(markdown);
    if actual == expected {
        return Ok(());
    }
    let mut error = document_error(
        AgentErrorCode::DocumentRevisionConflict,
        "document Markdown changed outside the capability transaction",
        document_id,
        true,
    );
    error
        .context
        .insert("expectedMarkdownSha256".into(), serde_json::json!(expected));
    error
        .context
        .insert("actualMarkdownSha256".into(), serde_json::json!(actual));
    Err(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DocumentMutationRevision, DocumentRepository, DocumentSnapshot, PendingDocumentMutation,
    };
    use contracts::{AgentScene, AgentSkillSelector};
    use contracts::{AgentScopeSelector, DocumentRecord};
    use domain::{DiarizationSegment, DocumentState, SpeakerTaggedSegment, TranscriptSegment};
    use std::collections::BTreeMap;
    use std::sync::{mpsc, Arc, Mutex};

    #[derive(Clone)]
    struct StoredMutation {
        fingerprint: String,
        commit: DocumentMutationCommit,
        before: Vec<(String, Option<DocumentRecord>)>,
        undone: bool,
        undo_commit: Option<DocumentMutationCommit>,
    }

    #[derive(Default)]
    struct Repository {
        state: Mutex<RepositoryState>,
    }

    #[derive(Default)]
    struct RepositoryState {
        records: BTreeMap<String, DocumentRecord>,
        mutations: BTreeMap<String, StoredMutation>,
        pending: BTreeMap<String, PendingDocumentMutation>,
    }

    impl DocumentRepository for Repository {
        fn snapshot(&self) -> Result<DocumentSnapshot, String> {
            Ok(DocumentSnapshot {
                documents: self
                    .state
                    .lock()
                    .unwrap()
                    .records
                    .values()
                    .map(|record| record.state.clone())
                    .collect(),
                issues: Vec::new(),
            })
        }

        fn get(&self, id: &DocumentId) -> Result<Option<DocumentState>, String> {
            Ok(self
                .state
                .lock()
                .unwrap()
                .records
                .get(id.as_str())
                .map(|record| record.state.clone()))
        }

        fn read(&self, id: &DocumentId) -> Result<Option<DocumentRecord>, String> {
            Ok(self.state.lock().unwrap().records.get(id.as_str()).cloned())
        }

        fn watch(&self) -> Result<mpsc::Receiver<crate::DocumentChange>, String> {
            let (_sender, receiver) = mpsc::channel();
            Ok(receiver)
        }

        fn save(&self, state: DocumentState, markdown: Option<String>) -> Result<(), String> {
            self.state.lock().unwrap().records.insert(
                state.id().as_str().into(),
                DocumentRecord {
                    state,
                    markdown: markdown.unwrap_or_default(),
                },
            );
            Ok(())
        }

        fn find_agent_mutation(
            &self,
            mutation_id: &str,
            fingerprint: &str,
        ) -> Result<Option<DocumentMutationCommit>, DocumentMutationStoreError> {
            let state = self.state.lock().unwrap();
            let Some(stored) = state.mutations.get(mutation_id) else {
                return Ok(None);
            };
            if stored.fingerprint != fingerprint || stored.undone {
                return Err(DocumentMutationStoreError::IdempotencyConflict);
            }
            let mut commit = stored.commit.clone();
            commit.replayed = true;
            Ok(Some(commit))
        }

        fn commit_agent_mutation(
            &self,
            mut batch: DocumentMutationBatch,
        ) -> Result<DocumentMutationCommit, DocumentMutationStoreError> {
            let mut state = self.state.lock().unwrap();
            if let Some(stored) = state.mutations.get(&batch.mutation_id) {
                if stored.fingerprint != batch.fingerprint || stored.undone {
                    return Err(DocumentMutationStoreError::IdempotencyConflict);
                }
                let mut commit = stored.commit.clone();
                commit.replayed = true;
                return Ok(commit);
            }
            for write in &batch.writes {
                let id = write.record.state.id().as_str();
                match (write.expected_revision, state.records.get(id)) {
                    (Some(expected), Some(current)) if current.state.revision() != expected => {
                        return Err(DocumentMutationStoreError::RevisionConflict {
                            document_id: id.into(),
                            expected,
                            actual: current.state.revision(),
                        });
                    }
                    (Some(_), None) => {
                        return Err(DocumentMutationStoreError::NotFound {
                            document_id: id.into(),
                        });
                    }
                    (None, Some(_)) => {
                        return Err(DocumentMutationStoreError::AlreadyExists {
                            document_id: id.into(),
                        });
                    }
                    _ => {}
                }
            }
            let before = batch
                .writes
                .iter()
                .map(|write| {
                    let id = write.record.state.id().as_str().to_string();
                    (id.clone(), state.records.get(&id).cloned())
                })
                .collect::<Vec<_>>();
            batch.provenance.applied_at_ms = now_ms();
            let commit = DocumentMutationCommit {
                mutation_id: batch.mutation_id.clone(),
                undo_token: batch.undo_token.clone(),
                records: batch
                    .writes
                    .iter()
                    .map(|write| write.record.clone())
                    .collect(),
                revisions: batch
                    .writes
                    .iter()
                    .map(|write| DocumentMutationRevision {
                        document_id: write.record.state.id().as_str().into(),
                        previous_revision: write.expected_revision,
                        revision: Some(write.record.state.revision()),
                    })
                    .collect(),
                provenance: batch.provenance,
                replayed: false,
            };
            for record in &commit.records {
                state
                    .records
                    .insert(record.state.id().as_str().into(), record.clone());
            }
            state.mutations.insert(
                commit.mutation_id.clone(),
                StoredMutation {
                    fingerprint: batch.fingerprint,
                    commit: commit.clone(),
                    before,
                    undone: false,
                    undo_commit: None,
                },
            );
            Ok(commit)
        }

        fn agent_mutation_document_ids(
            &self,
            undo_token: &str,
        ) -> Result<Vec<String>, DocumentMutationStoreError> {
            let state = self.state.lock().unwrap();
            let stored = state
                .mutations
                .values()
                .find(|stored| stored.commit.undo_token == undo_token)
                .ok_or(DocumentMutationStoreError::UndoNotFound)?;
            Ok(stored
                .commit
                .revisions
                .iter()
                .filter(|revision| revision.previous_revision.is_some())
                .map(|revision| revision.document_id.clone())
                .collect())
        }

        fn undo_agent_mutation(
            &self,
            undo_token: &str,
        ) -> Result<DocumentMutationCommit, DocumentMutationStoreError> {
            let mut state = self.state.lock().unwrap();
            let mutation_id = state
                .mutations
                .iter()
                .find(|(_, stored)| stored.commit.undo_token == undo_token)
                .map(|(id, _)| id.clone())
                .ok_or(DocumentMutationStoreError::UndoNotFound)?;
            let stored = state.mutations.get(&mutation_id).unwrap().clone();
            if let Some(mut commit) = stored.undo_commit {
                commit.replayed = true;
                return Ok(commit);
            }
            let mut records = Vec::new();
            let mut revisions = Vec::new();
            for (id, before) in stored.before {
                let current_revision = state
                    .records
                    .get(&id)
                    .ok_or_else(|| DocumentMutationStoreError::NotFound {
                        document_id: id.clone(),
                    })?
                    .state
                    .revision();
                match before {
                    Some(mut record) => {
                        record.state.schema.revision = current_revision + 1;
                        state.records.insert(id.clone(), record.clone());
                        revisions.push(DocumentMutationRevision {
                            document_id: id,
                            previous_revision: Some(current_revision),
                            revision: Some(record.state.revision()),
                        });
                        records.push(record);
                    }
                    None => {
                        state.records.remove(&id);
                        revisions.push(DocumentMutationRevision {
                            document_id: id,
                            previous_revision: Some(current_revision),
                            revision: None,
                        });
                    }
                }
            }
            let commit = DocumentMutationCommit {
                mutation_id: format!("undo:{undo_token}"),
                undo_token: undo_token.into(),
                records,
                revisions,
                provenance: AgentMutationProvenance {
                    actor: AgentActor::User,
                    mutation_id: format!("undo:{undo_token}"),
                    skill: Some(AgentSkillSelector {
                        id: "undo".into(),
                        version: 1,
                    }),
                    connection_id: None,
                    protocol: None,
                    requested_model: None,
                    resolved_model: None,
                    source_revisions: Vec::new(),
                    applied_at_ms: now_ms(),
                },
                replayed: false,
            };
            let stored = state.mutations.get_mut(&mutation_id).unwrap();
            stored.undone = true;
            stored.undo_commit = Some(commit.clone());
            Ok(commit)
        }

        fn save_pending_agent_mutation(
            &self,
            pending: PendingDocumentMutation,
        ) -> Result<(), DocumentMutationStoreError> {
            let mut state = self.state.lock().unwrap();
            if state.pending.contains_key(&pending.confirmation_id) {
                return Err(DocumentMutationStoreError::IdempotencyConflict);
            }
            state
                .pending
                .insert(pending.confirmation_id.clone(), pending);
            Ok(())
        }

        fn take_pending_agent_mutation(
            &self,
            confirmation_id: &str,
        ) -> Result<Option<PendingDocumentMutation>, DocumentMutationStoreError> {
            Ok(self.state.lock().unwrap().pending.remove(confirmation_id))
        }

        fn find_pending_agent_mutation(
            &self,
            mutation_id: &str,
            fingerprint: &str,
        ) -> Result<Option<PendingDocumentMutation>, DocumentMutationStoreError> {
            Ok(self
                .state
                .lock()
                .unwrap()
                .pending
                .values()
                .find(|pending| {
                    pending.batch.mutation_id == mutation_id
                        && pending.batch.fingerprint == fingerprint
                })
                .cloned())
        }
    }

    fn gateway() -> (WorkspaceCapabilityPort, Arc<Repository>) {
        let repository = Arc::new(Repository::default());
        let mut allowed = DocumentState::new(DocumentId::new("allowed"), "Allowed", "a.wav");
        allowed.source.audio_end_ms = Some(4_000);
        for index in 0..4 {
            let id = format!("seg-{}", index + 1);
            allowed.results.transcript.push(TranscriptSegment {
                id: id.clone(),
                start_ms: index * 1_000,
                end_ms: (index + 1) * 1_000,
                text: format!("Fact {}", index + 1),
            });
            allowed.results.diarization.push(DiarizationSegment {
                id: format!("dia-{}", index + 1),
                speaker_id: "speaker-1".into(),
                speaker_profile_id: None,
                speaker_label: Some("Speaker".into()),
                start_ms: index * 1_000,
                end_ms: (index + 1) * 1_000,
                confidence: Some(1.0),
            });
            allowed.results.speaker_segments.push(SpeakerTaggedSegment {
                transcript_segment_id: id,
                speaker_id: "speaker-1".into(),
                speaker_profile_id: None,
                speaker_label: Some("Speaker".into()),
                start_ms: index * 1_000,
                end_ms: (index + 1) * 1_000,
                text: format!("Fact {}", index + 1),
            });
        }
        repository
            .save(allowed, Some("# Allowed\n".into()))
            .unwrap();
        repository
            .save(
                DocumentState::new(DocumentId::new("denied"), "Denied", "b.wav"),
                Some("# Denied\n".into()),
            )
            .unwrap();
        (
            WorkspaceCapabilityPort::new(
                DocumentService::new(repository.clone()),
                WorkspaceAccessPolicy::documents([DocumentId::new("allowed")]),
            ),
            repository,
        )
    }

    fn request(
        id: &str,
        _scene: AgentScene,
        mut arguments: serde_json::Value,
    ) -> CapabilityRequest {
        if let Some(object) = arguments.as_object_mut() {
            let is_mutation = object.contains_key("patch") || object.contains_key("children");
            if is_mutation && !object.contains_key("sourceMarkdownSha256") {
                object.insert(
                    "sourceMarkdownSha256".into(),
                    serde_json::Value::String(markdown_sha256("# Allowed\n")),
                );
            }
        }
        CapabilityRequest::new(
            id,
            format!("call-{id}"),
            AgentScopeSelector::default(),
            arguments,
        )
    }

    fn error_code(response: CapabilityResponse) -> AgentErrorCode {
        response.error.as_ref().expect("expected tool error").code
    }

    fn mutation_receipt(response: CapabilityResponse) -> AgentMutationReceipt {
        assert!(response.ok, "{response:?}");
        match response.result.as_ref().expect("expected tool result") {
            AgentToolResult::Mutation(receipt) => receipt.clone(),
            other => panic!("expected mutation receipt, got {other:?}"),
        }
    }

    #[test]
    fn list_and_all_read_projections_share_the_gateway_scope() {
        let (gateway, _) = gateway();
        let list = gateway.execute(
            LIST_DOCUMENTS_TOOL,
            CapabilityRequest::new(
                "list-1",
                "call-1",
                AgentScopeSelector::default(),
                serde_json::json!({}),
            ),
        );
        let AgentToolResult::Documents { documents } = list.result.as_ref().unwrap() else {
            panic!("expected documents");
        };
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].document_id, "allowed");

        for projection in [
            "metadata",
            "markdown",
            "transcript",
            "speaker_segments",
            "complete",
        ] {
            let response = gateway.execute(
                READ_DOCUMENT_TOOL,
                CapabilityRequest::new(
                    format!("read-{projection}"),
                    "call-read",
                    AgentScopeSelector::default(),
                    serde_json::json!({
                        "documentId": "allowed",
                        "projection": projection
                    }),
                ),
            );
            assert!(response.ok, "projection {projection}: {response:?}");
        }
    }

    #[test]
    fn capability_boundary_projects_initial_follow_up_and_cli_wire_outputs() {
        let (gateway, repository) = gateway();
        let mut record = repository
            .read(&DocumentId::new("allowed"))
            .unwrap()
            .unwrap();
        record.state.source.audio_asset_id = Some(DocumentId::new("/Users/example/private.wav"));
        record.markdown = "# Allowed\n\nAudio: `/Users/example/private.wav`\n".into();
        repository
            .save(record.state, Some(record.markdown))
            .unwrap();

        let response = gateway.execute(
            READ_DOCUMENT_TOOL,
            request(
                "projection-read",
                AgentScene::Ask,
                serde_json::json!({
                    "documentId": "allowed",
                    "projection": "markdown"
                }),
            ),
        );
        let json = serde_json::to_string(&response).unwrap();
        assert!(response.ok, "{response:?}");
        assert!(!json.contains("/Users/example/private.wav"));
        let Some(AgentToolResult::Document(document)) = response.result.as_ref() else {
            panic!("expected document projection");
        };
        assert_eq!(document.metadata.audio_asset_id, "[redacted-local-path]");
        assert_eq!(
            document.markdown.as_deref(),
            Some("# Allowed\n\nAudio: `[local audio asset]`")
        );

        let pending_gateway = gateway.with_write_mode(AgentWriteMode::ConfirmEach);
        let pending = pending_gateway.execute(
            UPDATE_DOCUMENT_TOOL,
            request(
                "projection-pending",
                AgentScene::Organize,
                serde_json::json!({
                    "documentId": "allowed",
                    "expectedRevision": 0,
                    "sourceMarkdownSha256": markdown_sha256(
                        "# Allowed\n\nAudio: `/Users/example/private.wav`\n",
                    ),
                    "patch": {
                        "markdown": "Audio: `/Users/example/next-private.wav`"
                    }
                }),
            ),
        );
        let pending_json = serde_json::to_string(&pending).unwrap();
        assert!(pending.ok, "{pending:?}");
        assert!(!pending_json.contains("/Users/example/next-private.wav"));
        let Some(AgentToolResult::Pending(pending)) = pending.result.as_ref() else {
            panic!("expected pending mutation");
        };
        let markdown_diffs = pending
            .field_diffs
            .iter()
            .filter(|diff| diff.field == "markdown")
            .collect::<Vec<_>>();
        assert_eq!(markdown_diffs.len(), 1);
        assert_eq!(
            markdown_diffs[0].after.as_deref(),
            Some("Audio: `[local audio asset]`")
        );
    }

    #[test]
    fn capability_projection_policy_redacts_paths_without_redacting_regular_asset_ids() {
        let mut result = AgentToolResult::Documents {
            documents: vec![AgentDocumentSummary {
                document_id: "doc".into(),
                revision: 1,
                title: "Doc".into(),
                tags: Vec::new(),
                summary: None,
                document_status: "ready".into(),
                parent_document_id: None,
                depth: 0,
                sibling_order: 0,
                audio_asset_id: "asset-123".into(),
                markdown_sha256: String::new(),
            }],
        };
        CapabilityProjectionPolicy::project_result(&mut result);
        let AgentToolResult::Documents { documents } = result else {
            panic!("expected document summaries");
        };
        assert_eq!(documents[0].audio_asset_id, "asset-123");

        let mut result = AgentToolResult::Documents {
            documents: vec![AgentDocumentSummary {
                audio_asset_id: "C:\\Users\\example\\private.wav".into(),
                ..documents[0].clone()
            }],
        };
        CapabilityProjectionPolicy::project_result(&mut result);
        let AgentToolResult::Documents { documents } = result else {
            panic!("expected document summaries");
        };
        assert_eq!(documents[0].audio_asset_id, "[redacted-local-path]");

        let mut error = AgentToolError::new(
            AgentErrorCode::WriteTransactionFailed,
            "cannot open /Users/example/private.wav",
            false,
        );
        error
            .context
            .insert("apiKey".into(), serde_json::json!("super-secret"));
        error.context.insert(
            "path".into(),
            serde_json::json!("/Users/example/private.wav"),
        );
        let projected = CapabilityProjectionPolicy::project_response(CapabilityResponse::failure(
            "error-projection",
            error,
        ));
        let error = projected.error.as_ref().unwrap();
        assert!(!error.message.contains("/Users/example/private.wav"));
        assert_eq!(
            error.context["apiKey"],
            serde_json::json!("[redacted-secret]")
        );
        assert_eq!(
            error.context["path"],
            serde_json::json!("[redacted-local-path]")
        );
    }

    #[test]
    fn update_detects_external_markdown_changes_when_source_hash_is_supplied() {
        let (gateway, repository) = gateway();
        let original = repository
            .read(&DocumentId::new("allowed"))
            .unwrap()
            .unwrap();
        let source_hash = markdown_sha256(&original.markdown);
        let first = gateway.execute(
            UPDATE_DOCUMENT_TOOL,
            request(
                "hash-update",
                AgentScene::Organize,
                serde_json::json!({
                    "documentId": "allowed",
                    "expectedRevision": 0,
                    "sourceMarkdownSha256": source_hash,
                    "patch": {"title": "Hash checked"}
                }),
            ),
        );
        assert!(first.ok, "{first:?}");
        let mut external = repository
            .read(&DocumentId::new("allowed"))
            .unwrap()
            .unwrap();
        external.markdown = "# External edit\n".into();
        repository
            .save(external.state, Some(external.markdown))
            .unwrap();
        let stale = gateway.execute(
            UPDATE_DOCUMENT_TOOL,
            request(
                "hash-stale",
                AgentScene::Organize,
                serde_json::json!({
                    "documentId": "allowed",
                    "expectedRevision": 1,
                    "sourceMarkdownSha256": source_hash,
                    "patch": {"title": "Must reject"}
                }),
            ),
        );
        assert_eq!(error_code(stale), AgentErrorCode::DocumentRevisionConflict);
    }

    #[test]
    fn denied_and_malformed_reads_return_stable_errors() {
        let (gateway, _) = gateway();
        let denied = gateway.execute(
            READ_DOCUMENT_TOOL,
            CapabilityRequest::new(
                "denied",
                "call-denied",
                AgentScopeSelector::default(),
                serde_json::json!({"documentId": "denied", "projection": "metadata"}),
            ),
        );
        assert_eq!(
            denied.error.as_ref().unwrap().code,
            AgentErrorCode::DocumentAccessDenied
        );

        let malformed = gateway.execute(
            READ_DOCUMENT_TOOL,
            CapabilityRequest::new(
                "malformed",
                "call-malformed",
                AgentScopeSelector::default(),
                serde_json::json!({
                    "documentId": "allowed",
                    "projection": "metadata",
                    "processing": true
                }),
            ),
        );
        assert_eq!(
            malformed.error.as_ref().unwrap().code,
            AgentErrorCode::ToolArgumentsInvalid
        );
    }

    #[test]
    fn bounded_reads_preserve_ids_and_reject_changed_snapshots() {
        let (port, repository) = gateway();
        let call = |id: &str, page: serde_json::Value| {
            port.execute(READ_DOCUMENT_TOOL,
            request(id, AgentScene::Ask, serde_json::json!({"documentId":"allowed","projection":"transcript","page":page})))
        };
        let first = call(
            "first",
            serde_json::json!({"offset":0,"limit":2,"snapshot":null}),
        );
        let AgentToolResult::Document(first) = first.result.as_ref().unwrap() else {
            panic!()
        };
        assert_eq!(first.transcript.as_ref().unwrap().len(), 2);
        let page = first.page.as_ref().unwrap();
        assert_eq!(page.next_offset, Some(2));
        let second = call(
            "second",
            serde_json::json!({"offset":2,"limit":2,"snapshot":page.snapshot}),
        );
        let AgentToolResult::Document(second) = second.result.as_ref().unwrap() else {
            panic!()
        };
        assert_ne!(
            first.transcript.as_ref().unwrap()[0].id,
            second.transcript.as_ref().unwrap()[0].id
        );
        let mut record = repository
            .read(&DocumentId::new("allowed"))
            .unwrap()
            .unwrap();
        record.state.results.transcript[0].text = "external edit without revision".into();
        repository
            .save(record.state, Some(record.markdown))
            .unwrap();
        let stale = call(
            "stale",
            serde_json::json!({"offset":2,"limit":2,"snapshot":page.snapshot}),
        );
        assert_eq!(error_code(stale), AgentErrorCode::DocumentRevisionConflict);
        assert_eq!(
            error_code(call(
                "limit",
                serde_json::json!({"offset":0,"limit":257,"snapshot":null})
            )),
            AgentErrorCode::ToolArgumentsInvalid
        );
    }

    #[test]
    fn read_projections_have_a_fixed_result_byte_limit() {
        let (gateway, repository) = gateway();
        let mut record = repository
            .read(&DocumentId::new("allowed"))
            .unwrap()
            .unwrap();
        let oversized_markdown = "x".repeat(MAX_CAPABILITY_RESULT_BYTES + 1);
        repository
            .save(record.state, Some(oversized_markdown))
            .unwrap();

        let response = gateway.execute(
            READ_DOCUMENT_TOOL,
            request(
                "oversized-read",
                AgentScene::Ask,
                serde_json::json!({
                    "documentId": "allowed",
                    "projection": "markdown"
                }),
            ),
        );
        assert_eq!(
            error_code(response.clone()),
            AgentErrorCode::ContextLimitExceeded
        );
        let error = response.error.as_ref().unwrap();
        assert_eq!(error.document_id.as_deref(), Some("allowed"));
        assert_eq!(
            error.context["maxBytes"],
            serde_json::json!(MAX_CAPABILITY_RESULT_BYTES)
        );
        assert!(
            error.context["actualBytes"].as_u64().unwrap() > MAX_CAPABILITY_RESULT_BYTES as u64
        );

        record = repository
            .read(&DocumentId::new("allowed"))
            .unwrap()
            .unwrap();
        assert_eq!(record.markdown.len(), MAX_CAPABILITY_RESULT_BYTES + 1);
    }

    #[test]
    fn segment_pages_can_read_large_transcripts_without_returning_the_whole_result() {
        let (gateway, repository) = gateway();
        let mut record = repository
            .read(&DocumentId::new("allowed"))
            .unwrap()
            .unwrap();
        record.state.results.transcript = (0..4_000)
            .map(|index| TranscriptSegment {
                id: format!("large-{index}"),
                start_ms: index * 1_000,
                end_ms: (index + 1) * 1_000,
                text: "bounded transcript text ".repeat(16),
            })
            .collect();
        repository
            .save(record.state, Some("# Large transcript\n".into()))
            .unwrap();

        let response = gateway.execute(
            READ_DOCUMENT_TOOL,
            request(
                "large-page",
                AgentScene::Ask,
                serde_json::json!({
                    "documentId": "allowed",
                    "projection": "transcript",
                    "page": {"offset": 0, "limit": 1, "snapshot": null}
                }),
            ),
        );
        assert!(response.ok, "{response:?}");
        let AgentToolResult::Document(projection) = response.result.as_ref().unwrap() else {
            panic!("expected document projection");
        };
        assert_eq!(projection.transcript.as_ref().unwrap().len(), 1);
        assert_eq!(projection.page.as_ref().unwrap().total, 4_000);
    }

    #[test]
    fn list_results_use_the_same_fixed_result_byte_limit() {
        let (gateway, repository) = gateway();
        let mut record = repository
            .read(&DocumentId::new("allowed"))
            .unwrap()
            .unwrap();
        record.state.content.summary = Some("summary ".repeat(MAX_CAPABILITY_RESULT_BYTES));
        repository
            .save(record.state, Some("# Allowed\n".into()))
            .unwrap();

        let response = gateway.execute(
            LIST_DOCUMENTS_TOOL,
            CapabilityRequest::new(
                "oversized-list",
                "call-oversized-list",
                AgentScopeSelector::default(),
                serde_json::json!({}),
            ),
        );
        assert_eq!(error_code(response), AgentErrorCode::ContextLimitExceeded);
    }

    #[test]
    fn capability_grants_are_independent_of_skills_and_can_only_shrink() {
        let (port, _) = gateway();
        let restricted = port
            .read_only()
            .restrict_tools([UPDATE_DOCUMENT_TOOL.into(), READ_DOCUMENT_TOOL.into()]);
        assert_eq!(restricted.describe().tools.len(), 1);
        assert_eq!(
            error_code(restricted.undo("unknown", "undo")),
            AgentErrorCode::ToolNotAllowed
        );
        assert_eq!(
            error_code(restricted.confirm("unknown", "confirm")),
            AgentErrorCode::ToolNotAllowed
        );
        let response = restricted.execute(UPDATE_DOCUMENT_TOOL, request("denied-write",
            AgentScene::Organize, serde_json::json!({"documentId":"allowed","expectedRevision":0,"patch":{"title":"no"}})));
        assert_eq!(error_code(response), AgentErrorCode::ToolNotAllowed);
        let call: CapabilityRequest = serde_json::from_value(serde_json::json!({
            "schemaVersion":1,"requestId":"read","toolCallId":"read",
            "scope":{},"arguments":{"documentId":"allowed","projection":"metadata"}
        }))
        .unwrap();
        assert!(restricted.execute(READ_DOCUMENT_TOOL, call).ok);
    }

    #[test]
    fn create_document_accepts_only_granted_assets_or_explicit_no_audio() {
        let (gateway, repository) = gateway();
        let no_audio = request(
            "create-no-audio",
            AgentScene::Organize,
            serde_json::json!({
                "title": "Notes",
                "tags": ["manual"],
                "summary": "A manually created document",
                "markdown": "# Notes\n",
                "source": {"type": "no_audio"}
            }),
        );
        let receipt = mutation_receipt(gateway.execute(CREATE_DOCUMENT_TOOL, no_audio.clone()));
        assert_eq!(receipt.affected_documents.len(), 1);
        let replay = mutation_receipt(gateway.execute(CREATE_DOCUMENT_TOOL, no_audio));
        assert_eq!(replay.mutation_id, receipt.mutation_id);
        let created = repository
            .snapshot()
            .unwrap()
            .documents
            .into_iter()
            .find(|document| document.title() == "Notes")
            .expect("created document");
        assert!(created.audio_path().is_empty());
        assert!(created.source.audio_asset_id.is_none());
        assert_eq!(created.content.tags, ["manual"]);

        let audio = gateway.execute(
            CREATE_DOCUMENT_TOOL,
            request(
                "create-audio",
                AgentScene::Organize,
                serde_json::json!({
                    "title": "Audio notes",
                    "markdown": "# Audio notes\n",
                    "source": {"type": "audio_asset", "assetId": "allowed"}
                }),
            ),
        );
        assert!(audio.ok, "{audio:?}");
        let audio_document = repository
            .snapshot()
            .unwrap()
            .documents
            .into_iter()
            .find(|document| document.title() == "Audio notes")
            .expect("audio-backed document");
        assert_eq!(audio_document.audio_path(), "a.wav");
        assert_eq!(
            audio_document
                .source
                .audio_asset_id
                .as_ref()
                .map(DocumentId::as_str),
            Some("allowed")
        );

        let denied = gateway.execute(
            CREATE_DOCUMENT_TOOL,
            request(
                "create-denied-asset",
                AgentScene::Organize,
                serde_json::json!({
                    "title": "Denied",
                    "markdown": "# Denied\n",
                    "source": {"type": "audio_asset", "assetId": "denied"}
                }),
            ),
        );
        assert_eq!(error_code(denied), AgentErrorCode::DocumentAccessDenied);
    }

    #[test]
    fn create_document_validates_parent_revision_depth_and_audio_identity() {
        let (gateway, repository) = gateway();
        let mismatch = gateway.execute(
            CREATE_DOCUMENT_TOOL,
            request(
                "create-mismatch",
                AgentScene::Organize,
                serde_json::json!({
                    "title": "Mismatch",
                    "parentDocumentId": "allowed",
                    "expectedParentRevision": 0,
                    "sourceMarkdownSha256": "5dce6514e74b5d67956735f225b4bad042eb791adb7f0baa7764970f5adfb457",
                    "markdown": "# Mismatch\n",
                    "source": {"type": "no_audio"}
                }),
            ),
        );
        assert_eq!(error_code(mismatch), AgentErrorCode::TreeCrossAudioAsset);

        let stale_revision = gateway.execute(
            CREATE_DOCUMENT_TOOL,
            request(
                "create-stale-revision",
                AgentScene::Organize,
                serde_json::json!({
                    "title": "Stale revision",
                    "parentDocumentId": "allowed",
                    "expectedParentRevision": 1,
                    "sourceMarkdownSha256": "5dce6514e74b5d67956735f225b4bad042eb791adb7f0baa7764970f5adfb457",
                    "markdown": "# Stale revision\n",
                    "source": {"type": "audio_asset", "assetId": "allowed"}
                }),
            ),
        );
        assert_eq!(
            error_code(stale_revision),
            AgentErrorCode::DocumentRevisionConflict
        );

        let child = gateway.execute(
            CREATE_DOCUMENT_TOOL,
            request(
                "create-child",
                AgentScene::Organize,
                serde_json::json!({
                    "title": "Child",
                    "parentDocumentId": "allowed",
                    "expectedParentRevision": 0,
                    "sourceMarkdownSha256": "5dce6514e74b5d67956735f225b4bad042eb791adb7f0baa7764970f5adfb457",
                    "markdown": "# Child\n",
                    "source": {"type": "audio_asset", "assetId": "allowed"}
                }),
            ),
        );
        assert!(child.ok, "{child:?}");
        let child_state = repository
            .snapshot()
            .unwrap()
            .documents
            .into_iter()
            .find(|document| document.title() == "Child")
            .expect("created child");
        assert_eq!(
            child_state
                .hierarchy
                .parent_id
                .as_ref()
                .map(DocumentId::as_str),
            Some("allowed")
        );
        assert_eq!(child_state.hierarchy.depth, 1);
        assert_eq!(
            repository
                .read(&DocumentId::new("allowed"))
                .unwrap()
                .unwrap()
                .state
                .revision(),
            1
        );

        let (depth_gateway, depth_repository) = self::gateway();
        depth_repository
            .state
            .lock()
            .unwrap()
            .records
            .get_mut("allowed")
            .unwrap()
            .state
            .hierarchy
            .depth = 2;
        let too_deep = depth_gateway.execute(
            CREATE_DOCUMENT_TOOL,
            request(
                "create-too-deep",
                AgentScene::Organize,
                serde_json::json!({
                    "title": "Too deep",
                    "parentDocumentId": "allowed",
                    "expectedParentRevision": 0,
                    "sourceMarkdownSha256": "5dce6514e74b5d67956735f225b4bad042eb791adb7f0baa7764970f5adfb457",
                    "markdown": "# Too deep\n",
                    "source": {"type": "audio_asset", "assetId": "allowed"}
                }),
            ),
        );
        assert_eq!(error_code(too_deep), AgentErrorCode::TreeMaxDepthExceeded);
    }

    #[test]
    fn create_document_rejects_stale_parent_markdown_even_when_revision_is_unchanged() {
        let (gateway, repository) = gateway();
        let mut parent = repository
            .read(&DocumentId::new("allowed"))
            .unwrap()
            .unwrap();
        parent.markdown = "# External edit\n".into();
        repository
            .save(parent.state, Some(parent.markdown))
            .unwrap();

        let response = gateway.execute(
            CREATE_DOCUMENT_TOOL,
            request(
                "create-stale-markdown",
                AgentScene::Organize,
                serde_json::json!({
                    "title": "Stale child",
                    "parentDocumentId": "allowed",
                    "expectedParentRevision": 0,
                    "sourceMarkdownSha256": "5dce6514e74b5d67956735f225b4bad042eb791adb7f0baa7764970f5adfb457",
                    "markdown": "# Stale child\n",
                    "source": {"type": "audio_asset", "assetId": "allowed"}
                }),
            ),
        );
        assert_eq!(
            error_code(response),
            AgentErrorCode::DocumentRevisionConflict
        );
        assert_eq!(repository.snapshot().unwrap().documents.len(), 2);
    }

    #[test]
    fn create_document_confirm_each_is_idempotent_and_undoable() {
        let (automatic, repository) = gateway();
        let gateway = automatic.with_write_mode(AgentWriteMode::ConfirmEach);
        let request = request(
            "create-confirmed",
            AgentScene::Organize,
            serde_json::json!({
                "title": "Confirmed notes",
                "markdown": "# Confirmed notes\n",
                "source": {"type": "no_audio"}
            }),
        );
        let pending = gateway.execute(CREATE_DOCUMENT_TOOL, request.clone());
        let AgentToolResult::Pending(pending) = pending.result.as_ref().unwrap() else {
            panic!("expected pending creation");
        };
        let replay = gateway.execute(CREATE_DOCUMENT_TOOL, request);
        let AgentToolResult::Pending(replay) = replay.result.as_ref().unwrap() else {
            panic!("expected pending replay");
        };
        assert_eq!(replay.confirmation_id, pending.confirmation_id);
        assert_eq!(repository.snapshot().unwrap().documents.len(), 2);

        let receipt = mutation_receipt(gateway.confirm(&pending.confirmation_id, "confirm-create"));
        assert_eq!(receipt.affected_documents.len(), 1);
        assert_eq!(repository.snapshot().unwrap().documents.len(), 3);
        let undone = mutation_receipt(gateway.undo(&receipt.undo_token, "undo-create"));
        assert_eq!(undone.affected_documents.len(), 1);
        assert_eq!(repository.snapshot().unwrap().documents.len(), 2);
    }

    #[test]
    fn create_document_rejects_path_like_audio_asset_ids() {
        let (gateway, _) = gateway();
        let response = gateway.execute(
            CREATE_DOCUMENT_TOOL,
            request(
                "create-path",
                AgentScene::Organize,
                serde_json::json!({
                    "title": "Unsafe import",
                    "source": {"type": "audio_asset", "assetId": "../recording.wav"}
                }),
            ),
        );
        assert_eq!(error_code(response), AgentErrorCode::ToolArgumentsInvalid);
    }

    #[test]
    fn automatic_update_enforces_revision_readonly_scope_and_idempotency_then_undoes() {
        let (gateway, repository) = gateway();
        let update = request(
            "update-1",
            AgentScene::Organize,
            serde_json::json!({
                "documentId": "allowed",
                "expectedRevision": 0,
                "patch": {
                    "title": "Organized",
                    "tags": ["decision"],
                    "summary": "One factual summary",
                    "documentStatus": "review",
                    "markdown": "# Organized\n"
                }
            }),
        );
        let receipt = mutation_receipt(gateway.execute(UPDATE_DOCUMENT_TOOL, update.clone()));
        assert_eq!(receipt.provenance.actor, AgentActor::Assistant);
        assert_eq!(receipt.affected_documents[0].previous_revision, Some(0));
        assert_eq!(receipt.affected_documents[0].revision, Some(1));
        let stored = repository
            .read(&DocumentId::new("allowed"))
            .unwrap()
            .unwrap();
        assert_eq!(stored.state.title(), "Organized");
        assert_eq!(stored.state.content.tags, ["decision"]);
        assert_eq!(stored.state.document_status.value, "review");
        assert_eq!(stored.markdown, "# Organized\n");

        let mut narrowed = gateway.clone();
        narrowed.access = WorkspaceAccessPolicy::documents([]);
        assert_eq!(
            error_code(narrowed.execute(UPDATE_DOCUMENT_TOOL, update.clone())),
            AgentErrorCode::DocumentAccessDenied
        );

        let replay = mutation_receipt(gateway.execute(UPDATE_DOCUMENT_TOOL, update));
        assert_eq!(replay.mutation_id, receipt.mutation_id);
        assert_eq!(
            repository
                .read(&DocumentId::new("allowed"))
                .unwrap()
                .unwrap()
                .state
                .revision(),
            1
        );
        assert_eq!(
            error_code(gateway.execute(
                UPDATE_DOCUMENT_TOOL,
                request(
                    "update-1",
                    AgentScene::Organize,
                    serde_json::json!({
                        "documentId": "allowed",
                        "expectedRevision": 1,
                        "patch": {"title": "Different payload"}
                    }),
                ),
            )),
            AgentErrorCode::IdempotencyConflict
        );
        assert_eq!(
            error_code(gateway.execute(
                UPDATE_DOCUMENT_TOOL,
                request(
                    "stale",
                    AgentScene::Organize,
                    serde_json::json!({
                        "documentId": "allowed",
                        "expectedRevision": 0,
                        "patch": {"title": "Stale"}
                    }),
                ),
            )),
            AgentErrorCode::DocumentRevisionConflict
        );
        assert_eq!(
            error_code(gateway.execute(
                UPDATE_DOCUMENT_TOOL,
                request(
                    "readonly",
                    AgentScene::Organize,
                    serde_json::json!({
                        "documentId": "allowed",
                        "expectedRevision": 1,
                        "patch": {"processing": {}}
                    }),
                ),
            )),
            AgentErrorCode::DocumentFieldReadOnly
        );
        assert_eq!(
            error_code(gateway.execute(
                UPDATE_DOCUMENT_TOOL,
                request(
                    "unsafe-id",
                    AgentScene::Organize,
                    serde_json::json!({
                        "documentId": "../allowed",
                        "expectedRevision": 1,
                        "patch": {"title": "Unsafe"}
                    }),
                ),
            )),
            AgentErrorCode::ToolArgumentsInvalid
        );

        let mut denied = gateway.clone();
        denied.access = WorkspaceAccessPolicy::documents([]);
        assert_eq!(
            error_code(denied.undo(&receipt.undo_token, "denied-undo")),
            AgentErrorCode::DocumentAccessDenied
        );
        let undo = mutation_receipt(gateway.undo(&receipt.undo_token, "undo-update"));
        assert!(undo.affected_documents.iter().any(|revision| {
            revision.document_id == "allowed"
                && revision.previous_revision == Some(1)
                && revision.revision == Some(2)
        }));
        let restored = repository
            .read(&DocumentId::new("allowed"))
            .unwrap()
            .unwrap();
        assert_eq!(restored.state.title(), "Allowed");
        assert_eq!(restored.markdown, "# Allowed\n");
    }

    #[test]
    fn confirm_each_accepts_rejects_and_expires_without_early_side_effects() {
        let (automatic, repository) = gateway();
        let gateway = automatic.with_write_mode(AgentWriteMode::ConfirmEach);
        let pending = gateway.execute(
            UPDATE_DOCUMENT_TOOL,
            request(
                "pending-accept",
                AgentScene::Organize,
                serde_json::json!({
                    "documentId": "allowed",
                    "expectedRevision": 0,
                    "patch": {"title": "Confirmed"}
                }),
            ),
        );
        let AgentToolResult::Pending(pending) = pending.result.as_ref().unwrap() else {
            panic!("expected pending mutation");
        };
        assert_eq!(
            repository
                .read(&DocumentId::new("allowed"))
                .unwrap()
                .unwrap()
                .state
                .title(),
            "Allowed"
        );
        mutation_receipt(gateway.confirm(&pending.confirmation_id, "confirm-1"));
        assert_eq!(
            repository
                .read(&DocumentId::new("allowed"))
                .unwrap()
                .unwrap()
                .state
                .title(),
            "Confirmed"
        );

        let rejected = gateway.execute(
            UPDATE_DOCUMENT_TOOL,
            request(
                "pending-reject",
                AgentScene::Organize,
                serde_json::json!({
                    "documentId": "allowed",
                    "expectedRevision": 1,
                    "patch": {"title": "Rejected"}
                }),
            ),
        );
        let AgentToolResult::Pending(rejected) = rejected.result.as_ref().unwrap() else {
            panic!("expected pending mutation");
        };
        let mut denied = gateway.clone();
        denied.access = WorkspaceAccessPolicy::documents([]);
        assert_eq!(
            error_code(denied.reject(&rejected.confirmation_id, "denied-reject")),
            AgentErrorCode::DocumentAccessDenied
        );
        assert_eq!(
            error_code(gateway.reject(&rejected.confirmation_id, "reject-1")),
            AgentErrorCode::WritePolicyRejected
        );
        assert_eq!(
            repository
                .read(&DocumentId::new("allowed"))
                .unwrap()
                .unwrap()
                .state
                .title(),
            "Confirmed"
        );

        let expiring = gateway.clone().with_confirmation_ttl_ms(0);
        let expired = expiring.execute(
            UPDATE_DOCUMENT_TOOL,
            request(
                "pending-expire",
                AgentScene::Organize,
                serde_json::json!({
                    "documentId": "allowed",
                    "expectedRevision": 1,
                    "patch": {"title": "Expired"}
                }),
            ),
        );
        let AgentToolResult::Pending(expired) = expired.result.as_ref().unwrap() else {
            panic!("expected pending mutation");
        };
        std::thread::sleep(std::time::Duration::from_millis(2));
        assert_eq!(
            error_code(expiring.confirm(&expired.confirmation_id, "expire-1")),
            AgentErrorCode::WriteConfirmationExpired
        );
    }

    #[test]
    fn confirm_rechecks_pending_markdown_fingerprint_before_commit() {
        let (automatic, repository) = gateway();
        let gateway = automatic.with_write_mode(AgentWriteMode::ConfirmEach);
        let pending = gateway.execute(
            UPDATE_DOCUMENT_TOOL,
            request(
                "pending-content-conflict",
                AgentScene::Organize,
                serde_json::json!({
                    "documentId": "allowed",
                    "expectedRevision": 0,
                    "patch": {"title": "Must not commit"}
                }),
            ),
        );
        let AgentToolResult::Pending(pending) = pending.result.as_ref().unwrap() else {
            panic!("expected pending mutation");
        };
        let current = repository
            .read(&DocumentId::new("allowed"))
            .unwrap()
            .unwrap();
        repository
            .save(current.state, Some("# changed outside pending\n".into()))
            .unwrap();
        assert_eq!(
            error_code(gateway.confirm(&pending.confirmation_id, "confirm-content-conflict")),
            AgentErrorCode::DocumentRevisionConflict
        );
        let after = repository
            .read(&DocumentId::new("allowed"))
            .unwrap()
            .unwrap();
        assert_eq!(after.state.title(), "Allowed");
        assert_eq!(after.markdown, "# changed outside pending\n");
    }

    fn semantic_arguments(mode: &str) -> serde_json::Value {
        serde_json::json!({
            "parentDocumentId": "allowed",
            "expectedParentRevision": 0,
            "mode": mode,
            "children": [
                {
                    "startSegmentId": "seg-1",
                    "endSegmentId": "seg-2",
                    "title": "First",
                    "tags": ["first"],
                    "summary": "Facts one and two",
                    "evidenceSegmentIds": ["seg-1"]
                },
                {
                    "startSegmentId": "seg-3",
                    "endSegmentId": "seg-4",
                    "title": "Second",
                    "tags": ["second"],
                    "summary": "Facts three and four",
                    "evidenceSegmentIds": ["seg-4"]
                }
            ]
        })
    }

    #[test]
    fn semantic_partition_creates_continuous_children_and_advances_parent_atomically() {
        let (gateway, repository) = gateway();
        let receipt = mutation_receipt(gateway.execute(
            CREATE_SEMANTIC_CHILDREN_TOOL,
            request(
                "partition",
                AgentScene::Partition,
                semantic_arguments("partition"),
            ),
        ));
        assert_eq!(receipt.affected_documents.len(), 3);
        assert!(receipt.uncovered_segment_ids.is_empty());
        let state = repository.state.lock().unwrap();
        assert_eq!(state.records["allowed"].state.revision(), 1);
        let children = state
            .records
            .values()
            .filter(|record| {
                record.state.hierarchy.parent_id.as_ref() == Some(&DocumentId::new("allowed"))
            })
            .collect::<Vec<_>>();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].state.results.transcript.len(), 2);
        assert_eq!(children[1].state.results.transcript.len(), 2);
        for child in children {
            assert_eq!(child.state.hierarchy.depth, 1);
            assert_eq!(
                child.state.source.audio_asset_id.as_ref(),
                Some(&DocumentId::new("allowed"))
            );
            assert_eq!(
                child.state.results.speaker_segments.len(),
                child.state.results.transcript.len()
            );
            assert!(child.markdown.contains("## Evidence"));
        }
    }

    #[test]
    fn semantic_validator_returns_each_stable_range_error_without_side_effects() {
        let cases = [
            (
                AgentErrorCode::SegmentNotFound,
                serde_json::json!({
                    "parentDocumentId": "allowed", "expectedParentRevision": 0,
                    "mode": "extract", "children": [{
                        "startSegmentId": "missing", "endSegmentId": "seg-2",
                        "title": "Missing", "evidenceSegmentIds": []
                    }]
                }),
            ),
            (
                AgentErrorCode::RangeInvalidOrder,
                serde_json::json!({
                    "parentDocumentId": "allowed", "expectedParentRevision": 0,
                    "mode": "extract", "children": [{
                        "startSegmentId": "seg-3", "endSegmentId": "seg-2",
                        "title": "Reverse", "evidenceSegmentIds": []
                    }]
                }),
            ),
            (
                AgentErrorCode::RangeOverlap,
                serde_json::json!({
                    "parentDocumentId": "allowed", "expectedParentRevision": 0,
                    "mode": "extract", "children": [
                        {"startSegmentId": "seg-1", "endSegmentId": "seg-3", "title": "A", "evidenceSegmentIds": []},
                        {"startSegmentId": "seg-3", "endSegmentId": "seg-4", "title": "B", "evidenceSegmentIds": []}
                    ]
                }),
            ),
            (
                AgentErrorCode::RangeCoverageGap,
                serde_json::json!({
                    "parentDocumentId": "allowed", "expectedParentRevision": 0,
                    "mode": "partition", "children": [{
                        "startSegmentId": "seg-1", "endSegmentId": "seg-2",
                        "title": "Gap", "evidenceSegmentIds": []
                    }]
                }),
            ),
            (
                AgentErrorCode::RangeOutsideParent,
                serde_json::json!({
                    "parentDocumentId": "allowed", "expectedParentRevision": 0,
                    "mode": "extract", "children": [{
                        "startSegmentId": "seg-1", "endSegmentId": "seg-2",
                        "title": "Evidence", "evidenceSegmentIds": ["seg-3"]
                    }]
                }),
            ),
        ];
        for (index, (expected, arguments)) in cases.into_iter().enumerate() {
            let (gateway, repository) = gateway();
            let scene = if arguments["mode"] == "partition" {
                AgentScene::Partition
            } else {
                AgentScene::Extract
            };
            let response = gateway.execute(
                CREATE_SEMANTIC_CHILDREN_TOOL,
                request(&format!("range-{index}"), scene, arguments),
            );
            assert_eq!(error_code(response), expected);
            assert_eq!(repository.state.lock().unwrap().records.len(), 2);
        }

        let (gateway, repository) = gateway();
        repository
            .state
            .lock()
            .unwrap()
            .records
            .get_mut("allowed")
            .unwrap()
            .state
            .hierarchy
            .depth = 2;
        assert_eq!(
            error_code(gateway.execute(
                CREATE_SEMANTIC_CHILDREN_TOOL,
                request(
                    "depth",
                    AgentScene::Partition,
                    semantic_arguments("partition"),
                ),
            )),
            AgentErrorCode::TreeMaxDepthExceeded
        );
    }

    #[test]
    fn extract_reports_every_uncovered_segment() {
        let (gateway, _) = gateway();
        let response = gateway.execute(
            CREATE_SEMANTIC_CHILDREN_TOOL,
            request(
                "extract",
                AgentScene::Extract,
                serde_json::json!({
                    "parentDocumentId": "allowed",
                    "expectedParentRevision": 0,
                    "mode": "extract",
                    "children": [{
                        "startSegmentId": "seg-2",
                        "endSegmentId": "seg-3",
                        "title": "Middle",
                        "evidenceSegmentIds": ["seg-2"]
                    }]
                }),
            ),
        );
        let receipt = mutation_receipt(response);
        assert_eq!(receipt.uncovered_segment_ids, ["seg-1", "seg-4"]);
    }

    #[test]
    fn move_updates_subtree_siblings_and_both_parent_revisions_or_returns_tree_errors() {
        let repository = Arc::new(Repository::default());
        let root = DocumentState::new(DocumentId::new("root"), "Root", "a.wav");
        let mut first =
            root.semantic_child(DocumentId::new("first"), "First", 0, 1_000, Vec::new());
        first.hierarchy.sibling_order = 0;
        let mut second = root.semantic_child(
            DocumentId::new("second"),
            "Second",
            1_000,
            2_000,
            Vec::new(),
        );
        second.hierarchy.sibling_order = 1;
        let other = DocumentState::new(DocumentId::new("other"), "Other", "b.wav");
        for document in [root, first, second, other] {
            repository
                .save(document, Some("# Fixture\n".into()))
                .unwrap();
        }
        let gateway = WorkspaceCapabilityPort::new(
            DocumentService::new(repository.clone()),
            WorkspaceAccessPolicy::AllDocuments,
        );
        let receipt = mutation_receipt(gateway.execute(
            MOVE_DOCUMENT_TOOL,
            request(
                "move-success",
                AgentScene::Move,
                serde_json::json!({
                    "documentId": "second",
                    "expectedRevision": 0,
                    "newParentDocumentId": "first",
                    "placement": "first",
                    "targetSiblingDocumentId": null
                }),
            ),
        ));
        let affected = receipt
            .affected_documents
            .iter()
            .map(|revision| revision.document_id.as_str())
            .collect::<BTreeSet<_>>();
        assert!(affected.is_superset(&BTreeSet::from(["root", "first", "second"])));
        let state = repository.state.lock().unwrap();
        assert_eq!(
            state.records["second"].state.hierarchy.parent_id.as_ref(),
            Some(&DocumentId::new("first"))
        );
        assert_eq!(state.records["second"].state.hierarchy.depth, 2);
        drop(state);

        assert_eq!(
            error_code(gateway.execute(
                MOVE_DOCUMENT_TOOL,
                request(
                    "move-cycle",
                    AgentScene::Move,
                    serde_json::json!({
                        "documentId": "first",
                        "expectedRevision": 1,
                        "newParentDocumentId": "second",
                        "placement": "first",
                        "targetSiblingDocumentId": null
                    }),
                ),
            )),
            AgentErrorCode::TreeCycle
        );
        assert_eq!(
            error_code(gateway.execute(
                MOVE_DOCUMENT_TOOL,
                request(
                    "move-cross-source",
                    AgentScene::Move,
                    serde_json::json!({
                        "documentId": "first",
                        "expectedRevision": 1,
                        "newParentDocumentId": "other",
                        "placement": "first",
                        "targetSiblingDocumentId": null
                    }),
                ),
            )),
            AgentErrorCode::TreeCrossAudioAsset
        );
        assert_eq!(
            error_code(gateway.execute(
                MOVE_DOCUMENT_TOOL,
                request(
                    "move-missing-parent",
                    AgentScene::Move,
                    serde_json::json!({
                        "documentId": "first",
                        "expectedRevision": 1,
                        "newParentDocumentId": "missing",
                        "placement": "first",
                        "targetSiblingDocumentId": null
                    }),
                ),
            )),
            AgentErrorCode::TreeParentNotFound
        );
    }
}
