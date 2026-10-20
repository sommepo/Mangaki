use std::{
    collections::HashMap,
    fs,
    hash::{Hash, Hasher},
    io::{Read, Write},
    os::fd::FromRawFd,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::{AtomicU64, Ordering}},
    time::Duration,
};

use axum::{
    body::Body,
    Json, Router,
    extract::{Path as AxumPath, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use reqwest::{Client, Url};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use futures::StreamExt;
use uuid::Uuid;
use jni::objects::{JByteArray, JClass, JObject, JString, JValue};
use chrono::{Datelike, Local, NaiveDate};
use manga_reader_core::{StatEvent, StatEventKind, StatsRanges, calculate_stats};

type KeyedLocks<K> = Arc<tokio::sync::Mutex<HashMap<K, Arc<tokio::sync::Mutex<()>>>>>;
type SmbSessions = Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<Option<SmbSession>>>>>>;

#[derive(Clone)]
pub struct ForkApiState {
    path: PathBuf,
    cache_dir: PathBuf,
    offline_dir: PathBuf,
    thumbnail_dir: PathBuf,
    inner: Arc<Mutex<ForkState>>,
    job_worker: Arc<tokio::sync::Mutex<()>>,
    // Snapshots are numbered while the state lock is held so a slower writer
    // can never replace a newer state file with an older snapshot.
    generation: Arc<AtomicU64>,
    persisted_generation: Arc<Mutex<u64>>,
    persist_requested: Arc<tokio::sync::Notify>,
    http: Client,
    smb_sessions: SmbSessions,
    archive_downloads: KeyedLocks<PathBuf>,
    remote_archives: Arc<Mutex<Vec<(String, OpenRemoteArchive)>>>,
    progress_writeback: Arc<tokio::sync::Mutex<()>>,
    thumbnail_permits: Arc<tokio::sync::Semaphore>,
}

struct SmbSession {
    client: smb2::SmbClient,
    share: smb2::Tree,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ForkState {
    #[serde(default)]
    show_sensitive: bool,
    #[serde(default)]
    sources: Vec<Source>,
    #[serde(default)]
    series: Vec<Series>,
    #[serde(default)]
    categories: Vec<Category>,
    #[serde(default)]
    jobs: Vec<Job>,
    #[serde(default)]
    stats: StatsRanges,
    #[serde(default)]
    events: Vec<StatEvent>,
    #[serde(default)]
    pending_progress: Vec<PendingProgress>,
    #[serde(default)]
    offline_pages: Vec<OfflinePage>,
}

impl Default for ForkState {
    fn default() -> Self {
        Self { show_sensitive: false, sources: Vec::new(), series: Vec::new(), categories: Vec::new(), jobs: Vec::new(), stats: StatsRanges::default(), events: Vec::new(), pending_progress: Vec::new(), offline_pages: Vec::new() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Source {
    id: String,
    kind: SourceKind,
    name: String,
    status: SourceStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    address: Option<String>,
    #[serde(default)]
    allow_http: bool,
    credential_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    library_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    grouping_overrides: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum SourceKind { Local, Smb, Webdav, Komga }

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SourceStatus { Connected, NeedsPermission, Offline, Error }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Job {
    id: String,
    kind: String,
    title: String,
    sensitive: bool,
    state: String,
    completed_pages: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_pages: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(default)]
    pages: Vec<JobPage>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    prepare_ocr_after: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    reprocess: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ocr_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    self_hosted_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobPage {
    volume_id: String,
    page: u32,
    fingerprint: String,
    state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Series {
    id: String,
    source_id: String,
    title: String,
    #[serde(default)]
    sensitive: bool,
    #[serde(default)]
    category_ids: Vec<String>,
    #[serde(default)]
    volumes: Vec<Volume>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Volume {
    id: String,
    title: String,
    page_count: u32,
    fingerprint: String,
    #[serde(default)]
    pages: Vec<String>,
    #[serde(default)]
    sensitive: bool,
    #[serde(default)]
    progress_page: u32,
    #[serde(default)]
    ocr_state: OcrState,
    #[serde(default)]
    download_state: DownloadState,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Category { id: String, name: String, order: u32 }

#[derive(Debug, Deserialize)]
struct CategoryRequest { name: String }

#[derive(Debug, Deserialize)]
struct CategoryOrderRequest { ids: Vec<String> }

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeriesSensitiveRequest { series_ids: Vec<String>, sensitive: bool }

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VolumeSensitiveRequest { volume_ids: Vec<String>, sensitive: bool }

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeriesCategoryRequest { series_ids: Vec<String>, category_id: String, assigned: bool }

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProgressRequest { page: u32 }

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrepareOcrRequest {
    #[serde(default)]
    series_ids: Vec<String>,
    #[serde(default)]
    volume_ids: Vec<String>,
    #[serde(default)]
    reprocess: bool,
    #[serde(default)]
    prepare_ocr_after: bool,
    #[serde(default)]
    ocr_provider: Option<String>,
    #[serde(default)]
    self_hosted_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SelfHostedOcrTestRequest { url: String }

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SelfHostedOcrTestResult { ok: bool, engine: String, model_version: String }

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum OcrState {
    #[default]
    NotPrepared,
    Queued,
    Processing,
    Partial,
    Ready,
    Failed,
    OutOfDate,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum DownloadState {
    #[default]
    NotDownloaded,
    Queued,
    Downloading,
    Partial,
    Downloaded,
    Failed,
    OutOfDate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScanPayload { series: Vec<ScannedSeries> }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScannedSeries { id: String, title: String, volumes: Vec<ScannedVolume> }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScannedVolume {
    id: String,
    title: String,
    page_count: u32,
    fingerprint: String,
    pages: Vec<String>,
    #[serde(default)]
    archive_uri: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    progress_page: u32,
}

#[derive(Debug, Clone)]
struct RemoteEntry {
    path: String,
    size: u64,
    modified: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanPreviewItem { id: String, title: String, volume_count: usize, page_count: u32, warnings: usize }

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanPreview { series: Vec<ScanPreviewItem> }

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ApplyScanRequest { #[serde(default)] title_overrides: std::collections::HashMap<String, String> }

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct IncrementalRefresh {
    new_series: usize,
    new_volumes: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KomgaPage<T> { content: Vec<T> }

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KomgaBook {
    id: String,
    series_id: String,
    series_title: String,
    name: String,
    file_hash: String,
    last_modified: String,
    media: KomgaMedia,
    read_progress: Option<KomgaProgress>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KomgaMedia { pages_count: u32, status: String }

#[derive(Debug, Deserialize)]
struct KomgaProgress {
    /// One-based page number, as used by Komga's read-progress API.
    page: u32,
    #[serde(default)]
    completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PendingProgress {
    source_id: String,
    volume_id: String,
    remote_book_id: String,
    /// Zero-based page index, converted to Komga's one-based numbering on send.
    page: u32,
    #[serde(default)]
    page_count: u32,
    updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OfflinePage {
    volume_id: String,
    page: u32,
    fingerprint: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SourceCredentials {
    #[serde(default)]
    username: String,
    password: Option<String>,
    api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StatEventRequest {
    kind: StatEventKind,
    local_date: NaiveDate,
    occurred_at_ms: i64,
    #[serde(default = "one")]
    value: u64,
    unique_key: Option<String>,
    series_id: Option<String>,
    #[serde(default)]
    sensitive: bool,
}

fn one() -> u64 { 1 }

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrivacyRequest { show_sensitive: bool }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupPayload {
    schema_version: u32,
    state: ForkState,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceRequest {
    kind: SourceKind,
    name: String,
    #[serde(default)]
    address: String,
    #[serde(default)]
    username: String,
    password: Option<String>,
    api_key: Option<String>,
    #[serde(default)]
    allow_http: bool,
    #[serde(default)]
    library_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RemoteChoice { id: String, name: String }

#[derive(Debug, Serialize)]
struct ConnectionResult {
    ok: bool,
    detail: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    libraries: Vec<RemoteChoice>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OcrSelfTestResult {
    engine: &'static str,
    model_version: &'static str,
    regions: usize,
    recognized_japanese: bool,
    normalized_geometry: bool,
}

#[derive(Debug)]
struct ApiError(StatusCode, String);
impl IntoResponse for ApiError { fn into_response(self) -> Response { (self.0, self.1).into_response() } }

impl ForkApiState {
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join("fork-state.json");
        let cache_dir = data_dir.join("remote-archive-cache-v2");
        // Earlier builds cached archives without validating them against
        // remote changes; discard that cache instead of trusting it.
        let legacy_cache = data_dir.join("remote-archive-cache");
        if legacy_cache.is_dir() { std::thread::spawn(move || { let _ = fs::remove_dir_all(legacy_cache); }); }
        let offline_dir = data_dir.join("offline-pages");
        let thumbnail_dir = data_dir.join("cover-thumbnails");
        let _ = fs::create_dir_all(&cache_dir);
        let _ = fs::create_dir_all(&offline_dir);
        let _ = fs::create_dir_all(&thumbnail_dir);
        let inner = fs::read(&path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .pool_idle_timeout(Duration::from_secs(90))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .unwrap_or_default();
        Self {
            path,
            cache_dir,
            offline_dir,
            thumbnail_dir,
            inner: Arc::new(Mutex::new(inner)),
            job_worker: Arc::new(tokio::sync::Mutex::new(())),
            generation: Arc::new(AtomicU64::new(0)),
            persisted_generation: Arc::new(Mutex::new(0)),
            persist_requested: Arc::new(tokio::sync::Notify::new()),
            http,
            smb_sessions: Arc::default(),
            archive_downloads: Arc::default(),
            remote_archives: Arc::default(),
            progress_writeback: Arc::default(),
            // Decoding full pages is memory hungry; a scrolling library grid
            // must not decode dozens of them at once.
            thumbnail_permits: Arc::new(tokio::sync::Semaphore::new(2)),
        }
    }

    /// Writes the state immediately. Use for user-initiated structural changes
    /// whose failure should be reported to the caller.
    fn persist(&self, value: &ForkState) -> Result<(), ApiError> {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let bytes = serde_json::to_vec(value).map_err(internal)?;
        self.write_snapshot(generation, &bytes)
    }

    /// Coalesces high-frequency changes (page turns, statistics ticks, job
    /// page results) into one background write instead of rewriting the whole
    /// library file on every event.
    fn persist_soon(&self) {
        self.persist_requested.notify_one();
    }

    fn write_snapshot(&self, generation: u64, bytes: &[u8]) -> Result<(), ApiError> {
        let mut persisted = self.persisted_generation.lock().map_err(internal)?;
        if *persisted >= generation { return Ok(()); }
        let temporary = self.path.with_extension("json.tmp");
        let mut file = fs::File::create(&temporary).map_err(internal)?;
        file.write_all(bytes).map_err(internal)?;
        file.sync_all().map_err(internal)?;
        drop(file);
        fs::rename(&temporary, &self.path).map_err(internal)?;
        *persisted = generation;
        Ok(())
    }
}

fn spawn_persister(state: ForkApiState) {
    tokio::spawn(async move {
        loop {
            state.persist_requested.notified().await;
            tokio::time::sleep(Duration::from_millis(750)).await;
            let snapshot = match state.inner.lock() {
                Ok(guard) => {
                    let generation = state.generation.fetch_add(1, Ordering::SeqCst) + 1;
                    serde_json::to_vec(&*guard).ok().map(|bytes| (generation, bytes))
                }
                Err(_) => None,
            };
            let Some((generation, bytes)) = snapshot else { continue; };
            let writer = state.clone();
            match tokio::task::spawn_blocking(move || writer.write_snapshot(generation, &bytes)).await {
                Ok(Err(error)) => tracing::warn!("Could not save library state: {}", error.1),
                Err(error) => tracing::warn!("Library state writer failed: {error}"),
                Ok(Ok(())) => {}
            }
        }
    });
}

pub fn router(state: ForkApiState) -> Router {
    let recover = {
        let mut guard = state.inner.lock().expect("fork state lock poisoned");
        let mut ids = Vec::new();
        for job in &mut guard.jobs {
            if matches!(job.kind.as_str(), "ocr" | "download") && matches!(job.state.as_str(), "queued" | "processing") {
                job.state = "queued".into();
                ids.push(job.id.clone());
            }
        }
        let _ = state.persist(&guard);
        ids
    };
    spawn_persister(state.clone());
    let pending_sources: std::collections::HashSet<String> = state.inner.lock()
        .map(|guard| guard.pending_progress.iter().map(|pending| pending.source_id.clone()).collect())
        .unwrap_or_default();
    for source_id in pending_sources {
        let state = state.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            flush_pending_progress(&state, &source_id).await;
        });
    }
    for id in recover {
        let state = state.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            run_job(state, id).await;
        });
    }
    Router::new()
        .route("/state", get(get_state))
        .route("/privacy", put(set_privacy))
        .route("/sources", post(add_source))
        .route("/sources/{id}", delete(remove_source))
        .route("/sources/{id}/scan", post(scan_source))
        .route("/sources/{id}/refresh", post(refresh_source))
        .route("/sources/{id}/preview", post(preview_source))
        .route("/sources/{id}/apply-scan", post(apply_source_scan))
        .route("/sources/test", post(test_source))
        .route("/catalogue", get(get_catalogue))
        .route("/volumes/{id}/pages/{page}", get(get_page))
        .route("/volumes/{id}/thumbnail", get(get_thumbnail))
        .route("/volumes/{id}/progress", put(set_progress))
        .route("/categories", post(add_category))
        .route("/categories/order", put(reorder_categories))
        .route("/categories/{id}", put(rename_category).delete(remove_category))
        .route("/series/sensitive", put(set_series_sensitive))
        .route("/volumes/sensitive", put(set_volume_sensitive))
        .route("/series/categories", put(set_series_category))
        .route("/events", post(record_event))
        .route("/stats.csv", get(export_stats_csv))
        .route("/stats/detail", get(get_stats_detail))
        .route("/stats/sessions", get(get_reading_sessions))
        .route("/stats/sessions/{id}", delete(delete_stats_session))
        // Backups of large libraries exceed axum's 2 MiB default body limit.
        .route("/backup", get(export_backup).put(restore_backup).layer(axum::extract::DefaultBodyLimit::max(256 * 1024 * 1024)))
        .route("/jobs/{id}/{action}", post(control_job))
        .route("/jobs/ocr", post(prepare_ocr))
        .route("/jobs/download", post(prepare_download))
        .route("/ocr/self-test", post(ocr_self_test))
        .route("/ocr/self-hosted/test", post(test_self_hosted_ocr))
        .with_state(state)
}

async fn test_self_hosted_ocr(
    Json(request): Json<SelfHostedOcrTestRequest>,
) -> Result<Json<SelfHostedOcrTestResult>, ApiError> {
    let mut url = manatan_ocr_server::logic::validate_self_hosted_ocr_url(&request.url)
        .map_err(|error| ApiError(StatusCode::BAD_REQUEST, error.to_string()))?;
    url.set_path("/health");
    let response = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(120))
        .build().map_err(internal)?
        .get(url).send().await.map_err(connection)?
        .error_for_status().map_err(connection)?;
    let value: serde_json::Value = response.json().await.map_err(connection)?;
    if value.get("status").and_then(serde_json::Value::as_str) != Some("ok") {
        return Err(ApiError(StatusCode::BAD_GATEWAY, "OCR server health response was invalid".into()));
    }
    Ok(Json(SelfHostedOcrTestResult {
        ok: true,
        engine: value.get("engine").and_then(serde_json::Value::as_str).unwrap_or("Self-hosted OCR").to_string(),
        model_version: value.get("modelVersion").and_then(serde_json::Value::as_str).unwrap_or("unknown").to_string(),
    }))
}

async fn ocr_self_test() -> Result<Json<OcrSelfTestResult>, ApiError> {
    tokio::task::spawn_blocking(run_android_ocr_self_test)
        .await
        .map_err(internal)?
        .map(Json)
}

fn run_android_ocr_self_test() -> Result<OcrSelfTestResult, ApiError> {
    let android = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(android.vm().cast()) }.map_err(internal)?;
    let mut env = vm.attach_current_thread().map_err(internal)?;
    let context = unsafe { JObject::from_raw(android.context().cast()) };
    let class = app_class(&mut env, &context, "dev.local.mangareader.OnDeviceOcr")?;
    let output = match env.call_static_method(class, "selfTestJapanese", "()Ljava/lang/String;", &[])
        .and_then(|value| value.l()) {
        Ok(output) => output,
        Err(error) => return Err(clear_java_exception(&mut env, error)),
    };
    let output = JString::from(output);
    let json: String = env.get_string(&output).map_err(connection)?.into();
    let regions: Vec<serde_json::Value> = serde_json::from_str(&json).map_err(connection)?;
    let recognized_japanese = regions.iter().filter_map(|region| region.get("text")?.as_str())
        .flat_map(str::chars)
        .any(|ch| ('\u{3040}'..='\u{30ff}').contains(&ch) || ('\u{3400}'..='\u{9fff}').contains(&ch));
    let normalized_geometry = regions.iter().all(|region| {
        let Some(bounds) = region.get("tightBoundingBox") else { return false; };
        let values = ["x", "y", "width", "height"].map(|key| bounds.get(key).and_then(serde_json::Value::as_f64));
        matches!(values, [Some(x), Some(y), Some(width), Some(height)]
            if x >= 0.0 && y >= 0.0 && width > 0.0 && height > 0.0
                && x + width <= 1.001 && y + height <= 1.001)
    });
    Ok(OcrSelfTestResult {
        engine: "ML Kit Japanese text recognition v2 (bundled)",
        model_version: "com.google.mlkit:text-recognition-japanese:16.0.1",
        regions: regions.len(), recognized_japanese, normalized_geometry,
    })
}

fn normalized_ocr_provider(request: &PrepareOcrRequest) -> Result<(Option<String>, Option<String>), ApiError> {
    match request.ocr_provider.as_deref().unwrap_or("on_device") {
        "on_device" => Ok((None, None)),
        "lens" => Ok((Some("lens".into()), None)),
        "self_hosted" => {
            let value = request.self_hosted_url.as_deref().ok_or_else(|| {
                ApiError(StatusCode::BAD_REQUEST, "Self-hosted OCR URL is required".into())
            })?;
            manatan_ocr_server::logic::validate_self_hosted_ocr_url(value)
                .map_err(|error| ApiError(StatusCode::BAD_REQUEST, error.to_string()))?;
            Ok((Some("self_hosted".into()), Some(value.trim().trim_end_matches('/').to_string())))
        }
        _ => Err(ApiError(StatusCode::BAD_REQUEST, "Unknown OCR provider".into())),
    }
}

async fn prepare_ocr(
    State(state): State<ForkApiState>,
    Json(request): Json<PrepareOcrRequest>,
) -> Result<Json<Job>, ApiError> {
    if request.series_ids.is_empty() && request.volume_ids.is_empty() {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Select at least one series or volume".into()));
    }
    let (ocr_provider, self_hosted_url) = normalized_ocr_provider(&request)?;
    let series_ids: std::collections::HashSet<_> = request.series_ids.iter().collect();
    let volume_ids: std::collections::HashSet<_> = request.volume_ids.iter().collect();
    let mut guard = state.inner.lock().map_err(internal)?;
    let mut pages = Vec::new();
    let mut titles = Vec::new();
    let active_pages: std::collections::HashSet<(String, u32, String)> = guard.jobs.iter()
        .filter(|job| matches!(job.state.as_str(), "queued" | "processing" | "paused"))
        .flat_map(|job| job.pages.iter())
        .map(|page| (page.volume_id.clone(), page.page, page.fingerprint.clone()))
        .collect();
    for series in &guard.series {
        let selected_series = series_ids.contains(&series.id);
        for volume in &series.volumes {
            if !selected_series && !volume_ids.contains(&volume.id) { continue; }
            if volume.page_count == 0 || volume.error.is_some() { continue; }
            if !request.reprocess && volume.ocr_state == OcrState::Ready { continue; }
            titles.push(series.title.clone());
            for page in 0..volume.page_count {
                let identity = (volume.id.clone(), page, volume.fingerprint.clone());
                if active_pages.contains(&identity) { continue; }
                pages.push(JobPage {
                    volume_id: volume.id.clone(),
                    page,
                    fingerprint: volume.fingerprint.clone(),
                    state: "queued".into(),
                    error: None,
                });
            }
        }
    }
    if pages.is_empty() {
        return Err(ApiError(StatusCode::CONFLICT, "All selected pages are already ready or queued".into()));
    }
    titles.sort();
    titles.dedup();
    let sensitive = guard.series.iter().any(|series| {
        (series_ids.contains(&series.id) || series.volumes.iter().any(|volume| volume_ids.contains(&volume.id)))
            && (series.sensitive || series.volumes.iter().any(|volume| volume_ids.contains(&volume.id) && volume.sensitive))
    });
    let total_pages = u32::try_from(pages.len()).map_err(internal)?;
    let id = Uuid::new_v4().to_string();
    let job = Job {
        id: id.clone(), kind: "ocr".into(),
        title: if titles.len() == 1 { titles[0].clone() } else { format!("{} selected series", titles.len()) },
        sensitive, state: "queued".into(), completed_pages: 0,
        total_pages: Some(total_pages), error: None, pages,
        prepare_ocr_after: false, reprocess: request.reprocess,
        ocr_provider, self_hosted_url,
    };
    let selected_volumes: std::collections::HashSet<_> = job.pages.iter().map(|page| page.volume_id.as_str()).collect();
    for volume in guard.series.iter_mut().flat_map(|series| &mut series.volumes) {
        if selected_volumes.contains(volume.id.as_str()) { volume.ocr_state = OcrState::Queued; }
    }
    guard.jobs.push(job.clone());
    state.persist(&guard)?;
    drop(guard);
    spawn_job_worker(state.clone(), id);
    Ok(Json(job))
}

async fn prepare_download(
    State(state): State<ForkApiState>,
    Json(request): Json<PrepareOcrRequest>,
) -> Result<Json<Job>, ApiError> {
    if request.series_ids.is_empty() && request.volume_ids.is_empty() {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Select at least one series or volume".into()));
    }
    let (ocr_provider, self_hosted_url) = normalized_ocr_provider(&request)?;
    let series_ids: std::collections::HashSet<_> = request.series_ids.iter().collect();
    let volume_ids: std::collections::HashSet<_> = request.volume_ids.iter().collect();
    let mut guard = state.inner.lock().map_err(internal)?;
    let remote_sources: std::collections::HashSet<_> = guard.sources.iter()
        .filter(|source| source.kind != SourceKind::Local)
        .map(|source| source.id.as_str()).collect();
    let active_pages: std::collections::HashSet<(String, u32, String)> = guard.jobs.iter()
        .filter(|job| job.kind == "download" && matches!(job.state.as_str(), "queued" | "processing" | "paused"))
        .flat_map(|job| job.pages.iter())
        .map(|page| (page.volume_id.clone(), page.page, page.fingerprint.clone()))
        .collect();
    let mut pages = Vec::new();
    let mut titles = Vec::new();
    for series in &guard.series {
        if !remote_sources.contains(series.source_id.as_str()) { continue; }
        let selected_series = series_ids.contains(&series.id);
        for volume in &series.volumes {
            if !selected_series && !volume_ids.contains(&volume.id) { continue; }
            if volume.page_count == 0 || volume.error.is_some() { continue; }
            if !request.reprocess && volume.download_state == DownloadState::Downloaded { continue; }
            titles.push(series.title.clone());
            for page in 0..volume.page_count {
                let identity = (volume.id.clone(), page, volume.fingerprint.clone());
                let already_downloaded = guard.offline_pages.iter().any(|stored| {
                    stored.volume_id == volume.id && stored.page == page && stored.fingerprint == volume.fingerprint
                        && offline_page_path(&state.offline_dir, stored).is_file()
                });
                if !request.reprocess && (active_pages.contains(&identity) || already_downloaded) { continue; }
                pages.push(JobPage { volume_id: volume.id.clone(), page, fingerprint: volume.fingerprint.clone(), state: "queued".into(), error: None });
            }
        }
    }
    if pages.is_empty() { return Err(ApiError(StatusCode::CONFLICT, "All selected remote pages are already downloaded or queued".into())); }
    if request.reprocess {
        let selected: std::collections::HashSet<_> = pages.iter().map(|page| (page.volume_id.as_str(), page.page, page.fingerprint.as_str())).collect();
        guard.offline_pages.retain(|stored| !selected.contains(&(stored.volume_id.as_str(), stored.page, stored.fingerprint.as_str())));
    }
    titles.sort();
    titles.dedup();
    let sensitive = guard.series.iter().any(|series| {
        (series_ids.contains(&series.id) || series.volumes.iter().any(|volume| volume_ids.contains(&volume.id)))
            && (series.sensitive || series.volumes.iter().any(|volume| volume_ids.contains(&volume.id) && volume.sensitive))
    });
    let id = Uuid::new_v4().to_string();
    let job = Job {
        id: id.clone(), kind: "download".into(),
        title: if titles.len() == 1 { titles[0].clone() } else { format!("{} selected series", titles.len()) },
        sensitive, state: "queued".into(), completed_pages: 0,
        total_pages: Some(u32::try_from(pages.len()).map_err(internal)?), error: None, pages,
        prepare_ocr_after: request.prepare_ocr_after, reprocess: false,
        ocr_provider, self_hosted_url,
    };
    let selected_volumes: std::collections::HashSet<_> = job.pages.iter().map(|page| page.volume_id.as_str()).collect();
    for volume in guard.series.iter_mut().flat_map(|series| &mut series.volumes) {
        if selected_volumes.contains(volume.id.as_str()) { volume.download_state = DownloadState::Queued; }
    }
    guard.jobs.push(job.clone());
    state.persist(&guard)?;
    drop(guard);
    spawn_job_worker(state.clone(), id);
    Ok(Json(job))
}

fn spawn_job_worker(state: ForkApiState, id: String) {
    tokio::spawn(async move { run_job(state, id).await; });
}

async fn run_job(state: ForkApiState, id: String) {
    // A single background worker prevents multiple recovered or newly queued
    // jobs from flooding the local or self-hosted OCR engine concurrently.
    let worker = Arc::clone(&state.job_worker);
    let _worker_guard = worker.lock().await;
    let kind = state.inner.lock().ok().and_then(|guard| guard.jobs.iter().find(|job| job.id == id).map(|job| job.kind.clone()));
    match kind.as_deref() {
        Some("ocr") => run_ocr_job(state, id).await,
        Some("download") => run_download_job(state, id).await,
        _ => {}
    }
}

async fn run_ocr_job(state: ForkApiState, id: String) {
    {
        let Ok(mut guard) = state.inner.lock() else { return; };
        let Some(job) = guard.jobs.iter_mut().find(|job| job.id == id) else { return; };
        if job.state != "queued" { return; }
        job.state = "processing".into();
        let volume_ids: std::collections::HashSet<_> = job.pages.iter().map(|page| page.volume_id.clone()).collect();
        for volume in guard.series.iter_mut().flat_map(|series| &mut series.volumes) {
            if volume_ids.contains(&volume.id) { volume.ocr_state = OcrState::Processing; }
        }
        state.persist_soon();
    }
    loop {
        let next = {
            let Ok(guard) = state.inner.lock() else { return; };
            let Some(job) = guard.jobs.iter().find(|job| job.id == id) else { return; };
            if job.state != "processing" { return; }
            job.pages.iter().find(|page| page.state == "queued").cloned()
        };
        let Some(page) = next else { break; };
        let page_url = format!(
            "http://127.0.0.1:14568/api/fork/volumes/{}/pages/{}?fingerprint={}",
            urlencoding::encode(&page.volume_id), page.page, urlencoding::encode(&page.fingerprint),
        );
        let mut endpoint = reqwest::Url::parse("http://127.0.0.1:14568/api/ocr/ocr").expect("static OCR URL");
        endpoint.query_pairs_mut().append_pair("url", &page_url).append_pair("language", "japanese");
        let (force_refresh, ocr_provider, self_hosted_url) = state.inner.lock().ok()
            .and_then(|guard| guard.jobs.iter().find(|job| job.id == id)
                .map(|job| (job.reprocess, job.ocr_provider.clone(), job.self_hosted_url.clone())))
            .unwrap_or((false, None, None));
        if force_refresh { endpoint.query_pairs_mut().append_pair("refresh", "true"); }
        if let Some(provider) = ocr_provider.as_deref() {
            endpoint.query_pairs_mut().append_pair("provider", provider);
        }
        if let Some(remote_url) = self_hosted_url.as_deref() {
            endpoint.query_pairs_mut().append_pair("remote_url", remote_url);
        }
        // OCR of one page can legitimately take a while on a slow phone or a
        // remote engine, but a hung request must not wedge the queue forever.
        let result = state.http.get(endpoint).timeout(Duration::from_secs(300)).send().await
            .and_then(|response| response.error_for_status());
        let remaining = {
            let mut guard = match state.inner.lock() { Ok(guard) => guard, Err(_) => return };
            let Some(job) = guard.jobs.iter_mut().find(|job| job.id == id) else { return; };
            if let Some(stored) = job.pages.iter_mut().find(|stored| stored.volume_id == page.volume_id && stored.page == page.page && stored.fingerprint == page.fingerprint) {
                match result {
                    Ok(_) => { stored.state = "complete".into(); stored.error = None; }
                    Err(error) => { stored.state = "failed".into(); stored.error = Some(error.to_string()); }
                }
            }
            job.completed_pages = job.pages.iter().filter(|page| page.state == "complete").count() as u32;
            let failed = job.pages.iter().filter(|page| page.state == "failed").count();
            let remaining = job.pages.iter().any(|page| page.state == "queued");
            // A pause or cancel issued while this page was in flight wins.
            if !remaining && job.state == "processing" {
                job.state = if failed == 0 { "complete" } else if job.completed_pages == 0 { "failed" } else { "partial" }.into();
                job.error = (failed > 0).then(|| format!("{failed} page(s) failed; retry missing pages"));
            }
            update_ocr_rollups(&mut guard, &id);
            state.persist_soon();
            remaining
        };
        if !remaining { break; }
        // Yield briefly so an interactive reader request is not starved by a
        // long background volume preparation job.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

fn update_ocr_rollups(state: &mut ForkState, job_id: &str) {
    let Some(job) = state.jobs.iter().find(|job| job.id == job_id) else { return; };
    let mut updates = Vec::new();
    for volume_id in job.pages.iter().map(|page| &page.volume_id).collect::<std::collections::HashSet<_>>() {
        let pages: Vec<_> = job.pages.iter().filter(|page| &page.volume_id == volume_id).collect();
        let complete = pages.iter().filter(|page| page.state == "complete").count();
        let failed = pages.iter().filter(|page| page.state == "failed").count();
        let value = if complete == pages.len() { OcrState::Ready }
            else if job.state == "cancelled" { if complete == 0 { OcrState::NotPrepared } else { OcrState::Partial } }
            else if job.state == "paused" { OcrState::Partial }
            else if failed == pages.len() { OcrState::Failed }
            else if failed > 0 { OcrState::Partial }
            else if job.state == "queued" { OcrState::Queued }
            else { OcrState::Processing };
        updates.push(((*volume_id).clone(), value));
    }
    for volume in state.series.iter_mut().flat_map(|series| &mut series.volumes) {
        if let Some((_, value)) = updates.iter().find(|(id, _)| id == &volume.id) { volume.ocr_state = *value; }
    }
}

async fn run_download_job(state: ForkApiState, id: String) {
    {
        let Ok(mut guard) = state.inner.lock() else { return; };
        let Some(job) = guard.jobs.iter_mut().find(|job| job.id == id) else { return; };
        if job.state != "queued" { return; }
        job.state = "processing".into();
        let volume_ids: std::collections::HashSet<_> = job.pages.iter().map(|page| page.volume_id.clone()).collect();
        for volume in guard.series.iter_mut().flat_map(|series| &mut series.volumes) {
            if volume_ids.contains(&volume.id) { volume.download_state = DownloadState::Downloading; }
        }
        state.persist_soon();
    }
    loop {
        let next = {
            let Ok(guard) = state.inner.lock() else { return; };
            let Some(job) = guard.jobs.iter().find(|job| job.id == id) else { return; };
            if job.state != "processing" { return; }
            job.pages.iter().find(|page| page.state == "queued").cloned()
        };
        let Some(page) = next else { break; };
        let page_url = format!(
            "http://127.0.0.1:14568/api/fork/volumes/{}/pages/{}?fingerprint={}",
            urlencoding::encode(&page.volume_id), page.page, urlencoding::encode(&page.fingerprint),
        );
        let result = async {
            let response = state.http.get(page_url).header(EXPECTED_FINGERPRINT_HEADER, &page.fingerprint).timeout(Duration::from_secs(300)).send().await.map_err(connection)?.error_for_status().map_err(connection)?;
            if response.content_length().is_some_and(|length| length > MAX_PAGE_BYTES) {
                return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "Page exceeds 64 MiB safety limit".into()));
            }
            let bytes = response.bytes().await.map_err(connection)?;
            if bytes.len() as u64 > MAX_PAGE_BYTES { return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "Page exceeds 64 MiB safety limit".into())); }
            let record = OfflinePage { volume_id: page.volume_id.clone(), page: page.page, fingerprint: page.fingerprint.clone() };
            let path = offline_page_path(&state.offline_dir, &record);
            tokio::task::spawn_blocking(move || -> Result<(), ApiError> {
                let temporary = path.with_extension("tmp");
                fs::write(&temporary, &bytes).map_err(internal)?;
                fs::rename(&temporary, &path).map_err(internal)
            }).await.map_err(internal)??;
            Ok(record)
        }.await;
        let mut guard = match state.inner.lock() { Ok(guard) => guard, Err(_) => return };
        let Some(job) = guard.jobs.iter_mut().find(|job| job.id == id) else { return; };
        if let Some(stored) = job.pages.iter_mut().find(|stored| stored.volume_id == page.volume_id && stored.page == page.page && stored.fingerprint == page.fingerprint) {
            match &result {
                Ok(_) => { stored.state = "complete".into(); stored.error = None; }
                Err(error) => { stored.state = "failed".into(); stored.error = Some(error.1.clone()); }
            }
        }
        if let Ok(record) = result {
            guard.offline_pages.retain(|stored| !(stored.volume_id == record.volume_id && stored.page == record.page));
            guard.offline_pages.push(record);
        }
        let Some(job) = guard.jobs.iter_mut().find(|job| job.id == id) else { return; };
        job.completed_pages = job.pages.iter().filter(|page| page.state == "complete").count() as u32;
        let failed = job.pages.iter().filter(|page| page.state == "failed").count();
        let remaining = job.pages.iter().any(|page| page.state == "queued");
        if !remaining && job.state == "processing" {
            job.state = if failed == 0 { "complete" } else if job.completed_pages == 0 { "failed" } else { "partial" }.into();
            job.error = (failed > 0).then(|| format!("{failed} page(s) failed; retry missing pages"));
        }
        update_download_rollups(&mut guard, &id);
        state.persist_soon();
        if !remaining { break; }
    }
    queue_download_ocr_follow_up(&state, &id);
}

fn queue_download_ocr_follow_up(state: &ForkApiState, download_id: &str) {
    let Ok(mut guard) = state.inner.lock() else { return; };
    let Some(download) = guard.jobs.iter().find(|job| job.id == download_id && job.state == "complete" && job.prepare_ocr_after).cloned() else { return; };
    let active: std::collections::HashSet<_> = guard.jobs.iter()
        .filter(|job| job.kind == "ocr" && matches!(job.state.as_str(), "queued" | "processing" | "paused"))
        .flat_map(|job| job.pages.iter().map(|page| (page.volume_id.clone(), page.page, page.fingerprint.clone())))
        .collect();
    let ready: std::collections::HashSet<_> = guard.series.iter().flat_map(|series| &series.volumes)
        .filter(|volume| volume.ocr_state == OcrState::Ready).map(|volume| volume.id.clone()).collect();
    let pages = download.pages.iter().filter(|page| !ready.contains(&page.volume_id) && !active.contains(&(page.volume_id.clone(), page.page, page.fingerprint.clone())))
        .map(|page| JobPage { volume_id: page.volume_id.clone(), page: page.page, fingerprint: page.fingerprint.clone(), state: "queued".into(), error: None }).collect::<Vec<_>>();
    if pages.is_empty() { return; }
    let id = Uuid::new_v4().to_string();
    let volume_ids: std::collections::HashSet<_> = pages.iter().map(|page| page.volume_id.clone()).collect();
    let job = Job {
        id: id.clone(), kind: "ocr".into(), title: download.title, sensitive: download.sensitive,
        state: "queued".into(), completed_pages: 0, total_pages: Some(pages.len() as u32), error: None,
        pages, prepare_ocr_after: false, reprocess: false,
        ocr_provider: download.ocr_provider, self_hosted_url: download.self_hosted_url,
    };
    for volume in guard.series.iter_mut().flat_map(|series| &mut series.volumes) {
        if volume_ids.contains(&volume.id) { volume.ocr_state = OcrState::Queued; }
    }
    guard.jobs.push(job);
    let _ = state.persist(&guard);
    drop(guard);
    spawn_job_worker(state.clone(), id);
}

fn update_download_rollups(state: &mut ForkState, job_id: &str) {
    let Some(job) = state.jobs.iter().find(|job| job.id == job_id) else { return; };
    let mut updates = Vec::new();
    for volume_id in job.pages.iter().map(|page| &page.volume_id).collect::<std::collections::HashSet<_>>() {
        let pages: Vec<_> = job.pages.iter().filter(|page| &page.volume_id == volume_id).collect();
        let complete = pages.iter().filter(|page| page.state == "complete").count();
        let failed = pages.iter().filter(|page| page.state == "failed").count();
        let value = if complete == pages.len() { DownloadState::Downloaded }
            else if job.state == "cancelled" { if complete == 0 { DownloadState::NotDownloaded } else { DownloadState::Partial } }
            else if job.state == "paused" { DownloadState::Partial }
            else if failed == pages.len() { DownloadState::Failed }
            else if failed > 0 { DownloadState::Partial }
            else if job.state == "queued" { DownloadState::Queued }
            else { DownloadState::Downloading };
        updates.push(((*volume_id).clone(), value));
    }
    for volume in state.series.iter_mut().flat_map(|series| &mut series.volumes) {
        if let Some((_, value)) = updates.iter().find(|(id, _)| id == &volume.id) { volume.download_state = *value; }
    }
}

fn offline_page_path(directory: &Path, page: &OfflinePage) -> PathBuf {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    page.volume_id.hash(&mut hash);
    page.page.hash(&mut hash);
    page.fingerprint.hash(&mut hash);
    directory.join(format!("{:016x}.page", hash.finish()))
}

async fn export_backup(State(state): State<ForkApiState>) -> Result<Json<BackupPayload>, ApiError> {
    let mut value = state.inner.lock().map_err(internal)?.clone();
    value.stats = calculate_stats(&value.events, value.show_sensitive, Local::now().date_naive());
    value.offline_pages.clear();
    let local_sources: std::collections::HashSet<_> = value.sources.iter().filter(|source| source.kind == SourceKind::Local).map(|source| source.id.clone()).collect();
    for series in &mut value.series {
        if !local_sources.contains(&series.source_id) {
            for volume in &mut series.volumes { volume.download_state = DownloadState::NotDownloaded; }
        }
    }
    for source in &mut value.sources {
        source.credential_ref = None;
        source.detail = None;
    }
    Ok(Json(BackupPayload { schema_version: 1, state: value }))
}

async fn restore_backup(State(state): State<ForkApiState>, Json(mut backup): Json<BackupPayload>) -> Result<Json<StateView>, ApiError> {
    if backup.schema_version != 1 { return Err(ApiError(StatusCode::BAD_REQUEST, "Unsupported backup version".into())); }
    if backup.state.sources.len() > 1_000 || backup.state.jobs.len() > 10_000 || backup.state.events.len() > 1_000_000 {
        return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "Backup exceeds safety limits".into()));
    }
    let mut ids = std::collections::HashSet::new();
    if backup.state.sources.iter().any(|source| !ids.insert(source.id.clone())) {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Backup contains duplicate source identities".into()));
    }
    for source in &mut backup.state.sources {
        source.credential_ref = None;
        source.detail = Some("Re-authorisation required after restore".into());
        source.status = if source.kind == SourceKind::Local { SourceStatus::NeedsPermission } else { SourceStatus::Offline };
    }
    for job in &mut backup.state.jobs {
        if matches!(job.state.as_str(), "queued" | "processing") { job.state = "paused".into(); }
    }
    backup.state.offline_pages.clear();
    let local_sources: std::collections::HashSet<_> = backup.state.sources.iter().filter(|source| source.kind == SourceKind::Local).map(|source| source.id.clone()).collect();
    for series in &mut backup.state.series {
        if !local_sources.contains(&series.source_id) {
            for volume in &mut series.volumes { volume.download_state = DownloadState::NotDownloaded; }
        }
    }
    backup.state.stats = calculate_stats(&backup.state.events, backup.state.show_sensitive, Local::now().date_naive());
    let mut guard = state.inner.lock().map_err(internal)?;
    state.persist(&backup.state)?;
    *guard = backup.state;
    Ok(Json(state_view(&guard)))
}

/// The UI-facing summary of the fork state. The persisted state also holds the
/// whole catalogue with per-page locators, the statistics event log and per-page
/// job records; none of those are needed by the polling UI, and serialising
/// them on every poll made large libraries slow.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StateView {
    show_sensitive: bool,
    sources: Vec<Source>,
    categories: Vec<Category>,
    jobs: Vec<JobView>,
    stats: StatsRanges,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JobView {
    id: String,
    kind: String,
    title: String,
    sensitive: bool,
    state: String,
    completed_pages: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_pages: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn state_view(guard: &ForkState) -> StateView {
    StateView {
        show_sensitive: guard.show_sensitive,
        sources: guard.sources.clone(),
        categories: guard.categories.clone(),
        jobs: guard.jobs.iter().map(|job| JobView {
            id: job.id.clone(),
            kind: job.kind.clone(),
            title: if job.sensitive && !guard.show_sensitive { "Hidden manga".into() } else { job.title.clone() },
            sensitive: job.sensitive,
            state: job.state.clone(),
            completed_pages: job.completed_pages,
            total_pages: job.total_pages,
            error: job.error.clone(),
        }).collect(),
        stats: calculate_stats(&guard.events, guard.show_sensitive, Local::now().date_naive()),
    }
}

/// Catalogue entries visible under the current privacy setting, without the
/// internal page locators (pages are fetched by index through `get_page`).
fn visible_catalogue(guard: &ForkState) -> Vec<Series> {
    guard.series.iter()
        .filter(|series| guard.show_sensitive || (!series.sensitive && series.volumes.iter().any(|volume| !volume.sensitive)))
        .map(|series| Series {
            id: series.id.clone(),
            source_id: series.source_id.clone(),
            title: series.title.clone(),
            sensitive: series.sensitive,
            category_ids: series.category_ids.clone(),
            volumes: series.volumes.iter()
                .filter(|volume| guard.show_sensitive || !volume.sensitive)
                .map(Volume::clone_without_pages)
                .collect(),
        })
        .collect()
}

impl Volume {
    fn clone_without_pages(&self) -> Volume {
        Volume {
            id: self.id.clone(),
            title: self.title.clone(),
            page_count: self.page_count,
            fingerprint: self.fingerprint.clone(),
            pages: Vec::new(),
            sensitive: self.sensitive,
            progress_page: self.progress_page,
            ocr_state: self.ocr_state,
            download_state: self.download_state,
            error: self.error.clone(),
        }
    }
}

async fn get_state(State(state): State<ForkApiState>) -> Result<Json<StateView>, ApiError> {
    let guard = state.inner.lock().map_err(internal)?;
    Ok(Json(state_view(&guard)))
}

async fn set_privacy(State(state): State<ForkApiState>, Json(request): Json<PrivacyRequest>) -> Result<Json<StateView>, ApiError> {
    let mut guard = state.inner.lock().map_err(internal)?;
    guard.show_sensitive = request.show_sensitive;
    guard.stats = calculate_stats(&guard.events, guard.show_sensitive, Local::now().date_naive());
    state.persist(&guard)?;
    Ok(Json(state_view(&guard)))
}

async fn add_source(State(state): State<ForkApiState>, Json(request): Json<SourceRequest>) -> Result<Json<Source>, ApiError> {
    validate_source(&request)?;
    if request.kind == SourceKind::Komga && request.library_ids.is_empty() {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Test the connection and select at least one Komga library".into()));
    }
    let tested = if request.kind == SourceKind::Local { None } else { Some(test_connection(&request).await?) };
    let id = Uuid::new_v4().to_string();
    let credential_ref = if request.password.as_deref().is_some_and(|value| !value.is_empty()) || request.api_key.as_deref().is_some_and(|value| !value.is_empty()) {
        let payload = serde_json::json!({ "username": &request.username, "password": request.password.as_deref(), "apiKey": request.api_key.as_deref() }).to_string();
        secure_store_put(&id, &payload)?;
        Some(id.clone())
    } else { None };
    let source = Source {
        id, kind: request.kind, name: request.name.trim().to_string(),
        status: if request.kind == SourceKind::Local && !request.address.starts_with("content://") { SourceStatus::NeedsPermission } else { SourceStatus::Connected },
        detail: tested.map(|value| value.detail),
        address: (!request.address.is_empty()).then_some(request.address), allow_http: request.allow_http, credential_ref,
        library_ids: request.library_ids,
        grouping_overrides: std::collections::HashMap::new(),
    };
    let mut guard = state.inner.lock().map_err(internal)?;
    guard.sources.push(source.clone());
    state.persist(&guard)?;
    Ok(Json(source))
}

async fn remove_source(
    State(state): State<ForkApiState>,
    AxumPath(id): AxumPath<String>,
) -> Result<StatusCode, ApiError> {
    let (credential_ref, local_address, offline_files) = {
        let mut guard = state.inner.lock().map_err(internal)?;
        let index = guard
            .sources
            .iter()
            .position(|source| source.id == id)
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Source not found".into()))?;
        let source = guard.sources.remove(index);
        let volume_ids: std::collections::HashSet<_> = guard.series.iter().filter(|series| series.source_id == id)
            .flat_map(|series| series.volumes.iter().map(|volume| volume.id.clone())).collect();
        let offline_files = guard.offline_pages.iter().filter(|page| volume_ids.contains(&page.volume_id))
            .map(|page| offline_page_path(&state.offline_dir, page)).collect::<Vec<_>>();
        guard.offline_pages.retain(|page| !volume_ids.contains(&page.volume_id));
        guard.series.retain(|series| series.source_id != id);
        state.persist(&guard)?;
        let local_address = (source.kind == SourceKind::Local).then_some(source.address).flatten();
        (source.credential_ref, local_address, offline_files)
    };
    forget_source_connections(&state, &id);
    if let Some(reference) = credential_ref {
        secure_store_delete(&reference)?;
    }
    if let Some(address) = local_address {
        release_android_grant(&address)?;
    }
    for file in offline_files { let _ = fs::remove_file(file); }
    Ok(StatusCode::NO_CONTENT)
}

async fn get_catalogue(State(state): State<ForkApiState>) -> Result<Json<Vec<Series>>, ApiError> {
    let guard = state.inner.lock().map_err(internal)?;
    Ok(Json(visible_catalogue(&guard)))
}

async fn add_category(State(state): State<ForkApiState>, Json(request): Json<CategoryRequest>) -> Result<Json<Category>, ApiError> {
    let name = request.name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Category name must be 1–80 characters".into()));
    }
    let mut guard = state.inner.lock().map_err(internal)?;
    if guard.categories.iter().any(|category| category.name.eq_ignore_ascii_case(name)) {
        return Err(ApiError(StatusCode::CONFLICT, "A category with that name already exists".into()));
    }
    let category = Category { id: Uuid::new_v4().to_string(), name: name.into(), order: guard.categories.len() as u32 };
    guard.categories.push(category.clone());
    state.persist(&guard)?;
    Ok(Json(category))
}

async fn rename_category(State(state): State<ForkApiState>, AxumPath(id): AxumPath<String>, Json(request): Json<CategoryRequest>) -> Result<Json<Category>, ApiError> {
    let name = request.name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Category name must be 1–80 characters".into()));
    }
    let mut guard = state.inner.lock().map_err(internal)?;
    if guard.categories.iter().any(|category| category.id != id && category.name.eq_ignore_ascii_case(name)) {
        return Err(ApiError(StatusCode::CONFLICT, "A category with that name already exists".into()));
    }
    let category = guard.categories.iter_mut().find(|category| category.id == id)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Category not found".into()))?;
    category.name = name.into();
    let response = category.clone();
    state.persist(&guard)?;
    Ok(Json(response))
}

async fn remove_category(State(state): State<ForkApiState>, AxumPath(id): AxumPath<String>) -> Result<StatusCode, ApiError> {
    let mut guard = state.inner.lock().map_err(internal)?;
    let count = guard.categories.len();
    guard.categories.retain(|category| category.id != id);
    if count == guard.categories.len() { return Err(ApiError(StatusCode::NOT_FOUND, "Category not found".into())); }
    for series in &mut guard.series { series.category_ids.retain(|category_id| category_id != &id); }
    state.persist(&guard)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn reorder_categories(State(state): State<ForkApiState>, Json(request): Json<CategoryOrderRequest>) -> Result<StatusCode, ApiError> {
    let mut guard = state.inner.lock().map_err(internal)?;
    let current: std::collections::HashSet<_> = guard.categories.iter().map(|category| category.id.as_str()).collect();
    let requested: std::collections::HashSet<_> = request.ids.iter().map(String::as_str).collect();
    if request.ids.len() != guard.categories.len() || current != requested {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Category order must contain every category exactly once".into()));
    }
    let positions: std::collections::HashMap<_, _> = request.ids.iter().enumerate().map(|(index, id)| (id.as_str(), index)).collect();
    guard.categories.sort_by_key(|category| positions[category.id.as_str()]);
    for (index, category) in guard.categories.iter_mut().enumerate() { category.order = index as u32; }
    state.persist(&guard)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_series_sensitive(State(state): State<ForkApiState>, Json(request): Json<SeriesSensitiveRequest>) -> Result<StatusCode, ApiError> {
    if request.series_ids.is_empty() || request.series_ids.len() > 10_000 {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Select between 1 and 10,000 series".into()));
    }
    let selected: std::collections::HashSet<_> = request.series_ids.iter().collect();
    let mut guard = state.inner.lock().map_err(internal)?;
    for series in &mut guard.series { if selected.contains(&series.id) { series.sensitive = request.sensitive; } }
    state.persist(&guard)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_volume_sensitive(State(state): State<ForkApiState>, Json(request): Json<VolumeSensitiveRequest>) -> Result<StatusCode, ApiError> {
    if request.volume_ids.is_empty() || request.volume_ids.len() > 100_000 {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Select between 1 and 100,000 volumes".into()));
    }
    let selected: std::collections::HashSet<_> = request.volume_ids.iter().collect();
    let mut guard = state.inner.lock().map_err(internal)?;
    for volume in guard.series.iter_mut().flat_map(|series| &mut series.volumes) {
        if selected.contains(&volume.id) { volume.sensitive = request.sensitive; }
    }
    state.persist(&guard)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_series_category(State(state): State<ForkApiState>, Json(request): Json<SeriesCategoryRequest>) -> Result<StatusCode, ApiError> {
    let mut guard = state.inner.lock().map_err(internal)?;
    if !guard.categories.iter().any(|category| category.id == request.category_id) {
        return Err(ApiError(StatusCode::NOT_FOUND, "Category not found".into()));
    }
    let selected: std::collections::HashSet<_> = request.series_ids.iter().collect();
    for series in &mut guard.series {
        if !selected.contains(&series.id) { continue; }
        series.category_ids.retain(|id| id != &request.category_id);
        if request.assigned { series.category_ids.push(request.category_id.clone()); }
    }
    state.persist(&guard)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn scan_source(
    State(state): State<ForkApiState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Vec<Series>>, ApiError> {
    apply_scan(&state, &id, ApplyScanRequest::default()).await.map(Json)
}

// This is intentionally additive. A pull-to-refresh is a convenient way to
// pick up files copied into a local folder (for example by Calibre), not a
// destructive reconciliation when Android has not enumerated every document.
async fn refresh_source(
    State(state): State<ForkApiState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<IncrementalRefresh>, ApiError> {
    let source = state.inner.lock().map_err(internal)?.sources.iter().find(|source| source.id == id)
        .cloned().ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Source not found".into()))?;
    if source.kind != SourceKind::Local {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Incremental refresh is available for local folders only".into()));
    }
    let mut scanned = collect_scan(&state, &id).await?;
    apply_grouping_overrides(&mut scanned, &source.grouping_overrides);
    let mut guard = state.inner.lock().map_err(internal)?;
    let mut result = IncrementalRefresh { new_series: 0, new_volumes: 0 };
    for item in scanned.series {
        if let Some(existing) = guard.series.iter_mut().find(|series| series.id == item.id && series.source_id == id) {
            for volume in item.volumes {
                if existing.volumes.iter().any(|old| old.id == volume.id) { continue; }
                existing.volumes.push(new_volume(volume));
                result.new_volumes += 1;
            }
            existing.volumes.sort_by(|left, right| manga_reader_core::natural_cmp(&left.title, &right.title));
        } else {
            let volume_count = item.volumes.len();
            guard.series.push(Series {
                id: item.id,
                source_id: id.clone(),
                title: item.title,
                sensitive: false,
                category_ids: Vec::new(),
                volumes: item.volumes.into_iter().map(new_volume).collect(),
            });
            result.new_series += 1;
            result.new_volumes += volume_count;
        }
    }
    guard.series.sort_by(|left, right| manga_reader_core::natural_cmp(&left.title, &right.title));
    if let Some(source) = guard.sources.iter_mut().find(|source| source.id == id) {
        source.status = SourceStatus::Connected;
        source.detail = Some(if result.new_volumes == 0 {
            "No new volumes found".into()
        } else {
            format!("Added {} new volume{}", result.new_volumes, if result.new_volumes == 1 { "" } else { "s" })
        });
    }
    state.persist(&guard)?;
    Ok(Json(result))
}

fn new_volume(volume: ScannedVolume) -> Volume {
    Volume {
        id: volume.id,
        title: volume.title,
        page_count: volume.page_count,
        fingerprint: volume.fingerprint,
        pages: volume.pages,
        sensitive: false,
        progress_page: 0,
        ocr_state: OcrState::NotPrepared,
        download_state: DownloadState::Downloaded,
        error: volume.error,
    }
}

async fn preview_source(
    State(state): State<ForkApiState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<ScanPreview>, ApiError> {
    let mut scanned = collect_scan(&state, &id).await?;
    let overrides = state.inner.lock().map_err(internal)?.sources.iter().find(|source| source.id == id)
        .map(|source| source.grouping_overrides.clone()).unwrap_or_default();
    apply_grouping_overrides(&mut scanned, &overrides);
    Ok(Json(ScanPreview { series: scanned.series.into_iter().map(|series| ScanPreviewItem {
        id: series.id,
        title: series.title,
        volume_count: series.volumes.len(),
        page_count: series.volumes.iter().map(|volume| volume.page_count).sum(),
        warnings: series.volumes.iter().filter(|volume| volume.error.is_some()).count(),
    }).collect() }))
}

async fn apply_source_scan(
    State(state): State<ForkApiState>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<ApplyScanRequest>,
) -> Result<Json<Vec<Series>>, ApiError> {
    apply_scan(&state, &id, request).await.map(Json)
}

async fn collect_scan(state: &ForkApiState, id: &str) -> Result<ScanPayload, ApiError> {
    let source = state.inner.lock().map_err(internal)?.sources.iter().find(|source| source.id == id)
        .cloned().ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Source not found".into()))?;
    let scanned_result = match source.kind {
        SourceKind::Local => {
            let uri = source.address.clone().ok_or_else(|| ApiError(StatusCode::CONFLICT, "Re-authorise this folder before scanning".into()))?;
            tokio::task::spawn_blocking(move || scan_local_source(&uri)).await.map_err(internal)?
        }
        SourceKind::Komga => scan_komga(state, &source).await,
        SourceKind::Smb => scan_smb(state, &source).await,
        SourceKind::Webdav => scan_webdav(state, &source).await,
    };
    let scanned = match scanned_result {
        Ok(scanned) => scanned,
        Err(error) => {
            if let Ok(mut guard) = state.inner.lock() {
                if let Some(stored) = guard.sources.iter_mut().find(|stored| stored.id == id) {
                    stored.status = SourceStatus::Offline;
                    stored.detail = Some(error.1.clone());
                }
                let _ = state.persist(&guard);
            }
            return Err(error);
        }
    };
    if scanned.series.len() > 10_000 || scanned.series.iter().map(|series| series.volumes.len()).sum::<usize>() > 100_000 {
        return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "Scan exceeds catalogue safety limits".into()));
    }
    Ok(scanned)
}

fn scan_local_source(uri: &str) -> Result<ScanPayload, ApiError> {
    let mut scanned: ScanPayload = serde_json::from_str(&scan_android_documents(uri)?).map_err(internal)?;
    for series in &mut scanned.series {
        for volume in &mut series.volumes {
            if let Some(archive_uri) = volume.archive_uri.take() {
                match archive_android_pages(&archive_uri) {
                    Ok((pages, fingerprint)) => {
                        volume.page_count = pages.len().try_into().map_err(internal)?;
                        volume.pages = pages;
                        volume.fingerprint = fingerprint;
                    }
                    Err(_) => {
                        volume.page_count = 0;
                        volume.pages.clear();
                        volume.fingerprint = format!("unreadable:{}", volume.id);
                        volume.error = Some("Archive is not a readable ZIP/CBZ file".into());
                    }
                }
            }
        }
        series.volumes.retain(|volume| volume.page_count > 0 || volume.error.is_some());
    }
    Ok(scanned)
}

async fn apply_scan(state: &ForkApiState, id: &str, request: ApplyScanRequest) -> Result<Vec<Series>, ApiError> {
    if request.title_overrides.len() > 10_000 || request.title_overrides.values().any(|title| title.trim().is_empty() || title.chars().count() > 200) {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Grouping titles must be 1–200 characters".into()));
    }
    let mut scanned = collect_scan(state, id).await?;
    let (source_kind, mut grouping_overrides) = state.inner.lock().map_err(internal)?.sources.iter().find(|source| source.id == id)
        .map(|source| (source.kind, source.grouping_overrides.clone())).ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Source not found".into()))?;
    grouping_overrides.extend(request.title_overrides.into_iter().map(|(id, title)| (id, title.trim().into())));
    apply_grouping_overrides(&mut scanned, &grouping_overrides);
    let mut guard = state.inner.lock().map_err(internal)?;
    let mut replacement = Vec::with_capacity(scanned.series.len());
    for item in scanned.series {
        let previous = guard.series.iter().find(|series| series.id == item.id && series.source_id == id);
        let volumes = item.volumes.into_iter().map(|volume| {
            let old = previous.and_then(|series| series.volumes.iter().find(|old| old.id == volume.id));
            let unchanged = old.is_some_and(|old| old.fingerprint == volume.fingerprint);
            Volume {
                id: volume.id,
                title: volume.title,
                page_count: volume.page_count,
                fingerprint: volume.fingerprint,
                pages: volume.pages,
                sensitive: old.is_some_and(|old| old.sensitive),
                progress_page: old.map_or(volume.progress_page, |old| old.progress_page.min(volume.page_count.saturating_sub(1))),
                ocr_state: if unchanged { old.map_or(OcrState::NotPrepared, |old| old.ocr_state) } else if old.is_some() { OcrState::OutOfDate } else { OcrState::NotPrepared },
                download_state: if source_kind == SourceKind::Local {
                    DownloadState::Downloaded
                } else if unchanged {
                    old.map_or(DownloadState::NotDownloaded, |old| old.download_state)
                } else if old.is_some() {
                    DownloadState::OutOfDate
                } else {
                    DownloadState::NotDownloaded
                },
                error: volume.error,
            }
        }).collect();
        replacement.push(Series {
            id: item.id,
            source_id: id.to_string(),
            title: item.title,
            sensitive: previous.is_some_and(|series| series.sensitive),
            category_ids: previous.map_or_else(Vec::new, |series| series.category_ids.clone()),
            volumes,
        });
    }
    guard.series.retain(|series| series.source_id != id);
    guard.series.extend(replacement);
    guard.series.sort_by(|left, right| manga_reader_core::natural_cmp(&left.title, &right.title));
    if let Some(source) = guard.sources.iter_mut().find(|source| source.id == id) {
        source.status = SourceStatus::Connected;
        source.detail = Some("Catalogue refreshed".into());
        source.grouping_overrides = grouping_overrides;
    }
    state.persist(&guard)?;
    // Respect the privacy setting: a rescan response must not reveal hidden
    // series or volumes that the catalogue endpoint would exclude.
    Ok(visible_catalogue(&guard))
}

fn apply_grouping_overrides(scanned: &mut ScanPayload, overrides: &std::collections::HashMap<String, String>) {
    for series in &mut scanned.series {
        if let Some(title) = overrides.get(&series.id) { series.title = title.clone(); }
    }
    scanned.series.sort_by(|left, right| manga_reader_core::natural_cmp(&left.title, &right.title).then_with(|| left.id.cmp(&right.id)));
    let mut merged: Vec<ScannedSeries> = Vec::new();
    let mut by_title: HashMap<String, usize> = HashMap::new();
    for mut series in scanned.series.drain(..) {
        let key = series.title.to_ascii_lowercase();
        if let Some(&index) = by_title.get(&key) {
            merged[index].volumes.append(&mut series.volumes);
        } else {
            by_title.insert(key, merged.len());
            merged.push(series);
        }
    }
    for series in &mut merged {
        series.volumes.sort_by(|left, right| manga_reader_core::natural_cmp(&left.title, &right.title));
    }
    scanned.series = merged;
}

/// Sent by the offline download job so a volume rescanned after the job was
/// queued is not stored under its old content version. The `fingerprint`
/// query parameter is only a cache key (the OCR cache is keyed by page URL) and
/// is deliberately not enforced.
const EXPECTED_FINGERPRINT_HEADER: &str = "x-mangaki-expected-fingerprint";

/// Resolves a page to its locator, its stored offline copy (if any) and an
/// ETag derived from the volume's content fingerprint. Page URLs are stable
/// across rescans, so cached copies are validated against the fingerprint
/// instead of trusting a fixed lifetime.
fn page_target(state: &ForkApiState, headers: &axum::http::HeaderMap, id: &str, page: usize, variant: &str) -> Result<(String, Option<PathBuf>, String), ApiError> {
    let guard = state.inner.lock().map_err(internal)?;
    let volume = guard.series.iter().flat_map(|series| &series.volumes).find(|volume| volume.id == id)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Volume not found".into()))?;
    let expected = headers.get(EXPECTED_FINGERPRINT_HEADER).and_then(|value| value.to_str().ok());
    if expected.is_some_and(|expected| expected != volume.fingerprint) {
        return Err(ApiError(StatusCode::CONFLICT, "Volume content changed since this job was queued".into()));
    }
    let locator = volume.pages.get(page).cloned().ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Page not found".into()))?;
    let record = OfflinePage { volume_id: volume.id.clone(), page: page.try_into().map_err(internal)?, fingerprint: volume.fingerprint.clone() };
    let offline = guard.offline_pages.iter().any(|stored| stored.volume_id == record.volume_id && stored.page == record.page && stored.fingerprint == record.fingerprint)
        .then(|| offline_page_path(&state.offline_dir, &record));
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    volume.id.hash(&mut hash);
    volume.fingerprint.hash(&mut hash);
    page.hash(&mut hash);
    variant.hash(&mut hash);
    Ok((locator, offline, format!("\"{:016x}\"", hash.finish())))
}

fn not_modified(headers: &axum::http::HeaderMap, etag: &str) -> Option<Result<Response, ApiError>> {
    let matches = headers.get(header::IF_NONE_MATCH).and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|tag| tag.trim() == etag));
    matches.then(|| Response::builder().status(StatusCode::NOT_MODIFIED)
        .header(header::ETAG, etag)
        .header(header::CACHE_CONTROL, "private, no-cache")
        .body(Body::empty()).map_err(internal))
}

async fn load_page(state: &ForkApiState, locator: String, offline: Option<PathBuf>) -> Result<Vec<u8>, ApiError> {
    if let Some(path) = offline {
        if let Ok(bytes) = tokio::fs::read(path).await { return Ok(bytes); }
    }
    if locator.starts_with("komga:") {
        read_komga_page(state, &locator).await
    } else if locator.starts_with("webdav:") || locator.starts_with("smb:") || locator.starts_with("rzip:") {
        read_remote_page(state, &locator).await
    } else {
        // Storage Access Framework reads are blocking JNI/file I/O.
        tokio::task::spawn_blocking(move || read_android_page(&locator)).await.map_err(internal)?
    }
}

async fn get_page(
    State(state): State<ForkApiState>,
    AxumPath((id, page)): AxumPath<(String, usize)>,
    headers: axum::http::HeaderMap,
) -> Result<Response, ApiError> {
    let (locator, offline, etag) = page_target(&state, &headers, &id, page, "page")?;
    if let Some(response) = not_modified(&headers, &etag) { return response; }
    let bytes = load_page(&state, locator, offline).await?;
    Response::builder().status(StatusCode::OK)
        .header(header::CONTENT_TYPE, image_mime(&bytes))
        .header(header::ETAG, etag)
        .header(header::CACHE_CONTROL, "private, no-cache")
        .body(Body::from(bytes)).map_err(internal)
}

/// A small JPEG of a volume's first page for library and volume grids, cached
/// on disk per content version. Decoding full-resolution pages for every cover
/// made large libraries slow to scroll.
async fn get_thumbnail(
    State(state): State<ForkApiState>,
    AxumPath(id): AxumPath<String>,
    headers: axum::http::HeaderMap,
) -> Result<Response, ApiError> {
    const THUMBNAIL_VARIANT: &str = "thumbnail-v1";
    let (locator, offline, etag) = page_target(&state, &headers, &id, 0, THUMBNAIL_VARIANT)?;
    if let Some(response) = not_modified(&headers, &etag) { return response; }
    let path = state.thumbnail_dir.join(format!("{}.jpg", etag.trim_matches('"')));
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(_) => {
            let _permit = state.thumbnail_permits.acquire().await.map_err(internal)?;
            let page = load_page(&state, locator, offline).await?;
            tokio::task::spawn_blocking(move || match make_thumbnail(&page) {
                Some(thumbnail) => {
                    let temporary = path.with_extension(format!("{}.tmp", Uuid::new_v4().simple()));
                    if fs::write(&temporary, &thumbnail).is_ok() && fs::rename(&temporary, &path).is_err() {
                        let _ = fs::remove_file(&temporary);
                    }
                    thumbnail
                }
                // Serve the page itself rather than a broken cover.
                None => page,
            }).await.map_err(internal)?
        }
    };
    Response::builder().status(StatusCode::OK)
        .header(header::CONTENT_TYPE, image_mime(&bytes))
        .header(header::ETAG, etag)
        .header(header::CACHE_CONTROL, "private, no-cache")
        .body(Body::from(bytes)).map_err(internal)
}

fn make_thumbnail(page: &[u8]) -> Option<Vec<u8>> {
    const MAX_WIDTH: u32 = 360;
    const MAX_HEIGHT: u32 = 540;
    let image = image::load_from_memory(page).ok()?;
    let thumbnail = if image.width() > MAX_WIDTH || image.height() > MAX_HEIGHT {
        image.thumbnail(MAX_WIDTH, MAX_HEIGHT)
    } else {
        image
    };
    let mut output = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, 82).encode_image(&thumbnail.to_rgb8()).ok()?;
    Some(output.into_inner())
}

fn image_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG") { "image/png" }
    else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" { "image/webp" }
    else if bytes.starts_with(b"GIF8") { "image/gif" }
    else { "image/jpeg" }
}

async fn read_komga_page(state: &ForkApiState, locator: &str) -> Result<Vec<u8>, ApiError> {
    let mut parts = locator.split(':');
    if parts.next() != Some("komga") { return Err(ApiError(StatusCode::BAD_REQUEST, "Invalid Komga page locator".into())); }
    let source_id = parts.next().ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "Invalid Komga source locator".into()))?;
    let book_id = parts.next().ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "Invalid Komga book locator".into()))?;
    let page: u32 = parts.next().ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "Invalid Komga page locator".into()))?.parse().map_err(connection)?;
    if parts.next().is_some() { return Err(ApiError(StatusCode::BAD_REQUEST, "Invalid Komga page locator".into())); }
    let source = state.inner.lock().map_err(internal)?.sources.iter().find(|source| source.id == source_id && source.kind == SourceKind::Komga)
        .cloned().ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Komga source is unavailable".into()))?;
    let base = source.address.as_deref().ok_or_else(|| ApiError(StatusCode::CONFLICT, "Komga address is missing".into()))?.trim_end_matches('/');
    let credentials = source_credentials(&source)?;
    let url = format!("{base}/api/v1/books/{}/pages/{page}?zero_based=true", urlencoding::encode(book_id));
    let response = authenticate_credentials(state.http.get(url).timeout(Duration::from_secs(60)), &credentials).send().await.map_err(connection)?;
    if !response.status().is_success() { return Err(ApiError(StatusCode::BAD_GATEWAY, format!("Komga page returned {}", response.status()))); }
    if response.content_length().is_some_and(|length| length > MAX_PAGE_BYTES) {
        return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "Page exceeds 64 MiB safety limit".into()));
    }
    let bytes = response.bytes().await.map_err(connection)?;
    if bytes.len() as u64 > MAX_PAGE_BYTES { return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "Page exceeds 64 MiB safety limit".into())); }
    Ok(bytes.to_vec())
}

const MAX_PAGE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;
const REMOTE_BLOCK_BYTES: u64 = 256 * 1024;
const REMOTE_TAIL_BYTES: u64 = 64 * 1024;
const OPEN_REMOTE_ARCHIVES: usize = 6;

async fn read_remote_page(state: &ForkApiState, locator: &str) -> Result<Vec<u8>, ApiError> {
    if let Some(value) = locator.strip_prefix("rzip:") {
        let (encoded, index) = value.split_once(':').ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "Invalid remote archive page locator".into()))?;
        let remote = String::from_utf8(URL_SAFE_NO_PAD.decode(encoded).map_err(connection)?).map_err(connection)?;
        let index: usize = index.parse().map_err(connection)?;
        return with_remote_zip(state, &remote, false, Some(index), move |archive| archive.read_entry(index)).await;
    }
    read_remote_bytes(state, locator, MAX_PAGE_BYTES).await
}

/// Lists image entries of a remote archive in natural reading order. Only the
/// archive's central directory is transferred when the server supports
/// positioned reads, so scanning a NAS no longer downloads every volume.
async fn remote_archive_pages(state: &ForkApiState, locator: &str, invalidate: bool) -> Result<Vec<usize>, ApiError> {
    with_remote_zip(state, locator, invalidate, None, RemoteZip::page_indexes).await
}

/// Runs `operation` against a remote archive on a blocking thread. For ranged
/// archives the entry about to be read is fetched in one request first. A
/// ranged archive that fails (for example because the file was replaced on the
/// server or the connection dropped) is reopened once before the error is
/// reported.
async fn with_remote_zip<T, F>(state: &ForkApiState, locator: &str, invalidate: bool, entry: Option<usize>, operation: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: Fn(&mut RemoteZip) -> Result<T, ApiError> + Clone + Send + 'static,
{
    if invalidate { forget_remote_archive(state, locator, true); }
    let mut archive = open_remote_zip(state, locator).await?;
    let ranged = matches!(archive, RemoteZip::Ranged(_));
    let result = match prefetch_entry(&archive, entry).await {
        Ok(()) => {
            let first = operation.clone();
            tokio::task::spawn_blocking(move || first(&mut archive)).await.map_err(internal)?
        }
        Err(error) => Err(error),
    };
    match result {
        Err(error) if ranged && error.0 != StatusCode::PAYLOAD_TOO_LARGE => {
            forget_remote_archive(state, locator, false);
            let mut archive = open_remote_zip(state, locator).await.map_err(|_| error)?;
            prefetch_entry(&archive, entry).await?;
            tokio::task::spawn_blocking(move || operation(&mut archive)).await.map_err(internal)?
        }
        other => other,
    }
}

async fn prefetch_entry(archive: &RemoteZip, entry: Option<usize>) -> Result<(), ApiError> {
    let RemoteZip::Ranged(open) = archive else { return Ok(()); };
    let Some(&(start, length)) = entry.and_then(|index| open.spans.get(index)).and_then(Option::as_ref) else { return Ok(()); };
    let reader = open.archive.clone().into_inner();
    if reader.blocks.lock().map_err(internal)?.find(start).is_some_and(|(first, data)| first + data.len() as u64 >= start + length) {
        return Ok(());
    }
    let data = reader.file.read_at(start, length).await.map_err(|error| ApiError(StatusCode::BAD_GATEWAY, error))?;
    reader.blocks.lock().map_err(internal)?.insert((start, Arc::new(data)));
    Ok(())
}

#[derive(Clone)]
struct OpenRemoteArchive {
    archive: zip::ZipArchive<RangeReader>,
    /// Absolute byte span of each entry (local header through compressed
    /// data), indexed like the archive, when the central directory could be
    /// parsed. Used to fetch a page in a single request.
    spans: Arc<Vec<Option<(u64, u64)>>>,
}

enum RemoteZip {
    Ranged(OpenRemoteArchive),
    Cached(zip::ZipArchive<std::io::BufReader<fs::File>>),
}

impl RemoteZip {
    fn page_indexes(&mut self) -> Result<Vec<usize>, ApiError> {
        match self {
            Self::Ranged(open) => Ok(zip_page_indexes(&open.archive)),
            Self::Cached(archive) => Ok(zip_page_indexes(archive)),
        }
    }

    fn read_entry(&mut self, index: usize) -> Result<Vec<u8>, ApiError> {
        match self {
            Self::Ranged(open) => zip_read_entry(&mut open.archive, index),
            Self::Cached(archive) => zip_read_entry(archive, index),
        }
    }
}

/// Uses only central-directory names, so listing a remote archive performs no
/// per-entry I/O.
fn zip_page_indexes<R: Read + std::io::Seek>(archive: &zip::ZipArchive<R>) -> Vec<usize> {
    let mut entries = (0..archive.len())
        .filter_map(|index| archive.name_for_index(index).map(|name| (name.to_string(), index)))
        .filter(|(name, _)| is_archive_page_name(name))
        .take(100_001)
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| manga_reader_core::natural_cmp(&left.0, &right.0));
    entries.into_iter().map(|(_, index)| index).collect()
}

fn zip_read_entry<R: Read + std::io::Seek>(archive: &mut zip::ZipArchive<R>, index: usize) -> Result<Vec<u8>, ApiError> {
    let entry = archive.by_index(index).map_err(connection)?;
    if entry.size() > MAX_PAGE_BYTES { return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "Page exceeds 64 MiB safety limit".into())); }
    let mut page = Vec::with_capacity(entry.size() as usize);
    entry.take(MAX_PAGE_BYTES + 1).read_to_end(&mut page).map_err(connection)?;
    if page.len() as u64 > MAX_PAGE_BYTES { return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "Page exceeds 64 MiB safety limit".into())); }
    Ok(page)
}

