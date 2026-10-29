//! Rendering. The layout adapts to the terminal size: wide terminals get a
//! full dashboard, small ones degrade to gauges plus the process table.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Cell, Clear, Paragraph, Row, Sparkline, SparklineBar, Table,
};

use crate::app::{App, Mode, SortKey};
use crate::fmt;
use crate::history::History;
use crate::platform::Pressure;
use crate::theme::Theme;

const EIGHTHS: [&str; 8] = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"];

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);
    draw_header(f, header, app);
    draw_footer(f, footer, app);

    if body.width < 60 || body.height < 14 {
        draw_compact(f, body, app);
    } else {
        draw_dashboard(f, body, app);
    }

    match app.mode.clone() {
        Mode::Help => draw_help(f, area, &app.theme()),
        Mode::Confirm { pid, name, force } => {
            draw_confirm(f, area, &app.theme(), pid, &name, force)
        }
        _ => {}
    }
}

fn draw_dashboard(f: &mut Frame, area: Rect, app: &mut App) {
    let h = area.height;
    let top_h = if h < 24 {
        h / 2
    } else {
        (h * 40 / 100).clamp(10, 20)
    };
    let mid_h = if h >= 30 {
        (h * 22 / 100).clamp(7, 11)
    } else {
        0
    };
    let [top, mid, procs] = Layout::vertical([
        Constraint::Length(top_h),
        Constraint::Length(mid_h),
        Constraint::Fill(1),
    ])
    .areas(area);

    let split = if top.width >= 110 { 62 } else { 56 };
    let [cpu, mem] =
        Layout::horizontal([Constraint::Percentage(split), Constraint::Fill(1)]).areas(top);
    draw_cpu(f, cpu, app);
    draw_memory(f, mem, app);

    if mid_h > 0 {
        let [net, disk, sys] = Layout::horizontal([
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
        ])
        .areas(mid);
        draw_network(f, net, app);
        draw_disks(f, disk, app);
        draw_system(f, sys, app);
    }
    draw_processes(f, procs, app);
}

fn draw_compact(f: &mut Frame, area: Rect, app: &mut App) {
    let t = app.theme();
    let [cpu, mem, procs] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(area);
    let s = &app.sample;
    let bar_w = area.width.saturating_sub(12);
    let cpu_line = gauge_line("CPU ", s.cpu.total, bar_w, &t, t.cpu);
    let mem_line = gauge_line("MEM ", s.memory.used_pct(), bar_w, &t, t.mem);
    f.render_widget(Paragraph::new(cpu_line), cpu);
    f.render_widget(Paragraph::new(mem_line), mem);
    draw_processes(f, procs, app);
}

fn panel<'a>(t: &Theme, title: &'a str, color: Color) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(t.border))
        .title(Line::from(Span::styled(
            format!(" {title} "),
            Style::new().fg(color).add_modifier(Modifier::BOLD),
        )))
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let info = &app.info;
    let sep = Span::styled(" │ ", Style::new().fg(t.border));
    let chip = match info.topology {
        Some(top) => format!(
            "{} · {}P+{}E",
            info.cpu_model, top.performance, top.efficiency
        ),
        None => format!("{} · {} threads", info.cpu_model, info.logical_cores),
    };
    let mut right = Vec::new();
    if app.paused {
        right.push(Span::styled(
            " ⏸ PAUSED ",
            Style::new()
                .fg(t.accent_fg)
                .bg(t.warn)
                .add_modifier(Modifier::BOLD),
        ));
        right.push(Span::raw(" "));
    }
    right.push(Span::styled(
        format!("⟳ {:.2}s ", app.interval.as_secs_f64()),
        Style::new().fg(t.muted),
    ));
    right.push(Span::styled(
        format!("{} ", crate::platform::local_time()),
        Style::new().fg(t.title).add_modifier(Modifier::BOLD),
    ));
    let right = Line::from(right).right_aligned();
    let right_w = right.width() as u16;

    // Segments in display order, tagged with a drop priority (lowest goes
    // first when the terminal is too narrow for everything).
    let mut segments: Vec<(u8, Vec<Span>)> = vec![
        (
            9,
            vec![
                Span::styled(
                    " ◆ vigil ",
                    Style::new()
                        .fg(t.accent_fg)
                        .bg(t.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    info.host.clone(),
                    Style::new().fg(t.title).add_modifier(Modifier::BOLD),
                ),
            ],
        ),
        (
            1,
            vec![
                sep.clone(),
                Span::styled(info.os.clone(), Style::new().fg(t.fg)),
            ],
        ),
        (
            3,
            vec![sep.clone(), Span::styled(chip, Style::new().fg(t.fg))],
        ),
        (
            2,
            vec![
                sep,
                Span::styled(
                    format!("up {}", fmt::duration(app.sample.uptime_secs)),
                    Style::new().fg(t.muted),
                ),
            ],
        ),
    ];
    let width = |segs: &[(u8, Vec<Span>)]| -> usize {
        segs.iter().flat_map(|(_, s)| s).map(|s| s.width()).sum()
    };
    let room = area.width.saturating_sub(right_w + 1) as usize;
    while width(&segments) > room && segments.len() > 1 {
        let lowest = (1..segments.len())
            .min_by_key(|&i| segments[i].0)
            .unwrap_or(1);
        segments.remove(lowest);
    }
    let left = Line::from(
        segments
            .into_iter()
            .flat_map(|(_, s)| s)
            .collect::<Vec<_>>(),
    );
    let [l, r] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(right_w)]).areas(area);
    f.render_widget(Paragraph::new(left), l);
    f.render_widget(Paragraph::new(right), r);
}

