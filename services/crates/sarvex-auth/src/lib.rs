use anyhow::{anyhow, Context, Result};
use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::Utc;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    Demo,
    Jwt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMethod {
    Jwt,
    Demo,
    ApiKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub user_id: String,
    pub scopes: Vec<String>,
    pub method: AuthMethod,
}

#[derive(Debug, Clone)]
pub struct Authenticator {
    mode: AuthMode,
    secret: Option<String>,
    issuer: String,
    audience: String,
    ttl_seconds: i64,
    api_keys: Arc<RwLock<HashMap<String, ApiKeyRecord>>>,
}

#[derive(Debug, Clone)]
struct ApiKeyRecord {
    user_id: String,
    scopes: Vec<String>,
    expires_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Claims {
    sub: String,
    role: String,
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    #[serde(default)]
    scope: String,
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
            api_keys: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub fn mode(&self) -> AuthMode {
        self.mode
    }

    pub fn issue(&self, user_id: &str, role: &str) -> Result<String> {
        self.issue_with_scopes(user_id, role, &[])
    }

    pub fn issue_with_scopes(
        &self,
        user_id: &str,
        role: &str,
        scopes: &[String],
    ) -> Result<String> {
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
            scope: scopes.join(" "),
        };
        encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(self.secret.as_deref().unwrap_or_default().as_bytes()),
        )
        .context("failed to sign JWT")
    }

    pub fn verify(&self, token: &str) -> Result<String> {
        Ok(self.verify_principal(token)?.user_id)
    }

    pub fn verify_principal(&self, token: &str) -> Result<Principal> {
        if self.mode == AuthMode::Demo {
            let encoded = token
                .strip_prefix("demo.")
                .ok_or_else(|| anyhow!("invalid demo token"))?;
            let decoded = URL_SAFE_NO_PAD
                .decode(encoded)
                .context("invalid demo token")?;
            let user_id = String::from_utf8(decoded).context("invalid demo token")?;
            return (!user_id.trim().is_empty())
                .then_some(Principal {
                    user_id,
                    scopes: vec!["*".to_owned()],
                    method: AuthMethod::Demo,
                })
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
        Ok(Principal {
            user_id: token.claims.sub,
            scopes: token
                .claims
                .scope
                .split_whitespace()
                .filter(|scope| !scope.is_empty())
                .map(str::to_owned)
                .collect(),
            method: AuthMethod::Jwt,
        })
    }

    pub fn verify_authorization(&self, value: &str) -> Result<String> {
        let token = value
            .strip_prefix("Bearer ")
            .or_else(|| value.strip_prefix("bearer "))
            .ok_or_else(|| anyhow!("bearer token required"))?;
        self.verify(token)
    }

    pub fn verify_request(
        &self,
        authorization: Option<&str>,
        api_key: Option<&str>,
    ) -> Result<Principal> {
        if let Some(value) = authorization {
            let token = value
                .strip_prefix("Bearer ")
                .or_else(|| value.strip_prefix("bearer "))
                .ok_or_else(|| anyhow!("bearer token required"))?;
            return self.verify_principal(token);
        }
        if let Some(value) = api_key {
            return self.verify_api_key(value);
        }
        Err(anyhow!("authentication required"))
    }

    pub fn register_api_key(
        &self,
        key_hash: String,
        user_id: String,
        scopes: Vec<String>,
        expires_at: Option<i64>,
    ) {
        if let Ok(mut keys) = self.api_keys.write() {
            keys.insert(
                key_hash,
                ApiKeyRecord {
                    user_id,
                    scopes,
                    expires_at,
                },
            );
        }
    }

    pub fn revoke_api_key(&self, key_hash: &str) {
        if let Ok(mut keys) = self.api_keys.write() {
            keys.remove(key_hash);
        }
    }

    pub fn verify_api_key(&self, raw: &str) -> Result<Principal> {
        let record = self
            .api_keys
            .read()
            .map_err(|_| anyhow!("api key store unavailable"))?
            .get(&hash_api_key(raw.trim()))
            .cloned()
            .ok_or_else(|| anyhow!("invalid api key"))?;
        if record
            .expires_at
            .is_some_and(|value| value <= Utc::now().timestamp())
        {
            return Err(anyhow!("api key expired"));
        }
        Ok(Principal {
            user_id: record.user_id,
            scopes: record.scopes,
            method: AuthMethod::ApiKey,
        })
    }
}

pub fn hash_api_key(raw: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(raw.as_bytes()))
}

pub fn generate_api_key() -> (String, String, String) {
    let raw = format!(
        "svx_live_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    );
    let prefix = raw.chars().take(17).collect::<String>();
    (raw.clone(), prefix, hash_api_key(&raw))
}

pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| anyhow!("failed to hash password: {error:?}"))
}

pub fn verify_password(password: &str, encoded_hash: &str) -> Result<()> {
    let parsed = PasswordHash::new(encoded_hash)
        .map_err(|error| anyhow!("invalid password hash: {error:?}"))?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| anyhow!("invalid credentials"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn demo_auth() -> Authenticator {
        Authenticator {
            mode: AuthMode::Demo,
            secret: None,
            issuer: "s".into(),
            audience: "a".into(),
            ttl_seconds: 60,
            api_keys: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    #[test]
    fn demo_tokens_round_trip() {
        let auth = demo_auth();
        let token = auth.issue("user-1", "trader").unwrap();
        assert_eq!(auth.verify(&token).unwrap(), "user-1");
    }
    #[test]
    fn jwt_tokens_are_signed_and_round_trip() {
        let auth = Authenticator {
            mode: AuthMode::Jwt,
            secret: Some("a".repeat(32)),
            issuer: "s".into(),
            audience: "a".into(),
            ttl_seconds: 60,
            api_keys: Arc::new(RwLock::new(HashMap::new())),
        };
        let token = auth.issue("user-1", "trader").unwrap();
        assert_eq!(auth.verify(&token).unwrap(), "user-1");
        assert!(auth.verify(&(token + "x")).is_err());
    }
    #[test]
    fn password_and_api_key_round_trip() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(verify_password("correct horse battery staple", &hash).is_ok());
        assert!(verify_password("wrong", &hash).is_err());
        let (raw, _, hash) = generate_api_key();
        let auth = demo_auth();
        auth.register_api_key(hash, "user-1".into(), vec!["*".into()], None);
        assert_eq!(auth.verify_api_key(&raw).unwrap().user_id, "user-1");
    }
}
