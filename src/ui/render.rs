use crate::model::attr::{Attr, Kind, State, Value};
use crate::model::TabId;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Tabs, Wrap};
use ratatui::Frame;

use super::app::{App, Level, Mode};

const SPARK: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

fn level_color(level: Level) -> Color {
    match level {
        Level::Info => Color::Gray,
        Level::Success => Color::Green,
        Level::Warn => Color::Yellow,
        Level::Error => Color::Red,
    }
}

fn sparkline(vals: &[f64], max: f64) -> String {
    vals.iter()
        .map(|v| {
            let ratio = (v / max).clamp(0.0, 1.0);
            SPARK[(ratio * 7.999) as usize]
        })
        .collect()
}

fn bar(value: f64, max: f64, width: usize) -> String {
    let filled = ((value / max).clamp(0.0, 1.0) * width as f64).round() as usize;
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

fn value_spans(attr: &Attr, app: &App) -> Vec<Span<'static>> {
    let dim = Style::default().fg(Color::DarkGray);
    let bold = Style::default().add_modifier(Modifier::BOLD);
    match &attr.state {
        State::Unavailable => return vec![Span::styled("not supported", dim)],
        State::Error(e) => {
            return vec![Span::styled(
                format!("⚠ {e}"),
                Style::default().fg(Color::Red),
            )]
        }
        State::Value(_) => {}
    }
    let current = attr.current().unwrap();
    let mut spans: Vec<Span<'static>> = match (&attr.kind, current) {
        (Kind::Toggle, Value::Bool(on)) => vec![Span::styled(
            if *on { "[ ON ]" } else { "[ off ]" },
            if *on {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            },
        )],
        (Kind::SwitchKey, Value::Text(t)) => {
            vec![Span::styled(format!("‹ {t} ›"), bold)]
        }
        (Kind::Enum, Value::Text(t)) => {
            vec![Span::styled(format!("◀ {t} ▶"), bold)]
        }
        (Kind::Percent { max }, Value::Pct(p)) => {
            let color = if *p > 80 {
                Color::Red
            } else if *p > 60 {
                Color::Yellow
            } else {
                Color::Cyan
            };
            let suffix = if *max == 100 {
                format!("{p}%")
            } else {
                format!("{p}/{max}")
            };
            vec![
                Span::styled(bar(*p as f64, *max as f64, 12), Style::default().fg(color)),
                Span::raw(format!(" {suffix}")),
            ]
        }
        (Kind::Sensor { unit, max }, Value::Num(n)) => {
            let color = if *unit == "°C" {
                if n > &90.0 {
                    Color::Red
                } else if n > &80.0 {
                    Color::Yellow
                } else {
                    Color::Green
                }
            } else {
                Color::Cyan
            };
            let history = app
                .history
                .get(&attr.id)
                .map(|h| sparkline(&h.iter().copied().collect::<Vec<f64>>(), *max))
                .unwrap_or_default();
            vec![
                Span::styled(format!("{n:>5.0}"), Style::default().fg(color)),
                Span::styled(format!(" {unit:<3}"), dim),
                Span::styled(history, Style::default().fg(color)),
            ]
        }
        (Kind::ReadText, Value::Text(t)) if t.is_empty() => {
            vec![Span::styled("—", dim)]
        }
        (Kind::ReadText, Value::Text(t)) | (Kind::EcGet, Value::Text(t)) => {
            vec![Span::raw(t.clone())]
        }
        (Kind::Percent { .. } | Kind::Sensor { .. }, other) => {
            vec![Span::raw(format!("{other:?}"))]
        }
        _ => vec![Span::styled("—", dim)],
    };

    if let Some(staged) = &attr.staged {
        let wire = staged.as_wire();
        if wire != current_wire(current) {
            spans.push(Span::styled(
                format!("  ⇒ apply: {wire}"),
                Style::default()
                    .fg(Color::LightYellow)
                    .add_modifier(Modifier::BOLD | Modifier::SLOW_BLINK),
            ));
        }
    }
    spans
}

fn current_wire(v: &Value) -> String {
    v.as_wire()
}

