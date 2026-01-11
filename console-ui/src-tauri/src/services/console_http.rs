use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use reqwest::header::CONTENT_TYPE;
use reqwest::Client;
use serde_json::Value;

use crate::services::http_utils::join_base_path;
use crate::services::logging::{append_log_line, escape_log_body};

pub const CONSOLE_HTTP_HOST: &str = "127.0.0.1:19309";
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const HTTP_IO_TIMEOUT: Duration = Duration::from_secs(5);
static HTTP_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
static HTTP_CLIENT: OnceLock<Client> = OnceLock::new();

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
        console_http_request_with_timeout("POST", path, Some(payload), log_path, io_timeout)
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
    body: Option<String>,
    log_path: &Path,
    io_timeout: Duration,
) -> Result<HttpResponse, String> {
    let request_id = HTTP_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let body_len = body.as_ref().map(|value| value.len()).unwrap_or(0);
    let _ = append_log_line(
        log_path,
        &format!("console http {method}#{request_id} start path={path} body_len={body_len}"),
    );
    let base_url = format!("http://{}", CONSOLE_HTTP_HOST);
    let url = join_base_path(&base_url, path).map_err(|err| {
        let _ = append_log_line(
            log_path,
            &format!("console http {method}#{request_id} invalid url: {err}"),
        );
        err
    })?;
    let client = http_client().map_err(|err| err.to_string())?;
    let mut request = match method {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        _ => return Err(format!("console http unsupported method {method}")),
    };
    request = request
        .header("Accept", "application/json")
        .header("Connection", "close")
        .timeout(io_timeout);
    if let Some(body) = body {
        request = request.header(CONTENT_TYPE, "application/json").body(body);
    }
    let response = match request.send().await {
        Ok(response) => response,
        Err(err) => {
            let _ = append_log_line(
                log_path,
                &format!(
                    "console http {method}#{request_id} reqwest error timeout={} connect={} status={}",
                    err.is_timeout(),
                    err.is_connect(),
                    err.status()
                        .map(|value| value.as_u16().to_string())
                        .unwrap_or_else(|| "none".to_string())
                ),
            );
            let _ = append_log_line(
                log_path,
                &format!("console http {method}#{request_id} reqwest error={err:?}"),
            );
            return Err(err.to_string());
        }
    };
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("unknown")
        .to_string();
    let body = response.text().await.map_err(|err| {
        let _ = append_log_line(
            log_path,
            &format!("console http {method}#{request_id} read error: {err}"),
        );
        err.to_string()
    })?;
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

fn build_http_client() -> Result<Client, reqwest::Error> {
    Client::builder()
        .connect_timeout(HTTP_CONNECT_TIMEOUT)
        .build()
}

fn http_client() -> Result<&'static Client, reqwest::Error> {
    if let Some(client) = HTTP_CLIENT.get() {
        return Ok(client);
    }
    let client = build_http_client()?;
    let _ = HTTP_CLIENT.set(client);
    Ok(HTTP_CLIENT
        .get()
        .expect("http client should be initialized"))
}
