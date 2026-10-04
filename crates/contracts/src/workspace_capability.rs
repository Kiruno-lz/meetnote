use crate::AgentSkillSelector;
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::ops::{Deref, DerefMut};

pub const AGENT_TOOL_SCHEMA_VERSION: u32 = 1;
pub const WORKSPACE_CAPABILITY_ID: &str = "workspace.documents";
pub const WORKSPACE_CAPABILITY_VERSION: u32 = 1;

use crate::{CapabilityEnvelope, CapabilityErrorContext};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentScopeSelector {
    /// Empty means all documents granted by the caller's access policy.
    #[serde(default)]
    pub document_ids: Vec<String>,
    /// Select every document granted by the caller. This never broadens the caller grant.
    #[serde(default)]
    pub all_documents: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfirmationMutationArguments {
    pub confirmation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UndoMutationArguments {
    pub undo_token: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityRequest {
    pub schema_version: u32,
    pub request_id: String,
    pub tool_call_id: String,
    #[serde(default)]
    pub scope: AgentScopeSelector,
    pub arguments: Value,
}

impl CapabilityRequest {
    pub fn new(
        request_id: impl Into<String>,
        tool_call_id: impl Into<String>,
        scope: AgentScopeSelector,
        arguments: Value,
    ) -> Self {
        Self {
            schema_version: AGENT_TOOL_SCHEMA_VERSION,
            request_id: request_id.into(),
            tool_call_id: tool_call_id.into(),
            scope,
            arguments,
        }
    }
}

/// Workspace's typed view over the common capability response envelope.
///
/// The transparent wrapper preserves the existing two-argument constructors
/// used by the Workspace port while keeping one serialized envelope schema
/// for every capability service.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CapabilityResponse(CapabilityEnvelope<AgentToolResult, AgentToolError>);

impl Deref for CapabilityResponse {
    type Target = CapabilityEnvelope<AgentToolResult, AgentToolError>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for CapabilityResponse {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl CapabilityResponse {
    pub fn success(request_id: impl Into<String>, result: AgentToolResult) -> Self {
        Self(
            CapabilityEnvelope::<AgentToolResult, AgentToolError>::success_with_error_type(
                WORKSPACE_CAPABILITY_ID,
                WORKSPACE_CAPABILITY_VERSION,
                request_id,
                result,
            ),
        )
    }

    pub fn failure(request_id: impl Into<String>, error: AgentToolError) -> Self {
        Self(CapabilityEnvelope::failure(
            WORKSPACE_CAPABILITY_ID,
            WORKSPACE_CAPABILITY_VERSION,
            request_id,
            error,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", tag = "type", content = "payload")]
pub enum AgentToolResult {
    Documents {
        documents: Vec<AgentDocumentSummary>,
    },
    Document(AgentDocumentProjection),
    Mutation(AgentMutationReceipt),
    Pending(AgentPendingMutation),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentDocumentSummary {
    pub document_id: String,
    pub revision: u64,
    pub title: String,
    pub tags: Vec<String>,
    pub summary: Option<String>,
    pub document_status: String,
    pub parent_document_id: Option<String>,
    pub depth: u8,
    pub sibling_order: u32,
    pub audio_asset_id: String,
    pub markdown_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentDocumentProjection {
    pub metadata: AgentDocumentSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<DocumentPage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markdown: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript: Option<Vec<AgentTranscriptSegment>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker_segments: Option<Vec<AgentSpeakerSegment>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTranscriptSegment {
    pub id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSpeakerSegment {
    pub transcript_segment_id: String,
    pub speaker_id: String,
    pub speaker_profile_id: Option<String>,
    pub speaker_label: Option<String>,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentMutationReceipt {
    pub mutation_id: String,
    pub affected_documents: Vec<AgentDocumentRevision>,
    pub undo_token: String,
    pub provenance: AgentMutationProvenance,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uncovered_segment_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentDocumentRevision {
    pub document_id: String,
    pub previous_revision: Option<u64>,
    pub revision: Option<u64>,
    pub deleted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentActor {
    Assistant,
    ExternalAgent,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentMutationProvenance {
    pub actor: AgentActor,
    pub mutation_id: String,
    pub skill: Option<AgentSkillSelector>,
    pub connection_id: Option<String>,
    pub protocol: Option<String>,
    pub requested_model: Option<String>,
    pub resolved_model: Option<String>,
    pub source_revisions: Vec<AgentSourceRevision>,
    pub applied_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSourceRevision {
    pub document_id: String,
    pub revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentPendingMutation {
    pub confirmation_id: String,
    pub expires_at_ms: u64,
    pub affected_document_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub field_diffs: Vec<AgentFieldDiff>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tree_changes: Vec<AgentTreeChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ranges: Vec<AgentRangePreview>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uncovered_segment_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentFieldDiff {
    pub document_id: String,
    pub field: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTreeChange {
    pub document_id: String,
    pub previous_parent_id: Option<String>,
    pub new_parent_id: Option<String>,
    pub previous_sibling_order: Option<u32>,
    pub new_sibling_order: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentRangePreview {
    pub document_id: Option<String>,
    pub start_segment_id: String,
    pub end_segment_id: String,
    pub evidence_segment_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentErrorCode {
    ToolUnknown,
    ToolNotAllowed,
    ToolArgumentsInvalid,
    ToolLoopLimitReached,
    SkillNotFound,
    DocumentNotFound,
    DocumentAccessDenied,
    DocumentRevisionConflict,
    DocumentFieldReadOnly,
    SegmentNotFound,
    RangeInvalidOrder,
    RangeOutsideParent,
    RangeOverlap,
    RangeCoverageGap,
    TreeParentNotFound,
    TreeCycle,
    TreeMaxDepthExceeded,
    TreeCrossAudioAsset,
    WritePolicyRejected,
    WriteConfirmationExpired,
    IdempotencyConflict,
    WriteTransactionFailed,
    ConnectionNotConfigured,
    ProviderCapabilityUnsupported,
    ProviderAuthenticationFailed,
    ProviderRateLimited,
    ProviderTimeout,
    ProviderResponseInvalid,
    ContextLimitUnknown,
    ContextLimitExceeded,
    TaskCancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentToolError {
    pub code: AgentErrorCode,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "CapabilityErrorContext::is_empty")]
    pub context: CapabilityErrorContext,
}

impl AgentToolError {
    pub fn new(code: AgentErrorCode, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
            tool_call_id: None,
            field: None,
            document_id: None,
            expected_revision: None,
            actual_revision: None,
            context: CapabilityErrorContext::default(),
        }
    }

    pub fn with_tool_call(mut self, tool_call_id: impl Into<String>) -> Self {
        self.tool_call_id = Some(tool_call_id.into());
        self
    }
}

impl AgentErrorCode {
    pub const ALL: [Self; 31] = [
        Self::ToolUnknown,
        Self::ToolNotAllowed,
        Self::ToolArgumentsInvalid,
        Self::ToolLoopLimitReached,
        Self::SkillNotFound,
        Self::DocumentNotFound,
        Self::DocumentAccessDenied,
        Self::DocumentRevisionConflict,
        Self::DocumentFieldReadOnly,
        Self::SegmentNotFound,
        Self::RangeInvalidOrder,
        Self::RangeOutsideParent,
        Self::RangeOverlap,
        Self::RangeCoverageGap,
        Self::TreeParentNotFound,
        Self::TreeCycle,
        Self::TreeMaxDepthExceeded,
        Self::TreeCrossAudioAsset,
        Self::WritePolicyRejected,
        Self::WriteConfirmationExpired,
        Self::IdempotencyConflict,
        Self::WriteTransactionFailed,
        Self::ConnectionNotConfigured,
        Self::ProviderCapabilityUnsupported,
        Self::ProviderAuthenticationFailed,
        Self::ProviderRateLimited,
        Self::ProviderTimeout,
        Self::ProviderResponseInvalid,
        Self::ContextLimitUnknown,
        Self::ContextLimitExceeded,
        Self::TaskCancelled,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceCapabilityDescription {
    pub schema_version: u32,
    pub capability_id: String,
    pub capability_version: u32,
    pub write_mode: AgentWriteMode,
    pub authorization: AgentScopeSelector,
    pub tools: Vec<AgentToolDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentToolDefinition {
    pub name: String,
    pub description: String,
    pub side_effect: bool,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentWriteMode {
    #[default]
    Automatic,
    ConfirmEach,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListDocumentsArguments {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DocumentProjection {
    Metadata,
    Markdown,
    Transcript,
    SpeakerSegments,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadDocumentArguments {
    #[schemars(length(min = 1))]
    pub document_id: String,
    pub projection: DocumentProjection,
    #[serde(default)]
    pub page: Option<DocumentPageRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentDocumentPatch {
    #[schemars(length(min = 1))]
    pub title: Option<String>,
    pub tags: Option<Vec<String>>,
    pub summary: Option<Option<String>>,
    #[schemars(length(min = 1))]
    pub document_status: Option<String>,
    pub markdown: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateDocumentArguments {
    #[schemars(length(min = 1))]
    pub document_id: String,
    pub expected_revision: u64,
    #[serde(default)]
    #[schemars(
        description = "SHA-256 from read_document.metadata.markdownSha256; required for external consumers and automatically supplied by Built-in"
    )]
    pub source_markdown_sha256: Option<String>,
    pub patch: AgentDocumentPatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum CreateDocumentSource {
    /// Reference an already authorized Workspace document's audio asset.
    AudioAsset {
        #[schemars(length(min = 1))]
        #[serde(rename = "assetId")]
        asset_id: String,
    },
    /// Create a document which deliberately has no audio source.
    NoAudio,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateDocumentArguments {
    #[schemars(length(min = 1))]
    pub title: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub summary: Option<String>,
    #[serde(default)]
    pub document_status: Option<String>,
    #[serde(default)]
    pub markdown: String,
    pub source: CreateDocumentSource,
    pub parent_document_id: Option<String>,
    pub expected_parent_revision: Option<u64>,
    #[serde(default)]
    #[schemars(
        description = "SHA-256 from read_document.metadata.markdownSha256; required when creating under a parent"
    )]
    pub source_markdown_sha256: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SemanticSplitMode {
    Partition,
    Extract,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SemanticChildArguments {
    #[schemars(length(min = 1))]
    pub start_segment_id: String,
    #[schemars(length(min = 1))]
    pub end_segment_id: String,
    #[schemars(length(min = 1))]
    pub title: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub summary: Option<String>,
    #[serde(default)]
    pub evidence_segment_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateSemanticChildrenArguments {
    #[schemars(length(min = 1))]
    pub parent_document_id: String,
    pub expected_parent_revision: u64,
    #[serde(default)]
    #[schemars(
        description = "SHA-256 from read_document.metadata.markdownSha256; required for external consumers and automatically supplied by Built-in"
    )]
    pub source_markdown_sha256: Option<String>,
    pub mode: SemanticSplitMode,
    #[schemars(length(min = 1))]
    pub children: Vec<SemanticChildArguments>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MovePlacement {
    First,
    Last,
    Before,
    After,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MoveDocumentArguments {
    #[schemars(length(min = 1))]
    pub document_id: String,
    pub expected_revision: u64,
    pub new_parent_document_id: Option<String>,
    pub placement: MovePlacement,
    pub target_sibling_document_id: Option<String>,
}

pub fn schema_for_arguments<T: JsonSchema>() -> Value {
    serde_json::to_value(schema_for!(T)).expect("JSON schema serialization cannot fail")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_use_stable_screaming_snake_case() {
        assert_eq!(AgentErrorCode::ALL.len(), 31);
        let serialized = AgentErrorCode::ALL
            .into_iter()
            .map(|code| serde_json::to_string(&code).unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(serialized.len(), 31);
        assert_eq!(
            serde_json::to_string(&AgentErrorCode::DocumentRevisionConflict).unwrap(),
            "\"DOCUMENT_REVISION_CONFLICT\""
        );
        assert_eq!(
            serde_json::to_string(&AgentErrorCode::ProviderCapabilityUnsupported).unwrap(),
            "\"PROVIDER_CAPABILITY_UNSUPPORTED\""
        );
    }

    #[test]
    fn generated_schema_and_typed_decode_reject_unknown_fields() {
        let schema = schema_for_arguments::<ReadDocumentArguments>();
        assert_eq!(schema["additionalProperties"], false);
        let error = serde_json::from_value::<ReadDocumentArguments>(serde_json::json!({
            "documentId": "doc-1",
            "projection": "metadata",
            "unexpected": true
        }))
        .unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn capability_envelope_keeps_service_payloads_on_the_common_wire() {
        let envelope = CapabilityEnvelope::success(
            "settings.app",
            1,
            "settings-update-1",
            serde_json::json!({"revision": "abc"}),
        );
        let value = serde_json::to_value(&envelope).unwrap();
        assert_eq!(value["schemaVersion"], crate::CAPABILITY_ENVELOPE_VERSION);
        assert_eq!(value["capabilityId"], "settings.app");
        assert_eq!(value["requestId"], "settings-update-1");
        assert_eq!(value["result"]["revision"], "abc");
        assert!(value.get("error").is_none());
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentPageRequest {
    pub offset: usize,
    #[schemars(range(min = 1, max = 256))]
    pub limit: usize,
    pub snapshot: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentPage {
    pub snapshot: String,
    pub total: usize,
    pub next_offset: Option<usize>,
}
