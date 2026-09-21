use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::Utc;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    Demo,
    Jwt,
}

#[derive(Debug, Clone)]
pub struct Authenticator {
    mode: AuthMode,
    secret: Option<String>,
    issuer: String,
    audience: String,
    ttl_seconds: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Claims {
    sub: String,
    role: String,
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
}

impl Authenticator {
    pub fn from_env() -> Result<Self> {
        let mode = match std::env::var("AUTH_MODE")
            .unwrap_or_else(|_| "demo".to_owned())
            .to_ascii_lowercase()
            .as_str()
        {
            "demo" => AuthMode::Demo,
            "jwt" | "production" => AuthMode::Jwt,
            other => return Err(anyhow!("unsupported AUTH_MODE: {other}")),
        };
        let secret = std::env::var("JWT_SECRET").ok();
        if mode == AuthMode::Jwt && secret.as_deref().unwrap_or("").len() < 32 {
            return Err(anyhow!("JWT_SECRET must be at least 32 bytes in jwt mode"));
        }
        Ok(Self {
            mode,
            secret,
            issuer: std::env::var("JWT_ISSUER").unwrap_or_else(|_| "sarvex".to_owned()),
            audience: std::env::var("JWT_AUDIENCE").unwrap_or_else(|_| "sarvex-client".to_owned()),
            ttl_seconds: std::env::var("JWT_TTL_SECONDS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(3600),
        })
    }

    pub fn mode(&self) -> AuthMode {
        self.mode
    }

    pub fn issue(&self, user_id: &str, role: &str) -> Result<String> {
        let user_id = user_id.trim();
        if user_id.is_empty() {
            return Err(anyhow!("user_id is required"));
        }
        if self.mode == AuthMode::Demo {
            return Ok(format!(
                "demo.{}",
                URL_SAFE_NO_PAD.encode(user_id.as_bytes())
            ));
        }
        let now = Utc::now().timestamp();
        let claims = Claims {
            sub: user_id.to_owned(),
            role: role.to_owned(),
            iss: self.issuer.clone(),
            aud: self.audience.clone(),
            iat: now,
            exp: now.saturating_add(self.ttl_seconds),
        };
        encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(self.secret.as_deref().unwrap_or_default().as_bytes()),
        )
        .context("failed to sign JWT")
    }

    pub fn verify(&self, token: &str) -> Result<String> {
        if self.mode == AuthMode::Demo {
            let encoded = token
                .strip_prefix("demo.")
                .ok_or_else(|| anyhow!("invalid demo token"))?;
            let decoded = URL_SAFE_NO_PAD
                .decode(encoded)
                .context("invalid demo token")?;
            let user_id = String::from_utf8(decoded).context("invalid demo token")?;
            return (!user_id.trim().is_empty())
                .then_some(user_id)
                .ok_or_else(|| anyhow!("invalid demo token"));
        }
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(std::slice::from_ref(&self.issuer));
        validation.set_audience(std::slice::from_ref(&self.audience));
        let token = decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.secret.as_deref().unwrap_or_default().as_bytes()),
            &validation,
        )?;
        Ok(token.claims.sub)
    }

    pub fn verify_authorization(&self, value: &str) -> Result<String> {
        let token = value
            .strip_prefix("Bearer ")
            .or_else(|| value.strip_prefix("bearer "))
            .ok_or_else(|| anyhow!("bearer token required"))?;
        self.verify(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_tokens_round_trip() {
        let auth = Authenticator {
            mode: AuthMode::Demo,
            secret: None,
            issuer: "s".into(),
            audience: "a".into(),
            ttl_seconds: 60,
        };
        let token = auth.issue("user-1", "trader").expect("token");
        assert_eq!(auth.verify(&token).expect("verify"), "user-1");
    }

    #[test]
    fn jwt_tokens_are_signed_and_round_trip() {
        let auth = Authenticator {
            mode: AuthMode::Jwt,
            secret: Some("a".repeat(32)),
            issuer: "s".into(),
            audience: "a".into(),
            ttl_seconds: 60,
        };
        let token = auth.issue("user-1", "trader").expect("token");
        assert_eq!(auth.verify(&token).expect("verify"), "user-1");
        assert!(auth.verify(&(token + "x")).is_err());
    }
}
