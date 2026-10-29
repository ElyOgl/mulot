//! vigil: a fast, beautiful terminal system monitor.

mod app;
mod collector;
mod fmt;
mod history;
mod platform;
mod theme;
mod ui;

use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Parser;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use serde::Serialize;

use crate::app::{App, SortKey};
use crate::collector::{Collector, Sample, StaticInfo};
use crate::theme::ThemeName;

#[derive(Parser, Debug)]
#[command(name = "vigil", version, about, long_about = None)]
struct Cli {
    /// Refresh interval in milliseconds (250–10000).
    #[arg(short, long, default_value_t = 1000, value_parser = clap::value_parser!(u64).range(250..=10_000))]
    interval: u64,

    /// Color theme (default: auto-detected from the terminal).
    #[arg(short, long, value_enum, env = "VIGIL_THEME")]
    theme: Option<ThemeName>,

    /// Initial process sort column.
    #[arg(short, long, value_enum, default_value_t = SortKey::Cpu)]
    sort: SortKey,

    /// Print one JSON snapshot of the whole system and exit.
    #[arg(long)]
    json: bool,

    /// Number of top processes (by CPU) to include in --json output.
    #[arg(long, default_value_t = 15)]
    top: usize,
}

#[derive(Serialize)]
struct Report<'a> {
    vigil_version: &'static str,
    system: &'a StaticInfo,
    #[serde(flatten)]
    sample: &'a Sample,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.json {
        return print_json(cli.top);
    }
    let collector = Collector::new()?;
    let theme = cli.theme.unwrap_or_else(ThemeName::detect);
    let app = App::new(
        collector,
        theme,
        Duration::from_millis(cli.interval),
        cli.sort,
    );
    ratatui::run(|terminal| run(terminal, app))
}

fn run(terminal: &mut ratatui::DefaultTerminal, mut app: App) -> Result<()> {
    let mut next_tick = Instant::now() + app.interval;
    loop {
        terminal.draw(|f| ui::draw(f, &mut app))?;
        let timeout = next_tick.saturating_duration_since(Instant::now());
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => app.on_key(key),
                _ => {}
            }
        }
        if app.should_quit {
            return Ok(());
        }
        if Instant::now() >= next_tick {
            app.tick();
            next_tick = Instant::now() + app.interval;
        }
    }
}

fn print_json(top: usize) -> Result<()> {
    let mut collector = Collector::new()?;
    std::thread::sleep(Duration::from_millis(500));
    let mut sample = collector.sample();
    sample.processes.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
    sample.processes.truncate(top);
    let report = Report {
        vigil_version: env!("CARGO_PKG_VERSION"),
        system: collector.info(),
        sample: &sample,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
