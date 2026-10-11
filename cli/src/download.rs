use crate::{crypto, http};
use anyhow::{bail, Result};
use bytes::Bytes;
use futures::{stream, Stream, StreamExt};
use indicatif::ProgressBar;
use reqwest::header::CONTENT_DISPOSITION;
use reqwest::Response;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncWrite, AsyncWriteExt};

enum Target {
    Stdout,
    File(PathBuf),
}

pub async fn get(input: &str, output: Option<&str>) -> Result<()> {
    let link = http::parse_link(input);
    let client = http::client()?;
    let response = http::send(|| client.get(&link.url)).await?;
    let filename = attachment_name(&response).unwrap_or_else(|| link.id.clone());
    let target = target(output, &filename)?;

    match link.key {
        Some(key) => {
            let sealed = response.bytes().await?.to_vec();
            let plain = Bytes::from(crypto::decrypt(sealed, &key)?);
            save(target, stream::iter([Ok(plain)]), ProgressBar::hidden()).await
        }
        None => {
            let progress = http::progress(response.content_length().unwrap_or(0))?;
            save(target, response.bytes_stream(), progress).await
        }
    }
}

fn target(output: Option<&str>, filename: &str) -> Result<Target> {
    match output {
        Some("-") => Ok(Target::Stdout),
        Some(path) => {
            let mut path = PathBuf::from(path);
            if path.is_dir() {
                path.push(filename);
            }
            if path.exists() {
                bail!("{} already exists", path.display());
            }
            Ok(Target::File(path))
        }
        None if std::io::stdout().is_terminal() => Ok(Target::File(unused_path(filename))),
        None => Ok(Target::Stdout),
    }
}

async fn save(
    target: Target,
    chunks: impl Stream<Item = reqwest::Result<Bytes>> + Unpin,
    progress: ProgressBar,
) -> Result<()> {
    match target {
        Target::Stdout => {
            let mut stdout = tokio::io::stdout();
            copy(chunks, &mut stdout, &progress).await?;
            stdout.flush().await?;
        }
        Target::File(path) => {
            let partial = partial_path(&path);
            let mut file = tokio::fs::File::create(&partial).await?;
            let copied = copy(chunks, &mut file, &progress).await;
            if copied.is_ok() {
                file.flush().await?;
                drop(file);
                tokio::fs::rename(&partial, &path).await?;
            } else {
                drop(file);
                let _ = tokio::fs::remove_file(&partial).await;
            }
            copied?;
            println!("{}", path.display());
        }
    }
    progress.finish_and_clear();
    Ok(())
}

async fn copy(
    mut chunks: impl Stream<Item = reqwest::Result<Bytes>> + Unpin,
    writer: &mut (impl AsyncWrite + Unpin),
    progress: &ProgressBar,
) -> Result<()> {
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk?;
        writer.write_all(&chunk).await?;
        progress.inc(chunk.len() as u64);
    }
    Ok(())
}

fn attachment_name(response: &Response) -> Option<String> {
    let header = response.headers().get(CONTENT_DISPOSITION)?.to_str().ok()?;
    let encoded = header.split("filename*=UTF-8''").nth(1)?;
    let decoded = percent_decode(encoded.split(';').next()?.trim());
    let name = Path::new(&decoded)
        .file_name()?
        .to_string_lossy()
        .into_owned();
    Some(name)
}

fn unused_path(filename: &str) -> PathBuf {
    let original = PathBuf::from(filename);
    let stem = original
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let extension = original
        .extension()
        .map(|extension| format!(".{}", extension.to_string_lossy()))
        .unwrap_or_default();
    let mut candidate = original.clone();
    let mut counter = 1;
    while candidate.exists() {
        candidate = PathBuf::from(format!("{stem} ({counter}){extension}"));
        counter += 1;
    }
    candidate
}

fn partial_path(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!(".{name}.part"))
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escaped = value
            .get(index + 1..index + 3)
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match (bytes[index], escaped) {
            (b'%', Some(byte)) => {
                decoded.push(byte);
                index += 3;
            }
            (byte, _) => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}
