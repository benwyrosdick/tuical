use chrono::{Datelike, Days, Local, TimeZone, Timelike, Weekday};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table},
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
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(frame.area());

    draw_header(frame, app, vertical[0]);
    draw_current_view(frame, app, vertical[1]);
    draw_status(frame, app, vertical[2]);
    draw_help(frame, app, vertical[3]);

    if app.show_calendar_modal {
        draw_calendar_modal(frame, app);
    }

    if app.show_event_modal {
        draw_event_modal(frame, app);
    }

    if app.loading_message.is_some() {
        draw_loading_modal(frame, app);
    }
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
            app.visible_event_count()
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
            Line::from("")
        } else {
            event_summary_line(app, events)
        };

        let row = Row::new(vec![
            Cell::from(format!("{hour:02}:00")),
            Cell::from(summary),
        ]);
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
                    .map(|event| event_detail_line(app, event, "  ")),
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
    let month_start = start_of_month_grid(first_day);

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
    for (index, label) in ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
        .iter()
        .enumerate()
    {
        let header = Paragraph::new(*label)
            .style(month_header_style(index).add_modifier(Modifier::BOLD))
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
    let mut events = events_for_date(app, date);
    events.sort_by_key(|event| (!event.all_day, event.starts_at));
    let event_line_limit = area.height.saturating_sub(3) as usize;
    let event_width = area.width.saturating_sub(3) as usize;
    let style = month_date_style(date, visible_month, app.selected_date);

    let mut lines = vec![Line::from(Span::styled(
        format!("{:>2}", date.day()),
        style,
    ))];

    for event in events.iter().take(event_line_limit) {
        lines.push(month_event_line(event, event_width, date, visible_month));
    }

    let hidden_count = events.len().saturating_sub(event_line_limit);
    if hidden_count > 0 && event_line_limit > 0 {
        lines.push(Line::from(Span::styled(
            format!("+{hidden_count} more"),
            Style::default().fg(Color::DarkGray),
        )));
    }

    let fill_style = month_cell_fill_style(date, visible_month);
    let cell = Paragraph::new(lines).style(fill_style).block(
        Block::default()
            .borders(Borders::ALL)
            .style(fill_style)
            .border_style(month_cell_border_style(
                date,
                visible_month,
                app.selected_date,
            )),
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
                let calendar_style = calendar_color_style(&calendar.color);
                let style = if index == app.selected_calendar_index {
                    selected_calendar_style(&calendar.color)
                } else if app.is_calendar_visible(&calendar.id) {
                    calendar_style
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

fn draw_calendar_modal(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(82, 72, frame.area());
    frame.render_widget(Clear, area);
    draw_calendars(frame, app, area);
}

fn draw_loading_modal(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(36, 18, frame.area());
    frame.render_widget(Clear, area);
    let message = app.loading_message.as_deref().unwrap_or("Loading ...");
    let modal = Paragraph::new(message)
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .alignment(Alignment::Center)
        .block(Block::default().title(" Loading ").borders(Borders::ALL));
    frame.render_widget(modal, area);
}

fn draw_event_modal(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(62, 48, frame.area());
    frame.render_widget(Clear, area);

    let Some(event) = app.selected_event() else {
        return;
    };

    let mut lines = vec![
        Line::from(Span::styled(
            event.title.clone(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(format!("When: {}", format_event_range(event))),
        Line::from(format!(
            "Calendar: {}",
            app.calendar_name(&event.calendar_id).unwrap_or("unknown")
        )),
    ];

    if let Some(location) = event
        .location
        .as_deref()
        .filter(|location| !location.is_empty())
    {
        lines.push(Line::from(format!("Location: {location}")));
    }

    if let Some(description) = event
        .description
        .as_deref()
        .filter(|description| !description.is_empty())
    {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Description",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        lines.extend(description.lines().take(8).map(Line::from));
    }

    let modal = Paragraph::new(lines).block(
        Block::default()
            .title(" Event ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow)),
    );
    frame.render_widget(modal, area);
}

fn draw_status(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let status = Paragraph::new(app.status.as_str())
        .style(Style::default().fg(Color::Green))
        .block(Block::default().title(" Status ").borders(Borders::ALL));

    frame.render_widget(status, area);
}

fn draw_help(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let commands = if app.show_calendar_modal {
        app.selected_calendar()
            .map(|calendar| {
                format!(
                    "calendar: {} | j/k select | space show/hide | C or Esc close | q quit",
                    calendar.name
                )
            })
            .unwrap_or_else(|| "calendar: none configured | C or Esc close | q quit".to_string())
    } else if app.show_event_modal {
        "event: Enter/Esc close | q quit".to_string()
    } else {
        match app.view {
            CalendarView::Month => "month: h/j/k/l move day, Enter open day | views: d day, w week | sync: r refresh, L login | C calendars | q quit".to_string(),
            _ => "views: d day, w week, m month | nav: h/l prev/next, t today | events: j/k select, Enter details | sync: r refresh, L login | C calendars | q quit"
                .to_string(),
        }
    };
    let help = Paragraph::new(commands)
        .style(Style::default().fg(Color::Yellow))
        .block(Block::default().title(" Commands ").borders(Borders::ALL));

    frame.render_widget(help, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn start_of_week(date: chrono::NaiveDate) -> chrono::NaiveDate {
    date.checked_sub_days(Days::new(date.weekday().num_days_from_monday().into()))
        .unwrap_or(date)
}

fn start_of_month_grid(date: chrono::NaiveDate) -> chrono::NaiveDate {
    date.checked_sub_days(Days::new(date.weekday().num_days_from_sunday().into()))
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
            if event.all_day {
                hour == 0
            } else {
                event.starts_at.with_timezone(&Local).hour() == hour
            }
        })
        .collect()
}

fn event_summary_line(app: &App, events: Vec<&Event>) -> Line<'static> {
    let mut spans = Vec::new();

    for (index, event) in events.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
        }

        spans.push(Span::styled(
            format_event(event),
            selectable_event_style(app, event),
        ));
    }

    Line::from(spans)
}

fn event_detail_line(app: &App, event: &Event, prefix: &'static str) -> Line<'static> {
    Line::from(Span::styled(
        format!("{prefix}{}", format_event(event)),
        selectable_event_style(app, event),
    ))
}

fn selectable_event_style(app: &App, event: &Event) -> Style {
    if app.is_event_selected(event) {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else if event.all_day {
        calendar_color_style(&event.color).add_modifier(Modifier::BOLD)
    } else {
        calendar_color_style(&event.color)
    }
}

fn calendar_color_style(color: &str) -> Style {
    Style::default().fg(parse_calendar_color(color))
}

fn parse_calendar_color(color: &str) -> Color {
    let color = color.trim();
    match color.to_ascii_lowercase().as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" | "purple" => Color::Magenta,
        "cyan" => Color::Cyan,
        "gray" | "grey" => Color::Gray,
        "darkgray" | "dark_gray" | "dark-grey" => Color::DarkGray,
        "white" => Color::White,
        _ => parse_hex_color(color).unwrap_or(Color::White),
    }
}

fn selected_calendar_style(color: &str) -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(parse_calendar_color(color))
        .add_modifier(Modifier::BOLD)
}

fn parse_hex_color(color: &str) -> Option<Color> {
    let hex = color.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }

    let red = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let green = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let blue = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::Rgb(red, green, blue))
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

fn month_date_style(
    date: chrono::NaiveDate,
    visible_month: u32,
    selected_date: chrono::NaiveDate,
) -> Style {
    if date == selected_date {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        date_style(date, visible_month)
    }
}

fn month_header_style(index: usize) -> Style {
    if index == 0 || index == 6 {
        Style::default().fg(Color::Magenta)
    } else {
        Style::default().fg(Color::Yellow)
    }
}

fn month_cell_fill_style(date: chrono::NaiveDate, visible_month: u32) -> Style {
    let background = if date.month() != visible_month {
        Color::Rgb(10, 10, 14)
    } else {
        Color::Rgb(16, 20, 28)
    };

    Style::default().bg(background)
}

fn month_event_style(event: &Event, date: chrono::NaiveDate, visible_month: u32) -> Style {
    if date.month() == visible_month {
        calendar_color_style(&event.color)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn month_event_time_style(date: chrono::NaiveDate, visible_month: u32) -> Style {
    if date.month() == visible_month {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn month_all_day_event_style(event: &Event, date: chrono::NaiveDate, visible_month: u32) -> Style {
    if date.month() == visible_month {
        calendar_color_style(&event.color).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn month_cell_border_style(
    date: chrono::NaiveDate,
    visible_month: u32,
    selected_date: chrono::NaiveDate,
) -> Style {
    if date == selected_date {
        Style::default().fg(Color::Yellow)
    } else if date == Local::now().date_naive() {
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

fn format_event_range(event: &Event) -> String {
    let starts_at = event.starts_at.with_timezone(&Local);
    let ends_at = event.ends_at.with_timezone(&Local);

    if event.all_day {
        return starts_at.format("%a %Y-%m-%d all day").to_string();
    }

    if starts_at.date_naive() == ends_at.date_naive() {
        format!(
            "{} {}-{}",
            starts_at.format("%a %Y-%m-%d"),
            starts_at.format("%H:%M"),
            ends_at.format("%H:%M")
        )
    } else {
        format!(
            "{} to {}",
            starts_at.format("%a %Y-%m-%d %H:%M"),
            ends_at.format("%a %Y-%m-%d %H:%M")
        )
    }
}

fn month_event_line(
    event: &Event,
    max_chars: usize,
    date: chrono::NaiveDate,
    visible_month: u32,
) -> Line<'static> {
    if event.all_day {
        return Line::from(Span::styled(
            truncate(&event.title, max_chars),
            month_all_day_event_style(event, date, visible_month),
        ));
    }

    let time = short_time(event);
    let title_width = max_chars.saturating_sub(time.chars().count() + 1);

    Line::from(vec![
        Span::styled(time, month_event_time_style(date, visible_month)),
        Span::raw(" "),
        Span::styled(
            truncate(&event.title, title_width),
            month_event_style(event, date, visible_month),
        ),
    ])
}

fn short_time(event: &Event) -> String {
    let starts_at = event.starts_at.with_timezone(&Local);
    let hour = starts_at.hour();
    let minute = starts_at.minute();
    let suffix = if hour < 12 { "a" } else { "p" };
    let hour = match hour % 12 {
        0 => 12,
        hour => hour,
    };

    if minute == 0 {
        format!("{hour}{suffix}")
    } else {
        format!("{hour}:{minute:02}{suffix}")
    }
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
