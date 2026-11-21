//! A loopback HTTP stub: each accepted connection gets the next scripted
//! response, and the request it carried is sent back for assertions.
#![allow(dead_code)]

use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Debug)]
pub struct CapturedRequest {
    pub method: String,
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub raw_body: String,
    /// The body as JSON, `Null` when it is empty or not JSON.
    pub body: Value,
}

pub struct ResponseSpec {
    pub status: u16,
    pub content_type: &'static str,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub delay: Duration,
}

impl ResponseSpec {
    pub fn json(body: Value) -> Self {
        Self::status(200, body)
    }

    pub fn status(status: u16, body: Value) -> Self {
        Self::text(status, body.to_string())
    }

    pub fn text(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "application/json",
            headers: Vec::new(),
            body: body.into(),
            delay: Duration::ZERO,
        }
    }

    pub fn event_stream(events: &[Value]) -> Self {
        let body = events
            .iter()
            .map(|event| {
                format!(
                    "event: {}\ndata: {event}\n\n",
                    event["type"].as_str().unwrap()
                )
            })
            .collect::<String>();
        Self {
            content_type: "text/event-stream",
            ..Self::text(200, body)
        }
    }
}

pub fn spawn_server(
    responses: Vec<ResponseSpec>,
) -> (String, Receiver<CapturedRequest>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, receiver) = mpsc::channel();
    let handle = thread::spawn(move || {
        for response in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            sender.send(request).unwrap();
            if !response.delay.is_zero() {
                thread::sleep(response.delay);
            }
            write_response(&mut stream, response);
        }
    });
    (format!("http://{address}"), receiver, handle)
}

fn read_request(stream: &mut TcpStream) -> CapturedRequest {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).unwrap();
        assert!(read > 0, "client closed before sending complete headers");
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };

    let header_text = std::str::from_utf8(&bytes[..header_end]).unwrap();
    let mut lines = header_text.split("\r\n");
    let mut request_line = lines.next().unwrap().split_whitespace();
    let method = request_line.next().unwrap().to_owned();
    let path = request_line.next().unwrap().to_owned();
    let mut headers = BTreeMap::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').unwrap();
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    while bytes.len() - header_end < content_length {
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).unwrap();
        assert!(read > 0, "client closed before sending complete body");
        bytes.extend_from_slice(&chunk[..read]);
    }
    let raw_body =
        String::from_utf8(bytes[header_end..header_end + content_length].to_vec()).unwrap();
    let body = serde_json::from_str(&raw_body).unwrap_or(Value::Null);

    CapturedRequest {
        method,
        path,
        headers,
        raw_body,
        body,
    }
}

fn write_response(stream: &mut TcpStream, response: ResponseSpec) {
    let reason = match response.status {
        200 => "OK",
        302 => "Found",
        307 => "Temporary Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        413 => "Payload Too Large",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        _ => "Test Status",
    };
    let mut headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        response.status,
        reason,
        response.content_type,
        response.body.len()
    );
    for (name, value) in response.headers {
        headers.push_str(&format!("{name}: {value}\r\n"));
    }
    headers.push_str("\r\n");
    let _ = stream.write_all(headers.as_bytes());
    let _ = stream.write_all(response.body.as_bytes());
}
