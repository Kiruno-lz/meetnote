//! The APIs a provider serves, and the one a model is best asked on: a port
//! of magpie's `Provider.Native` (internal/provider/provider.go).

use crate::{AuthMode, LlmProtocol};

/// Every API, in the order a provider is said to speak them; the native
/// rule's last resort is the first one served.
pub const PROTOCOLS: [LlmProtocol; 3] = [
    LlmProtocol::OpenAiChatCompletions,
    LlmProtocol::OpenAiResponses,
    LlmProtocol::AnthropicMessages,
];

/// The base URL each API is asked at, version included (`…/v1`); `None`
/// where the provider does not serve it. The catalog holds
/// `Endpoints<&'static str>`, a configured provider `Endpoints<String>`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Endpoints<S = String> {
    pub chat: Option<S>,
    pub responses: Option<S>,
    pub messages: Option<S>,
}

impl<S: AsRef<str>> Endpoints<S> {
    pub fn get(&self, protocol: LlmProtocol) -> Option<&str> {
        match protocol {
            LlmProtocol::OpenAiChatCompletions => self.chat.as_ref(),
            LlmProtocol::OpenAiResponses => self.responses.as_ref(),
            LlmProtocol::AnthropicMessages => self.messages.as_ref(),
        }
        .map(AsRef::as_ref)
    }

    /// The served APIs and their base URLs, in [`PROTOCOLS`] order.
    pub fn iter(&self) -> impl Iterator<Item = (LlmProtocol, &str)> {
        PROTOCOLS
            .into_iter()
            .filter_map(|protocol| self.get(protocol).map(|url| (protocol, url)))
    }

    pub fn protocols(&self) -> Vec<LlmProtocol> {
        self.iter().map(|(protocol, _)| protocol).collect()
    }

    pub fn owned(&self) -> Endpoints {
        let owned = |url: &Option<S>| url.as_ref().map(|url| url.as_ref().to_owned());
        Endpoints {
            chat: owned(&self.chat),
            responses: owned(&self.responses),
            messages: owned(&self.messages),
        }
    }
}

impl Endpoints {
    pub fn set(&mut self, protocol: LlmProtocol, url: Option<String>) {
        *match protocol {
            LlmProtocol::OpenAiChatCompletions => &mut self.chat,
            LlmProtocol::OpenAiResponses => &mut self.responses,
            LlmProtocol::AnthropicMessages => &mut self.messages,
        } = url;
    }
}

/// Where a vendor asks a model on Responses although another API it
/// serves would take it too.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Prefer {
    /// The rule alone: Messages for Claude, else the first API served.
    #[default]
    Nothing,
    /// OpenAI's own models (GPT, Codex, o-series), served best on
    /// Responses: OpenAI's API and magpie's gateway.
    ResponsesForOpenAi,
    /// Every model: only Responses reports the cache and the reasoning
    /// (ModelScope; magpie's `ResponsesTellsCache`).
    Responses,
}

/// The API `model` is best asked on among `available`, restricted to
/// `listed` when the model list named the APIs that serve it; `None` when
/// none is left.
pub fn native(
    available: &[LlmProtocol],
    prefer: Prefer,
    listed: &[LlmProtocol],
    model: &str,
) -> Option<LlmProtocol> {
    let candidates = PROTOCOLS
        .into_iter()
        .filter(|protocol| available.contains(protocol))
        .filter(|protocol| listed.is_empty() || listed.contains(protocol))
        .collect::<Vec<_>>();
    let responses_first = match prefer {
        Prefer::Nothing => false,
        Prefer::ResponsesForOpenAi => openai_model(model),
        Prefer::Responses => true,
    };
    if responses_first && candidates.contains(&LlmProtocol::OpenAiResponses) {
        return Some(LlmProtocol::OpenAiResponses);
    }
    if claude_model(model) && candidates.contains(&LlmProtocol::AnthropicMessages) {
        return Some(LlmProtocol::AnthropicMessages);
    }
    candidates.first().copied()
}

/// The header a key goes in on `protocol`: Anthropic's `x-api-key` on
/// Messages (which every compatible vendor takes, magpie tries it first),
/// a Bearer token elsewhere.
pub fn default_auth(protocol: LlmProtocol) -> AuthMode {
    match protocol {
        LlmProtocol::AnthropicMessages => AuthMode::XApiKey,
        LlmProtocol::OpenAiChatCompletions | LlmProtocol::OpenAiResponses => AuthMode::Bearer,
    }
}

/// The model's name after any `vendor/` prefix, lowercased.
fn bare(model: &str) -> String {
    model
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .to_ascii_lowercase()
}

