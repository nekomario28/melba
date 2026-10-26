//! Private, bounded stdin/stdout protocol. No credentials belong in argv.
use serde::{Deserialize, Serialize};
use std::ops::{Deref, DerefMut};
use zeroize::Zeroize;

pub const MAX_PACKET: usize = 65_536;
pub const PROOF_FILE: &str = "wrapper-auth.json";

pub fn erase(bytes: &mut [u8]) {
    bytes.zeroize();
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);
impl From<String> for Secret {
    fn from(value: String) -> Self {
        Self(value)
    }
}
impl Deref for Secret {
    type Target = String;
    fn deref(&self) -> &String {
        &self.0
    }
}
impl DerefMut for Secret {
    fn deref_mut(&mut self) -> &mut String {
        &mut self.0
    }
}
impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptKind {
    Dialog,
    Credentials,
    CredentialsAndCode,
    Code,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Button {
    pub id: u32,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prompt {
    pub id: u32,
    pub kind: PromptKind,
    pub title: String,
    pub message: String,
    pub buttons: Vec<Button>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Input {
    Credentials {
        id: u32,
        username: Secret,
        password: Secret,
    },
    Code {
        id: u32,
        code: Secret,
    },
    CredentialsAndCode {
        id: u32,
        username: Secret,
        password: Secret,
        code: Secret,
    },
    Select {
        id: u32,
        button: u32,
    },
    /// Internal commit acknowledgement, sent only for the active attempt.
    Commit,
    Cancel,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Browser { url: String },
    Prompt { prompt: Prompt },
    Checking,
    ReadyToCommit,
    Complete,
    SignedOut,
    Failed { message: String },
}

/// An account-binding marker, not a credential and not live validation.
#[derive(Serialize, Deserialize)]
pub struct Proof {
    pub version: u32,
    pub token_sha256: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn debugging_an_authentication_request_cannot_disclose_credentials() {
        let input = Input::Credentials {
            id: 1,
            username: "private@example.invalid".to_string().into(),
            password: "fixture-secret".to_string().into(),
        };
        let text = format!("{input:?}");
        assert!(!text.contains("private@"));
        assert!(!text.contains("fixture-secret"));
        assert!(text.contains("REDACTED"));
    }
}
