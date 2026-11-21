//! The vendors MeetNote knows how to reach. Each region serves up to three
//! APIs; base URLs carry their API version, OpenAI-SDK style (an Anthropic
//! root `R` of magpie is `R/v1` here). The first region is the default.
//! Endpoints and key pages follow magpie's internal/provider/presets.go.

use crate::vendors::endpoints::{Endpoints, Prefer};
use crate::vendors::quota::BalanceSource;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VendorKind {
    /// The models' own maker.
    Vendor,
    /// An aggregator reselling many makers' models.
    Relay,
    /// A server on this machine.
    Local,
}

/// A browser sign-in a vendor offers besides an API key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignIn {
    /// A ChatGPT subscription, through the Codex backend.
    ChatGpt,
    /// OpenRouter's OAuth PKCE, which mints an API key.
    OpenRouter,
}

/// A region or plan: its own endpoints, which its keys work at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub label: &'static str,
    pub endpoints: Endpoints<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vendor {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: VendorKind,
    pub featured: bool,
    /// Where the native rule picks Responses beyond its defaults.
    pub prefer: Prefer,
    pub regions: &'static [Region],
    pub website: &'static str,
    pub keys_url: Option<&'static str>,
    pub sign_in: Option<SignIn>,
    pub key_required: bool,
    pub balance: Option<BalanceSource>,
}

impl Vendor {
    pub fn default_region(&self) -> &'static Region {
        &self.regions[0]
    }

    pub fn region(&self, label: &str) -> Option<&'static Region> {
        self.regions.iter().find(|region| region.label == label)
    }
}

pub fn catalog() -> &'static [Vendor] {
    CATALOG
}

pub fn vendor(id: &str) -> Option<&'static Vendor> {
    CATALOG.iter().find(|vendor| vendor.id == id)
}

