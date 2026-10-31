use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Error, Result};

#[derive(Clone, Debug, Default)]
pub struct PathEnvironment {
    pub home: Option<PathBuf>,
    pub config_home: Option<PathBuf>,
    pub app_data: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppPaths {
    pub directory: PathBuf,
    pub configuration: PathBuf,
    pub database: PathBuf,
}

impl AppPaths {
    pub fn discover(environment: &PathEnvironment) -> Result<Self> {
        let root = environment
            .config_home
            .as_deref()
            .filter(|path| !path.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .or_else(|| {
                environment
                    .home
                    .as_deref()
                    .filter(|path| !path.as_os_str().is_empty())
                    .map(|path| path.join(".config"))
            })
            .or_else(|| environment.app_data.clone())
            .ok_or_else(|| {
                Error::Compatibility(
                    "No configuration home is available. Set XDG_CONFIG_HOME, HOME, or APPDATA."
                        .into(),
                )
            })?;
        if !root.is_absolute() {
            return Err(Error::Compatibility(
                "The configuration home must be an absolute directory.".into(),
            ));
        }
        let directory = root.join("ers");
        Ok(Self {
            configuration: directory.join("configuration.json"),
            database: directory.join("states.db"),
            directory,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub default_viewer: String,
    pub inline_images: bool,
    pub dictionary_client: String,
    pub show_progress_indicator: bool,
    pub page_scroll_animation: bool,
    pub mouse_support: bool,
    pub start_with_double_spread: bool,
    pub annotation_notes_width: i64,
    pub annotation_scrolloff: i64,
    pub annotation_color_fg: i64,
    pub annotation_color_bg: i64,
    pub annotation_ascii_connectors: bool,
    pub default_color_fg: i64,
    pub default_color_bg: i64,
    pub dark_color_fg: i64,
    pub dark_color_bg: i64,
    pub light_color_fg: i64,
    pub light_color_bg: i64,
    pub between_chapters: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_viewer: "auto".into(),
            inline_images: true,
            dictionary_client: "auto".into(),
            show_progress_indicator: true,
            page_scroll_animation: true,
            mouse_support: false,
            start_with_double_spread: false,
            annotation_notes_width: 36,
            annotation_scrolloff: 0,
            annotation_color_fg: 223,
            annotation_color_bg: 52,
            annotation_ascii_connectors: false,
            default_color_fg: -1,
            default_color_bg: -1,
            dark_color_fg: 252,
            dark_color_bg: 235,
            light_color_fg: 238,
            light_color_bg: 253,
            between_chapters: false,
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum InputKey {
    Character(char),
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    Tab,
    Enter,
    Escape,
    ControlI,
    Backspace,
    Delete,
    BackTab,
}

impl std::fmt::Display for InputKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Character(' ') => formatter.write_str("Space"),
            Self::Character(character @ '\0'..='\u{1f}') => {
                write!(formatter, "Ctrl+{}", (*character as u8 + b'@') as char)
            }
            Self::Character('\u{7f}') => formatter.write_str("Backspace"),
            Self::Character(character) if character.is_control() => {
                write!(formatter, "{}", character.escape_unicode())
            }
            Self::Character(character) => write!(formatter, "{character}"),
            Self::ControlI => formatter.write_str("Ctrl+I"),
            Self::BackTab => formatter.write_str("Shift+Tab"),
            key => write!(formatter, "{key:?}"),
        }
    }
}

impl InputKey {
    pub const fn from_character(character: char) -> Self {
        match character {
            '\t' => Self::Tab,
            '\n' | '\r' => Self::Enter,
            '\u{1b}' => Self::Escape,
            '\u{7f}' | '\u{08}' => Self::Backspace,
            _ => Self::Character(character),
        }
    }
}

macro_rules! define_commands {
    ($(($command:ident, $name:literal, $binding:literal, [$($builtin:expr),*])),* $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
        pub enum Command { $($command),* }
        impl Command {
            pub fn all() -> &'static [Self] { &[$(Self::$command),*] }
            pub fn name(self) -> &'static str { match self { $(Self::$command => $name),* } }
            pub fn default_binding(self) -> &'static str { match self { $(Self::$command => $binding),* } }
            pub fn builtin_keys(self) -> &'static [InputKey] { match self { $(Self::$command => &[$($builtin),*]),* } }
        }
    };
}

define_commands!(
    (ScrollUp, "scroll_up", "k", [InputKey::Up]),
    (ScrollDown, "scroll_down", "j", [InputKey::Down]),
    (PageUp, "page_up", "h", [InputKey::PageUp, InputKey::Left]),
    (
        PageDown,
        "page_down",
        "l",
        [
            InputKey::PageDown,
            InputKey::Character(' '),
            InputKey::Right
        ]
    ),
    (HalfScreenUp, "half_screen_up", "\u{15}", []),
    (HalfScreenDown, "half_screen_down", "\u{04}", []),
    (NextChapter, "next_chapter", "L", []),
    (PrevChapter, "prev_chapter", "H", []),
    (BeginningOfCh, "beginning_of_ch", "g", [InputKey::Home]),
    (EndOfCh, "end_of_ch", "G", [InputKey::End]),
    (Shrink, "shrink", "-", []),
    (Enlarge, "enlarge", "+", []),
    (SetWidth, "set_width", "=", []),
    (Metadata, "metadata", "M", []),
    (DefineWord, "define_word", "d", []),
    (TableOfContents, "table_of_contents", "t", [InputKey::Tab]),
    (Follow, "follow", "f", [InputKey::Enter]),
    (OpenImage, "open_image", "o", []),
    (RegexSearch, "regex_search", "/", []),
    (ShowHideProgress, "show_hide_progress", "s", []),
    (MarkPosition, "mark_position", "m", []),
    (JumpToPosition, "jump_to_position", "`", []),
    (AddBookmark, "add_bookmark", "b", []),
    (ShowBookmarks, "show_bookmarks", "B", []),
    (
        Quit,
        "quit",
        "q",
        [InputKey::Character('\u{03}'), InputKey::Escape]
    ),
    (Help, "help", "?", []),
    (SwitchColor, "switch_color", "c", []),
    (DoubleSpreadToggle, "double_spread_toggle", "D", []),
    (Library, "library", "R", []),
    (AnnotationSelect, "annotation_select", "v", []),
    (AnnotationList, "annotation_list", "a", []),
    (AnnotationLink, "annotation_link", "O", []),
    (JumpBackward, "jump_backward", "\u{0f}", []),
    (JumpForward, "jump_forward", "", [InputKey::ControlI]),
);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KeyBindings {
    entries: BTreeMap<String, Value>,
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            entries: Command::all()
                .iter()
                .map(|command| {
                    (
                        command.name().into(),
                        Value::String(command.default_binding().into()),
                    )
                })
                .collect(),
        }
    }
}

impl KeyBindings {
    pub fn set(&mut self, command: Command, binding: impl Into<String>) {
        self.entries
            .insert(command.name().into(), Value::String(binding.into()));
    }

