use crate::{clipboard, config, crypto, feedback, history, http};
use anyhow::{bail, Context, Result};
use bytes::Bytes;
use futures::stream::{BoxStream, Fuse};
use futures::{stream, StreamExt, TryStreamExt};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use reqwest::{Body, Client, RequestBuilder, Response};
use serde::{Deserialize, Serialize};
use std::io::{self, IsTerminal, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

const PART_SIZE: u64 = 32 * 1024 * 1024;
const PARALLEL_PARTS: usize = 4;
const READ_BUFFER_SIZE: usize = 1024 * 1024;
const ENCRYPTED_FILENAME: &str = "encrypted";
pub const ONCE: &str = "once";

#[derive(Clone)]
enum Source {
    File(PathBuf),
    Memory(Bytes),
    Sealed(Box<Source>, Arc<crypto::Sealer>),
}

struct PartReader {
    chunks: Fuse<BoxStream<'static, io::Result<Bytes>>>,
    pending: Bytes,
    size: u64,
}

struct Uploaded {
    url: String,
    delete_token: String,
    expires_at: Option<String>,
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

pub async fn upload(
    file: Option<&Path>,
    name: Option<String>,
    encrypt: bool,
    expires: Option<&str>,
) -> Result<()> {
    let settings = config::load()?;
    let client = http::client()?;
    let mut links = Vec::new();
    if file.is_none() && !std::io::stdin().is_terminal() {
        let filename = name.unwrap_or_else(|| "stdin".into());
        links.push(upload_piped(&client, filename, encrypt, expires).await?);
    } else {
        let inputs = match file {
            Some(path) => vec![(
                Source::File(path.to_path_buf()),
                name.unwrap_or(file_name(path)?),
            )],
            None if settings.clipboard => clipboard_inputs(name, settings.preview && !encrypt)?,
            None => {
                bail!("pass a file or pipe into keep; clipboard sharing is disabled (see keep config)")
            }
        };
        for (source, filename) in inputs {
            links.push(upload_one(&client, source, filename, encrypt, expires).await?);
        }
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
    expires: Option<&str>,
) -> Result<String> {
    let progress = upload_progress(source.size()?)?;
    let mut key = None;
    let mut encrypted_name = None;
    if encrypt {
        let secret = crypto::new_key()?;
        let sealer = crypto::Sealer::new(&secret, source.size()?)?;
        source = Source::Sealed(Box::new(source), Arc::new(sealer));
        encrypted_name = Some(crypto::encrypt_name(&filename, &secret)?);
        key = Some(secret);
    }
    let stored_filename = match encrypted_name {
        Some(_) => ENCRYPTED_FILENAME,
        None => &filename,
    };
    let encrypted_name = encrypted_name.as_deref();

    let size = source.size()?;
    progress.set_length(size);
    progress.set_message("Uploading");
    let uploaded = if size <= PART_SIZE {
        upload_whole(
            client,
            &source,
            size,
            stored_filename,
            encrypted_name,
            expires,
            &progress,
        )
        .await?
    } else {
        upload_parts(
            client,
            &source,
            size,
            stored_filename,
            encrypted_name,
            expires,
            &progress,
        )
        .await?
    };
    progress.finish_and_clear();
    Ok(finish(uploaded, key, filename, size, expires))
}

async fn upload_piped(
    client: &Client,
    filename: String,
    encrypt: bool,
    expires: Option<&str>,
) -> Result<String> {
    let stdin = ReaderStream::with_capacity(tokio::io::stdin(), READ_BUFFER_SIZE);
    let mut key = None;
    let mut encrypted_name = None;
    let chunks = match encrypt {
        true => {
            let secret = crypto::new_key()?;
            encrypted_name = Some(crypto::encrypt_name(&filename, &secret)?);
            let sealed = crypto::encrypt_stream(stdin, &secret)?;
            key = Some(secret);
            sealed.boxed()
        }
        false => stdin.boxed(),
    };
    let stored_filename = match encrypted_name {
        Some(_) => ENCRYPTED_FILENAME,
        None => &filename,
    };
    let encrypted_name = encrypted_name.as_deref();

    let mut reader = PartReader {
        chunks: chunks.fuse(),
        pending: Bytes::new(),
        size: 0,
    };
    let first = reader.next_part().await?;
    let uploaded = if reader.has_more().await? {
        let progress = piped_progress()?;
        let uploaded = upload_piped_parts(
            client,
            first,
            &mut reader,
            stored_filename,
            encrypted_name,
            expires,
            &progress,
        )
        .await?;
        progress.finish_and_clear();
        uploaded
    } else {
        let size = first.len() as u64;
        let progress = upload_progress(size)?;
        progress.set_message("Uploading");
        let source = Source::Memory(first);
        let uploaded = upload_whole(
            client,
            &source,
            size,
            stored_filename,
            encrypted_name,
            expires,
            &progress,
        )
        .await?;
        progress.finish_and_clear();
        uploaded
    };
    Ok(finish(uploaded, key, filename, reader.size, expires))
}

fn finish(
    uploaded: Uploaded,
    key: Option<String>,
    filename: String,
    size: u64,
    expires: Option<&str>,
) -> String {
    let mut expires_at = uploaded.expires_at;
    if expires == Some(ONCE) {
        expires_at = Some(ONCE.into());
    }
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
        expires_at,
    };
    if let Err(error) = history::add(&entry) {
        eprintln!("keep: uploaded, but could not save it for keep ls and keep rm: {error:#}");
    }
    entry.link
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

fn piped_progress() -> Result<ProgressBar> {
    if !std::io::stderr().is_terminal() {
        return Ok(ProgressBar::hidden());
    }
    let style = ProgressStyle::with_template("{spinner} {msg:10} {bytes:>10} {bytes_per_sec:>12}")?;
    let progress = ProgressBar::with_draw_target(None, ProgressDrawTarget::hidden())
        .with_style(style)
        .with_message("Uploading");
    show_upload_progress(&progress);
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
    encrypted_name: Option<&str>,
    expires: Option<&str>,
    progress: &ProgressBar,
) -> Result<Uploaded> {
    let url = format!("{}/{}", http::base_url(), encode_segment(filename));
    let sent = Arc::new(AtomicU64::new(0));
    let response = http::send(|| {
        let request = client
            .put(&url)
            .header("content-length", size)
            .body(source.body(0, size, progress, sent.clone()));
        with_details(request, encrypted_name, expires)
    })
    .await?;
    uploaded(response).await
}

async fn upload_parts(
    client: &Client,
    source: &Source,
    size: u64,
    filename: &str,
    encrypted_name: Option<&str>,
    expires: Option<&str>,
    progress: &ProgressBar,
) -> Result<Uploaded> {
    let base = http::base_url();
    let started = start_parts(client, &base, filename, encrypted_name, expires).await?;

    let parts: Vec<Part> = stream::iter(1..=size.div_ceil(PART_SIZE))
        .map(|number| {
            let offset = (number - 1) * PART_SIZE;
            let length = PART_SIZE.min(size - offset);
            let sent = Arc::new(AtomicU64::new(0));
            upload_part(client, &base, &started, number, length, move || {
                source.body(offset, length, progress, sent.clone())
            })
        })
        .buffer_unordered(PARALLEL_PARTS)
        .try_collect()
        .await?;
    complete_parts(client, &base, &started, parts).await
}

async fn upload_piped_parts(
    client: &Client,
    first: Bytes,
    reader: &mut PartReader,
    filename: &str,
    encrypted_name: Option<&str>,
    expires: Option<&str>,
    progress: &ProgressBar,
) -> Result<Uploaded> {
    let base = http::base_url();
    let started = start_parts(client, &base, filename, encrypted_name, expires).await?;
    let parts: Vec<Part> = stream::try_unfold(
        (reader, Some(first), 0),
        |(reader, first, number)| async move {
            let part = match first {
                Some(part) => part,
                None => reader.next_part().await?,
            };
            if part.is_empty() {
                return Ok(None);
            }
            Ok(Some(((number + 1, part), (reader, None, number + 1))))
        },
    )
    .map_ok(|(number, part): (u64, Bytes)| {
        progress.inc(part.len() as u64);
        let length = part.len() as u64;
        upload_part(client, &base, &started, number, length, move || {
            Body::from(part.clone())
        })
    })
    .try_buffer_unordered(PARALLEL_PARTS)
    .try_collect()
    .await?;
    complete_parts(client, &base, &started, parts).await
}

async fn start_parts(
    client: &Client,
    base: &str,
    filename: &str,
    encrypted_name: Option<&str>,
    expires: Option<&str>,
) -> Result<Started> {
    let started = http::send(|| {
        let request = client
            .post(format!("{base}/api/multipart"))
            .query(&[("filename", filename)]);
        with_details(request, encrypted_name, expires)
    })
    .await?
    .json()
    .await?;
    Ok(started)
}

async fn complete_parts(
    client: &Client,
    base: &str,
    started: &Started,
    mut parts: Vec<Part>,
) -> Result<Uploaded> {
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
    base: &str,
    started: &Started,
    number: u64,
    length: u64,
    body: impl Fn() -> Body,
) -> Result<Part> {
    let url = format!("{base}/api/multipart/{}/{number}", started.id);
    let part = http::send(|| {
        client
            .put(&url)
            .header("x-upload-id", &started.upload_id)
            .header("content-length", length)
            .body(body())
    })
    .await?
    .json()
    .await?;
    Ok(part)
}

fn with_details(
    request: RequestBuilder,
    encrypted_name: Option<&str>,
    expires: Option<&str>,
) -> RequestBuilder {
    let mut request = request.header("x-encrypted", encrypted_name.is_some().to_string());
    if let Some(encrypted_name) = encrypted_name {
        request = request.header("x-encrypted-name", encrypted_name);
    }
    if let Some(expires) = expires {
        request = request.header("x-expires", expires);
    }
    request
}

async fn uploaded(response: Response) -> Result<Uploaded> {
    let header = |name: &str| {
        let value = response.headers().get(name)?.to_str().ok()?;
        Some(value.to_string())
    };
    let delete_token = header("x-delete-token").unwrap_or_default();
    let expires_at = header("x-expires-at");
    let url = response.text().await?.trim().to_string();
    Ok(Uploaded {
        url,
        delete_token,
        expires_at,
    })
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
            Source::Sealed(_, sealer) => Ok(sealer.sealed_size()),
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
            Source::Sealed(plain, sealer) => {
                let plain = plain.clone();
                let sealer = sealer.clone();
                let end = offset + length;
                let indices = sealer.chunks_between(offset, end);
                stream::once(async move {
                    let (plain_offset, _) = sealer.plain_range(indices.start);
                    let reader =
                        tokio::task::spawn_blocking(move || plain.reader(plain_offset)).await??;
                    Ok::<_, io::Error>(stream::try_unfold(
                        (reader, indices, sealer),
                        move |(mut reader, mut indices, sealer)| async move {
                            let Some(index) = indices.next() else {
                                return Ok(None);
                            };
                            let (_, plain_length) = sealer.plain_range(index);
                            let start = sealer.sealed_offset(index);
                            let (sealed, reader, sealer) = tokio::task::spawn_blocking(move || {
                                let mut chunk = vec![0; plain_length as usize];
                                reader.read_exact(&mut chunk)?;
                                let sealed = sealer.seal(index, chunk).map_err(io::Error::other)?;
                                Ok::<_, io::Error>((sealed, reader, sealer))
                            })
                            .await??;
                            let from = offset.saturating_sub(start) as usize;
                            let to = (end - start).min(sealed.len() as u64) as usize;
                            Ok(Some((sealed.slice(from..to), (reader, indices, sealer))))
                        },
                    ))
                })
                .try_flatten()
                .boxed()
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

    fn reader(&self, offset: u64) -> io::Result<Box<dyn Read + Send>> {
        match self {
            Source::File(path) => {
                let mut file = std::fs::File::open(path)?;
                file.seek(SeekFrom::Start(offset))?;
                Ok(Box::new(file))
            }
            Source::Memory(bytes) => Ok(Box::new(io::Cursor::new(bytes.slice(offset as usize..)))),
            Source::Sealed(..) => unreachable!("encrypted sources are never encrypted again"),
        }
    }
}

impl PartReader {
    async fn next_part(&mut self) -> Result<Bytes> {
        let mut part = Vec::with_capacity(PART_SIZE as usize);
        let mut chunk = std::mem::take(&mut self.pending);
        loop {
            let room = PART_SIZE as usize - part.len();
            if chunk.len() >= room {
                part.extend_from_slice(&chunk[..room]);
                self.pending = chunk.slice(room..);
                break;
            }
            part.extend_from_slice(&chunk);
            match self.chunks.next().await {
                Some(next) => chunk = next?,
                None => break,
            }
        }
        self.size += part.len() as u64;
        Ok(Bytes::from(part))
    }

    async fn has_more(&mut self) -> Result<bool> {
        if self.pending.is_empty() {
            if let Some(chunk) = self.chunks.next().await {
                self.pending = chunk?;
            }
        }
        Ok(!self.pending.is_empty())
    }
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
