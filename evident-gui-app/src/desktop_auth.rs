//! Stage 13.4 — desktop token Keychain helpers + browser connect bridge.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

const SERVICE: &str = "com.evidentledger.desktop";
const ACCOUNT: &str = "desktop_token";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthState {
    Unauthenticated,
    Authenticated,
    Expired,
    Revoked,
}

pub fn load_token_from_keychain() -> Option<String> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT).ok()?;
    entry.get_password().ok().filter(|t| t.starts_with("desktop_"))
}

pub fn save_token_to_keychain(token: &str) -> Result<(), String> {
    let entry =
        keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| format!("keychain entry: {e}"))?;
    entry
        .set_password(token)
        .map_err(|e| format!("keychain save: {e}"))
}

pub fn clear_token_from_keychain() {
    if let Ok(entry) = keyring::Entry::new(SERVICE, ACCOUNT) {
        let _ = entry.delete_credential();
    }
}

/// Opens the dashboard connect page and waits for localhost callback with the token.
pub fn connect_via_browser(server_base: &str, timeout: Duration) -> Result<String, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("bind: {e}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("nonblocking: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("local_addr: {e}"))?
        .port();
    let redirect = format!("http://127.0.0.1:{port}/callback");
    let url = format!(
        "{}/dashboard/desktop/connect?redirect_uri={}",
        server_base.trim_end_matches('/'),
        urlencoding_encode(&redirect)
    );
    open::that(&url).map_err(|e| format!("open browser: {e}"))?;

    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() > deadline {
            return Err("Timed out waiting for desktop connect callback".into());
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let first_line = req.lines().next().unwrap_or("");
                let token = extract_token_from_request_line(first_line);
                let body = if token.is_some() {
                    "<html><body><h1>Connected</h1><p>You can return to Evident Ledger.</p></body></html>"
                } else {
                    "<html><body><h1>Missing token</h1></body></html>"
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                return token.ok_or_else(|| "Callback did not include token".to_string());
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("accept: {e}")),
        }
    }
}

fn extract_token_from_request_line(line: &str) -> Option<String> {
    // GET /callback?token=desktop_...&expires_at=... HTTP/1.1
    let path = line.split_whitespace().nth(1)?;
    let query = path.split('?').nth(1)?;
    for pair in query.split('&') {
        let mut it = pair.splitn(2, '=');
        let k = it.next()?;
        let v = it.next().unwrap_or("");
        if k == "token" {
            let decoded = urlencoding_decode(v);
            if decoded.starts_with("desktop_") {
                return Some(decoded);
            }
        }
    }
    None
}

fn urlencoding_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn urlencoding_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &s[i + 1..i + 3];
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
