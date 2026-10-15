use crate::http;
use anyhow::{Context, Result};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub link: String,
    pub filename: String,
    pub size: u64,
    pub delete_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

pub fn add(entry: &Entry) -> Result<()> {
    let path = history_path()?;
    let mut file = open_private(&path, OpenOptions::new().append(true))?;
    writeln!(file, "{}", serde_json::to_string(entry)?)?;
    Ok(())
}

pub fn list() -> Result<()> {
    let entries = load()?;
    let width = entries
        .iter()
        .map(|entry| entry.filename.chars().count())
        .max()
        .unwrap_or(0)
        .min(40);
    let stdout = std::io::stdout();
    let mut output = BufWriter::new(stdout.lock());
    for entry in entries {
        let status = match entry.expires_at.as_deref() {
            Some(expires_at) => expiry_status(expires_at),
            None => String::new(),
        };
        let line = format!(
            "{:>9}  {:<width$}  {}  {status}",
            format_size(entry.size),
            entry.filename,
            entry.link
        );
        writeln!(output, "{}", line.trim_end())?;
    }
    output.flush()?;
    Ok(())
}

pub async fn remove(input: &str) -> Result<()> {
    let link = http::parse_link(input);
    let mut entries = load()?;
    let index = entries
        .iter()
        .position(|entry| http::parse_link(&entry.link).id == link.id)
        .with_context(|| format!("{} was not uploaded from this machine", link.id))?;

    let client = http::client()?;
    let token = entries[index].delete_token.clone();
    let response = http::send_unchecked(|| client.delete(&link.url).bearer_auth(&token)).await?;
    if response.status() != StatusCode::NOT_FOUND {
        http::ensure_success(response).await?;
    }

    entries.remove(index);
    save(&entries)
}

fn load() -> Result<Vec<Entry>> {
    let text = match std::fs::read_to_string(history_path()?) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let entries = text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    Ok(entries)
}

fn save(entries: &[Entry]) -> Result<()> {
    let path = history_path()?;
    let partial = path.with_extension("jsonl.part");
    let mut file = open_private(&partial, OpenOptions::new().write(true).truncate(true))?;
    for entry in entries {
        writeln!(file, "{}", serde_json::to_string(entry)?)?;
    }
    file.sync_all()?;
    std::fs::rename(&partial, &path)?;
    Ok(())
}

fn history_path() -> Result<PathBuf> {
    let directory = dirs::config_dir()
        .context("could not find a config directory")?
        .join("keep");
    std::fs::create_dir_all(&directory)?;
    Ok(directory.join("uploads.jsonl"))
}

fn open_private(path: &Path, options: &mut OpenOptions) -> Result<File> {
    options.create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

fn expiry_status(expires_at: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default();
    match unix_seconds(expires_at) {
        Some(expires) if expires <= now => "expired".into(),
        Some(_) => format!(
            "expires {} UTC",
            expires_at.get(..16).unwrap_or(expires_at).replace('T', " ")
        ),
        None => String::new(),
    }
}

fn unix_seconds(iso: &str) -> Option<i64> {
    let field = |range: std::ops::Range<usize>| iso.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (field(0..4)?, field(5..7)?, field(8..10)?);
    let (hour, minute, second) = (field(11..13)?, field(14..16)?, field(17..19)?);
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

fn format_size(bytes: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        return format!("{bytes} B");
    }
    format!("{value:.1} {}", units[unit])
}
