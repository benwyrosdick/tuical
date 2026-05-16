use std::{collections::HashSet, time::Duration};

use anyhow::Result;
use chrono::{Datelike, Days, Local, Months, TimeZone, Utc};
use crossterm::event::{self, Event as TerminalEvent, KeyCode, KeyEventKind};
use ratatui::DefaultTerminal;

use crate::{
    config::TuicalConfig,
    model::{Calendar, CalendarView, Event, TimeRange},
    provider::{
        CalendarProvider,
        google::{GoogleProvider, StoredGoogleAccount, TokenStore},
        ical::IcalProvider,
    },
    settings::AppSettings,
    ui,
};

const TOKEN_FILE: &str = "tokens.toml";
const SETTINGS_FILE: &str = "settings.toml";

pub struct App {
    pub config: TuicalConfig,
    pub view: CalendarView,
    pub selected_date: chrono::NaiveDate,
    pub calendars: Vec<Calendar>,
    pub events: Vec<Event>,
    loaded_event_ranges: Vec<TimeRange>,
    pub hidden_calendar_ids: HashSet<String>,
    pub selected_calendar_index: usize,
    pub selected_event_index: usize,
    pub show_calendar_modal: bool,
    pub show_event_modal: bool,
    pub loading_message: Option<String>,
    pending_event_refresh: bool,
    pub status: String,
    should_quit: bool,
}

impl App {
    pub fn new(config: TuicalConfig, settings: AppSettings) -> Self {
        let status = if config.google_is_configured() {
            format!(
                "Google configured. Times use {} timezone. Press L to log in with browser OAuth.",
                config.timezone_label()
            )
        } else {
            format!(
                "Times use {} timezone. Add config.toml from config.example.toml to enable Google.",
                config.timezone_label()
            )
        };

        Self {
            config,
            view: settings.last_view.unwrap_or(CalendarView::Week),
            selected_date: Local::now().date_naive(),
            calendars: Vec::new(),
            events: Vec::new(),
            loaded_event_ranges: Vec::new(),
            hidden_calendar_ids: settings.hidden_calendar_set(),
            selected_calendar_index: 0,
            selected_event_index: 0,
            show_calendar_modal: false,
            show_event_modal: false,
            loading_message: None,
            pending_event_refresh: false,
            status,
            should_quit: false,
        }
    }

    pub async fn run(mut self) -> Result<()> {
        self.load_configured_calendars().await?;

        let mut terminal = ratatui::init();
        let result = self.run_loop(&mut terminal).await;
        ratatui::restore();
        result
    }

    async fn run_loop(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.should_quit {
            terminal.draw(|frame| ui::draw(frame, self))?;

            if self.pending_event_refresh {
                self.pending_event_refresh = false;
                self.refresh_events().await?;
                self.loading_message = None;
                continue;
            }

            if event::poll(Duration::from_millis(100))? {
                self.handle_event(event::read()?).await?;
            }
        }

        Ok(())
    }

    async fn load_configured_calendars(&mut self) -> Result<()> {
        self.calendars.clear();
        self.events.clear();
        self.loaded_event_ranges.clear();

        if let Err(error) = self.sync_google().await {
            if self.config.google_is_configured() {
                self.status = format!("Google sync failed: {error}");
            }
        }

        for (index, calendar_config) in self.config.ical.iter().cloned().enumerate() {
            let provider = IcalProvider::new(index, calendar_config);
            self.calendars.extend(provider.list_calendars().await?);
        }

        if self.status.starts_with("Google configured") && self.calendars.is_empty() {
            self.status = "Google configured. Press L to log in with browser OAuth.".to_string();
        }

        self.clamp_calendar_selection();

        Ok(())
    }

