//! What is left on an account: a ChatGPT plan's usage windows, credits and
//! reset cards, or an API key's prepaid balance. Ported from magpie's
//! subscription_usage.go, codex_resets.go and balance.go.

use crate::codex;
use crate::http::{endpoint_url, header_secret, Http, VENDOR_TIMEOUT};
use crate::vendors::oauth::chatgpt::{ChatGptEndpoints, ChatGptTokens};
use crate::{Credential, LlmCancellation, LlmError, LlmErrorKind};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Quota {
    pub plan: Option<String>,
    pub windows: Vec<QuotaWindow>,
    /// Credits or money left, as the vendor counts it ("12.5 credits").
    pub balance: Option<String>,
    /// Rate-limit resets the account can still use.
    pub reset_cards: Option<u32>,
    /// When the paid plan runs out, in Unix seconds.
    pub subscription_until: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QuotaWindow {
    /// The window's length (5 hours, 7 days).
    pub span_seconds: Option<u64>,
    pub used_percent: f32,
    /// When the window starts again, in Unix seconds.
    pub resets_at: Option<u64>,
}

/// Where a vendor tells an API key's prepaid balance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BalanceSource {
    /// `GET {base}/credits`, in dollars.
    OpenRouter,
    /// `GET {origin}/user/balance`, per currency.
    DeepSeek,
    /// `GET {base}/users/me/balance`, in yuan on `.cn`, dollars on `.ai`.
    Moonshot,
}

/// A ChatGPT plan's usage, as Codex's `/status` reads it.
pub fn chatgpt_quota(
    endpoints: &ChatGptEndpoints,
    tokens: &ChatGptTokens,
    cancellation: &LlmCancellation,
) -> Result<Quota, LlmError> {
    let backend = endpoints.backend.as_str();
    let http = Http::for_url(backend, VENDOR_TIMEOUT)?;
    let read = |path: &str| {
        http.get(endpoint_url(backend, path)?)
            .header("Authorization", format!("Bearer {}", tokens.access_token))
            .header("Accept", "application/json")
            .header("chatgpt-account-id", tokens.account_id.clone())
            .headers(codex::client_headers())
            .send(cancellation)?
            .json()
    };
    let mut quota = parse_chatgpt_usage(&read("/wham/usage")?, now());
    quota.subscription_until = tokens.subscription_until;
    if quota.reset_cards.is_some_and(|count| count > 0) {
        // the count alone stands when the details cannot be read
        if let Ok(details) = read("/wham/rate-limit-reset-credits") {
            quota.reset_cards = Some(available_reset_cards(&details));
        }
    }
    Ok(quota)
}

