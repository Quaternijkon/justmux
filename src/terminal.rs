//! Terminal lifecycle helpers.
use anyhow::{Context, Result};
use crossterm::{
    cursor::{Hide, Show},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use std::io::{self, Stdout, Write};
use std::process::Command;
use std::sync::{
    OnceLock,
    atomic::{AtomicBool, Ordering},
};

static TERMINAL_ACTIVE: AtomicBool = AtomicBool::new(false);
static HOOKS_INSTALLED: OnceLock<()> = OnceLock::new();

pub struct TerminalGuard {
    restored: bool,
}
impl TerminalGuard {
    pub fn enter() -> Result<Self> {
        enable_raw_mode().context("could not enable terminal raw mode")?;
        TERMINAL_ACTIVE.store(true, Ordering::SeqCst);
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen, Hide) {
            let _ = disable_raw_mode();
            TERMINAL_ACTIVE.store(false, Ordering::SeqCst);
            return Err(error).context("could not enter the alternate screen");
        }
        Ok(Self { restored: false })
    }
    pub fn restore(&mut self) -> Result<()> {
        if self.restored {
            return Ok(());
        }
        self.restored = true;
        TERMINAL_ACTIVE.store(false, Ordering::SeqCst);
        let raw = disable_raw_mode();
        let mut stdout = io::stdout();
        let screen = execute!(stdout, Show, LeaveAlternateScreen);
        raw.context("could not disable terminal raw mode")?;
        screen.context("could not leave the alternate screen")
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.restore();
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
    let mut stdout: Stdout = io::stdout();
    let _ = execute!(stdout, Show, LeaveAlternateScreen);
    let _ = stdout.flush();
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
    fn emergency_restore_is_idempotent() {
        restore_terminal_now();
        restore_terminal_now();
    }
}
