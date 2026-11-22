use crate::error::AppError;
use axum::http::StatusCode;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
use tokio::{
    fs,
    sync::{Mutex as AsyncMutex, OwnedMutexGuard},
};

const CACHE_VERSION: &str = "v2";

struct Entry {
    path: PathBuf,
    size: u64,
    last_used: u64,
}

#[derive(Default)]
struct CacheState {
    entries: HashMap<String, Entry>,
    total: u64,
}

/// 有界磁盘分片缓存。缓存目录不可用时由调用方禁用，不影响播放。
pub struct TranscodeCache {
    root: PathBuf,
    max_bytes: u64,
    state: Mutex<CacheState>,
    flights: Mutex<HashMap<String, Weak<AsyncMutex<()>>>>,
    sequence: AtomicU64,
    available: AtomicBool,
}

impl TranscodeCache {
    pub async fn open(root: PathBuf, max_bytes: u64) -> Result<Self, AppError> {
        if max_bytes == 0 {
            return Err(AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Transcode cache size must be greater than zero",
            ));
        }
        fs::create_dir_all(&root).await?;
        let mut entries = HashMap::new();
        let mut total = 0u64;
        let mut sequence = 0u64;
        let mut directory = fs::read_dir(&root).await?;
        while let Some(item) = directory.next_entry().await? {
            let path = item.path();
            let metadata = match item.metadata().await {
                Ok(metadata) if metadata.is_file() => metadata,
                _ => continue,
            };
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if name.ends_with(".tmp") {
                let _ = fs::remove_file(&path).await;
                continue;
            }
            let Some(key) = name.strip_suffix(".ts") else {
                continue;
            };
            if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                continue;
            }
            sequence += 1;
            total = total.saturating_add(metadata.len());
            entries.insert(
                key.to_owned(),
                Entry {
                    path,
                    size: metadata.len(),
                    last_used: sequence,
                },
            );
        }
        let cache = Self {
            root,
            max_bytes,
            state: Mutex::new(CacheState { entries, total }),
            flights: Mutex::default(),
            sequence: AtomicU64::new(sequence),
            available: AtomicBool::new(true),
        };
        cache.evict().await;
        Ok(cache)
    }

    pub fn key(path: &str, revision: u64, profile: &str, index: u32) -> String {
        let mut digest = Sha256::new();
        digest.update(CACHE_VERSION.as_bytes());
        digest.update([0]);
        digest.update(path.as_bytes());
        digest.update(revision.to_le_bytes());
        digest.update([0]);
        digest.update(profile.as_bytes());
        digest.update(index.to_le_bytes());
        let mut key = String::with_capacity(64);
        for byte in digest.finalize() {
            write!(key, "{byte:02x}").expect("writing to a String cannot fail");
        }
        key
    }

    pub fn available(&self) -> bool {
        self.available.load(Ordering::Relaxed)
    }

    pub async fn lock(&self, key: &str) -> OwnedMutexGuard<()> {
        let lock = {
            let mut flights = self.flights.lock().expect("cache flights are not poisoned");
            flights.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = flights.get(key).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(AsyncMutex::new(()));
                flights.insert(key.to_owned(), Arc::downgrade(&lock));
                lock
            }
        };
        lock.lock_owned().await
    }

    pub async fn get(&self, key: &str) -> Option<Vec<u8>> {
        if !self.available() {
            return None;
        }
        let path = {
            let mut state = self.state.lock().expect("cache state is not poisoned");
            let sequence = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
            let entry = state.entries.get_mut(key)?;
            entry.last_used = sequence;
            entry.path.clone()
        };
        match fs::read(&path).await {
            Ok(bytes) => Some(bytes),
            Err(error) => {
                eprintln!("Could not read cached segment {}: {error}", path.display());
                self.remove_key(key).await;
                None
            }
        }
    }

    pub async fn put(&self, key: &str, bytes: &[u8]) {
        if !self.available() || bytes.len() as u64 > self.max_bytes {
            return;
        }
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let temporary = self.root.join(format!(".{key}.{sequence}.tmp"));
        let destination = self.root.join(format!("{key}.ts"));
        if let Err(error) = fs::write(&temporary, bytes).await {
            eprintln!("Could not write transcode cache: {error}");
            let _ = fs::remove_file(&temporary).await;
            return;
        }
        if let Err(error) = fs::rename(&temporary, &destination).await {
            eprintln!("Could not commit transcode cache: {error}");
            let _ = fs::remove_file(&temporary).await;
            return;
        }
        {
            let mut state = self.state.lock().expect("cache state is not poisoned");
            if let Some(previous) = state.entries.insert(
                key.to_owned(),
                Entry {
                    path: destination,
                    size: bytes.len() as u64,
                    last_used: sequence,
                },
            ) {
                state.total = state.total.saturating_sub(previous.size);
            }
            state.total = state.total.saturating_add(bytes.len() as u64);
        }
        self.evict().await;
    }

    async fn remove_key(&self, key: &str) {
        let path = self
            .state
            .lock()
            .expect("cache state is not poisoned")
            .entries
            .remove(key)
            .map(|entry| entry.path);
        if let Some(path) = path {
            let _ = fs::remove_file(path).await;
        }
    }

    async fn evict(&self) {
        let paths = {
            let mut state = self.state.lock().expect("cache state is not poisoned");
            let mut removed = Vec::new();
            while state.total > self.max_bytes {
                let Some(key) = state
                    .entries
                    .iter()
                    .min_by_key(|(_, entry)| entry.last_used)
                    .map(|(key, _)| key.clone())
                else {
                    break;
                };
                if let Some(entry) = state.entries.remove(&key) {
                    state.total = state.total.saturating_sub(entry.size);
                    removed.push(entry.path);
                }
            }
            removed
        };
        for path in paths {
            let _ = fs::remove_file(path).await;
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}
