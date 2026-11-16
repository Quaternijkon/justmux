//! Terminal lifecycle helpers.
use anyhow::{Context, Result};
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    execute,
    terminal::{
        Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};
use ratatui::layout::Rect;
use std::io::{self, Write};
use std::process::Command;
use std::sync::{
    OnceLock,
    atomic::{AtomicBool, AtomicU16, Ordering},
};

static TERMINAL_ACTIVE: AtomicBool = AtomicBool::new(false);
static FULLSCREEN: AtomicBool = AtomicBool::new(false);
static INLINE_BOTTOM: AtomicU16 = AtomicU16::new(0);
static HOOKS_INSTALLED: OnceLock<()> = OnceLock::new();

/// The main screen preserves the terminal's own scrollback. Fullscreen is opt-in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ScreenMode {
    #[default]
    Inline,
    Fullscreen,
}

pub const INLINE_HEIGHT: u16 = 16;

pub struct TerminalGuard {
    restored: bool,
    mode: ScreenMode,
}
impl TerminalGuard {
    pub fn enter(mode: ScreenMode) -> Result<Self> {
        enable_raw_mode().context("could not enable terminal raw mode")?;
        FULLSCREEN.store(mode == ScreenMode::Fullscreen, Ordering::SeqCst);
        INLINE_BOTTOM.store(u16::MAX, Ordering::SeqCst);
        TERMINAL_ACTIVE.store(true, Ordering::SeqCst);
        let mut stdout = io::stdout();
        let entered = match mode {
            ScreenMode::Fullscreen => execute!(stdout, EnterAlternateScreen, Hide),
            ScreenMode::Inline => execute!(stdout, Hide),
        };
        if let Err(error) = entered {
            restore_terminal_now();
            return Err(error).context("could not initialise terminal display");
        }
        Ok(Self {
            restored: false,
            mode,
        })
    }

    /// Reserve a compact region below the existing prompt without clearing the
    /// main screen. Fixed viewports avoid Ratatui's whole-screen clear on a
    /// horizontal shrink; this method explicitly reanchors on a resize instead.
    pub fn inline_area(&self, previous: Option<Rect>) -> Result<Rect> {
        let (width, height) =
            crossterm::terminal::size().context("could not read terminal dimensions")?;
        let mut stdout = io::stdout();
        if let Some(previous) = previous {
            execute!(stdout, MoveTo(0, previous.y.min(height.saturating_sub(1))))?;
        }
        let (_, row) = crossterm::cursor::position()
            .context("could not read cursor position; try jm --fullscreen if the terminal does not support cursor reports")?;
        INLINE_BOTTOM.store(row, Ordering::SeqCst);
        let requested = INLINE_HEIGHT.min(height);
        let area = reserved_inline_area(width, height, row, requested);
        // Newlines scroll old content into the terminal's native history when
        // there is insufficient space below the cursor. They never erase it.
        for _ in 1..requested {
            stdout.write_all(b"\r\n")?;
        }
        execute!(stdout, MoveTo(0, area.y), Clear(ClearType::FromCursorDown))?;
        INLINE_BOTTOM.store(area.bottom().saturating_sub(1), Ordering::SeqCst);
        Ok(area)
    }

