use std::{collections::HashSet, fs, path::Path};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::model::CalendarView;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub hidden_calendar_ids: Vec<String>,
    #[serde(default)]
    pub last_view: Option<CalendarView>,
}

impl AppSettings {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(path)
            .with_context(|| format!("failed to read settings from {}", path.display()))?;
        toml::from_str(&contents)
            .with_context(|| format!("failed to parse settings from {}", path.display()))
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let contents = toml::to_string_pretty(self)?;
        fs::write(path, contents).context("failed to write settings file")
    }

    pub fn hidden_calendar_set(&self) -> HashSet<String> {
        self.hidden_calendar_ids.iter().cloned().collect()
    }

    pub fn from_app_state(hidden_calendar_ids: &HashSet<String>, last_view: CalendarView) -> Self {
        let mut hidden_calendar_ids: Vec<String> = hidden_calendar_ids.iter().cloned().collect();
        hidden_calendar_ids.sort();
        Self {
            hidden_calendar_ids,
            last_view: Some(last_view),
        }
    }
}