const fn url(url: &'static str) -> Option<&'static str> {
    if url.is_empty() {
        None
    } else {
        Some(url)
    }
}

/// A region serving Chat Completions, Responses and Anthropic Messages at
/// the given bases; `""` where an API is not served.
const fn region(
    label: &'static str,
    chat: &'static str,
    responses: &'static str,
    messages: &'static str,
) -> Region {
    Region {
        label,
        endpoints: Endpoints {
            chat: url(chat),
            responses: url(responses),
            messages: url(messages),
        },
    }
}

/// A key-authenticated vendor; the common case.
const fn keyed(
    id: &'static str,
    name: &'static str,
    kind: VendorKind,
    regions: &'static [Region],
    website: &'static str,
    keys_url: &'static str,
) -> Vendor {
    Vendor {
        id,
        name,
        kind,
        featured: false,
        prefer: Prefer::Nothing,
        regions,
        website,
        keys_url: Some(keys_url),
        sign_in: None,
        key_required: true,
        balance: None,
    }
}

const fn featured(vendor: Vendor) -> Vendor {
    Vendor {
        featured: true,
        ..vendor
    }
}

const fn local(
    id: &'static str,
    name: &'static str,
    regions: &'static [Region],
    website: &'static str,
) -> Vendor {
    Vendor {
        keys_url: None,
        key_required: false,
        ..keyed(id, name, VendorKind::Local, regions, website, "")
    }
}

use VendorKind::{Relay, Vendor as Maker};

static CATALOG: &[Vendor] = &[
    featured(keyed(
        "anthropic",
        "Anthropic",
        Maker,
        &[region("Global", "", "", "https://api.anthropic.com/v1")],
        "https://console.anthropic.com",
        "https://console.anthropic.com/settings/keys",
    )),
    featured(Vendor {
        prefer: Prefer::ResponsesForOpenAi,
        sign_in: Some(SignIn::ChatGpt),
        ..keyed(
            "openai",
            "OpenAI",
            Maker,
            &[region(
                "Global",
                "https://api.openai.com/v1",
                "https://api.openai.com/v1",
                "",
            )],
            "https://platform.openai.com",
            "https://platform.openai.com/api-keys",
        )
    }),
    featured(keyed(
        "google",
        "Google Gemini",
        Maker,
        &[region(
            "Global",
            "https://generativelanguage.googleapis.com/v1beta/openai",
            "",
            "",
        )],
        "https://aistudio.google.com",
        "https://aistudio.google.com/apikey",
    )),
    featured(Vendor {
        sign_in: Some(SignIn::OpenRouter),
        balance: Some(BalanceSource::OpenRouter),
        ..keyed(
            "openrouter",
            "OpenRouter",
            Relay,
            &[region(
                "Global",
                "https://openrouter.ai/api/v1",
                "",
                "https://openrouter.ai/api/v1",
            )],
            "https://openrouter.ai",
            "https://openrouter.ai/keys",
        )
    }),
    featured(Vendor {
        balance: Some(BalanceSource::DeepSeek),
        ..keyed(
            "deepseek",
            "DeepSeek",
            Maker,
            &[region(
                "Global",
                "https://api.deepseek.com/v1",
                "https://api.deepseek.com/v1",
                "https://api.deepseek.com/anthropic/v1",
            )],
            "https://platform.deepseek.com",
            "https://platform.deepseek.com/api_keys",
        )
    }),
    featured(Vendor {
        balance: Some(BalanceSource::Moonshot),
        ..keyed(
            "moonshot",
            "Kimi",
            Maker,
            &[
                region(
                    "Global",
                    "https://api.moonshot.ai/v1",
                    "",
                    "https://api.moonshot.ai/anthropic/v1",
                ),
                region(
                    "China",
                    "https://api.moonshot.cn/v1",
                    "",
                    "https://api.moonshot.cn/anthropic/v1",
                ),
            ],
            "https://platform.kimi.ai",
            "https://platform.kimi.ai/console/api-keys",
        )
    }),
    featured(keyed(
        "xai",
        "xAI",
        Maker,
        &[region(
            "Global",
            "https://api.x.ai/v1",
            "https://api.x.ai/v1",
            "https://api.x.ai/v1",
        )],
        "https://console.x.ai",
        "https://console.x.ai",
    )),
    keyed(
        "mistral",
        "Mistral",
        Maker,
        &[region("Global", "https://api.mistral.ai/v1", "", "")],
        "https://console.mistral.ai",
        "https://console.mistral.ai/api-keys",
    ),
    keyed(
        "groq",
        "Groq",
        Maker,
        &[region(
            "Global",
            "https://api.groq.com/openai/v1",
            "https://api.groq.com/openai/v1",
            "",
        )],
        "https://console.groq.com",
        "https://console.groq.com/keys",
    ),
    keyed(
        "together",
        "Together AI",
        Relay,
        &[region("Global", "https://api.together.xyz/v1", "", "")],
        "https://api.together.ai",
        "https://api.together.ai/settings/api-keys",
    ),
    keyed(
        "fireworks",
        "Fireworks",
        Relay,
        &[region(
            "Global",
            "https://api.fireworks.ai/inference/v1",
            "",
            "",
        )],
        "https://fireworks.ai",
        "https://app.fireworks.ai/settings/users/api-keys",
    ),
    keyed(
        "siliconflow",
        "SiliconFlow",
        Relay,
        &[
            region("China", "https://api.siliconflow.cn/v1", "", ""),
            region("Global", "https://api.siliconflow.com/v1", "", ""),
        ],
        "https://cloud.siliconflow.cn",
        "https://cloud.siliconflow.cn/account/ak",
    ),
    keyed(
        "zhipu",
        "Zhipu GLM",
        Maker,
        &[
            region(
                "Pay as you go",
                "https://open.bigmodel.cn/api/paas/v4",
                "",
                "https://open.bigmodel.cn/api/anthropic/v1",
            ),
            region(
                "Coding Plan",
                "https://open.bigmodel.cn/api/coding/paas/v4",
                "https://open.bigmodel.cn/api/v1",
                "https://open.bigmodel.cn/api/anthropic/v1",
            ),
        ],
        "https://open.bigmodel.cn",
        "https://open.bigmodel.cn/usercenter/proj-mgmt/apikeys",
    ),
    keyed(
        "zai",
        "Z.ai",
        Maker,
        &[
            region(
                "Pay as you go",
                "https://api.z.ai/api/paas/v4",
                "",
                "https://api.z.ai/api/anthropic/v1",
            ),
            region(
                "Coding Plan",
                "https://api.z.ai/api/coding/paas/v4",
                "https://api.z.ai/api/v1",
                "https://api.z.ai/api/anthropic/v1",
            ),
        ],
        "https://z.ai",
        "https://z.ai/manage-apikey/apikey-list",
    ),
    keyed(
        "minimax",
        "MiniMax",
        Maker,
        &[
            region(
                "Global",
                "https://api.minimax.io/v1",
                "",
                "https://api.minimax.io/anthropic/v1",
            ),
            region(
                "China",
                "https://api.minimaxi.com/v1",
                "",
                "https://api.minimaxi.com/anthropic/v1",
            ),
        ],
        "https://platform.minimax.io",
        "https://platform.minimax.io/user-center/basic-information/interface-key",
    ),
    keyed(
        "qwen",
        "Qwen",
        Maker,
        &[
            region(
                "International",
                "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
                "",
                "https://dashscope-intl.aliyuncs.com/apps/anthropic/v1",
            ),
            region(
                "China",
                "https://dashscope.aliyuncs.com/compatible-mode/v1",
                "",
                "https://dashscope.aliyuncs.com/apps/anthropic/v1",
            ),
        ],
        "https://modelstudio.console.alibabacloud.com",
        "https://modelstudio.console.alibabacloud.com/?tab=playground#/api-key",
    ),
    keyed(
        "stepfun",
        "StepFun",
        Maker,
        &[
            region(
                "Global",
                "https://api.stepfun.ai/v1",
                "",
                "https://api.stepfun.ai/v1",
            ),
            region(
                "China",
                "https://api.stepfun.com/v1",
                "",
                "https://api.stepfun.com/v1",
            ),
            region(
                "Step Plan · Global",
                "https://api.stepfun.ai/step_plan/v1",
                "",
                "https://api.stepfun.ai/step_plan/v1",
            ),
            region(
                "Step Plan · China",
                "https://api.stepfun.com/step_plan/v1",
                "",
                "https://api.stepfun.com/step_plan/v1",
            ),
        ],
        "https://platform.stepfun.ai",
        "https://platform.stepfun.ai/interface-key",
    ),
    keyed(
        "ollama-cloud",
        "Ollama Cloud",
        Maker,
        &[region(
            "Global",
            "https://ollama.com/v1",
            "",
            "https://ollama.com/v1",
        )],
        "https://docs.ollama.com/cloud",
        "https://ollama.com/settings/keys",
    ),
    keyed(
        "nvidia",
        "NVIDIA NIM",
        Relay,
        &[region(
            "Global",
            "https://integrate.api.nvidia.com/v1",
            "",
            "",
        )],
        "https://build.nvidia.com",
        "https://build.nvidia.com/settings/api-keys",
    ),
    keyed(
        "vercel",
        "Vercel AI Gateway",
        Relay,
        &[region(
            "Global",
            "https://ai-gateway.vercel.sh/v1",
            "https://ai-gateway.vercel.sh/v1",
            "https://ai-gateway.vercel.sh/v1",
        )],
        "https://vercel.com/docs/ai-gateway",
        "https://vercel.com/d?to=%2F%5Bteam%5D%2F%7E%2Fai-gateway%2Fapi-keys",
    ),
    Vendor {
        prefer: Prefer::Responses,
        ..keyed(
            "modelscope",
            "ModelScope",
            Relay,
            &[region(
                "China",
                "https://api-inference.modelscope.cn/v1",
                "https://api-inference.modelscope.cn/v1",
                "",
            )],
            "https://modelscope.cn/docs/model-service/API-Inference/intro",
            "https://modelscope.cn/my/myaccesstoken",
        )
    },
    local(
        "ollama",
        "Ollama",
        &[region(
            "This computer",
            "http://localhost:11434/v1",
            "",
            "http://localhost:11434/v1",
        )],
        "https://ollama.com",
    ),
    local(
        "lmstudio",
        "LM Studio",
        &[region("This computer", "http://localhost:1234/v1", "", "")],
        "https://lmstudio.ai",
    ),
    local(
        "omlx",
        "oMLX",
        &[region(
            "This computer",
            "http://localhost:8000/v1",
            "http://localhost:8000/v1",
            "http://localhost:8000/v1",
        )],
        "https://omlx.ai",
    ),
    // magpie's gateway; requests from this computer need no gateway key.
    Vendor {
        prefer: Prefer::ResponsesForOpenAi,
        ..local(
            "magpie",
            "magpie",
            &[region(
                "This computer",
                "http://127.0.0.1:3425/v1",
                "http://127.0.0.1:3425/v1",
                "http://127.0.0.1:3425/v1",
            )],
            "https://usemagpie.ai",
        )
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::is_loopback_host;
    use crate::LlmProtocol::{
        AnthropicMessages as Messages, OpenAiChatCompletions as Chat, OpenAiResponses as Responses,
    };
    use std::collections::HashSet;
    use url::Url;

    #[test]
    fn ids_are_unique_and_urls_are_secure_unless_local() {
        let mut ids = HashSet::new();
        for vendor in catalog() {
            assert!(ids.insert(vendor.id), "duplicate vendor {}", vendor.id);
            assert!(!vendor.regions.is_empty(), "{} has no region", vendor.id);
            let mut labels = HashSet::new();
            for region in vendor.regions {
                assert!(
                    labels.insert(region.label),
                    "{} {}",
                    vendor.id,
                    region.label
                );
                assert!(
                    !region.endpoints.protocols().is_empty(),
                    "{} {} serves no API",
                    vendor.id,
                    region.label
                );
                for (_, base) in region.endpoints.iter() {
                    let url = Url::parse(base).unwrap();
                    let local = url.host_str().is_some_and(is_loopback_host);
                    assert_eq!(local, vendor.kind == VendorKind::Local, "{base}");
                    assert!(
                        url.scheme() == "https" || local && url.scheme() == "http",
                        "{base}"
                    );
                    assert!(url.query().is_none() && !base.ends_with('/'), "{base}");
                }
                if let Some(messages) = region.endpoints.messages {
                    assert!(messages.ends_with("/v1"), "{messages} lacks its version");
                }
            }
            for page in std::iter::once(vendor.website).chain(vendor.keys_url) {
                assert_eq!(Url::parse(page).unwrap().scheme(), "https", "{page}");
            }
            assert_eq!(vendor.key_required, vendor.kind != VendorKind::Local);
        }
    }

    #[test]
    fn featured_vendors_lead_in_order() {
        let featured = catalog()
            .iter()
            .take_while(|vendor| vendor.featured)
            .map(|vendor| vendor.id)
            .collect::<Vec<_>>();
        assert_eq!(
            featured,
            [
                "anthropic",
                "openai",
                "google",
                "openrouter",
                "deepseek",
                "moonshot",
                "xai"
            ]
        );
        assert_eq!(catalog().iter().filter(|vendor| vendor.featured).count(), 7);
    }

    fn protocols(id: &str, label: &str) -> Vec<crate::LlmProtocol> {
        vendor(id)
            .unwrap()
            .region(label)
            .unwrap()
            .endpoints
            .protocols()
    }

    #[test]
    fn each_vendor_serves_the_apis_magpie_knows() {
        for (id, label, expected) in [
            ("anthropic", "Global", &[Messages][..]),
            ("openai", "Global", &[Chat, Responses]),
            ("google", "Global", &[Chat]),
            ("openrouter", "Global", &[Chat, Messages]),
            ("deepseek", "Global", &[Chat, Responses, Messages]),
            ("moonshot", "Global", &[Chat, Messages]),
            ("moonshot", "China", &[Chat, Messages]),
            ("xai", "Global", &[Chat, Responses, Messages]),
            ("groq", "Global", &[Chat, Responses]),
            ("zhipu", "Pay as you go", &[Chat, Messages]),
            ("zhipu", "Coding Plan", &[Chat, Responses, Messages]),
            ("modelscope", "China", &[Chat, Responses]),
            ("ollama", "This computer", &[Chat, Messages]),
            ("lmstudio", "This computer", &[Chat]),
            ("omlx", "This computer", &[Chat, Responses, Messages]),
            ("magpie", "This computer", &[Chat, Responses, Messages]),
        ] {
            assert_eq!(protocols(id, label), expected, "{id} {label}");
        }
        let deepseek = vendor("deepseek").unwrap().default_region().endpoints;
        assert_eq!(deepseek.chat, Some("https://api.deepseek.com/v1"));
        assert_eq!(
            deepseek.messages,
            Some("https://api.deepseek.com/anthropic/v1")
        );
        let kimi = vendor("moonshot").unwrap();
        assert_eq!(kimi.default_region().label, "Global");
        assert_eq!(
            kimi.region("China").unwrap().endpoints.messages,
            Some("https://api.moonshot.cn/anthropic/v1")
        );
        assert!(kimi.region("Mars").is_none());
        let magpie = vendor("magpie").unwrap().default_region().endpoints;
        assert!(magpie
            .iter()
            .all(|(_, base)| base == "http://127.0.0.1:3425/v1"));
        let omlx = vendor("omlx").unwrap().default_region().endpoints;
        assert!(omlx
            .iter()
            .all(|(_, base)| base == "http://localhost:8000/v1"));
    }

    #[test]
    fn prefers_and_sign_ins_match_the_vendor() {
        for vendor in catalog() {
            let expected = match vendor.id {
                "openai" | "magpie" => Prefer::ResponsesForOpenAi,
                "modelscope" => Prefer::Responses,
                _ => Prefer::Nothing,
            };
            assert_eq!(vendor.prefer, expected, "{}", vendor.id);
        }
        assert_eq!(vendor("openai").unwrap().sign_in, Some(SignIn::ChatGpt));
        assert_eq!(
            vendor("openrouter").unwrap().sign_in,
            Some(SignIn::OpenRouter)
        );
        assert_eq!(vendor("xai").unwrap().sign_in, None);
        assert!(vendor("missing").is_none());
        let magpie = vendor("magpie").unwrap();
        assert_eq!(magpie.kind, VendorKind::Local);
        assert!(!magpie.key_required);
        let omlx = vendor("omlx").unwrap();
        assert_eq!(omlx.kind, VendorKind::Local);
        assert!(!omlx.key_required);
    }
}
