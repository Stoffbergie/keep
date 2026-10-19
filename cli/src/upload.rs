use crate::{clipboard, config, crypto, feedback, history, http};
use anyhow::{bail, Context, Result};
use bytes::{Bytes, BytesMut};
use futures_util::stream::{BoxStream, Fuse};
use futures_util::{stream, StreamExt, TryStreamExt};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use reqwest::{Body, Client, RequestBuilder, Response};
use serde::{Deserialize, Serialize};
use std::io::{self, IsTerminal, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::io::ReaderStream;

const PART_SIZE: u64 = 32 * 1024 * 1024;
const WHOLE_UPLOAD_LIMIT: u64 = 64 * 1024 * 1024;
const PARALLEL_PARTS: usize = 4;
const READ_BUFFER_SIZE: usize = 1024 * 1024;
const ENCRYPTED_FILENAME: &str = "encrypted";
pub const ONCE: &str = "once";

pub struct Lifetime {
    expires: Option<String>,
    once: bool,
}

impl Lifetime {
    pub fn new(expires: Option<String>, once: bool) -> Result<Self> {
        if expires.as_deref() == Some(ONCE) {
            return Ok(Self {
                expires: None,
                once: true,
            });
        }
        if let Some(value) = &expires {
            let valid = value.len() >= 2
                && value.len() <= 7
                && matches!(value.as_bytes().last(), Some(b'm' | b'h' | b'd'))
                && value.as_bytes()[0] != b'0'
                && value.as_bytes()[..value.len() - 1]
                    .iter()
                    .all(u8::is_ascii_digit);
            if !valid {
                bail!("invalid expiry {value:?}; use minutes, hours or days, like 30m, 12h or 7d; use --once for one download");
            }
        }
        Ok(Self { expires, once })
    }
}

#[derive(Clone)]
enum Source {
    File(PathBuf),
    Memory(Bytes),
    Chunks(Vec<Bytes>),
    Sealed(Arc<Source>, Arc<crypto::Sealer>),
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

#[derive(Serialize)]
struct Completion<'a> {
    parts: &'a [Part],
}

pub async fn upload(
    files: &[PathBuf],
    name: Option<String>,
    encrypt: bool,
    lifetime: &Lifetime,
    no_copy: bool,
) -> Result<()> {
    if files.len() > 1 && name.is_some() {
        bail!("--name needs a single file; multiple files keep their own names");
    }
    if files.len() > 1 && files.iter().any(|path| path == Path::new("-")) {
        bail!("use - on its own to share stdin; share files in a separate command");
    }
    if name.as_ref().is_some_and(|name| name.is_empty()) {
        bail!("--name needs a filename, like build.log");
    }
    let settings = config::load()?;
    let copy_links = !no_copy && settings.copy && std::io::stdout().is_terminal();
    let mut links = Vec::new();
    let explicit_stdin = files.first().is_some_and(|path| path == Path::new("-"));
    if explicit_stdin || (files.is_empty() && !std::io::stdin().is_terminal()) {
        let filename = name.unwrap_or_else(|| "stdin".into());
        let client = http::client()?;
        let link = upload_piped(&client, filename, encrypt, lifetime).await?;
        if copy_links {
            links.push(link);
        }
    } else {
        let inputs = if !files.is_empty() {
            files
                .iter()
                .map(|path| {
                    let filename = name.clone().unwrap_or(file_name(path)?);
                    Ok((Source::File(path.clone()), filename))
                })
                .collect::<Result<Vec<_>>>()?
        } else if settings.clipboard {
            clipboard_inputs(name, settings.preview && !encrypt)?
        } else {
            bail!("pass a file or pipe into keep; clipboard sharing is disabled (see keep config)");
        };
        for (source, _) in &inputs {
            if let Source::File(path) = source {
                let file = std::fs::File::open(path).with_context(|| {
                    format!(
                        "can't read {}; check its path and permissions",
                        path.display()
                    )
                })?;
                if !file.metadata()?.is_file() {
                    bail!(
                        "{} is not a regular file; archive directories first",
                        path.display()
                    );
                }
            }
        }
        let client = http::client()?;
        for (source, filename) in inputs {
            let result = upload_one(&client, source, filename.clone(), encrypt, lifetime).await;
            let link = result.with_context(|| format!("could not upload {filename}"))?;
            if copy_links {
                links.push(link);
            }
        }
    }
    if copy_links {
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
    lifetime: &Lifetime,
) -> Result<String> {
    let mut size = source.size()?;
    let progress = upload_progress(size)?;
    let mut key = None;
    let mut encrypted_name = None;
    if encrypt {
        let secret = crypto::new_key()?;
        let sealer = crypto::Sealer::new(&secret, size)?;
        size = sealer.sealed_size();
        source = Source::Sealed(Arc::new(source), Arc::new(sealer));
        encrypted_name = Some(crypto::encrypt_name(&filename, &secret)?);
        key = Some(secret);
    }
    let stored_filename = match encrypted_name {
        Some(_) => ENCRYPTED_FILENAME,
        None => &filename,
    };
    let encrypted_name = encrypted_name.as_deref();

    if progress.length().is_some() {
        progress.set_length(size);
    }
    progress.set_message("Uploading");
    let uploaded = if size <= WHOLE_UPLOAD_LIMIT {
        upload_whole(
            client,
            &source,
            size,
            stored_filename,
            encrypted_name,
            lifetime,
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
            lifetime,
            &progress,
        )
        .await?
    };
    progress.finish_and_clear();
    Ok(finish(uploaded, key, filename, size, lifetime))
}

async fn upload_piped(
    client: &Client,
    filename: String,
    encrypt: bool,
    lifetime: &Lifetime,
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
            lifetime,
            &progress,
        )
        .await?;
        progress.finish_and_clear();
        uploaded
    } else {
        let source = Source::Chunks(first);
        let size = source.size()?;
        let progress = upload_progress(size)?;
        progress.set_message("Uploading");
        let uploaded = upload_whole(
            client,
            &source,
            size,
            stored_filename,
            encrypted_name,
            lifetime,
            &progress,
        )
        .await?;
        progress.finish_and_clear();
        uploaded
    };
    Ok(finish(uploaded, key, filename, reader.size, lifetime))
}

