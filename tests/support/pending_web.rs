use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;
use std::time::Duration;

pub struct PendingWeb {
    pub location: String,
    pub requests: crossbeam_channel::Receiver<()>,
    release: crossbeam_channel::Sender<()>,
    stopped: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl PendingWeb {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let location = format!("http://{}/pending", listener.local_addr().unwrap());
        let stopped = Arc::new(AtomicBool::new(false));
        let flag = stopped.clone();
        let (sender, requests) = crossbeam_channel::bounded(1);
        let (release, ready) = crossbeam_channel::bounded(1);
        let thread = std::thread::spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                if let Ok((mut stream, _)) = listener.accept() {
                    if !read_headers(&mut stream) {
                        break;
                    }
                    sender.send(()).unwrap();
                    while !flag.load(Ordering::SeqCst) {
                        if ready.recv_timeout(Duration::from_millis(2)).is_ok() {
                            let body =
                                "<html><title>Deferred fixture</title><p>Remote book.</p></html>";
                            let response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            );
                            let _ = stream.write_all(response.as_bytes());
                            break;
                        }
                    }
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        });
        Self {
            location,
            requests,
            release,
            stopped,
            thread: Some(thread),
        }
    }

    pub fn respond(&self) {
        self.release.send(()).unwrap();
    }
}

fn read_headers(stream: &mut TcpStream) -> bool {
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut headers = Vec::new();
    let mut buffer = [0u8; 512];
    while headers.len() < 8192 {
        let Ok(length) = stream.read(&mut buffer) else {
            return false;
        };
        if length == 0 {
            return false;
        }
        headers.extend_from_slice(&buffer[..length]);
        if headers.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            return true;
        }
    }
    false
}

impl Drop for PendingWeb {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}
