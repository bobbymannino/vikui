//! Persists the signed-in account in the OS credential store
//! (macOS Keychain, Windows Credential Manager, Secret Service on Linux).
//!
//! All functions block on the platform store; call them from a background task.

use std::fmt;

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use vikunja::{RefreshToken, ServerUrl, Token};

const SERVICE: &str = "vikui";
const ACCOUNT: &str = "session";

/// Everything needed to talk to the server as the signed-in user.
///
/// The password is kept so the session can be renewed once the token expires.
#[derive(Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub server: ServerUrl,
    pub username: String,
    pub password: String,
    pub token: Token,
    /// Present on Vikunja 2.0+; absent in sessions saved before refresh support.
    #[serde(default)]
    pub refresh_token: Option<RefreshToken>,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("server", &self.server)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("token", &self.token)
            .field("refresh_token", &self.refresh_token)
            .finish()
    }
}

fn entry() -> anyhow::Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, ACCOUNT).context("credential store is unavailable")
}

/// Load saved credentials; `Ok(None)` when nobody is signed in.
pub fn load() -> anyhow::Result<Option<Credentials>> {
    let secret = match entry()?.get_password() {
        Ok(secret) => secret,
        Err(keyring::Error::NoEntry) => return Ok(None),
        Err(err) => return Err(err).context("failed to read credentials"),
    };
    let credentials = serde_json::from_str(&secret).context("stored credentials are corrupt")?;
    Ok(Some(credentials))
}

pub fn save(credentials: &Credentials) -> anyhow::Result<()> {
    let secret = serde_json::to_string(credentials).context("failed to encode credentials")?;
    entry()?
        .set_password(&secret)
        .context("failed to save credentials")
}

/// Remove saved credentials. Succeeds if none were stored.
pub fn clear() -> anyhow::Result<()> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(err).context("failed to delete credentials"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_secrets() {
        let creds = Credentials {
            server: ServerUrl::parse("x.io").unwrap(),
            username: "ana".into(),
            password: "hunter2".into(),
            token: Token::new("jwt-secret"),
            refresh_token: Some(RefreshToken::new("refresh-secret")),
        };
        let debug = format!("{creds:?}");
        assert!(debug.contains("ana"));
        assert!(!debug.contains("hunter2"));
        assert!(!debug.contains("jwt-secret"));
        assert!(!debug.contains("refresh-secret"));
    }

    #[test]
    fn round_trips_through_json() {
        let creds = Credentials {
            server: ServerUrl::parse("https://x.io/api/v1").unwrap(),
            username: "ana".into(),
            password: "p".into(),
            token: Token::new("t"),
            refresh_token: None,
        };
        let json = serde_json::to_string(&creds).unwrap();
        let back: Credentials = serde_json::from_str(&json).unwrap();
        assert_eq!(back.server, creds.server);
        assert_eq!(back.token, creds.token);
    }

    #[test]
    fn loads_sessions_saved_before_refresh_tokens() {
        let json = r#"{"server":"https://x.io","username":"ana","password":"p","token":"t"}"#;
        let creds: Credentials = serde_json::from_str(json).unwrap();
        assert!(creds.refresh_token.is_none());
    }
}
