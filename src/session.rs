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
