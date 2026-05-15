use chrono::{Datelike, Days, Local, TimeZone, Timelike, Weekday};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table},
};

use crate::{
    app::App,
    model::{CalendarView, Event},
};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(6),
            Constraint::Length(3),
        ])
        .split(frame.area());

    draw_header(frame, app, vertical[0]);
    draw_current_view(frame, app, vertical[1]);
    draw_calendars(frame, app, vertical[2]);
    draw_status(frame, app, vertical[3]);
}

fn draw_header(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let title = format!(
        "tuical | {} view | {}",
        app.view.title(),
        app.selected_date.format("%A, %Y-%m-%d")
    );
    let help = "d day | w week | m month | h/l prev/next | t today | r refresh | L login | q quit";

    let header = Paragraph::new(vec![
        Line::from(Span::styled(
            title,
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(help),
    ])
    .block(Block::default().borders(Borders::ALL));

    frame.render_widget(header, area);
}

fn draw_current_view(frame: &mut Frame<'_>, app: &App, area: Rect) {
    match app.view {
        CalendarView::Day => draw_day(frame, app, area),
        CalendarView::Week => draw_week(frame, app, area),
        CalendarView::Month => draw_month(frame, app, area),
    }
}

fn draw_day(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let rows = (0..24).map(|hour| {
        let events = events_for_hour(app, hour);
        let summary = if events.is_empty() {
            String::new()
        } else {
            events
                .into_iter()
                .map(format_event)
                .collect::<Vec<_>>()
                .join(" | ")
        };

        Row::new(vec![format!("{hour:02}:00"), summary])
    });

    let table = Table::new(rows, [Constraint::Length(8), Constraint::Min(20)])
        .header(Row::new(vec!["Time", "Schedule"]).style(Style::default().fg(Color::Yellow)))
        .block(
            Block::default()
                .title(format!(" Day: {} ", app.selected_date.format("%Y-%m-%d")))
                .borders(Borders::ALL),
        );

    frame.render_widget(table, area);
}

fn draw_week(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let week_start = start_of_week(app.selected_date);
    let days = (0..7).map(|offset| {
        let date = week_start
            .checked_add_days(Days::new(offset))
            .unwrap_or(week_start);
        let date_style = date_style(date, app.selected_date.month());
        let mut lines = vec![Line::from(vec![
            Span::styled(
                format!("{} ", weekday_label(date.weekday())),
                date_style.add_modifier(Modifier::BOLD),
            ),
            Span::styled(date.format("%Y-%m-%d").to_string(), date_style),
        ])];

        let events = events_for_date(app, date);
        if events.is_empty() {
            lines.push(Line::from("  no events"));
        } else {
            lines.extend(
                events
                    .into_iter()
                    .take(5)
                    .map(|event| Line::from(format!("  {}", format_event(event)))),
            );
        }

        ListItem::new(lines)
    });

    let list = List::new(days)
        .block(Block::default().title(" Week ").borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    frame.render_widget(list, area);
}

fn draw_month(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(first_day) = app.selected_date.with_day(1) else {
        return;
    };
    let month_start = start_of_week(first_day);

    let rows = (0..6).map(|week| {
        let cells = (0..7).map(|day| {
            let offset = week * 7 + day;
            let date = month_start
                .checked_add_days(Days::new(offset))
                .unwrap_or(month_start);
            let style = date_style(date, app.selected_date.month());
            let events = events_for_date(app, date);
            let label = if let Some(event) = events.first() {
                let more = events.len().saturating_sub(1);
                if more > 0 {
                    format!("{:>2} {} +{more}", date.day(), truncate(&event.title, 8))
                } else {
                    format!("{:>2} {}", date.day(), truncate(&event.title, 10))
                }
            } else {
                format!("{:>2}", date.day())
            };

            Cell::from(label).style(style)
        });

        Row::new(cells)
    });

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(14),
            Constraint::Percentage(14),
            Constraint::Percentage(14),
            Constraint::Percentage(14),
            Constraint::Percentage(14),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
        ],
    )
    .header(
        Row::new(vec!["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"])
            .style(Style::default().fg(Color::Yellow)),
    )
    .block(
        Block::default()
            .title(format!(" Month: {} ", app.selected_date.format("%B %Y")))
            .borders(Borders::ALL),
    );

    frame.render_widget(table, area);
}

fn draw_calendars(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let items: Vec<ListItem<'_>> = if app.calendars.is_empty() {
        vec![ListItem::new("No calendars configured yet.")]
    } else {
        app.calendars
            .iter()
            .map(|calendar| {
                let access = if calendar.read_only {
                    "read-only"
                } else {
                    "writable"
                };
                ListItem::new(format!(
                    "{} [{}] {} ({access})",
                    calendar.name, calendar.provider_id, calendar.color
                ))
            })
            .collect()
    };

    let list = List::new(items).block(Block::default().title(" Calendars ").borders(Borders::ALL));
    frame.render_widget(list, area);
}

fn draw_status(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let status = Paragraph::new(app.status.as_str())
        .style(Style::default().fg(Color::Green))
        .block(Block::default().title(" Status ").borders(Borders::ALL));

    frame.render_widget(status, area);
}

fn start_of_week(date: chrono::NaiveDate) -> chrono::NaiveDate {
    date.checked_sub_days(Days::new(date.weekday().num_days_from_monday().into()))
        .unwrap_or(date)
}

fn weekday_label(weekday: Weekday) -> &'static str {
    match weekday {
        Weekday::Mon => "Mon",
        Weekday::Tue => "Tue",
        Weekday::Wed => "Wed",
        Weekday::Thu => "Thu",
        Weekday::Fri => "Fri",
        Weekday::Sat => "Sat",
        Weekday::Sun => "Sun",
    }
}

fn events_for_hour(app: &App, hour: u32) -> Vec<&Event> {
    events_for_date(app, app.selected_date)
        .into_iter()
        .filter(|event| {
            (event.all_day && hour == 0) || event.starts_at.with_timezone(&Local).hour() == hour
        })
        .collect()
}

fn events_for_date(app: &App, date: chrono::NaiveDate) -> Vec<&Event> {
    let day_start = local_midnight_utc(date);
    let day_end = local_midnight_utc(date.checked_add_days(Days::new(1)).unwrap_or(date));

    let mut events: Vec<&Event> = app
        .events
        .iter()
        .filter(|event| event.starts_at < day_end && event.ends_at > day_start)
        .collect();
    events.sort_by_key(|event| event.starts_at);
    events
}

fn local_midnight_utc(date: chrono::NaiveDate) -> chrono::DateTime<chrono::Utc> {
    Local
        .with_ymd_and_hms(date.year(), date.month(), date.day(), 0, 0, 0)
        .single()
        .map(|local| local.with_timezone(&chrono::Utc))
        .unwrap_or_else(|| date.and_hms_opt(0, 0, 0).unwrap().and_utc())
}

fn date_style(date: chrono::NaiveDate, visible_month: u32) -> Style {
    if date == Local::now().date_naive() {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else if date.month() == visible_month {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn format_event(event: &Event) -> String {
    let time = if event.all_day {
        "all-day".to_string()
    } else {
        format!("{}", event.starts_at.with_timezone(&Local).format("%H:%M"))
    };

    format!("{time} {}", event.title)
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}
