//! Terminal lifecycle helpers.
use anyhow::{Context, Result};
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
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
static MOUSE_ACTIVE: AtomicBool = AtomicBool::new(false);
static INLINE_BOTTOM: AtomicU16 = AtomicU16::new(0);
static HOOKS_INSTALLED: OnceLock<()> = OnceLock::new();

/// Display surface mode: Fullscreen (alternate screen, default) or Inline (native scrollback).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ScreenMode {
    #[default]
    Fullscreen,
    Inline,
}

impl ScreenMode {
    /// Canonical lowercase name for config serialization and diagnostics.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fullscreen => "fullscreen",
            Self::Inline => "inline",
        }
    }

    /// Parse a display surface mode name or alias (case-insensitive).
    pub fn from_name(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "fullscreen" | "full" | "alt" => Some(Self::Fullscreen),
            "inline" | "native" | "compact" => Some(Self::Inline),
            _ => None,
        }
    }
}

pub const INLINE_HEIGHT: u16 = 16;

/// Terminal entry configuration controlling display mode, mouse capture, and inline height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSettings {
    pub mode: ScreenMode,
    pub mouse: bool,
    pub inline_height: u16,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            mode: ScreenMode::Fullscreen,
            mouse: false,
            inline_height: INLINE_HEIGHT,
        }
    }
}

pub struct TerminalGuard {
    restored: bool,
    mode: ScreenMode,
    mouse_active: bool,
    inline_height: u16,
}
impl TerminalGuard {
    pub fn enter(mode: ScreenMode) -> Result<Self> {
        Self::enter_with(TerminalSettings {
            mode,
            mouse: false,
            inline_height: INLINE_HEIGHT,
        })
    }