/// The prepaid balance on an API key, such as `$12.34` or `¥110.00 · $5.00`.
pub fn balance(
    source: BalanceSource,
    base_url: &str,
    credential: &Credential,
    cancellation: &LlmCancellation,
) -> Result<String, LlmError> {
    let url = match source {
        BalanceSource::OpenRouter => endpoint_url(base_url, "/credits")?,
        BalanceSource::DeepSeek => {
            let mut url = endpoint_url(base_url, "")?;
            url.set_path("/user/balance");
            url
        }
        BalanceSource::Moonshot => endpoint_url(base_url, "/users/me/balance")?,
    };
    let yuan = url.host_str().is_some_and(|host| host.ends_with(".cn"));
    let body = Http::for_url(base_url, VENDOR_TIMEOUT)?
        .get(url)
        .header(
            "Authorization",
            format!("Bearer {}", header_secret(credential)?),
        )
        .header("Accept", "application/json")
        .send(cancellation)?
        .json()?;
    match source {
        BalanceSource::OpenRouter => parse_openrouter_balance(&body),
        BalanceSource::DeepSeek => parse_deepseek_balance(&body),
        BalanceSource::Moonshot => parse_moonshot_balance(&body, if yuan { "¥" } else { "$" }),
    }
    .ok_or_else(|| {
        LlmError::new(
            LlmErrorKind::ResponseInvalid,
            "the vendor's reply holds no balance",
        )
    })
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// `/wham/usage`: the plan, the primary and secondary windows, credits when
/// some are left and not unlimited, and the reset cards' count.
fn parse_chatgpt_usage(usage: &Value, now: u64) -> Quota {
    let windows = ["primary_window", "secondary_window"]
        .iter()
        .filter_map(|key| usage.get("rate_limit")?.get(*key))
        .filter(|window| window.is_object())
        .map(|window| {
            let seconds = |key: &str| window.get(key).and_then(Value::as_u64).filter(|n| *n > 0);
            QuotaWindow {
                span_seconds: seconds("limit_window_seconds"),
                used_percent: window
                    .get("used_percent")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0) as f32,
                resets_at: seconds("reset_at")
                    .or_else(|| seconds("reset_after_seconds").map(|after| now + after)),
            }
        })
        .collect();
    let credits = usage.get("credits");
    let balance = credits
        .filter(|credits| {
            credits.get("has_credits").and_then(Value::as_bool) == Some(true)
                && credits.get("unlimited").and_then(Value::as_bool) != Some(true)
        })
        .and_then(|credits| number(credits.get("balance")?))
        .filter(|balance| *balance > 0.0)
        .map(|balance| format!("{} credits", compact(balance)));
    Quota {
        plan: usage
            .get("plan_type")
            .and_then(Value::as_str)
            .filter(|plan| !plan.is_empty())
            .map(str::to_owned),
        windows,
        balance,
        reset_cards: usage
            .pointer("/rate_limit_reset_credits/available_count")
            .and_then(Value::as_u64)
            .map(|count| count.min(u64::from(u32::MAX)) as u32),
        subscription_until: None,
    }
}

/// `/wham/rate-limit-reset-credits`: the cards still available.
fn available_reset_cards(details: &Value) -> u32 {
    details
        .get("credits")
        .and_then(Value::as_array)
        .map(|cards| {
            cards
                .iter()
                .filter(|card| card.get("status").and_then(Value::as_str) == Some("available"))
                .count() as u32
        })
        .unwrap_or(0)
}

/// `{"data":{"total_credits":20,"total_usage":3.5}}`, in dollars.
fn parse_openrouter_balance(body: &Value) -> Option<String> {
    let credits = number(body.pointer("/data/total_credits")?)?;
    let used = body
        .pointer("/data/total_usage")
        .and_then(number)
        .unwrap_or(0.0);
    Some(money("$", credits - used))
}

/// `{"balance_infos":[{"currency":"CNY","total_balance":"110.00"}, ...]}`:
/// every currency, those with money left first, then by code (DeepSeek's
/// own order changes between reads).
fn parse_deepseek_balance(body: &Value) -> Option<String> {
    let mut amounts = body
        .get("balance_infos")?
        .as_array()?
        .iter()
        .filter_map(|info| {
            let total = number(info.get("total_balance")?)?;
            let code = info
                .get("currency")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_uppercase();
            Some((code, total))
        })
        .collect::<Vec<_>>();
    if amounts.is_empty() {
        return None;
    }
    amounts.sort_by(|(a_code, a_total), (b_code, b_total)| {
        (*b_total > 0.0)
            .cmp(&(*a_total > 0.0))
            .then_with(|| a_code.cmp(b_code))
    });
    Some(
        amounts
            .iter()
            .map(|(code, total)| money(&currency_sign(code), *total))
            .collect::<Vec<_>>()
            .join(" · "),
    )
}

/// `{"code":0,"data":{"available_balance":49.58},"status":true}`.
fn parse_moonshot_balance(body: &Value, sign: &str) -> Option<String> {
    Some(money(
        sign,
        number(body.pointer("/data/available_balance")?)?,
    ))
}

/// An amount given as a JSON number or a string of one.
fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse().ok())
        .filter(|number: &f64| number.is_finite())
}

fn money(sign: &str, amount: f64) -> String {
    format!("{sign}{amount:.2}")
}

fn currency_sign(code: &str) -> String {
    match code {
        "CNY" | "RMB" => "¥".into(),
        "USD" => "$".into(),
        "" => String::new(),
        other => format!("{other} "),
    }
}