fn finish(
    uploaded: Uploaded,
    key: Option<String>,
    filename: String,
    size: u64,
    lifetime: &Lifetime,
) -> String {
    let expires_at = uploaded.expires_at;
    let encrypted = key.is_some();
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
        once: lifetime.once,
    };
    if let Err(error) = history::add(&entry) {
        eprintln!("keep: uploaded, but could not save it for keep ls and keep rm: {error:#}");
    }
    if std::io::stderr().is_terminal() {
        let mut details = vec![feedback::preview(&entry.filename)];
        if encrypted {
            details.push("encrypted".into());
        }
        details.push(history::lifetime_status(&entry).into_owned());
        feedback::note(&details.join(" · "));
    }
    entry.link
}

fn upload_progress(size: u64) -> Result<ProgressBar> {
    if !std::io::stderr().is_terminal() {
        return Ok(ProgressBar::hidden());
    }
    let style = ProgressStyle::with_template(
        "{spinner} {msg:10} {bytes:>10}/{total_bytes:10} {bytes_per_sec:>12} {eta:>4} left [{bar:16}]",
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
    lifetime: &Lifetime,
    progress: &ProgressBar,
) -> Result<Uploaded> {
    let url = format!("{}/{}", http::base_url(), encode_segment(filename));
    let sent = progress.length().map(|_| Arc::new(AtomicU64::new(0)));
    let response = http::send(|| {
        let request = client
            .put(&url)
            .header("content-length", size)
            .body(source.body(0, size, progress, sent.clone()));
        with_details(request, encrypted_name, lifetime)
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
    lifetime: &Lifetime,
    progress: &ProgressBar,
) -> Result<Uploaded> {
    let base = http::base_url();
    let started = start_parts(client, &base, filename, encrypted_name, lifetime).await?;

    let part_size = match source {
        Source::Sealed(..) => {
            let chunks_per_part = PART_SIZE / crypto::CHUNK_SIZE as u64;
            PART_SIZE + crypto::HEADER_LEN as u64 + chunks_per_part * crypto::TAG_LEN as u64
        }
        _ => PART_SIZE,
    };
    let parts: Vec<Part> = stream::iter(1..=size.div_ceil(part_size))
        .map(|number| {
            let offset = (number - 1) * part_size;
            let length = part_size.min(size - offset);
            let sent = progress.length().map(|_| Arc::new(AtomicU64::new(0)));
            upload_part(client, &base, &started, number, length, move || {
                source.body(offset, length, progress, sent.clone())
            })
        })
        .buffer_unordered(PARALLEL_PARTS)
        .try_collect()
        .await?;
    if !progress.is_hidden() {
        progress.set_style(ProgressStyle::default_spinner());
        progress.set_message("Saving file");
    }
    complete_parts(client, &base, &started, parts).await
}

async fn upload_piped_parts(
    client: &Client,
    first: Vec<Bytes>,
    reader: &mut PartReader,
    filename: &str,
    encrypted_name: Option<&str>,
    lifetime: &Lifetime,
    progress: &ProgressBar,
) -> Result<Uploaded> {
    let base = http::base_url();
    let started = start_parts(client, &base, filename, encrypted_name, lifetime).await?;
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
    .map_ok(|(number, chunks): (u64, Vec<Bytes>)| {
        let length = chunks.iter().map(|chunk| chunk.len() as u64).sum();
        if !progress.is_hidden() {
            progress.inc(length);
        }
        let chunks = Arc::new(chunks);
        upload_part(client, &base, &started, number, length, move || {
            let chunks = chunks.clone();
            Body::wrap_stream(
                stream::iter(0..chunks.len())
                    .map(move |index| Ok::<_, io::Error>(chunks[index].clone())),
            )
        })
    })
    .try_buffer_unordered(PARALLEL_PARTS)
    .try_collect()
    .await?;
    if !progress.is_hidden() {
        progress.set_style(ProgressStyle::default_spinner());
        progress.set_message("Saving file");
    }
    complete_parts(client, &base, &started, parts).await
}

async fn start_parts(
    client: &Client,
    base: &str,
    filename: &str,
    encrypted_name: Option<&str>,
    lifetime: &Lifetime,
) -> Result<Started> {
    let started = http::send(|| {
        let request = client
            .post(format!("{base}/api/multipart"))
            .query(&[("filename", filename)]);
        with_details(request, encrypted_name, lifetime)
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
            .json(&Completion { parts: &parts })
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
    mut request: RequestBuilder,
    encrypted_name: Option<&str>,
    lifetime: &Lifetime,
) -> RequestBuilder {
    if let Some(encrypted_name) = encrypted_name {
        request = request
            .header("x-encrypted", "true")
            .header("x-encrypted-name", encrypted_name);
    }
    if let Some(expires) = lifetime.expires.as_deref() {
        request = request.header("x-expires", expires);
    }
    if lifetime.once {
        request = request.header("x-once", "true");
        if lifetime.expires.is_none() {
            request = request.header("x-expires", ONCE);
        }
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
    let mut url = response.text().await?;
    let length = url.trim().len();
    let start = url.len() - url.trim_start().len();
    if start > 0 {
        url.replace_range(..start, "");
    }
    url.truncate(length);
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
            Source::Chunks(chunks) => Ok(chunks.iter().map(|chunk| chunk.len() as u64).sum()),
            Source::Sealed(_, sealer) => Ok(sealer.sealed_size()),
        }
    }

    fn body(
        &self,
        offset: u64,
        length: u64,
        progress: &ProgressBar,
        sent: Option<Arc<AtomicU64>>,
    ) -> Body {
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
            Source::Chunks(chunks) => {
                let mut skip = offset as usize;
                let mut remaining = length as usize;
                let mut selected = Vec::new();
                for chunk in chunks {
                    let start = skip.min(chunk.len());
                    skip -= start;
                    let count = remaining.min(chunk.len() - start);
                    if count > 0 {
                        selected.push(chunk.slice(start..start + count));
                        remaining -= count;
                    }
                    if remaining == 0 {
                        break;
                    }
                }
                stream::iter(selected.into_iter().map(Ok::<_, io::Error>)).boxed()
            }
            Source::File(path) => {
                let path = path.clone();
                let chunks = stream::once(async move {
                    let file = tokio::task::spawn_blocking(move || {
                        let mut file = std::fs::File::open(path)?;
                        file.seek(SeekFrom::Start(offset))?;
                        Ok::<_, io::Error>(file)
                    })
                    .await??;
                    Ok::<_, io::Error>(stream::try_unfold(
                        (file, length, BytesMut::new()),
                        |(mut file, remaining, mut buffer)| async move {
                            if remaining == 0 {
                                return Ok(None);
                            }
                            let length = remaining.min(READ_BUFFER_SIZE as u64) as usize;
                            let (chunk, file, buffer) = tokio::task::spawn_blocking(move || {
                                buffer.resize(length, 0);
                                file.read_exact(&mut buffer)?;
                                let chunk = buffer.split().freeze();
                                Ok::<_, io::Error>((chunk, file, buffer))
                            })
                            .await??;
                            Ok(Some((chunk, (file, remaining - length as u64, buffer))))
                        },
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
                            let header_length = if index == 0 { crypto::HEADER_LEN } else { 0 };
                            let needed = end.saturating_sub(start + header_length as u64);
                            let plain_length = plain_length.min(needed);
                            let (sealed, reader, sealer) = tokio::task::spawn_blocking(move || {
                                let capacity =
                                    plain_length as usize + crypto::TAG_LEN + header_length;
                                let mut chunk = Vec::with_capacity(capacity);
                                chunk.resize(plain_length as usize + header_length, 0);
                                reader.read_exact(&mut chunk[header_length..])?;
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
        let Some(sent) = sent else {
            return Body::wrap_stream(chunks);
        };
        let progress = progress.clone();
        let mut position = 0;
        Body::wrap_stream(chunks.inspect_ok(move |chunk| {
            position += chunk.len() as u64;
            let previous = sent.fetch_max(position, Ordering::Relaxed);
            let advanced = position.saturating_sub(previous);
            if progress.position() + advanced == progress.length().unwrap_or(0) {
                progress.set_style(ProgressStyle::default_spinner());
                progress.set_message("Waiting for server");
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
            Source::Chunks(..) => unreachable!("piped input is encrypted before buffering"),
            Source::Sealed(..) => unreachable!("encrypted sources are never encrypted again"),
        }
    }
}

impl PartReader {
    async fn next_part(&mut self) -> Result<Vec<Bytes>> {
        let mut part = Vec::new();
        let mut length = 0;
        let mut chunk = std::mem::take(&mut self.pending);
        loop {
            let room = PART_SIZE as usize - length;
            if chunk.len() >= room {
                part.push(chunk.split_to(room));
                self.pending = chunk;
                length += room;
                break;
            }
            length += chunk.len();
            if !chunk.is_empty() {
                part.push(chunk);
            }
            match self.chunks.next().await {
                Some(next) => chunk = next?,
                None => break,
            }
        }
        self.size += length as u64;
        Ok(part)
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
    if !metadata.is_file() {
        bail!(
            "{} is not a regular file; pipe its contents into keep -",
            path.display()
        );
    }
    let name = path
        .file_name()
        .with_context(|| format!("{} has no file name", path.display()))?;
    Ok(name.to_string_lossy().into_owned())
}

fn encode_segment(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(HEX[(byte >> 4) as usize] as char);
            encoded.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    encoded
}