/// Reads local-header offsets and compressed sizes from the central directory.
/// Returns an empty list (no hints) for anything unusual such as ZIP64 values;
/// reads then fall back to block-sized requests and stay correct.
fn central_directory_spans<R: Read + std::io::Seek>(archive: &zip::ZipArchive<R>, reader: &mut R) -> Vec<Option<(u64, u64)>> {
    use std::io::SeekFrom;
    const SLACK: u64 = 4 * 1024;
    let parse = |reader: &mut R, start: u64| -> Option<Vec<Option<(u64, u64)>>> {
        reader.seek(SeekFrom::Start(start)).ok()?;
        let mut spans = Vec::with_capacity(archive.len());
        for _ in 0..archive.len() {
            let mut fixed = [0u8; 46];
            reader.read_exact(&mut fixed).ok()?;
            if fixed[..4] != [0x50, 0x4b, 0x01, 0x02] { return None; }
            let u16_at = |at: usize| u64::from(u16::from_le_bytes([fixed[at], fixed[at + 1]]));
            let u32_at = |at: usize| u32::from_le_bytes([fixed[at], fixed[at + 1], fixed[at + 2], fixed[at + 3]]);
            let (compressed, local) = (u32_at(20), u32_at(42));
            let (name, extra, comment) = (u16_at(28), u16_at(30), u16_at(32));
            reader.seek(SeekFrom::Current(i64::try_from(name + extra + comment).ok()?)).ok()?;
            spans.push((compressed != u32::MAX && local != u32::MAX)
                .then(|| (archive.offset() + u64::from(local), 30 + name + u64::from(compressed) + SLACK)));
        }
        Some(spans)
    };
    let start = archive.central_directory_start();
    parse(reader, start).or_else(|| parse(reader, start + archive.offset())).unwrap_or_default()
}

