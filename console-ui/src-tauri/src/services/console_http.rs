use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::services::logging::{append_log_line, escape_log_body};

pub const CONSOLE_HTTP_HOST: &str = "127.0.0.1:19309";
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const HTTP_IO_TIMEOUT: Duration = Duration::from_secs(5);
pub const HTTP_RELOAD_TIMEOUT: Duration = Duration::from_secs(120);
static HTTP_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

struct HttpResponse {
    status: u16,
    body: String,
}

pub async fn console_get(path: &str, log_path: &Path) -> Result<Value, String> {
    let response =
        console_http_request_with_timeout("GET", path, None, log_path, HTTP_IO_TIMEOUT).await?;
    if response.status / 100 != 2 {
        return Err(format!(
            "console http GET status {} for {}",
            response.status, path
        ));
    }
    serde_json::from_str(&response.body).map_err(|err| {
        let _ = append_log_line(log_path, &format!("console http GET parse error: {err}"));
        err.to_string()
    })
}

pub async fn console_post(path: &str, payload: Value, log_path: &Path) -> Result<(), String> {
    console_post_with_timeout(path, payload, log_path, HTTP_IO_TIMEOUT).await
}

pub async fn console_post_with_timeout(
    path: &str,
    payload: Value,
    log_path: &Path,
    io_timeout: Duration,
) -> Result<(), String> {
    let payload = payload.to_string();
    let _ = append_log_line(log_path, &format!("console http POST payload: {}", payload));
    let response =
        console_http_request_with_timeout("POST", path, Some(&payload), log_path, io_timeout)
            .await?;
    if response.status / 100 != 2 {
        return Err(format!(
            "console http POST status {} for {}",
            response.status, path
        ));
    }
    Ok(())
}

async fn console_http_request_with_timeout(
    method: &str,
    path: &str,
    body: Option<&str>,
    log_path: &Path,
    io_timeout: Duration,
) -> Result<HttpResponse, String> {
    let request_id = HTTP_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let body_len = body.map(|value| value.len()).unwrap_or(0);
    let _ = append_log_line(
        log_path,
        &format!("console http {method}#{request_id} start path={path} body_len={body_len}"),
    );
    let mut stream = timeout(HTTP_CONNECT_TIMEOUT, TcpStream::connect(CONSOLE_HTTP_HOST))
        .await
        .map_err(|_| "console http connect timed out".to_string())?
        .map_err(|err| err.to_string())?;
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {CONSOLE_HTTP_HOST}\r\nAccept: application/json\r\nConnection: close\r\n"
    );
    if let Some(body) = body {
        request.push_str("Content-Type: application/json\r\n");
        request.push_str(&format!("Content-Length: {}\r\n", body.len()));
        request.push_str("\r\n");
        request.push_str(body);
    } else {
        request.push_str("\r\n");
    }
    timeout(io_timeout, stream.write_all(request.as_bytes()))
        .await
        .map_err(|_| "console http write timed out".to_string())?
        .map_err(|err| err.to_string())?;
    let mut buffer = Vec::new();
    timeout(io_timeout, stream.read_to_end(&mut buffer))
        .await
        .map_err(|_| "console http read timed out".to_string())?
        .map_err(|err| err.to_string())?;
    let (status, headers, body) = parse_http_response(&buffer)?;
    let content_type = headers
        .get("content-type")
        .cloned()
        .unwrap_or_else(|| "unknown".to_string());
    let _ = append_log_line(
        log_path,
        &format!(
            "console http {method}#{request_id} status={} content-type={}",
            status, content_type
        ),
    );
    let _ = append_log_line(
        log_path,
        &format!("console http {method}#{request_id} body_len={}", body.len()),
    );
    let _ = append_log_line(
        log_path,
        &format!(
            "console http {method}#{request_id} body: {}",
            escape_log_body(&body)
        ),
    );
    Ok(HttpResponse { status, body })
}

fn parse_http_response(bytes: &[u8]) -> Result<(u16, HashMap<String, String>, String), String> {
    let header_end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "console http response missing header".to_string())?;
    let head = String::from_utf8_lossy(&bytes[..header_end]);
    let body_bytes = &bytes[(header_end + 4)..];
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(0);
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_lowercase(), value.trim().to_string());
        }
    }
    let body = if headers
        .get("transfer-encoding")
        .map(|value| value.to_lowercase().contains("chunked"))
        .unwrap_or(false)
    {
        decode_chunked_body(body_bytes)?
    } else {
        body_bytes.to_vec()
    };
    Ok((status, headers, String::from_utf8_lossy(&body).to_string()))
}

fn decode_chunked_body(body: &[u8]) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut index = 0usize;
    while index < body.len() {
        let line_end = find_crlf(body, index)
            .ok_or_else(|| "console http chunked response missing size line".to_string())?;
        let line = String::from_utf8_lossy(&body[index..line_end]);
        let size_str = line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_str, 16)
            .map_err(|_| "console http chunked size parse failed".to_string())?;
        index = line_end + 2;
        if size == 0 {
            break;
        }
        if index + size > body.len() {
            return Err("console http chunked body truncated".to_string());
        }
        output.extend_from_slice(&body[index..index + size]);
        index += size;
        if index + 2 > body.len() || &body[index..index + 2] != b"\r\n" {
            return Err("console http chunked body missing terminator".to_string());
        }
        index += 2;
    }
    Ok(output)
}

fn find_crlf(body: &[u8], start: usize) -> Option<usize> {
    body[start..]
        .windows(2)
        .position(|window| window == b"\r\n")
        .map(|offset| start + offset)
}
