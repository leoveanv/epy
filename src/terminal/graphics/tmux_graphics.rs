use ratatui::layout::Size;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const COMMAND_TIMEOUT: Duration = Duration::from_millis(80);
const ANCESTRY_TIMEOUT: Duration = Duration::from_millis(200);
const OUTPUT_MAX: u64 = 8192;

pub(super) struct TmuxGraphics {
    pane: String,
    previous: Option<Option<String>>,
}

impl TmuxGraphics {
    pub fn acquire() -> Option<Self> {
        let pane = std::env::var("TMUX_PANE").ok()?;
        if !valid_pane(&pane) {
            return None;
        }
        let details = tmux(&[
            "display-message",
            "-p",
            "-t",
            &pane,
            "#{pane_tty}\t#{client_pid}\t#{client_termname}\t#{allow-passthrough}",
        ])?;
        let mut fields = details.trim_end().split('\t');
        let pane_tty = fields.next()?;
        let client_pid = fields.next()?.parse::<u32>().ok()?;
        let terminal = fields.next()?;
        let passthrough = fields.next()?;
        let stdin_tty = rustix::termios::ttyname(std::io::stdin(), Vec::new()).ok()?;
        if stdin_tty.to_str().ok()? != pane_tty
            || !matches!(passthrough, "on" | "all")
            || !single_client(client_pid)
            || !local_graphics_terminal(client_pid, terminal)
        {
            return None;
        }
        let mut graphics = Self {
            pane,
            previous: None,
        };
        if passthrough == "on" {
            let option = tmux(&[
                "show-options",
                "-p",
                "-q",
                "-v",
                "-t",
                &graphics.pane,
                "allow-passthrough",
            ])?;
            graphics.previous = Some((!option.trim().is_empty()).then(|| option.trim().to_owned()));
            tmux(&[
                "set-option",
                "-p",
                "-t",
                &graphics.pane,
                "allow-passthrough",
                "all",
            ])?;
        }
        Some(graphics)
    }

    pub fn cell_size(&self) -> Option<Size> {
        let output = tmux(&[
            "display-message",
            "-p",
            "-t",
            &self.pane,
            "#{client_cell_width}\t#{client_cell_height}",
        ])?;
        let (width, height) = output.trim().split_once('\t')?;
        let width = width.parse::<u16>().ok().filter(|value| *value > 0)?;
        let height = height.parse::<u16>().ok().filter(|value| *value > 0)?;
        Some(Size::new(width, height))
    }
}

impl Drop for TmuxGraphics {
    fn drop(&mut self) {
        let Some(previous) = &self.previous else {
            return;
        };
        let current = tmux(&[
            "show-options",
            "-p",
            "-q",
            "-v",
            "-t",
            &self.pane,
            "allow-passthrough",
        ]);
        if current.as_deref().map(str::trim) != Some("all") {
            return;
        }
        match previous {
            Some(value) => {
                let _ = tmux(&[
                    "set-option",
                    "-p",
                    "-t",
                    &self.pane,
                    "allow-passthrough",
                    value,
                ]);
            }
            None => {
                let _ = tmux(&[
                    "set-option",
                    "-p",
                    "-u",
                    "-t",
                    &self.pane,
                    "allow-passthrough",
                ]);
            }
        }
    }
}

fn valid_pane(pane: &str) -> bool {
    pane.strip_prefix('%')
        .is_some_and(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
}

fn single_client(client_pid: u32) -> bool {
    let Some(clients) = tmux(&["list-clients", "-F", "#{client_pid}"]) else {
        return false;
    };
    let mut clients = clients.lines();
    clients.next().and_then(|pid| pid.parse::<u32>().ok()) == Some(client_pid)
        && clients.next().is_none()
}

struct ProcessInfo {
    parent: u32,
    executable: String,
}

// #NOTE: A file transfer works only when the terminal and tmux client share the local filesystem.
fn local_graphics_terminal(pid: u32, terminal: &str) -> bool {
    let deadline = Instant::now() + ANCESTRY_TIMEOUT;
    graphics_ancestor(pid, terminal, |pid| {
        (Instant::now() < deadline)
            .then(|| process_info(pid))
            .flatten()
    })
}

fn graphics_ancestor(
    mut pid: u32,
    terminal: &str,
    mut process_info: impl FnMut(u32) -> Option<ProcessInfo>,
) -> bool {
    let expected = match terminal.to_ascii_lowercase().as_str() {
        "ghostty" | "xterm-ghostty" => "ghostty",
        "kitty" | "xterm-kitty" => "kitty",
        _ => return false,
    };
    for _ in 0..16 {
        if pid <= 1 {
            return false;
        }
        let Some(process) = process_info(pid) else {
            return false;
        };
        let name = std::path::Path::new(&process.executable)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if name.eq_ignore_ascii_case(expected) {
            return true;
        }
        pid = process.parent;
    }
    false
}

fn process_info(pid: u32) -> Option<ProcessInfo> {
    let output = command(
        "/bin/ps",
        &["-p", &pid.to_string(), "-o", "ppid=", "-o", "comm="],
    )?;
    let (parent, executable) = output.trim().split_once(char::is_whitespace)?;
    Some(ProcessInfo {
        parent: parent.parse().ok()?,
        executable: executable.trim().into(),
    })
}

fn tmux(arguments: &[&str]) -> Option<String> {
    command("tmux", arguments)
}

fn command(program: &str, arguments: &[&str]) -> Option<String> {
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let mut output = String::new();
                child
                    .stdout
                    .take()?
                    .take(OUTPUT_MAX)
                    .read_to_string(&mut output)
                    .ok()?;
                return Some(output);
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(1)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ProcessInfo, graphics_ancestor};
    use std::collections::BTreeMap;

    struct ProcessFixture {
        entries: BTreeMap<u32, (u32, &'static str)>,
    }

    impl ProcessFixture {
        fn new() -> Self {
            Self {
                entries: BTreeMap::new(),
            }
        }

        fn process(mut self, pid: u32, parent: u32, executable: &'static str) -> Self {
            self.entries.insert(pid, (parent, executable));
            self
        }

        fn supports(&self, pid: u32, terminal: &str) -> bool {
            graphics_ancestor(pid, terminal, |pid| {
                self.entries
                    .get(&pid)
                    .map(|(parent, executable)| ProcessInfo {
                        parent: *parent,
                        executable: (*executable).into(),
                    })
            })
        }
    }

    #[test]
    fn local_terminal_ancestry_enables_file_transfers() {
        let fixture = ProcessFixture::new()
            .process(20, 19, "/opt/homebrew/bin/tmux")
            .process(19, 18, "/usr/bin/login")
            .process(18, 1, "/Applications/Ghostty.app/Contents/MacOS/ghostty");
        assert!(fixture.supports(20, "xterm-ghostty"));
        assert!(!fixture.supports(20, "xterm-kitty"));
    }

    #[test]
    fn remote_or_unknown_clients_do_not_enable_file_transfers() {
        let fixture = ProcessFixture::new()
            .process(20, 19, "/opt/homebrew/bin/tmux")
            .process(19, 1, "/usr/sbin/sshd");
        assert!(!fixture.supports(20, "xterm-ghostty"));
        assert!(!fixture.supports(20, "xterm-kitty"));
        assert!(!fixture.supports(20, "xterm-256color"));
    }
}