async fn open_remote_zip(state: &ForkApiState, locator: &str) -> Result<RemoteZip, ApiError> {
    if let Some(open) = cached_remote_zip(state, locator) { return Ok(RemoteZip::Ranged(open)); }
    let cache_path = remote_cache_path(state, locator);
    if !tokio::fs::metadata(&cache_path).await.is_ok_and(|metadata| metadata.is_file()) {
        match open_remote_file(state, locator).await? {
            RemoteProbe::Ranged(file) => {
                let tail = match &file {
                    RemoteFile::Webdav { tail, .. } => Some((tail.0, Arc::clone(&tail.1))),
                    RemoteFile::Smb(_) => {
                        // One request for the end of the archive covers the
                        // zip end record and, for typical volumes, the whole
                        // central directory.
                        let start = file.size().saturating_sub(REMOTE_TAIL_BYTES);
                        let data = file.read_at(start, REMOTE_TAIL_BYTES).await.map_err(|error| ApiError(StatusCode::BAD_GATEWAY, error))?;
                        Some((start, Arc::new(data)))
                    }
                };
                let reader = RangeReader::new(Arc::new(file), tokio::runtime::Handle::current(), tail);
                let open = tokio::task::spawn_blocking(move || -> Result<OpenRemoteArchive, ApiError> {
                    let archive = zip::ZipArchive::new(reader.clone())
                        .map_err(|error| ApiError(StatusCode::UNPROCESSABLE_ENTITY, format!("Remote archive is not a readable ZIP/CBZ file: {error}")))?;
                    let mut scratch = reader;
                    let spans = central_directory_spans(&archive, &mut scratch);
                    Ok(OpenRemoteArchive { archive, spans: Arc::new(spans) })
                }).await.map_err(internal)??;
                if let Ok(mut cache) = state.remote_archives.lock() {
                    cache.retain(|(key, _)| key != locator);
                    if cache.len() >= OPEN_REMOTE_ARCHIVES {
                        let (_, evicted) = cache.remove(0);
                        close_remote_archive(evicted);
                    }
                    cache.push((locator.to_string(), open.clone()));
                }
                return Ok(RemoteZip::Ranged(open));
            }
            // The server ignores HTTP ranges: keep the full response it already
            // started sending as one bounded download in the archive cache.
            RemoteProbe::Full(response) => store_remote_archive(state, locator, response).await?,
        }
    }
    let archive = tokio::task::spawn_blocking(move || -> Result<_, ApiError> {
        let file = fs::File::open(&cache_path).map_err(internal)?;
        // Touch the file so cache trimming evicts least recently used archives.
        let _ = file.set_modified(std::time::SystemTime::now());
        zip::ZipArchive::new(std::io::BufReader::new(file))
            .map_err(|error| ApiError(StatusCode::UNPROCESSABLE_ENTITY, format!("Remote archive is not a readable ZIP/CBZ file: {error}")))
    }).await.map_err(internal)??;
    Ok(RemoteZip::Cached(archive))
}

