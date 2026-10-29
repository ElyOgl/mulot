# Upstream analysis: terminal-system-monitor

Analysis of [krishnapschauhan/terminal-system-monitor](https://github.com/krishnapschauhan/terminal-system-monitor) at commit `ffefb79`, done with [rea](https://github.com/morluto/rea) (static JavaScript application graph) plus controlled checks on an Apple M5 MacBook Air running macOS 26.5. This is the groundwork mulot was designed from.

## What it is

| | |
|---|---|
| Source | one ES module, `index.js`, 156 lines (858 AST nodes) |
| Dependencies | `blessed` 0.1.81, `blessed-contrib` 4.11, `chalk` 5.6 (60 packages in the lockfile) |
| Entry | `npm start` → `node index.js` |
| Configuration | none: rea found no `process.argv`, `process.env` or file reads |

### Runtime structure (from the rea graph)

- **1 timer.** `setInterval(update, 1000)` is scheduled once and never cancelled, so the process only ends through `process.exit` on `q` / `esc` / `C-c`.
- **1 child process per tick.** `exec("wmic logicaldisk get size,freespace,caption")` runs inside `update()`, so a new process is spawned every second. Its callback parses stdout and calls `screen.render()` a second time.
- **Boundaries.** All numeric parsing goes through `Number()` coercions of `toFixed()` strings, and the strings drive the color thresholds.
- **Widgets.** A 12×12 `blessed-contrib` grid holds a line chart (30-sample CPU average), a per-core table, and memory, disk and info boxes.

## Findings

| # | Finding | Evidence | mulot |
|---|---|---|---|
| 1 | **Memory is wrong on macOS.** `used = totalmem − freemem` counts file cache and compressed pages as used. | On the test Mac it reported **98.6 % used** while `vm_stat` showed ~10 GB reclaimable. The real state was kernel pressure *warn* with 9.6 / 10 GB swap in use, and the dashboard can't show either. | Uses Activity Monitor's formula (app + wired + compressed), shows kernel pressure, swap and a breakdown |
| 2 | **The disk panel only works on Windows**, and only where `wmic` still exists. Microsoft deprecated it and removes it in recent Windows 11 builds. | `command -v wmic` → not found, so the panel permanently says "Disk data unavailable" on macOS/Linux | Reads volumes in-process on every OS and de-duplicates APFS containers |
| 3 | **Spawns a process every second** (`exec` inside the 1 s timer). | rea child-process relation inside `update`'s call-flow | No subprocesses at all |
| 4 | **Blank first second.** Nothing renders until the first `setInterval` fire. | timer is the only caller of `update()` | Primes counters at startup and draws immediately |
| 5 | **Possible `NaN%`.** `calcCPU` divides by `totalDiff` with no zero guard (two `os.cpus()` reads in the same clock tick). | `calcCPU` data-flow: `idleDiff / totalDiff` → `toFixed` | Usage comes from sysinfo deltas, and non-finite history values are clamped |
| 6 | **Hybrid CPUs are flattened.** Apple Silicon E- and P-cores show up as `CPU 0…9` with no distinction. | `hw.perflevel0/1` = 4 P + 6 E on the M5 | Cores labelled `E0…E5 / P0…P3`. The E-first order was verified by loading the P-cores and watching which bars rose |
| 7 | **No processes, network, GPU, temperatures or battery.** | not in the graph | All present (GPU and sensors need no root) |
| 8 | **No controls** besides quit, and a hard-coded window title ("Krishna System Monitor"). | `screen.key(["escape","q","C-c"])` is the only listener | Pause, speed, sort, filter, kill, themes, help, CLI flags, JSON output |
| 9 | **Chalk escape codes inside blessed widgets.** Table column widths are computed on strings that contain ANSI codes, so columns can misalign. | `colorPercent()` output passed to `contrib.table` | Styles are attributes on cells, never inline escape codes |

## What mulot keeps

The concept: one screen, color-coded thresholds (green / yellow / red), a CPU history graph, per-core usage, memory, disks and system info, with `q`/`esc` to quit.
