//! Minimal blocking client for the [Vikunja](https://vikunja.io) REST API.
//!
//! Calls block the current thread; run them on a background executor.

mod error;
mod server_url;

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use ureq::Agent;
use ureq::tls::{RootCerts, TlsConfig};

pub use error::Error;
pub use server_url::{ServerUrl, ServerUrlError};

const TIMEOUT: Duration = Duration::from_secs(20);
const USER_AGENT: &str = concat!("vikui/", env!("CARGO_PKG_VERSION"));

/// A bearer token returned by a successful login.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Token(String);

impl Token {
    #[must_use]
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

/// Credentials for `POST /login`.
#[derive(Clone, Serialize)]
pub struct LoginRequest<'a> {
    pub username: &'a str,
    pub password: &'a str,
    /// One-time code for accounts with two-factor authentication enabled.
    #[serde(rename = "totp_passcode", skip_serializing_if = "Option::is_none")]
    pub totp: Option<&'a str>,
    /// Ask the server for a long-lived token so users stay signed in.
    pub long_token: bool,
}

impl fmt::Debug for LoginRequest<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoginRequest")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("totp", &self.totp.map(|_| "<redacted>"))
            .field("long_token", &self.long_token)
            .finish()
    }
}

#[derive(Deserialize)]
struct LoginResponse {
    token: Token,
}

/// A client bound to one Vikunja server.
#[derive(Clone, Debug)]
pub struct Client {
    agent: Agent,
    server: ServerUrl,
}

impl Client {
    #[must_use]
    pub fn new(server: ServerUrl) -> Self {
        let agent = Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .user_agent(USER_AGENT)
            // Use the OS trust store so self-hosted servers with private CAs work.
            .tls_config(
                TlsConfig::builder()
                    .root_certs(RootCerts::PlatformVerifier)
                    .build(),
            )
            // Inspect error bodies ourselves; Vikunja returns `{ code, message }`.
            .http_status_as_error(false)
            .build()
            .into();
        Self { agent, server }
    }

    #[must_use]
    pub fn server(&self) -> &ServerUrl {
        &self.server
    }

    /// Exchange credentials for a bearer token.
    pub fn login(&self, request: &LoginRequest<'_>) -> Result<Token, Error> {
        let mut response = self
            .agent
            .post(self.server.endpoint("login").as_str())
            .send_json(request)?;

        if response.status().is_success() {
            let body: LoginResponse = response.body_mut().read_json()?;
            Ok(body.token)
        } else {
            let status = response.status().as_u16();
            let body = response.body_mut().read_to_string().unwrap_or_default();
            Err(Error::from_response(status, &body))
        }
    }
}