/// One of OpenAI's own by its name: a GPT, a Codex or an o-series model.
fn openai_model(model: &str) -> bool {
    let name = bare(model);
    let bytes = name.as_bytes();
    name.starts_with("gpt-")
        || name.starts_with("codex")
        || bytes.len() > 1 && bytes[0] == b'o' && bytes[1].is_ascii_digit()
}

/// One of Anthropic's Claude models by its name.
fn claude_model(model: &str) -> bool {
    bare(model).starts_with("claude")
}

#[cfg(test)]
mod tests {
    use super::*;
    use LlmProtocol::{
        AnthropicMessages as Messages, OpenAiChatCompletions as Chat, OpenAiResponses as Responses,
    };

    #[test]
    fn model_families_are_read_after_any_vendor_prefix() {
        for model in ["gpt-5.1", "openai/GPT-4o", "codex-mini", "o3", "o4-mini"] {
            assert!(openai_model(model), "{model}");
        }
        for model in ["gpt4", "omni", "o", "claude-opus-5-5", "chatgpt-4o"] {
            assert!(!openai_model(model), "{model}");
        }
        for model in ["claude-opus-5-5", "anthropic/Claude-Sonnet-5-5"] {
            assert!(claude_model(model), "{model}");
        }
        assert!(!claude_model("deepseek-chat"));
    }

    #[test]
    fn the_native_api_follows_magpies_rule() {
        let all = [Chat, Responses, Messages];
        for (available, prefer, listed, model, expected) in [
            // 1. the listing restricts the candidates
            (
                &all[..],
                Prefer::Nothing,
                &[Messages][..],
                "kimi-k2",
                Some(Messages),
            ),
            (
                &all,
                Prefer::Nothing,
                &[Responses, Messages],
                "deepseek",
                Some(Responses),
            ),
            (&[Chat], Prefer::Nothing, &[Messages], "claude-x", None),
            (&[], Prefer::Nothing, &[], "gpt-5", None),
            // 2. Responses for every model
            (
                &[Chat, Responses],
                Prefer::Responses,
                &[],
                "qwen3",
                Some(Responses),
            ),
            (&[Chat], Prefer::Responses, &[], "qwen3", Some(Chat)),
            (
                &all,
                Prefer::Responses,
                &[Chat, Messages],
                "qwen3",
                Some(Chat),
            ),
            // 3. Responses for OpenAI's models
            (
                &all,
                Prefer::ResponsesForOpenAi,
                &[],
                "openai/gpt-5",
                Some(Responses),
            ),
            (&all, Prefer::ResponsesForOpenAi, &[], "o3", Some(Responses)),
            (&all, Prefer::ResponsesForOpenAi, &[], "qwen3", Some(Chat)),
            (
                &all,
                Prefer::ResponsesForOpenAi,
                &[Chat],
                "gpt-5",
                Some(Chat),
            ),
            (&all, Prefer::Nothing, &[], "gpt-5", Some(Chat)),
            // 4. Messages for Claude
            (
                &all,
                Prefer::ResponsesForOpenAi,
                &[],
                "claude-opus-5-5",
                Some(Messages),
            ),
            (
                &all,
                Prefer::Responses,
                &[],
                "claude-opus-5-5",
                Some(Responses),
            ),
            (
                &[Chat, Responses],
                Prefer::Nothing,
                &[],
                "claude-x",
                Some(Chat),
            ),
            // 5. the first served, Chat before Responses before Messages
            (
                &[Messages, Responses],
                Prefer::Nothing,
                &[],
                "glm-5",
                Some(Responses),
            ),
            (&[Messages], Prefer::Nothing, &[], "glm-5", Some(Messages)),
        ] {
            assert_eq!(
                native(available, prefer, listed, model),
                expected,
                "{available:?} {prefer:?} {listed:?} {model}"
            );
        }
    }

    #[test]
    fn endpoints_are_read_and_set_per_api() {
        let catalog = Endpoints {
            chat: Some("https://a/v1"),
            responses: None,
            messages: Some("https://a/anthropic/v1"),
        };
        assert_eq!(catalog.protocols(), [Chat, Messages]);
        let mut owned = catalog.owned();
        owned.set(Responses, Some("https://b/v1".into()));
        owned.set(Chat, None);
        assert_eq!(
            owned.iter().collect::<Vec<_>>(),
            [
                (Responses, "https://b/v1"),
                (Messages, "https://a/anthropic/v1")
            ]
        );
        assert_eq!(default_auth(Messages), AuthMode::XApiKey);
        assert_eq!(default_auth(Chat), AuthMode::Bearer);
        assert_eq!(default_auth(Responses), AuthMode::Bearer);
    }
}
