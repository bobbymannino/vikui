//! Minimal blocking client for the [Vikunja](https://vikunja.io) REST API.
//!
//! Calls block the current thread; run them on a background executor.

mod error;
mod project;
mod server_url;
mod task;

use std::fmt;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use ureq::Agent;
use ureq::tls::{RootCerts, TlsConfig};

pub use error::Error;
pub use project::{Project, ProjectKind};
pub use server_url::{ServerUrl, ServerUrlError};
pub use task::{
    RelativeReminder, RelativeTo, SavedFilter, SavedQuery, Task, TaskQuery, TaskReminder,
};

const TIMEOUT: Duration = Duration::from_secs(20);
const USER_AGENT: &str = concat!("vikui/", env!("CARGO_PKG_VERSION"));
/// Vikunja's default (and usual maximum) page size.
const PAGE_SIZE: u32 = 50;
/// Guards against a misbehaving server reporting endless pages.
const MAX_PAGES: u32 = 1_000;
/// Cookie Vikunja 2.0+ uses to hand out its rotating refresh token.
const REFRESH_COOKIE: &str = "vikunja_refresh_token";
/// Some servers (or proxies in front of them) reject `Accept: */*`, ureq's default.
const ACCEPT_JSON: &str = "application/json";

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

/// A refresh token (Vikunja 2.0+) that can be exchanged for a new [`Token`].
///
/// Every refresh rotates it; always keep the newest one.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RefreshToken(String);

impl RefreshToken {
    #[must_use]
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for RefreshToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RefreshToken(<redacted>)")
    }
}

/// What a login or refresh hands back.
#[derive(Clone, Debug)]
pub struct Auth {
    pub token: Token,
    /// `None` on servers older than Vikunja 2.0, whose tokens are long-lived instead.
    pub refresh_token: Option<RefreshToken>,
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
struct TokenResponse {
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

    /// Exchange credentials for a bearer token (and, on Vikunja 2.0+, a refresh token).
    pub fn login(&self, request: &LoginRequest<'_>) -> Result<Auth, Error> {
        let mut response = self
            .agent
            .post(self.server.endpoint("login").as_str())
            .header("Accept", ACCEPT_JSON)
            .send_json(request)?;
        auth_from(&mut response)
    }

    /// Trade a refresh token for a new bearer token and a new (rotated) refresh token.
    pub fn refresh(&self, refresh_token: &RefreshToken) -> Result<Auth, Error> {
        let mut response = self
            .agent
            .post(self.server.endpoint("user/token/refresh").as_str())
            .header("Accept", ACCEPT_JSON)
            .header(
                "Cookie",
                &format!("{REFRESH_COOKIE}={}", refresh_token.expose()),
            )
            .send_empty()?;
        auth_from(&mut response)
    }

    /// Every non-archived project the user can access, in the server's order.
    pub fn projects(&self, token: &Token) -> Result<Vec<Project>, Error> {
        self.get_all_pages(token, "projects", &[])
    }

    /// Every task matching `query`, across all projects the user can access.
    pub fn tasks(&self, token: &Token, query: &TaskQuery) -> Result<Vec<Task>, Error> {
        let mut params = Vec::new();
        if let Some(filter) = query.filter.as_deref().filter(|f| !f.trim().is_empty()) {
            params.push(("filter", filter.to_owned()));
            // Interpret dates in filters (e.g. `due_date < now/d`) in the user's time zone.
            if let Some(zone) = jiff::tz::TimeZone::system().iana_name() {
                params.push(("filter_timezone", zone.to_owned()));
            }
        }
        if let Some(search) = query.search.as_deref().filter(|s| !s.trim().is_empty()) {
            params.push(("s", search.to_owned()));
        }
        if query.include_nulls {
            params.push(("filter_include_nulls", "true".to_owned()));
        }
        self.get_all_pages(token, "tasks", &params)
    }

    /// One saved filter, including the query it stores.
    pub fn saved_filter(&self, token: &Token, filter_id: i64) -> Result<SavedFilter, Error> {
        let mut response = self
            .agent
            .get(
                self.server
                    .endpoint(&format!("filters/{filter_id}"))
                    .as_str(),
            )
            .header("Accept", ACCEPT_JSON)
            .header("Authorization", &format!("Bearer {}", token.expose()))
            .call()?;
        if !response.status().is_success() {
            return Err(error_from(&mut response));
        }
        Ok(response.body_mut().read_json()?)
    }

    /// `GET` a paginated collection, following pages until all items are fetched.
    fn get_all_pages<T: DeserializeOwned>(
        &self,
        token: &Token,
        path: &str,
        params: &[(&str, String)],
    ) -> Result<Vec<T>, Error> {
        let url = self.server.endpoint(path);
        let mut items = Vec::new();
        let mut page = 1;
        loop {
            let mut request = self
                .agent
                .get(url.as_str())
                .header("Accept", ACCEPT_JSON)
                .header("Authorization", &format!("Bearer {}", token.expose()))
                .query("page", page.to_string())
                .query("per_page", PAGE_SIZE.to_string());
            for (name, value) in params {
                request = request.query(*name, value);
            }
            let mut response = request.call()?;
            if !response.status().is_success() {
                return Err(error_from(&mut response));
            }

            let total_pages = response
                .headers()
                .get("x-pagination-total-pages")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.trim().parse::<u32>().ok())
                .unwrap_or(1)
                .min(MAX_PAGES);
            // Vikunja answers an empty collection with `null`.
            let batch: Option<Vec<T>> = response.body_mut().read_json()?;
            let batch = batch.unwrap_or_default();
            let done = batch.is_empty() || page >= total_pages;
            items.extend(batch);
            if done {
                return Ok(items);
            }
            page += 1;
        }
    }
}

type Response = ureq::http::Response<ureq::Body>;

fn auth_from(response: &mut Response) -> Result<Auth, Error> {
    if !response.status().is_success() {
        return Err(error_from(response));
    }
    let refresh_token = refresh_cookie(response.headers());
    let body: TokenResponse = response.body_mut().read_json()?;
    Ok(Auth {
        token: body.token,
        refresh_token,
    })
}

fn refresh_cookie(headers: &ureq::http::HeaderMap) -> Option<RefreshToken> {
    headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|cookie| cookie.split(';').next())
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, value)| *name == REFRESH_COOKIE && !value.is_empty())
        .map(|(_, value)| RefreshToken::new(value))
}

fn error_from(response: &mut Response) -> Error {
    let status = response.status().as_u16();
    let body = response.body_mut().read_to_string().unwrap_or_default();
    log::debug!("HTTP {status} from server: {body}");
    Error::from_response(status, &body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ureq::http::{HeaderMap, HeaderValue};

    #[test]
    fn reads_refresh_cookie_among_others() {
        let mut headers = HeaderMap::new();
        headers.append("set-cookie", HeaderValue::from_static("other=1; Path=/"));
        headers.append(
            "set-cookie",
            HeaderValue::from_static(
                "vikunja_refresh_token=abc.def; Path=/api/v1/user/token/refresh; HttpOnly",
            ),
        );
        assert_eq!(refresh_cookie(&headers), Some(RefreshToken::new("abc.def")));
    }

    #[test]
    fn ignores_missing_or_cleared_refresh_cookie() {
        let mut headers = HeaderMap::new();
        assert_eq!(refresh_cookie(&headers), None);
        headers.append(
            "set-cookie",
            HeaderValue::from_static("vikunja_refresh_token=; Max-Age=0"),
        );
        assert_eq!(refresh_cookie(&headers), None);
    }
}
