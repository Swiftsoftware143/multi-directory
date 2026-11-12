//! On-disk cache for proxied Google Places photos.
//!
//! A Google place photo is billed per REQUEST, and the old shape of `place_photos` billed it again
//! on every page view: 24 proxied cards per city page, every time the page loaded. The bytes do
//! not change between views of the same reference, so they are stored once here and served from
//! disk afterwards.
//!
//! What this module guarantees, and why each rule exists:
//!
//!   * **Keyed by the Places photo RESOURCE NAME**, hashed — never by business id. Two listings
//!     that share a photo therefore share one entry, and the key is a stable 64-hex filename, so
//!     no untrusted string ever reaches a path.
//!   * **Content is served for at most `ttl_secs`** (default 7 days, hard-capped at 29 days).
//!     Google Maps Platform terms allow photo caching for at most 30 days; the cap is enforced in
//!     `CacheConfig::from_env`, not left to an operator. An entry past its expiry is DELETED and
//!     reported as a miss — an expired entry is never served as a hit.
//!   * **Bounded on disk with LRU eviction.** The whole directory is capped (`max_bytes`, default
//!     512 MiB); when a store would cross the cap the least-recently-used entries are removed
//!     until usage is back under 90% of it. A hit touches the entry's mtime, so "recently used" is
//!     real access order rather than write order.
//!   * **A failure is never cached.** Empty or oversized bytes are refused by `put`, and the
//!     handler only calls it with bytes Google actually returned. A 404 from Google therefore
//!     cannot pin a broken entry.
//!   * **One fetch per key at a time** (`claim`). A city page load issues up to 24 image requests
//!     at once; without this, two visitors arriving together would each pay for the same photo.
//!
//! Counters (`stats`) make the effect measurable: hits, misses, expired, stores, evictions and the
//! number of BILLED Google requests the process made. They are exposed by
//! `GET /api/v1/place-photo/cache-stats` and, per image, by the `X-Photo-Cache` header.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex as StdMutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

/// Seven days: long enough that a popular city page stops costing anything, short enough that a
/// changed photo is picked up within a week and far inside the 30-day terms ceiling.
pub const DEFAULT_TTL_SECS: u64 = 7 * 24 * 3600;
/// Google Maps Platform terms cap photo caching at 30 days. Keep one day of slack.
pub const MAX_TTL_SECS: u64 = 29 * 24 * 3600;
/// 2 GiB: one warm city is ~300 photos at ~380 KB (~115 MB), so this holds every photo in a
/// ten-city network (the fleet's ceiling) with room to spare, while still being a hard bound on a
/// 1.3 TB disk. LRU eviction covers anything beyond it.
pub const DEFAULT_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MIN_MAX_BYTES: u64 = 1024 * 1024;
const MAX_MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;
/// Evict down to this percentage of the cap, so the next few hundred stores do not each evict.
const EVICT_TARGET_PCT: u64 = 90;
/// Never store a single entry bigger than this, whatever the cap says.
const MAX_ENTRY_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct CacheConfig {
    pub dir: PathBuf,
    pub ttl_secs: u64,
    pub max_bytes: u64,
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

/// The terms guarantee, in one place: no environment value can make cached content live longer
/// than `MAX_TTL_SECS`, and a nonsense value (0) still gets a usable window.
fn clamp_ttl(secs: u64) -> u64 {
    secs.clamp(60, MAX_TTL_SECS)
}

impl CacheConfig {
    /// Resolved per call from the environment, so a deployment can tune it without a code change.
    /// The TTL clamp is the terms guarantee: no configuration can make content live longer than
    /// `MAX_TTL_SECS`.
    pub fn from_env() -> Self {
        let dir = std::env::var("MD_PHOTO_CACHE_DIR")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(crate::handlers::audience::uploads_root()).join(".place-photo-cache")
            });
        Self {
            dir,
            ttl_secs: clamp_ttl(env_u64("MD_PHOTO_CACHE_TTL_SECS", DEFAULT_TTL_SECS)),
            max_bytes: env_u64("MD_PHOTO_CACHE_MAX_BYTES", DEFAULT_MAX_BYTES)
                .clamp(MIN_MAX_BYTES, MAX_MAX_BYTES),
        }
    }
}

