use crate::{clipboard, config, crypto, feedback, history, http};
use anyhow::{bail, Context, Result};
use bytes::Bytes;
use futures::{stream, StreamExt, TryStreamExt};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use reqwest::{Body, Client, Response};
use serde::{Deserialize, Serialize};
use std::io::{IsTerminal, Read, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

const PART_SIZE: u64 = 32 * 1024 * 1024;
const PARALLEL_PARTS: usize = 4;
const READ_BUFFER_SIZE: usize = 1024 * 1024;

enum Source {
    File(PathBuf),
    Memory(Bytes),
}

struct Uploaded {
    url: String,
    delete_token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Started {
    id: String,
    upload_id: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Part {
    part_number: u64,
    etag: String,
}

pub async fn upload(file: Option<&Path>, name: Option<String>, encrypt: bool) -> Result<()> {
    let settings = config::load()?;
    let inputs = match file {
        Some(path) => vec![(
            Source::File(path.to_path_buf()),
            name.unwrap_or(file_name(path)?),
        )],
        None if !std::io::stdin().is_terminal() => {
            vec![(
                Source::Memory(read_stdin()?),
                name.unwrap_or_else(|| "stdin".into()),
            )]
        }
        None if settings.clipboard => clipboard_inputs(name, settings.preview && !encrypt)?,
        None => {
            bail!("pass a file or pipe into keep; clipboard sharing is disabled (see keep config)")
        }
    };
    let client = http::client()?;
    let mut links = Vec::new();
    for (source, filename) in inputs {
        links.push(upload_one(&client, source, filename, encrypt).await?);
    }
    if settings.copy && std::io::stdout().is_terminal() {
        match clipboard::copy(&links.join("\n")).await {
            Ok(()) => feedback::note("Copied to clipboard!"),
            Err(_) => feedback::note("Link ready. Clipboard unavailable; copy the link above."),
        }
    }
    Ok(())
}

fn clipboard_inputs(name: Option<String>, preview: bool) -> Result<Vec<(Source, String)>> {
    match clipboard::read()? {
        clipboard::Contents::Files(paths) => {
            if paths.len() > 1 && name.is_some() {
                bail!("--name needs a single file; copied files keep their own names");
            }
            let mut inputs = Vec::with_capacity(paths.len());
            for path in paths {
                let filename = name.clone().unwrap_or(file_name(&path)?);
                inputs.push((Source::File(path), filename));
            }
            let kind = if inputs.len() == 1 { "file" } else { "files" };
            feedback::note(&format!("Clipboard · {} {kind}", inputs.len()));
            if preview {
                for (_, filename) in inputs.iter().take(5) {
                    feedback::note(&feedback::preview(filename));
                }
                if inputs.len() > 5 {
                    feedback::note("…");
                }
            }
            Ok(inputs)
        }
        clipboard::Contents::Image { png, width, height } => {
            let filename = name.unwrap_or_else(|| "clipboard.png".into());
            feedback::note(&format!("Clipboard · image · {width} × {height}"));
            Ok(vec![(Source::Memory(png.into()), filename)])
        }
        clipboard::Contents::Text(text) => {
            let filename = name.unwrap_or_else(|| "clipboard.txt".into());
            feedback::note("Clipboard · text");
            if preview {
                feedback::note(&feedback::preview(&text));
            }
            Ok(vec![(Source::Memory(text.into()), filename)])
        }
    }
}

async fn upload_one(
    client: &Client,
    mut source: Source,
    filename: String,
    encrypt: bool,
) -> Result<String> {
    let progress = upload_progress(source.size()?)?;
    let mut key = None;
    if encrypt {
        progress.set_message("Encrypting");
        let (sealed, secret) = crypto::encrypt(source.read_all().await?)?;
        source = Source::Memory(Bytes::from(sealed));
        key = Some(secret);
    }

    let size = source.size()?;
    progress.set_length(size);
    progress.set_message("Uploading");
    let uploaded = if size <= PART_SIZE {
        upload_whole(client, &source, size, &filename, encrypt, &progress).await?
    } else {
        upload_parts(client, &source, size, &filename, encrypt, &progress).await?
    };
    progress.finish_and_clear();

    let link = match key {
        Some(key) => format!("{}#{}", uploaded.url, key),
        None => uploaded.url,
    };
    println!("{link}");
    let entry = history::Entry {
        link,
        filename,
        size,
        delete_token: uploaded.delete_token,
    };
    if let Err(error) = history::add(&entry) {
        eprintln!("keep: uploaded, but could not save it for keep ls and keep rm: {error:#}");
    }
    Ok(entry.link)
}

fn upload_progress(size: u64) -> Result<ProgressBar> {
    if !std::io::stderr().is_terminal() {
        return Ok(ProgressBar::hidden());
    }
    let style = ProgressStyle::with_template(
        "{spinner} {msg:10} {bytes:>10}/{total_bytes:10} {bytes_per_sec:>12} [{bar:16}]",
    )?
    .progress_chars("█▉▊▋▌▍▎▏ ");
    let progress = ProgressBar::with_draw_target(Some(size), ProgressDrawTarget::hidden())
        .with_style(style)
        .with_message("Preparing");
    if size >= PART_SIZE {
        show_upload_progress(&progress);
    } else {
        let display = progress.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if !display.is_finished() {
                show_upload_progress(&display);
            }
        });
    }
    Ok(progress)
}

fn show_upload_progress(progress: &ProgressBar) {
    progress.set_draw_target(ProgressDrawTarget::stderr());
    progress.tick();
    progress.enable_steady_tick(Duration::from_millis(80));
}

async fn upload_whole(
    client: &Client,
    source: &Source,
    size: u64,
    filename: &str,
    encrypted: bool,
    progress: &ProgressBar,
) -> Result<Uploaded> {
    let url = format!("{}/{}", http::base_url(), encode_segment(filename));
    let sent = Arc::new(AtomicU64::new(0));
    let response = http::send(|| {
        client
            .put(&url)
            .header("x-encrypted", encrypted.to_string())
            .header("content-length", size)
            .body(source.body(0, size, progress, sent.clone()))
    })
    .await?;
    uploaded(response).await
}

async fn upload_parts(
    client: &Client,
    source: &Source,
    size: u64,
    filename: &str,
    encrypted: bool,
    progress: &ProgressBar,
) -> Result<Uploaded> {
    let base = http::base_url();
    let started: Started = http::send(|| {
        client
            .post(format!("{base}/api/multipart"))
            .query(&[("filename", filename)])
            .header("x-encrypted", encrypted.to_string())
    })
    .await?
    .json()
    .await?;

    let mut parts: Vec<Part> = stream::iter(1..=size.div_ceil(PART_SIZE))
        .map(|number| upload_part(client, source, size, &base, &started, number, progress))
        .buffer_unordered(PARALLEL_PARTS)
        .try_collect()
        .await?;
    parts.sort_by_key(|part| part.part_number);

    let response = http::send(|| {
        client
            .post(format!("{base}/api/multipart/{}", started.id))
            .header("x-upload-id", &started.upload_id)
            .json(&serde_json::json!({ "parts": parts }))
    })
    .await?;
    uploaded(response).await
}

async fn upload_part(
    client: &Client,
    source: &Source,
    size: u64,
    base: &str,
    started: &Started,
    number: u64,
    progress: &ProgressBar,
) -> Result<Part> {
    let offset = (number - 1) * PART_SIZE;
    let length = PART_SIZE.min(size - offset);
    let url = format!("{base}/api/multipart/{}/{number}", started.id);
    let sent = Arc::new(AtomicU64::new(0));
    let part = http::send(|| {
        client
            .put(&url)
            .header("x-upload-id", &started.upload_id)
            .header("content-length", length)
            .body(source.body(offset, length, progress, sent.clone()))
    })
    .await?
    .json()
    .await?;
    Ok(part)
}

async fn uploaded(response: Response) -> Result<Uploaded> {
    let delete_token = response
        .headers()
        .get("x-delete-token")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let url = response.text().await?.trim().to_string();
    Ok(Uploaded { url, delete_token })
}

impl Source {
    fn size(&self) -> Result<u64> {
        match self {
            Source::File(path) => {
                let metadata = std::fs::metadata(path)
                    .with_context(|| format!("can't read {}", path.display()))?;
                Ok(metadata.len())
            }
            Source::Memory(bytes) => Ok(bytes.len() as u64),
        }
    }

    fn body(&self, offset: u64, length: u64, progress: &ProgressBar, sent: Arc<AtomicU64>) -> Body {
        let chunks = match self {
            Source::Memory(bytes) => {
                let bytes = bytes.slice(offset as usize..(offset + length) as usize);
                if progress.length().is_none() {
                    return bytes.into();
                }
                stream::unfold(bytes, |mut bytes| async move {
                    if bytes.is_empty() {
                        return None;
                    }
                    let chunk = bytes.split_to(READ_BUFFER_SIZE.min(bytes.len()));
                    Some((Ok::<_, std::io::Error>(chunk), bytes))
                })
                .boxed()
            }
            Source::File(path) => {
                let path = path.clone();
                let chunks = stream::once(async move {
                    let mut file = tokio::fs::File::open(path).await?;
                    file.seek(SeekFrom::Start(offset)).await?;
                    file.set_max_buf_size(READ_BUFFER_SIZE);
                    Ok::<_, std::io::Error>(ReaderStream::with_capacity(
                        file.take(length),
                        READ_BUFFER_SIZE,
                    ))
                })
                .try_flatten();
                chunks.boxed()
            }
        };
        if progress.length().is_none() {
            return Body::wrap_stream(chunks);
        }
        let progress = progress.clone();
        let mut position = 0;
        Body::wrap_stream(chunks.inspect_ok(move |chunk| {
            position += chunk.len() as u64;
            let previous = sent.fetch_max(position, Ordering::Relaxed);
            let advanced = position.saturating_sub(previous);
            if progress.position() + advanced == progress.length().unwrap_or(0) {
                progress.set_style(ProgressStyle::default_spinner());
                progress.set_message("Finalizing upload");
            }
            progress.inc(advanced);
        }))
    }

    async fn read_all(self) -> Result<Vec<u8>> {
        match self {
            Source::File(path) => Ok(tokio::fs::read(path).await?),
            Source::Memory(bytes) => Ok(bytes.into()),
        }
    }
}

fn read_stdin() -> Result<Bytes> {
    let mut stdin = std::io::stdin();
    let mut buffer = Vec::new();
    stdin.read_to_end(&mut buffer)?;
    Ok(Bytes::from(buffer))
}

fn file_name(path: &Path) -> Result<String> {
    let metadata =
        std::fs::metadata(path).with_context(|| format!("can't read {}", path.display()))?;
    if metadata.is_dir() {
        bail!("{} is a directory, archive it first", path.display());
    }
    let name = path
        .file_name()
        .with_context(|| format!("{} has no file name", path.display()))?;
    Ok(name.to_string_lossy().into_owned())
}

fn encode_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}
