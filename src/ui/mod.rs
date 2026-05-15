use chrono::{Datelike, Days, Local, TimeZone, Timelike, Weekday};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Row, Table},
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
            Constraint::Length(7),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(frame.area());

    draw_header(frame, app, vertical[0]);
    draw_current_view(frame, app, vertical[1]);
    draw_calendars(frame, app, vertical[2]);
    draw_status(frame, app, vertical[3]);
    draw_help(frame, app, vertical[4]);
}

fn draw_header(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let title = format!(
        "tuical | {} view | {}",
        app.view.title(),
        app.selected_date.format("%A, %Y-%m-%d")
    );
    let header = Paragraph::new(vec![
        Line::from(Span::styled(
            title,
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(format!(
            "{} calendar(s), {} visible event(s)",
            app.calendars.len(),
            visible_event_count(app)
        )),
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
    let now = Local::now();
    let is_today = app.selected_date == now.date_naive();
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

        let row = Row::new(vec![format!("{hour:02}:00"), summary]);
        if is_today && hour == now.hour() {
            row.style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            row
        }
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

    let block = Block::default()
        .title(format!(" Month: {} ", app.selected_date.format("%B %Y")))
        .borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Ratio(1, 6),
            Constraint::Ratio(1, 6),
            Constraint::Ratio(1, 6),
            Constraint::Ratio(1, 6),
            Constraint::Ratio(1, 6),
            Constraint::Ratio(1, 6),
        ])
        .split(inner);

    draw_month_weekday_header(frame, rows[0]);

    for week in 0..6 {
        let cells = split_week_row(rows[week + 1]);
        for day in 0..7 {
            let offset = (week * 7 + day) as u64;
            let date = month_start
                .checked_add_days(Days::new(offset))
                .unwrap_or(month_start);
            draw_month_cell(frame, app, cells[day], date, app.selected_date.month());
        }
    }
}

fn draw_month_weekday_header(frame: &mut Frame<'_>, area: Rect) {
    let cells = split_week_row(area);
    for (index, label) in ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
        .iter()
        .enumerate()
    {
        let header = Paragraph::new(*label)
            .style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
            .alignment(Alignment::Center);
        frame.render_widget(header, cells[index]);
    }
}

fn draw_month_cell(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    date: chrono::NaiveDate,
    visible_month: u32,
) {
    let events = events_for_date(app, date);
    let event_line_limit = area.height.saturating_sub(3) as usize;
    let event_width = area.width.saturating_sub(3) as usize;
    let style = date_style(date, visible_month);

    let mut lines = vec![Line::from(Span::styled(
        format!("{:>2}", date.day()),
        style,
    ))];

    for event in events.iter().take(event_line_limit) {
        lines.push(Line::from(Span::styled(
            truncate(&event.title, event_width),
            month_event_style(date, visible_month),
        )));
    }

    let hidden_count = events.len().saturating_sub(event_line_limit);
    if hidden_count > 0 && event_line_limit > 0 {
        lines.push(Line::from(Span::styled(
            format!("+{hidden_count} more"),
            Style::default().fg(Color::DarkGray),
        )));
    }

    let cell = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(month_cell_border_style(date, visible_month)),
    );
    frame.render_widget(cell, area);
}

fn split_week_row(area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(14),
            Constraint::Percentage(14),
            Constraint::Percentage(14),
            Constraint::Percentage(14),
            Constraint::Percentage(14),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
        ])
        .split(area)
}

fn draw_calendars(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let items: Vec<ListItem<'_>> = if app.calendars.is_empty() {
        vec![ListItem::new("No calendars configured yet.")]
    } else {
        app.calendars
            .iter()
            .enumerate()
            .map(|(index, calendar)| {
                let access = if calendar.read_only {
                    "read-only"
                } else {
                    "writable"
                };
                let marker = if app.is_calendar_visible(&calendar.id) {
                    "[x]"
                } else {
                    "[ ]"
                };
                let style = if index == app.selected_calendar_index {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else if app.is_calendar_visible(&calendar.id) {
                    Style::default()
                } else {
                    Style::default().fg(Color::DarkGray)
                };

                ListItem::new(Line::from(format!(
                    "{marker} {} [{}] {} ({access})",
                    calendar.name, calendar.provider_id, calendar.color
                )))
                .style(style)
            })
            .collect()
    };

    let title = if app.calendars.is_empty() {
        " Calendars ".to_string()
    } else {
        let selected = app.selected_calendar_index.min(app.calendars.len() - 1);
        format!(" Calendars {}/{} ", selected + 1, app.calendars.len())
    };

    let list = List::new(items)
        .block(Block::default().title(title).borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");
    let mut state = ListState::default().with_selected(if app.calendars.is_empty() {
        None
    } else {
        Some(app.selected_calendar_index.min(app.calendars.len() - 1))
    });

    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_status(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let status = Paragraph::new(app.status.as_str())
        .style(Style::default().fg(Color::Green))
        .block(Block::default().title(" Status ").borders(Borders::ALL));

    frame.render_widget(status, area);
}

fn draw_help(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let calendar_help = app
        .selected_calendar()
        .map(|calendar| format!("calendar: {} | j/k select | space show/hide", calendar.name))
        .unwrap_or_else(|| "calendar: none configured".to_string());
    let commands = format!(
        "views: d day, w week, m month | nav: h/l prev/next, t today | sync: r refresh, L login | {calendar_help} | q quit"
    );
    let help = Paragraph::new(commands)
        .style(Style::default().fg(Color::Yellow))
        .block(Block::default().title(" Commands ").borders(Borders::ALL));

    frame.render_widget(help, area);
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
        .filter(|event| app.is_calendar_visible(&event.calendar_id))
        .filter(|event| event.starts_at < day_end && event.ends_at > day_start)
        .collect();
    events.sort_by_key(|event| event.starts_at);
    events
}

fn visible_event_count(app: &App) -> usize {
    app.events
        .iter()
        .filter(|event| app.is_calendar_visible(&event.calendar_id))
        .count()
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

fn month_event_style(date: chrono::NaiveDate, visible_month: u32) -> Style {
    if date.month() == visible_month {
        Style::default()
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn month_cell_border_style(date: chrono::NaiveDate, visible_month: u32) -> Style {
    if date == Local::now().date_naive() {
        Style::default().fg(Color::Yellow)
    } else if date.month() == visible_month {
        Style::default().fg(Color::Blue)
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