fn cached_remote_zip(state: &ForkApiState, locator: &str) -> Option<OpenRemoteArchive> {
    let mut cache = state.remote_archives.lock().ok()?;
    let index = cache.iter().position(|(key, _)| key == locator)?;
    let entry = cache.remove(index);
    let open = entry.1.clone();
    cache.push(entry);
    Some(open)
}

fn forget_remote_archive(state: &ForkApiState, locator: &str, include_disk_cache: bool) {
    if let Ok(mut cache) = state.remote_archives.lock() {
        if let Some(index) = cache.iter().position(|(key, _)| key == locator) {
            let (_, open) = cache.remove(index);
            close_remote_archive(open);
        }
    }
    if include_disk_cache { let _ = fs::remove_file(remote_cache_path(state, locator)); }
}

fn forget_source_connections(state: &ForkApiState, source_id: &str) {
    if let Ok(mut cache) = state.remote_archives.lock() {
        let (removed, kept): (Vec<_>, Vec<_>) = cache.drain(..).partition(|(key, _)| key.split(':').nth(1) == Some(source_id));
        *cache = kept;
        for (_, open) in removed { close_remote_archive(open); }
    }
    let sessions = Arc::clone(&state.smb_sessions);
    let source_id = source_id.to_string();
    tokio::spawn(async move { sessions.lock().await.remove(&source_id); });
}

