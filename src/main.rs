mod backend;
mod elevate;
mod model;
mod ui;

use crate::backend::SysfsBackend;
use crate::model::attr::AttrId;
use crate::ui::app::{Action, App, Status};
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self, Write};
use std::time::{Duration, Instant};

const TICK: Duration = Duration::from_millis(1000);

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let exit = match args.first().map(String::as_str) {
        Some("__write") | Some("__read") => elevate::helper_main(&args),
        Some("--help") | Some("-h") => {
            print_help();
            0
        }
        Some("--version" | "-V") => {
            println!("msi-ec-tui {}", env!("CARGO_PKG_VERSION"));
            0
        }
        _ => match real_main() {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("msi-ec-tui: {e:#}");
                1
            }
        },
    };
    std::process::exit(exit);
}

fn print_help() {
    println!(
        "msi-ec-tui {} — terminal UI for the msi-ec kernel driver

USAGE:
    msi-ec-tui                    start the TUI (reads work unprivileged)
    msi-ec-tui --help             show this help
    msi-ec-tui --version          print version

Writes to attributes that require root are re-executed through pkexec
(falling back to sudo), which prompts for authentication. The privileged
helper refuses any path outside the msi-ec/power-supply/leds trees.

The raw EC debug interface is only shown when the driver is loaded with
debug=1. Writing unknown EC addresses may damage your laptop; the EC can
be reset with the reset button on its bottom.",
        env!("CARGO_PKG_VERSION")
    );
}

fn enter_terminal() -> Result<()> {
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let _ = stdout.flush();
    Ok(())
}

fn leave_terminal() {
    let _ = terminal::disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
}

fn new_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    Ok(Terminal::new(CrosstermBackend::new(io::stdout()))?)
}

/// Run one action. Terminal is already suspended (raw mode off, main screen),
/// so pkexec/sudo can prompt freely.
fn run_action(app: &mut App, action: &Action) -> Result<(), String> {
    match action {
        Action::Write { path, value, .. } => {
            elevate::write_escalated(path, value).map_err(|e| format!("{path:?}: {e}"))
        }
        Action::Refresh => Ok(()),
        Action::EcGet { addr } => {
            let get = app.disc.msi_ec_dir().join("debug/ec_get");
            elevate::write_escalated(&get, addr).map_err(|e| format!("ec_get write: {e}"))?;
            let value = elevate::read_escalated(&get).map_err(|e| format!("ec_get read: {e}"))?;
            let value = value.lines().next().unwrap_or("").trim().to_string();
            if let Some(a) = app.disc.find_mut(&AttrId::EcGet) {
                use crate::model::attr::{State, Value};
                a.state = State::Value(Value::Text(value.clone()));
            }
            app.ec_get_last = Some((addr.clone(), value));
            Ok(())
        }
        Action::LoadDump => {
            let dump = app.disc.msi_ec_dir().join("debug/ec_dump");
            let text = elevate::read_escalated(&dump).map_err(|e| e.to_string())?;
            app.dump = text.lines().map(str::to_string).collect();
            app.dump_scroll = 0;
            Ok(())
        }
    }
}

fn real_main() -> Result<()> {
    let backend = SysfsBackend;
    let sysroot = std::env::var("MSI_EC_SYSROOT").unwrap_or_else(|_| "/sys".into());
    let discovery = model::discover(&backend, sysroot);
    let mut app = App::new(discovery);

    // Restore the terminal even on panic.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        leave_terminal();
        hook(info);
    }));

    enter_terminal()?;
    let mut term = new_terminal()?;

    let mut last_tick = Instant::now();
    while !app.quit {
        term.draw(|f| ui::render::draw(f, &app))?;

        let timeout = TICK.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key)
                    if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat =>
                {
                    app.on_key(key);
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }

        if last_tick.elapsed() >= TICK {
            app.disc.refresh_sensors(&backend);
            app.push_history();
            last_tick = Instant::now();
        }

        if let Some(action) = app.pending.take() {
            if action == Action::Refresh {
                let tab = app.current_tab();
                app.disc.refresh_tab(&backend, tab);
                app.set_status(Status::info("refreshed"));
                continue;
            }

            // Suspend the UI so the escalation prompt is usable.
            leave_terminal();
            let mut term_out = io::stderr();
            let _ = writeln!(
                term_out,
                "msi-ec-tui: authenticating for privileged EC access…"
            );
            let _ = term_out.flush();
            let result = run_action(&mut app, &action).map(|_| String::new());
            enter_terminal()?;
            term = new_terminal()?;
            app.record_result(&action, result);

            if matches!(action, Action::Write { .. }) {
                let tab = app.current_tab();
                app.disc.refresh_tab(&backend, tab);
            }
        }
    }

    leave_terminal();
    Ok(())
}
