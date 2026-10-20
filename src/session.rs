use std::path::{Path, PathBuf};

use crate::{config, timefmt, AppError};

/// The token document and its persistence location for one command run.
pub struct Session {
    tokens: config::Tokens,
    path: PathBuf,
}

impl Session {
    pub fn load(dir: &Path) -> Result<Self, AppError> {
        Ok(Self {
            tokens: config::load_tokens(dir)?,
            path: config::tokens_path(dir),
        })
    }

    pub fn load_or_default(dir: &Path) -> Result<Self, AppError> {
        let path = config::tokens_path(dir);
        let tokens = if path.exists() {
            config::load_tokens(dir)?
        } else {
            config::Tokens::default()
        };
        Ok(Self { tokens, path })
    }

    pub fn tokens(&self) -> &config::Tokens {
        &self.tokens
    }
    pub fn tokens_mut(&mut self) -> &mut config::Tokens {
        &mut self.tokens
    }

    pub fn replace(&mut self, replace: impl FnOnce(&mut config::Tokens)) {
        replace(&mut self.tokens);
    }

    pub fn persist(&self) -> Result<(), AppError> {
        config::write_tokens(&self.path, &self.tokens)
    }

    pub fn refresh_withings_if_expiring(
        &mut self,
        client: &crate::http::HttpClient,
        client_id: &str,
        client_secret: &str,
    ) -> Result<(), AppError> {
        let now = timefmt::now_epoch();
        if self
            .tokens
            .withings
            .expires_at
            .is_some_and(|expires| expires > now as u64 + 60)
        {
            return Ok(());
        }
        self.refresh_withings(client, client_id, client_secret)
    }

    pub fn recover_withings<T>(
        &mut self,
        client: &crate::http::HttpClient,
        client_id: &str,
        client_secret: &str,
        mut operation: impl FnMut(&str) -> Result<T, crate::Fault>,
    ) -> Result<T, AppError> {
        match operation(&self.tokens.withings.access_token) {
            Ok(value) => Ok(value),
            Err(crate::Fault::Failed(error)) => Err(error),
            Err(crate::Fault::Unauthorized) => {
                self.refresh_withings(client, client_id, client_secret)?;
                match operation(&self.tokens.withings.access_token) {
                    Ok(value) => Ok(value),
                    Err(crate::Fault::Failed(error)) => Err(error),
                    Err(crate::Fault::Unauthorized) => Err(AppError::new(
                        crate::EXIT_AUTH,
                        "token refresh succeeded but Withings still rejected the token; re-run `auth withings`",
                    )),
                }
            }
        }
    }

    pub fn recover_garmin<T>(
        &mut self,
        client: &crate::http::HttpClient,
        mut operation: impl FnMut(&str) -> Result<T, crate::Fault>,
    ) -> Result<T, AppError> {
        match operation(&self.tokens.garmin.access_token) {
            Ok(value) => Ok(value),
            Err(crate::Fault::Failed(error)) => Err(error),
            Err(crate::Fault::Unauthorized) => {
                let client_id = self.tokens.garmin.client_id.clone().unwrap_or_default();
                let refreshed =
                    crate::garmin::refresh(client, &client_id, &self.tokens.garmin.refresh_token)
                        .map_err(|error| {
                        AppError::new(
                            crate::EXIT_AUTH,
                            format!(
                                "Garmin token refresh failed: {}; re-run `auth garmin`",
                                error.message
                            ),
                        )
                    })?;
                self.tokens.garmin.access_token = refreshed.access_token;
                if let Some(rotate) = refreshed.refresh_token {
                    self.tokens.garmin.refresh_token = rotate;
                }
                self.persist()?;
                match operation(&self.tokens.garmin.access_token) {
                    Ok(value) => Ok(value),
                    Err(crate::Fault::Unauthorized) => Err(AppError::new(
                        crate::EXIT_METRIC_FAILURE,
                        "unauthorized (HTTP 401)",
                    )),
                    Err(crate::Fault::Failed(error)) => Err(error),
                }
            }
        }
    }

    fn refresh_withings(
        &mut self,
        client: &crate::http::HttpClient,
        client_id: &str,
        client_secret: &str,
    ) -> Result<(), AppError> {
        let refreshed = crate::withings::refresh(
            client,
            client_id,
            client_secret,
            &self.tokens.withings.refresh_token,
        )
        .map_err(|error| {
            AppError::new(
                crate::EXIT_AUTH,
                format!(
                    "Withings token refresh failed: {}; re-run `auth withings`",
                    error.message
                ),
            )
        })?;
        self.tokens.withings.access_token = refreshed.access_token;
        if let Some(rotate) = refreshed.refresh_token {
            self.tokens.withings.refresh_token = rotate;
        }
        self.tokens.withings.expires_at = Some(timefmt::now_epoch() as u64 + refreshed.expires_in);
        self.persist()
    }
}