// ---------------------------------------------------------------- counters

#[derive(Default)]
struct Counters {
    hits: AtomicU64,
    misses: AtomicU64,
    expired: AtomicU64,
    stores: AtomicU64,
    evictions: AtomicU64,
    evicted_bytes: AtomicU64,
    errors: AtomicU64,
    google_calls: AtomicU64,
    bytes_served: AtomicU64,
    bytes_stored: AtomicU64,
}

static C: OnceLock<Counters> = OnceLock::new();

fn counters() -> &'static Counters {
    C.get_or_init(Counters::default)
}

/// One BILLED Google Places request was issued by this process. Called at the single place each
/// request is sent, so the number the acceptance test reads is the number of calls, not a guess.
pub fn note_google_call() {
    counters().google_calls.fetch_add(1, Ordering::Relaxed);
}

/// A cache hit was served by a path that did not go through `get` (the single-flight wait loop).
pub fn note_hit(bytes: usize) {
    counters().hits.fetch_add(1, Ordering::Relaxed);
    counters()
        .bytes_served
        .fetch_add(bytes as u64, Ordering::Relaxed);
}

// ---------------------------------------------------------------- keys and paths

/// The cache key for a photo resource name: `sha256(name)`, hex. Stable across restarts and
/// builds (unlike `DefaultHasher`), and safe as a filename whatever the name contains.
pub(crate) fn key_of(name: &str) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, name.trim().as_bytes());
    hex::encode(digest.as_ref())
}

