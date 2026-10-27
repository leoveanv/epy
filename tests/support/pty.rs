use rustix::fs::{Mode, OFlags};
use rustix::process::{Pid, Signal};
use rustix::termios::Winsize;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

pub struct PtyReader {
    child: Child,
    master: Option<File>,
    output: Vec<u8>,
}

impl PtyReader {
    pub fn start(book: &Path, configuration_home: &Path) -> Self {
        Self::start_with_tmux(book, configuration_home, false)
    }

    #[allow(dead_code)]
    pub fn start_with_tmux(book: &Path, configuration_home: &Path, tmux: bool) -> Self {
        Self::start_sized(book, configuration_home, tmux, 100, 30)
    }

    #[allow(dead_code)]
    pub fn start_sized(
        book: &Path,
        configuration_home: &Path,
        tmux: bool,
        columns: u16,
        rows: u16,
    ) -> Self {
        // #NOTE: Atomic close-on-exec prevents parallel readers from inheriting another test's terminal lease.
        let master = rustix::fs::open(
            c"/dev/ptmx",
            OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        rustix::pty::grantpt(&master).unwrap();
        rustix::pty::unlockpt(&master).unwrap();
        let name = rustix::pty::ptsname(&master, Vec::new()).unwrap();
        let slave = rustix::fs::open(
            name.as_c_str(),
            OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        rustix::termios::tcsetwinsize(
            &slave,
            Winsize {
                ws_row: rows,
                ws_col: columns,
                ws_xpixel: 800,
                ws_ypixel: 480,
            },
        )
        .unwrap();
        rustix::fs::fcntl_setfl(&master, OFlags::NONBLOCK).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_ers"));
        command
            .arg(book)
            .env_remove("TMUX")
            .env("TERM", "xterm-256color")
            .env("TERM_PROGRAM", "ers-test")
            .env("PATH", "/nonexistent")
            .env("XDG_CONFIG_HOME", configuration_home)
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave));
        if tmux {
            command.env("TMUX", "/tmp/ers-test-tmux,1,0");
        }
        // #NOTE: Only an async-signal-safe syscall runs after fork. No controlling terminal means EOF, not SIGHUP, must end the reader.
        unsafe {
            command.pre_exec(|| {
                rustix::process::setsid()
                    .map(|_| ())
                    .map_err(std::io::Error::from)
            });
        }
        let child = command.spawn().unwrap();
        Self {
            child,
            master: Some(File::from(master)),
            output: Vec::new(),
        }
    }

    pub fn wait_text(&mut self, expected: &str) {
        self.wait_text_after(0, expected);
    }

    pub fn wait_text_after(&mut self, checkpoint: usize, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.output[checkpoint..]
            .windows(expected.len())
            .any(|bytes| bytes == expected.as_bytes())
        {
            self.drain();
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "Reader exited early"
            );
            assert!(
                Instant::now() < deadline,
                "Missing {expected:?}: {}",
                String::from_utf8_lossy(&self.output[self.output.len().saturating_sub(2000)..])
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    pub fn send(&mut self, keys: &[u8]) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut remaining = keys;
        while !remaining.is_empty() {
            match self.master.as_mut().unwrap().write(remaining) {
                Ok(0) => panic!("The terminal stopped accepting input"),
                Ok(written) => remaining = &remaining[written..],
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    self.drain();
                    assert!(Instant::now() < deadline, "Terminal input remained blocked");
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(error) => panic!("PTY write failed: {error}"),
            }
        }
    }

    #[allow(dead_code)]
    pub fn output_checkpoint(&mut self) -> usize {
        self.drain();
        self.output.len()
    }

    #[allow(dead_code)]
    pub fn output_since(&mut self, checkpoint: usize) -> Vec<u8> {
        self.drain();
        self.output[checkpoint..].to_vec()
    }

    #[allow(dead_code)]
    pub fn wait_output_after(&mut self, checkpoint: usize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.output.len() <= checkpoint {
            self.drain();
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "Reader exited early"
            );
            assert!(
                Instant::now() < deadline,
                "The reader did not render another frame"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[allow(dead_code)]
    pub fn saw_output(&mut self, expected: &str) -> bool {
        self.drain();
        self.output
            .windows(expected.len())
            .any(|bytes| bytes == expected.as_bytes())
    }

    #[allow(dead_code)]
    pub fn resize(&self, columns: u16, rows: u16) {
        rustix::termios::tcsetwinsize(
            self.master.as_ref().unwrap(),
            Winsize {
                ws_row: rows,
                ws_col: columns,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )
        .unwrap();
        // #NOTE: The isolated PTY has no controlling terminal to deliver its resize signal.
        rustix::process::kill_process(Pid::from_child(&self.child), Signal::WINCH).unwrap();
    }

    pub fn close_terminal(&mut self) {
        self.master.take();
    }

    pub fn terminate(&self) {
        rustix::process::kill_process(Pid::from_child(&self.child), Signal::TERM).unwrap();
    }

    pub fn wait_exit(&mut self) -> ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            self.drain();
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "The reader survived its terminal"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn drain(&mut self) {
        let Some(master) = &mut self.master else {
            return;
        };
        let mut bytes = [0; 8192];
        for _ in 0..16 {
            match master.read(&mut bytes) {
                Ok(0) => break,
                Ok(count) => self.output.extend_from_slice(&bytes[..count]),
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error()) =>
                {
                    break;
                }
                Err(error) => panic!("PTY read failed: {error}"),
            }
        }
    }
}

impl Drop for PtyReader {
    fn drop(&mut self) {
        self.master.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