/// Releases the server-side handle of an evicted SMB archive once no in-flight
/// request still shares it.
fn close_remote_archive(open: OpenRemoteArchive) {
    let file = open.archive.into_inner().file;
    if let Ok(RemoteFile::Smb(reader)) = Arc::try_unwrap(file) {
        tokio::spawn(async move { let _ = reader.close().await; });
    }
}

fn remote_cache_path(state: &ForkApiState, locator: &str) -> PathBuf {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    locator.hash(&mut hash);
    state.cache_dir.join(format!("{:016x}.zip", hash.finish()))
}

/// Streams a full archive response into the on-disk cache. Concurrent requests
/// for the same uncached archive share one download.
async fn store_remote_archive(state: &ForkApiState, locator: &str, mut response: reqwest::Response) -> Result<(), ApiError> {
    let cache_path = remote_cache_path(state, locator);
    let lock = {
        let mut downloads = state.archive_downloads.lock().await;
        Arc::clone(downloads.entry(cache_path.clone()).or_default())
    };
    let _download = lock.lock().await;
    if tokio::fs::metadata(&cache_path).await.is_ok_and(|metadata| metadata.is_file()) { return Ok(()); }
    let too_large = || ApiError(StatusCode::PAYLOAD_TOO_LARGE, "Remote archive exceeds the 512 MiB download limit".into());
    if response.content_length().is_some_and(|length| length > MAX_ARCHIVE_BYTES) { return Err(too_large()); }
    let expected = response.content_length().unwrap_or(0);
    let cache_dir = state.cache_dir.clone();
    tokio::task::spawn_blocking(move || trim_remote_cache(&cache_dir, expected)).await.map_err(internal)??;
    let temporary = cache_path.with_extension(format!("{}.tmp", Uuid::new_v4().simple()));
    let result = async {
        let mut file = tokio::fs::File::create(&temporary).await.map_err(internal)?;
        let mut written = 0u64;
        while let Some(chunk) = response.chunk().await.map_err(connection)? {
            written += chunk.len() as u64;
            if written > MAX_ARCHIVE_BYTES { return Err(too_large()); }
            tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await.map_err(internal)?;
        }
        tokio::io::AsyncWriteExt::flush(&mut file).await.map_err(internal)?;
        tokio::fs::rename(&temporary, &cache_path).await.map_err(internal)
    }.await;
    if result.is_err() { let _ = tokio::fs::remove_file(&temporary).await; }
    result
}

fn trim_remote_cache(cache_dir: &Path, incoming: u64) -> Result<(), ApiError> {
    const CACHE_LIMIT_BYTES: u64 = 1024 * 1024 * 1024;
    if incoming > CACHE_LIMIT_BYTES { return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "Remote archive exceeds the 1 GiB cache limit".into())); }
    let mut files = fs::read_dir(cache_dir).map_err(internal)?.filter_map(Result::ok).filter_map(|entry| {
        let metadata = entry.metadata().ok()?;
        if !metadata.is_file() { return None; }
        Some((entry.path(), metadata.len(), metadata.modified().ok()))
    }).collect::<Vec<_>>();
    let mut total = files.iter().map(|(_, size, _)| *size).sum::<u64>();
    files.sort_by_key(|(_, _, modified)| *modified);
    for (path, size, _) in files {
        if total.saturating_add(incoming) <= CACHE_LIMIT_BYTES { break; }
        fs::remove_file(path).map_err(internal)?;
        total = total.saturating_sub(size);
    }
    Ok(())
}

struct RemoteLocation {
    protocol: String,
    source: Source,
    path: String,
}

fn parse_remote_locator(state: &ForkApiState, locator: &str) -> Result<RemoteLocation, ApiError> {
    let mut parts = locator.splitn(3, ':');
    let protocol = parts.next().ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "Invalid remote locator".into()))?;
    let source_id = parts.next().ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "Invalid remote source locator".into()))?;
    let encoded_path = parts.next().ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "Invalid remote path locator".into()))?;
    let path = String::from_utf8(URL_SAFE_NO_PAD.decode(encoded_path).map_err(connection)?).map_err(connection)?;
    let source = state.inner.lock().map_err(internal)?.sources.iter().find(|source| source.id == source_id).cloned()
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Remote source is unavailable".into()))?;
    let matches_source = matches!((protocol, source.kind), ("webdav", SourceKind::Webdav) | ("smb", SourceKind::Smb));
    if !matches_source { return Err(ApiError(StatusCode::BAD_REQUEST, "Remote locator does not match its source".into())); }
    Ok(RemoteLocation { protocol: protocol.to_string(), source, path })
}

fn webdav_url(source: &Source, path: &str) -> Result<Url, ApiError> {
    let address = source.address.as_deref().ok_or_else(|| ApiError(StatusCode::CONFLICT, "WebDAV address is missing".into()))?;
    // Mirrors reqwest_dav's URL construction; scanned paths are already
    // percent-encoded because they come from WebDAV hrefs.
    Url::parse(&format!("{}/{}", address.trim_end_matches('/'), path.trim_start_matches('/'))).map_err(connection)
}

fn webdav_request(state: &ForkApiState, url: Url, credentials: &SourceCredentials) -> reqwest::RequestBuilder {
    let builder = state.http.get(url);
    if credentials.username.is_empty() { builder } else { builder.basic_auth(&credentials.username, credentials.password.as_deref()) }
}

enum RemoteFile {
    Smb(smb2::FileReader),
    Webdav { http: Client, url: Url, credentials: SourceCredentials, size: u64, tail: (u64, Arc<Vec<u8>>) },
}

impl RemoteFile {
    fn size(&self) -> u64 {
        match self {
            Self::Smb(reader) => reader.size(),
            Self::Webdav { size, .. } => *size,
        }
    }

    async fn read_at(&self, offset: u64, length: u64) -> Result<Vec<u8>, String> {
        let end = offset.saturating_add(length).min(self.size());
        if offset >= end { return Ok(Vec::new()); }
        match self {
            Self::Smb(reader) => reader.read_at(offset, end - offset).await.map_err(|error| error.to_string()),
            Self::Webdav { http, url, credentials, .. } => {
                let mut request = http.get(url.clone()).timeout(Duration::from_secs(60))
                    .header(header::RANGE, format!("bytes={offset}-{}", end - 1));
                if !credentials.username.is_empty() { request = request.basic_auth(&credentials.username, credentials.password.as_deref()); }
                let response = request.send().await.map_err(|error| error.to_string())?;
                if response.status() != StatusCode::PARTIAL_CONTENT {
                    return Err(format!("WebDAV range request returned {}", response.status()));
                }
                let bytes = response.bytes().await.map_err(|error| error.to_string())?;
                if bytes.len() as u64 != end - offset { return Err("WebDAV range response was truncated".into()); }
                Ok(bytes.to_vec())
            }
        }
    }
}

enum RemoteProbe {
    Ranged(RemoteFile),
    /// A WebDAV server that answered a range request with the whole file.
    Full(reqwest::Response),
}

/// Opens a remote file for positioned reads. SMB always supports them; for
/// WebDAV the first request asks for the archive tail, which also carries the
/// zip central directory for typical volumes.
async fn open_remote_file(state: &ForkApiState, locator: &str) -> Result<RemoteProbe, ApiError> {
    let location = parse_remote_locator(state, locator)?;
    let credentials = source_credentials(&location.source)?;
    if location.protocol == "smb" {
        return smb_file_reader(state, &location.source, &credentials, &location.path).await.map(|reader| RemoteProbe::Ranged(RemoteFile::Smb(reader)));
    }
    let url = webdav_url(&location.source, &location.path)?;
    let response = webdav_request(state, url.clone(), &credentials)
        .header(header::RANGE, format!("bytes=-{REMOTE_TAIL_BYTES}"))
        .send().await.map_err(connection)?;
    if response.status() == StatusCode::OK { return Ok(RemoteProbe::Full(response)); }
    if response.status() != StatusCode::PARTIAL_CONTENT {
        return Err(ApiError(StatusCode::BAD_GATEWAY, format!("WebDAV server returned {}", response.status())));
    }
    // Content-Range: bytes <first>-<last>/<total>
    let range = response.headers().get(header::CONTENT_RANGE).and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("bytes "))
        .and_then(|value| {
            let (span, total) = value.split_once('/')?;
            let (first, _) = span.split_once('-')?;
            Some((first.trim().parse::<u64>().ok()?, total.trim().parse::<u64>().ok()?))
        })
        .ok_or_else(|| ApiError(StatusCode::BAD_GATEWAY, "WebDAV range response had no usable Content-Range".into()))?;
    let (first, size) = range;
    let tail = response.bytes().await.map_err(connection)?;
    if first.saturating_add(tail.len() as u64) != size {
        return Err(ApiError(StatusCode::BAD_GATEWAY, "WebDAV range response was truncated".into()));
    }
    Ok(RemoteProbe::Ranged(RemoteFile::Webdav { http: state.http.clone(), url, credentials, size, tail: (first, Arc::new(tail.to_vec())) }))
}

/// Recently fetched byte ranges of one remote archive, shared by every request
/// reading it. The archive tail (central directory) stays pinned.
#[derive(Default)]
struct BlockCache {
    pinned: Option<(u64, Arc<Vec<u8>>)>,
    recent: std::collections::VecDeque<(u64, Arc<Vec<u8>>)>,
}

impl BlockCache {
    fn find(&self, position: u64) -> Option<(u64, Arc<Vec<u8>>)> {
        self.pinned.iter().chain(self.recent.iter().rev())
            .find(|(start, data)| *start <= position && position < start + data.len() as u64)
            .cloned()
    }

    fn insert(&mut self, block: (u64, Arc<Vec<u8>>)) {
        if self.recent.len() >= 8 { self.recent.pop_front(); }
        self.recent.push_back(block);
    }
}

/// A synchronous `Read + Seek` view over a remote file for the zip reader.
/// It must only be used from blocking threads.
#[derive(Clone)]
struct RangeReader {
    file: Arc<RemoteFile>,
    runtime: tokio::runtime::Handle,
    position: u64,
    blocks: Arc<Mutex<BlockCache>>,
}

impl RangeReader {
    fn new(file: Arc<RemoteFile>, runtime: tokio::runtime::Handle, tail: Option<(u64, Arc<Vec<u8>>)>) -> Self {
        Self { file, runtime, position: 0, blocks: Arc::new(Mutex::new(BlockCache { pinned: tail, recent: Default::default() })) }
    }

}

impl Read for RangeReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let size = self.file.size();
        if buffer.is_empty() || self.position >= size { return Ok(0); }
        let position = self.position;
        let cached = self.blocks.lock().map_err(|_| std::io::Error::other("remote block cache poisoned"))?.find(position);
        let (start, data) = match cached {
            Some(block) => block,
            None => {
                let length = REMOTE_BLOCK_BYTES.max(buffer.len() as u64);
                let file = Arc::clone(&self.file);
                let data = self.runtime.block_on(async move { file.read_at(position, length).await })
                    .map_err(std::io::Error::other)?;
                if data.is_empty() { return Ok(0); }
                let block = (position, Arc::new(data));
                self.blocks.lock().map_err(|_| std::io::Error::other("remote block cache poisoned"))?.insert(block.clone());
                block
            }
        };
        let offset = (position - start) as usize;
        let count = buffer.len().min(data.len() - offset);
        buffer[..count].copy_from_slice(&data[offset..offset + count]);
        self.position += count as u64;
        Ok(count)
    }
}

impl std::io::Seek for RangeReader {
    fn seek(&mut self, target: std::io::SeekFrom) -> std::io::Result<u64> {
        let size = i128::from(self.file.size());
        let next = match target {
            std::io::SeekFrom::Start(offset) => i128::from(offset),
            std::io::SeekFrom::End(delta) => size + i128::from(delta),
            std::io::SeekFrom::Current(delta) => i128::from(self.position) + i128::from(delta),
        };
        if next < 0 { return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "seek before start of remote file")); }
        self.position = u64::try_from(next).map_err(std::io::Error::other)?;
        Ok(self.position)
    }
}

async fn smb_connect(source: &Source, credentials: &SourceCredentials) -> Result<SmbSession, ApiError> {
    let address = source.address.as_deref().ok_or_else(|| ApiError(StatusCode::CONFLICT, "SMB address is missing".into()))?;
    let (server, share_name, _) = parse_smb_address(address)?;
    let mut client = smb2::connect(&server, &credentials.username, credentials.password.as_deref().unwrap_or_default()).await.map_err(connection)?;
    let share = client.connect_share(&share_name).await.map_err(connection)?;
    Ok(SmbSession { client, share })
}

/// Opens a file over the source's pooled SMB session, reconnecting once if the
/// cached session has gone stale. Readers share the session's connection, so
/// page requests no longer pay a TCP connect, negotiate and login each time.
async fn smb_file_reader(state: &ForkApiState, source: &Source, credentials: &SourceCredentials, path: &str) -> Result<smb2::FileReader, ApiError> {
    let slot = {
        let mut sessions = state.smb_sessions.lock().await;
        Arc::clone(sessions.entry(source.id.clone()).or_default())
    };
    let mut session = slot.lock().await;
    let path = path.trim_start_matches('/');
    let mut last_error = None;
    for _ in 0..2 {
        if session.is_none() { *session = Some(smb_connect(source, credentials).await?); }
        let Some(active) = session.as_ref() else { continue; };
        match active.client.open_file_reader(&active.share, path).await {
            Ok(reader) => return Ok(reader),
            Err(error) => { *session = None; last_error = Some(error); }
        }
    }
    Err(last_error.map_or_else(|| ApiError(StatusCode::BAD_GATEWAY, "SMB connection failed".into()), connection))
}

async fn read_remote_bytes(state: &ForkApiState, locator: &str, limit: u64) -> Result<Vec<u8>, ApiError> {
    let location = parse_remote_locator(state, locator)?;
    let credentials = source_credentials(&location.source)?;
    let too_large = || ApiError(StatusCode::PAYLOAD_TOO_LARGE, "Remote file exceeds the safety limit".into());
    if location.protocol == "smb" {
        let reader = smb_file_reader(state, &location.source, &credentials, &location.path).await?;
        if reader.size() > limit {
            let _ = reader.close().await;
            return Err(too_large());
        }
        let result = reader.read_at(0, reader.size()).await.map_err(connection);
        let _ = reader.close().await;
        return result;
    }
    let url = webdav_url(&location.source, &location.path)?;
    let mut response = webdav_request(state, url, &credentials).send().await.map_err(connection)?
        .error_for_status().map_err(connection)?;
    if response.content_length().is_some_and(|length| length > limit) { return Err(too_large()); }
    let mut bytes = Vec::with_capacity(response.content_length().unwrap_or(0).min(limit) as usize);
    while let Some(chunk) = response.chunk().await.map_err(connection)? {
        if (bytes.len() + chunk.len()) as u64 > limit { return Err(too_large()); }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn set_progress(State(state): State<ForkApiState>, AxumPath(id): AxumPath<String>, Json(request): Json<ProgressRequest>) -> Result<StatusCode, ApiError> {
    let writeback_source = {
        let mut guard = state.inner.lock().map_err(internal)?;
        let series = guard.series.iter_mut().find(|series| series.volumes.iter().any(|volume| volume.id == id))
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Volume not found".into()))?;
        let source_id = series.source_id.clone();
        let volume = series.volumes.iter_mut().find(|volume| volume.id == id)
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Volume not found".into()))?;
        if request.page >= volume.page_count { return Err(ApiError(StatusCode::BAD_REQUEST, "Page is outside this volume".into())); }
        let unchanged = volume.progress_page == request.page;
        volume.progress_page = request.page;
        let page_count = volume.page_count;
        let remote_book_id = volume.id.split_once(":komga-book:").map(|(_, remote)| remote.to_string());
        if unchanged {
            None
        } else if let Some(remote_book_id) = remote_book_id {
            guard.pending_progress.retain(|pending| pending.volume_id != id);
            guard.pending_progress.push(PendingProgress {
                source_id: source_id.clone(), volume_id: id.clone(), remote_book_id,
                page: request.page, page_count, updated_at_ms: chrono::Utc::now().timestamp_millis(),
            });
            Some(source_id)
        } else {
            None
        }
    };
    state.persist_soon();
    // Server writeback must not delay the page turn that triggered it.
    if let Some(source_id) = writeback_source {
        let state = state.clone();
        tokio::spawn(async move { flush_pending_progress(&state, &source_id).await; });
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn flush_pending_progress(state: &ForkApiState, source_id: &str) {
    // Serialise writebacks so an older page cannot reach the server after a
    // newer one when several page turns happen in quick succession.
    let _writeback = state.progress_writeback.lock().await;
    let (source, pending) = {
        let Ok(guard) = state.inner.lock() else { return; };
        let Some(source) = guard.sources.iter().find(|source| source.id == source_id && source.kind == SourceKind::Komga).cloned() else { return; };
        (source, guard.pending_progress.iter().filter(|item| item.source_id == source_id).cloned().collect::<Vec<_>>())
    };
    if pending.is_empty() { return; }
    let Ok(credentials) = source_credentials(&source) else { return; };
    let Some(base) = source.address.as_deref() else { return; };
    let mut changed = false;
    for item in pending {
        let url = format!("{}/api/v1/books/{}/read-progress", base.trim_end_matches('/'), urlencoding::encode(&item.remote_book_id));
        // Komga numbers pages from 1; the reader stores zero-based indexes.
        let completed = item.page_count > 0 && item.page + 1 >= item.page_count;
        let body = serde_json::json!({ "page": item.page + 1, "completed": completed });
        let response = authenticate_credentials(state.http.patch(url).timeout(Duration::from_secs(20)).json(&body), &credentials).send().await;
        let Ok(response) = response else { break; };
        let status = response.status();
        // A read-only account or a book removed on the server cannot accept
        // progress; keep reading locally instead of retrying forever.
        if status.is_success() || matches!(status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND | StatusCode::BAD_REQUEST) {
            if let Ok(mut guard) = state.inner.lock() {
                guard.pending_progress.retain(|pending| !(pending.volume_id == item.volume_id && pending.updated_at_ms <= item.updated_at_ms));
                changed = true;
            }
        }
    }
    if changed { state.persist_soon(); }
}

async fn test_source(Json(request): Json<SourceRequest>) -> Result<Json<ConnectionResult>, ApiError> {
    validate_source(&request)?;
    Ok(Json(test_connection(&request).await?))
}

async fn test_connection(request: &SourceRequest) -> Result<ConnectionResult, ApiError> {
    match request.kind {
        SourceKind::Local => Ok(ConnectionResult { ok: true, detail: "Android folder permission is required".into(), libraries: Vec::new() }),
        SourceKind::Webdav => test_webdav(request).await,
        SourceKind::Komga => test_komga(request).await,
        SourceKind::Smb => test_smb(request).await,
    }
}

fn validate_source(request: &SourceRequest) -> Result<(), ApiError> {
    if request.name.trim().is_empty() { return Err(ApiError(StatusCode::BAD_REQUEST, "Source name is required".into())); }
    if request.kind == SourceKind::Local && !request.address.starts_with("content://") { return Err(ApiError(StatusCode::BAD_REQUEST, "Select an Android document folder".into())); }
    if request.kind != SourceKind::Local && request.address.trim().is_empty() { return Err(ApiError(StatusCode::BAD_REQUEST, "Server address is required".into())); }
    if matches!(request.kind, SourceKind::Webdav | SourceKind::Komga) {
        let url = Url::parse(&request.address).map_err(|_| ApiError(StatusCode::BAD_REQUEST, "Enter a valid base URL".into()))?;
        if !url.username().is_empty() || url.password().is_some() {
            return Err(ApiError(StatusCode::BAD_REQUEST, "Keep credentials out of the URL and use the credential fields".into()));
        }
        if url.scheme() != "https" && !request.allow_http { return Err(ApiError(StatusCode::BAD_REQUEST, "HTTP requires explicit per-source approval".into())); }
    }
    Ok(())
}

fn client(_request: &SourceRequest) -> Result<Client, ApiError> {
    Client::builder().redirect(reqwest::redirect::Policy::limited(5)).build().map_err(internal)
}

fn authenticate(builder: reqwest::RequestBuilder, request: &SourceRequest) -> reqwest::RequestBuilder {
    if let Some(key) = request.api_key.as_deref().filter(|key| !key.is_empty()) { return builder.header("X-API-Key", key); }
    if !request.username.is_empty() { return builder.basic_auth(&request.username, request.password.as_deref()); }
    builder
}

async fn test_webdav(request: &SourceRequest) -> Result<ConnectionResult, ApiError> {
    let auth = if request.username.is_empty() {
        reqwest_dav::Auth::Anonymous
    } else {
        reqwest_dav::Auth::Basic(request.username.clone(), request.password.clone().unwrap_or_default())
    };
    let client = reqwest_dav::ClientBuilder::new()
        .set_host(request.address.trim_end_matches('/').to_string())
        .set_auth(auth)
        .build()
        .map_err(connection)?;
    client.list("/", reqwest_dav::Depth::Number(0)).await.map_err(connection)?;
    Ok(ConnectionResult { ok: true, detail: "WebDAV connection and folder listing successful".into(), libraries: Vec::new() })
}

async fn test_komga(request: &SourceRequest) -> Result<ConnectionResult, ApiError> {
    let base = request.address.trim_end_matches('/');
    let response = authenticate(client(request)?.get(format!("{base}/api/v1/libraries")), request).send().await.map_err(connection)?;
    if !response.status().is_success() { return Err(ApiError(StatusCode::BAD_GATEWAY, format!("Komga returned {}", response.status()))); }
    let libraries = response.json::<Vec<RemoteChoice>>().await.map_err(connection)?;
    Ok(ConnectionResult { ok: true, detail: format!("Komga connection successful: {} libraries", libraries.len()), libraries })
}

async fn scan_webdav(state: &ForkApiState, source: &Source) -> Result<ScanPayload, ApiError> {
    let address = source.address.as_deref().ok_or_else(|| ApiError(StatusCode::CONFLICT, "WebDAV address is missing".into()))?;
    let credentials = source_credentials(source)?;
    let auth = if credentials.username.is_empty() {
        reqwest_dav::Auth::Anonymous
    } else {
        reqwest_dav::Auth::Basic(credentials.username, credentials.password.unwrap_or_default())
    };
    let client = reqwest_dav::ClientBuilder::new()
        .set_host(address.trim_end_matches('/').to_string())
        .set_auth(auth)
        .build()
        .map_err(connection)?;
    let mut queue = vec!["/".to_string()];
    let mut visited = std::collections::HashSet::new();
    let mut entries = Vec::new();
    while let Some(directory) = queue.pop() {
        if !visited.insert(directory.clone()) { continue; }
        if visited.len() > 100_000 { return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "WebDAV folder tree exceeds safety limit".into())); }
        let listed = client.list(&directory, reqwest_dav::Depth::Number(1)).await.map_err(connection)?;
        for entity in listed {
            match entity {
                reqwest_dav::list_cmd::ListEntity::File(file) => {
                    let path = dav_relative_path(address, &file.href)?;
                    if path == directory || (!is_image_name(&path) && !is_archive_name(&path)) { continue; }
                    entries.push(RemoteEntry {
                        path,
                        size: u64::try_from(file.content_length.max(0)).map_err(internal)?,
                        modified: file.tag.unwrap_or_else(|| file.last_modified.to_rfc3339()),
                    });
                }
                reqwest_dav::list_cmd::ListEntity::Folder(folder) => {
                    let path = dav_relative_path(address, &folder.href)?;
                    if path != directory && path != "/" { queue.push(path); }
                }
            }
        }
    }
    scan_remote_entries(state, source, "webdav", entries).await
}

async fn scan_smb(state: &ForkApiState, source: &Source) -> Result<ScanPayload, ApiError> {
    let address = source.address.as_deref().ok_or_else(|| ApiError(StatusCode::CONFLICT, "SMB address is missing".into()))?;
    let (server, share_name, root) = parse_smb_address(address)?;
    let credentials = source_credentials(source)?;
    let mut client = smb2::connect(&server, &credentials.username, credentials.password.as_deref().unwrap_or_default()).await.map_err(connection)?;
    let mut share = client.connect_share(&share_name).await.map_err(connection)?;
    let mut queue = vec![root.clone()];
    let mut visited = std::collections::HashSet::new();
    let mut entries = Vec::new();
    while let Some(directory) = queue.pop() {
        if !visited.insert(directory.clone()) { continue; }
        if visited.len() > 100_000 { return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "SMB folder tree exceeds safety limit".into())); }
        for entry in client.list_directory(&mut share, &directory).await.map_err(connection)? {
            if entry.name == "." || entry.name == ".." { continue; }
            let path = join_remote_path(&directory, &entry.name);
            if entry.is_directory {
                queue.push(path);
            } else if is_image_name(&path) || is_archive_name(&path) {
                entries.push(RemoteEntry { path, size: entry.size, modified: entry.modified.0.to_string() });
            }
        }
    }
    client.disconnect_share(&share).await.map_err(connection)?;
    scan_remote_entries(state, source, "smb", entries).await
}

