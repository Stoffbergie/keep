use crate::{crypto, history, http};
use anyhow::{bail, Context, Result};
use bytes::Bytes;
use futures::{stream, StreamExt, TryStreamExt};
use indicatif::ProgressBar;
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};
use std::io::{IsTerminal, Read, SeekFrom};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

const PART_SIZE: u64 = 32 * 1024 * 1024;
const PARALLEL_PARTS: usize = 4;

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
    let mut source = match file {
        Some(path) => Source::File(path.to_path_buf()),
        None => Source::Memory(read_stdin()?),
    };
    let filename = match (name, file) {
        (Some(name), _) => name,
        (None, Some(path)) => file_name(path)?,
        (None, None) => "stdin".to_string(),
    };

    let mut key = None;
    if encrypt {
        let (sealed, secret) = crypto::encrypt(source.read_all().await?)?;
        source = Source::Memory(Bytes::from(sealed));
        key = Some(secret);
    }

    let size = source.size()?;
    let client = http::client()?;
    let uploaded = if size <= PART_SIZE {
        upload_whole(&client, &source, size, &filename, encrypt).await?
    } else {
        upload_parts(&client, &source, size, &filename, encrypt).await?
    };

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
    Ok(())
}

async fn upload_whole(
    client: &Client,
    source: &Source,
    size: u64,
    filename: &str,
    encrypted: bool,
) -> Result<Uploaded> {
    let body = source.read(0, size).await?;
    let url = format!("{}/{}", http::base_url(), encode_segment(filename));
    let response = http::send(|| {
        client
            .put(&url)
            .header("x-encrypted", encrypted.to_string())
            .body(body.clone())
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

    let progress = http::progress(size)?;
    let mut parts: Vec<Part> = stream::iter(1..=size.div_ceil(PART_SIZE))
        .map(|number| upload_part(client, source, size, &base, &started, number, &progress))
        .buffer_unordered(PARALLEL_PARTS)
        .try_collect()
        .await?;
    progress.finish_and_clear();
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
    let body = source.read(offset, length).await?;
    let url = format!("{base}/api/multipart/{}/{number}", started.id);
    let part = http::send(|| {
        client
            .put(&url)
            .header("x-upload-id", &started.upload_id)
            .body(body.clone())
    })
    .await?
    .json()
    .await?;
    progress.inc(length);
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

    async fn read(&self, offset: u64, length: u64) -> Result<Bytes> {
        match self {
            Source::Memory(bytes) => Ok(bytes.slice(offset as usize..(offset + length) as usize)),
            Source::File(path) => {
                let mut file = tokio::fs::File::open(path).await?;
                file.seek(SeekFrom::Start(offset)).await?;
                let mut buffer = vec![0; length as usize];
                file.read_exact(&mut buffer).await?;
                Ok(Bytes::from(buffer))
            }
        }
    }

    async fn read_all(&self) -> Result<Vec<u8>> {
        match self {
            Source::File(path) => Ok(tokio::fs::read(path).await?),
            Source::Memory(bytes) => Ok(bytes.to_vec()),
        }
    }
}

fn read_stdin() -> Result<Bytes> {
    let mut stdin = std::io::stdin();
    if stdin.is_terminal() {
        bail!("nothing to upload, pass a file or pipe into keep (see keep --help)");
    }
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