    async fn handle_event(&mut self, event: TerminalEvent) -> Result<()> {
        let TerminalEvent::Key(key) = event else {
            return Ok(());
        };

        if key.kind != KeyEventKind::Press {
            return Ok(());
        }

        if self.show_calendar_modal {
            return self.handle_calendar_modal_key(key.code);
        }

        if self.show_event_modal {
            return self.handle_event_modal_key(key.code);
        }

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('C') | KeyCode::Char('c') => self.show_calendar_modal = true,
            KeyCode::Char('d') => {
                self.change_view_and_queue_refresh(CalendarView::Day)?;
            }
            KeyCode::Char('w') => {
                self.change_view_and_queue_refresh(CalendarView::Week)?;
            }
            KeyCode::Char('m') => {
                self.change_view_and_queue_refresh(CalendarView::Month)?;
            }
            KeyCode::Char('t') => {
                self.selected_date = Local::now().date_naive();
                self.refresh_events().await?;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if self.view == CalendarView::Month {
                    self.move_month_selection(7);
                } else {
                    self.select_next_event();
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.view == CalendarView::Month {
                    self.move_month_selection(-7);
                } else {
                    self.select_previous_event();
                }
            }
            KeyCode::Enter => {
                if self.view == CalendarView::Month {
                    self.change_view_and_queue_refresh(CalendarView::Day)?;
                } else {
                    self.open_selected_event();
                }
            }
            KeyCode::Char('h') | KeyCode::Left => {
                if self.view == CalendarView::Month {
                    self.move_month_selection(-1);
                } else if self.view == CalendarView::Week {
                    self.move_week_selection(-1);
                    self.refresh_events().await?;
                } else {
                    self.move_backward();
                    self.refresh_events().await?;
                }
            }
            KeyCode::Char('l') | KeyCode::Right => {
                if self.view == CalendarView::Month {
                    self.move_month_selection(1);
                } else if self.view == CalendarView::Week {
                    self.move_week_selection(1);
                    self.refresh_events().await?;
                } else {
                    self.move_forward();
                    self.refresh_events().await?;
                }
            }
            KeyCode::Char('L') => self.login_google().await?,
            KeyCode::Char('r') => {
                self.loaded_event_ranges.clear();
                self.refresh_events().await?;
            }
            _ => {}
        }

        Ok(())
    }

    fn handle_event_modal_key(&mut self, key_code: KeyCode) -> Result<()> {
        match key_code {
            KeyCode::Esc | KeyCode::Enter => self.show_event_modal = false,
            KeyCode::Char('q') => self.should_quit = true,
            _ => {}
        }

        Ok(())
    }

    fn handle_calendar_modal_key(&mut self, key_code: KeyCode) -> Result<()> {
        match key_code {
            KeyCode::Esc | KeyCode::Char('C') | KeyCode::Char('c') => {
                self.show_calendar_modal = false;
            }
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('j') | KeyCode::Down => self.select_next_calendar(),
            KeyCode::Char('k') | KeyCode::Up => self.select_previous_calendar(),
            KeyCode::Char(' ') => self.toggle_selected_calendar_visibility()?,
            _ => {}
        }

        Ok(())
    }

    fn move_backward(&mut self) {
        self.selected_date = match self.view {
            CalendarView::Day => self
                .selected_date
                .checked_sub_days(Days::new(1))
                .unwrap_or(self.selected_date),
            CalendarView::Week => self
                .selected_date
                .checked_sub_days(Days::new(7))
                .unwrap_or(self.selected_date),
            CalendarView::Month => self
                .selected_date
                .checked_sub_months(Months::new(1))
                .unwrap_or(self.selected_date),
        };
    }

    fn move_forward(&mut self) {
        self.selected_date = match self.view {
            CalendarView::Day => self
                .selected_date
                .checked_add_days(Days::new(1))
                .unwrap_or(self.selected_date),
            CalendarView::Week => self
                .selected_date
                .checked_add_days(Days::new(7))
                .unwrap_or(self.selected_date),
            CalendarView::Month => self
                .selected_date
                .checked_add_months(Months::new(1))
                .unwrap_or(self.selected_date),
        };
    }

    fn move_month_selection(&mut self, days: i64) {
        let previous_month = (self.selected_date.year(), self.selected_date.month());
        self.selected_date = if days.is_negative() {
            self.selected_date
                .checked_sub_days(Days::new(days.unsigned_abs()))
        } else {
            self.selected_date.checked_add_days(Days::new(days as u64))
        }
        .unwrap_or(self.selected_date);

        let current_month = (self.selected_date.year(), self.selected_date.month());
        if current_month != previous_month {
            self.loading_message = Some("Loading month ...".to_string());
            self.pending_event_refresh = true;
        }
    }

