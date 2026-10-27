use crate::{Error, Result};
use crossterm::event::KeyboardEnhancementFlags;
use crossterm::{cursor, event, execute, terminal};
use std::io::{IsTerminal, Write, stdout};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

#[derive(Default)]
pub(super) struct TerminalModes {
    raw: bool,
    alternate: bool,
    paste: bool,
    focus: bool,
    cursor_hidden: bool,
    keyboard: Option<KeyboardMode>,
    mouse: bool,
}

#[derive(Clone, Copy)]
pub(super) enum KeyboardMode {
    Kitty,
    Tmux,
}

impl KeyboardMode {
    fn enable(self) -> std::io::Result<()> {
        match self {
            Self::Kitty => execute!(
                stdout(),
                event::PushKeyboardEnhancementFlags(
                    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                )
            ),
            Self::Tmux => xterm_keyboard_mode(true),
        }
    }

    fn restore(self) -> std::io::Result<()> {
        match self {
            Self::Kitty => execute!(stdout(), event::PopKeyboardEnhancementFlags),
            Self::Tmux => xterm_keyboard_mode(false),
        }
    }
}

fn xterm_keyboard_mode(enabled: bool) -> std::io::Result<()> {
    let mut output = stdout();
    output.write_all(if enabled {
        b"\x1b[>4;2m"
    } else {
        b"\x1b[>4;0m"
    })?;
    output.flush()
}

pub(super) struct TerminalSession {
    modes: Arc<std::sync::Mutex<TerminalModes>>,
    keyboard: KeyboardMode,
    #[cfg(unix)]
    original: rustix::termios::Termios,
}

impl TerminalSession {
    pub(super) fn enter(
        mouse: bool,
        keyboard: KeyboardMode,
        modes: Arc<std::sync::Mutex<TerminalModes>>,
    ) -> Result<Self> {
        if !std::io::stdin().is_terminal() || !stdout().is_terminal() {
            return Err(Error::Compatibility(
                "Interactive reading requires a terminal. Use --dump for text output.".into(),
            ));
        }
        let session = Self {
            modes,
            keyboard,
            #[cfg(unix)]
            original: rustix::termios::tcgetattr(std::io::stdin()).map_err(std::io::Error::from)?,
        };
        session.enable(mouse)?;
        Ok(session)
    }

    pub(super) fn suspend<T>(&self, mouse: bool, operation: impl FnOnce() -> T) -> Result<T> {
        restore_terminal(&self.modes);
        let result = operation();
        self.restore_attributes()?;
        self.enable(mouse)?;
        Ok(result)
    }

    fn restore_attributes(&self) -> Result<()> {
        #[cfg(unix)]
        rustix::termios::tcsetattr(
            std::io::stdin(),
            rustix::termios::OptionalActions::Now,
            &self.original,
        )
        .map_err(std::io::Error::from)?;
        Ok(())
    }

    fn enable(&self, mouse: bool) -> Result<()> {
        terminal::enable_raw_mode()?;
        self.modes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .raw = true;
        execute!(stdout(), terminal::EnterAlternateScreen)?;
        self.modes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .alternate = true;
        execute!(stdout(), event::EnableBracketedPaste)?;
        self.modes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .paste = true;
        execute!(stdout(), event::EnableFocusChange)?;
        self.modes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .focus = true;
        execute!(stdout(), cursor::Hide)?;
        self.modes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .cursor_hidden = true;
        self.keyboard.enable()?;
        self.modes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .keyboard = Some(self.keyboard);
        if mouse {
            execute!(stdout(), event::EnableMouseCapture)?;
            self.modes
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .mouse = true;
        }
        Ok(())
    }
}

// #NOTE: Panic cleanup and normal cleanup share ownership so they cannot pop the terminal's keyboard-mode stack twice.
pub(super) fn restore_terminal(modes: &std::sync::Mutex<TerminalModes>) {
    let modes = std::mem::take(&mut *modes.lock().unwrap_or_else(|error| error.into_inner()));
    if let Some(keyboard) = modes.keyboard {
        let _ = keyboard.restore();
    }
    if modes.focus {
        let _ = execute!(stdout(), event::DisableFocusChange);
    }
    if modes.paste {
        let _ = execute!(stdout(), event::DisableBracketedPaste);
    }
    if modes.mouse {
        let _ = execute!(stdout(), event::DisableMouseCapture);
    }
    if modes.cursor_hidden {
        let _ = execute!(stdout(), cursor::Show);
    }
    if modes.alternate {
        let _ = execute!(stdout(), terminal::LeaveAlternateScreen);
    }
    if modes.raw {
        let _ = terminal::disable_raw_mode();
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        restore_terminal(&self.modes);
        let _ = self.restore_attributes();
    }
}

pub(super) struct Signals {
    pub(super) requested: Arc<AtomicBool>,
    registrations: Vec<signal_hook::SigId>,
}

impl Signals {
    pub(super) fn register() -> Result<Self> {
        let requested = Arc::new(AtomicBool::new(false));
        let mut registrations = Vec::new();
        #[cfg(unix)]
        for signal in [
            signal_hook::consts::SIGTERM,
            signal_hook::consts::SIGHUP,
            signal_hook::consts::SIGINT,
        ] {
            registrations.push(signal_hook::flag::register(signal, requested.clone())?);
        }
        Ok(Self {
            requested,
            registrations,
        })
    }
}

impl Drop for Signals {
    fn drop(&mut self) {
        for registration in &self.registrations {
            signal_hook::low_level::unregister(*registration);
        }
    }
}
