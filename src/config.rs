use std::{fs, path::Path};

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TuicalConfig {
    #[serde(default)]
    pub user: UserConfig,
    pub google: Option<GoogleConfig>,
    #[serde(default)]
    pub ical: Vec<IcalConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserConfig {
    #[serde(default = "default_timezone")]
    pub timezone: String,
}

impl Default for UserConfig {
    fn default() -> Self {
        Self {
            timezone: default_timezone(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct GoogleConfig {
    pub client_id: String,
    #[allow(dead_code)]
    #[serde(default)]
    pub client_secret: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IcalConfig {
    pub name: String,
    #[allow(dead_code)]
    pub url: String,
    #[serde(default = "default_calendar_color")]
    pub color: String,
}

impl TuicalConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();

        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(path)
            .with_context(|| format!("failed to read config from {}", path.display()))?;
        toml::from_str(&contents)
            .with_context(|| format!("failed to parse config from {}", path.display()))
    }

    pub fn google_is_configured(&self) -> bool {
        self.google
            .as_ref()
            .is_some_and(|google| !google.client_id.trim().is_empty())
    }

    pub fn timezone_label(&self) -> &str {
        if self.user.timezone.trim().is_empty() {
            "local"
        } else {
            self.user.timezone.as_str()
        }
    }
}

fn default_timezone() -> String {
    "local".to_string()
}

fn default_calendar_color() -> String {
    "cyan".to_string()
}