    pub fn restore(&mut self) -> Result<()> {
        if self.restored {
            return Ok(());
        }
        self.restored = true;
        if !TERMINAL_ACTIVE.swap(false, Ordering::SeqCst) {
            return Ok(());
        }
        let raw = disable_raw_mode();
        let row = INLINE_BOTTOM.load(Ordering::SeqCst);
        let mut stdout = io::stdout();
        let screen = write_restore(&mut stdout, self.mode, row);
        raw.context("could not disable terminal raw mode")?;
        screen.context("could not restore terminal display")
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

fn reserved_inline_area(width: u16, height: u16, row: u16, requested: u16) -> Rect {
    let requested = requested.min(height);
    Rect::new(
        0,
        row.min(height.saturating_sub(requested)),
        width,
        requested,
    )
}

fn write_restore(output: &mut impl Write, mode: ScreenMode, row: u16) -> io::Result<()> {
    match mode {
        ScreenMode::Fullscreen => execute!(output, Show, LeaveAlternateScreen),
        ScreenMode::Inline => {
            if row != u16::MAX {
                let row = crossterm::terminal::size()
                    .map_or(row, |(_, height)| row.min(height.saturating_sub(1)));
                execute!(output, MoveTo(0, row))?;
            }
            execute!(output, Show)?;
            output.write_all(b"\r\n")?;
            output.flush()
        }
    }
}
#[cfg(unix)]
pub fn exec_tmux(mut guard: TerminalGuard, mut command: Command) -> Result<()> {
    guard.restore()?;
    use std::os::unix::process::CommandExt;
    Err(command.exec()).context("could not exec tmux client")
}
#[cfg(not(unix))]
pub fn exec_tmux(mut guard: TerminalGuard, mut command: Command) -> Result<()> {
    guard.restore()?;
    let status = command.status().context("could not run tmux client")?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("tmux client exited with {status}")
    }
}
pub fn restore_terminal_now() {
    if !TERMINAL_ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    let _ = disable_raw_mode();
    let mode = if FULLSCREEN.load(Ordering::SeqCst) {
        ScreenMode::Fullscreen
    } else {
        ScreenMode::Inline
    };
    let row = INLINE_BOTTOM.load(Ordering::SeqCst);
    let _ = write_restore(&mut io::stdout(), mode, row);
}
/// Install a panic hook that restores terminal state before printing a panic.
/// The hook is process-wide and intentionally idempotent.
pub fn install_restore_hooks() -> Result<()> {
    if HOOKS_INSTALLED.get().is_some() {
        return Ok(());
    }
    // Signal handlers only wake this helper; terminal IO runs on a normal
    // thread, never inside an async-signal-unsafe handler.
    #[cfg(unix)]
    {
        use signal_hook::{
            consts::{SIGHUP, SIGINT, SIGQUIT, SIGTERM},
            iterator::Signals,
        };
        let mut signals = Signals::new([SIGHUP, SIGINT, SIGQUIT, SIGTERM])
            .context("could not install terminal signal handlers")?;
        std::thread::Builder::new()
            .name("jm-terminal-restore".into())
            .spawn(move || {
                if let Some(signal) = signals.forever().next() {
                    restore_terminal_now();
                    std::process::exit(128 + signal);
                }
            })
            .context("could not start terminal signal handling")?;
    }
    std::panic::set_hook(Box::new(|panic| {
        restore_terminal_now();
        eprintln!("jm: unrecoverable error: {panic}");
    }));
    let _ = HOOKS_INSTALLED.set(());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inline_restoration_preserves_the_main_screen() {
        let mut output = Vec::new();
        write_restore(&mut output, ScreenMode::Inline, 12).expect("restore output");
        let output = String::from_utf8(output).expect("UTF-8 escapes");
        assert!(output.contains("\x1b[?25h"));
        assert!(output.ends_with("\r\n"));
        assert!(!output.contains("1049"));
        assert!(!output.contains("\x1b[2J"));
        assert!(!output.contains("\x1b[3J"));
    }

    #[test]
    fn fullscreen_restoration_leaves_the_alternate_screen() {
        let mut output = Vec::new();
        write_restore(&mut output, ScreenMode::Fullscreen, 0).expect("restore output");
        assert!(output.windows(8).any(|s| s == b"\x1b[?1049l"));
    }

    #[test]
    fn inline_area_reserves_space_and_clamps_to_small_terminals() {
        assert_eq!(reserved_inline_area(80, 30, 2, 16), Rect::new(0, 2, 80, 16));
        assert_eq!(
            reserved_inline_area(80, 30, 28, 16),
            Rect::new(0, 14, 80, 16)
        );
        assert_eq!(reserved_inline_area(20, 8, 28, 16), Rect::new(0, 0, 20, 8));
    }

    #[test]
    fn emergency_restore_is_idempotent() {
        restore_terminal_now();
        restore_terminal_now();
    }
}
