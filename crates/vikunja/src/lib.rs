//! Minimal blocking client for the [Vikunja](https://vikunja.io) REST API.
//!
//! Calls block the current thread; run them on a background executor.

mod error;
mod new_task;
mod project;
mod server_url;
mod task;
mod task_update;

use std::fmt;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use ureq::Agent;
use ureq::tls::{RootCerts, TlsConfig};

pub use error::Error;
pub use new_task::NewTask;
pub use project::{Project, ProjectKind};
pub use server_url::{ServerUrl, ServerUrlError};
pub use task::{
    Label, RelativeReminder, RelativeTo, Repeat, SavedFilter, SavedQuery, Task, TaskQuery,
    TaskReminder,
};
pub use task_update::{Change, TaskUpdate};

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

    /// Every label the user can see: their own and those on tasks they can access.
    pub fn labels(&self, token: &Token) -> Result<Vec<Label>, Error> {
        self.get_all_pages(token, "labels", &[])
    }

    /// The project new tasks go to by default, from the user's settings; `None` if the
    /// user hasn't chosen one.
    pub fn default_project_id(&self, token: &Token) -> Result<Option<i64>, Error> {
        let user: CurrentUser = self.get_json(token, "user")?;
        Ok(Some(user.settings.default_project_id).filter(|&id| id > 0))
    }

    /// Create `task` in `project_id`, attach its labels, and return it as saved.
    ///
    /// The task exists once the first request succeeds, so a label that can't be
    /// attached is logged rather than failing the call (which would invite creating it
    /// again); compare the returned task's labels with the ones asked for.
    pub fn create_task(
        &self,
        token: &Token,
        project_id: i64,
        task: &NewTask,
    ) -> Result<Task, Error> {
        let created: Task = self.put_json(
            token,
            &format!("projects/{project_id}/tasks"),
            &task.to_json(),
        )?;
        if task.label_ids.is_empty() {
            return Ok(created);
        }
        let path = format!("tasks/{}/labels", created.id);
        for &label_id in &task.label_ids {
            let attached: Result<serde_json::Value, Error> =
                self.put_json(token, &path, &serde_json::json!({ "label_id": label_id }));
            if let Err(err) = attached {
                log::warn!(
                    "couldn't add label {label_id} to task {}: {err}",
                    created.id
                );
            }
        }
        self.get_json(token, &format!("tasks/{}", created.id))
            .or_else(|err| {
                log::warn!("couldn't reload task {}: {err}", created.id);
                Ok(created)
            })
    }

    /// One task, as the server has it now.
    pub fn task(&self, token: &Token, task_id: i64) -> Result<Task, Error> {
        self.get_json(token, &format!("tasks/{task_id}"))
    }

    /// Apply `update` to `task_id` and return the task as saved.
    ///
    /// Vikunja's update replaces every field, so this reads the whole task, changes
    /// only what `update` names, and sends the rest back unchanged. Labels are added and
    /// removed after; one that fails is logged, since the other changes are saved by
    /// then. Compare the returned task's labels with the ones asked for.
    pub fn update_task(
        &self,
        token: &Token,
        task_id: i64,
        update: &TaskUpdate,
    ) -> Result<Task, Error> {
        let path = format!("tasks/{task_id}");
        if update.changes_fields() {
            let mut task: serde_json::Value = self.get_json(token, &path)?;
            update.apply(&mut task);
            let _saved: serde_json::Value = self.post_json(token, &path, &task)?;
        }
        let labels = format!("tasks/{task_id}/labels");
        for &label_id in &update.add_label_ids {
            let added: Result<serde_json::Value, Error> =
                self.put_json(token, &labels, &serde_json::json!({ "label_id": label_id }));
            if let Err(err) = added {
                log::warn!("couldn't add label {label_id} to task {task_id}: {err}");
            }
        }
        for &label_id in &update.remove_label_ids {
            if let Err(err) = self.delete(token, &format!("{labels}/{label_id}")) {
                log::warn!("couldn't remove label {label_id} from task {task_id}: {err}");
            }
        }
        self.get_json(token, &path)
    }

    /// One saved filter, including the query it stores.
    pub fn saved_filter(&self, token: &Token, filter_id: i64) -> Result<SavedFilter, Error> {
        self.get_json(token, &format!("filters/{filter_id}"))
    }

    /// Mark a task done (or not done) and return it as the server saved it.
    ///
    /// Vikunja's update replaces every field, so this reads the whole task, changes only
    /// `done`, and sends the rest back unchanged.
    pub fn set_task_done(&self, token: &Token, task_id: i64, done: bool) -> Result<Task, Error> {
        let path = format!("tasks/{task_id}");
        let mut task: serde_json::Value = self.get_json(token, &path)?;
        set_done(&mut task, done);
        self.post_json(token, &path, &task)
    }

    fn get_json<T: DeserializeOwned>(&self, token: &Token, path: &str) -> Result<T, Error> {
        let mut response = self
            .agent
            .get(self.server.endpoint(path).as_str())
            .header("Accept", ACCEPT_JSON)
            .header("Authorization", &format!("Bearer {}", token.expose()))
            .call()?;
        if !response.status().is_success() {
            return Err(error_from(&mut response));
        }
        Ok(response.body_mut().read_json()?)
    }

    fn post_json<T: DeserializeOwned>(
        &self,
        token: &Token,
        path: &str,
        body: &impl Serialize,
    ) -> Result<T, Error> {
        let mut response = self
            .agent
            .post(self.server.endpoint(path).as_str())
            .header("Accept", ACCEPT_JSON)
            .header("Authorization", &format!("Bearer {}", token.expose()))
            .send_json(body)?;
        if !response.status().is_success() {
            return Err(error_from(&mut response));
        }
        Ok(response.body_mut().read_json()?)
    }

    fn put_json<T: DeserializeOwned>(
        &self,
        token: &Token,
        path: &str,
        body: &impl Serialize,
    ) -> Result<T, Error> {
        let mut response = self
            .agent
            .put(self.server.endpoint(path).as_str())
            .header("Accept", ACCEPT_JSON)
            .header("Authorization", &format!("Bearer {}", token.expose()))
            .send_json(body)?;
        if !response.status().is_success() {
            return Err(error_from(&mut response));
        }
        Ok(response.body_mut().read_json()?)
    }

    fn delete(&self, token: &Token, path: &str) -> Result<(), Error> {
        let mut response = self
            .agent
            .delete(self.server.endpoint(path).as_str())
            .header("Accept", ACCEPT_JSON)
            .header("Authorization", &format!("Bearer {}", token.expose()))
            .call()?;
        if !response.status().is_success() {
            return Err(error_from(&mut response));
        }
        Ok(())
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

/// The parts of `GET /user` Vikui reads.
#[derive(Deserialize)]
struct CurrentUser {
    #[serde(default)]
    settings: UserSettings,
}

#[derive(Default, Deserialize)]
struct UserSettings {
    /// `0` when unset.
    #[serde(default)]
    default_project_id: i64,
}

/// Set `done` on a task's JSON, leaving every other field as the server sent it.
fn set_done(task: &mut serde_json::Value, done: bool) {
    if let Some(fields) = task.as_object_mut() {
        fields.insert("done".to_owned(), serde_json::Value::Bool(done));
    }
}

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
    fn reads_the_default_project() {
        let user: CurrentUser =
            serde_json::from_str(r#"{"id":1,"settings":{"default_project_id":4}}"#).unwrap();
        assert_eq!(user.settings.default_project_id, 4);
        let unset: CurrentUser = serde_json::from_str(r#"{"id":1}"#).unwrap();
        assert_eq!(unset.settings.default_project_id, 0);
    }

    #[test]
    fn set_done_keeps_other_fields() {
        let mut task = serde_json::json!({
            "id": 7, "title": "Pay rent", "done": false, "description": "<p>x</p>",
            "labels": [{"id": 1}], "repeat_after": 86400
        });
        set_done(&mut task, true);
        assert_eq!(
            task,
            serde_json::json!({
                "id": 7, "title": "Pay rent", "done": true, "description": "<p>x</p>",
                "labels": [{"id": 1}], "repeat_after": 86400
            })
        );
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