/// Lays out span groups left to right, starting a new line instead of ever
/// splitting a group.
fn flow(groups: Vec<Vec<Span<'static>>>, width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut current: Vec<Span> = Vec::new();
    let mut used = 0usize;
    for group in groups {
        let w: usize = group.iter().map(|s| s.width()).sum();
        if used > 0 && used + w > width as usize {
            lines.push(Line::from(std::mem::take(&mut current)));
            used = 0;
        }
        used += w;
        current.extend(group);
    }
    if !current.is_empty() {
        lines.push(Line::from(current));
    }
    lines
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let key =
        |k: &'static str| Span::styled(k, Style::new().fg(t.accent).add_modifier(Modifier::BOLD));
    let label = |l: &'static str| Span::styled(l, Style::new().fg(t.muted));
    let line = if app.mode == Mode::Filter {
        Line::from(vec![
            key(" / "),
            Span::styled(
                app.filter.clone(),
                Style::new().fg(t.fg).add_modifier(Modifier::BOLD),
            ),
            Span::styled("█", Style::new().fg(t.accent)),
            label("   enter keep · esc clear · ↑↓ select"),
        ])
    } else if let Some(msg) = app.status() {
        Line::from(vec![
            Span::styled(" ● ", Style::new().fg(t.accent)),
            Span::styled(msg.to_string(), Style::new().fg(t.fg)),
        ])
    } else {
        let hints = [
            (" q", " quit"),
            ("space", " pause"),
            ("/", " filter"),
            ("c m i p n", " sort"),
            ("x", " kill"),
            ("t", " theme"),
            ("+-", " speed"),
        ];
        let help = [key("?"), label(" help")];
        let budget = (area.width as usize).saturating_sub(7);
        let mut spans = Vec::new();
        let mut used = 0;
        for (k, l) in hints {
            let w = k.chars().count() + l.chars().count() + 2;
            if used + w > budget {
                break;
            }
            used += w;
            spans.extend([key(k), label(l), Span::raw("  ")]);
        }
        spans.extend(help);
        Line::from(spans)
    };
    f.render_widget(Paragraph::new(line), area);
}

/// A solid horizontal bar with 1/8-cell precision.
fn bar(ratio: f64, width: u16, fill: Color, track: Color) -> Vec<Span<'static>> {
    let width = width as usize;
    if width == 0 {
        return Vec::new();
    }
    let eighths = (ratio.clamp(0.0, 1.0) * width as f64 * 8.0).round() as usize;
    let full = (eighths / 8).min(width);
    let rem = if full < width { eighths % 8 } else { 0 };
    let mut spans = vec![Span::styled(
        "█".repeat(full),
        Style::new().fg(fill).bg(track),
    )];
    let mut used = full;
    if rem > 0 {
        spans.push(Span::styled(EIGHTHS[rem], Style::new().fg(fill).bg(track)));
        used += 1;
    }
    spans.push(Span::styled(
        " ".repeat(width - used),
        Style::new().bg(track),
    ));
    spans
}

