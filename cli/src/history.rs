use crate::{feedback, http, upload};
use anyhow::{Context, Result};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::fmt::Write as _;
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
    let _lock = lock_history()?;
    let path = history_path()?;
    let mut file = open_private(&path, OpenOptions::new().append(true))?;
    let mut line = serde_json::to_string(entry)?;
    line.push('\n');
    file.write_all(line.as_bytes())?;
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
    let mut size = String::new();
    for entry in entries {
        size.clear();
        format_size(entry.size, &mut size)?;
        let status = lifetime_status(&entry);
        let status = status.trim_end();
        let encrypted = http::link_key(&entry.link).is_some();
        let prefix = match (encrypted, status.is_empty()) {
            (true, true) => "encrypted ·",
            (true, false) => "encrypted · ",
            (false, _) => "",
        };
        if status.is_empty() && !encrypted {
            let line = format!("{:>9}  {:<width$}  {}", size, entry.filename, entry.link);
            writeln!(output, "{}", line.trim_end())?;
        } else {
            writeln!(
                output,
                "{:>9}  {:<width$}  {}  {prefix}{status}",
                size, entry.filename, entry.link
            )?;
        }
    }
    output.flush()?;
    Ok(())
}

pub async fn remove(inputs: &[String]) -> Result<()> {
    let mut entries = load()?;
    let mut links = Vec::with_capacity(inputs.len());
    for input in inputs {
        let link = http::parse_link(input);
        let full_url = input.contains("://");
        let mut matches = entries
            .iter()
            .filter(|entry| http::link_id(&entry.link) == link.id)
            .filter(|entry| !full_url || http::parse_link(&entry.link).url == link.url);
        let Some(entry) = matches.next() else {
            anyhow::bail!("{} was not uploaded from this machine; delete it from the machine that uploaded it", link.id);
        };
        let destination = http::parse_link(&entry.link).url;
        if !full_url && matches.any(|other| http::parse_link(&other.link).url != destination) {
            anyhow::bail!(
                "{} matches uploads on more than one server; use the complete link",
                link.id
            );
        }
        links.push(entry.link.clone());
    }
    let client = http::client()?;
    for stored_link in links {
        let Some(index) = entries.iter().position(|entry| entry.link == stored_link) else {
            continue;
        };
        let link = http::parse_link(&stored_link);
        let token = &entries[index].delete_token;
        let response = http::send_unchecked(|| client.delete(link.url.as_ref()).bearer_auth(token))
            .await
            .with_context(|| format!("could not delete {}", link.id))?;
        if response.status() != StatusCode::NOT_FOUND {
            http::ensure_success(response)
                .await
                .with_context(|| format!("could not delete {}", link.id))?;
        }
        {
            let _lock = lock_history()?;
            let mut current_entries = load()?;
            current_entries
                .retain(|entry| entry.link != entries[index].link || entry.delete_token != *token);
            save(&current_entries)?;
        }
        entries.remove(index);
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

fn lock_history() -> Result<File> {
    let path = history_path()?.with_extension("lock");
    let file = open_private(&path, OpenOptions::new().read(true).write(true))?;
    file.lock()?;
    Ok(file)
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

pub fn lifetime_status(entry: &Entry) -> Cow<'static, str> {
    let once = entry.once || entry.expires_at.as_deref() == Some(upload::ONCE);
    let expiry = entry
        .expires_at
        .as_deref()
        .filter(|value| *value != upload::ONCE)
        .map(expiry_status);
    match (once, expiry) {
        (_, Some(expiry)) if expiry == "expired" => expiry,
        (true, Some(expiry)) => format!("one download or {expiry}, whichever comes first").into(),
        (true, None) => "until the first download".into(),
        (false, Some(expiry)) => expiry,
        (false, None) => "until you delete it".into(),
    }
}

fn expiry_status(expires_at: &str) -> Cow<'static, str> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default();
    match unix_seconds(expires_at) {
        Some(expires) if expires <= now => "expired".into(),
        Some(_) => format!(
            "expires {} UTC",
            expires_at.get(..16).unwrap_or(expires_at).replace('T', " ")
        )
        .into(),
        None => "".into(),
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

fn format_size(bytes: u64, output: &mut String) -> std::fmt::Result {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        return write!(output, "{bytes} B");
    }
    write!(output, "{value:.1} {}", units[unit])
}
