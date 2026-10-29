//! Application state and key handling. Rendering lives in `ui.rs`.

use std::cmp::Ordering;
use std::time::{Duration, Instant};

use clap::ValueEnum;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::TableState;

use crate::collector::{Collector, ProcSample, Sample, StaticInfo};
use crate::history::History;
use crate::theme::{Theme, ThemeName};

const HISTORY: usize = 400;
const MIN_INTERVAL: Duration = Duration::from_millis(250);
const MAX_INTERVAL: Duration = Duration::from_secs(10);
const STATUS_TTL: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SortKey {
    Cpu,
    Mem,
    Io,
    Pid,
    Name,
}

impl SortKey {
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Cpu => "CPU",
            SortKey::Mem => "MEM",
            SortKey::Io => "I/O",
            SortKey::Pid => "PID",
            SortKey::Name => "NAME",
        }
    }

    fn next(self) -> Self {
        match self {
            SortKey::Cpu => SortKey::Mem,
            SortKey::Mem => SortKey::Io,
            SortKey::Io => SortKey::Pid,
            SortKey::Pid => SortKey::Name,
            SortKey::Name => SortKey::Cpu,
        }
    }

    /// Resource columns read best biggest-first, identifiers smallest-first.
    fn default_desc(self) -> bool {
        matches!(self, SortKey::Cpu | SortKey::Mem | SortKey::Io)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Normal,
    Filter,
    Confirm { pid: u32, name: String, force: bool },
    Help,
}

pub struct Histories {
    pub cpu: History,
    pub mem: History,
    pub rx: History,
    pub tx: History,
    pub io: History,
    pub gpu: History,
}

pub struct App {
    collector: Collector,
    pub info: StaticInfo,
    pub sample: Sample,
    pub hist: Histories,
    pub theme_name: ThemeName,
    pub interval: Duration,
    pub paused: bool,
    pub mode: Mode,
    pub sort: SortKey,
    pub sort_desc: bool,
    pub filter: String,
    pub table: TableState,
    /// Indices into `sample.processes`, filtered and sorted for display.
    pub rows: Vec<usize>,
    pub page_size: usize,
    selected_pid: Option<u32>,
    status: Option<(String, Instant)>,
    pub should_quit: bool,
}

impl App {
    pub fn new(
        mut collector: Collector,
        theme: ThemeName,
        interval: Duration,
        sort: SortKey,
    ) -> Self {
        let info = collector.info().clone();
        let sample = collector.sample();
        let mut app = Self {
            collector,
            info,
            sample: Sample::default(),
            hist: Histories {
                cpu: History::new(HISTORY),
                mem: History::new(HISTORY),
                rx: History::new(HISTORY),
                tx: History::new(HISTORY),
                io: History::new(HISTORY),
                gpu: History::new(HISTORY),
            },
            theme_name: theme,
            interval: interval.clamp(MIN_INTERVAL, MAX_INTERVAL),
            paused: false,
            mode: Mode::Normal,
            sort,
            sort_desc: sort.default_desc(),
            filter: String::new(),
            table: TableState::default().with_selected(Some(0)),
            rows: Vec::new(),
            page_size: 10,
            selected_pid: None,
            status: None,
            should_quit: false,
        };
        app.ingest(sample);
        app
    }

    pub fn theme(&self) -> Theme {
        Theme::get(self.theme_name)
    }

    pub fn tick(&mut self) {
        if !self.paused {
            let sample = self.collector.sample();
            self.ingest(sample);
        }
    }

    fn ingest(&mut self, sample: Sample) {
        self.hist.cpu.push(sample.cpu.total);
        self.hist.mem.push(sample.memory.used_pct());
        self.hist.rx.push(sample.network.rx_rate);
        self.hist.tx.push(sample.network.tx_rate);
        self.hist
            .io
            .push(sample.disk_io.read + sample.disk_io.write);
        self.hist
            .gpu
            .push(sample.gpu.as_ref().map(|g| g.usage).unwrap_or(0.0));
        self.sample = sample;
        self.rebuild_rows();
    }

