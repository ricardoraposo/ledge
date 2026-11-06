use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use crate::paths;

const MAGIC: &[u8] = b"KN1";
const NONCE_LEN: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyBackend {
    Keyring,
    File,
}

struct KeyringItem {
    schema: &'static str,
    app: &'static str,
}

const KEYRING_ITEM: KeyringItem = KeyringItem { schema: "io.github.ricardoraposo.Ledge.Key", app: "ledge" };
const LEGACY_KEYRING_ITEM: KeyringItem = KeyringItem { schema: "dev.keepnote.Key", app: "keepnote" };

fn attributes(item: &KeyringItem) -> HashMap<&'static str, &'static str> {
    HashMap::from([("app", item.app), ("purpose", "notes-key")])
}

fn schema(item: &KeyringItem) -> libsecret::Schema {
    libsecret::Schema::new(
        item.schema,
        libsecret::SchemaFlags::NONE,
        HashMap::from([
            ("app", libsecret::SchemaAttributeType::String),
            ("purpose", libsecret::SchemaAttributeType::String),
        ]),
    )
}

fn decode_key(text: &str) -> Result<Vec<u8>, String> {
    let key = STANDARD.decode(text.trim()).map_err(|error| error.to_string())?;
    if key.len() != 32 {
        return Err(format!("the stored key has {} bytes, not 32", key.len()));
    }
    Ok(key)
}

fn new_key() -> Vec<u8> {
    Aes256Gcm::generate_key(&mut OsRng).to_vec()
}

fn lookup(item: &KeyringItem) -> Result<Option<Vec<u8>>, String> {
    let stored = libsecret::password_lookup_sync(Some(&schema(item)), attributes(item), None::<&gtk::gio::Cancellable>)
        .map_err(|error| error.to_string())?;
    stored.map(|text| decode_key(&text)).transpose()
}

pub fn keyring_lookup() -> Result<Option<Vec<u8>>, String> {
    match lookup(&KEYRING_ITEM)? {
        Some(key) => Ok(Some(key)),
        None => lookup(&LEGACY_KEYRING_ITEM),
    }
}

fn key_file() -> std::path::PathBuf {
    paths::data_dir().join(".notes-key")
}

fn keyring_key(has_sealed_notes: bool) -> Result<(Vec<u8>, KeyBackend), String> {
    if let Some(key) = keyring_lookup()? {
        return Ok((key, KeyBackend::Keyring));
    }
    if key_file().exists() {
        return file_key(false).map(|key| (key, KeyBackend::File));
    }
    if has_sealed_notes {
        return Err(MISSING_KEY.into());
    }
    let schema = schema(&KEYRING_ITEM);
    let key = new_key();
    libsecret::password_store_sync(
        Some(&schema),
        attributes(&KEYRING_ITEM),
        Some("default"),
        "Ledge notes key",
        &STANDARD.encode(&key),
        None::<&gtk::gio::Cancellable>,
    )
    .map_err(|error| error.to_string())?;
    Ok((key, KeyBackend::Keyring))
}

const MISSING_KEY: &str = "your notes are encrypted, but their key is not in the keyring or in the key file. \
    Ledge stops here instead of making a new key that could not open them. Unlock the keyring, or restore \
    the .notes-key file, then start Ledge again.";

fn file_key(has_sealed_notes: bool) -> Result<Vec<u8>, String> {
    let path = key_file();
    if path.exists() {
        let text = fs::read_to_string(&path).map_err(|error| error.to_string())?;
        return decode_key(&text);
    }
    if has_sealed_notes {
        return Err(MISSING_KEY.into());
    }
    let key = new_key();
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|error| error.to_string())?;
    file.write_all(STANDARD.encode(&key).as_bytes()).map_err(|error| error.to_string())?;
    Ok(key)
}

pub fn load_key(has_sealed_notes: bool) -> Result<(Vec<u8>, KeyBackend), String> {
    let wants_file = std::env::var("LEDGE_KEY_BACKEND").is_ok_and(|value| value == "file");
    if paths::isolated() || wants_file {
        return file_key(has_sealed_notes).map(|key| (key, KeyBackend::File));
    }
    match keyring_key(has_sealed_notes) {
        Ok(found) => Ok(found),
        Err(error) if error == MISSING_KEY => Err(error),
        Err(error) => {
            eprintln!("ledge: keyring unavailable ({error}); using a key file");
            file_key(has_sealed_notes).map(|key| (key, KeyBackend::File))
        }
    }
}

pub struct Sealer {
    cipher: Aes256Gcm,
}

impl Sealer {
    pub fn new(key: &[u8]) -> Self {
        Sealer { cipher: Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key)) }
    }

    pub fn seal(&self, plaintext: &[u8], context: &str) -> Vec<u8> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let sealed = self
            .cipher
            .encrypt(&nonce, Payload { msg: plaintext, aad: context.as_bytes() })
            .expect("AES-GCM encryption");
        let mut out = Vec::with_capacity(MAGIC.len() + NONCE_LEN + sealed.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&sealed);
        out
    }

    pub fn open(&self, blob: &[u8], context: &str) -> Result<Vec<u8>, String> {
        if blob.is_empty() {
            return Ok(Vec::new());
        }
        if !blob.starts_with(MAGIC) {
            return Ok(blob.to_vec());
        }
        if blob.len() < MAGIC.len() + NONCE_LEN {
            return Err("sealed data is too short".into());
        }
        let nonce = Nonce::from_slice(&blob[MAGIC.len()..MAGIC.len() + NONCE_LEN]);
        self.cipher
            .decrypt(nonce, Payload { msg: &blob[MAGIC.len() + NONCE_LEN..], aad: context.as_bytes() })
            .map_err(|_| "this data was sealed with another key".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn keyring_probe() {
        match keyring_lookup() {
            Ok(Some(_)) => println!("keyring: Ledge key found"),
            Ok(None) => println!("keyring: no Ledge key"),
            Err(error) => println!("keyring: error {error}"),
        }
    }

    #[test]
    fn seal_round_trip_and_context() {
        let sealer = Sealer::new(&new_key());
        let blob = sealer.seal(b"secret words", "ID");
        assert!(blob.starts_with(MAGIC));
        assert_eq!(sealer.open(&blob, "ID").unwrap(), b"secret words");
        assert!(sealer.open(&blob, "OTHER").is_err());
        assert_eq!(sealer.open(b"plain", "ID").unwrap(), b"plain");
    }
}
