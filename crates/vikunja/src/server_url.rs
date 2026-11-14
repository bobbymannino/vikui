use std::fmt;

use serde::{Deserialize, Serialize};
use url::Url;

/// The root URL of a Vikunja server, normalised so that API paths can be joined onto it.
///
/// Accepts what users typically paste: a bare host (`tasks.example.com`, assumed HTTPS),
/// a full URL, a trailing slash, or a URL already ending in `/api/v1`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ServerUrl(Url);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ServerUrlError {
    #[error("Enter a server address.")]
    Empty,
    #[error("“{0}” is not a valid server address.")]
    Invalid(String),
    #[error("Server address must use http or https.")]
    UnsupportedScheme,
}

impl ServerUrl {
    pub fn parse(input: &str) -> Result<Self, ServerUrlError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(ServerUrlError::Empty);
        }

        let with_scheme = if input.contains("://") {
            input.to_owned()
        } else {
            format!("https://{input}")
        };

        let mut url =
            Url::parse(&with_scheme).map_err(|_| ServerUrlError::Invalid(input.to_owned()))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(ServerUrlError::UnsupportedScheme);
        }
        if url.host_str().is_none_or(str::is_empty) {
            return Err(ServerUrlError::Invalid(input.to_owned()));
        }

        url.set_query(None);
        url.set_fragment(None);

        let path = url.path().trim_end_matches('/');
        let path = path.strip_suffix("/api/v1").unwrap_or(path);
        let path = format!("{path}/");
        url.set_path(&path);

        Ok(Self(url))
    }

    /// The URL for an API endpoint, e.g. `endpoint("login")` → `…/api/v1/login`.
    #[must_use]
    pub fn endpoint(&self, path: &str) -> Url {
        let mut url = self.0.clone();
        let base = url.path().to_owned();
        url.set_path(&format!("{base}api/v1/{}", path.trim_start_matches('/')));
        url
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for ServerUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str().trim_end_matches('/'))
    }
}

impl fmt::Debug for ServerUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ServerUrl({})", self.0)
    }
}

impl TryFrom<String> for ServerUrl {
    type Error = ServerUrlError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<ServerUrl> for String {
    fn from(value: ServerUrl) -> Self {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> String {
        ServerUrl::parse(input).unwrap().as_str().to_owned()
    }

    #[test]
    fn bare_host_defaults_to_https() {
        assert_eq!(parse("tasks.example.com"), "https://tasks.example.com/");
    }

    #[test]
    fn strips_trailing_slash_api_suffix_query_and_fragment() {
        assert_eq!(parse("  https://x.io/  "), "https://x.io/");
        assert_eq!(parse("https://x.io/api/v1"), "https://x.io/");
        assert_eq!(parse("https://x.io/api/v1/"), "https://x.io/");
        assert_eq!(
            parse("http://x.io:3456/sub/api/v1?a=1#f"),
            "http://x.io:3456/sub/"
        );
    }

    #[test]
    fn builds_endpoints() {
        let url = ServerUrl::parse("https://x.io/sub").unwrap();
        assert_eq!(
            url.endpoint("login").as_str(),
            "https://x.io/sub/api/v1/login"
        );
        assert_eq!(
            url.endpoint("/info").as_str(),
            "https://x.io/sub/api/v1/info"
        );
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(ServerUrl::parse("   "), Err(ServerUrlError::Empty));
        assert_eq!(
            ServerUrl::parse("ftp://x.io"),
            Err(ServerUrlError::UnsupportedScheme)
        );
        assert!(matches!(
            ServerUrl::parse("https://"),
            Err(ServerUrlError::Invalid(_))
        ));
    }

    #[test]
    fn display_and_serde_round_trip() {
        let url = ServerUrl::parse("x.io/api/v1").unwrap();
        assert_eq!(url.to_string(), "https://x.io");
        let json = serde_json::to_string(&url).unwrap();
        assert_eq!(json, "\"https://x.io\"");
        assert_eq!(serde_json::from_str::<ServerUrl>(&json).unwrap(), url);
    }
}