    pub fn status(&self) -> Option<&str> {
        self.status
            .as_ref()
            .filter(|(_, at)| at.elapsed() < STATUS_TTL)
            .map(|(msg, _)| msg.as_str())
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now()));
    }

    pub fn selected_process(&self) -> Option<&ProcSample> {
        let idx = *self.rows.get(self.table.selected()?)?;
        self.sample.processes.get(idx)
    }

    /// Re-applies filter and sort, keeping the same process selected even
    /// when it moves to another row.
    pub fn rebuild_rows(&mut self) {
        let needle = self.filter.to_lowercase();
        let procs = &self.sample.processes;
        let mut rows: Vec<usize> = (0..procs.len())
            .filter(|&i| {
                needle.is_empty()
                    || procs[i].name.to_lowercase().contains(&needle)
                    || procs[i].user.to_lowercase().contains(&needle)
                    || procs[i].pid.to_string().starts_with(&needle)
            })
            .collect();
        let key = self.sort;
        rows.sort_by(|&a, &b| {
            let ord = compare(&procs[a], &procs[b], key);
            let ord = if self.sort_desc { ord.reverse() } else { ord };
            ord.then(procs[a].pid.cmp(&procs[b].pid))
        });
        self.rows = rows;

        let target = self
            .selected_pid
            .and_then(|pid| self.rows.iter().position(|&i| procs[i].pid == pid));
        let index = match target {
            Some(i) => i,
            None => self
                .table
                .selected()
                .unwrap_or(0)
                .min(self.rows.len().saturating_sub(1)),
        };
        self.table.select(if self.rows.is_empty() {
            None
        } else {
            Some(index)
        });
        self.remember_selection();
    }

    fn remember_selection(&mut self) {
        self.selected_pid = self.selected_process().map(|p| p.pid);
    }

    fn move_selection(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() as isize - 1;
        let current = self.table.selected().unwrap_or(0) as isize;
        self.table
            .select(Some((current + delta).clamp(0, last) as usize));
        self.remember_selection();
    }

    fn set_sort(&mut self, key: SortKey) {
        if self.sort == key {
            self.sort_desc = !self.sort_desc;
        } else {
            self.sort = key;
            self.sort_desc = key.default_desc();
        }
        self.rebuild_rows();
    }

    fn change_interval(&mut self, faster: bool) {
        let ms = self.interval.as_millis() as u64;
        let step = if ms > 2000 || (!faster && ms >= 2000) {
            1000
        } else {
            250
        };
        let ms = if faster {
            ms.saturating_sub(step)
        } else {
            ms + step
        };
        self.interval = Duration::from_millis(ms).clamp(MIN_INTERVAL, MAX_INTERVAL);
        self.set_status(format!("Refresh every {:.2}s", self.interval.as_secs_f64()));
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        match self.mode.clone() {
            Mode::Help => self.mode = Mode::Normal,
            Mode::Confirm { pid, name, force } => {
                if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
                    let sig = if force { "SIGKILL" } else { "SIGTERM" };
                    match self.collector.signal(pid, force) {
                        Ok(()) => self.set_status(format!("Sent {sig} to {name} ({pid})")),
                        Err(e) => self.set_status(format!("Could not signal {name} ({pid}): {e}")),
                    }
                } else {
                    self.set_status("Cancelled");
                }
                self.mode = Mode::Normal;
            }
            Mode::Filter => match key.code {
                KeyCode::Esc => {
                    self.filter.clear();
                    self.mode = Mode::Normal;
                    self.rebuild_rows();
                }
                KeyCode::Enter => self.mode = Mode::Normal,
                KeyCode::Backspace => {
                    self.filter.pop();
                    self.rebuild_rows();
                }
                KeyCode::Up => self.move_selection(-1),
                KeyCode::Down => self.move_selection(1),
                KeyCode::Char(c) => {
                    self.filter.push(c);
                    self.rebuild_rows();
                }
                _ => {}
            },
            Mode::Normal => self.on_normal_key(key.code),
        }
    }

    fn on_normal_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => {
                if self.filter.is_empty() || code == KeyCode::Char('q') {
                    self.should_quit = true;
                } else {
                    self.filter.clear();
                    self.rebuild_rows();
                }
            }
            KeyCode::Char(' ') => {
                self.paused = !self.paused;
                self.set_status(if self.paused { "Paused" } else { "Resumed" });
            }
            KeyCode::Char('+') | KeyCode::Char('=') => self.change_interval(false),
            KeyCode::Char('-') | KeyCode::Char('_') => self.change_interval(true),
            KeyCode::Char('c') => self.set_sort(SortKey::Cpu),
            KeyCode::Char('m') => self.set_sort(SortKey::Mem),
            KeyCode::Char('i') => self.set_sort(SortKey::Io),
            KeyCode::Char('p') => self.set_sort(SortKey::Pid),
            KeyCode::Char('n') => self.set_sort(SortKey::Name),
            KeyCode::Char('s') => {
                self.sort = self.sort.next();
                self.sort_desc = self.sort.default_desc();
                self.rebuild_rows();
            }
            KeyCode::Char('r') => {
                self.sort_desc = !self.sort_desc;
                self.rebuild_rows();
            }
            KeyCode::Char('/') => self.mode = Mode::Filter,
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::PageUp => self.move_selection(-(self.page_size as isize)),
            KeyCode::PageDown => self.move_selection(self.page_size as isize),
            KeyCode::Home | KeyCode::Char('g') => self.move_selection(isize::MIN / 2),
            KeyCode::End | KeyCode::Char('G') => self.move_selection(isize::MAX / 2),
            KeyCode::Char('x') | KeyCode::Delete | KeyCode::Char('X') => {
                if let Some(p) = self.selected_process() {
                    self.mode = Mode::Confirm {
                        pid: p.pid,
                        name: p.name.clone(),
                        force: code == KeyCode::Char('X'),
                    };
                }
            }
            KeyCode::Char('t') => {
                self.theme_name = self.theme_name.next();
                self.set_status(format!("Theme: {}", self.theme().name));
            }
            KeyCode::Char('?') | KeyCode::Char('h') => self.mode = Mode::Help,
            _ => {}
        }
    }
}