async fn scan_remote_entries(
    state: &ForkApiState,
    source: &Source,
    protocol: &str,
    mut entries: Vec<RemoteEntry>,
) -> Result<ScanPayload, ApiError> {
    entries.sort_by(|left, right| manga_reader_core::natural_cmp(&left.path, &right.path));
    let mut grouped: std::collections::HashMap<(String, String), Vec<RemoteEntry>> = std::collections::HashMap::new();
    let mut archives = Vec::new();
    for entry in entries {
        if is_archive_name(&entry.path) { archives.push(entry); continue; }
        let parent = remote_parent(&entry.path);
        let components: Vec<_> = parent.split('/').filter(|part| !part.is_empty()).collect();
        let (series_path, volume_path) = match components.len() {
            0 => (String::new(), String::new()),
            1 => (parent.clone(), parent),
            _ => (components[..components.len() - 1].join("/"), parent),
        };
        grouped.entry((series_path, volume_path)).or_default().push(entry);
    }
    let mut series_map: std::collections::HashMap<String, ScannedSeries> = std::collections::HashMap::new();
    for ((series_path, volume_path), mut pages) in grouped {
        pages.sort_by(|left, right| manga_reader_core::natural_cmp(&left.path, &right.path));
        let series_title = remote_title(&series_path).unwrap_or_else(|| source.name.clone());
        let volume_title = remote_title(&volume_path).unwrap_or_else(|| "Volume 1".into());
        let series_id = stable_remote_id(&source.id, "series", &series_path);
        let volume_id = stable_remote_id(&source.id, "volume", &volume_path);
        let mut fingerprint = std::collections::hash_map::DefaultHasher::new();
        let locators = pages.into_iter().map(|page| {
            page.path.hash(&mut fingerprint);
            page.size.hash(&mut fingerprint);
            page.modified.hash(&mut fingerprint);
            remote_locator(protocol, &source.id, &page.path)
        }).collect::<Vec<_>>();
        series_map.entry(series_id.clone()).or_insert_with(|| ScannedSeries {
            id: series_id,
            title: series_title,
            volumes: Vec::new(),
        }).volumes.push(ScannedVolume {
            id: volume_id,
            title: volume_title,
            page_count: locators.len().try_into().map_err(internal)?,
            fingerprint: format!("{:016x}", fingerprint.finish()),
            pages: locators,
            archive_uri: None,
            error: None,
            progress_page: 0,
        });
    }
    // Unchanged archives keep their page list from the previous scan, so a
    // rescan only touches new or modified files.
    let known: HashMap<String, (String, Vec<String>)> = {
        let guard = state.inner.lock().map_err(internal)?;
        guard.series.iter().filter(|series| series.source_id == source.id)
            .flat_map(|series| &series.volumes)
            .filter(|volume| volume.error.is_none() && !volume.pages.is_empty())
            .map(|volume| (volume.id.clone(), (volume.fingerprint.clone(), volume.pages.clone())))
            .collect()
    };
    let listed = futures::stream::iter(archives.into_iter().map(|archive| {
        let volume_id = stable_remote_id(&source.id, "volume", &archive.path);
        let fingerprint = format!("{}:{}:{}", archive.size, archive.modified, archive.path);
        let reusable = known.get(&volume_id).filter(|(previous, _)| previous == &fingerprint).map(|(_, pages)| pages.clone());
        async move {
            let locator = remote_locator(protocol, &source.id, &archive.path);
            let result = match reusable {
                Some(pages) => Ok(pages),
                None => remote_archive_pages(state, &locator, true).await.map(|indexes| {
                    let encoded = URL_SAFE_NO_PAD.encode(locator.as_bytes());
                    indexes.into_iter().map(|index| format!("rzip:{encoded}:{index}")).collect::<Vec<_>>()
                }),
            };
            (archive, volume_id, fingerprint, result)
        }
    })).buffer_unordered(4).collect::<Vec<_>>().await;
    for (archive, volume_id, fingerprint, result) in listed {
        let parent = remote_parent(&archive.path);
        let stem = remote_file_stem(&archive.path);
        let series_key = if parent.is_empty() { archive.path.clone() } else { parent.clone() };
        let series_title = if parent.is_empty() { stem.clone() } else { remote_title(&parent).unwrap_or_else(|| source.name.clone()) };
        let series_id = stable_remote_id(&source.id, "series", &series_key);
        let (pages, error) = match result {
            Ok(pages) => (pages, None),
            Err(error) if error.0 == StatusCode::UNPROCESSABLE_ENTITY => (Vec::new(), Some("Remote archive is not a readable ZIP/CBZ file".into())),
            Err(error) => (Vec::new(), Some(format!("Could not read remote archive: {}", error.1))),
        };
        series_map.entry(series_id.clone()).or_insert_with(|| ScannedSeries { id: series_id, title: series_title, volumes: Vec::new() })
            .volumes.push(ScannedVolume {
                id: volume_id,
                title: stem,
                page_count: pages.len().try_into().map_err(internal)?,
                fingerprint,
                pages,
                archive_uri: None,
                error,
                progress_page: 0,
            });
    }
    let mut series: Vec<_> = series_map.into_values().collect();
    for item in &mut series { item.volumes.sort_by(|left, right| manga_reader_core::natural_cmp(&left.title, &right.title)); }
    series.sort_by(|left, right| manga_reader_core::natural_cmp(&left.title, &right.title));
    Ok(ScanPayload { series })
}

fn dav_relative_path(address: &str, href: &str) -> Result<String, ApiError> {
    let base = Url::parse(address).map_err(connection)?;
    let absolute = base.join(href).map_err(connection)?;
    let base_path = base.path().trim_end_matches('/');
    let path = absolute.path();
    let relative = path.strip_prefix(base_path).unwrap_or(path);
    Ok(format!("/{}", relative.trim_matches('/')))
}

fn parse_smb_address(address: &str) -> Result<(String, String, String), ApiError> {
    let url = Url::parse(address).map_err(|_| ApiError(StatusCode::BAD_REQUEST, "Use smb://server/share/optional-folder".into()))?;
    if url.scheme() != "smb" || !url.username().is_empty() || url.password().is_some() {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Use smb://server/share/path and enter credentials in their own fields".into()));
    }
    let host = url.host_str().ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "SMB server is required".into()))?;
    let server = format!("{}:{}", host, url.port().unwrap_or(445));
    let mut segments = url.path_segments().ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "SMB share is required".into()))?.filter(|part| !part.is_empty());
    let share = segments.next().ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "SMB share is required".into()))?.to_string();
    Ok((server, share, segments.collect::<Vec<_>>().join("/")))
}

fn join_remote_path(parent: &str, child: &str) -> String {
    if parent.is_empty() || parent == "/" { child.trim_matches('/').to_string() }
    else { format!("{}/{}", parent.trim_matches('/'), child.trim_matches('/')) }
}

fn remote_parent(path: &str) -> String {
    path.trim_matches('/').rsplit_once('/').map_or_else(String::new, |(parent, _)| parent.to_string())
}

fn remote_title(path: &str) -> Option<String> {
    let value = path.trim_matches('/').rsplit('/').next().filter(|value| !value.is_empty())?;
    Some(urlencoding::decode(value).map_or_else(|_| value.to_string(), |value| value.into_owned()))
}

fn remote_file_stem(path: &str) -> String {
    let title = remote_title(path).unwrap_or_else(|| "Volume".into());
    title.rsplit_once('.').map_or(title.clone(), |(stem, _)| stem.to_string())
}

fn stable_remote_id(source_id: &str, kind: &str, path: &str) -> String {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    source_id.hash(&mut hash);
    kind.hash(&mut hash);
    path.hash(&mut hash);
    format!("{source_id}:remote-{kind}:{:016x}", hash.finish())
}

fn remote_locator(protocol: &str, source_id: &str, path: &str) -> String {
    format!("{protocol}:{source_id}:{}", URL_SAFE_NO_PAD.encode(path.as_bytes()))
}

fn is_archive_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".zip") || lower.ends_with(".cbz")
}

async fn scan_komga(state: &ForkApiState, source: &Source) -> Result<ScanPayload, ApiError> {
    let base = source.address.as_deref().ok_or_else(|| ApiError(StatusCode::CONFLICT, "Komga address is missing".into()))?.trim_end_matches('/');
    let credentials = source_credentials(source)?;
    let client = &state.http;
    let mut all_books = Vec::new();
    for library_id in &source.library_ids {
        let mut page = 0u32;
        loop {
            let mut url = Url::parse(&format!("{base}/api/v1/books")).map_err(connection)?;
            url.query_pairs_mut()
                .append_pair("library_id", library_id)
                .append_pair("page", &page.to_string())
                .append_pair("size", "500")
                .append_pair("sort", "number,asc");
            let response = authenticate_credentials(client.get(url), &credentials).send().await.map_err(connection)?;
            if !response.status().is_success() {
                return Err(ApiError(StatusCode::BAD_GATEWAY, format!("Komga catalogue returned {}", response.status())));
            }
            let page_result: KomgaPage<KomgaBook> = response.json().await.map_err(connection)?;
            let count = page_result.content.len();
            all_books.extend(page_result.content);
            if count < 500 { break; }
            page += 1;
            if page > 10_000 { return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "Komga catalogue pagination exceeded safety limit".into())); }
        }
    }
    let mut grouped: std::collections::HashMap<String, ScannedSeries> = std::collections::HashMap::new();
    for book in all_books {
        let series_key = format!("{}:komga-series:{}", source.id, book.series_id);
        let volume_id = format!("{}:komga-book:{}", source.id, book.id);
        let readable = book.media.status.eq_ignore_ascii_case("READY") && book.media.pages_count > 0;
        let pages = if readable {
            (0..book.media.pages_count).map(|page| format!("komga:{}:{}:{page}", source.id, book.id)).collect()
        } else { Vec::new() };
        let last_page = book.media.pages_count.saturating_sub(1);
        let progress = book.read_progress.as_ref().map_or(0, |progress| {
            if progress.completed { last_page } else { progress.page.saturating_sub(1).min(last_page) }
        });
        let entry = grouped.entry(series_key.clone()).or_insert_with(|| ScannedSeries {
            id: series_key,
            title: book.series_title.clone(),
            volumes: Vec::new(),
        });
        entry.volumes.push(ScannedVolume {
            id: volume_id,
            title: book.name,
            page_count: book.media.pages_count,
            fingerprint: if book.file_hash.is_empty() { book.last_modified } else { book.file_hash },
            pages,
            archive_uri: None,
            error: (!readable).then(|| format!("Komga media is {}", book.media.status)),
            progress_page: progress,
        });
    }
    let mut series: Vec<_> = grouped.into_values().collect();
    for item in &mut series { item.volumes.sort_by(|left, right| manga_reader_core::natural_cmp(&left.title, &right.title)); }
    series.sort_by(|left, right| manga_reader_core::natural_cmp(&left.title, &right.title));
    Ok(ScanPayload { series })
}

fn authenticate_credentials(builder: reqwest::RequestBuilder, credentials: &SourceCredentials) -> reqwest::RequestBuilder {
    if let Some(key) = credentials.api_key.as_deref().filter(|key| !key.is_empty()) { return builder.header("X-API-Key", key); }
    if !credentials.username.is_empty() { return builder.basic_auth(&credentials.username, credentials.password.as_deref()); }
    builder
}

fn source_credentials(source: &Source) -> Result<SourceCredentials, ApiError> {
    let Some(reference) = source.credential_ref.as_deref() else { return Ok(SourceCredentials::default()); };
    let value = secure_store_get(reference)?.ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED, "Source credentials need to be entered again".into()))?;
    serde_json::from_str(&value).map_err(internal)
}

async fn test_smb(request: &SourceRequest) -> Result<ConnectionResult, ApiError> {
    let url = Url::parse(&request.address).map_err(|_| ApiError(StatusCode::BAD_REQUEST, "Use smb://server/share/optional-folder".into()))?;
    if url.scheme() != "smb" || !url.username().is_empty() || url.password().is_some() {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Use smb://server/share/path and enter credentials in their own fields".into()));
    }
    let host = url.host_str().ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "SMB server is required".into()))?;
    let address = format!("{}:{}", host, url.port().unwrap_or(445));
    let mut segments = url.path_segments().ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "SMB share is required".into()))?.filter(|part| !part.is_empty());
    let share_name = segments.next().ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "SMB share is required".into()))?;
    let root = segments.collect::<Vec<_>>().join("/");
    let mut smb = smb2::connect(&address, &request.username, request.password.as_deref().unwrap_or_default()).await.map_err(connection)?;
    let mut share = smb.connect_share(share_name).await.map_err(connection)?;
    smb.list_directory(&mut share, &root).await.map_err(connection)?;
    smb.disconnect_share(&share).await.map_err(connection)?;
    Ok(ConnectionResult { ok: true, detail: "SMB 2/3 connection and folder listing successful".into(), libraries: Vec::new() })
}

async fn control_job(State(state): State<ForkApiState>, AxumPath((id, action)): AxumPath<(String, String)>) -> Result<Json<Job>, ApiError> {
    let mut guard = state.inner.lock().map_err(internal)?;
    let index = guard.jobs.iter().position(|job| job.id == id)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Job not found".into()))?;
    let should_start;
    {
        let job = &mut guard.jobs[index];
        match action.as_str() {
            "pause" if matches!(job.state.as_str(), "queued" | "processing") => {
                job.state = "paused".into(); should_start = false;
            }
            "resume" if job.state == "paused" => {
                job.state = "queued".into(); should_start = true;
            }
            "cancel" if !matches!(job.state.as_str(), "complete" | "cancelled") => {
                job.state = "cancelled".into(); should_start = false;
            }
            "retry" if matches!(job.state.as_str(), "failed" | "partial") => {
                for page in &mut job.pages {
                    if page.state == "failed" { page.state = "queued".into(); page.error = None; }
                }
                job.error = None;
                job.state = "queued".into(); should_start = true;
            }
            _ => return Err(ApiError(StatusCode::CONFLICT, "Action is not valid for the current job state".into())),
        }
    }
    if guard.jobs[index].kind == "ocr" { update_ocr_rollups(&mut guard, &id); }
    else if guard.jobs[index].kind == "download" { update_download_rollups(&mut guard, &id); }
    let response = guard.jobs[index].clone();
    state.persist(&guard)?;
    drop(guard);
    if should_start { spawn_job_worker(state.clone(), id); }
    Ok(Json(response))
}

