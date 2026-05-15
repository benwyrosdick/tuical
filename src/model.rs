use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CalendarView {
    Day,
    Week,
    Month,
}

impl CalendarView {
    pub fn title(self) -> &'static str {
        match self {
            Self::Day => "Day",
            Self::Week => "Week",
            Self::Month => "Month",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CalendarSource {
    Google,
    IcalUrl,
    CalDav,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(dead_code)]
pub struct ProviderCapabilities {
    pub read: bool,
    pub create: bool,
    pub update: bool,
    pub delete: bool,
    pub move_between_calendars: bool,
}

impl ProviderCapabilities {
    pub const fn google() -> Self {
        Self {
            read: true,
            create: true,
            update: true,
            delete: true,
            move_between_calendars: true,
        }
    }

    pub const fn read_only() -> Self {
        Self {
            read: true,
            create: false,
            update: false,
            delete: false,
            move_between_calendars: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Calendar {
    pub id: String,
    pub remote_id: String,
    pub provider_id: String,
    pub source: CalendarSource,
    pub name: String,
    pub color: String,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct Event {
    pub id: String,
    pub provider_id: String,
    pub calendar_id: String,
    pub title: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub all_day: bool,
    pub color: String,
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct TimeRange {
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct NewEvent {
    pub title: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub all_day: bool,
}
