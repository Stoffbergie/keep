use crate::{feedback, http, upload};
use anyhow::{Context, Result};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, ErrorKind, Write};
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
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub once: bool,
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
        let mut status = lifetime_status(&entry);
        if http::link_key(&entry.link).is_some() {
            status = format!("encrypted · {status}");
        }
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

pub async fn remove(inputs: &[String]) -> Result<()> {
    let links: Vec<_> = inputs.iter().map(|input| http::parse_link(input)).collect();
    let mut entries = load()?;
    for link in &links {
        if !entries
            .iter()
            .any(|entry| http::link_id(&entry.link) == link.id)
        {
            anyhow::bail!("{} was not uploaded from this machine; delete it from the machine that uploaded it", link.id);
        }
    }
    let client = http::client()?;
    for link in links {
        let Some(index) = entries
            .iter()
            .position(|entry| http::link_id(&entry.link) == link.id)
        else {
            continue;
        };
        let token = &entries[index].delete_token;
        let response = http::send_unchecked(|| client.delete(&link.url).bearer_auth(token))
            .await
            .with_context(|| format!("could not delete {}", link.id))?;
        if response.status() != StatusCode::NOT_FOUND {
            http::ensure_success(response)
                .await
                .with_context(|| format!("could not delete {}", link.id))?;
        }
        entries.remove(index);
        save(&entries)?;
        feedback::note(&format!("Deleted {}", link.id));
    }
    Ok(())
}

fn load() -> Result<Vec<Entry>> {
    let file = match File::open(history_path()?) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut entries = Vec::new();
    while reader.read_line(&mut line)? > 0 {
        if let Ok(entry) = serde_json::from_str(&line) {
            entries.push(entry);
        }
        line.clear();
    }
    Ok(entries)
}

fn save(entries: &[Entry]) -> Result<()> {
    let path = history_path()?;
    let partial = path.with_extension("jsonl.part");
    let mut file = BufWriter::new(open_private(
        &partial,
        OpenOptions::new().write(true).truncate(true),
    )?);
    for entry in entries {
        serde_json::to_writer(&mut file, entry)?;
        file.write_all(b"\n")?;
    }
    file.flush()?;
    file.get_ref().sync_all()?;
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

pub fn lifetime_status(entry: &Entry) -> String {
    let once = entry.once || entry.expires_at.as_deref() == Some(upload::ONCE);
    let expiry = entry
        .expires_at
        .as_deref()
        .filter(|value| *value != upload::ONCE)
        .map(expiry_status);
    match (once, expiry) {
        (_, Some(expiry)) if expiry == "expired" => expiry,
        (true, Some(expiry)) => format!("one download or {expiry}, whichever comes first"),
        (true, None) => "until the first download".into(),
        (false, Some(expiry)) => expiry,
        (false, None) => "until you delete it".into(),
    }
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
