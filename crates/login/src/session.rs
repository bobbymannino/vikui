//! A signed-in session that renews its token when the server rejects it.

use std::sync::{Mutex, MutexGuard, PoisonError};

use vikunja::{Auth, Client, LoginRequest, ServerUrl, Token};

use crate::credentials::{self, Credentials};

/// Shares one set of credentials between API calls and renews them on `401`.
///
/// Renewal tries the refresh token first (Vikunja 2.0+), then signs in again with the
/// saved password. Renewed credentials are written back to the keychain.
///
/// Everything here blocks; call it from a background task.
pub struct Session {
    client: Client,
    credentials: Mutex<Credentials>,
}

impl Session {
    #[must_use]
    pub fn new(credentials: Credentials) -> Self {
        Self {
            client: Client::new(credentials.server.clone()),
            credentials: Mutex::new(credentials),
        }
    }

    /// The server this session signs in to.
    #[must_use]
    pub fn server(&self) -> ServerUrl {
        self.lock().server.clone()
    }

    /// A copy of the current credentials, e.g. to pre-fill the sign-in form.
    #[must_use]
    pub fn credentials(&self) -> Credentials {
        self.lock().clone()
    }

    /// Run `call` with the current token. If the server answers
    /// [`vikunja::Error::Unauthorized`], renew the session once and retry.
    ///
    /// Still `Unauthorized` afterwards means the user has to sign in again.
    pub fn call<T>(
        &self,
        call: impl Fn(&Client, &Token) -> Result<T, vikunja::Error>,
    ) -> Result<T, vikunja::Error> {
        let token = self.lock().token.clone();
        match call(&self.client, &token) {
            Err(vikunja::Error::Unauthorized) => {
                let token = self.renew(&token)?;
                call(&self.client, &token)
            }
            result => result,
        }
    }

    fn renew(&self, rejected: &Token) -> Result<Token, vikunja::Error> {
        // Holding the lock across the network call stops two callers from spending the
        // same (single-use) refresh token.
        let mut credentials = self.lock();
        if credentials.token != *rejected {
            // Another call renewed the session while this one was waiting.
            return Ok(credentials.token.clone());
        }

        let auth = self
            .refresh(&credentials)
            .map_or_else(|| self.sign_in_again(&credentials), Ok)?;
        credentials.token = auth.token;
        credentials.refresh_token = auth.refresh_token;
        if let Err(err) = credentials::save(&credentials) {
            log::warn!("renewed session could not be saved: {err:#}");
        }
        Ok(credentials.token.clone())
    }

    fn refresh(&self, credentials: &Credentials) -> Option<Auth> {
        let refresh_token = credentials.refresh_token.as_ref()?;
        self.client
            .refresh(refresh_token)
            .inspect_err(|err| log::info!("token refresh failed: {err}"))
            .ok()
    }

    fn sign_in_again(&self, credentials: &Credentials) -> Result<Auth, vikunja::Error> {
        let result = self.client.login(&LoginRequest {
            username: &credentials.username,
            password: &credentials.password,
            totp: None,
            long_token: true,
        });
        match result {
            Ok(auth) => Ok(auth),
            // Keep connectivity problems distinct from "you need to sign in".
            Err(err @ vikunja::Error::Network(_)) => Err(err),
            Err(err) => {
                log::info!("signing in with saved password failed: {err}");
                Err(vikunja::Error::Unauthorized)
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, Credentials> {
        self.credentials
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}
