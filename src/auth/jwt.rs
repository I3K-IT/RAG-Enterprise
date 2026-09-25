//! JWT HS256: create / decode / verify.
//! Claims: { user_id, username, role, exp }.
//! Expiry: 480 min (configurable via Settings).

use anyhow::Result;
use chrono::Utc;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::auth::rbac::Role;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub user_id: i64,
    pub username: String,
    pub role: Role,
    pub exp: u64,
}

pub fn create_token(
    user_id: i64,
    username: &str,
    role: Role,
    secret: &str,
    expiry_minutes: u64,
) -> Result<String> {
    // Checked arithmetic: `Settings::load` already rejects an overflowing
    // expiry, but Settings is also constructible programmatically (by another
    // launcher, or a test), so signing must never panic (debug) or wrap
    // (release) on it — return an error instead.
    let ttl_secs = expiry_minutes
        .checked_mul(60)
        .ok_or_else(|| anyhow::anyhow!("jwt expiry out of range"))?;
    let exp = (Utc::now().timestamp() as u64)
        .checked_add(ttl_secs)
        .ok_or_else(|| anyhow::anyhow!("jwt expiry out of range"))?;
    let claims = Claims { user_id, username: username.to_owned(), role, exp };
    let token = encode(&Header::default(), &claims, &EncodingKey::from_secret(secret.as_bytes()))?;
    Ok(token)
}

pub fn decode_token(token: &str, secret: &str) -> Result<Claims> {
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )?;
    Ok(data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn roundtrip_preserves_claims() {
        let token = create_token(7, "alice", Role::User, SECRET, 60).unwrap();
        let claims = decode_token(&token, SECRET).unwrap();
        assert_eq!(claims.user_id, 7);
        assert_eq!(claims.username, "alice");
        assert_eq!(claims.role, Role::User);
    }

    #[test]
    fn wrong_secret_fails_to_decode() {
        let token = create_token(7, "alice", Role::User, SECRET, 60).unwrap();
        assert!(decode_token(&token, "fedcba9876543210fedcba9876543210").is_err());
    }

    #[test]
    fn overflowing_expiry_is_an_error_not_a_panic() {
        assert!(create_token(7, "alice", Role::User, SECRET, u64::MAX).is_err());
    }
}
