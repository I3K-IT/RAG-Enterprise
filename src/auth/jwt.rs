//! JWT HS256: create / decode / verify.
//! Claims mirror MAPPA §3: { user_id, username, role, exp }.
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
    let exp = (Utc::now().timestamp() as u64) + expiry_minutes * 60;
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