    fn move_week_selection(&mut self, days: i64) {
        self.selected_date = if days.is_negative() {
            self.selected_date
                .checked_sub_days(Days::new(days.unsigned_abs()))
        } else {
            self.selected_date.checked_add_days(Days::new(days as u64))
        }
        .unwrap_or(self.selected_date);
        self.clamp_event_selection();
    }

    fn change_view(&mut self, view: CalendarView) -> Result<()> {
        self.view = view;
        self.save_settings()
    }

    fn change_view_and_queue_refresh(&mut self, view: CalendarView) -> Result<()> {
        self.change_view(view)?;
        self.loading_message = Some(format!("Loading {} ...", view.title().to_lowercase()));
        self.pending_event_refresh = true;
        Ok(())
    }

    pub fn is_calendar_visible(&self, calendar_id: &str) -> bool {
        !self.hidden_calendar_ids.contains(calendar_id)
    }

    pub fn selected_calendar(&self) -> Option<&Calendar> {
        self.calendars.get(self.selected_calendar_index)
    }

    pub fn selectable_events(&self) -> Vec<&Event> {
        if !matches!(self.view, CalendarView::Day | CalendarView::Week) {
            return Vec::new();
        }

        let range = TimeRange {
            starts_at: local_midnight_utc(self.selected_date),
            ends_at: local_midnight_utc(
                self.selected_date
                    .checked_add_days(Days::new(1))
                    .unwrap_or(self.selected_date),
            ),
        };
        let mut events: Vec<&Event> = self
            .events
            .iter()
            .filter(|event| self.is_calendar_visible(&event.calendar_id))
            .filter(|event| event.starts_at < range.ends_at && event.ends_at > range.starts_at)
            .collect();
        events.sort_by_key(|event| (event.starts_at, event.ends_at, event.title.clone()));
        events
    }

    pub fn selected_event(&self) -> Option<&Event> {
        self.selectable_events()
            .get(self.selected_event_index)
            .copied()
    }

    pub fn is_event_selected(&self, event: &Event) -> bool {
        self.selected_event().is_some_and(|selected| {
            selected.id == event.id
                && selected.calendar_id == event.calendar_id
                && selected.provider_id == event.provider_id
        })
    }

    pub fn calendar_name(&self, calendar_id: &str) -> Option<&str> {
        self.calendars
            .iter()
            .find(|calendar| calendar.id == calendar_id)
            .map(|calendar| calendar.name.as_str())
    }

    pub fn visible_event_count(&self) -> usize {
        let range = self.visible_time_range();
        self.events
            .iter()
            .filter(|event| self.is_calendar_visible(&event.calendar_id))
            .filter(|event| event.starts_at < range.ends_at && event.ends_at > range.starts_at)
            .count()
    }

    fn select_next_calendar(&mut self) {
        if self.calendars.is_empty() {
            return;
        }

        self.selected_calendar_index = (self.selected_calendar_index + 1) % self.calendars.len();
    }

    fn select_previous_calendar(&mut self) {
        if self.calendars.is_empty() {
            return;
        }

        self.selected_calendar_index = if self.selected_calendar_index == 0 {
            self.calendars.len() - 1
        } else {
            self.selected_calendar_index - 1
        };
    }

    fn select_next_event(&mut self) {
        let event_count = self.selectable_events().len();
        if event_count == 0 {
            self.status = "No selectable event in this view.".to_string();
            return;
        }

        self.selected_event_index = (self.selected_event_index + 1) % event_count;
    }

    fn select_previous_event(&mut self) {
        let event_count = self.selectable_events().len();
        if event_count == 0 {
            self.status = "No selectable event in this view.".to_string();
            return;
        }

        self.selected_event_index = if self.selected_event_index == 0 {
            event_count - 1
        } else {
            self.selected_event_index - 1
        };
    }

    fn open_selected_event(&mut self) {
        if self.selected_event().is_some() {
            self.show_event_modal = true;
        } else {
            self.status = "No event selected.".to_string();
        }
    }

