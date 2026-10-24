//! Whisper models: a pinned catalog, downloads checked against it, and the
//! folder they're kept in. Nothing is downloaded until someone asks: from
//! Koe's window or `aizome-koe model download`.
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

#[derive(Deserialize)]
pub struct Catalog {
    pub repository: String,
    pub revision: String,
    pub models: Vec<Model>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Model {
    pub name: String,
    pub sha256: String,
    pub bytes: u64,
}
impl Model {
    pub fn file_name(&self) -> String {
        format!("ggml-{}.bin", self.name)
    }
    /// "Base", "Base (English)".
    pub fn title(&self) -> String {
        let (size, english) = match self.name.strip_suffix(".en") {
            Some(size) => (size, true),
            None => (self.name.as_str(), false),
        };
        let mut chars = size.chars();
        let size = chars.next().map(|c| c.to_uppercase().chain(chars).collect::<String>()).unwrap_or_default();
        if english { format!("{size} (English)") } else { size }
    }
    /// What it's for, as the window says it.
    pub fn description(&self) -> String {
        let languages = if self.name.ends_with(".en") { "English only" } else { "Many languages" };
        let speed = match self.name.trim_end_matches(".en") {
            "tiny" => "fastest, least accurate",
            "base" => "fast and accurate enough for most dictation",
            _ => "more accurate, slower and larger",
        };
        format!("{languages} · {} · {speed}", size_text(self.bytes))
    }
}

/// A size as people read it: "141 MB".
pub fn size_text(bytes: u64) -> String {
    let mb = bytes as f64 / 1_000_000.0;
    if mb >= 1000.0 { format!("{:.1} GB", mb / 1000.0) } else { format!("{mb:.0} MB") }
}

pub fn catalog() -> Catalog {
    serde_json::from_str(include_str!("../data/models.json")).expect("bundled catalog is valid")
}
pub fn find(name: &str) -> Result<Model> {
    catalog()
        .models
        .into_iter()
        .find(|m| m.name == name)
        .with_context(|| format!("no model {name}; aizome-koe model list names them"))
}
/// The catalog model a path is, if it's one.
pub fn named(path: &Path) -> Option<Model> {
    let file = path.file_name()?.to_str()?;
    catalog().models.into_iter().find(|m| m.file_name() == file)
}
pub fn directory() -> Result<PathBuf> {
    Ok(crate::config::data_dir()?.join("models"))
}
/// Where the catalog's files are downloaded from.
pub fn source() -> String {
    let catalog = catalog();
    format!("https://huggingface.co/{}/resolve/{}", catalog.repository, catalog.revision)
}

/// What's in the folder for a model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Installed {
    Missing,
    /// The file is there at the catalog's size (`verify` reads it all).
    Present,
    /// A file of another size: a copy that isn't the catalog's.
    Damaged,
}
pub fn installed(dir: &Path, model: &Model) -> Installed {
    match std::fs::metadata(dir.join(model.file_name())) {
        Ok(meta) if meta.is_file() && meta.len() == model.bytes => Installed::Present,
        Ok(_) => Installed::Damaged,
        Err(_) => Installed::Missing,
    }
}

pub fn verify(path: &Path, model: &Model) -> Result<()> {
    let mut file = std::fs::File::open(path)?;
    ensure!(file.metadata()?.len() == model.bytes, "model size does not match the catalog");
    let mut hasher = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        let n = file.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        hasher.update(&bytes[..n]);
    }
    ensure!(format!("{:x}", hasher.finalize()) == model.sha256, "model SHA-256 does not match the catalog");
    Ok(())
}

/// Bytes free for this user on the filesystem that holds `dir` (or would).
pub fn free_bytes(dir: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let existing = dir.ancestors().find(|p| p.exists())?;
    let path = std::ffi::CString::new(existing.as_os_str().as_bytes()).ok()?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: path is NUL-terminated and stat is written by statvfs before
    // it is read; nothing is read when the call fails.
    let stat = unsafe {
        if libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) != 0 {
            return None;
        }
        stat.assume_init()
    };
    Some(stat.f_bavail as u64 * stat.f_frsize as u64)
}

/// Delete a downloaded model.
pub fn remove(dir: &Path, model: &Model) -> Result<()> {
    match std::fs::remove_file(dir.join(model.file_name())) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).context("cannot remove the model"),
    }
}

pub async fn download(name: &str, dir: &Path) -> Result<PathBuf> {
    eprintln!("Downloading {name}, pinned SHA-256 from the catalog");
    download_from(&source(), &find(name)?, dir, &AtomicBool::new(false), |_| {}).await
}

/// Download `model` from `base` into `dir`, checking its size and SHA-256
/// against the catalog before it takes the model's name. `progress` hears
/// the bytes so far; setting `cancel` stops it, leaving nothing behind.
/// Only HTTPS, or plain HTTP to this computer (tests).
pub async fn download_from(
    base: &str,
    model: &Model,
    dir: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let dest = dir.join(model.file_name());
    if dest.exists() {
        verify(&dest, model).context("existing model failed verification; remove it to download again")?;
        return Ok(dest);
    }
    if let Some(free) = free_bytes(dir) {
        // Room for the model and a little more, so the disk isn't left full.
        let needed = model.bytes + 64 * 1024 * 1024;
        ensure!(
            free >= needed,
            "not enough free space: {} needs {}, and {} is free",
            model.title(),
            size_text(needed),
            size_text(free)
        );
    }
    let url = reqwest::Url::parse(&format!("{base}/{}", model.file_name())).context("invalid model source")?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    ensure!(url.scheme() == "https" || (url.scheme() == "http" && local), "models are only downloaded over HTTPS");
    let client = reqwest::Client::builder()
        // Redirects to the file's CDN stay on HTTPS too.
        .https_only(!local)
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(60))
        .build()?;
    let mut response = client
        .get(url)
        .send()
        .await
        .context("cannot reach the model server")?
        .error_for_status()
        .context("the model server refused the download")?;
    let mut file =
        tempfile::Builder::new().prefix(&format!(".{}.", model.file_name())).suffix(".part").tempfile_in(dir)?;
    let mut hash = Sha256::new();
    let mut count = 0_u64;
    progress(0);
    while let Some(chunk) = response.chunk().await.context("the download was interrupted")? {
        if cancel.load(Ordering::Relaxed) {
            bail!("download cancelled");
        }
        count += chunk.len() as u64;
        ensure!(count <= model.bytes, "model download exceeded expected size");
        file.write_all(&chunk)?;
        hash.update(&chunk);
        progress(count);
    }
    ensure!(
        count == model.bytes && format!("{:x}", hash.finalize()) == model.sha256,
        "download failed size or SHA-256 verification"
    );
    file.as_file().sync_all()?;
    file.persist_noclobber(&dest).context("cannot install downloaded model; another download may have completed")?;
    Ok(dest)
}
