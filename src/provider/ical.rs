use anyhow::{Result, bail};
use async_trait::async_trait;

use crate::{
    config::IcalConfig,
    model::{Calendar, CalendarSource, Event, NewEvent, ProviderCapabilities, TimeRange},
    provider::CalendarProvider,
};

#[derive(Debug, Clone)]
pub struct IcalProvider {
    id: String,
    config: IcalConfig,
}

impl IcalProvider {
    pub fn new(index: usize, config: IcalConfig) -> Self {
        Self {
            id: format!("ical-{index}"),
            config,
        }
    }
}

#[async_trait]
impl CalendarProvider for IcalProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::read_only()
    }

    async fn list_calendars(&self) -> Result<Vec<Calendar>> {
        Ok(vec![Calendar {
            id: self.id.clone(),
            provider_id: self.id.clone(),
            source: CalendarSource::IcalUrl,
            name: self.config.name.clone(),
            color: self.config.color.clone(),
            read_only: true,
        }])
    }

    async fn list_events(&self, _calendar_id: &str, _range: TimeRange) -> Result<Vec<Event>> {
        bail!("iCal URL event fetch is not implemented yet")
    }

    async fn create_event(&self, _calendar_id: &str, _event: NewEvent) -> Result<Event> {
        bail!("iCal URL calendars are read-only")
    }

    async fn update_event(&self, _event: Event) -> Result<Event> {
        bail!("iCal URL calendars are read-only")
    }

    async fn move_event(
        &self,
        _event_id: &str,
        _from_calendar_id: &str,
        _to_calendar_id: &str,
    ) -> Result<Event> {
        bail!("iCal URL calendars are read-only")
    }
}
