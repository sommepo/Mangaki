use std::{
    fs,
    hash::{Hash, Hasher},
    io::Read,
    os::fd::FromRawFd,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
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
use uuid::Uuid;
use jni::objects::{JByteArray, JClass, JObject, JString, JValue};
use chrono::{Datelike, Local, NaiveDate};
use manga_reader_core::{StatEvent, StatEventKind, StatsRanges, calculate_stats};

#[derive(Clone)]
pub struct ForkApiState {
    path: PathBuf,
    cache_dir: PathBuf,
    offline_dir: PathBuf,
    inner: Arc<Mutex<ForkState>>,
    job_worker: Arc<tokio::sync::Mutex<()>>,
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
struct KomgaProgress { page: u32 }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PendingProgress {
    source_id: String,
    volume_id: String,
    remote_book_id: String,
    page: u32,
    updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OfflinePage {
    volume_id: String,
    page: u32,
    fingerprint: String,
}

#[derive(Debug, Deserialize, Default)]
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
        let cache_dir = data_dir.join("remote-archive-cache");
        let offline_dir = data_dir.join("offline-pages");
        let _ = fs::create_dir_all(&cache_dir);
        let _ = fs::create_dir_all(&offline_dir);
        let inner = fs::read(&path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
        Self {
            path,
            cache_dir,
            offline_dir,
            inner: Arc::new(Mutex::new(inner)),
            job_worker: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    fn persist(&self, value: &ForkState) -> Result<(), ApiError> {
        let bytes = serde_json::to_vec_pretty(value).map_err(internal)?;
        let temporary = self.path.with_extension("json.tmp");
        fs::write(&temporary, bytes).map_err(internal)?;
        fs::rename(&temporary, &self.path).map_err(internal)
    }
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
    for id in recover {
        let state = state.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
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
        .route("/backup", get(export_backup).put(restore_backup))
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
        let _ = state.persist(&guard);
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
        let result = Client::new().get(endpoint).send().await
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
            if !remaining {
                job.state = if failed == 0 { "complete" } else if job.completed_pages == 0 { "failed" } else { "partial" }.into();
                job.error = (failed > 0).then(|| format!("{failed} page(s) failed; retry missing pages"));
            }
            update_ocr_rollups(&mut guard, &id);
            let _ = state.persist(&guard);
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
        let _ = state.persist(&guard);
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
            let response = Client::new().get(page_url).send().await.map_err(connection)?.error_for_status().map_err(connection)?;
            const MAX_PAGE_BYTES: u64 = 64 * 1024 * 1024;
            if response.content_length().is_some_and(|length| length > MAX_PAGE_BYTES) {
                return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "Page exceeds 64 MiB safety limit".into()));
            }
            let bytes = response.bytes().await.map_err(connection)?;
            if bytes.len() as u64 > MAX_PAGE_BYTES { return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "Page exceeds 64 MiB safety limit".into())); }
            let record = OfflinePage { volume_id: page.volume_id.clone(), page: page.page, fingerprint: page.fingerprint.clone() };
            let path = offline_page_path(&state.offline_dir, &record);
            let temporary = path.with_extension("tmp");
            fs::write(&temporary, &bytes).map_err(internal)?;
            fs::rename(&temporary, &path).map_err(internal)?;
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
        if !remaining {
            job.state = if failed == 0 { "complete" } else if job.completed_pages == 0 { "failed" } else { "partial" }.into();
            job.error = (failed > 0).then(|| format!("{failed} page(s) failed; retry missing pages"));
        }
        update_download_rollups(&mut guard, &id);
        let _ = state.persist(&guard);
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

async fn restore_backup(State(state): State<ForkApiState>, Json(mut backup): Json<BackupPayload>) -> Result<Json<ForkState>, ApiError> {
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
    *guard = backup.state.clone();
    Ok(Json(backup.state))
}

async fn get_state(State(state): State<ForkApiState>) -> Result<Json<ForkState>, ApiError> {
    let mut guard = state.inner.lock().map_err(internal)?;
    guard.stats = calculate_stats(&guard.events, guard.show_sensitive, Local::now().date_naive());
    Ok(Json(guard.clone()))
}

async fn set_privacy(State(state): State<ForkApiState>, Json(request): Json<PrivacyRequest>) -> Result<Json<ForkState>, ApiError> {
    let mut guard = state.inner.lock().map_err(internal)?;
    guard.show_sensitive = request.show_sensitive;
    guard.stats = calculate_stats(&guard.events, guard.show_sensitive, Local::now().date_naive());
    state.persist(&guard)?;
    Ok(Json(guard.clone()))
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
    Ok(Json(guard.series.iter().filter(|series| {
        guard.show_sensitive || (!series.sensitive && series.volumes.iter().any(|volume| !volume.sensitive))
    }).cloned().map(|mut series| {
        if !guard.show_sensitive { series.volumes.retain(|volume| !volume.sensitive); }
        series
    }).collect()))
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
            let uri = source.address.as_deref().ok_or_else(|| ApiError(StatusCode::CONFLICT, "Re-authorise this folder before scanning".into()))?;
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
        SourceKind::Komga => scan_komga(&source).await,
        SourceKind::Smb => scan_smb(&state, &source).await,
        SourceKind::Webdav => scan_webdav(&state, &source).await,
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
    Ok(guard.series.clone())
}

fn apply_grouping_overrides(scanned: &mut ScanPayload, overrides: &std::collections::HashMap<String, String>) {
    for series in &mut scanned.series {
        if let Some(title) = overrides.get(&series.id) { series.title = title.clone(); }
    }
    scanned.series.sort_by(|left, right| manga_reader_core::natural_cmp(&left.title, &right.title).then_with(|| left.id.cmp(&right.id)));
    let mut merged: Vec<ScannedSeries> = Vec::new();
    for mut series in scanned.series.drain(..) {
        if let Some(existing) = merged.iter_mut().find(|existing| existing.title.eq_ignore_ascii_case(&series.title)) {
            existing.volumes.append(&mut series.volumes);
            existing.volumes.sort_by(|left, right| manga_reader_core::natural_cmp(&left.title, &right.title));
        } else {
            merged.push(series);
        }
    }
    scanned.series = merged;
}

async fn get_page(
    State(state): State<ForkApiState>,
    AxumPath((id, page)): AxumPath<(String, usize)>,
) -> Result<Response, ApiError> {
    let (locator, offline) = {
        let guard = state.inner.lock().map_err(internal)?;
        let volume = guard.series.iter().flat_map(|series| &series.volumes).find(|volume| volume.id == id)
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Volume not found".into()))?;
        let locator = volume.pages.get(page).cloned().ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Page not found".into()))?;
        let record = OfflinePage { volume_id: volume.id.clone(), page: page.try_into().map_err(internal)?, fingerprint: volume.fingerprint.clone() };
        let offline = guard.offline_pages.iter().any(|stored| stored.volume_id == record.volume_id && stored.page == record.page && stored.fingerprint == record.fingerprint)
            .then(|| offline_page_path(&state.offline_dir, &record));
        (locator, offline)
    };
    let bytes = if let Some(path) = offline.filter(|path| path.is_file()) {
        fs::read(path).map_err(internal)?
    } else if locator.starts_with("komga:") {
        read_komga_page(&state, &locator).await?
    } else if locator.starts_with("webdav:") || locator.starts_with("smb:") || locator.starts_with("rzip:") {
        read_remote_page(&state, &locator).await?
    } else {
        read_android_page(&locator)?
    };
    let mime = if bytes.starts_with(b"\x89PNG") { "image/png" }
        else if bytes.starts_with(b"RIFF") { "image/webp" }
        else { "image/jpeg" };
    Ok(Response::builder().status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CACHE_CONTROL, "private, max-age=3600")
        .body(Body::from(bytes)).map_err(internal)?)
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
    let response = authenticate_credentials(Client::new().get(url), &credentials).send().await.map_err(connection)?;
    if !response.status().is_success() { return Err(ApiError(StatusCode::BAD_GATEWAY, format!("Komga page returned {}", response.status()))); }
    const MAX_PAGE_BYTES: u64 = 64 * 1024 * 1024;
    if response.content_length().is_some_and(|length| length > MAX_PAGE_BYTES) {
        return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "Page exceeds 64 MiB safety limit".into()));
    }
    let bytes = response.bytes().await.map_err(connection)?;
    if bytes.len() as u64 > MAX_PAGE_BYTES { return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "Page exceeds 64 MiB safety limit".into())); }
    Ok(bytes.to_vec())
}