async fn record_event(State(state): State<ForkApiState>, Json(request): Json<StatEventRequest>) -> Result<StatusCode, ApiError> {
    if request.value == 0 || request.value > 3600 { return Err(ApiError(StatusCode::BAD_REQUEST, "Invalid event value".into())); }
    let now = chrono::Utc::now().timestamp_millis();
    if (request.occurred_at_ms - now).abs() > 86_400_000 { return Err(ApiError(StatusCode::BAD_REQUEST, "Event time is outside the accepted window".into())); }
    let device_date = Local::now().date_naive();
    if (request.local_date - device_date).num_days().abs() > 1 { return Err(ApiError(StatusCode::BAD_REQUEST, "Event date is outside the accepted window".into())); }
    let mut guard = state.inner.lock().map_err(internal)?;
    if matches!(request.kind, StatEventKind::CardMined | StatEventKind::VolumeComplete)
        && request.unique_key.as_ref().is_some_and(|key| guard.events.iter().any(|event| std::mem::discriminant(&event.kind) == std::mem::discriminant(&request.kind) && event.unique_key.as_ref() == Some(key))) {
        return Ok(StatusCode::NO_CONTENT);
    }
    guard.events.push(StatEvent { id: Uuid::new_v4().to_string(), kind: request.kind, local_date: request.local_date, occurred_at_ms: request.occurred_at_ms, value: request.value, unique_key: request.unique_key, series_id: request.series_id, sensitive: request.sensitive });
    state.persist_soon();
    Ok(StatusCode::NO_CONTENT)
}

async fn export_stats_csv(State(state): State<ForkApiState>) -> Result<Response, ApiError> {
    let guard = state.inner.lock().map_err(internal)?;
    let mut csv = String::from("local_date,event,value,series_id,sensitive\n");
    for event in guard.events.iter().filter(|event| guard.show_sensitive || !event.sensitive) {
        let kind = match event.kind {
            StatEventKind::ActiveReading => "active_reading_seconds",
            StatEventKind::PageEncounter => "page_turn",
            StatEventKind::VolumeComplete => "volume_complete",
            StatEventKind::DictionaryLookup => "dictionary_lookup",
            StatEventKind::CardMined => "card_mined",
            StatEventKind::OcrCharacters => "ocr_characters",
        };
        csv.push_str(&format!("{},{},{},{},{}\n", event.local_date, kind, event.value, event.series_id.as_deref().unwrap_or(""), event.sensitive));
    }
    Ok(Response::builder().status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/csv; charset=utf-8")
        .header(header::CONTENT_DISPOSITION, "attachment; filename=reader-statistics.csv")
        .body(Body::from(csv)).map_err(internal)?)
}

#[derive(Debug, Deserialize)]
struct StatsDetailQuery { range: Option<String> }

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatsHistoryDay { local_date: NaiveDate, pages: u64, active_seconds: u64 }

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SeriesStats { series_id: String, title: String, pages: u64, active_seconds: u64, lookups: u64, cards: u64 }

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatsDetail { history: Vec<StatsHistoryDay>, per_series: Vec<SeriesStats> }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadingSession {
    session_id: String,
    series_id: String,
    title: String,
    started_at_ms: i64,
    ended_at_ms: i64,
    pages: u64,
    active_seconds: u64,
    #[serde(skip)]
    event_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadingSessions { sessions: Vec<ReadingSession> }

/// The volume (id or legacy fingerprint) a page-level statistics event refers
/// to, recovered from its unique key.
fn event_volume_key(event: &StatEvent) -> Option<&str> {
    let key = event.unique_key.as_deref()?;
    key.strip_prefix("/fork/reader/")
        .and_then(|value| value.split_once('#').map(|(volume_id, _)| volume_id))
        .or_else(|| {
            key.rsplit_once(':')
                .filter(|(_, page)| page.parse::<u32>().is_ok())
                .map(|(volume_id, _)| volume_id)
        })
}

/// Identifiers (series ids, volume ids and volume fingerprints) of content
/// currently flagged NSFW.
fn hidden_content(guard: &ForkState) -> std::collections::HashSet<&str> {
    let mut hidden = std::collections::HashSet::new();
    for series in &guard.series {
        if series.sensitive { hidden.insert(series.id.as_str()); }
        for volume in series.volumes.iter().filter(|volume| series.sensitive || volume.sensitive) {
            hidden.insert(volume.id.as_str());
            hidden.insert(volume.fingerprint.as_str());
        }
    }
    hidden
}

fn reading_sessions(guard: &ForkState, include_date: &dyn Fn(NaiveDate) -> bool) -> Vec<ReadingSession> {
    const SESSION_GAP_MS: i64 = 15 * 60 * 1_000;
    // Index volumes once instead of scanning the catalogue for every event.
    let mut series_by_volume: HashMap<&str, &str> = HashMap::new();
    for series in &guard.series {
        for volume in &series.volumes {
            series_by_volume.entry(volume.fingerprint.as_str()).or_insert(series.id.as_str());
        }
    }
    for series in &guard.series {
        for volume in &series.volumes {
            series_by_volume.insert(volume.id.as_str(), series.id.as_str());
        }
    }
    let titles: HashMap<&str, &str> = guard.series.iter().map(|series| (series.id.as_str(), series.title.as_str())).collect();
    let series_for_event = |event: &StatEvent| -> Option<String> {
        if let Some(series_id) = &event.series_id {
            return Some(series_id.clone());
        }
        series_by_volume.get(event_volume_key(event)?).map(|series_id| (*series_id).to_string())
    };
    // Events keep the sensitivity they were recorded with, but a series or
    // volume marked NSFW later must disappear from itemised history too.
    let hidden = (!guard.show_sensitive).then(|| hidden_content(guard));
    let is_hidden = |event: &StatEvent, series_id: Option<&str>| hidden.as_ref().is_some_and(|hidden| {
        series_id.is_some_and(|series_id| hidden.contains(series_id))
            || event_volume_key(event).is_some_and(|volume| hidden.contains(volume))
    });

    // Older fork readers recorded the volume in uniqueKey but omitted seriesId.
    // Recover those page anchors and associate adjacent active-reading ticks so
    // existing history remains visible without mutating the user's event log.
    let mut annotated: Vec<_> = guard.events.iter()
        .filter(|event| {
            (guard.show_sensitive || !event.sensitive)
                && matches!(event.kind, StatEventKind::PageEncounter | StatEventKind::ActiveReading)
        })
        .map(|event| (event, series_for_event(event)))
        .collect();
    annotated.sort_by_key(|(event, _)| event.occurred_at_ms);
    let mut last_series: Option<(String, i64)> = None;
    for (event, series_id) in &mut annotated {
        if series_id.is_none() {
            if let Some((candidate, occurred_at_ms)) = &last_series {
                if event.occurred_at_ms - occurred_at_ms <= SESSION_GAP_MS {
                    *series_id = Some(candidate.clone());
                }
            }
        }
        if let Some(series_id) = series_id {
            last_series = Some((series_id.clone(), event.occurred_at_ms));
        }
    }
    let mut next_series: Option<(String, i64)> = None;
    for (event, series_id) in annotated.iter_mut().rev() {
        if series_id.is_none() {
            if let Some((candidate, occurred_at_ms)) = &next_series {
                if occurred_at_ms - event.occurred_at_ms <= SESSION_GAP_MS {
                    *series_id = Some(candidate.clone());
                }
            }
        }
        if let Some(series_id) = series_id {
            next_series = Some((series_id.clone(), event.occurred_at_ms));
        }
    }

    let mut grouped: std::collections::HashMap<String, Vec<&StatEvent>> = std::collections::HashMap::new();
    for (event, series_id) in annotated.into_iter()
        .filter(|(event, series_id)| include_date(event.local_date) && !is_hidden(event, series_id.as_deref())) {
        if let Some(series_id) = series_id {
            grouped.entry(series_id).or_default().push(event);
        }
    }
    let mut sessions = Vec::new();
    for (series_id, mut events) in grouped {
        events.sort_by_key(|event| event.occurred_at_ms);
        let title = titles.get(series_id.as_str()).map_or_else(|| "Unavailable manga".to_string(), |title| (*title).to_string());
        let mut current: Option<ReadingSession> = None;
        for event in events {
            let starts_new = current.as_ref().is_some_and(|session| event.occurred_at_ms - session.ended_at_ms > SESSION_GAP_MS);
            if starts_new {
                if let Some(session) = current.take().filter(|session| session.pages > 0 || session.active_seconds > 0) {
                    sessions.push(session);
                }
            }
            let session = current.get_or_insert_with(|| ReadingSession {
                session_id: event.id.clone(), series_id: series_id.clone(), title: title.clone(),
                started_at_ms: event.occurred_at_ms, ended_at_ms: event.occurred_at_ms,
                pages: 0, active_seconds: 0, event_ids: Vec::new(),
            });
            session.ended_at_ms = session.ended_at_ms.max(event.occurred_at_ms);
            session.event_ids.push(event.id.clone());
            match event.kind {
                StatEventKind::PageEncounter => session.pages += event.value,
                StatEventKind::ActiveReading => session.active_seconds += event.value,
                _ => {}
            }
        }
        if let Some(session) = current.filter(|session| session.pages > 0 || session.active_seconds > 0) {
            sessions.push(session);
        }
    }
    sessions.sort_by(|left, right| right.started_at_ms.cmp(&left.started_at_ms));
    sessions
}

async fn get_stats_detail(
    State(state): State<ForkApiState>,
    Query(query): Query<StatsDetailQuery>,
) -> Result<Json<StatsDetail>, ApiError> {
    let guard = state.inner.lock().map_err(internal)?;
    let today = Local::now().date_naive();
    let range = query.range.as_deref().unwrap_or("week");
    let include_date = |date: NaiveDate| match range {
        "day" => date == today,
        "week" => date >= today - chrono::Duration::days(6) && date <= today,
        "month" => date.year() == today.year() && date.month() == today.month(),
        "all" => true,
        _ => false,
    };
    if !matches!(range, "day" | "week" | "month" | "all") {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Unknown statistics range".into()));
    }
    let mut history: std::collections::BTreeMap<NaiveDate, (u64, u64)> = std::collections::BTreeMap::new();
    let mut per_series: std::collections::HashMap<String, (u64, u64, u64, u64)> = std::collections::HashMap::new();
    let hidden = (!guard.show_sensitive).then(|| hidden_content(&guard));
    for event in guard.events.iter().filter(|event| (guard.show_sensitive || !event.sensitive) && include_date(event.local_date)) {
        let hidden_now = hidden.as_ref().is_some_and(|hidden| {
            event.series_id.as_deref().is_some_and(|series_id| hidden.contains(series_id))
                || event_volume_key(event).is_some_and(|volume| hidden.contains(volume))
        });
        if hidden_now { continue; }
        let day = history.entry(event.local_date).or_default();
        match event.kind {
            StatEventKind::PageEncounter => day.0 += event.value,
            StatEventKind::ActiveReading => day.1 += event.value,
            _ => {}
        }
        if let Some(series_id) = &event.series_id {
            let values = per_series.entry(series_id.clone()).or_default();
            match event.kind {
                StatEventKind::PageEncounter => values.0 += event.value,
                StatEventKind::ActiveReading => values.1 += event.value,
                StatEventKind::DictionaryLookup => values.2 += event.value,
                StatEventKind::CardMined => values.3 += event.value,
                _ => {}
            }
        }
    }
    let history = history.into_iter().map(|(local_date, (pages, active_seconds))| StatsHistoryDay { local_date, pages, active_seconds }).collect();
    let mut per_series: Vec<_> = per_series.into_iter().map(|(series_id, (pages, active_seconds, lookups, cards))| {
        let title = guard.series.iter().find(|series| series.id == series_id).map(|series| series.title.clone()).unwrap_or_else(|| "Unavailable series".into());
        SeriesStats { series_id, title, pages, active_seconds, lookups, cards }
    }).collect();
    per_series.sort_by(|left, right| right.active_seconds.cmp(&left.active_seconds).then_with(|| right.pages.cmp(&left.pages)));
    Ok(Json(StatsDetail { history, per_series }))
}

async fn get_reading_sessions(
    State(state): State<ForkApiState>,
    Query(query): Query<StatsDetailQuery>,
) -> Result<Json<ReadingSessions>, ApiError> {
    let guard = state.inner.lock().map_err(internal)?;
    let today = Local::now().date_naive();
    let range = query.range.as_deref().unwrap_or("week");
    if !matches!(range, "day" | "week" | "month" | "all") {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Unknown statistics range".into()));
    }
    let include_date = |date: NaiveDate| match range {
        "day" => date == today,
        "week" => date >= today - chrono::Duration::days(6) && date <= today,
        "month" => date.year() == today.year() && date.month() == today.month(),
        "all" => true,
        _ => false,
    };
    Ok(Json(ReadingSessions { sessions: reading_sessions(&guard, &include_date) }))
}

async fn delete_stats_session(
    State(state): State<ForkApiState>,
    AxumPath(id): AxumPath<String>,
) -> Result<StatusCode, ApiError> {
    let mut guard = state.inner.lock().map_err(internal)?;
    let sessions = reading_sessions(&guard, &|_| true);
    let session = sessions.into_iter().find(|session| session.event_ids.iter().any(|event_id| event_id == &id))
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Reading session was not found".into()))?;
    let ids: std::collections::HashSet<_> = session.event_ids.into_iter().collect();
    guard.events.retain(|event| !ids.contains(&event.id));
    guard.stats = calculate_stats(&guard.events, guard.show_sensitive, Local::now().date_naive());
    state.persist(&guard)?;
    Ok(StatusCode::NO_CONTENT)
}

fn internal(error: impl std::fmt::Display) -> ApiError { ApiError(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()) }
fn connection(error: impl std::fmt::Display) -> ApiError { ApiError(StatusCode::BAD_GATEWAY, error.to_string()) }

fn scan_android_documents(uri: &str) -> Result<String, ApiError> {
    let android = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(android.vm().cast()) }.map_err(internal)?;
    let mut env = vm.attach_current_thread().map_err(internal)?;
    let context = unsafe { JObject::from_raw(android.context().cast()) };
    let uri = env.new_string(uri).map_err(internal)?;
    let scanner = app_class(&mut env, &context, "dev.local.mangareader.DocumentScanner")?;
    let result = match env.call_static_method(
        scanner, "scan",
        "(Landroid/content/Context;Ljava/lang/String;)Ljava/lang/String;",
        &[JValue::Object(&context), JValue::Object(&uri)],
    ).and_then(|value| value.l()) {
        Ok(result) => result,
        Err(error) => return Err(clear_java_exception(&mut env, error)),
    };
    let result = JString::from(result);
    Ok(env.get_string(&result).map_err(connection)?.into())
}

fn read_android_page(locator: &str) -> Result<Vec<u8>, ApiError> {
    if let Some(value) = locator.strip_prefix("zip:") {
        let (encoded_uri, index) = value.split_once(':')
            .ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "Invalid archive page locator".into()))?;
        let uri = String::from_utf8(URL_SAFE_NO_PAD.decode(encoded_uri).map_err(internal)?).map_err(internal)?;
        let index: usize = index.parse().map_err(internal)?;
        let file = open_android_archive(&uri)?;
        // Buffering turns the zip parser's many small reads into a few
        // descriptor reads.
        let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(connection)?;
        return zip_read_entry(&mut archive, index);
    }
    let android = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(android.vm().cast()) }.map_err(internal)?;
    let mut env = vm.attach_current_thread().map_err(internal)?;
    let context = unsafe { JObject::from_raw(android.context().cast()) };
    let locator = env.new_string(locator).map_err(internal)?;
    let scanner = app_class(&mut env, &context, "dev.local.mangareader.DocumentScanner")?;
    let result = match env.call_static_method(
        scanner, "readPage",
        "(Landroid/content/Context;Ljava/lang/String;)[B",
        &[JValue::Object(&context), JValue::Object(&locator)],
    ).and_then(|value| value.l()) {
        Ok(result) => result,
        Err(error) => return Err(clear_java_exception(&mut env, error)),
    };
    let result = JByteArray::from(result);
    env.convert_byte_array(&result).map_err(connection)
}

fn archive_android_pages(uri: &str) -> Result<(Vec<String>, String), ApiError> {
    let file = open_android_archive(uri)?;
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(connection)?;
    let mut entries = Vec::new();
    let mut fingerprint = std::collections::hash_map::DefaultHasher::new();
    for index in 0..archive.len() {
        let entry = archive.by_index_raw(index).map_err(connection)?;
        if entry.is_dir() || !is_archive_page_name(entry.name()) { continue; }
        entry.name_raw().hash(&mut fingerprint);
        entry.crc32().hash(&mut fingerprint);
        entry.size().hash(&mut fingerprint);
        entries.push((entry.name().to_string(), index));
        if entries.len() > 100_000 {
            return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "Archive contains too many pages".into()));
        }
    }
    entries.sort_by(|left, right| manga_reader_core::natural_cmp(&left.0, &right.0));
    let encoded_uri = URL_SAFE_NO_PAD.encode(uri.as_bytes());
    let pages = entries.into_iter().map(|(_, index)| format!("zip:{encoded_uri}:{index}")).collect();
    Ok((pages, format!("{:016x}", fingerprint.finish())))
}

fn is_image_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.ends_with(".jpg") || name.ends_with(".jpeg") || name.ends_with(".png") || name.ends_with(".webp")
}

/// macOS adds `__MACOSX/` folders and `._name` resource forks to archives it
/// creates; they carry image extensions but are not pages.
fn is_archive_page_name(name: &str) -> bool {
    let hidden = name.split(['/', '\\']).any(|part| part == "__MACOSX" || part.starts_with('.'));
    !hidden && !name.ends_with('/') && is_image_name(name)
}

fn open_android_archive(uri: &str) -> Result<fs::File, ApiError> {
    let android = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(android.vm().cast()) }.map_err(internal)?;
    let mut env = vm.attach_current_thread().map_err(internal)?;
    let context = unsafe { JObject::from_raw(android.context().cast()) };
    let uri = env.new_string(uri).map_err(internal)?;
    let scanner = app_class(&mut env, &context, "dev.local.mangareader.DocumentScanner")?;
    let fd = match env.call_static_method(
        scanner, "openArchive", "(Landroid/content/Context;Ljava/lang/String;)I",
        &[JValue::Object(&context), JValue::Object(&uri)],
    ).and_then(|value| value.i()) {
        Ok(fd) if fd >= 0 => fd,
        Ok(_) => return Err(ApiError(StatusCode::BAD_GATEWAY, "Archive descriptor is unavailable".into())),
        Err(error) => return Err(clear_java_exception(&mut env, error)),
    };
    Ok(unsafe { fs::File::from_raw_fd(fd) })
}

fn release_android_grant(uri: &str) -> Result<(), ApiError> {
    let android = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(android.vm().cast()) }.map_err(internal)?;
    let mut env = vm.attach_current_thread().map_err(internal)?;
    let context = unsafe { JObject::from_raw(android.context().cast()) };
    let uri = env.new_string(uri).map_err(internal)?;
    let scanner = app_class(&mut env, &context, "dev.local.mangareader.DocumentScanner")?;
    match env.call_static_method(
        scanner, "releaseGrant", "(Landroid/content/Context;Ljava/lang/String;)V",
        &[JValue::Object(&context), JValue::Object(&uri)],
    ) {
        Ok(_) => Ok(()),
        Err(error) => Err(clear_java_exception(&mut env, error)),
    }
}

fn app_class<'local>(
    env: &mut jni::JNIEnv<'local>,
    context: &JObject<'local>,
    name: &str,
) -> Result<JClass<'local>, ApiError> {
    let loader = match env.call_method(context, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])
        .and_then(|value| value.l()) {
        Ok(loader) => loader,
        Err(error) => return Err(clear_java_exception(env, error)),
    };
    let name = env.new_string(name).map_err(internal)?;
    let class = match env.call_method(
        loader,
        "loadClass",
        "(Ljava/lang/String;)Ljava/lang/Class;",
        &[JValue::Object(&name)],
    ).and_then(|value| value.l()) {
        Ok(class) => class,
        Err(error) => return Err(clear_java_exception(env, error)),
    };
    Ok(JClass::from(class))
}

fn clear_java_exception(env: &mut jni::JNIEnv<'_>, error: jni::errors::Error) -> ApiError {
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    connection(error)
}

fn secure_store_put(id: &str, payload: &str) -> Result<(), ApiError> {
    let context = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) }.map_err(internal)?;
    let mut env = vm.attach_current_thread().map_err(internal)?;
    let context = unsafe { JObject::from_raw(context.context().cast()) };
    let id = env.new_string(id).map_err(internal)?;
    let payload = env.new_string(payload).map_err(internal)?;
    let class = app_class(&mut env, &context, "dev.local.mangareader.SecureStore")?;
    match env.call_static_method(
        class, "put",
        "(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;)V",
        &[JValue::Object(&context), JValue::Object(&id), JValue::Object(&payload)],
    ) {
        Ok(_) => Ok(()),
        Err(error) => Err(clear_java_exception(&mut env, error)),
    }
}