/// Two levels of fan-out (`<dir>/ab/<hash>.img`) keeps a directory from holding tens of thousands
/// of entries in one listing.
fn entry_path(cfg: &CacheConfig, key: &str) -> PathBuf {
    cfg.dir.join(&key[..2]).join(format!("{key}.img"))
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------- entry format

struct EntryMeta {
    content_type: String,
    expires_at: i64,
}

fn encode(name: &str, bytes: &[u8], content_type: &str, expires_at: i64) -> Vec<u8> {
    let header = json!({
        "ct": content_type,
        "exp": expires_at,
        "name": name.trim(),
    });
    let mut out = serde_json::to_vec(&header).unwrap_or_default();
    out.push(b'\n');
    out.extend_from_slice(bytes);
    out
}

/// Split one file into its JSON header line and the image bytes. A file that does not parse is
/// treated as absent (`None`) — never served, never trusted.
fn decode(raw: &[u8]) -> Option<(EntryMeta, &[u8])> {
    let nl = raw.iter().position(|b| *b == b'\n')?;
    let header: Value = serde_json::from_slice(&raw[..nl]).ok()?;
    let body = &raw[nl + 1..];
    if body.is_empty() {
        return None;
    }
    Some((
        EntryMeta {
            content_type: header
                .get("ct")
                .and_then(Value::as_str)
                .filter(|v| v.starts_with("image/"))
                .unwrap_or("image/jpeg")
                .to_string(),
            expires_at: header.get("exp").and_then(Value::as_i64)?,
        },
        body,
    ))
}

// ---------------------------------------------------------------- read

/// The cached bytes for `name`, counted, with the entry's mtime touched so LRU order reflects use.
/// An entry past its expiry is deleted and answers as a miss.
pub async fn get(cfg: &CacheConfig, name: &str) -> Option<(Vec<u8>, String)> {
    let path = entry_path(cfg, &key_of(name));
    let raw = match tokio::fs::read(&path).await {
        Ok(raw) => raw,
        Err(_) => {
            // Self-heal: the file is gone (cleared directory, eviction), so this key is not warm.
            warm_forget(&key_of(name));
            counters().misses.fetch_add(1, Ordering::Relaxed);
            return None;
        }
    };
    let Some((meta, body)) = decode(&raw) else {
        // Unreadable entry: remove it rather than keeping a poisoned file forever.
        let _ = tokio::fs::remove_file(&path).await;
        counters().errors.fetch_add(1, Ordering::Relaxed);
        counters().misses.fetch_add(1, Ordering::Relaxed);
        return None;
    };
    if meta.expires_at <= now_unix() {
        let _ = tokio::fs::remove_file(&path).await;
        warm_forget(&key_of(name));
        counters().expired.fetch_add(1, Ordering::Relaxed);
        counters().misses.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let bytes = body.to_vec();
    touch(&path).await;
    warm_mark(&key_of(name), meta.expires_at);
    note_hit(bytes.len());
    Some((bytes, meta.content_type))
}

/// Same read, no counters and no mtime touch — for probing while another request is fetching.
pub async fn peek(cfg: &CacheConfig, name: &str) -> Option<(Vec<u8>, String)> {
    let raw = tokio::fs::read(entry_path(cfg, &key_of(name))).await.ok()?;
    let (meta, body) = decode(&raw)?;
    if meta.expires_at <= now_unix() {
        return None;
    }
    Some((body.to_vec(), meta.content_type))
}

async fn touch(path: &Path) {
    let path = path.to_path_buf();
    let _ = tokio::task::spawn_blocking(move || {
        if let Ok(f) = std::fs::File::options().write(true).open(&path) {
            let _ = f.set_modified(SystemTime::now());
        }
    })
    .await;
}

// ---------------------------------------------------------------- write

/// Store one photo's bytes. Refuses anything that is not a usable image, so a failed or empty
/// Google answer can never become a cache entry.
pub async fn put(cfg: &CacheConfig, name: &str, bytes: &[u8], content_type: &str) {
    if bytes.is_empty()
        || bytes.len() as u64 > MAX_ENTRY_BYTES
        || bytes.len() as u64 > cfg.max_bytes
    {
        return;
    }
    put_at(
        cfg,
        name,
        bytes,
        content_type,
        now_unix() + cfg.ttl_secs as i64,
    )
    .await;
}

/// `put` with an explicit expiry — the only way a test can place an entry that is already stale.
pub async fn put_at(
    cfg: &CacheConfig,
    name: &str,
    bytes: &[u8],
    content_type: &str,
    expires_at: i64,
) {
    let path = entry_path(cfg, &key_of(name));
    let Some(parent) = path.parent() else { return };
    if tokio::fs::create_dir_all(parent).await.is_err() {
        counters().errors.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let payload = encode(name, bytes, content_type, expires_at);
    // Write beside the target and rename, so a reader never sees half a file.
    let tmp = parent.join(format!(
        ".{}.tmp",
        path.file_stem().and_then(|s| s.to_str()).unwrap_or("entry")
    ));
    if tokio::fs::write(&tmp, &payload).await.is_err() {
        counters().errors.fetch_add(1, Ordering::Relaxed);
        return;
    }
    if tokio::fs::rename(&tmp, &path).await.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
        counters().errors.fetch_add(1, Ordering::Relaxed);
        return;
    }
    counters().stores.fetch_add(1, Ordering::Relaxed);
    counters()
        .bytes_stored
        .fetch_add(payload.len() as u64, Ordering::Relaxed);
    warm_mark(&key_of(name), expires_at);
    size_grew(payload.len() as u64);
    enforce_cap(cfg).await;
}

// ---------------------------------------------------------------- size accounting

static EST_BYTES: AtomicU64 = AtomicU64::new(0);
static SEEDED: AtomicU64 = AtomicU64::new(0);

fn evict_lock() -> &'static tokio::sync::Mutex<()> {
    static L: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    L.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn size_grew(n: u64) {
    EST_BYTES.fetch_add(n, Ordering::Relaxed);
}

struct DiskEntry {
    key: String,
    path: PathBuf,
    size: u64,
    mtime: SystemTime,
    expired: bool,
}

/// Walk the cache directory. Blocking filesystem work stays off the async runtime.
async fn scan(cfg: &CacheConfig) -> (u64, usize, Vec<DiskEntry>) {
    let root = cfg.dir.clone();
    let now = now_unix();
    let entries = tokio::task::spawn_blocking(move || {
        let mut out: Vec<DiskEntry> = Vec::new();
        let Ok(shards) = std::fs::read_dir(&root) else {
            return out;
        };
        for shard in shards.flatten() {
            let Ok(kind) = shard.file_type() else {
                continue;
            };
            if !kind.is_dir() {
                continue;
            }
            let Ok(files) = std::fs::read_dir(shard.path()) else {
                continue;
            };
            for f in files.flatten() {
                let path = f.path();
                if path.extension().and_then(|e| e.to_str()) != Some("img") {
                    continue;
                }
                let Ok(meta) = f.metadata() else { continue };
                let mtime = meta.modified().unwrap_or(UNIX_EPOCH);
                // Cheap staleness check from the header line only (first 1 KiB, never the body).
                let expired = {
                    use std::io::Read;
                    let mut head = Vec::with_capacity(1024);
                    let mut f = match std::fs::File::open(&path) {
                        Ok(f) => f,
                        Err(_) => continue,
                    };
                    if f.by_ref().take(1024).read_to_end(&mut head).is_err() {
                        continue;
                    }
                    let nl = head.iter().position(|b| *b == b'\n');
                    nl.and_then(|nl| serde_json::from_slice::<Value>(&head[..nl]).ok())
                        .and_then(|v| v.get("exp").and_then(Value::as_i64))
                        .map(|exp| exp <= now)
                        .unwrap_or(true)
                };
                out.push(DiskEntry {
                    key: path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default()
                        .to_string(),
                    path,
                    size: meta.len(),
                    mtime,
                    expired,
                });
            }
        }
        out
    })
    .await
    .unwrap_or_default();
    let total = entries.iter().map(|e| e.size).sum();
    (total, entries.len(), entries)
}

/// Bring usage back under the cap. Expired entries go first (they are worthless), then the
/// least-recently-used ones. Called after every store, and cheap while under the cap.
async fn enforce_cap(cfg: &CacheConfig) {
    if SEEDED.load(Ordering::Relaxed) == 0 {
        // First store in this process: learn what is already on disk before trusting the estimate.
        let (total, _n, _e) = scan(cfg).await;
        EST_BYTES.store(total, Ordering::Relaxed);
        SEEDED.store(1, Ordering::Relaxed);
    }
    if EST_BYTES.load(Ordering::Relaxed) <= cfg.max_bytes {
        return;
    }
    let _guard = evict_lock().lock().await;
    let (total, _n, mut entries) = scan(cfg).await;
    let target = cfg.max_bytes / 100 * EVICT_TARGET_PCT;
    let mut cur = total;
    let mut evicted = 0u64;
    let mut freed = 0u64;

    for e in entries.iter().filter(|e| e.expired) {
        if tokio::fs::remove_file(&e.path).await.is_ok() {
            warm_forget(&e.key);
            cur = cur.saturating_sub(e.size);
            evicted += 1;
            freed += e.size;
        }
    }
    if cur > target {
        entries.sort_by_key(|e| e.mtime);
        for e in entries.iter().filter(|e| !e.expired) {
            if cur <= target {
                break;
            }
            if tokio::fs::remove_file(&e.path).await.is_ok() {
                warm_forget(&e.key);
                cur = cur.saturating_sub(e.size);
                evicted += 1;
                freed += e.size;
            }
        }
    }
    EST_BYTES.store(cur, Ordering::Relaxed);
    if evicted > 0 {
        counters().evictions.fetch_add(evicted, Ordering::Relaxed);
        counters().evicted_bytes.fetch_add(freed, Ordering::Relaxed);
        tracing::info!(
            evicted, freed_bytes = freed, remaining_bytes = cur, cap = cfg.max_bytes,
            dir = %cfg.dir.display(), "place-photo cache evicted to stay inside its size cap"
        );
    }
}

// ---------------------------------------------------------------- single flight

static INFLIGHT: OnceLock<StdMutex<HashSet<String>>> = OnceLock::new();

/// Held for the duration of one cold fetch. Dropping it (including on an error path) releases the
/// key, so a failed fetch cannot wedge later requests.
pub struct FetchClaim(String);

impl Drop for FetchClaim {
    fn drop(&mut self) {
        if let Some(set) = INFLIGHT.get() {
            let mut s = set.lock().unwrap_or_else(|e| e.into_inner());
            s.remove(&self.0);
        }
    }
}

/// `Some` when this request owns the fetch for `name`; `None` when another one already does.
pub fn claim(name: &str) -> Option<FetchClaim> {
    let key = name.trim().to_string();
    let set = INFLIGHT.get_or_init(|| StdMutex::new(HashSet::new()));
    let mut guard = set.lock().unwrap_or_else(|e| e.into_inner());
    if guard.insert(key.clone()) {
        Some(FetchClaim(key))
    } else {
        None
    }
}

// ---------------------------------------------------------------- warm map

/// Keys this PROCESS knows are on disk, with the expiry it stored. Lets the list renderer decide
/// whether showing a photo would cost a billed call, without touching the filesystem per card:
/// a warm card consumes no cold-miss budget, which is the whole point of the cache.
static WARM: OnceLock<StdMutex<HashMap<String, i64>>> = OnceLock::new();

fn warm_map() -> &'static StdMutex<HashMap<String, i64>> {
    WARM.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn warm_mark(key: &str, expires_at: i64) {
    let mut m = warm_map().lock().unwrap_or_else(|e| e.into_inner());
    m.insert(key.to_string(), expires_at);
}

fn warm_forget(key: &str) {
    let mut m = warm_map().lock().unwrap_or_else(|e| e.into_inner());
    m.remove(key);
}

/// TRUE when serving this photo name should hit the disk cache rather than Google. Two facts are
/// required: this process recorded a future expiry for the key, AND the entry is still on disk
/// (the directory can be cleared by an operator, and a stale "warm" answer would let one page
/// render hand out every card's photo as a billed call).
pub fn is_warm(name: &str) -> bool {
    let key = key_of(name);
    {
        let m = warm_map().lock().unwrap_or_else(|e| e.into_inner());
        if !matches!(m.get(&key), Some(exp) if *exp > now_unix()) {
            return false;
        }
    }
    entry_path(&CacheConfig::from_env(), &key).exists()
}

/// The keys of every NON-EXPIRED entry on disk, in one directory walk.
///
/// The warm-up job needs to know what is already cached without spending a Google call to find
/// out, and `is_warm` answers for THIS process only (its map is empty after a restart, and a
/// restart must not make a fully warmed city look cold again). This reads the directory, so the
/// answer is what the disk actually holds.
pub async fn warm_keys(cfg: &CacheConfig) -> HashSet<String> {
    let (_total, _n, all) = scan(cfg).await;
    all.into_iter()
        .filter(|e| !e.expired)
        .map(|e| e.key)
        .collect()
}

/// Populate the in-process warm map from what is already on disk.
///
/// `is_warm` is what decides whether a city page may show a photo for free or must spend its
/// cold-miss budget on it. Without this seed every entry looks cold after a restart, so a fully
/// warmed city would render letter avatars until 24 photos had been re-requested (each one a disk
/// hit — no Google call — but the page would still look unwarmed). Runs once at boot, off the
/// request path.
pub async fn seed_warm_map(cfg: &CacheConfig) {
    let (_total, _n, all) = scan(cfg).await;
    let expires = now_unix() + cfg.ttl_secs as i64;
    let mut marked = 0usize;
    for e in all.iter().filter(|e| !e.expired) {
        warm_mark(&e.key, expires);
        marked += 1;
    }
    if marked > 0 {
        tracing::info!(
            entries = marked, dir = %cfg.dir.display(),
            "photo cache served-from-disk map seeded"
        );
    }
}

/// Seed the warm map at boot without holding up the listener.
pub fn start_warm_map_seed() {
    tokio::spawn(async move {
        seed_warm_map(&CacheConfig::from_env()).await;
    });
}

// ---------------------------------------------------------------- stats

/// Hit/miss/eviction/billed-call numbers, plus what is on disk right now.
pub async fn stats(cfg: &CacheConfig) -> Value {
    let (bytes, entries, all) = scan(cfg).await;
    let expired = all.iter().filter(|e| e.expired).count();
    let c = counters();
    json!({
        "hits": c.hits.load(Ordering::Relaxed),
        "misses": c.misses.load(Ordering::Relaxed),
        "expired": c.expired.load(Ordering::Relaxed),
        "stores": c.stores.load(Ordering::Relaxed),
        "evictions": c.evictions.load(Ordering::Relaxed),
        "evicted_bytes": c.evicted_bytes.load(Ordering::Relaxed),
        "errors": c.errors.load(Ordering::Relaxed),
        "google_calls": c.google_calls.load(Ordering::Relaxed),
        "bytes_served": c.bytes_served.load(Ordering::Relaxed),
        "bytes_stored": c.bytes_stored.load(Ordering::Relaxed),
        "entries": entries,
        "entries_expired": expired,
        "bytes": bytes,
        "max_bytes": cfg.max_bytes,
        "ttl_secs": cfg.ttl_secs,
        "max_ttl_secs": MAX_TTL_SECS,
        "dir": cfg.dir.display().to_string(),
    })
}

/// A verdict word for the `X-Photo-Cache` response header.
pub const HEADER: &str = "x-photo-cache";

/// Seconds of `Duration` used by the handler's short wait on a duplicate fetch.
pub const WAIT_STEP: Duration = Duration::from_millis(100);

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_dir(tag: &str) -> Dir {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!(
            "md-photo-cache-{}-{}-{}",
            tag,
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&p);
        Dir(p)
    }

    fn cfg(dir: &Dir, ttl_secs: u64, max_bytes: u64) -> CacheConfig {
        CacheConfig {
            dir: dir.0.clone(),
            ttl_secs,
            max_bytes,
        }
    }

    const NAME: &str = "places/ChIJtesttesttest/photos/AWCtestphoto";

    #[tokio::test]
    async fn a_stored_photo_comes_back_byte_for_byte() {
        let d = temp_dir("hit");
        let c = cfg(&d, 3600, 10 * 1024 * 1024);
        assert!(get(&c, NAME).await.is_none(), "nothing stored yet");
        put(&c, NAME, b"jpegbytes", "image/jpeg").await;
        let (bytes, ct) = get(&c, NAME).await.expect("cache hit");
        assert_eq!(bytes, b"jpegbytes");
        assert_eq!(ct, "image/jpeg");
    }

    #[tokio::test]
    async fn two_names_are_two_entries_but_one_name_is_shared() {
        let d = temp_dir("share");
        let c = cfg(&d, 3600, 10 * 1024 * 1024);
        put(&c, NAME, b"shared", "image/jpeg").await;
        // The same resource name is one entry however many listings point at it: the key is the
        // name, not the business.
        assert!(get(&c, NAME).await.is_some());
        assert!(get(&c, "places/ChIJother/photos/AWCother").await.is_none());
        assert_eq!(stats(&c).await["entries"], 1);
    }

    #[tokio::test]
    async fn an_expired_entry_is_deleted_and_never_served() {
        let d = temp_dir("ttl");
        let c = cfg(&d, 3600, 10 * 1024 * 1024);
        // Inside its window: served.
        put(&c, NAME, b"fresh", "image/jpeg").await;
        assert!(get(&c, NAME).await.is_some());
        // One second past the TTL: not a hit, and the file is gone (not served, not kept).
        put_at(&c, NAME, b"stale", "image/jpeg", now_unix() - 1).await;
        assert!(
            get(&c, NAME).await.is_none(),
            "an expired entry must not be a cache hit"
        );
        assert!(!entry_path(&c, &key_of(NAME)).exists());
    }

    #[tokio::test]
    async fn the_stored_expiry_uses_the_configured_ttl_and_never_exceeds_the_terms_ceiling() {
        let d = temp_dir("ttl-env");
        let c = cfg(&d, 42, 10 * 1024 * 1024);
        put(&c, NAME, b"x", "image/png").await;
        let raw = std::fs::read(entry_path(&c, &key_of(NAME))).unwrap();
        let (meta, _) = decode(&raw).unwrap();
        let delta = meta.expires_at - now_unix();
        assert!((41..=43).contains(&delta), "expiry follows cfg.ttl_secs");
        // And no configuration value can be talked past the 30-day terms ceiling.
        assert_eq!(clamp_ttl(999_999_999), MAX_TTL_SECS);
        assert_eq!(
            clamp_ttl(0),
            60,
            "a nonsense value still gets a usable window"
        );
        assert!(
            MAX_TTL_SECS < 30 * 24 * 3600,
            "inside Google's 30-day ceiling"
        );
    }

    #[tokio::test]
    async fn an_empty_or_oversized_answer_is_never_stored() {
        let d = temp_dir("nofail");
        let c = cfg(&d, 3600, 10 * 1024 * 1024);
        put(&c, NAME, b"", "image/jpeg").await;
        assert!(
            get(&c, NAME).await.is_none(),
            "an empty body must not be cached"
        );
        let big = vec![0u8; (MAX_ENTRY_BYTES + 1) as usize];
        put(&c, NAME, &big, "image/jpeg").await;
        assert!(
            get(&c, NAME).await.is_none(),
            "an oversized body must not be cached"
        );
    }

    #[tokio::test]
    async fn the_directory_stays_inside_its_cap_by_evicting_the_least_recently_used() {
        let d = temp_dir("lru");
        let cap = 20 * 1024;
        let c = cfg(&d, 3600, cap);
        let body = vec![7u8; 4096];
        for i in 0..12u32 {
            put(
                &c,
                &format!("places/ChIJ{i}/photos/AWC{i}"),
                &body,
                "image/jpeg",
            )
            .await;
            // Keep the mtimes strictly ordered, so "least recently used" is unambiguous.
            tokio::time::sleep(Duration::from_millis(3)).await;
        }
        let (bytes, entries, _) = scan(&c).await;
        assert!(
            bytes <= cap,
            "disk usage {bytes} must stay inside cap {cap}"
        );
        assert!(entries < 12, "eviction must have removed entries");
        // The most recent store is the one that survives; the first is gone.
        assert!(
            get(&c, "places/ChIJ11/photos/AWC11").await.is_some(),
            "newest entry must survive eviction"
        );
        assert!(
            get(&c, "places/ChIJ0/photos/AWC0").await.is_none(),
            "oldest entry must be the first to go"
        );
        assert!(stats(&c).await["evictions"].as_u64().unwrap() >= 1);
    }

    #[tokio::test]
    async fn a_claimed_key_is_released_when_the_guard_drops() {
        let a = claim(NAME).expect("first claim wins");
        assert!(claim(NAME).is_none(), "second claim waits");
        drop(a);
        assert!(claim(NAME).is_some(), "released after the guard drops");
    }

    #[tokio::test]
    async fn the_key_is_the_name_and_is_a_plain_hash() {
        let k = key_of("  places/ChIJa/photos/AWCb  ");
        assert_eq!(k.len(), 64);
        assert!(k.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(k, key_of("places/ChIJa/photos/AWCb"), "trimmed and stable");
        assert_ne!(k, key_of("places/ChIJa/photos/AWCc"));
    }
}
