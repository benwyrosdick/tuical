pub mod google;
pub mod ical;

use anyhow::Result;
use async_trait::async_trait;

use crate::model::{Calendar, Event, NewEvent, ProviderCapabilities, TimeRange};

#[async_trait]
#[allow(dead_code)]
pub trait CalendarProvider {
    fn id(&self) -> &str;
    fn capabilities(&self) -> ProviderCapabilities;

    async fn list_calendars(&self) -> Result<Vec<Calendar>>;
    async fn list_events(&self, calendar_id: &str, range: TimeRange) -> Result<Vec<Event>>;

    async fn create_event(&self, calendar_id: &str, event: NewEvent) -> Result<Event>;
    async fn update_event(&self, event: Event) -> Result<Event>;
    async fn move_event(
        &self,
        event_id: &str,
        from_calendar_id: &str,
        to_calendar_id: &str,
    ) -> Result<Event>;
}
