use crate::model::attr::{Attr, AttrId, Kind, Value};
use crate::model::{Discovery, TabId};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

pub const HISTORY_LEN: usize = 90;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Success,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub struct Status {
    pub level: Level,
    pub msg: String,
}

impl Status {
    pub fn info(msg: impl Into<String>) -> Self {
        Self {
            level: Level::Info,
            msg: msg.into(),
        }
    }
    pub fn success(msg: impl Into<String>) -> Self {
        Self {
            level: Level::Success,
            msg: msg.into(),
        }
    }
    pub fn warn(msg: impl Into<String>) -> Self {
        Self {
            level: Level::Warn,
            msg: msg.into(),
        }
    }
    pub fn error(msg: impl Into<String>) -> Self {
        Self {
            level: Level::Error,
            msg: msg.into(),
        }
    }
}

/// Something the event loop must do outside the UI (it owns the terminal).
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Write {
        attr: AttrId,
        path: PathBuf,
        value: String,
    },
    Refresh,
    /// Write an address into debug/ec_get, then read the value back.
    EcGet {
        addr: String,
    },
    LoadDump,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputKind {
    EcGet,
    EcSet,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Normal,
    Input { kind: InputKind, text: String },
    Confirm { value: String },
    Help,
}

pub struct App {
    pub disc: Discovery,
    pub tabs: Vec<TabId>,
    pub tab: usize,
    pub sel: usize,
    pub status: Status,
    pub mode: Mode,
    pub pending: Option<Action>,
    pub quit: bool,
    pub history: HashMap<AttrId, VecDeque<f64>>,
    pub ec_get_last: Option<(String, String)>,
    pub dump: Vec<String>,
    pub dump_scroll: usize,
}

impl App {
    pub fn new(disc: Discovery) -> Self {
        let mut tabs = vec![
            TabId::Performance,
            TabId::Thermals,
            TabId::Battery,
            TabId::InputLed,
            TabId::Info,
        ];
        if disc.debug_present {
            tabs.push(TabId::Debug);
        }
        let status = if disc.device_present {
            Status::info("r refresh · ? help · q quit")
        } else {
            Status::warn("msi-ec driver not detected")
        };
        Self {
            disc,
            tabs,
            tab: 0,
            sel: 0,
            status,
            mode: Mode::Normal,
            pending: None,
            quit: false,
            history: HashMap::new(),
            ec_get_last: None,
            dump: Vec::new(),
            dump_scroll: 0,
        }
    }

    pub fn current_tab(&self) -> TabId {
        self.tabs[self.tab]
    }

    fn rows(&self) -> usize {
        self.disc
            .group(self.current_tab())
            .map(|g| g.attrs.len())
            .unwrap_or(0)
    }

    pub fn selected_attr(&self) -> Option<&Attr> {
        let tab = self.current_tab();
        if tab == TabId::Debug {
            return None;
        }
        self.disc.group(tab)?.attrs.get(self.sel)
    }

    pub fn set_status(&mut self, s: Status) {
        self.status = s;
    }

