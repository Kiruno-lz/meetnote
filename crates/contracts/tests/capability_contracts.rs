use contracts::{
    AgentToolError, AgentToolResult, CapabilityEnvelope, CapabilityErrorContext, CapabilityService,
    CapabilityServiceError, CAPABILITY_ENVELOPE_VERSION, WORKSPACE_CAPABILITY_ID,
    WORKSPACE_CAPABILITY_VERSION,
};
use serde_json::json;

#[test]
fn service_error_context_is_typed_locatable_and_serializable() {
    let error =
        CapabilityServiceError::new("SETTINGS_FIELD_NOT_GRANTED", "field is not granted", false)
            .with_context(
                CapabilityErrorContext::new(CapabilityService::Settings, "update")
                    .with_resource("settings", "app")
                    .with_field("assistant")
                    .with_path("assistant.connections")
                    .with_revisions("before", "after"),
            );

    let value = serde_json::to_value(&error).unwrap();
    assert_eq!(value["code"], "SETTINGS_FIELD_NOT_GRANTED");
    assert_eq!(value["retryable"], false);
    assert_eq!(value["context"]["service"], "settings");
    assert_eq!(value["context"]["operation"], "update");
    assert_eq!(value["context"]["resourceType"], "settings");
    assert_eq!(value["context"]["resourceId"], "app");
    assert_eq!(value["context"]["field"], "assistant");
    assert_eq!(value["context"]["path"], "assistant.connections");
    assert_eq!(value["context"]["expectedRevision"], "before");
    assert_eq!(value["context"]["actualRevision"], "after");
    assert_eq!(
        serde_json::from_value::<CapabilityServiceError>(value).unwrap(),
        error
    );
}

#[test]
fn common_envelope_has_one_success_or_failure_payload() {
    let success =
        CapabilityEnvelope::success("settings.app", 1, "settings-1", json!({"revision": "next"}));
    let success_value = serde_json::to_value(&success).unwrap();
    assert_eq!(success_value["schemaVersion"], CAPABILITY_ENVELOPE_VERSION);
    assert_eq!(success_value["capabilityId"], "settings.app");
    assert_eq!(success_value["requestId"], "settings-1");
    assert_eq!(success_value["ok"], true);
    assert!(success_value.get("error").is_none());

    let failure = CapabilityEnvelope::<serde_json::Value, CapabilityServiceError>::failure(
        "settings.app",
        1,
        "settings-2",
        CapabilityServiceError::new("SETTINGS_REVISION_CONFLICT", "revision changed", true),
    );
    let failure_value = serde_json::to_value(&failure).unwrap();
    assert_eq!(failure_value["ok"], false);
    assert!(failure_value.get("result").is_none());
    assert_eq!(failure_value["error"]["code"], "SETTINGS_REVISION_CONFLICT");
}

#[test]
fn workspace_response_uses_the_common_envelope_without_changing_accessors() {
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