fn list_lines(app: &App) -> Vec<Line<'static>> {
    let Some(group) = app.disc.group(app.current_tab()) else {
        return vec![Line::from("no attributes in this group")];
    };
    if group.attrs.is_empty() {
        return vec![Line::from(Span::styled(
            "nothing exposed by this driver",
            Style::default().fg(Color::DarkGray),
        ))];
    }
    group
        .attrs
        .iter()
        .enumerate()
        .map(|(i, attr)| {
            let cursor = if i == app.sel {
                Span::styled("▸ ", Style::default().fg(Color::Cyan))
            } else {
                Span::raw("  ")
            };
            let label_style = if attr.current().is_some() {
                Style::default()
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let mut spans = vec![
                cursor,
                Span::styled(format!("{:<22}", attr.label), label_style),
            ];
            spans.extend(value_spans(attr, app));
            if attr.writable && matches!(attr.state, State::Value(_)) {
                spans.push(Span::styled("  (w)", Style::default().fg(Color::DarkGray)));
            }
            Line::from(spans)
        })
        .collect()
}

fn debug_lines(app: &App, body: Rect) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let dim = Style::default().fg(Color::DarkGray);

    if let Some(attr) = app.disc.find(&crate::model::attr::AttrId::EcFwVersion) {
        let v = attr.current().map(|v| v.as_wire()).unwrap_or_default();
        lines.push(Line::from(format!("EC firmware: {v}")));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  g  ", Style::default().fg(Color::Cyan)),
        Span::raw("read EC memory  (debug/ec_get)"),
    ]));
    if let Some((addr, val)) = &app.ec_get_last {
        lines.push(Line::from(vec![Span::styled(
            format!("       last: [{addr}] = {val}"),
            Style::default().fg(Color::Green),
        )]));
    }
    lines.push(Line::from(vec![
        Span::styled("  s  ", Style::default().fg(Color::Cyan)),
        Span::styled(
            "WRITE EC memory (debug/ec_set) — dangerous",
            Style::default().fg(Color::Red),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  d  ", Style::default().fg(Color::Cyan)),
        Span::raw("dump EC memory (debug/ec_dump)"),
    ]));
    lines.push(Line::from(""));

    if app.dump.is_empty() {
        lines.push(Line::from(Span::styled("no dump loaded", dim)));
    } else {
        lines.push(Line::from(Span::styled(
            format!(
                "ec_dump {} (↑/↓ PgUp/PgDn to scroll, {}/{})",
                app.dump.len(),
                app.dump_scroll + 1,
                app.dump.len()
            ),
            dim,
        )));
        let visible = body.height.saturating_sub(lines.len() as u16).max(1) as usize;
        for row in app.dump.iter().skip(app.dump_scroll).take(visible) {
            lines.push(Line::from(row.clone()));
        }
    }
    lines
}

fn centered(r: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(r.width.saturating_sub(2));
    let h = h.min(r.height.saturating_sub(2));
    Rect {
        x: r.x + (r.width.saturating_sub(w)) / 2,
        y: r.y + (r.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

fn draw_help(f: &mut Frame, area: Rect) {
    let area = centered(area, 78, 20);
    f.render_widget(Clear, area);
    let text = [
        ("Tab / Shift-Tab", "switch tabs"),
        ("↑/↓ j/k", "select attribute"),
        ("←/→ h/l", "change value (staged)"),
        ("Enter / Space", "apply staged value, or flip toggle"),
        ("Esc", "discard staged value / close dialogs"),
        ("r", "refresh current tab"),
        ("?", "toggle this help"),
        ("q / Ctrl-C", "quit"),
        ("", ""),
        ("DEBUG tab:", ""),
        ("g", "read EC address (ec_get)"),
        ("s", "write EC memory (ec_set) — risky!"),
        ("d", "load EC dump"),
        ("", ""),
        (
            "Writes that need root are executed through pkexec/sudo.",
            "",
        ),
        (
            "The privileged helper only writes allow-listed msi-ec paths.",
            "",
        ),
    ];
    let lines: Vec<Line> = text
        .iter()
        .map(|(k, d)| {
            if d.is_empty() {
                if k.is_empty() {
                    Line::from("")
                } else if k.ends_with(':') {
                    Line::from(Span::styled(
                        *k,
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ))
                } else {
                    Line::from(Span::styled(*k, Style::default().fg(Color::Gray)))
                }
            } else {
                Line::from(vec![
                    Span::styled(format!("{:<20}", *k), Style::default().fg(Color::Cyan)),
                    Span::raw(*d),
                ])
            }
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" help ")
                .title_alignment(Alignment::Center),
        ),
        area,
    );
}

fn draw_confirm(f: &mut Frame, app: &App, area: Rect) {
    let Mode::Confirm { value } = &app.mode else {
        return;
    };
    let path = app
        .disc
        .find(&crate::model::attr::AttrId::EcSet)
        .map(|a| a.path.display().to_string())
        .unwrap_or_default();
    let area = centered(area, 70, 8);
    f.render_widget(Clear, area);
    let lines = vec![
        Line::from(Span::styled(
            "⚠ Writing raw values into EC memory can brick your firmware!",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(format!("target: {path}")),
        Line::from(format!("value:  {value}")),
        Line::from(""),
        Line::from(Span::styled(
            "press [y] to write anyway · any other key aborts",
            Style::default().fg(Color::Yellow),
        )),
    ];
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red))
                .title(" DANGER ")
                .title_alignment(Alignment::Center),
        ),
        area,
    );
}

fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    let Mode::Input { kind, text } = &app.mode else {
        return;
    };
    let prompt = match kind {
        super::app::InputKind::EcGet => "ec_get · EC address (hex): ",
        super::app::InputKind::EcSet => "ec_set · 'aa=vv' (hex): ",
    };
    let area = Rect {
        x: area.x + 2,
        y: area.bottom().saturating_sub(1),
        width: area.width.saturating_sub(4).min(70),
        height: 1,
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(prompt, Style::default().fg(Color::Cyan)),
            Span::raw(text.clone()),
            Span::styled("▌", Style::default().add_modifier(Modifier::SLOW_BLINK)),
        ])),
        area,
    );
}

fn draw_missing_device(f: &mut Frame, app: &App, area: Rect) {
    let text = vec![
        Line::from(""),
        Line::from(Span::styled(
            "msi-ec driver not detected",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("No /sys/devices/platform/msi-ec directory was found."),
        Line::from(""),
        Line::from("If this is an MSI laptop:"),
        Line::from(Span::styled(
            "  1. install the module:  https://github.com/BeardOverflow/msi-ec",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(Span::styled(
            "     (Arch: `paru -S msi-ec-dkms-git`, or the in-tree kernel module)",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  2. load it:             sudo modprobe msi-ec",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(Span::styled(
            "  3. check support list:  https://github.com/BeardOverflow/msi-ec/discussions/277",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Press q to quit.",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let para = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title(" msi-ec-tui "));
    f.render_widget(para, area);
    let _ = &app.status;
}

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    if !app.disc.device_present {
        draw_missing_device(f, app, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(area);
    let (tabs_area, body_area, status_area) = (chunks[0], chunks[1], chunks[2]);

    let titles: Vec<String> = app
        .tabs
        .iter()
        .map(|t| {
            if *t == TabId::Debug {
                "⚠ DEBUG".to_string()
            } else {
                t.title().to_string()
            }
        })
        .collect();
    let tabs = Tabs::new(titles)
        .select(app.tab)
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .divider(Span::raw("  "));
    f.render_widget(tabs, tabs_area);

    if app.current_tab() == TabId::Debug {
        let lines = debug_lines(app, body_area);
        let para = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red))
                .title(" EC debug interface "),
        );
        f.render_widget(para, body_area);
    } else {
        let lines = list_lines(app);
        let para = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} ", app.current_tab().title())),
        );
        f.render_widget(para, body_area);
    }

    let status = Paragraph::new(Line::from(vec![
        Span::styled("● ", Style::default().fg(level_color(app.status.level))),
        Span::styled(
            app.status.msg.clone(),
            Style::default().fg(level_color(app.status.level)),
        ),
    ]));
    f.render_widget(status, status_area);

    if app.mode != Mode::Normal {
        match &app.mode {
            Mode::Help => draw_help(f, area),
            Mode::Confirm { .. } => draw_confirm(f, app, area),
            Mode::Input { .. } => draw_input(f, app, body_area),
            Mode::Normal => {}
        }
    }
}