    fn toggle_selected_calendar_visibility(&mut self) -> Result<()> {
        let Some(calendar) = self.selected_calendar() else {
            self.status = "No calendar selected.".to_string();
            return Ok(());
        };

        let calendar_id = calendar.id.clone();
        let calendar_name = calendar.name.clone();

        if self.hidden_calendar_ids.remove(&calendar_id) {
            self.status = format!("Showing calendar: {calendar_name}");
        } else {
            self.hidden_calendar_ids.insert(calendar_id);
            self.status = format!("Hiding calendar: {calendar_name}");
        }

        self.save_settings()?;
        self.clamp_event_selection();
        Ok(())
    }

    fn save_settings(&self) -> Result<()> {
        AppSettings::from_app_state(&self.hidden_calendar_ids, self.view).save(SETTINGS_FILE)
    }

    fn clamp_calendar_selection(&mut self) {
        if self.calendars.is_empty() {
            self.selected_calendar_index = 0;
        } else if self.selected_calendar_index >= self.calendars.len() {
            self.selected_calendar_index = self.calendars.len() - 1;
        }
    }

    fn clamp_event_selection(&mut self) {
        let event_count = self.selectable_events().len();
        if event_count == 0 {
            self.selected_event_index = 0;
            self.show_event_modal = false;
        } else if self.selected_event_index >= event_count {
            self.selected_event_index = event_count - 1;
        }
    }

    async fn login_google(&mut self) -> Result<()> {
        let Some(google_config) = self.config.google.clone() else {
            self.status = "Google is not configured in config.toml.".to_string();
            return Ok(());
        };

        let provider = GoogleProvider::new(google_config);
        self.status = "Opening Google login in your browser...".to_string();
        let tokens = provider.login_with_browser().await?;
        let account_id =
            StoredGoogleAccount::new("Google account".to_string(), None, tokens.clone()).id;
        let token_provider = GoogleProvider::with_tokens(
            self.config.google.clone().expect("checked above"),
            tokens.clone(),
            format!("google:{account_id}"),
        );
        let summary = token_provider.account_summary().await?;
        let account = StoredGoogleAccount {
            id: account_id,
            label: summary.label,
            primary_calendar_id: summary.primary_calendar_id,
            tokens,
        };
        let mut store = TokenStore::load(TOKEN_FILE)?;
        store.upsert_google_account(account);
        store.save(TOKEN_FILE)?;
        self.status = "Google login complete. Fetching calendars and events...".to_string();
        self.load_configured_calendars().await?;
        Ok(())
    }

    async fn refresh_events(&mut self) -> Result<()> {
        let range = self.eager_time_range();

        if self.is_range_cached(range) {
            self.status = format!(
                "Using cached {} event(s) for {}.",
                self.visible_event_count(),
                self.view.title().to_lowercase()
            );
            self.clamp_event_selection();
            return Ok(());
        }

        if let Err(error) = self.sync_google_events(range).await {
            if self.config.google_is_configured() {
                self.status = format!("Google event refresh failed: {error}");
            }
        } else {
            self.loaded_event_ranges.push(range);
        }

        self.clamp_event_selection();

        Ok(())
    }

    async fn sync_google(&mut self) -> Result<()> {
        let Some(google_config) = self.config.google.clone() else {
            return Ok(());
        };

        let mut store = TokenStore::load(TOKEN_FILE)?;
        if store.google.is_empty() {
            return Ok(());
        };
        let token_provider = GoogleProvider::new(google_config.clone());
        let mut google_calendar_count = 0;

        for account in &mut store.google {
            if account.tokens.is_expired() {
                account.tokens = token_provider.refresh_tokens(&account.tokens).await?;
            }

            let provider = GoogleProvider::with_tokens(
                google_config.clone(),
                account.tokens.clone(),
                account.provider_id(),
            );

            if account.primary_calendar_id.is_none() {
                let summary = provider.account_summary().await?;
                account.label = summary.label;
                account.primary_calendar_id = summary.primary_calendar_id;
            }

            let mut google_calendars = provider.list_calendars().await?;
            google_calendar_count += google_calendars.len();

            for calendar in &mut google_calendars {
                calendar.name = format!("{} / {}", account.label, calendar.name);
            }

            self.calendars.extend(google_calendars);
        }

        store.save(TOKEN_FILE)?;
        let range = self.eager_time_range();
        self.sync_google_events(range).await?;
        self.loaded_event_ranges.push(range);
        self.status = format!(
            "Loaded {} Google account(s), {} calendar(s), and {} event(s). Press r to refresh.",
            store.google.len(),
            google_calendar_count,
            self.visible_event_count()
        );

        Ok(())
    }

