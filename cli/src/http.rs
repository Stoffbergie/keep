use crate::feedback;
use anyhow::{bail, Result};
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::{Client, RequestBuilder, Response, StatusCode};
use std::borrow::Cow;
use std::io::IsTerminal;
use std::time::{Duration, SystemTime};

const DEFAULT_URL: &str = "https://keep.stoff.dev";
const ATTEMPTS: u32 = 5;
const PROGRESS_FROM_BYTES: u64 = 32 * 1024 * 1024;

pub struct Link<'a> {
    pub url: String,
    pub id: &'a str,
    pub key: Option<&'a str>,
}

pub fn base_url() -> Cow<'static, str> {
    match std::env::var("KEEP_URL") {
        Ok(mut url) => {
            url.truncate(url.trim_end_matches('/').len());
            Cow::Owned(url)
        }
        Err(_) => Cow::Borrowed(DEFAULT_URL),
    }
}

pub fn client() -> Result<Client> {
    let client = Client::builder()
        .user_agent(concat!("keep/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .build()?;
    Ok(client)
}

fn link_address(address: &str) -> &str {
    address
        .trim()
        .trim_end_matches('/')
        .trim_end_matches("/raw")
}

pub fn link_id(input: &str) -> &str {
    let (address, _) = input.split_once('#').unwrap_or((input, ""));
    link_address(address).rsplit('/').next().unwrap_or_default()
}

pub fn parse_link(input: &str) -> Link<'_> {
    let (address, fragment) = input.split_once('#').unwrap_or((input, ""));
    let address = link_address(address);
    let url = if address.contains("://") {
        address.to_string()
    } else {
        format!("{}/{}", base_url(), address)
    };
    let id = address.rsplit('/').next().unwrap_or_default();
    let key = fragment_key(fragment);
    Link { url, id, key }
}

pub fn link_key(input: &str) -> Option<&str> {
    let (_, fragment) = input.split_once('#')?;
    fragment_key(fragment)
}

fn fragment_key(fragment: &str) -> Option<&str> {
    let key = fragment.trim_start_matches("key=");
    if key.is_empty() {
        None
    } else {
        Some(key)
    }
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
        let server_delay = result.as_ref().ok().and_then(retry_after);
        let delay = server_delay.unwrap_or_else(|| Duration::from_millis(250 << attempt));
        if server_delay.is_some() && delay >= Duration::from_secs(1) {
            feedback::note(&format!("Retrying in {:.0}s…", delay.as_secs_f64().ceil()));
        }
        drop(result);
        tokio::time::sleep(delay).await;
        attempt += 1;
    }
}

fn retry_after(response: &Response) -> Option<Duration> {
    let value = response.headers().get("retry-after")?.to_str().ok()?;
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let date = httpdate::parse_http_date(value).ok()?;
    Some(date.duration_since(SystemTime::now()).unwrap_or_default())
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