async fn read_remote_page(state: &ForkApiState, locator: &str) -> Result<Vec<u8>, ApiError> {
    if let Some(value) = locator.strip_prefix("rzip:") {
        let (encoded, index) = value.split_once(':').ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "Invalid remote archive page locator".into()))?;
        let remote = String::from_utf8(URL_SAFE_NO_PAD.decode(encoded).map_err(connection)?).map_err(connection)?;
        let index: usize = index.parse().map_err(connection)?;
        let bytes = cached_remote_archive(state, &remote).await?;
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(connection)?;
        let mut entry = archive.by_index(index).map_err(connection)?;
        const MAX_PAGE_BYTES: u64 = 64 * 1024 * 1024;
        if entry.size() > MAX_PAGE_BYTES { return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "Page exceeds 64 MiB safety limit".into())); }
        let mut page = Vec::with_capacity(entry.size() as usize);
        entry.take(MAX_PAGE_BYTES + 1).read_to_end(&mut page).map_err(connection)?;
        if page.len() as u64 > MAX_PAGE_BYTES { return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "Page exceeds 64 MiB safety limit".into())); }
        return Ok(page);
    }
    read_remote_bytes(state, locator, 64 * 1024 * 1024).await
}

async fn remote_archive_pages(state: &ForkApiState, locator: &str) -> Result<Vec<usize>, ApiError> {
    let bytes = cached_remote_archive(state, locator).await?;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(connection)?;
    let mut entries = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index_raw(index).map_err(connection)?;
        if entry.is_dir() || !is_image_name(entry.name()) { continue; }
        entries.push((entry.name().to_string(), index));
        if entries.len() > 100_000 { return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "Archive contains too many pages".into())); }
    }
    entries.sort_by(|left, right| manga_reader_core::natural_cmp(&left.0, &right.0));
    Ok(entries.into_iter().map(|(_, index)| index).collect())
}

