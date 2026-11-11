use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use application::desktop_assistant::AssistantSettingsSave;
use contracts::AgentWriteMode;
use rune::TextEdit;
use settings::{
    AssistantAuthMode, AssistantConnectionPatch, AssistantConnectionProfile, AssistantSettings,
    CredentialUpdate, LlmProtocol, SettingsPatch,
};

pub(crate) struct ConnectionDraft {
    pub profile_id: Option<String>,
    pub name: TextEdit,
    pub endpoint: TextEdit,
    pub model: TextEdit,
    pub protocol: LlmProtocol,
    pub auth: AssistantAuthMode,
    pub credential_mode: usize,
    pub credential: TextEdit,
    pub context_window: TextEdit,
    pub timeout: TextEdit,
    pub output_limit: TextEdit,
    pub tool_rounds: TextEdit,
    pub validation_retries: TextEdit,
    pub enabled: bool,
    pub confirm_writes: bool,
}

impl ConnectionDraft {
    pub fn active(settings: &AssistantSettings) -> Self {
        Self::new(settings, settings.active_connection())
    }

    pub fn new(settings: &AssistantSettings, profile: Option<&AssistantConnectionProfile>) -> Self {
        Self {
            profile_id: profile.map(|profile| profile.id.clone()),
            name: TextEdit::new(profile.map_or("", |profile| &profile.name)),
            endpoint: TextEdit::new(profile.map_or("", |profile| &profile.base_url)),
            model: TextEdit::new(profile.map_or("", |profile| &profile.model)),
            protocol: profile.map_or(LlmProtocol::OpenAiChatCompletions, |profile| {
                profile.protocol
            }),
            auth: profile.map_or(AssistantAuthMode::Bearer, |profile| profile.auth_mode),
            credential_mode: 0,
            credential: TextEdit::default(),
            context_window: TextEdit::new(
                profile
                    .and_then(|profile| profile.context_window_tokens)
                    .map(|limit| limit.to_string())
                    .unwrap_or_default(),
            ),
            timeout: TextEdit::new(
                profile
                    .map_or(120_000, |profile| profile.timeout_ms)
                    .to_string(),
            ),
            output_limit: TextEdit::new(
                profile
                    .map_or(4096, |profile| profile.max_output_tokens)
                    .to_string(),
            ),
            tool_rounds: TextEdit::new(settings.max_tool_rounds.to_string()),
            validation_retries: TextEdit::new(settings.max_validation_retries.to_string()),
            enabled: settings.enabled,
            confirm_writes: settings.write_mode == AgentWriteMode::ConfirmEach,
        }
    }

    pub(super) fn changes(
        &self,
        settings: &AssistantSettings,
    ) -> Result<AssistantSettingsSave, String> {
        let old = self.profile_id.as_ref().and_then(|id| {
            settings
                .connections
                .iter()
                .find(|profile| &profile.id == id)
        });
        let credential = self.credential.text.trim();
        let credential = match self.credential_mode {
            1 | 2 if credential.is_empty() => {
                return Err("Enter a credential or choose Keep saved credential".into());
            }
            1 => Some(CredentialUpdate::Direct(credential.to_owned())),
            2 => Some(CredentialUpdate::File(credential.to_owned())),
            3 => Some(CredentialUpdate::Clear),
            _ => None,
        };
        let name = self.name.text.trim();
        let endpoint = self.endpoint.text.trim();
        let model = self.model.text.trim();
        let mut patch = SettingsPatch {
            assistant_enabled: Some(self.enabled),
            write_mode: Some(if self.confirm_writes {
                AgentWriteMode::ConfirmEach
            } else {
                AgentWriteMode::Automatic
            }),
            max_tool_rounds: Some(number(&self.tool_rounds, "maximum tool rounds")?),
            max_validation_retries: Some(number(
                &self.validation_retries,
                "maximum validation retries",
            )?),
            ..Default::default()
        };
        if old.is_none()
            && name.is_empty()
            && endpoint.is_empty()
            && model.is_empty()
            && credential.is_none()
        {
            return Ok(AssistantSettingsSave {
                patch,
                new_connection: None,
            });
        }
        let context_window_tokens = (!self.context_window.text.trim().is_empty())
            .then(|| number(&self.context_window, "context token limit"))
            .transpose()?;
        let timeout_ms = number(&self.timeout, "timeout in milliseconds")?;
        let max_output_tokens = number(&self.output_limit, "maximum output tokens")?;
        let (id, new_connection, connection) = if let Some(old) = old {
            let connection = AssistantConnectionPatch {
                id: old.id.clone(),
                name: (old.name != name).then(|| name.to_owned()),
                protocol: (old.protocol != self.protocol).then_some(self.protocol),
                base_url: (old.base_url != endpoint).then(|| endpoint.to_owned()),
                model: (old.model != model).then(|| model.to_owned()),
                auth_mode: (old.auth_mode != self.auth).then_some(self.auth),
                context_window_tokens: (old.context_window_tokens != context_window_tokens)
                    .then_some(context_window_tokens),
                timeout_ms: (old.timeout_ms != timeout_ms).then_some(timeout_ms),
                max_output_tokens: (old.max_output_tokens != max_output_tokens)
                    .then_some(max_output_tokens),
                credential,
            };
            (old.id.clone(), None, connection)
        } else {
            let at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let id = format!("connection-{}-{at}", std::process::id());
            let profile = AssistantConnectionProfile {
                id: id.clone(),
                name: name.to_owned(),
                protocol: self.protocol,
                base_url: endpoint.to_owned(),
                model: model.to_owned(),
                auth_mode: self.auth,
                api_key: None,
                api_key_file: None,
                context_window_tokens,
                timeout_ms,
                max_output_tokens,
            };
            profile.validate().map_err(|error| error.to_string())?;
            let connection = AssistantConnectionPatch {
                id: id.clone(),
                credential,
                ..Default::default()
            };
            (id, Some(profile), connection)
        };
        patch.active_connection_id = Some(Some(id));
        patch.connection = Some(connection);
        Ok(AssistantSettingsSave {
            patch,
            new_connection,
        })
    }
}

fn number<T: FromStr>(value: &TextEdit, field: &str) -> Result<T, String> {
    value
        .text
        .trim()
        .parse()
        .map_err(|_| format!("Enter a valid number for {field}"))
}

pub(super) fn public_assistant(mut settings: AssistantSettings) -> AssistantSettings {
    for profile in &mut settings.connections {
        profile.api_key = None;
        profile.api_key_file = None;
    }
    settings
}