    async fn sync_google_events(&mut self, range: TimeRange) -> Result<usize> {
        let Some(google_config) = self.config.google.clone() else {
            return Ok(0);
        };
        let mut store = TokenStore::load(TOKEN_FILE)?;
        if store.google.is_empty() {
            return Ok(0);
        };

        let token_provider = GoogleProvider::new(google_config.clone());
        let mut fetched_event_count = 0;

        for account in &mut store.google {
            if account.tokens.is_expired() {
                account.tokens = token_provider.refresh_tokens(&account.tokens).await?;
            }

            let provider = GoogleProvider::with_tokens(
                google_config.clone(),
                account.tokens.clone(),
                account.provider_id(),
            );
            let google_calendars: Vec<Calendar> = self
                .calendars
                .iter()
                .filter(|calendar| calendar.provider_id == provider.id())
                .cloned()
                .collect();

            for calendar in google_calendars {
                let mut events = provider.list_events(&calendar.remote_id, range).await?;
                for event in &mut events {
                    event.calendar_id = calendar.id.clone();
                    event.color = calendar.color.clone();
                }
                fetched_event_count += events.len();
                self.merge_events(events);
            }
        }

        store.save(TOKEN_FILE)?;

        self.status = format!(
            "Fetched {} event(s), showing {} for {}.",
            fetched_event_count,
            self.visible_event_count(),
            self.view.title().to_lowercase()
        );
        self.clamp_event_selection();
        Ok(fetched_event_count)
    }

    fn merge_events(&mut self, events: Vec<Event>) {
        for event in events {
            self.events.retain(|existing| {
                !(existing.id == event.id
                    && existing.calendar_id == event.calendar_id
                    && existing.provider_id == event.provider_id)
            });
            self.events.push(event);
        }
    }

    fn is_range_cached(&self, range: TimeRange) -> bool {
        self.loaded_event_ranges.iter().any(|loaded_range| {
            loaded_range.starts_at <= range.starts_at && loaded_range.ends_at >= range.ends_at
        })
    }

    fn visible_time_range(&self) -> TimeRange {
        let (starts_on, ends_on) = self.visible_date_bounds();

        TimeRange {
            starts_at: local_midnight_utc(starts_on),
            ends_at: local_midnight_utc(ends_on),
        }
    }

    fn eager_time_range(&self) -> TimeRange {
        let (starts_on, ends_on) = self.visible_date_bounds();
        let eager_starts_on = starts_on
            .checked_sub_months(Months::new(1))
            .unwrap_or(starts_on);
        let eager_ends_on = ends_on
            .checked_add_months(Months::new(1))
            .unwrap_or(ends_on);

        TimeRange {
            starts_at: local_midnight_utc(eager_starts_on),
            ends_at: local_midnight_utc(eager_ends_on),
        }
    }

    fn visible_date_bounds(&self) -> (chrono::NaiveDate, chrono::NaiveDate) {
        let starts_on = match self.view {
            CalendarView::Day => self.selected_date,
            CalendarView::Week => start_of_week(self.selected_date),
            CalendarView::Month => self.selected_date.with_day(1).unwrap_or(self.selected_date),
        };
        let ends_on = match self.view {
            CalendarView::Day => starts_on.checked_add_days(Days::new(1)),
            CalendarView::Week => starts_on.checked_add_days(Days::new(7)),
            CalendarView::Month => starts_on.checked_add_months(Months::new(1)),
        }
        .unwrap_or(starts_on);

        (starts_on, ends_on)
    }
}

fn start_of_week(date: chrono::NaiveDate) -> chrono::NaiveDate {
    date.checked_sub_days(Days::new(date.weekday().num_days_from_sunday().into()))
        .unwrap_or(date)
}

fn local_midnight_utc(date: chrono::NaiveDate) -> chrono::DateTime<Utc> {
    Local
        .with_ymd_and_hms(date.year(), date.month(), date.day(), 0, 0, 0)
        .single()
        .map(|local| local.with_timezone(&Utc))
        .unwrap_or_else(|| date.and_hms_opt(0, 0, 0).unwrap().and_utc())
}
