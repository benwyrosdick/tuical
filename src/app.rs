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
    pub hidden_calendar_ids: HashSet<String>,
    pub selected_calendar_index: usize,
    pub show_calendar_modal: bool,
    pub loading_message: Option<String>,
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
            hidden_calendar_ids: settings.hidden_calendar_set(),
            selected_calendar_index: 0,
            show_calendar_modal: false,
            loading_message: None,
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

            if event::poll(Duration::from_millis(100))? {
                self.handle_event(event::read()?, terminal).await?;
            }
        }

        Ok(())
    }

    async fn load_configured_calendars(&mut self) -> Result<()> {
        self.calendars.clear();
        self.events.clear();

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

    async fn handle_event(
        &mut self,
        event: TerminalEvent,
        terminal: &mut DefaultTerminal,
    ) -> Result<()> {
        let TerminalEvent::Key(key) = event else {
            return Ok(());
        };

        if key.kind != KeyEventKind::Press {
            return Ok(());
        }

        if self.show_calendar_modal {
            return self.handle_calendar_modal_key(key.code);
        }

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('C') | KeyCode::Char('c') => self.show_calendar_modal = true,
            KeyCode::Char('d') => {
                self.change_view_and_refresh(CalendarView::Day, terminal)
                    .await?;
            }
            KeyCode::Char('w') => {
                self.change_view_and_refresh(CalendarView::Week, terminal)
                    .await?;
            }
            KeyCode::Char('m') => {
                self.change_view_and_refresh(CalendarView::Month, terminal)
                    .await?;
            }
            KeyCode::Char('t') => {
                self.selected_date = Local::now().date_naive();
                self.refresh_events().await?;
            }
            KeyCode::Char('h') | KeyCode::Left => {
                self.move_backward();
                self.refresh_events().await?;
            }
            KeyCode::Char('l') | KeyCode::Right => {
                self.move_forward();
                self.refresh_events().await?;
            }
            KeyCode::Char('L') => self.login_google().await?,
            KeyCode::Char('r') => self.refresh_events().await?,
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

    fn change_view(&mut self, view: CalendarView) -> Result<()> {
        self.view = view;
        self.save_settings()
    }

    async fn change_view_and_refresh(
        &mut self,
        view: CalendarView,
        terminal: &mut DefaultTerminal,
    ) -> Result<()> {
        self.change_view(view)?;
        self.loading_message = Some(format!("Loading {} ...", view.title().to_lowercase()));
        terminal.draw(|frame| ui::draw(frame, self))?;
        let result = self.refresh_events().await;
        self.loading_message = None;
        result
    }

    pub fn is_calendar_visible(&self, calendar_id: &str) -> bool {
        !self.hidden_calendar_ids.contains(calendar_id)
    }

    pub fn selected_calendar(&self) -> Option<&Calendar> {
        self.calendars.get(self.selected_calendar_index)
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
        self.events.clear();

        if let Err(error) = self.sync_google_events().await {
            if self.config.google_is_configured() {
                self.status = format!("Google event refresh failed: {error}");
            }
        }

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
        self.sync_google_events().await?;
        self.status = format!(
            "Loaded {} Google account(s), {} calendar(s), and {} event(s). Press r to refresh.",
            store.google.len(),
            google_calendar_count,
            self.events.len()
        );

        Ok(())
    }

    async fn sync_google_events(&mut self) -> Result<()> {
        let Some(google_config) = self.config.google.clone() else {
            return Ok(());
        };
        let mut store = TokenStore::load(TOKEN_FILE)?;
        if store.google.is_empty() {
            return Ok(());
        };

        let token_provider = GoogleProvider::new(google_config.clone());
        let range = self.visible_time_range();

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
                self.events.extend(events);
            }
        }

        store.save(TOKEN_FILE)?;

        self.status = format!(
            "Loaded {} event(s) for {}.",
            self.events.len(),
            self.view.title().to_lowercase()
        );
        Ok(())
    }

    fn visible_time_range(&self) -> TimeRange {
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

        TimeRange {
            starts_at: local_midnight_utc(starts_on),
            ends_at: local_midnight_utc(ends_on),
        }
    }
}

fn start_of_week(date: chrono::NaiveDate) -> chrono::NaiveDate {
    date.checked_sub_days(Days::new(date.weekday().num_days_from_monday().into()))
        .unwrap_or(date)
}

fn local_midnight_utc(date: chrono::NaiveDate) -> chrono::DateTime<Utc> {
    Local
        .with_ymd_and_hms(date.year(), date.month(), date.day(), 0, 0, 0)
        .single()
        .map(|local| local.with_timezone(&Utc))
        .unwrap_or_else(|| date.and_hms_opt(0, 0, 0).unwrap().and_utc())
}
