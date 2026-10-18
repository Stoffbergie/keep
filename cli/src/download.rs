use crate::{crypto, http};
use anyhow::{bail, Context, Result};
use bytes::Bytes;
use futures::{stream, Stream, StreamExt, TryStreamExt};
use indicatif::ProgressBar;
use reqwest::header::CONTENT_DISPOSITION;
use reqwest::{Client, Response, StatusCode};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncWrite, AsyncWriteExt, BufWriter};

const WRITE_BUFFER_SIZE: usize = 256 * 1024;

enum Target {
    Stdout,
    File {
        path: PathBuf,
        partial: PathBuf,
        file: tokio::fs::File,
    },
}

pub async fn get(input: &str, output: Option<&str>) -> Result<()> {
    let link = http::parse_link(input);
    let client = http::client()?;
    let metadata = http::send_unchecked(|| client.head(&link.url)).await?;
    if metadata.status() == StatusCode::NOT_FOUND {
        bail!("Not found; check the link, or ask the uploader if it expired or was already downloaded");
    }
    let metadata = http::ensure_success(metadata).await?;
    let mut filename = attachment_name(&metadata).unwrap_or_else(|| link.id.clone());
    let encrypted = metadata
        .headers()
        .get("x-encrypted")
        .is_some_and(|value| value == "true")
        || encrypted_name(&metadata).is_some();
    if encrypted && link.key.is_none() {
        bail!("this file is encrypted; use the complete link, including #key");
    }
    let key = match &link.key {
        Some(key) => {
            let key =
                crypto::cipher(key).context("invalid key; use the complete encrypted link")?;
            if let Some(encrypted_name) = encrypted_name(&metadata) {
                let name = crypto::decrypt_name(encrypted_name, &key)?;
                filename = file_name_only(&name).unwrap_or(filename);
            }
            Some(key)
        }
        None => None,
    };
    let target = target(output, &filename).await?;
    let partial = match &target {
        Target::Stdout => None,
        Target::File { partial, .. } => Some(partial.clone()),
    };
    let size = metadata
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let progress = http::progress(size)?;
    let result = async {
        let response = http::send(|| client.get(&link.url)).await?;
        match key {
            Some(key) => {
                let chunks = response.bytes_stream().map_err(anyhow::Error::from);
                save(
                    target,
                    crypto::decrypt_stream(chunks, key),
                    progress.clone(),
                )
                .await
            }
            None => {
                save(
                    target,
                    resumed_chunks(client, link.url, response),
                    progress.clone(),
                )
                .await
            }
        }
    }
    .await;
    progress.finish_and_clear();
    if result.is_err() {
        if let Some(partial) = partial {
            let _ = tokio::fs::remove_file(partial).await;
        }
    }
    result
}

fn resumed_chunks(
    client: Client,
    url: String,
    response: Response,
) -> impl Stream<Item = Result<Bytes>> {
    let total = response.content_length();
    let etag = response
        .headers()
        .get("etag")
        .filter(|value| value.as_bytes().starts_with(b"\""))
        .cloned();
    let can_resume = response.status() == StatusCode::OK && etag.is_some();
    stream::try_unfold(
        (response.bytes_stream(), 0u64, 0u32, client, url, etag),
        move |(mut chunks, mut received, mut retries, client, url, etag)| async move {
            loop {
                match chunks.next().await {
                    Some(Ok(chunk)) => {
                        received += chunk.len() as u64;
                        return Ok(Some((
                            chunk,
                            (chunks, received, retries, client, url, etag),
                        )));
                    }
                    None => return Ok(None),
                    Some(Err(error)) => {
                        let Some(total) = total else {
                            return Err(error.into());
                        };
                        if !can_resume || retries == 4 || received >= total {
                            return Err(error.into());
                        }
                        retries += 1;
                        tokio::time::sleep(std::time::Duration::from_millis(250 << retries)).await;
                        let response = http::send(|| {
                            client
                                .get(&url)
                                .header("range", format!("bytes={received}-"))
                        })
                        .await?;
                        let expected = format!("bytes {received}-{}/{total}", total - 1);
                        if response.status() != StatusCode::PARTIAL_CONTENT
                            || response.headers().get("etag") != etag.as_ref()
                            || response
                                .headers()
                                .get("content-range")
                                .and_then(|v| v.to_str().ok())
                                != Some(expected.as_str())
                            || response.content_length() != Some(total - received)
                        {
                            bail!("could not resume: the file or download range changed");
                        }
                        chunks = response.bytes_stream();
                    }
                }
            }
        },
    )
}

async fn target(output: Option<&str>, filename: &str) -> Result<Target> {
    let path = match output {
        Some("-") => return Ok(Target::Stdout),
        Some(path) => {
            let mut path = PathBuf::from(path);
            if path.is_dir() {
                path.push(filename);
            }
            if path.exists() {
                bail!(
                    "{} already exists; choose another path with -o",
                    path.display()
                );
            }
            path
        }
        None => unused_path(filename),
    };
    let partial = partial_path(&path);
    let file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .await
        .with_context(|| {
            format!(
                "can't save {}; choose a writable directory or another path with -o",
                path.display()
            )
        })?;
    Ok(Target::File {
        path,
        partial,
        file,
    })
}

async fn save(
    target: Target,
    chunks: impl Stream<Item = Result<Bytes>>,
    progress: ProgressBar,
) -> Result<()> {
    match target {
        Target::Stdout => {
            let mut stdout = BufWriter::with_capacity(WRITE_BUFFER_SIZE, tokio::io::stdout());
            copy(chunks, &mut stdout, &progress).await?;
            stdout.flush().await?;
        }
        Target::File {
            path,
            partial,
            file,
        } => {
            let mut file = BufWriter::with_capacity(WRITE_BUFFER_SIZE, file);
            copy(chunks, &mut file, &progress).await?;
            file.flush().await?;
            drop(file);
            tokio::fs::rename(&partial, &path).await?;
            println!("{}", path.display());
        }
    }
    progress.finish_and_clear();
    Ok(())
}

async fn copy(
    chunks: impl Stream<Item = Result<Bytes>>,
    writer: &mut (impl AsyncWrite + Unpin),
    progress: &ProgressBar,
) -> Result<()> {
    let mut chunks = std::pin::pin!(chunks);
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
    file_name_only(&percent_decode(encoded.split(';').next()?.trim()))
}

fn encrypted_name(response: &Response) -> Option<&str> {
    response.headers().get("x-encrypted-name")?.to_str().ok()
}

fn file_name_only(name: &str) -> Option<String> {
    let name = Path::new(name).file_name()?.to_string_lossy().into_owned();
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