async fn cached_remote_archive(state: &ForkApiState, locator: &str) -> Result<Vec<u8>, ApiError> {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    locator.hash(&mut hash);
    let cache_path = state.cache_dir.join(format!("{:016x}.zip", hash.finish()));
    if let Ok(bytes) = fs::read(&cache_path) { return Ok(bytes); }
    const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;
    let bytes = read_remote_bytes(state, locator, MAX_ARCHIVE_BYTES).await?;
    trim_remote_cache(&state.cache_dir, bytes.len() as u64)?;
    let temporary = cache_path.with_extension("tmp");
    fs::write(&temporary, &bytes).map_err(internal)?;
    fs::rename(&temporary, &cache_path).map_err(internal)?;
    Ok(bytes)
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

async fn read_remote_bytes(state: &ForkApiState, locator: &str, limit: u64) -> Result<Vec<u8>, ApiError> {
    let mut parts = locator.splitn(3, ':');
    let protocol = parts.next().ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "Invalid remote locator".into()))?;
    let source_id = parts.next().ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "Invalid remote source locator".into()))?;
    let encoded_path = parts.next().ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "Invalid remote path locator".into()))?;
    let path = String::from_utf8(URL_SAFE_NO_PAD.decode(encoded_path).map_err(connection)?).map_err(connection)?;
    let source = state.inner.lock().map_err(internal)?.sources.iter().find(|source| source.id == source_id).cloned()
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Remote source is unavailable".into()))?;
    let credentials = source_credentials(&source)?;
    let bytes = match protocol {
        "webdav" if source.kind == SourceKind::Webdav => {
            let address = source.address.as_deref().ok_or_else(|| ApiError(StatusCode::CONFLICT, "WebDAV address is missing".into()))?;
            let auth = if credentials.username.is_empty() { reqwest_dav::Auth::Anonymous }
                else { reqwest_dav::Auth::Basic(credentials.username, credentials.password.unwrap_or_default()) };
            let client = reqwest_dav::ClientBuilder::new().set_host(address.trim_end_matches('/').to_string()).set_auth(auth).build().map_err(connection)?;
            let response = client.get(&path).await.map_err(connection)?;
            if response.content_length().is_some_and(|length| length > limit) { return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "Remote file exceeds the safety limit".into())); }
            response.bytes().await.map_err(connection)?.to_vec()
        }
        "smb" if source.kind == SourceKind::Smb => {
            let address = source.address.as_deref().ok_or_else(|| ApiError(StatusCode::CONFLICT, "SMB address is missing".into()))?;
            let (server, share_name, _) = parse_smb_address(address)?;
            let mut client = smb2::connect(&server, &credentials.username, credentials.password.as_deref().unwrap_or_default()).await.map_err(connection)?;
            let mut share = client.connect_share(&share_name).await.map_err(connection)?;
            let bytes = client.read_file(&mut share, path.trim_start_matches('/')).await.map_err(connection)?;
            client.disconnect_share(&share).await.map_err(connection)?;
            bytes
        }
        _ => return Err(ApiError(StatusCode::BAD_REQUEST, "Remote locator does not match its source".into())),
    };
    if bytes.len() as u64 > limit { return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "Remote file exceeds the safety limit".into())); }
    Ok(bytes)
}