fn compare(a: &ProcSample, b: &ProcSample, key: SortKey) -> Ordering {
    match key {
        SortKey::Cpu => a.cpu.total_cmp(&b.cpu),
        SortKey::Mem => a.memory.cmp(&b.memory),
        SortKey::Io => (a.read_rate + a.write_rate).total_cmp(&(b.read_rate + b.write_rate)),
        SortKey::Pid => a.pid.cmp(&b.pid),
        SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyEvent;

    fn proc(pid: u32, name: &str, cpu: f64, memory: u64) -> ProcSample {
        ProcSample {
            pid,
            name: name.into(),
            user: "elyo".into(),
            cpu,
            memory,
            read_rate: 0.0,
            write_rate: 0.0,
            status: "Run".into(),
        }
    }

    fn app_with(procs: Vec<ProcSample>) -> App {
        let collector = Collector::new().expect("collector");
        let mut app = App::new(
            collector,
            ThemeName::Ansi,
            Duration::from_secs(1),
            SortKey::Cpu,
        );
        app.sample.processes = procs;
        app.selected_pid = None;
        app.table.select(Some(0));
        app.rebuild_rows();
        app
    }

    fn press(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent::from(code));
    }

    #[test]
    fn sorts_and_keeps_selection_across_resort() {
        let mut app = app_with(vec![
            proc(1, "launchd", 1.0, 30),
            proc(2, "Safari", 40.0, 10),
            proc(3, "kernel_task", 10.0, 20),
        ]);
        assert_eq!(app.selected_process().unwrap().name, "Safari");
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected_process().unwrap().name, "kernel_task");
        press(&mut app, KeyCode::Char('m'));
        // kernel_task stays selected even though it moved rows.
        assert_eq!(app.selected_process().unwrap().name, "kernel_task");
        assert_eq!(app.sample.processes[app.rows[0]].name, "launchd");
    }

    #[test]
    fn same_sort_key_toggles_direction() {
        let mut app = app_with(vec![proc(5, "b", 0.0, 0), proc(9, "a", 0.0, 0)]);
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(app.sample.processes[app.rows[0]].pid, 5);
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(app.sample.processes[app.rows[0]].pid, 9);
    }

    #[test]
    fn filter_matches_name_user_and_pid() {
        let mut app = app_with(vec![proc(42, "Safari", 1.0, 1), proc(7, "Finder", 2.0, 2)]);
        press(&mut app, KeyCode::Char('/'));
        for c in "saf".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        assert_eq!(app.rows.len(), 1);
        press(&mut app, KeyCode::Backspace);
        press(&mut app, KeyCode::Backspace);
        press(&mut app, KeyCode::Backspace);
        press(&mut app, KeyCode::Char('7'));
        assert_eq!(app.selected_process().unwrap().name, "Finder");
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.rows.len(), 2);
    }

    #[test]
    fn kill_requires_confirmation() {
        let mut app = app_with(vec![proc(u32::MAX - 1, "ghost", 1.0, 1)]);
        press(&mut app, KeyCode::Char('x'));
        assert!(matches!(app.mode, Mode::Confirm { force: false, .. }));
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.status(), Some("Cancelled"));
    }

    #[test]
    fn interval_is_clamped() {
        let mut app = app_with(vec![]);
        for _ in 0..20 {
            press(&mut app, KeyCode::Char('-'));
        }
        assert_eq!(app.interval, MIN_INTERVAL);
        for _ in 0..40 {
            press(&mut app, KeyCode::Char('+'));
        }
        assert_eq!(app.interval, MAX_INTERVAL);
    }
}
