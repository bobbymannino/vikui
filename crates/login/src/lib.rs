//! Signing in to a Vikunja server and remembering the session.
//!
//! Everything here blocks (network and OS keychain); call it from a background task.

pub mod credentials;

use std::fmt;

use vikunja::{Client, LoginRequest, ServerUrl};

pub use credentials::Credentials;

/// What the user typed into the sign-in form.
#[derive(Clone)]
pub struct SignIn {
    pub server: ServerUrl,
    pub username: String,
    pub password: String,
    /// Two-factor code, for accounts that require one.
    pub totp: Option<String>,
}

impl fmt::Debug for SignIn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignIn")
            .field("server", &self.server)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("totp", &self.totp.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Api(#[from] vikunja::Error),
    #[error("Signed in, but the credentials could not be saved to the system keychain.")]
    Keychain(#[source] anyhow::Error),
}

/// Authenticate against the server and persist the session in the OS keychain.
pub fn sign_in(request: SignIn) -> Result<Credentials, Error> {
    let SignIn {
        server,
        username,
        password,
        totp,
    } = request;

    let token = Client::new(server.clone()).login(&LoginRequest {
        username: &username,
        password: &password,
        totp: totp.as_deref(),
        long_token: true,
    })?;

    let credentials = Credentials {
        server,
        username,
        password,
        token,
    };
    credentials::save(&credentials).map_err(Error::Keychain)?;
    Ok(credentials)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_in_debug_redacts_secrets() {
        let request = SignIn {
            server: ServerUrl::parse("x.io").unwrap(),
            username: "ana".into(),
            password: "hunter2".into(),
            totp: Some("123456".into()),
        };
        let debug = format!("{request:?}");
        assert!(debug.contains("ana"));
        assert!(!debug.contains("hunter2"));
        assert!(!debug.contains("123456"));
    }
}