fn secure_store_get(id: &str) -> Result<Option<String>, ApiError> {
    let android = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(android.vm().cast()) }.map_err(internal)?;
    let mut env = vm.attach_current_thread().map_err(internal)?;
    let context = unsafe { JObject::from_raw(android.context().cast()) };
    let id = env.new_string(id).map_err(internal)?;
    let class = app_class(&mut env, &context, "dev.local.mangareader.SecureStore")?;
    let value = match env.call_static_method(
        class, "get", "(Landroid/content/Context;Ljava/lang/String;)Ljava/lang/String;",
        &[JValue::Object(&context), JValue::Object(&id)],
    ).and_then(|value| value.l()) {
        Ok(value) => value,
        Err(error) => return Err(clear_java_exception(&mut env, error)),
    };
    if value.is_null() { return Ok(None); }
    let value = JString::from(value);
    Ok(Some(env.get_string(&value).map_err(connection)?.into()))
}

fn secure_store_delete(id: &str) -> Result<(), ApiError> {
    let context = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) }.map_err(internal)?;
    let mut env = vm.attach_current_thread().map_err(internal)?;
    let context = unsafe { JObject::from_raw(context.context().cast()) };
    let id = env.new_string(id).map_err(internal)?;
    let class = app_class(&mut env, &context, "dev.local.mangareader.SecureStore")?;
    match env.call_static_method(
        class, "delete",
        "(Landroid/content/Context;Ljava/lang/String;)V",
        &[JValue::Object(&context), JValue::Object(&id)],
    ) {
        Ok(_) => Ok(()),
        Err(error) => Err(clear_java_exception(&mut env, error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_state() -> ForkApiState {
        let directory = std::env::temp_dir().join(format!("mangaki-fork-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        ForkApiState::load(&directory)
    }

    fn remote_source(kind: SourceKind, address: String) -> Source {
        Source {
            id: Uuid::new_v4().to_string(), kind, name: "Fixture".into(), status: SourceStatus::Connected, detail: None,
            address: Some(address), allow_http: true, credential_ref: None, library_ids: vec!["library".into()],
            grouping_overrides: HashMap::new(),
        }
    }

    fn fixture_archive() -> (Vec<u8>, Vec<Vec<u8>>) {
        // Pages larger than the tail probe, out of natural order, one of them
        // deflated, with a non-image entry that must be ignored.
        let pages: Vec<Vec<u8>> = (0..3u8).map(|page| {
            let mut bytes = b"\x89PNG".to_vec();
            bytes.extend((0..400_000u32).map(|index| (index as u8).wrapping_mul(31).wrapping_add(page)));
            bytes
        }).collect();
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, page) in [("10.png", &pages[2]), ("notes.txt", &pages[0]), ("2.png", &pages[1]), ("1.png", &pages[0])] {
            let method = if name == "2.png" { zip::CompressionMethod::Deflated } else { zip::CompressionMethod::Stored };
            writer.start_file(name, options.compression_method(method)).unwrap();
            writer.write_all(if name.ends_with(".txt") { b"not a page" } else { page }).unwrap();
        }
        (writer.finish().unwrap().into_inner(), pages)
    }

    /// Serves `archive` at /dav/Series/Vol%201.cbz, honouring single byte
    /// ranges when `ranges` is true, and counts the bytes sent.
    async fn webdav_fixture(archive: Vec<u8>, ranges: bool) -> (String, Arc<AtomicU64>) {
        use axum::http::HeaderMap;
        let served = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&served);
        let archive = Arc::new(archive);
        let app = Router::new().route("/dav/Series/{name}", get(move |AxumPath(name): AxumPath<String>, headers: HeaderMap| {
            let archive = Arc::clone(&archive);
            let counter = Arc::clone(&counter);
            async move {
                assert_eq!(name, "Vol 1.cbz");
                let total = archive.len() as u64;
                let range = headers.get(header::RANGE).and_then(|value| value.to_str().ok()).and_then(|value| value.strip_prefix("bytes="))
                    .filter(|_| ranges)
                    .and_then(|value| {
                        let (first, last) = value.split_once('-')?;
                        if first.is_empty() { let length: u64 = last.parse().ok()?; return Some((total.saturating_sub(length), total - 1)); }
                        Some((first.parse().ok()?, last.parse::<u64>().ok()?.min(total - 1)))
                    });
                match range {
                    Some((first, last)) => {
                        let body = archive[first as usize..=last as usize].to_vec();
                        counter.fetch_add(body.len() as u64, Ordering::SeqCst);
                        Response::builder().status(StatusCode::PARTIAL_CONTENT)
                            .header(header::CONTENT_RANGE, format!("bytes {first}-{last}/{total}"))
                            .body(Body::from(body)).unwrap()
                    }
                    None => {
                        counter.fetch_add(total, Ordering::SeqCst);
                        Response::builder().status(StatusCode::OK).body(Body::from(archive.to_vec())).unwrap()
                    }
                }
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        (format!("http://{address}/dav"), served)
    }

    /// Lists and reads the fixture volume, returning bytes served after the
    /// listing, after the first page and after every page.
    async fn read_fixture_volume(ranges: bool) -> (ForkApiState, [u64; 3], u64) {
        let (archive, pages) = fixture_archive();
        let archive_size = archive.len() as u64;
        let (address, served) = webdav_fixture(archive, ranges).await;
        let state = temporary_state();
        let source = remote_source(SourceKind::Webdav, address);
        let source_id = source.id.clone();
        state.inner.lock().unwrap().sources.push(source);
        let locator = remote_locator("webdav", &source_id, "/Series/Vol%201.cbz");

        let indexes = remote_archive_pages(&state, &locator, true).await.unwrap();
        assert_eq!(indexes.len(), 3, "the text entry is not a page");
        let mut counts = [served.load(Ordering::SeqCst), 0, 0];
        let encoded = URL_SAFE_NO_PAD.encode(locator.as_bytes());
        for (position, index) in indexes.iter().enumerate() {
            let page = read_remote_page(&state, &format!("rzip:{encoded}:{index}")).await.unwrap();
            assert_eq!(page, pages[position], "pages follow natural order 1, 2, 10");
            if position == 0 { counts[1] = served.load(Ordering::SeqCst); }
        }
        counts[2] = served.load(Ordering::SeqCst);
        (state, counts, archive_size)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ranged_webdav_archives_are_read_without_downloading_whole_volume() {
        let (state, [listed, first_page, all_pages], archive_size) = read_fixture_volume(true).await;
        assert!(listed <= REMOTE_TAIL_BYTES, "listing transferred {listed} bytes");
        assert!(first_page - listed < 400_000 + 16 * 1024, "one page transferred {} bytes", first_page - listed);
        assert!(all_pages < archive_size + REMOTE_TAIL_BYTES + 3 * 16 * 1024, "served {all_pages} of {archive_size}");
        assert_eq!(fs::read_dir(&state.cache_dir).unwrap().count(), 0, "ranged reads need no full download");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn webdav_without_ranges_falls_back_to_one_cached_download() {
        let (state, [listed, _, all_pages], archive_size) = read_fixture_volume(false).await;
        assert_eq!(listed, archive_size, "the probe response itself becomes the cached copy");
        assert_eq!(all_pages, archive_size, "pages are then served from the cache");
        assert_eq!(fs::read_dir(&state.cache_dir).unwrap().count(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn komga_progress_uses_one_based_pages_in_both_directions() {
        let received = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
        let sink = Arc::clone(&received);
        let books = serde_json::json!({ "content": [
            { "id": "b1", "seriesId": "s1", "seriesTitle": "Series", "name": "Vol 1", "fileHash": "h1", "lastModified": "t",
              "media": { "pagesCount": 10, "status": "READY" }, "readProgress": { "page": 4, "completed": false } },
            { "id": "b2", "seriesId": "s1", "seriesTitle": "Series", "name": "Vol 2", "fileHash": "h2", "lastModified": "t",
              "media": { "pagesCount": 10, "status": "READY" }, "readProgress": { "page": 10, "completed": true } }
        ] });
        let app = Router::new()
            .route("/api/v1/books", get(move || { let books = books.clone(); async move { Json(books) } }))
            .route("/api/v1/books/{id}/read-progress", axum::routing::patch(move |Json(body): Json<serde_json::Value>| {
                let sink = Arc::clone(&sink);
                async move { sink.lock().unwrap().push(body); StatusCode::NO_CONTENT }
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });

        let state = temporary_state();
        let source = remote_source(SourceKind::Komga, format!("http://{address}"));
        let scanned = scan_komga(&state, &source).await.unwrap();
        let volumes = &scanned.series[0].volumes;
        assert_eq!(volumes[0].progress_page, 3, "Komga page 4 is zero-based index 3");
        assert_eq!(volumes[1].progress_page, 9, "completed books resume on their last page");

        let source_id = source.id.clone();
        {
            let mut guard = state.inner.lock().unwrap();
            guard.sources.push(source);
            guard.pending_progress.push(PendingProgress {
                source_id: source_id.clone(), volume_id: "v".into(), remote_book_id: "b1".into(),
                page: 9, page_count: 10, updated_at_ms: 1,
            });
        }
        flush_pending_progress(&state, &source_id).await;
        assert_eq!(received.lock().unwrap().as_slice(), &[serde_json::json!({ "page": 10, "completed": true })]);
        assert!(state.inner.lock().unwrap().pending_progress.is_empty());
    }

    #[tokio::test]
    async fn stored_pages_revalidate_against_the_volume_fingerprint() {
        let state = temporary_state();
        let record = OfflinePage { volume_id: "v".into(), page: 0, fingerprint: "fp1".into() };
        fs::write(offline_page_path(&state.offline_dir, &record), b"RIFF\0\0\0\0WEBPpage").unwrap();
        {
            let mut guard = state.inner.lock().unwrap();
            guard.series.push(Series { id: "s".into(), source_id: "src".into(), title: "S".into(), sensitive: false, category_ids: Vec::new(),
                volumes: vec![Volume { id: "v".into(), title: "V".into(), page_count: 1, fingerprint: "fp1".into(), pages: vec!["smb:missing:x".into()],
                    sensitive: false, progress_page: 0, ocr_state: OcrState::default(), download_state: DownloadState::default(), error: None }] });
            guard.offline_pages.push(record);
        }
        let request = |etag: Option<&str>, fingerprint: Option<&str>| {
            let mut headers = axum::http::HeaderMap::new();
            if let Some(etag) = etag { headers.insert(header::IF_NONE_MATCH, etag.parse().unwrap()); }
            if let Some(fingerprint) = fingerprint { headers.insert(EXPECTED_FINGERPRINT_HEADER, fingerprint.parse().unwrap()); }
            get_page(State(state.clone()), AxumPath(("v".to_string(), 0)), headers)
        };
        let response = request(None, None).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/webp");
        let etag = response.headers()[header::ETAG].to_str().unwrap().to_string();
        assert_eq!(request(Some(&etag), None).await.unwrap().status(), StatusCode::NOT_MODIFIED);
        assert_eq!(request(None, Some("stale")).await.unwrap_err().0, StatusCode::CONFLICT);

        state.inner.lock().unwrap().series[0].volumes[0].fingerprint = "fp2".into();
        // The changed volume no longer matches the client's copy or the stored file.
        assert!(request(Some(&etag), None).await.is_err_and(|error| error.0 != StatusCode::NOT_MODIFIED));
    }

    #[test]
    fn series_marked_nsfw_later_leave_itemised_history() {
        let date: NaiveDate = "2026-09-12".parse().unwrap();
        let mut state = ForkState::default();
        state.series.push(Series { id: "series".into(), source_id: "s".into(), title: "Private title".into(), sensitive: false,
            category_ids: Vec::new(), volumes: Vec::new() });
        state.events.push(StatEvent { id: "e".into(), kind: StatEventKind::PageEncounter, local_date: date, occurred_at_ms: 1,
            value: 1, unique_key: Some("v:0".into()), series_id: Some("series".into()), sensitive: false });
        assert_eq!(reading_sessions(&state, &|_| true).len(), 1);
        state.series[0].sensitive = true;
        assert!(reading_sessions(&state, &|_| true).is_empty());
        state.show_sensitive = true;
        assert_eq!(reading_sessions(&state, &|_| true).len(), 1);
    }

    #[tokio::test]
    async fn cover_thumbnails_are_small_cached_and_revalidated() {
        let state = temporary_state();
        let record = OfflinePage { volume_id: "v".into(), page: 0, fingerprint: "fp".into() };
        let mut page = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(1200, 1800, image::Rgb([200, 30, 30])).write_to(&mut page, image::ImageFormat::Png).unwrap();
        fs::write(offline_page_path(&state.offline_dir, &record), page.get_ref()).unwrap();
        {
            let mut guard = state.inner.lock().unwrap();
            guard.series.push(Series { id: "s".into(), source_id: "src".into(), title: "S".into(), sensitive: false, category_ids: Vec::new(),
                volumes: vec![Volume { id: "v".into(), title: "V".into(), page_count: 1, fingerprint: "fp".into(), pages: vec!["smb:missing:x".into()],
                    sensitive: false, progress_page: 0, ocr_state: OcrState::default(), download_state: DownloadState::default(), error: None }] });
            guard.offline_pages.push(record);
        }
        let fetch = |headers| get_thumbnail(State(state.clone()), AxumPath("v".to_string()), headers);
        let response = fetch(axum::http::HeaderMap::new()).await.unwrap();
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/jpeg");
        let etag = response.headers()[header::ETAG].clone();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let thumbnail = image::load_from_memory(&body).unwrap();
        assert_eq!((thumbnail.width(), thumbnail.height()), (360, 540));
        assert!(body.len() < page.get_ref().len());
        assert_eq!(fs::read_dir(&state.thumbnail_dir).unwrap().count(), 1, "thumbnail is cached on disk");

        let mut headers = axum::http::HeaderMap::new();
        headers.insert(header::IF_NONE_MATCH, etag);
        assert_eq!(fetch(headers).await.unwrap().status(), StatusCode::NOT_MODIFIED);
    }

    #[test]
    fn macos_archive_metadata_is_not_a_page() {
        assert!(is_archive_page_name("Vol 1/001.JPG"));
        assert!(!is_archive_page_name("__MACOSX/Vol 1/._001.jpg"));
        assert!(!is_archive_page_name("Vol 1/._001.jpg"));
        assert!(!is_archive_page_name("folder.png/"));
    }

    #[test]
    fn older_snapshots_never_replace_newer_state_files() {
        let state = temporary_state();
        state.write_snapshot(2, b"{\"showSensitive\":true}").unwrap();
        state.write_snapshot(1, b"{}").unwrap();
        let stored: ForkState = serde_json::from_slice(&fs::read(&state.path).unwrap()).unwrap();
        assert!(stored.show_sensitive);
    }

    #[test]
    fn ui_views_hide_sensitive_content_and_internal_locators() {
        let volume = |id: &str, sensitive: bool| Volume {
            id: id.into(), title: id.into(), page_count: 1, fingerprint: id.into(), pages: vec!["secret-locator".into()],
            sensitive, progress_page: 0, ocr_state: OcrState::default(), download_state: DownloadState::default(), error: None,
        };
        let mut state = ForkState::default();
        state.series.push(Series { id: "open".into(), source_id: "s".into(), title: "Open".into(), sensitive: false, category_ids: Vec::new(),
            volumes: vec![volume("v1", false), volume("v2", true)] });
        state.series.push(Series { id: "hidden".into(), source_id: "s".into(), title: "Hidden".into(), sensitive: true, category_ids: Vec::new(),
            volumes: vec![volume("v3", false)] });
        state.jobs.push(Job { id: "j".into(), kind: "ocr".into(), title: "Hidden".into(), sensitive: true, state: "queued".into(),
            completed_pages: 0, total_pages: Some(1), error: None, pages: Vec::new(), prepare_ocr_after: false, reprocess: false,
            ocr_provider: None, self_hosted_url: None });

        let catalogue = visible_catalogue(&state);
        assert_eq!(catalogue.len(), 1);
        assert_eq!(catalogue[0].volumes.iter().map(|volume| volume.id.as_str()).collect::<Vec<_>>(), vec!["v1"]);
        assert!(catalogue[0].volumes.iter().all(|volume| volume.pages.is_empty()));
        let view = serde_json::to_string(&state_view(&state)).unwrap();
        assert!(!view.contains("secret-locator") && !view.contains("\"Hidden\""));

        state.show_sensitive = true;
        assert_eq!(visible_catalogue(&state).iter().map(|series| series.volumes.len()).sum::<usize>(), 3);
    }

    #[test]
    fn defaults_do_not_fabricate_statistics_or_show_sensitive_content() {
        let state = ForkState::default();
        assert!(!state.show_sensitive);
        assert_eq!(state.stats.all.cards_mined, 0);
        assert_eq!(state.stats.all.ocr_characters, None);
    }

    #[test]
    fn plaintext_credentials_are_not_serialized() {
        let request = SourceRequest { kind: SourceKind::Komga, name: "K".into(), address: "https://example.invalid".into(), username: "u".into(), password: Some("secret".into()), api_key: Some("key".into()), allow_http: false, library_ids: vec!["library".into()] };
        let source = Source { id: "id".into(), kind: request.kind, name: request.name, status: SourceStatus::Connected, detail: None, address: Some(request.address), allow_http: false, credential_ref: Some("id".into()), library_ids: request.library_ids, grouping_overrides: std::collections::HashMap::new() };
        let json = serde_json::to_string(&source).unwrap();
        assert!(!json.contains("secret"));
        assert!(!json.contains("key"));
    }

    #[test]
    fn grouping_overrides_merge_detected_series_without_touching_pages() {
        let volume = |id: &str, title: &str| ScannedVolume {
            id: id.into(), title: title.into(), page_count: 1, fingerprint: id.into(),
            pages: vec![format!("page:{id}")], archive_uri: None, error: None, progress_page: 0,
        };
        let mut payload = ScanPayload { series: vec![
            ScannedSeries { id: "a".into(), title: "Work A".into(), volumes: vec![volume("v2", "Vol 2")] },
            ScannedSeries { id: "b".into(), title: "Work B".into(), volumes: vec![volume("v1", "Vol 1")] },
        ] };
        let overrides = [("a".into(), "Merged".into()), ("b".into(), "Merged".into())].into_iter().collect();
        apply_grouping_overrides(&mut payload, &overrides);
        assert_eq!(payload.series.len(), 1);
        assert_eq!(payload.series[0].title, "Merged");
        assert_eq!(payload.series[0].volumes.iter().map(|volume| volume.title.as_str()).collect::<Vec<_>>(), vec!["Vol 1", "Vol 2"]);
        assert_eq!(payload.series[0].volumes.iter().map(|volume| volume.pages.len()).sum::<usize>(), 2);
    }

    #[test]
    fn reading_sessions_recover_series_from_fork_page_keys() {
        let date: NaiveDate = "2026-09-12".parse().unwrap();
        let event = |id: &str, kind: StatEventKind, occurred_at_ms: i64, unique_key: Option<&str>| StatEvent {
            id: id.into(), kind, local_date: date, occurred_at_ms, value: if kind == StatEventKind::ActiveReading { 20 } else { 1 },
            unique_key: unique_key.map(str::to_owned), series_id: None, sensitive: false,
        };
        let mut state = ForkState::default();
        state.series.push(Series {
            id: "series".into(), source_id: "source".into(), title: "Manga".into(), sensitive: false,
            category_ids: Vec::new(), volumes: vec![Volume {
                id: "volume".into(), title: "Volume".into(), page_count: 2, fingerprint: "fp".into(),
                pages: Vec::new(), sensitive: false, progress_page: 0, ocr_state: OcrState::default(),
                download_state: DownloadState::default(), error: None,
            }],
        });
        state.events = vec![
            event("page-legacy", StatEventKind::PageEncounter, 1_000, Some("fp:0")),
            event("active", StatEventKind::ActiveReading, 21_000, None),
            event("page-route", StatEventKind::PageEncounter, 41_000, Some("/fork/reader/volume#1")),
        ];

        let sessions = reading_sessions(&state, &|_| true);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].series_id, "series");
        assert_eq!(sessions[0].title, "Manga");
        assert_eq!(sessions[0].pages, 2);
        assert_eq!(sessions[0].active_seconds, 20);
        assert_eq!(sessions[0].event_ids.len(), 3);
    }
}