/// `12`, `12.5`, `12.35`: at most two decimals, none trailing.
fn compact(amount: f64) -> String {
    let text = format!("{amount:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn chatgpt_usage_reads_windows_credits_and_reset_cards() {
        let quota = parse_chatgpt_usage(
            &json!({
                "plan_type": "plus",
                "rate_limit": {
                    "allowed": true,
                    "limit_reached": false,
                    "primary_window": {
                        "used_percent": 42.5,
                        "limit_window_seconds": 18000,
                        "reset_after_seconds": 600,
                        "reset_at": 1_800_000_600
                    },
                    "secondary_window": {
                        "used_percent": 7,
                        "limit_window_seconds": 604800,
                        "reset_after_seconds": 3600
                    }
                },
                "credits": {"has_credits": true, "unlimited": false, "balance": "1234.50"},
                "rate_limit_reset_credits": {"available_count": 2}
            }),
            1_800_000_000,
        );
        assert_eq!(quota.plan.as_deref(), Some("plus"));
        assert_eq!(
            quota.windows,
            vec![
                QuotaWindow {
                    span_seconds: Some(18_000),
                    used_percent: 42.5,
                    resets_at: Some(1_800_000_600)
                },
                QuotaWindow {
                    span_seconds: Some(604_800),
                    used_percent: 7.0,
                    resets_at: Some(1_800_003_600)
                },
            ]
        );
        assert_eq!(quota.balance.as_deref(), Some("1234.5 credits"));
        assert_eq!(quota.reset_cards, Some(2));
    }

    #[test]
    fn chatgpt_credits_hide_when_unlimited_or_spent() {
        for credits in [
            json!({"has_credits": true, "unlimited": true, "balance": "50"}),
            json!({"has_credits": true, "unlimited": false, "balance": "0"}),
            json!({"has_credits": false, "unlimited": false, "balance": "50"}),
        ] {
            let quota = parse_chatgpt_usage(&json!({"credits": credits, "rate_limit": {}}), 0);
            assert_eq!(quota.balance, None, "{credits}");
            assert!(quota.windows.is_empty());
            assert_eq!(quota.reset_cards, None);
        }
    }

    #[test]
    fn only_available_reset_cards_count() {
        let details = json!({
            "available_count": 3,
            "credits": [
                {"id": "a", "reset_type": "five_hour", "status": "available", "expires_at": "2026-12-01T00:00:00Z"},
                {"id": "b", "reset_type": "weekly", "status": "redeemed"},
                {"id": "c", "reset_type": "weekly", "status": "available", "expires_at": null}
            ]
        });
        assert_eq!(available_reset_cards(&details), 2);
        assert_eq!(available_reset_cards(&json!({})), 0);
    }

    #[test]
    fn balances_read_each_vendor_shape() {
        assert_eq!(
            parse_openrouter_balance(&json!({"data": {"total_credits": 20, "total_usage": 3.5}})),
            Some("$16.50".into())
        );
        assert_eq!(
            parse_openrouter_balance(&json!({"data": {"total_credits": "10"}})),
            Some("$10.00".into())
        );
        assert_eq!(
            parse_deepseek_balance(&json!({
                "is_available": true,
                "balance_infos": [
                    {"currency": "USD", "total_balance": "0.00", "granted_balance": "0.00", "topped_up_balance": "0.00"},
                    {"currency": "CNY", "total_balance": "110.00", "granted_balance": "10.00", "topped_up_balance": "100.00"}
                ]
            })),
            Some("¥110.00 · $0.00".into())
        );
        assert_eq!(
            parse_deepseek_balance(&json!({"balance_infos": [
                {"currency": "USD", "total_balance": "5"},
                {"currency": "CNY", "total_balance": "110"}
            ]})),
            Some("¥110.00 · $5.00".into())
        );
        assert_eq!(
            parse_moonshot_balance(
                &json!({"code": 0, "data": {"available_balance": 49.58, "voucher_balance": 0, "cash_balance": 49.58}, "status": true}),
                "¥"
            ),
            Some("¥49.58".into())
        );
        assert_eq!(parse_deepseek_balance(&json!({"balance_infos": []})), None);
        assert_eq!(parse_moonshot_balance(&json!({"data": {}}), "$"), None);
    }
}
