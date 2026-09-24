use crate::{CapabilityServiceError, CAPABILITY_ENVELOPE_VERSION};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Version of the Host-to-plugin invocation contract.
pub const CAPABILITY_CALL_SCHEMA_VERSION: u32 = 1;

/// Authorization is intentionally service-neutral. Each service only reads the
/// grant fields it owns and intersects them with the grant installed by Host.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityAuthorization {
    #[serde(default)]
    pub all_documents: bool,
    #[serde(default)]
    pub document_ids: Vec<String>,
    #[serde(default)]
    pub project_ids: Vec<String>,
    #[serde(default)]
    pub settings_fields: Vec<String>,
}

/// One externally visible Host call. The service owns interpretation of
/// `arguments`; Host only validates routing, version, and the caller grant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityCallRequest {
    pub schema_version: u32,
    pub request_id: String,
    pub call_id: String,
    pub capability_id: String,
    pub capability_version: u32,
    pub operation: String,
    #[serde(default)]
    pub authorization: CapabilityAuthorization,
    pub arguments: Value,
}

impl CapabilityCallRequest {
    pub fn new(
        request_id: impl Into<String>,
        call_id: impl Into<String>,
        capability_id: impl Into<String>,
        capability_version: u32,
        operation: impl Into<String>,
        authorization: CapabilityAuthorization,
        arguments: Value,
    ) -> Self {
        Self {
            schema_version: CAPABILITY_CALL_SCHEMA_VERSION,
            request_id: request_id.into(),
            call_id: call_id.into(),
            capability_id: capability_id.into(),
            capability_version,
            operation: operation.into(),
            authorization,
            arguments,
        }
    }
}

/// Generic Host response. `result` stays a JSON value because each service
/// owns its result type; the envelope still carries enough information to
/// correlate and safely route the response back to the caller.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityCallResponse {
    pub schema_version: u32,
    pub capability_id: String,
    pub capability_version: u32,
    pub request_id: String,
    pub call_id: String,
    pub operation: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CapabilityServiceError>,
}

impl CapabilityCallResponse {
    pub fn success(request: &CapabilityCallRequest, result: Value) -> Self {
        Self {
            schema_version: CAPABILITY_ENVELOPE_VERSION,
            capability_id: request.capability_id.clone(),
            capability_version: request.capability_version,
            request_id: request.request_id.clone(),
            call_id: request.call_id.clone(),
            operation: request.operation.clone(),
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(request: &CapabilityCallRequest, error: CapabilityServiceError) -> Self {
        Self {
            schema_version: CAPABILITY_ENVELOPE_VERSION,
            capability_id: request.capability_id.clone(),
            capability_version: request.capability_version,
            request_id: request.request_id.clone(),
            call_id: request.call_id.clone(),
            operation: request.operation.clone(),
            ok: false,
            result: None,
            error: Some(error),
        }
    }
}