async fn set_progress(State(state): State<ForkApiState>, AxumPath(id): AxumPath<String>, Json(request): Json<ProgressRequest>) -> Result<StatusCode, ApiError> {
    let source_id = {
        let mut guard = state.inner.lock().map_err(internal)?;
        let source_id = guard.series.iter().find(|series| series.volumes.iter().any(|volume| volume.id == id)).map(|series| series.source_id.clone())
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Volume not found".into()))?;
        let volume = guard.series.iter_mut().flat_map(|series| &mut series.volumes).find(|volume| volume.id == id)
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Volume not found".into()))?;
        if request.page >= volume.page_count { return Err(ApiError(StatusCode::BAD_REQUEST, "Page is outside this volume".into())); }
        volume.progress_page = request.page;
        let remote_book_id = volume.id.split_once(":komga-book:").map(|(_, remote)| remote.to_string());
        if let Some(remote_book_id) = remote_book_id {
            guard.pending_progress.retain(|pending| pending.volume_id != id);
            guard.pending_progress.push(PendingProgress {
                source_id: source_id.clone(), volume_id: id.clone(), remote_book_id,
                page: request.page, updated_at_ms: chrono::Utc::now().timestamp_millis(),
            });
        }
        state.persist(&guard)?;
        source_id
    };
    flush_pending_progress(&state, &source_id).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn flush_pending_progress(state: &ForkApiState, source_id: &str) {
    let (source, pending) = {
        let Ok(guard) = state.inner.lock() else { return; };
        let Some(source) = guard.sources.iter().find(|source| source.id == source_id && source.kind == SourceKind::Komga).cloned() else { return; };
        (source, guard.pending_progress.iter().filter(|item| item.source_id == source_id).cloned().collect::<Vec<_>>())
    };
    let Ok(credentials) = source_credentials(&source) else { return; };
    let Some(base) = source.address.as_deref() else { return; };
    let client = Client::new();
    for item in pending {
        let url = format!("{}/api/v1/books/{}/read-progress", base.trim_end_matches('/'), urlencoding::encode(&item.remote_book_id));
        let response = authenticate_credentials(client.patch(url).json(&serde_json::json!({ "page": item.page, "completed": false })), &credentials).send().await;
        if response.as_ref().is_ok_and(|response| response.status().is_success()) {
            if let Ok(mut guard) = state.inner.lock() {
                guard.pending_progress.retain(|pending| !(pending.volume_id == item.volume_id && pending.updated_at_ms <= item.updated_at_ms));
                let _ = state.persist(&guard);
            }
        }
    }
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
    for archive in archives {
        let parent = remote_parent(&archive.path);
        let stem = remote_file_stem(&archive.path);
        let series_key = if parent.is_empty() { archive.path.clone() } else { parent.clone() };
        let series_title = if parent.is_empty() { stem.clone() } else { remote_title(&parent).unwrap_or_else(|| source.name.clone()) };
        let series_id = stable_remote_id(&source.id, "series", &series_key);
        let volume_id = stable_remote_id(&source.id, "volume", &archive.path);
        let locator = remote_locator(protocol, &source.id, &archive.path);
        let result = remote_archive_pages(state, &locator).await;
        let (pages, error) = match result {
            Ok(indexes) => (indexes.into_iter().map(|index| format!("rzip:{}:{index}", URL_SAFE_NO_PAD.encode(locator.as_bytes()))).collect::<Vec<_>>(), None),
            Err(_) => (Vec::new(), Some("Remote archive is not a readable ZIP/CBZ file".into())),
        };
        series_map.entry(series_id.clone()).or_insert_with(|| ScannedSeries { id: series_id, title: series_title, volumes: Vec::new() })
            .volumes.push(ScannedVolume {
                id: volume_id,
                title: stem,
                page_count: pages.len().try_into().map_err(internal)?,
                fingerprint: format!("{}:{}:{}", archive.size, archive.modified, archive.path),
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

async fn scan_komga(source: &Source) -> Result<ScanPayload, ApiError> {
    let base = source.address.as_deref().ok_or_else(|| ApiError(StatusCode::CONFLICT, "Komga address is missing".into()))?.trim_end_matches('/');
    let credentials = source_credentials(source)?;
    let client = Client::builder().redirect(reqwest::redirect::Policy::limited(5)).build().map_err(internal)?;
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
        let progress = book.read_progress.as_ref().map_or(0, |progress| progress.page.min(book.media.pages_count.saturating_sub(1)));
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
    state.persist(&guard)?;
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

fn reading_sessions(guard: &ForkState, include_date: &dyn Fn(NaiveDate) -> bool) -> Vec<ReadingSession> {
    const SESSION_GAP_MS: i64 = 15 * 60 * 1_000;
    fn series_for_event(guard: &ForkState, event: &StatEvent) -> Option<String> {
        if let Some(series_id) = &event.series_id {
            return Some(series_id.clone());
        }
        let key = event.unique_key.as_deref()?;
        let volume_id = key
            .strip_prefix("/fork/reader/")
            .and_then(|value| value.split_once('#').map(|(volume_id, _)| volume_id))
            .or_else(|| {
                key.rsplit_once(':')
                    .filter(|(_, page)| page.parse::<u32>().is_ok())
                    .map(|(volume_id, _)| volume_id)
            })?;
        guard.series.iter()
            .find(|series| {
                series.volumes.iter().any(|volume| volume.id == volume_id || volume.fingerprint == volume_id)
            })
            .map(|series| series.id.clone())
    }

    // Older fork readers recorded the volume in uniqueKey but omitted seriesId.
    // Recover those page anchors and associate adjacent active-reading ticks so
    // existing history remains visible without mutating the user's event log.
    let mut annotated: Vec<_> = guard.events.iter()
        .filter(|event| {
            (guard.show_sensitive || !event.sensitive)
                && matches!(event.kind, StatEventKind::PageEncounter | StatEventKind::ActiveReading)
        })
        .map(|event| (event, series_for_event(guard, event)))
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
    for (event, series_id) in annotated.into_iter().filter(|(event, _)| include_date(event.local_date)) {
        if let Some(series_id) = series_id {
            grouped.entry(series_id).or_default().push(event);
        }
    }
    let mut sessions = Vec::new();
    for (series_id, mut events) in grouped {
        events.sort_by_key(|event| event.occurred_at_ms);
        let title = guard.series.iter().find(|series| series.id == series_id)
            .map(|series| series.title.clone()).unwrap_or_else(|| "Unavailable manga".into());
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
    for event in guard.events.iter().filter(|event| (guard.show_sensitive || !event.sensitive) && include_date(event.local_date)) {
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
        let mut archive = zip::ZipArchive::new(file).map_err(connection)?;
        let mut entry = archive.by_index(index).map_err(connection)?;
        const MAX_PAGE_BYTES: u64 = 64 * 1024 * 1024;
        if entry.size() > MAX_PAGE_BYTES {
            return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "Page exceeds 64 MiB safety limit".into()));
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.take(MAX_PAGE_BYTES + 1).read_to_end(&mut bytes).map_err(connection)?;
        if bytes.len() as u64 > MAX_PAGE_BYTES {
            return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "Page exceeds 64 MiB safety limit".into()));
        }
        return Ok(bytes);
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
    let mut archive = zip::ZipArchive::new(file).map_err(connection)?;
    let mut entries = Vec::new();
    let mut fingerprint = std::collections::hash_map::DefaultHasher::new();
    for index in 0..archive.len() {
        let entry = archive.by_index_raw(index).map_err(connection)?;
        if entry.is_dir() || !is_image_name(entry.name()) { continue; }
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