fn gauge_line(label: &str, pct: f64, bar_w: u16, t: &Theme, color: Color) -> Line<'static> {
    let mut spans = vec![Span::styled(label.to_string(), Style::new().fg(t.muted))];
    spans.extend(bar(pct / 100.0, bar_w, color, t.track));
    spans.push(Span::styled(
        format!(" {pct:>5.1}%"),
        Style::new().fg(t.level(pct)).add_modifier(Modifier::BOLD),
    ));
    Line::from(spans)
}

/// Right-aligned sparkline; when `level` is set each bar is colored by its
/// own utilisation so spikes stand out.
fn spark(f: &mut Frame, area: Rect, hist: &History, max: f64, color: Color, level: Option<&Theme>) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let max = max.max(1e-9);
    let data: Vec<SparklineBar> = hist
        .window(area.width as usize)
        .into_iter()
        .map(|v| match v {
            None => SparklineBar::from(None),
            Some(v) => {
                let scaled = ((v / max).clamp(0.0, 1.0) * 1000.0).round() as u64;
                let bar = SparklineBar::from(Some(scaled));
                match level {
                    Some(t) => bar.style(Style::new().fg(t.level(v / max * 100.0))),
                    None => bar,
                }
            }
        })
        .collect();
    let widget = Sparkline::default()
        .data(data)
        .max(1000)
        .bar_set(symbols::bar::NINE_LEVELS)
        .style(Style::new().fg(color));
    f.render_widget(widget, area);
}

fn draw_cpu(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let s = &app.sample;
    let mut title_right = vec![Span::styled(
        format!(" {:.1}% ", s.cpu.total),
        Style::new()
            .fg(t.level(s.cpu.total))
            .add_modifier(Modifier::BOLD),
    )];
    if let Some(temp) = s.cpu_temp_c {
        title_right.push(Span::styled(
            format!("{temp:.0}°C "),
            Style::new().fg(t.muted),
        ));
    }
    if s.cpu.frequency_mhz > 0 {
        title_right.push(Span::styled(
            format!("{:.2} GHz ", s.cpu.frequency_mhz as f64 / 1000.0),
            Style::new().fg(t.muted),
        ));
    }
    let block = panel(&t, "CPU", t.cpu).title_top(Line::from(title_right).right_aligned());
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width < 4 || inner.height == 0 {
        return;
    }

    let n = s.cpu.per_core.len().max(1);
    let max_rows = (inner.height as usize).saturating_sub(3).max(1);
    let (rows, cols) = core_grid(n, app.info.topology, inner.width as usize, max_rows);
    let graph_h = inner.height.saturating_sub(rows as u16 + 1);
    let [graph, _, cores] = Layout::vertical([
        Constraint::Length(graph_h),
        Constraint::Length(if graph_h > 0 { 1 } else { 0 }),
        Constraint::Fill(1),
    ])
    .areas(inner);
    if graph_h >= 2 {
        spark(f, graph, &app.hist.cpu, 100.0, t.cpu, Some(&t));
    }

    let cell_w = (cores.width as usize / cols).max(8);
    let label_w = if n > 10 { 3 } else { 2 };
    let mut lines = vec![Vec::new(); rows];
    for (i, usage) in s.cpu.per_core.iter().enumerate() {
        let label = match app.info.topology {
            Some(top) => top.label(i),
            None => i.to_string(),
        };
        let label_color = match app.info.topology {
            Some(top) if i < top.efficiency => t.muted,
            _ => t.fg,
        };
        let bar_w = cell_w.saturating_sub(label_w + 7) as u16;
        let line = &mut lines[i % rows];
        line.push(Span::styled(
            format!("{label:>label_w$} "),
            Style::new().fg(label_color),
        ));
        line.extend(bar(usage / 100.0, bar_w, t.level(*usage), t.track));
        line.push(Span::styled(
            format!("{usage:>4.0}% "),
            Style::new().fg(t.level(*usage)),
        ));
    }
    let lines: Vec<Line> = lines.into_iter().map(Line::from).collect();
    f.render_widget(Paragraph::new(lines), cores);
}

