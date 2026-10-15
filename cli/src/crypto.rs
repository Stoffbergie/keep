use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use bytes::Bytes;
use futures::{stream, Stream, StreamExt};
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};
use std::ops::Range;

const KEY_LEN: usize = 32;
const MAGIC: &[u8] = b"keep-v2\n";
const PREFIX_LEN: usize = 7;
const HEADER_LEN: usize = MAGIC.len() + PREFIX_LEN;
const TAG_LEN: usize = 16;
const CHUNK_SIZE: usize = 1024 * 1024;
const SEALED_CHUNK_SIZE: usize = CHUNK_SIZE + TAG_LEN;

pub struct Sealer {
    key: String,
    prefix: [u8; PREFIX_LEN],
    plain_size: u64,
}

struct Opening<S> {
    chunks: S,
    key: String,
    buffer: Vec<u8>,
    prefix: Option<[u8; PREFIX_LEN]>,
    index: u64,
    ended: bool,
    finished: bool,
}

pub fn new_key() -> Result<String> {
    let mut key = [0u8; KEY_LEN];
    SystemRandom::new()
        .fill(&mut key)
        .map_err(|_| anyhow!("could not generate a key"))?;
    Ok(URL_SAFE_NO_PAD.encode(key))
}

pub fn encrypt(mut sealed: Vec<u8>, key: &str) -> Result<Vec<u8>> {
    let mut nonce = [0u8; NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| anyhow!("could not generate a nonce"))?;

    let length = sealed.len();
    sealed.reserve_exact(NONCE_LEN + AES_256_GCM.tag_len());
    sealed.resize(length + NONCE_LEN, 0);
    sealed.copy_within(..length, NONCE_LEN);
    sealed[..NONCE_LEN].copy_from_slice(&nonce);
    let tag = cipher(key)?
        .seal_in_place_separate_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::empty(),
            &mut sealed[NONCE_LEN..],
        )
        .map_err(|_| anyhow!("encryption failed"))?;
    sealed.extend_from_slice(tag.as_ref());
    Ok(sealed)
}

pub fn decrypt(mut sealed: Vec<u8>, key: &str) -> Result<Bytes> {
    if sealed.len() < NONCE_LEN {
        bail!("this file is not encrypted");
    }
    let nonce = Nonce::try_assume_unique_for_key(&sealed[..NONCE_LEN])
        .map_err(|_| anyhow!("this file is not encrypted"))?;
    let length = cipher(key)?
        .open_in_place(nonce, Aad::empty(), &mut sealed[NONCE_LEN..])
        .map_err(|_| anyhow!("could not decrypt, check that the link is complete"))?
        .len();
    Ok(Bytes::from(sealed).slice(NONCE_LEN..NONCE_LEN + length))
}

pub fn encrypt_name(name: &str, key: &str) -> Result<String> {
    Ok(URL_SAFE_NO_PAD.encode(encrypt(name.as_bytes().to_vec(), key)?))
}

pub fn decrypt_name(sealed: &str, key: &str) -> Result<String> {
    let sealed = URL_SAFE_NO_PAD
        .decode(sealed)
        .map_err(|_| anyhow!("the stored filename is not encrypted"))?;
    Ok(String::from_utf8_lossy(&decrypt(sealed, key)?).into_owned())
}

pub fn decrypt_stream(
    chunks: impl Stream<Item = Result<Bytes>> + Unpin,
    key: &str,
) -> impl Stream<Item = Result<Bytes>> {
    let opening = Opening {
        chunks,
        key: key.to_string(),
        buffer: Vec::new(),
        prefix: None,
        index: 0,
        ended: false,
        finished: false,
    };
    stream::try_unfold(opening, |mut opening| async move {
        let plain = opening.next().await?;
        Ok(plain.map(|plain| (plain, opening)))
    })
}

impl Sealer {
    pub fn new(key: &str, plain_size: u64) -> Result<Self> {
        let mut prefix = [0u8; PREFIX_LEN];
        SystemRandom::new()
            .fill(&mut prefix)
            .map_err(|_| anyhow!("could not generate a nonce"))?;
        Ok(Sealer {
            key: key.to_string(),
            prefix,
            plain_size,
        })
    }

    pub fn sealed_size(&self) -> u64 {
        HEADER_LEN as u64 + self.plain_size + self.chunk_count() * TAG_LEN as u64
    }

