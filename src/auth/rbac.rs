//! Role enum with capability helpers.
//! Mirrors Python roles: admin | super_user | user (MAPPA §3).

use std::{fmt, str::FromStr};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Admin,
    SuperUser,
    User,
}

#[allow(dead_code)]
impl Role {
    pub fn can_upload(self) -> bool {
        matches!(self, Role::Admin | Role::SuperUser)
    }

    pub fn can_delete(self) -> bool {
        matches!(self, Role::Admin | Role::SuperUser)
    }

    pub fn can_manage_users(self) -> bool {
        matches!(self, Role::Admin)
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Role::Admin => write!(f, "admin"),
            Role::SuperUser => write!(f, "super_user"),
            Role::User => write!(f, "user"),
        }
    }
}

impl FromStr for Role {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "admin" => Ok(Role::Admin),
            "super_user" => Ok(Role::SuperUser),
            "user" => Ok(Role::User),
            other => Err(anyhow::anyhow!("unknown role: {other}")),
        }
    }
}
