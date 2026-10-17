use crate::{crypto, http};
use anyhow::{bail, Result};
use bytes::Bytes;
use futures::{stream, Stream, StreamExt, TryStreamExt};
use indicatif::ProgressBar;
use reqwest::header::CONTENT_DISPOSITION;
use reqwest::{Client, Response, StatusCode};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncWrite, AsyncWriteExt, BufWriter};

const WRITE_BUFFER_SIZE: usize = 256 * 1024;

enum Target {
    Stdout,
    File(PathBuf),
}

pub async fn get(input: &str, output: Option<&str>) -> Result<()> {
    let link = http::parse_link(input);
    let client = http::client()?;
    let response = http::send(|| client.get(&link.url)).await?;
    let mut filename = attachment_name(&response).unwrap_or_else(|| link.id.clone());
    if let (Some(key), Some(encrypted_name)) = (&link.key, encrypted_name(&response)) {
        let name = crypto::decrypt_name(encrypted_name, key)?;
        filename = file_name_only(&name).unwrap_or(filename);
    }
    let target = target(output, &filename)?;

    let progress = http::progress(response.content_length().unwrap_or(0))?;
    match link.key {
        Some(key) => {
            let chunks = response.bytes_stream().map_err(anyhow::Error::from);
            save(target, crypto::decrypt_stream(chunks, &key), progress).await
        }
        None => save(target, resumed_chunks(client, link.url, response), progress).await,
    }
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
        (response.bytes_stream(), 0u64, 0u32),
        move |(mut chunks, mut received, mut retries)| {
            let client = client.clone();
            let url = url.clone();
            let etag = etag.clone();
            async move {
                loop {
                    match chunks.next().await {
                        Some(Ok(chunk)) => {
                            received += chunk.len() as u64;
                            return Ok(Some((chunk, (chunks, received, retries))));
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
                            tokio::time::sleep(std::time::Duration::from_millis(250 << retries))
                                .await;
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
            }
        },
    )
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
    chunks: impl Stream<Item = Result<Bytes>>,
    progress: ProgressBar,
) -> Result<()> {
    match target {
        Target::Stdout => {
            let mut stdout = BufWriter::with_capacity(WRITE_BUFFER_SIZE, tokio::io::stdout());
            copy(chunks, &mut stdout, &progress).await?;
            stdout.flush().await?;
        }
        Target::File(path) => {
            let partial = partial_path(&path);
            let file = tokio::fs::File::create(&partial).await?;
            let mut file = BufWriter::with_capacity(WRITE_BUFFER_SIZE, file);
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
