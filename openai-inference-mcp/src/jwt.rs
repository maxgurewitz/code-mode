use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::Value;

pub fn decode_jwt_claims(token: &str) -> Result<Value> {
    let payload = token
        .split('.')
        .nth(1)
        .context("JWT is missing a payload segment")?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .context("failed to decode JWT payload")?;
    serde_json::from_slice(&bytes).context("failed to parse JWT payload JSON")
}

pub fn extract_expiry_ms(token: &str) -> Option<u64> {
    let claims = decode_jwt_claims(token).ok()?;
    claims
        .get("exp")
        .and_then(Value::as_u64)
        .map(|seconds| seconds.saturating_mul(1000))
}

pub fn extract_account_id(token: &str) -> Option<String> {
    let claims = decode_jwt_claims(token).ok()?;
    claims
        .get("https://api.openai.com/auth")
        .and_then(|value| value.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

pub fn extract_email(token: &str) -> Option<String> {
    let claims = decode_jwt_claims(token).ok()?;
    claims
        .get("email")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use serde_json::json;

    use super::{decode_jwt_claims, extract_account_id, extract_email, extract_expiry_ms};

    fn make_token(payload: serde_json::Value) -> String {
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"none","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(payload.to_string());
        format!("{header}.{payload}.")
    }

    #[test]
    fn extracts_expected_claims() {
        let token = make_token(json!({
            "exp": 123,
            "email": "me@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acct_123"
            }
        }));

        let claims = decode_jwt_claims(&token).expect("claims");
        assert_eq!(claims["exp"], 123);
        assert_eq!(extract_expiry_ms(&token), Some(123_000));
        assert_eq!(extract_email(&token).as_deref(), Some("me@example.com"));
        assert_eq!(extract_account_id(&token).as_deref(), Some("acct_123"));
    }
}