/// Chooses the core grid. On hybrid CPUs it prefers a row count that divides
/// both clusters, so efficiency and performance cores never share a column.
fn core_grid(
    n: usize,
    topology: Option<crate::platform::CoreTopology>,
    width: usize,
    max_rows: usize,
) -> (usize, usize) {
    const MIN_CELL: usize = 16;
    if let Some(top) = topology {
        let fits = (1..=max_rows.min(n)).find(|&rows| {
            top.efficiency % rows == 0
                && top.performance % rows == 0
                && width / n.div_ceil(rows) >= MIN_CELL
        });
        if let Some(rows) = fits {
            return (rows, n.div_ceil(rows));
        }
        let rows = top.efficiency.max(top.performance);
        if rows <= max_rows && width / 2 >= MIN_CELL && top.efficiency + top.performance == n {
            return (rows, 2);
        }
    }
    let mut cols = (width / 20).clamp(1, n);
    let mut rows = n.div_ceil(cols);
    if rows > max_rows {
        rows = max_rows;
        cols = n.div_ceil(rows);
    }
    (rows, cols)
}

fn draw_memory(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let m = &app.sample.memory;
    let mut title_right = Vec::new();
    if let Some(p) = m.pressure {
        let color = match p {
            Pressure::Normal => t.ok,
            Pressure::Warn => t.warn,
            Pressure::Critical => t.crit,
        };
        title_right.push(Span::styled(" ● ", Style::new().fg(color)));
        title_right.push(Span::styled(
            format!("pressure {} ", p.label()),
            Style::new().fg(color).add_modifier(Modifier::BOLD),
        ));
    }
    let block = panel(&t, "Memory", t.mem).title_top(Line::from(title_right).right_aligned());
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width < 10 || inner.height == 0 {
        return;
    }

    let amounts = inner.width >= 34;
    let amount_w = if amounts { 12 } else { 0 };
    let bar_w = inner.width.saturating_sub(5 + 8 + amount_w);
    let row = |label: &str, used: u64, total: u64, pct: f64, color: Color| {
        let mut line = gauge_line(label, pct, bar_w, &t, color);
        if amounts {
            line.push_span(Span::styled(
                format!(
                    " {:>5}/{:<5}",
                    fmt::bytes_short(used),
                    fmt::bytes_short(total)
                ),
                Style::new().fg(t.muted),
            ));
        }
        line
    };
    let mut lines = vec![row("RAM  ", m.used, m.total, m.used_pct(), t.mem)];
    if m.swap_total > 0 {
        lines.push(row(
            "Swap ",
            m.swap_used,
            m.swap_total,
            m.swap_pct(),
            t.swap,
        ));
    }
    let b = &m.breakdown;
    let mut groups = Vec::new();
    for (name, value, color) in [
        ("app", b.app, t.mem),
        ("wired", b.wired, t.crit),
        ("compressed", b.compressed, t.swap),
        ("cached", b.cached, t.ok),
    ] {
        if let Some(v) = value {
            groups.push(vec![
                Span::styled("■ ", Style::new().fg(color)),
                Span::styled(format!("{name} "), Style::new().fg(t.muted)),
                Span::styled(format!("{}  ", fmt::bytes_short(v)), Style::new().fg(t.fg)),
            ]);
        }
    }
    groups.push(vec![
        Span::styled("avail ", Style::new().fg(t.muted)),
        Span::styled(fmt::bytes_short(m.available), Style::new().fg(t.ok)),
    ]);
    lines.push(Line::default());
    lines.extend(flow(groups, inner.width));

    let text_h = lines.len() as u16;
    let [text, gap, graph] = Layout::vertical([
        Constraint::Length(text_h.min(inner.height)),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(inner);
    let _ = gap;
    f.render_widget(Paragraph::new(lines), text);
    if graph.height >= 2 {
        spark(f, graph, &app.hist.mem, 100.0, t.mem, None);
    }
}

fn draw_network(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let n = &app.sample.network;
    let mut block = panel(&t, "Network", t.rx);
    if let Some(iface) = n.interfaces.first() {
        let addr = iface.addresses.first().cloned().unwrap_or_default();
        block = block.title_bottom(
            Line::from(Span::styled(
                format!(" {} {addr} ", iface.name),
                Style::new().fg(t.muted),
            ))
            .right_aligned(),
        );
    }
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 2 {
        return;
    }
    let width = inner.width as usize;
    let peak = app
        .hist
        .rx
        .peak(width)
        .max(app.hist.tx.peak(width))
        .max(1024.0);
    let [rx_label, rx_graph, tx_label, tx_graph] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(inner);
    let label = |arrow: &str, rate: f64, total: u64, color: Color| {
        Line::from(vec![
            Span::styled(
                format!("{arrow} "),
                Style::new().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("{:<11}", fmt::rate(rate)), Style::new().fg(t.fg)),
            Span::styled(
                format!(" total {}", fmt::bytes(total)),
                Style::new().fg(t.muted),
            ),
        ])
    };
    f.render_widget(
        Paragraph::new(label("↓", n.rx_rate, n.rx_total, t.rx)),
        rx_label,
    );
    f.render_widget(
        Paragraph::new(label("↑", n.tx_rate, n.tx_total, t.tx)),
        tx_label,
    );
    spark(f, rx_graph, &app.hist.rx, peak, t.rx, None);
    spark(f, tx_graph, &app.hist.tx, peak, t.tx, None);
}

fn draw_disks(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let s = &app.sample;
    let block = panel(&t, "Disks", t.disk);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height == 0 || inner.width < 16 {
        return;
    }
    let io_h = if inner.height >= 4 { 1 } else { 0 };
    let max_disks = inner.height.saturating_sub(io_h) as usize;
    let mount_w = 13usize;
    let bar_w = inner.width.saturating_sub(mount_w as u16 + 1 + 7 + 11);
    let mut lines: Vec<Line> = s
        .disks
        .iter()
        .take(max_disks)
        .map(|d| {
            let label = if d.mount == "/" && !d.name.is_empty() && !d.name.starts_with("/dev") {
                d.name.clone()
            } else {
                d.mount
                    .rsplit('/')
                    .next()
                    .filter(|x| !x.is_empty())
                    .unwrap_or("/")
                    .to_string()
            };
            let pct = d.used_pct();
            let mut spans = vec![Span::styled(
                format!("{:<mount_w$} ", fmt::truncate(&label, mount_w)),
                Style::new().fg(t.fg),
            )];
            spans.extend(bar(pct / 100.0, bar_w, t.level(pct), t.track));
            spans.push(Span::styled(
                format!(" {pct:>4.0}% "),
                Style::new().fg(t.level(pct)),
            ));
            spans.push(Span::styled(
                format!(
                    "{:>4}/{:<5}",
                    fmt::bytes_short(d.used()),
                    fmt::bytes_short(d.total)
                ),
                Style::new().fg(t.muted),
            ));
            Line::from(spans)
        })
        .collect();
    let used_h = lines.len() as u16;
    if io_h > 0 {
        lines.push(Line::from(vec![
            Span::styled("R ", Style::new().fg(t.rx).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("{:<12}", fmt::rate(s.disk_io.read)),
                Style::new().fg(t.fg),
            ),
            Span::styled("W ", Style::new().fg(t.tx).add_modifier(Modifier::BOLD)),
            Span::styled(fmt::rate(s.disk_io.write), Style::new().fg(t.fg)),
        ]));
    }
    let text_h = lines.len() as u16;
    let [text, graph] =
        Layout::vertical([Constraint::Length(text_h), Constraint::Fill(1)]).areas(inner);
    f.render_widget(Paragraph::new(lines), text);
    let _ = used_h;
    if graph.height >= 1 {
        let peak = app.hist.io.peak(graph.width as usize).max(1024.0);
        spark(f, graph, &app.hist.io, peak, t.disk, None);
    }
}

fn draw_system(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let s = &app.sample;
    let block = panel(&t, "System", t.gpu);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height == 0 || inner.width < 16 {
        return;
    }
    let bar_w = inner.width.saturating_sub(5 + 8);
    let mut lines = Vec::new();
    if let Some(gpu) = &s.gpu {
        lines.push(gauge_line("GPU  ", gpu.usage, bar_w, &t, t.gpu));
    }
    if let Some(b) = &s.battery {
        let mut line = gauge_line("Batt ", b.percent, bar_w, &t, t.ok);
        line.spans.pop();
        let pct_color = if b.percent < 15.0 {
            t.crit
        } else if b.percent < 35.0 {
            t.warn
        } else {
            t.ok
        };
        line.push_span(Span::styled(
            format!(" {:>5.1}%", b.percent),
            Style::new().fg(pct_color).add_modifier(Modifier::BOLD),
        ));
        lines.push(line);
        let icon = match b.state.as_str() {
            "charging" => "⚡ charging",
            "full" => "● full",
            "discharging" => "▼ on battery",
            other => other,
        };
        let mut detail = vec![Span::styled(format!("     {icon}"), Style::new().fg(t.fg))];
        let eta = b.time_to_full_secs.or(b.time_to_empty_secs);
        if let Some(eta) = eta {
            detail.push(Span::styled(
                format!(" · {}", fmt::duration(eta)),
                Style::new().fg(t.muted),
            ));
        }
        if let Some(w) = b.power_watts {
            detail.push(Span::styled(
                format!(" · {w:.1} W"),
                Style::new().fg(t.muted),
            ));
        }
        if let Some(h) = b.health_percent {
            detail.push(Span::styled(
                format!(" · health {h:.0}%"),
                Style::new().fg(t.muted),
            ));
        }
        lines.push(Line::from(detail));
    }
    if let (Some(avg), Some(max)) = (s.cpu_temp_c, s.cpu_temp_max_c) {
        let color = if max >= 90.0 {
            t.crit
        } else if max >= 75.0 {
            t.warn
        } else {
            t.ok
        };
        lines.push(Line::from(vec![
            Span::styled("Temp ", Style::new().fg(t.muted)),
            Span::styled(
                format!("{avg:.0}°C"),
                Style::new().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" avg · {max:.0}°C max"), Style::new().fg(t.muted)),
        ]));
    }
    let [one, five, fifteen] = s.load_average;
    lines.push(Line::from(vec![
        Span::styled("Load ", Style::new().fg(t.muted)),
        Span::styled(
            format!("{one:.2} {five:.2} {fifteen:.2}"),
            Style::new().fg(t.level(one / app.info.logical_cores.max(1) as f64 * 100.0)),
        ),
        Span::styled(
            format!(" · {} procs", s.process_count),
            Style::new().fg(t.muted),
        ),
    ]));
    let text_h = (lines.len() as u16).min(inner.height);
    let [text, graph] =
        Layout::vertical([Constraint::Length(text_h), Constraint::Fill(1)]).areas(inner);
    f.render_widget(Paragraph::new(lines), text);
    if s.gpu.is_some() && graph.height >= 1 {
        spark(f, graph, &app.hist.gpu, 100.0, t.gpu, None);
    }
}

fn draw_processes(f: &mut Frame, area: Rect, app: &mut App) {
    let t = app.theme();
    let arrow = if app.sort_desc { "▼" } else { "▲" };
    let mut title_right = vec![Span::styled(
        format!(" sort {} {arrow} ", app.sort.label()),
        Style::new().fg(t.muted),
    )];
    if !app.filter.is_empty() {
        title_right.push(Span::styled(
            format!("filter “{}” ", app.filter),
            Style::new().fg(t.accent).add_modifier(Modifier::BOLD),
        ));
    }
    let title = format!("Processes {}", app.rows.len());
    let block = panel(&t, &title, t.title).title_top(Line::from(title_right).right_aligned());
    let inner = block.inner(area);

    let w = inner.width;
    let show_user = w >= 70;
    let show_io = w >= 90;
    let show_state = w >= 110;
    let show_mem_pct = w >= 60;

    let mut columns: Vec<(&str, Constraint, Option<SortKey>)> = vec![
        ("PID", Constraint::Length(7), Some(SortKey::Pid)),
        ("NAME", Constraint::Fill(1), Some(SortKey::Name)),
    ];
    if show_user {
        columns.push(("USER", Constraint::Length(12), None));
    }
    columns.push(("CPU%", Constraint::Length(7), Some(SortKey::Cpu)));
    columns.push(("MEM", Constraint::Length(8), Some(SortKey::Mem)));
    if show_mem_pct {
        columns.push(("MEM%", Constraint::Length(6), None));
    }
    if show_io {
        columns.push(("READ/s", Constraint::Length(11), Some(SortKey::Io)));
        columns.push(("WRITE/s", Constraint::Length(11), Some(SortKey::Io)));
    }
    if show_state {
        columns.push(("STATE", Constraint::Length(9), None));
    }

    let header = Row::new(columns.iter().map(|(name, _, key)| {
        let active = *key == Some(app.sort);
        let text = if active {
            format!("{name}{arrow}")
        } else {
            name.to_string()
        };
        let style = if active {
            Style::new().fg(t.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(t.muted).add_modifier(Modifier::BOLD)
        };
        Cell::from(text).style(style)
    }));

    let total_mem = app.sample.memory.total.max(1) as f64;
    let procs = &app.sample.processes;
    let rows: Vec<Row> = app
        .rows
        .iter()
        .map(|&i| {
            let p = &procs[i];
            let mem_pct = p.memory as f64 * 100.0 / total_mem;
            // Per-process CPU is "% of one core", so a process pegging a full
            // core reads red regardless of how many cores the machine has.
            let cpu_color = if p.cpu < 1.0 {
                t.muted
            } else {
                t.level(p.cpu.min(100.0))
            };
            let mut cells = vec![
                Cell::from(format!("{:>6}", p.pid)).style(Style::new().fg(t.muted)),
                Cell::from(p.name.clone()).style(Style::new().fg(t.fg)),
            ];
            if show_user {
                cells.push(Cell::from(fmt::truncate(&p.user, 12)).style(Style::new().fg(t.muted)));
            }
            cells.push(Cell::from(format!("{:>6.1}", p.cpu)).style(Style::new().fg(cpu_color)));
            cells.push(
                Cell::from(format!("{:>7}", fmt::bytes_short(p.memory)))
                    .style(Style::new().fg(t.fg)),
            );
            if show_mem_pct {
                cells.push(
                    Cell::from(format!("{mem_pct:>5.1}"))
                        .style(Style::new().fg(if mem_pct >= 10.0 { t.warn } else { t.muted })),
                );
            }
            if show_io {
                for rate in [p.read_rate, p.write_rate] {
                    let style = if rate < 1.0 {
                        Style::new().fg(t.muted)
                    } else {
                        Style::new().fg(t.fg)
                    };
                    let text = if rate < 1.0 {
                        "·".to_string()
                    } else {
                        fmt::rate(rate)
                    };
                    cells.push(Cell::from(format!("{text:>10}")).style(style));
                }
            }
            if show_state {
                cells.push(Cell::from(fmt::truncate(&p.status, 9)).style(Style::new().fg(t.muted)));
            }
            Row::new(cells)
        })
        .collect();

    app.page_size = inner.height.saturating_sub(2).max(1) as usize;
    let table = Table::new(rows, columns.iter().map(|(_, c, _)| *c))
        .header(header.bottom_margin(0))
        .block(block)
        .column_spacing(1)
        .row_highlight_style(
            Style::new()
                .bg(t.selection_bg)
                .fg(t.selection_fg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(Span::styled("▌", Style::new().fg(t.accent)));
    f.render_stateful_widget(table, area, &mut app.table);
}

fn popup(f: &mut Frame, area: Rect, t: &Theme, title: &str, w: u16, h: u16) -> Rect {
    let rect = area.centered(
        Constraint::Length(w.min(area.width)),
        Constraint::Length(h.min(area.height)),
    );
    f.render_widget(Clear, rect);
    let block = panel(t, title, t.accent).border_style(Style::new().fg(t.accent));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    inner
}

fn draw_help(f: &mut Frame, area: Rect, t: &Theme) {
    let rows: [(&str, &str); 14] = [
        ("q / esc", "quit (esc clears an active filter first)"),
        ("space", "pause / resume sampling"),
        ("+ / -", "slower / faster refresh (250 ms – 10 s)"),
        ("↑ ↓  j k", "move selection"),
        ("pgup pgdn", "page through processes"),
        ("g / G", "jump to top / bottom"),
        ("/", "filter by name, user or PID"),
        ("c m i p n", "sort by CPU, memory, I/O, PID, name"),
        ("", "press the same key again to reverse"),
        ("s / r", "cycle sort column / reverse order"),
        ("x", "terminate selected process (SIGTERM)"),
        ("X", "force kill selected process (SIGKILL)"),
        ("t", "cycle color theme"),
        ("? / h", "toggle this help"),
    ];
    let inner = popup(f, area, t, "Keys", 62, rows.len() as u16 + 4);
    let mut lines: Vec<Line> = rows
        .iter()
        .map(|(k, d)| {
            Line::from(vec![
                Span::styled(
                    format!(" {k:<11}"),
                    Style::new().fg(t.accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(d.to_string(), Style::new().fg(t.fg)),
            ])
        })
        .collect();
    lines.push(Line::default());
    lines.push(
        Line::from(Span::styled(
            format!("vigil {} · press any key", env!("CARGO_PKG_VERSION")),
            Style::new().fg(t.muted),
        ))
        .centered(),
    );
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_confirm(f: &mut Frame, area: Rect, t: &Theme, pid: u32, name: &str, force: bool) {
    let title = if force { "Force kill" } else { "Terminate" };
    let inner = popup(f, area, t, title, 52, 6);
    let signal = if force { "SIGKILL" } else { "SIGTERM" };
    let lines = vec![
        Line::from(vec![
            Span::styled("Send ", Style::new().fg(t.fg)),
            Span::styled(signal, Style::new().fg(t.crit).add_modifier(Modifier::BOLD)),
            Span::styled(" to ", Style::new().fg(t.fg)),
            Span::styled(
                fmt::truncate(name, 24),
                Style::new().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" ({pid})?"), Style::new().fg(t.muted)),
        ])
        .centered(),
        Line::default(),
        Line::from(vec![
            Span::styled("y", Style::new().fg(t.crit).add_modifier(Modifier::BOLD)),
            Span::styled(" confirm   ", Style::new().fg(t.muted)),
            Span::styled(
                "any other key",
                Style::new().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" cancel", Style::new().fg(t.muted)),
        ])
        .centered(),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::SortKey;
    use crate::collector::Collector;
    use crate::theme::ThemeName;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::time::Duration;

    fn render(width: u16, height: u16) -> String {
        let collector = Collector::new().expect("collector");
        let mut app = App::new(
            collector,
            ThemeName::Aurora,
            Duration::from_secs(1),
            SortKey::Cpu,
        );
        app.tick();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        buf.content()
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<_>>()
            .join("")
    }

    #[test]
    fn full_dashboard_renders_every_panel() {
        let screen = render(160, 48);
        for panel in [
            "vigil",
            "CPU",
            "Memory",
            "Network",
            "Disks",
            "System",
            "Processes",
        ] {
            assert!(screen.contains(panel), "missing panel {panel}");
        }
    }

    #[test]
    fn tiny_terminals_do_not_panic() {
        for (w, h) in [(1, 1), (20, 5), (59, 13), (80, 24), (100, 29), (300, 100)] {
            let screen = render(w, h);
            assert!(!screen.is_empty());
        }
    }

    #[test]
    fn core_grid_keeps_clusters_apart() {
        let m5 = crate::platform::CoreTopology {
            performance: 4,
            efficiency: 6,
        };
        // Two rows: E0-E5 fill three columns, P0-P3 the other two.
        assert_eq!(core_grid(10, Some(m5), 97, 12), (2, 5));
        // Too narrow for five columns: one column per cluster.
        assert_eq!(core_grid(10, Some(m5), 50, 12), (6, 2));
        // Not enough rows for that either: generic grid.
        assert_eq!(core_grid(10, Some(m5), 50, 4), (4, 3));
        assert_eq!(core_grid(16, None, 80, 2), (2, 8));
    }

    #[test]
    fn bar_has_exact_width() {
        for w in [0u16, 1, 7, 30] {
            for r in [0.0, 0.33, 0.5, 0.999, 1.0, 2.0] {
                let len: usize = bar(r, w, Color::Red, Color::Black)
                    .iter()
                    .map(|s| s.content.chars().count())
                    .sum();
                assert_eq!(len, w as usize, "ratio {r} width {w}");
            }
        }
    }
}
