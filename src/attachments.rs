use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::LazyLock;

use regex::Regex;
use sha2::{Digest, Sha256};

use crate::paths;
use crate::store::Store;

pub const MAX_BYTES: usize = 25 * 1024 * 1024;

static REFERENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"attachment:([0-9a-f]{64})").unwrap());

pub fn ids_in(body: &str) -> BTreeSet<String> {
    REFERENCE.captures_iter(body).map(|caps| caps[1].to_string()).collect()
}

pub fn sha256_hex(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

pub fn find_in(dir: &Path, digest: &str) -> Option<PathBuf> {
    let prefix = format!("{digest}.");
    fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.file_name().and_then(|n| n.to_str()).is_some_and(|name| name.starts_with(&prefix)))
}

pub fn extension_of(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| !e.is_empty())
        .map(str::to_lowercase)
        .unwrap_or_else(|| "bin".into())
}

pub struct Attachments {
    store: Rc<Store>,
    dir: PathBuf,
}

impl Attachments {
    pub fn new(store: Rc<Store>) -> Self {
        Attachments { store, dir: paths::attachments_dir() }
    }

    pub fn add_bytes(&self, data: &[u8], name: &str) -> Option<String> {
        if data.len() > MAX_BYTES {
            return None;
        }
        let digest = sha256_hex(data);
        let target = self.dir.join(format!("{digest}.{}", extension_of(name)));
        if !target.exists() && fs::write(&target, self.store.sealer.seal(data, &digest)).is_err() {
            return None;
        }
        Some(format!("attachment:{digest}"))
    }


    pub fn find(&self, digest: &str) -> Option<PathBuf> {
        find_in(&self.dir, digest)
    }

    pub fn data(&self, digest: &str) -> Option<Vec<u8>> {
        let path = self.find(digest)?;
        let blob = fs::read(path).ok()?;
        self.store.sealer.open(&blob, digest).ok()
    }

    pub fn extension(&self, digest: &str) -> Option<String> {
        let path = self.find(digest)?;
        path.extension().and_then(|e| e.to_str()).map(str::to_string)
    }

    pub fn open_path(&self, digest: &str) -> Option<PathBuf> {
        let data = self.data(digest)?;
        let extension = self.extension(digest).unwrap_or_else(|| "bin".into());
        let target = paths::runtime_dir().join(format!("{digest}.{extension}"));
        let mut file = OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&target).ok()?;
        file.write_all(&data).ok()?;
        Some(target)
    }
}