    /// Called by the event loop after an action finished.
    pub fn record_result(&mut self, action: &Action, result: Result<String, String>) {
        match (action, result) {
            (Action::Write { attr, value, .. }, Ok(msg)) => {
                if let Some(a) = self.disc.find_mut(attr) {
                    a.staged = None;
                }
                let short = if msg.is_empty() { value.clone() } else { msg };
                self.set_status(Status::success(format!("wrote {short}")));
            }
            (Action::Write { attr, .. }, Err(e)) => {
                if let Some(a) = self.disc.find_mut(attr) {
                    a.staged = None;
                }
                self.set_status(Status::error(format!("write failed: {e}")));
            }
            (Action::Refresh, Ok(_)) => self.set_status(Status::info("refreshed")),
            (Action::Refresh, Err(e)) => self.set_status(Status::error(e)),
            (Action::EcGet { .. }, Ok(v)) => self.set_status(Status::success(v)),
            (Action::EcGet { .. }, Err(e)) => self.set_status(Status::error(e)),
            (Action::LoadDump, Ok(_)) => self.set_status(Status::success("ec_dump loaded")),
            (Action::LoadDump, Err(e)) => self.set_status(Status::error(format!("ec_dump: {e}"))),
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        match self.mode.clone() {
            Mode::Help => {
                self.mode = Mode::Normal;
            }
            Mode::Confirm { value } => match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    if let Some(path) = self.debug_ec_set_path() {
                        self.pending = Some(Action::Write {
                            attr: AttrId::EcSet,
                            path,
                            value,
                        });
                    }
                    self.mode = Mode::Normal;
                }
                _ => {
                    self.mode = Mode::Normal;
                    self.set_status(Status::info("cancelled"));
                }
            },
            Mode::Input { kind, mut text } => match key.code {
                KeyCode::Esc => {
                    self.mode = Mode::Normal;
                    self.set_status(Status::info("cancelled"));
                }
                KeyCode::Enter => {
                    self.mode = Mode::Normal;
                    self.submit_input(kind, text.trim().to_string());
                }
                KeyCode::Backspace => {
                    text.pop();
                    self.mode = Mode::Input { kind, text };
                }
                KeyCode::Char(c) if !c.is_control() => {
                    text.push(c);
                    self.mode = Mode::Input { kind, text };
                }
                _ => {}
            },
            Mode::Normal => self.on_key_normal(key),
        }
    }

    fn debug_ec_set_path(&self) -> Option<PathBuf> {
        self.disc.find(&AttrId::EcSet).map(|a| a.path.clone())
    }

    fn submit_input(&mut self, kind: InputKind, text: String) {
        match kind {
            InputKind::EcGet => {
                let addr = text
                    .trim_start_matches("0x")
                    .trim_start_matches("0X")
                    .to_string();
                if addr.is_empty() || addr.len() > 2 || !addr.chars().all(|c| c.is_ascii_hexdigit())
                {
                    self.set_status(Status::error("ec_get: expected 1-2 hex digits"));
                } else {
                    self.pending = Some(Action::EcGet { addr });
                }
            }
            InputKind::EcSet => {
                let Some((a, v)) = text.split_once('=') else {
                    self.set_status(Status::error("ec_set: expected 'aa=vv'"));
                    return;
                };
                let ok = a.len() == 2
                    && v.len() == 2
                    && a.chars().all(|c| c.is_ascii_hexdigit())
                    && v.chars().all(|c| c.is_ascii_hexdigit());
                if !ok {
                    self.set_status(Status::error("ec_set: expected 'aa=vv' hex pairs"));
                } else {
                    self.mode = Mode::Confirm { value: text };
                }
            }
        }
    }

    fn on_key_normal(&mut self, key: KeyEvent) {
        let tab = self.current_tab();
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Tab => {
                self.tab = (self.tab + 1) % self.tabs.len();
                self.sel = 0;
            }
            KeyCode::BackTab => {
                self.tab = (self.tab + self.tabs.len() - 1) % self.tabs.len();
                self.sel = 0;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.rows() > 0 {
                    self.sel = (self.sel + 1).min(self.rows() - 1);
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.sel = self.sel.saturating_sub(1);
            }
            KeyCode::Char('r') => self.pending = Some(Action::Refresh),
            _ => {
                if tab == TabId::Debug {
                    self.on_key_debug(key);
                } else {
                    self.on_key_attr(key);
                }
            }
        }
    }

    fn on_key_debug(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('g') => {
                self.mode = Mode::Input {
                    kind: InputKind::EcGet,
                    text: String::new(),
                };
            }
            KeyCode::Char('s') => {
                self.mode = Mode::Input {
                    kind: InputKind::EcSet,
                    text: String::new(),
                };
            }
            KeyCode::Char('d') => self.pending = Some(Action::LoadDump),
            KeyCode::PageUp | KeyCode::Char('K') => {
                self.dump_scroll = self.dump_scroll.saturating_sub(10);
            }
            KeyCode::PageDown | KeyCode::Char('J') => {
                self.dump_scroll = (self.dump_scroll + 10).min(self.dump.len());
            }
            _ => {}
        }
    }

    fn on_key_attr(&mut self, key: KeyEvent) {
        let Some(attr) = self.selected_attr().cloned() else {
            return;
        };
        match key.code {
            KeyCode::Left | KeyCode::Char('h') => self.step(&attr, -1),
            KeyCode::Right | KeyCode::Char('l') => self.step(&attr, 1),
            KeyCode::Enter | KeyCode::Char(' ') => self.commit(&attr),
            KeyCode::Esc => {
                if let Some(a) = self.disc.find_mut(&attr.id) {
                    if a.staged.take().is_some() {
                        self.set_status(Status::info("staged change discarded"));
                    }
                }
            }
            _ => {}
        }
    }

    /// Options offered by ← / → for an attribute.
    fn choices(attr: &Attr) -> Vec<Value> {
        match &attr.kind {
            Kind::Toggle => vec![Value::Bool(false), Value::Bool(true)],
            Kind::SwitchKey => {
                vec![Value::Text("left".into()), Value::Text("right".into())]
            }
            Kind::Enum => attr.options.iter().cloned().map(Value::Text).collect(),
            Kind::Percent { max } => (0..=*max as u16).map(|i| Value::Pct(i as u8)).collect(),
            _ => Vec::new(),
        }
    }

    fn step(&mut self, attr: &Attr, dir: i32) {
        if !attr.writable {
            self.set_status(Status::warn(format!("{} is read-only", attr.label)));
            return;
        }
        let choices = Self::choices(attr);
        if choices.is_empty() {
            return;
        }
        let current = attr.staged.clone().or_else(|| attr.current().cloned());
        let Some(idx) = current.and_then(|c| choices.iter().position(|v| *v == c)) else {
            // Unknown current value: stage the first choice.
            if let Some(a) = self.disc.find_mut(&attr.id) {
                a.staged = Some(choices[0].clone());
            }
            return;
        };
        let next = (idx as i32 + dir).rem_euclid(choices.len() as i32) as usize;
        if let Some(a) = self.disc.find_mut(&attr.id) {
            a.staged = Some(choices[next].clone());
        }
    }

    fn commit(&mut self, attr: &Attr) {
        if !attr.writable {
            self.set_status(Status::warn(format!("{} is read-only", attr.label)));
            return;
        }
        let value = match (&attr.staged, attr.current()) {
            (Some(s), Some(c)) if s == c => {
                self.set_status(Status::info("already set"));
                return;
            }
            (Some(_), _) => attr.staged.clone(),
            (None, Some(Value::Bool(b))) => Some(Value::Bool(!b)),
            (None, Some(Value::Text(t)))
                if matches!(attr.kind, Kind::SwitchKey) && (t == "left" || t == "right") =>
            {
                Some(Value::Text(
                    if t == "left" { "right" } else { "left" }.into(),
                ))
            }
            (None, Some(_)) => {
                self.set_status(Status::info("use ←/→ to pick a value, Enter to apply"));
                return;
            }
            (None, None) => {
                self.set_status(Status::warn("attribute unavailable"));
                return;
            }
        };
        if let Some(v) = value {
            self.pending = Some(Action::Write {
                attr: attr.id.clone(),
                path: attr.path.clone(),
                value: v.as_wire(),
            });
        }
    }

    /// Push current sensor values into their history rings.
    pub fn push_history(&mut self) {
        for id in [
            AttrId::CpuTemp,
            AttrId::CpuFan,
            AttrId::GpuTemp,
            AttrId::GpuFan,
        ] {
            if let Some(Some(Value::Num(n))) = self.disc.find(&id).map(|a| a.current().cloned()) {
                let ring = self.history.entry(id).or_default();
                if ring.back() != Some(&n) {
                    ring.push_back(n);
                    if ring.len() > HISTORY_LEN {
                        ring.pop_front();
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::fake::FakeBackend;
    use crossterm::event::KeyEvent;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn app_with_fixture() -> App {
        let mut f = FakeBackend::new();
        let ec = "/sys/devices/platform/msi-ec";
        f.add(format!("{ec}/shift_mode"), "comfort")
            .add(
                format!("{ec}/available_shift_modes"),
                "eco\ncomfort\nsport\nturbo",
            )
            .add(format!("{ec}/debug/ec_get"), "f5")
            .add(format!("{ec}/debug/ec_set"), "");
        App::new(crate::model::discover(&f, "/sys"))
    }

    fn press(app: &mut App, codes: &[KeyCode]) {
        for c in codes {
            app.on_key(key(*c));
        }
    }

    #[test]
    fn debug_ec_set_flow_reaches_confirm() {
        let mut app = app_with_fixture();
        assert_eq!(app.tabs.len(), 6);
        for _ in 0..5 {
            app.on_key(key(KeyCode::Tab));
        }
        assert_eq!(app.current_tab(), TabId::Debug);
        press(
            &mut app,
            &[
                KeyCode::Char('s'),
                KeyCode::Char('f'),
                KeyCode::Char('5'),
                KeyCode::Char('='),
                KeyCode::Char('0'),
                KeyCode::Char('1'),
                KeyCode::Enter,
            ],
        );
        assert!(
            matches!(&app.mode, Mode::Confirm { value } if value == "f5=01"),
            "mode: {:?}",
            app.mode
        );
    }

    #[test]
    fn toggle_commit_produces_write() {
        let mut f = FakeBackend::new();
        let ec = "/sys/devices/platform/msi-ec";
        f.add(format!("{ec}/cooler_boost"), "off");
        let mut app = App::new(crate::model::discover(&f, "/sys"));
        // Performance tab, second row is cooler_boost
        app.on_key(key(KeyCode::Down));
        app.on_key(key(KeyCode::Enter));
        match app.pending.take() {
            Some(Action::Write { value, .. }) => assert_eq!(value, "on"),
            other => panic!("expected write, got {other:?}"),
        }
    }

    #[test]
    fn enum_stage_then_commit() {
        let mut app = app_with_fixture();
        // Performance tab, first row shift_mode = comfort
        app.on_key(key(KeyCode::Right));
        app.on_key(key(KeyCode::Right));
        let attr = app.selected_attr().unwrap();
        assert_eq!(attr.staged, Some(Value::Text("turbo".into())));
        app.on_key(key(KeyCode::Enter));
        match app.pending.take() {
            Some(Action::Write { value, .. }) => assert_eq!(value, "turbo"),
            other => panic!("expected write, got {other:?}"),
        }
    }
}
