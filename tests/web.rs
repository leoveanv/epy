use ers::book::{Book, BookLocation, BookOpenOptions, ResourceLimits};
use ers::cancellation::Cancellation;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

fn read_request_path(stream: &mut TcpStream) -> Option<String> {
    let mut request = Vec::new();
    let mut buffer = [0u8; 512];
    while request.len() < 8192 {
        let length = stream.read(&mut buffer).ok()?;
        if length == 0 {
            return None;
        }
        request.extend_from_slice(&buffer[..length]);
        if request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            return String::from_utf8_lossy(&request)
                .split_whitespace()
                .nth(1)
                .map(str::to_owned);
        }
    }
    None
}

struct WebServer {
    address: String,
    stopped: Arc<AtomicBool>,
    requests: crossbeam_channel::Receiver<String>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl WebServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let stopped = Arc::new(AtomicBool::new(false));
        let flag = stopped.clone();
        let (sender, requests) = crossbeam_channel::unbounded();
        let thread = std::thread::spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                // #NOTE: Accepted sockets can inherit nonblocking mode; HTTP headers may also span several reads.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let Some(path) = read_request_path(&mut stream) else {
                    continue;
                };
                sender.send(path.clone()).unwrap();
                if path == "/slow" {
                    while !flag.load(Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    continue;
                }
                let response = match path.as_str() {
                    "/redirect" => "HTTP/1.1 302 Found\r\nLocation: /page/index.html\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
                    "/page/index.html" => { let body = "<html><head><title>Remote fixture</title></head><body><p>Remote café passage.</p><img src='../image.png'/></body></html>"; format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()) },
                    "/image.png" => "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nimage".into(),
                    "/large" => "HTTP/1.1 200 OK\r\nContent-Length: 1000000000\r\nConnection: close\r\n\r\n".into(),
                    _ => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
                };
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Self {
            address,
            stopped,
            requests,
            thread: Some(thread),
        }
    }
    fn url(&self, path: &str) -> BookLocation {
        BookLocation::parse(&format!("{}{path}", self.address)).unwrap()
    }
}
impl Drop for WebServer {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

#[test]
fn redirected_web_pages_resolve_assets_against_the_final_url() {
    let server = WebServer::start();
    let mut book = Book::open(server.url("/redirect")).unwrap_or_else(|error| {
        panic!(
            "{error}; received paths: {:?}",
            server.requests.try_iter().collect::<Vec<_>>()
        )
    });
    let text = book.raw_text(&book.contents()[0].clone()).unwrap();
    assert!(text.contains("Remote café passage."));
    let target = book.resolve_resource("_", "../image.png").unwrap();
    assert_eq!(book.asset(&target.content_id).unwrap().bytes, b"image");
    assert_eq!(book.metadata().title.as_deref(), Some("Remote fixture"));
}

#[test]
fn unsuccessful_http_status_is_not_rendered_as_an_ebook() {
    let server = WebServer::start();
    assert!(Book::open(server.url("/missing")).is_err());
}

#[test]
fn oversized_http_responses_are_rejected_before_body_allocation() {
    let server = WebServer::start();
    assert!(
        Book::open_with_options(
            server.url("/large"),
            BookOpenOptions {
                limits: ResourceLimits {
                    resource_bytes_max: 1024,
                    ..ResourceLimits::default()
                },
                ..BookOpenOptions::default()
            }
        )
        .is_err()
    );
}

#[test]
fn web_loading_can_be_canceled_while_waiting_for_headers() {
    let server = WebServer::start();
    let location = server.url("/slow");
    let cancellation = Cancellation::default();
    let worker_cancel = cancellation.clone();
    let (sender, result) = crossbeam_channel::bounded(1);
    let worker = std::thread::spawn(move || {
        sender
            .send(
                Book::open_with_options(
                    location,
                    BookOpenOptions {
                        cancellation: worker_cancel,
                        ..BookOpenOptions::default()
                    },
                )
                .err(),
            )
            .unwrap();
    });
    assert_eq!(
        server
            .requests
            .recv_timeout(Duration::from_secs(5))
            .unwrap(),
        "/slow"
    );
    cancellation.cancel();
    assert!(matches!(
        result.recv_timeout(Duration::from_secs(2)).unwrap(),
        Some(ers::Error::Canceled)
    ));
    worker.join().unwrap();
}
