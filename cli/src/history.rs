use crate::http;
use anyhow::{Context, Result};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub link: String,
    pub filename: String,
    pub size: u64,
    pub delete_token: String,
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
    for entry in entries {
        println!(
            "{:>9}  {:<width$}  {}",
            format_size(entry.size),
            entry.filename,
            entry.link
        );
    }
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
