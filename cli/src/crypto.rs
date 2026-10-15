use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use bytes::Bytes;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};

const KEY_LEN: usize = 32;

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

fn cipher(key: &str) -> Result<LessSafeKey> {
    let key = URL_SAFE_NO_PAD
        .decode(key)
        .map_err(|_| anyhow!("the key in the link is not valid"))?;
    let key = UnboundKey::new(&AES_256_GCM, &key)
        .map_err(|_| anyhow!("the key in the link is not valid"))?;
    Ok(LessSafeKey::new(key))
}
