use std::time::Duration;

use anyhow::Result;
use chrono::{Datelike, Days, Local, Months, TimeZone, Utc};
use crossterm::event::{self, Event as TerminalEvent, KeyCode, KeyEventKind};
use ratatui::DefaultTerminal;

use crate::{
    config::TuicalConfig,
    model::{Calendar, CalendarView, Event, TimeRange},
    provider::{
        CalendarProvider,
        google::{GoogleProvider, GoogleTokenSet},
        ical::IcalProvider,
    },
    ui,
};

const TOKEN_FILE: &str = "tokens.toml";

pub struct App {
    pub config: TuicalConfig,
    pub view: CalendarView,
    pub selected_date: chrono::NaiveDate,
    pub calendars: Vec<Calendar>,
    pub events: Vec<Event>,
    pub status: String,
    should_quit: bool,
}

impl App {
    pub fn new(config: TuicalConfig) -> Self {
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
            view: CalendarView::Week,
            selected_date: Local::now().date_naive(),
            calendars: Vec::new(),
            events: Vec::new(),
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
                self.handle_event(event::read()?).await?;
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

        Ok(())
    }

    async fn handle_event(&mut self, event: TerminalEvent) -> Result<()> {
        let TerminalEvent::Key(key) = event else {
            return Ok(());
        };

        if key.kind != KeyEventKind::Press {
            return Ok(());
        }

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('d') => self.view = CalendarView::Day,
            KeyCode::Char('w') => self.view = CalendarView::Week,
            KeyCode::Char('m') => self.view = CalendarView::Month,
            KeyCode::Char('t') => self.selected_date = Local::now().date_naive(),
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

    async fn login_google(&mut self) -> Result<()> {
        let Some(google_config) = self.config.google.clone() else {
            self.status = "Google is not configured in config.toml.".to_string();
            return Ok(());
        };

        let provider = GoogleProvider::new(google_config);
        self.status = "Opening Google login in your browser...".to_string();
        let tokens = provider.login_with_browser().await?;
        tokens.save(TOKEN_FILE)?;
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

        let Some(tokens) = GoogleTokenSet::load(TOKEN_FILE)? else {
            return Ok(());
        };
        let token_provider = GoogleProvider::new(google_config.clone());
        let tokens = if tokens.is_expired() {
            let refreshed = token_provider.refresh_tokens(&tokens).await?;
            refreshed.save(TOKEN_FILE)?;
            refreshed
        } else {
            tokens
        };

        let provider = GoogleProvider::with_tokens(google_config, tokens);
        let google_calendars = provider.list_calendars().await?;
        let google_calendar_count = google_calendars.len();
        self.calendars.extend(google_calendars);
        self.sync_google_events().await?;
        self.status = format!(
            "Loaded {} Google calendar(s) and {} event(s). Press r to refresh.",
            google_calendar_count,
            self.events.len()
        );

        Ok(())
    }

    async fn sync_google_events(&mut self) -> Result<()> {
        let Some(google_config) = self.config.google.clone() else {
            return Ok(());
        };
        let Some(tokens) = GoogleTokenSet::load(TOKEN_FILE)? else {
            return Ok(());
        };

        let token_provider = GoogleProvider::new(google_config.clone());
        let tokens = if tokens.is_expired() {
            let refreshed = token_provider.refresh_tokens(&tokens).await?;
            refreshed.save(TOKEN_FILE)?;
            refreshed
        } else {
            tokens
        };

        let provider = GoogleProvider::with_tokens(google_config, tokens);
        let range = self.visible_time_range();
        let google_calendars: Vec<Calendar> = self
            .calendars
            .iter()
            .filter(|calendar| calendar.provider_id == "google")
            .cloned()
            .collect();

        for calendar in google_calendars {
            let mut events = provider.list_events(&calendar.id, range).await?;
            for event in &mut events {
                event.color = calendar.color.clone();
            }
            self.events.extend(events);
        }

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
