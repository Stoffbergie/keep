use anyhow::{bail, Result};
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::{Client, RequestBuilder, Response, StatusCode};
use std::io::IsTerminal;
use std::time::Duration;

const DEFAULT_URL: &str = "https://keep.stoff.dev";
const ATTEMPTS: u32 = 5;
const PROGRESS_FROM_BYTES: u64 = 32 * 1024 * 1024;

pub struct Link {
    pub url: String,
    pub id: String,
    pub key: Option<String>,
}

pub fn base_url() -> String {
    match std::env::var("KEEP_URL") {
        Ok(url) => url.trim_end_matches('/').to_string(),
        Err(_) => DEFAULT_URL.to_string(),
    }
}

pub fn client() -> Result<Client> {
    let client = Client::builder()
        .user_agent(concat!("keep/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .build()?;
    Ok(client)
}

pub fn parse_link(input: &str) -> Link {
    let (address, fragment) = input.split_once('#').unwrap_or((input, ""));
    let key = fragment.trim_start_matches("key=");
    let address = address
        .trim()
        .trim_end_matches('/')
        .trim_end_matches("/raw");
    let url = if address.contains("://") {
        address.to_string()
    } else {
        format!("{}/{}", base_url(), address)
    };
    let id = url.rsplit('/').next().unwrap_or_default().to_string();
    let key = if key.is_empty() {
        None
    } else {
        Some(key.to_string())
    };
    Link { url, id, key }
}

pub async fn send(request: impl Fn() -> RequestBuilder) -> Result<Response> {
    ensure_success(send_unchecked(request).await?).await
}

pub async fn ensure_success(response: Response) -> Result<Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let message = body.trim();
    if message.is_empty() {
        bail!("{status}");
    }
    bail!("{message} ({})", status.as_u16())
}

pub async fn send_unchecked(request: impl Fn() -> RequestBuilder) -> Result<Response> {
    let mut attempt = 1;
    loop {
        let result = request().send().await;
        let retry = match &result {
            Ok(response) => is_retryable(response.status()),
            Err(error) => !error.is_builder(),
        };
        if !retry || attempt == ATTEMPTS {
            return Ok(result?);
        }
        tokio::time::sleep(Duration::from_millis(250 << attempt)).await;
        attempt += 1;
    }
}

pub fn progress(total: u64) -> Result<ProgressBar> {
    if total < PROGRESS_FROM_BYTES || !std::io::stderr().is_terminal() {
        return Ok(ProgressBar::hidden());
    }
    let style =
        ProgressStyle::with_template("{bytes} / {total_bytes}  {bytes_per_sec}  {eta} left")?;
    Ok(ProgressBar::new(total).with_style(style))
}

fn is_retryable(status: StatusCode) -> bool {
    status.is_server_error()
        || status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::REQUEST_TIMEOUT
}