    pub fn binding(&self, command: Command) -> Result<&str> {
        self.entries
            .get(command.name())
            .map_or(Ok(command.default_binding()), |value| {
                value.as_str().ok_or_else(|| {
                    Error::Compatibility(format!(
                        "{} must be a string in configuration.json.",
                        command.name()
                    ))
                })
            })
    }

    pub fn effective_keys(&self, command: Command) -> Result<Vec<InputKey>> {
        let mut keys = self
            .binding(command)?
            .chars()
            .map(InputKey::from_character)
            .collect::<Vec<_>>();
        keys.extend_from_slice(command.builtin_keys());
        keys.sort_unstable();
        keys.dedup();
        Ok(keys)
    }

    pub fn command_for(&self, key: InputKey) -> Option<Command> {
        Command::all().iter().copied().find(|command| {
            self.effective_keys(*command)
                .is_ok_and(|keys| keys.contains(&key))
        })
    }

    fn validate_conflicts(&self) -> Result<()> {
        for command in Command::all() {
            self.binding(*command)?;
        }
        for reserved in [
            Command::AnnotationSelect,
            Command::AnnotationList,
            Command::AnnotationLink,
            Command::JumpBackward,
            Command::JumpForward,
        ] {
            let keys = self.effective_keys(reserved)?;
            for other in Command::all()
                .iter()
                .copied()
                .filter(|command| *command != reserved)
            {
                if self
                    .effective_keys(other)?
                    .iter()
                    .any(|key| keys.contains(key))
                {
                    return Err(Error::Compatibility(format!(
                        "{} conflicts with {}. Choose different keys in configuration.json.",
                        reserved.name(),
                        other.name()
                    )));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Configuration {
    pub settings: Settings,
    pub bindings: KeyBindings,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Configuration {
    pub fn from_json(content: &str) -> Result<Self> {
        let configuration = serde_json::from_str::<Self>(content)?;
        configuration.validate()?;
        Ok(configuration)
    }

    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(content) => Self::from_json(&content),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if !(16..=100).contains(&self.settings.annotation_notes_width) {
            return Err(Error::Compatibility(
                "annotation_notes_width must be between 16 and 100 cells.".into(),
            ));
        }
        if self.settings.annotation_scrolloff < 0 {
            return Err(Error::Compatibility(
                "annotation_scrolloff must be a non-negative integer.".into(),
            ));
        }
        self.bindings.validate_conflicts()
    }
}
