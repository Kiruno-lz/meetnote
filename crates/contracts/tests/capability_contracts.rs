use contracts::{
    AgentToolError, AgentToolResult, CapabilityErrorContext, CapabilityService,
    CapabilityServiceError, CAPABILITY_ENVELOPE_VERSION, WORKSPACE_CAPABILITY_ID,
    WORKSPACE_CAPABILITY_VERSION,
};
use serde_json::json;

#[test]
fn service_error_context_is_typed_locatable_and_serializable() {
    let error = CapabilityServiceError::new("TOOL_NOT_ALLOWED", "tool is not open", false)
        .with_context(
            CapabilityErrorContext::new(CapabilityService::Workspace, "update_document")
                .with_resource("capability", WORKSPACE_CAPABILITY_ID),
        );

    let value = serde_json::to_value(&error).unwrap();
    assert_eq!(value["code"], "TOOL_NOT_ALLOWED");
    assert_eq!(value["retryable"], false);
    assert_eq!(value["context"]["service"], "workspace");
    assert_eq!(value["context"]["operation"], "update_document");
    assert_eq!(value["context"]["resourceType"], "capability");
    assert_eq!(value["context"]["resourceId"], WORKSPACE_CAPABILITY_ID);
    assert_eq!(
        serde_json::from_value::<CapabilityServiceError>(value).unwrap(),
        error
    );
}

#[test]
fn workspace_response_has_one_success_or_failure_payload() {
    let response = contracts::CapabilityResponse::success(
        "workspace-1",
        AgentToolResult::Documents {
            documents: Vec::new(),
        },
    );
    assert!(response.ok);
    assert_eq!(response.schema_version, CAPABILITY_ENVELOPE_VERSION);
    assert_eq!(response.capability_id, WORKSPACE_CAPABILITY_ID);
    assert_eq!(response.capability_version, WORKSPACE_CAPABILITY_VERSION);
    assert!(response.error.is_none());

    let error = AgentToolError::new(
        contracts::AgentErrorCode::DocumentNotFound,
        "missing",
        false,
    );
    let failed = contracts::CapabilityResponse::failure("workspace-2", error);
    let value = serde_json::to_value(&failed).unwrap();
    assert_eq!(value["ok"], false);
    assert!(value.get("result").is_none());
    assert_eq!(value["error"]["code"], "DOCUMENT_NOT_FOUND");
}

#[test]
fn workspace_error_context_keeps_existing_extension_keys() {
    let mut error = AgentToolError::new(
        contracts::AgentErrorCode::ContextLimitExceeded,
        "too large",
        true,
    );
    error.context.insert("maxBytes".into(), json!(512 * 1024));
    error
        .context
        .insert("actualBytes".into(), json!(600 * 1024));

    let value = serde_json::to_value(&error).unwrap();
    assert_eq!(value["context"]["maxBytes"], 512 * 1024);
    assert_eq!(value["context"]["actualBytes"], 600 * 1024);
    assert!(value["context"].get("service").is_none());

    let decoded: AgentToolError = serde_json::from_value(value).unwrap();
    assert_eq!(decoded.context["maxBytes"], json!(512 * 1024));
    assert_eq!(decoded.context["actualBytes"], json!(600 * 1024));
}