    pub fn chunks_between(&self, start: u64, end: u64) -> Range<u64> {
        let first = start.saturating_sub(HEADER_LEN as u64) / SEALED_CHUNK_SIZE as u64;
        let last = (end - 1).saturating_sub(HEADER_LEN as u64) / SEALED_CHUNK_SIZE as u64;
        first..last + 1
    }

    pub fn plain_range(&self, index: u64) -> (u64, u64) {
        let offset = index * CHUNK_SIZE as u64;
        (offset, (CHUNK_SIZE as u64).min(self.plain_size - offset))
    }

    pub fn sealed_offset(&self, index: u64) -> u64 {
        match index {
            0 => 0,
            _ => HEADER_LEN as u64 + index * SEALED_CHUNK_SIZE as u64,
        }
    }

    pub fn seal(&self, index: u64, mut chunk: Vec<u8>) -> Result<Bytes> {
        let last = index + 1 == self.chunk_count();
        let tag = cipher(&self.key)?
            .seal_in_place_separate_tag(
                chunk_nonce(&self.prefix, index, last),
                Aad::empty(),
                &mut chunk,
            )
            .map_err(|_| anyhow!("encryption failed"))?;
        chunk.extend_from_slice(tag.as_ref());
        if index > 0 {
            return Ok(chunk.into());
        }
        let mut sealed = Vec::with_capacity(HEADER_LEN + chunk.len());
        sealed.extend_from_slice(MAGIC);
        sealed.extend_from_slice(&self.prefix);
        sealed.extend_from_slice(&chunk);
        Ok(sealed.into())
    }

    fn chunk_count(&self) -> u64 {
        self.plain_size.div_ceil(CHUNK_SIZE as u64).max(1)
    }
}

impl<S: Stream<Item = Result<Bytes>> + Unpin> Opening<S> {
    async fn next(&mut self) -> Result<Option<Bytes>> {
        if self.finished {
            return Ok(None);
        }
        let prefix = match self.prefix {
            Some(prefix) => prefix,
            None => {
                self.fill(HEADER_LEN).await?;
                if self.buffer.len() < HEADER_LEN || !self.buffer.starts_with(MAGIC) {
                    self.fill(usize::MAX).await?;
                    self.finished = true;
                    return decrypt(std::mem::take(&mut self.buffer), &self.key).map(Some);
                }
                let mut prefix = [0u8; PREFIX_LEN];
                prefix.copy_from_slice(&self.buffer[MAGIC.len()..HEADER_LEN]);
                self.buffer.drain(..HEADER_LEN);
                self.prefix = Some(prefix);
                prefix
            }
        };

        self.fill(SEALED_CHUNK_SIZE + 1).await?;
        let last = self.buffer.len() <= SEALED_CHUNK_SIZE;
        let length = self.buffer.len().min(SEALED_CHUNK_SIZE);
        let mut chunk: Vec<u8> = self.buffer.drain(..length).collect();
        let plain_length = cipher(&self.key)?
            .open_in_place(
                chunk_nonce(&prefix, self.index, last),
                Aad::empty(),
                &mut chunk,
            )
            .map_err(|_| anyhow!("could not decrypt, check that the link is complete"))?
            .len();
        chunk.truncate(plain_length);
        self.index += 1;
        self.finished = last;
        Ok(Some(chunk.into()))
    }

    async fn fill(&mut self, wanted: usize) -> Result<()> {
        while !self.ended && self.buffer.len() < wanted {
            match self.chunks.next().await {
                Some(chunk) => self.buffer.extend_from_slice(&chunk?),
                None => self.ended = true,
            }
        }
        Ok(())
    }
}

fn chunk_nonce(prefix: &[u8; PREFIX_LEN], index: u64, last: bool) -> Nonce {
    let mut nonce = [0u8; NONCE_LEN];
    nonce[..PREFIX_LEN].copy_from_slice(prefix);
    nonce[PREFIX_LEN..NONCE_LEN - 1].copy_from_slice(&(index as u32).to_be_bytes());
    nonce[NONCE_LEN - 1] = u8::from(last);
    Nonce::assume_unique_for_key(nonce)
}

fn cipher(key: &str) -> Result<LessSafeKey> {
    let key = URL_SAFE_NO_PAD
        .decode(key)
        .map_err(|_| anyhow!("the key in the link is not valid"))?;
    let key = UnboundKey::new(&AES_256_GCM, &key)
        .map_err(|_| anyhow!("the key in the link is not valid"))?;
    Ok(LessSafeKey::new(key))
}
