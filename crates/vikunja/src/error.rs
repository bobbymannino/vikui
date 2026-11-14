use serde::Deserialize;

/// Vikunja error codes (upstream `pkg/user/error.go`).
mod code {
    pub(super) const WRONG_USERNAME_OR_PASSWORD: u32 = 1011;
    pub(super) const EMAIL_NOT_CONFIRMED: u32 = 1012;
    pub(super) const INVALID_TOTP_PASSCODE: u32 = 1017;
    pub(super) const ACCOUNT_DISABLED: u32 = 1020;
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Incorrect username or password.")]
    InvalidCredentials,

    /// The account has two-factor authentication enabled and no (or a wrong) code was sent.
    #[error("Enter the code from your authenticator app.")]
    TotpRequired,

    #[error("Confirm your email address before signing in.")]
    EmailNotConfirmed,

    #[error("This account has been disabled.")]
    AccountDisabled,

    /// The token is missing, invalid or expired.
    #[error("Your session has expired. Sign in again.")]
    Unauthorized,

    #[error("{message}")]
    Api {
        status: u16,
        code: Option<u32>,
        message: String,
    },

    #[error("Could not reach the server: {0}")]
    Network(#[from] ureq::Error),
}

#[derive(Deserialize)]
struct ApiErrorBody {
    code: Option<u32>,
    message: Option<String>,
}

impl Error {
    pub(crate) fn from_response(status: u16, body: &str) -> Self {
        let parsed = serde_json::from_str::<ApiErrorBody>(body).ok();
        let code = parsed.as_ref().and_then(|b| b.code);

        match code {
            Some(code::WRONG_USERNAME_OR_PASSWORD) => Self::InvalidCredentials,
            Some(code::INVALID_TOTP_PASSCODE) => Self::TotpRequired,
            Some(code::EMAIL_NOT_CONFIRMED) => Self::EmailNotConfirmed,
            Some(code::ACCOUNT_DISABLED) => Self::AccountDisabled,
            _ if status == 401 => Self::Unauthorized,
            _ => Self::Api {
                status,
                code,
                message: parsed
                    .and_then(|b| b.message)
                    .filter(|m| !m.trim().is_empty())
                    .unwrap_or_else(|| format!("The server responded with HTTP {status}.")),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_known_codes() {
        let wrong = r#"{"code":1011,"message":"Wrong username or password."}"#;
        assert!(matches!(
            Error::from_response(412, wrong),
            Error::InvalidCredentials
        ));
        let totp = r#"{"code":1017,"message":"Invalid totp passcode."}"#;
        assert!(matches!(
            Error::from_response(412, totp),
            Error::TotpRequired
        ));
    }

    #[test]
    fn maps_401_to_unauthorized() {
        let expired = r#"{"code":11,"message":"missing, malformed, expired or otherwise invalid token provided"}"#;
        assert!(matches!(
            Error::from_response(401, expired),
            Error::Unauthorized
        ));
    }

    #[test]
    fn falls_back_to_server_message_then_status() {
        let other = r#"{"code":9999,"message":"Nope"}"#;
        assert_eq!(Error::from_response(400, other).to_string(), "Nope");
        assert_eq!(
            Error::from_response(502, "<html>bad gateway</html>").to_string(),
            "The server responded with HTTP 502."
        );
    }
}
