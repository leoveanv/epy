use super::images::ImageSupport;
use ratatui::layout::Size;

pub(in crate::terminal) struct ImageSupportDetector {
    in_tmux: bool,
    #[cfg(unix)]
    tmux: Option<super::tmux_graphics::TmuxGraphics>,
}

impl ImageSupportDetector {
    pub fn new() -> Self {
        let in_tmux = std::env::var_os("TMUX").is_some_and(|value| !value.is_empty());
        #[cfg(unix)]
        let tmux = in_tmux
            .then(super::tmux_graphics::TmuxGraphics::acquire)
            .flatten();
        Self {
            in_tmux,
            #[cfg(unix)]
            tmux,
        }
    }

    pub fn detect(&self) -> ImageSupport {
        let window = crossterm::terminal::window_size().ok();
        let cell_width = window
            .as_ref()
            .filter(|size| size.columns > 0 && size.width > 0)
            .map_or(8, |size| (size.width / size.columns).max(1));
        let cell_height = window
            .as_ref()
            .filter(|size| size.rows > 0 && size.height > 0)
            .map_or(16, |size| (size.height / size.rows).max(1));
        let mut cell_size = Size::new(cell_width, cell_height);
        let mut tmux_kitty = false;
        #[cfg(unix)]
        if let Some(tmux) = &self.tmux {
            cell_size = tmux.cell_size().unwrap_or(cell_size);
            tmux_kitty = true;
        }
        let terminal = std::env::var("TERM").unwrap_or_default();
        let program = std::env::var("TERM_PROGRAM").unwrap_or_default();
        let kitty = tmux_kitty
            || (!self.in_tmux && (kitty_terminal(&terminal) || kitty_terminal(&program)));
        ImageSupport {
            cell_size,
            kitty,
            in_tmux: tmux_kitty,
        }
    }
}

fn kitty_terminal(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "kitty" | "xterm-kitty" | "ghostty" | "xterm-ghostty"
    )
}