    pub fn enter_with(settings: TerminalSettings) -> Result<Self> {
        let inline_height = settings.inline_height.clamp(8, 40);
        let mouse_active = settings.mode == ScreenMode::Fullscreen && settings.mouse;
        enable_raw_mode().context("could not enable terminal raw mode")?;
        FULLSCREEN.store(settings.mode == ScreenMode::Fullscreen, Ordering::SeqCst);
        MOUSE_ACTIVE.store(mouse_active, Ordering::SeqCst);
        INLINE_BOTTOM.store(u16::MAX, Ordering::SeqCst);
        TERMINAL_ACTIVE.store(true, Ordering::SeqCst);
        let mut stdout = io::stdout();
        let entered = write_enter(&mut stdout, settings.mode, mouse_active);
        if let Err(error) = entered {
            restore_terminal_now();
            return Err(error).context("could not initialise terminal display");
        }
        Ok(Self {
            restored: false,
            mode: settings.mode,
            mouse_active,
            inline_height,
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
        let requested = self.inline_height.min(height);
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
        let mouse_active = MOUSE_ACTIVE.swap(false, Ordering::SeqCst) || self.mouse_active;
        self.mouse_active = false;
        let mut stdout = io::stdout();
        let screen = write_restore(&mut stdout, self.mode, row, mouse_active);
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

fn write_enter(output: &mut impl Write, mode: ScreenMode, mouse_active: bool) -> io::Result<()> {
    match (mode, mouse_active) {
        (ScreenMode::Fullscreen, true) => execute!(
            output,
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture,
            Hide
        ),
        (ScreenMode::Fullscreen, false) => {
            execute!(output, EnterAlternateScreen, EnableBracketedPaste, Hide)
        }
        (ScreenMode::Inline, _) => execute!(output, EnableBracketedPaste, Hide),
    }
}

fn write_restore(
    output: &mut impl Write,
    mode: ScreenMode,
    row: u16,
    mouse_active: bool,
) -> io::Result<()> {
    match mode {
        ScreenMode::Fullscreen => {
            if mouse_active {
                // Disable mouse tracking before leaving the alternate screen so
                // neither tmux nor the user's shell inherits mouse capture.
                execute!(
                    output,
                    DisableMouseCapture,
                    DisableBracketedPaste,
                    Show,
                    LeaveAlternateScreen
                )
            } else {
                execute!(output, DisableBracketedPaste, Show, LeaveAlternateScreen)
            }
        }
        ScreenMode::Inline => {
            if row != u16::MAX {
                let row = crossterm::terminal::size()
                    .map_or(row, |(_, height)| row.min(height.saturating_sub(1)));
                execute!(output, MoveTo(0, row))?;
            }
            execute!(output, DisableBracketedPaste, Show)?;
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
    let mouse_active = MOUSE_ACTIVE.swap(false, Ordering::SeqCst);
    let row = INLINE_BOTTOM.load(Ordering::SeqCst);
    let _ = write_restore(&mut io::stdout(), mode, row, mouse_active);
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
    fn screen_mode_defaults_to_fullscreen_and_parses_aliases() {
        assert_eq!(ScreenMode::default(), ScreenMode::Fullscreen);
        assert_eq!(ScreenMode::Fullscreen.as_str(), "fullscreen");
        assert_eq!(ScreenMode::Inline.as_str(), "inline");

        assert_eq!(
            ScreenMode::from_name("fullscreen"),
            Some(ScreenMode::Fullscreen)
        );
        assert_eq!(
            ScreenMode::from_name(" FULL "),
            Some(ScreenMode::Fullscreen)
        );
        assert_eq!(ScreenMode::from_name("alt"), Some(ScreenMode::Fullscreen));
        assert_eq!(ScreenMode::from_name("inline"), Some(ScreenMode::Inline));
        assert_eq!(ScreenMode::from_name("Native"), Some(ScreenMode::Inline));
        assert_eq!(ScreenMode::from_name("COMPACT"), Some(ScreenMode::Inline));
        assert_eq!(ScreenMode::from_name("unknown"), None);
        assert_eq!(ScreenMode::from_name(""), None);
    }

    #[test]
    fn terminal_settings_default_matches_contract() {
        let settings = TerminalSettings::default();
        assert_eq!(settings.mode, ScreenMode::Fullscreen);
        assert!(!settings.mouse);
        assert_eq!(settings.inline_height, INLINE_HEIGHT);
    }

    #[test]
    fn inline_restoration_preserves_the_main_screen() {
        for mouse_flag in [false, true] {
            let mut output = Vec::new();
            write_restore(&mut output, ScreenMode::Inline, 12, mouse_flag).expect("restore output");
            let output = String::from_utf8(output).expect("UTF-8 escapes");
            assert!(output.contains("\x1b[?2004l"));
            assert!(output.contains("\x1b[?25h"));
            assert!(output.ends_with("\r\n"));
            assert!(!output.contains("1049"));
            assert!(!output.contains("\x1b[2J"));
            assert!(!output.contains("\x1b[3J"));
            for mouse_seq in ["1000", "1002", "1003", "1006", "1015"] {
                assert!(
                    !output.contains(mouse_seq),
                    "inline restore must not emit mouse escape {mouse_seq}: {output:?}"
                );
            }
        }
    }

    #[test]
    fn fullscreen_restoration_leaves_the_alternate_screen() {
        let mut output = Vec::new();
        write_restore(&mut output, ScreenMode::Fullscreen, 0, false).expect("restore output");
        let text = String::from_utf8(output).expect("UTF-8 escapes");
        assert!(text.contains("\x1b[?1049l"));
        assert!(text.contains("\x1b[?2004l"));
        assert!(text.contains("\x1b[?25h"));
        for mouse_seq in ["1000", "1002", "1003", "1006", "1015"] {
            assert!(!text.contains(mouse_seq));
        }
    }

    #[test]
    fn fullscreen_mouse_enable_and_disable_ordering_is_safe() {
        let mut enter_mouse = Vec::new();
        write_enter(&mut enter_mouse, ScreenMode::Fullscreen, true).expect("enter with mouse");
        let enter_mouse = String::from_utf8(enter_mouse).expect("UTF-8 escapes");
        assert!(enter_mouse.contains("\x1b[?1049h"));
        assert!(enter_mouse.contains("\x1b[?1000h"));
        assert!(enter_mouse.contains("\x1b[?1006h"));

        let mut enter_no_mouse = Vec::new();
        write_enter(&mut enter_no_mouse, ScreenMode::Fullscreen, false)
            .expect("enter without mouse");
        let enter_no_mouse = String::from_utf8(enter_no_mouse).expect("UTF-8 escapes");
        assert!(enter_no_mouse.contains("\x1b[?1049h"));
        assert!(!enter_no_mouse.contains("1000"));
        assert!(!enter_no_mouse.contains("1006"));

        let mut enter_inline = Vec::new();
        write_enter(&mut enter_inline, ScreenMode::Inline, true).expect("enter inline");
        let enter_inline = String::from_utf8(enter_inline).expect("UTF-8 escapes");
        assert!(!enter_inline.contains("1049"));
        assert!(!enter_inline.contains("1000"));
        assert!(!enter_inline.contains("1006"));

        let mut restore_mouse = Vec::new();
        write_restore(&mut restore_mouse, ScreenMode::Fullscreen, 0, true)
            .expect("restore with mouse");
        let restore_mouse = String::from_utf8(restore_mouse).expect("UTF-8 escapes");
        let leave_alt = restore_mouse
            .find("\x1b[?1049l")
            .expect("LeaveAlternateScreen");
        for disable_seq in ["\x1b[?1000l", "\x1b[?1002l", "\x1b[?1003l", "\x1b[?1006l"] {
            let pos = restore_mouse
                .find(disable_seq)
                .unwrap_or_else(|| panic!("missing {disable_seq} in {restore_mouse:?}"));
            assert!(
                pos < leave_alt,
                "{disable_seq} ({pos}) must precede LeaveAlternateScreen ({leave_alt})"
            );
        }
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
