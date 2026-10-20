#![cfg(target_os = "android")]
mod fork_api;
use std::{
    collections::VecDeque,
    ffi::{CString, c_void},
    fs::{self, File},
    io::{self, BufReader},
    os::unix::io::FromRawFd,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    http::{StatusCode, Uri},
    response::IntoResponse,
    routing::get,
};
use eframe::egui;
use flate2::read::GzDecoder;
use jni::{
    JavaVM,
    objects::{JObject, JString, JValue},
    signature::{Primitive, ReturnType},
    sys::{JNI_VERSION_1_6, jint, jobject},
};
use lazy_static::lazy_static;
use manatan_server_public::{
    app::build_router_without_cors, build_state, config::Config as ManatanServerConfig,
};
use reqwest::Client;
use serde::Serialize;
use tar::Archive;
use tokio::{fs as tokio_fs, net::TcpListener};
use tracing::{error, info, trace, warn};
use tracing_log::LogTracer;
use tracing_subscriber::{EnvFilter, fmt::MakeWriter};
use winit::platform::android::{EventLoopBuilderExtAndroid, activity::AndroidApp};

lazy_static! {
    static ref LOG_BUFFER: Mutex<VecDeque<String>> = Mutex::new(VecDeque::with_capacity(500));
}

static WEBUI_DIR: OnceLock<PathBuf> = OnceLock::new();

const TACHI_DATA_DIR_NAME: &str = "manga_reader_backend";
const APP_HTTP_PORT: u16 = 14568;

fn get_files_dir_from_context(env: &mut jni::JNIEnv, context: &JObject) -> Option<PathBuf> {
    let dir_obj = env
        .call_method(context, "getFilesDir", "()Ljava/io/File;", &[])
        .ok()?
        .l()
        .ok()?;
    if dir_obj.is_null() {
        return None;
    }
    let path_obj = env
        .call_method(&dir_obj, "getAbsolutePath", "()Ljava/lang/String;", &[])
        .ok()?
        .l()
        .ok()?;
    let path_jstr: JString = path_obj.into();
    let path_rust: String = env.get_string(&path_jstr).ok()?.into();
    Some(PathBuf::from(path_rust))
}

fn get_external_files_dir_from_context(
    env: &mut jni::JNIEnv,
    context: &JObject,
) -> Option<PathBuf> {
    let null_obj = JObject::null();
    let dir_obj = env
        .call_method(
            context,
            "getExternalFilesDir",
            "(Ljava/lang/String;)Ljava/io/File;",
            &[JValue::Object(&null_obj)],
        )
        .ok()?
        .l()
        .ok()?;
    if dir_obj.is_null() {
        return None;
    }
    let path_obj = env
        .call_method(&dir_obj, "getAbsolutePath", "()Ljava/lang/String;", &[])
        .ok()?
        .l()
        .ok()?;
    let path_jstr: JString = path_obj.into();
    let path_rust: String = env.get_string(&path_jstr).ok()?.into();
    Some(PathBuf::from(path_rust))
}

fn get_external_files_dir(app: &AndroidApp) -> Option<PathBuf> {
    let vm_ptr = app.vm_as_ptr() as *mut jni::sys::JavaVM;
    let vm = unsafe { JavaVM::from_raw(vm_ptr).ok()? };
    let mut env = vm.attach_current_thread().ok()?;
    let activity_ptr = app.activity_as_ptr() as jni::sys::jobject;
    let context = unsafe { JObject::from_raw(activity_ptr) };
    get_external_files_dir_from_context(&mut env, &context)
}

fn get_external_storage_root(app: &AndroidApp) -> Option<PathBuf> {
    let vm_ptr = app.vm_as_ptr() as *mut jni::sys::JavaVM;
    let vm = unsafe { JavaVM::from_raw(vm_ptr).ok()? };
    let mut env = vm.attach_current_thread().ok()?;

    let env_cls = env.find_class("android/os/Environment").ok()?;
    let dir_obj = env
        .call_static_method(
            env_cls,
            "getExternalStorageDirectory",
            "()Ljava/io/File;",
            &[],
        )
        .ok()?
        .l()
        .ok()?;
    if dir_obj.is_null() {
        return None;
    }

    let path_obj = env
        .call_method(&dir_obj, "getAbsolutePath", "()Ljava/lang/String;", &[])
        .ok()?
        .l()
        .ok()?;
    let path_jstr: JString = path_obj.into();
    let path_rust: String = env.get_string(&path_jstr).ok()?.into();
    Some(PathBuf::from(path_rust))
}

fn get_external_storage_root_from_env(env: &mut jni::JNIEnv) -> Option<PathBuf> {
    let env_cls = env.find_class("android/os/Environment").ok()?;
    let dir_obj = env
        .call_static_method(
            env_cls,
            "getExternalStorageDirectory",
            "()Ljava/io/File;",
            &[],
        )
        .ok()?
        .l()
        .ok()?;
    if dir_obj.is_null() {
        return None;
    }

    let path_obj = env
        .call_method(&dir_obj, "getAbsolutePath", "()Ljava/lang/String;", &[])
        .ok()?
        .l()
        .ok()?;
    let path_jstr: JString = path_obj.into();
    let path_rust: String = env.get_string(&path_jstr).ok()?.into();
    Some(PathBuf::from(path_rust))
}

fn ensure_nomedia(dir: &Path) {
    let nomedia = dir.join(".nomedia");
    if nomedia.exists() {
        return;
    }
    if let Err(err) = File::create(&nomedia) {
        warn!("Failed to create .nomedia at {}: {err}", nomedia.display());
    }
}

fn copy_dir_recursive_merge(src: &Path, dst: &Path) -> io::Result<()> {
    if !dst.exists() {
        fs::create_dir_all(dst)?;
    }

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_recursive_merge(&src_path, &dst_path)?;
        } else if !dst_path.exists() {
            // Merge behavior: never overwrite existing files.
            let _ = fs::copy(&src_path, &dst_path);
        }
    }

    Ok(())
}

fn migrate_legacy_shared_root(legacy_root: &Path, new_root: &Path) {
    if !legacy_root.exists() {
        return;
    }

    if !new_root.exists() {
        if let Some(parent) = new_root.parent() {
            if let Err(err) = fs::create_dir_all(parent) {
                warn!(
                    "Failed to create shared storage parent {}: {err}",
                    parent.display()
                );
                return;
            }
        }

        match fs::rename(legacy_root, new_root) {
            Ok(()) => {
                info!(
                    "Migrated shared storage root from {} to {}",
                    legacy_root.display(),
                    new_root.display()
                );
                return;
            }
            Err(err) => {
                warn!(
                    "Failed to rename shared storage root ({} -> {}): {err}. Falling back to copy.",
                    legacy_root.display(),
                    new_root.display()
                );
                if let Err(copy_err) = copy_dir_recursive(legacy_root, new_root) {
                    warn!(
                        "Failed to copy shared storage root ({} -> {}): {copy_err}",
                        legacy_root.display(),
                        new_root.display()
                    );
                } else {
                    info!(
                        "Copied shared storage root from {} to {} (legacy preserved)",
                        legacy_root.display(),
                        new_root.display()
                    );
                }
                return;
            }
        }
    }

    // Both exist: try to merge legacy content without overwriting.
    if let Err(err) = copy_dir_recursive_merge(legacy_root, new_root) {
        warn!(
            "Failed to merge legacy shared storage root ({} -> {}): {err}",
            legacy_root.display(),
            new_root.display()
        );
    } else {
        info!(
            "Merged legacy shared storage root into {} (legacy preserved)",
            new_root.display()
        );
    }
}

fn resolve_manatan_shared_root_with_migration(app: &AndroidApp) -> Option<PathBuf> {
    // Never inspect or migrate stock Manatan's shared folders. The fork uses
    // its package-scoped external directory until a user grants a SAF tree.
    let new_root = get_external_files_dir(app)?.join("manga-reader-fork");
    if let Err(err) = fs::create_dir_all(&new_root) {
        warn!(
            "Failed to create package-scoped Manga Reader root {}: {err}",
            new_root.display()
        );
    }

    Some(new_root)
}

fn migrate_internal_dir_to_external(internal: &Path, external: &Path) {
    if !internal.exists() {
        return;
    }

    if let Some(parent) = external.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            warn!(
                "Failed to create external dir parent {}: {err}",
                parent.display()
            );
            return;
        }
    }

    if external.exists() {
        // Target exists already; merge without overwriting.
        if let Err(err) = copy_dir_recursive_merge(internal, external) {
            warn!(
                "Failed to merge directory ({} -> {}): {err}",
                internal.display(),
                external.display()
            );
        }
        return;
    }

    match fs::rename(internal, external) {
        Ok(()) => {
            info!(
                "Migrated directory from {} to {}",
                internal.display(),
                external.display()
            );
        }
        Err(err) => {
            warn!(
                "Failed to move directory ({} -> {}): {err}. Falling back to copy.",
                internal.display(),
                external.display()
            );
            match copy_dir_recursive(internal, external) {
                Ok(()) => info!(
                    "Copied directory from {} to {} (internal preserved)",
                    internal.display(),
                    external.display()
                ),
                Err(copy_err) => warn!(
                    "Failed to copy directory ({} -> {}): {copy_err}",
                    internal.display(),
                    external.display()
                ),
            }
        }
    }
}

fn prepare_shared_local_media_dirs(
    app: &AndroidApp,
    legacy_bases: &[PathBuf],
    fallback_base: &Path,
) -> (PathBuf, PathBuf) {
    // Prefer shared external storage so users can manage files directly.
    let shared_root = resolve_manatan_shared_root_with_migration(app);

    let (local_manga_dir, compatibility_dir) = if let Some(shared_root) = shared_root {
        (
            shared_root.join("local-manga"),
            shared_root.join(".backend-compat-unused"),
        )
    } else {
        // Fallback: app data storage.
        (
            fallback_base.join("local-manga"),
            fallback_base.join(".backend-compat-unused"),
        )
    };

    // Migrate any existing legacy directories (internal/app-external) to the shared defaults.
    for base in legacy_bases {
        migrate_internal_dir_to_external(&base.join("local-manga"), &local_manga_dir);
    }

    // Ensure directories exist.
    if let Err(err) = fs::create_dir_all(&local_manga_dir) {
        warn!(
            "Failed to create local-manga directory {}: {err}",
            local_manga_dir.display()
        );
    }
    ensure_nomedia(&local_manga_dir);
    let _ = fs::create_dir_all(&compatibility_dir);
    ensure_nomedia(&compatibility_dir);

    (local_manga_dir, compatibility_dir)
}

fn should_skip_app_data_entry(name: &str) -> bool {
    matches!(
        name,
        // These are migrated/configured elsewhere.
        "local-manga" | "local-anime" | "local-novel" | "local-sources" |
        // This is migrated separately so it lives in shared root.
        TACHI_DATA_DIR_NAME
    )
}

fn migrate_app_data_base_to_shared(src_base: &Path, dst_base: &Path) {
    if src_base == dst_base {
        return;
    }

    if !src_base.exists() {
        return;
    }

    if let Err(err) = fs::create_dir_all(dst_base) {
        warn!(
            "Failed to create shared app data dir {}: {err}",
            dst_base.display()
        );
        return;
    }

    let entries = match fs::read_dir(src_base) {
        Ok(e) => e,
        Err(err) => {
            warn!("Failed to read app data dir {}: {err}", src_base.display());
            return;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if should_skip_app_data_entry(&name_str) {
            continue;
        }

        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };

        let src = entry.path();
        let dst = dst_base.join(&name);

        if dst.exists() {
            if file_type.is_dir() {
                let _ = copy_dir_recursive_merge(&src, &dst);
            }
            continue;
        }

        match fs::rename(&src, &dst) {
            Ok(()) => {
                // Moved.
            }
            Err(_) => {
                if file_type.is_dir() {
                    if copy_dir_recursive(&src, &dst).is_ok() {
                        let _ = fs::remove_dir_all(&src);
                    }
                } else {
                    if fs::copy(&src, &dst).is_ok() {
                        let _ = fs::remove_file(&src);
                    }
                }
            }
        }
    }
}

fn resolve_shared_app_data_dir_with_migration(
    _app: &AndroidApp,
    internal_files_dir: &Path,
) -> PathBuf {
    // Databases, settings, categories, progress, stats, and OCR metadata are
    // private app data. Explicit export/restore is the only sharing boundary.
    internal_files_dir.to_path_buf()
}

fn resolve_tachidesk_data_dir_from_paths(
    internal_base: &Path,
    external_base: Option<PathBuf>,
    shared_base: Option<PathBuf>,
) -> PathBuf {
    let internal_current = internal_base.join(TACHI_DATA_DIR_NAME);

    if let Some(shared_base) = shared_base {
        let shared_dir = shared_base.join(TACHI_DATA_DIR_NAME);
        if shared_dir.exists() {
            return shared_dir;
        }
    }

    if let Some(external_base) = external_base {
        let external_dir = external_base.join(TACHI_DATA_DIR_NAME);
        if external_dir.exists() {
            return external_dir;
        }
        if internal_current.exists() {
            return internal_current;
        }
        return external_dir;
    }

    if internal_current.exists() {
        internal_current
    } else {
        internal_current
    }
}

fn resolve_tachidesk_data_dir_with_migration(app: &AndroidApp, fallback_base: &Path) -> PathBuf {
    let internal_base = app
        .internal_data_path()
        .unwrap_or_else(|| fallback_base.to_path_buf());
    let internal_dir = internal_base.join(TACHI_DATA_DIR_NAME);

    let _ = fs::create_dir_all(&internal_dir);
    internal_dir
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    if !dst.exists() {
        fs::create_dir_all(dst)?;
    }

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(())
}

struct GuiWriter;
impl io::Write for GuiWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let log_line = String::from_utf8_lossy(buf).to_string();
        print!("{}", log_line);
        if let Ok(mut logs) = LOG_BUFFER.lock() {
            if logs.len() >= 500 {
                logs.pop_front();
            }
            logs.push_back(log_line);
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct GuiMakeWriter;
impl<'a> MakeWriter<'a> for GuiMakeWriter {
    type Writer = GuiWriter;
    fn make_writer(&'a self) -> Self::Writer {
        GuiWriter
    }
}

fn start_foreground_service(app: &AndroidApp) {
    use jni::objects::{JObject, JValue};

    info!("Attempting to start Foreground Service...");

    let vm_ptr = app.vm_as_ptr() as *mut jni::sys::JavaVM;
    let vm = unsafe { JavaVM::from_raw(vm_ptr).unwrap() };
    let mut env = vm.attach_current_thread().unwrap();

    let activity_ptr = app.activity_as_ptr() as jni::sys::jobject;
    let context = unsafe { JObject::from_raw(activity_ptr) };

    let intent_class = env
        .find_class("android/content/Intent")
        .expect("Failed to find Intent class");
    let intent = env
        .new_object(&intent_class, "()V", &[])
        .expect("Failed to create Intent");

    let context_class = env
        .find_class("android/content/Context")
        .expect("Failed to find Context class");
    let service_class_name = env
        .new_string("dev.local.mangareader.MangatanService")
        .expect("Failed to create string");

    let pkg_name = get_package_name(&mut env, &context).unwrap_or("dev.local.mangareader".to_string());
    let pkg_name_jstr = env.new_string(&pkg_name).unwrap();

    let _ = env
        .call_method(
            &intent,
            "setClassName",
            "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
            &[
                JValue::Object(&pkg_name_jstr),
                JValue::Object(&service_class_name),
            ],
        )
        .expect("Failed to set class name on Intent");

    let sdk_int = get_android_sdk_version(app);
    if sdk_int >= 26 {
        info!("Calling startForegroundService (SDK >= 26)");
        let _ = env.call_method(
            &context,
            "startForegroundService",
            "(Landroid/content/Intent;)Landroid/content/ComponentName;",
            &[JValue::Object(&intent)],
        );
    } else {
        info!("Calling startService (SDK < 26)");
        let _ = env.call_method(
            &context,
            "startService",
            "(Landroid/content/Intent;)Landroid/content/ComponentName;",
            &[JValue::Object(&intent)],
        );
    }

    info!("Foreground Service start request sent.");
}

fn init_tracing() {
    LogTracer::init().expect("Failed to set logger");
    let filter =
        EnvFilter::new("info,manatan_android=trace,wgpu_core=off,wgpu_hal=off,naga=off,jni=info");
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(GuiMakeWriter)
        .with_ansi(false)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("Failed to set tracing subscriber");
}

fn redirect_stdout_to_gui() {
    let mut pipes = [0; 2];
    if unsafe { libc::pipe(pipes.as_mut_ptr()) } < 0 {
        return;
    }
    let [read_fd, write_fd] = pipes;
    unsafe {
        libc::dup2(write_fd, libc::STDOUT_FILENO);
        libc::dup2(write_fd, libc::STDERR_FILENO);
    }
    thread::spawn(move || {
        let file = unsafe { File::from_raw_fd(read_fd) };
        let reader = BufReader::new(file);
        use std::io::BufRead;
        for line in reader.lines() {
            if let Ok(l) = line {
                if let Ok(mut logs) = LOG_BUFFER.lock() {
                    if logs.len() >= 500 {
                        logs.pop_front();
                    }
                    logs.push_back(l);
                }
            }
        }
    });
}

type JniCreateJavaVM = unsafe extern "system" fn(
    pvm: *mut *mut jni::sys::JavaVM,
    penv: *mut *mut c_void,
    args: *mut c_void,
) -> jint;

struct ManatanApp {
    server_ready: Arc<AtomicBool>,
    #[cfg(feature = "native_webview")]
    webview_launcher: Box<dyn Fn() + Send + Sync>,
    #[cfg(feature = "native_webview")]
    webview_last_launch: Option<Instant>,
}

impl ManatanApp {
    fn new(
        _cc: &eframe::CreationContext<'_>,
        server_ready: Arc<AtomicBool>,
        #[cfg(feature = "native_webview")] webview_launcher: Box<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self {
            server_ready,
            #[cfg(feature = "native_webview")]
            webview_launcher,
            #[cfg(feature = "native_webview")]
            webview_last_launch: None,
        }
    }
}

impl eframe::App for ManatanApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let is_ready = self.server_ready.load(Ordering::Relaxed);
        if !is_ready {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        // --- NATIVE WEBVIEW MODE ---
        #[cfg(feature = "native_webview")]
        {
            let should_launch = self
                .webview_last_launch
                .is_none_or(|last_launch| last_launch.elapsed() >= Duration::from_secs(2));
            if is_ready && should_launch {
                info!("Server ready, auto-launching WebView...");
                (self.webview_launcher)();
                self.webview_last_launch = Some(Instant::now());
            }

            // update() is suspended while WebviewActivity is in front. If Android
            // later exposes this host again, retry after the short guard interval
            // instead of leaving the user on a permanent black surface.
            ctx.request_repaint_after(Duration::from_millis(250));

            // The native activity only owns the embedded services. Keep its surface
            // blank while the WebView starts and if the WebView task is dismissed;
            // runtime logs remain in logcat instead of becoming product UI.
            egui::CentralPanel::default()
                .frame(egui::Frame::default().fill(egui::Color32::BLACK))
                .show(ctx, |_ui| {});
            return; // Skip drawing the standard debug GUI
        }

        // --- DEBUG GUI (Only runs if feature is DISABLED) ---
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(20.0);
                ui.heading(egui::RichText::new("Manatan").size(32.0).strong());
                ui.add_space(20.0);

                if is_ready {
                    ui.heading(
                        egui::RichText::new("Server Started")
                            .color(egui::Color32::GREEN)
                            .strong(),
                    );
                } else {
                    ui.heading(
                        egui::RichText::new("Server is Starting...").color(egui::Color32::RED),
                    );
                    ctx.request_repaint_after(Duration::from_millis(500));
                }
                ui.add_space(20.0);

                if ui
                    .add(egui::Button::new("Open WebUI").min_size(egui::vec2(200.0, 50.0)))
                    .clicked()
                {
                    ctx.open_url(egui::OpenUrl::new_tab("http://127.0.0.1:14568"));
                    info!("User clicked Open WebUI");
                }

                ui.add_space(10.0);
                if ui
                    .add(egui::Button::new("Join our Discord").min_size(egui::vec2(200.0, 50.0)))
                    .clicked()
                {
                    ctx.open_url(egui::OpenUrl::new_tab("https://discord.gg/tDAtpPN8KK"));
                    info!("User clicked Discord");
                }
            });

            ui.add_space(20.0);
            ui.separator();
            ui.heading("Logs");

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.style_mut().override_text_style = Some(egui::TextStyle::Monospace);
                    ui.style_mut()
                        .text_styles
                        .get_mut(&egui::TextStyle::Monospace)
                        .unwrap()
                        .size = 10.0;
                    if let Ok(logs) = LOG_BUFFER.lock() {
                        for line in logs.iter() {
                            ui.label(line);
                        }
                    }
                });
        });
    }
}

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    init_tracing();
    redirect_stdout_to_gui();

    info!("Starting Manatan...");

    // SAF, Anki, and notification permissions are requested only from the
    // corresponding user action. The foreground reader does not need broad
    // storage access, permanent wake/Wi-Fi locks, or a startup service.

    let internal_files_dir = app.internal_data_path().expect("Failed to get data path");
    let files_dir = resolve_shared_app_data_dir_with_migration(&app, &internal_files_dir);

    let external_app_dir =
        get_external_files_dir(&app).unwrap_or_else(|| internal_files_dir.clone());
    let legacy_bases = vec![
        internal_files_dir.clone(),
        external_app_dir,
        files_dir.clone(),
    ];
    let (default_local_manga_dir, default_local_anime_dir) =
        prepare_shared_local_media_dirs(&app, &legacy_bases, &files_dir);

    let app_bg = app.clone();
    let files_dir_clone = files_dir.clone();

    let app_clone_2 = app.clone();
    let files_dir_clone_2 = files_dir.clone();

    let server_ready = Arc::new(AtomicBool::new(false));
    let server_ready_bg = server_ready.clone();
    let server_ready_gui = server_ready.clone();

    thread::spawn(move || {
        start_background_services(app_bg, files_dir);
    });

    let default_local_manga_dir_clone = default_local_manga_dir.clone();
    let default_local_anime_dir_clone = default_local_anime_dir.clone();
    thread::spawn(move || {
        info!("Starting Web Server Runtime...");
        let rt = tokio::runtime::Runtime::new().expect("Failed to build Tokio runtime");

        rt.spawn(async move {
            let client = Client::new();

            loop {
                let request = client.get("http://127.0.0.1:14568/health");

                match request.send().await {
                    Ok(resp) if resp.status().is_success() => {
                        if !server_ready_bg.load(Ordering::Relaxed) {
                            server_ready_bg.store(true, Ordering::Relaxed);
                            let app_clone_3 = app_clone_2.clone();
                            let files_dir_clone_3 = files_dir_clone_2.clone();
                            tokio::task::spawn_blocking(move || {
                                update_server_conf_local_source(&app_clone_3, &files_dir_clone_3);
                            });
                        }
                    }
                    _ => {
                        if server_ready_bg.load(Ordering::Relaxed) {
                            server_ready_bg.store(false, Ordering::Relaxed);
                        }
                    }
                }

                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });

        let internal_runtime_dir = internal_files_dir.clone();
        rt.block_on(async move {
            if let Err(e) = start_web_server(
                files_dir_clone,
                internal_runtime_dir,
                default_local_manga_dir_clone,
                default_local_anime_dir_clone,
            )
            .await
            {
                error!("Web Server Crashed: {:?}", e);
            }
        });
    });

    let sdk_version = get_android_sdk_version(&app);
    info!("Detected Android SDK Version: {}", sdk_version);

    let app_gui = app.clone();
    let mut options = eframe::NativeOptions::default();

    if sdk_version <= 29 {
        info!("SDK <= 29: Forcing OpenGL (GLES) backend for maximum compatibility.");
        options.wgpu_options.supported_backends = eframe::wgpu::Backends::GL;
    } else {
        info!("SDK > 29: Programmatically detecting best graphics backend...");
        if supports_vulkan(&app) {
            info!("Vulkan supported. Using primary backend (Vulkan preferred).");
            options.wgpu_options.supported_backends = eframe::wgpu::Backends::PRIMARY;
        } else {
            info!("Vulkan not supported or check failed. Forcing OpenGL (GLES) backend.");
            options.wgpu_options.supported_backends = eframe::wgpu::Backends::GL;
        }
    }

    options.event_loop_builder = Some(Box::new(move |builder| {
        builder.with_android_app(app_gui);
    }));

    let app_for_launcher = app.clone();

    eframe::run_native(
        "Manatan",
        options,
        Box::new(move |cc| {
            // Setup the launcher closure
            #[cfg(feature = "native_webview")]
            let launcher = Box::new(move || {
                launch_webview_activity(&app_for_launcher);
            });

            Ok(Box::new(ManatanApp::new(
                cc,
                server_ready_gui,
                #[cfg(feature = "native_webview")]
                launcher,
            )))
        }),
    )
    .unwrap_or_else(|e| {
        error!("GUI Failed to start: {:?}", e);
    });
}

fn launch_webview_activity(app: &AndroidApp) {
    use jni::objects::{JObject, JValue};

    info!("🚀 Launching Native Webview Activity...");

    let vm_ptr = app.vm_as_ptr() as *mut jni::sys::JavaVM;
    let vm = unsafe { JavaVM::from_raw(vm_ptr).unwrap() };
    let mut env = vm.attach_current_thread().unwrap();

    let activity_ptr = app.activity_as_ptr() as jni::sys::jobject;
    let context = unsafe { JObject::from_raw(activity_ptr) };

    // Find the class we just created
    let intent_class = env
        .find_class("android/content/Intent")
        .expect("Failed to find Intent class");
    let intent = env
        .new_object(&intent_class, "()V", &[])
        .expect("Failed to create Intent");

    // Helper to get package name
    let pkg_name = get_package_name(&mut env, &context).unwrap_or("dev.local.mangareader".to_string());
    let pkg_name_jstr = env.new_string(&pkg_name).unwrap();

    // Target the new Activity
    let activity_class_name = env.new_string("dev.local.mangareader.WebviewActivity").unwrap();

    let _ = env
        .call_method(
            &intent,
            "setClassName",
            "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
            &[
                JValue::Object(&pkg_name_jstr),
                JValue::Object(&activity_class_name),
            ],
        )
        .expect("Failed to set class name");

    let _ = env
        .call_method(
            &context,
            "startActivity",
            "(Landroid/content/Intent;)V",
            &[JValue::Object(&intent)],
        )
        .expect("Failed to start Webview Activity");
}

async fn start_web_server(
    data_dir: PathBuf,
    internal_runtime_dir: PathBuf,
    default_local_manga_dir: PathBuf,
    default_local_anime_dir: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("🚀 Initializing Manatan Server on port 4568...");

    // Pick a WebUI directory that actually contains an index.html.
    // We also support an extra nested folder level (some tar layouts unpack into a subdir).
    let candidates = [
        data_dir.join("webui"),
        data_dir.join("webui").join("webui"),
        internal_runtime_dir.join("webui"),
        internal_runtime_dir.join("webui").join("webui"),
    ];

    // Wait briefly for background extraction to finish.
    let mut selected = None;
    for _ in 0..50 {
        for dir in candidates.iter() {
            if dir.join("index.html").exists() {
                selected = Some(dir.clone());
                break;
            }
        }
        if selected.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let webui_dir = selected.unwrap_or_else(|| candidates[0].clone());
    info!("WebUI directory set to {}", webui_dir.display());
    let _ = WEBUI_DIR.set(webui_dir);

    let manatan_db_path = std::env::var("MANATAN_DB_PATH").unwrap_or_else(|_| {
        data_dir
            .join("manatan.sqlite")
            .to_string_lossy()
            .to_string()
    });
    let manatan_migrate_path = std::env::var("MANATAN_MIGRATE_PATH").ok();
    let manatan_runtime_url =
        std::env::var("MANATAN_JAVA_URL").unwrap_or_else(|_| "http://127.0.0.1:4566".to_string());
    // The manga-only fork does not perform account/tracker or extension-index
    // network work. Required user-owned sources are configured via /api/fork.
    let tracker_remote_search = false;
    let tracker_search_ttl_seconds = std::env::var("MANATAN_TRACKER_SEARCH_TTL_SECONDS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(3600);
    let downloads_path = std::env::var("MANATAN_DOWNLOADS_PATH")
        .unwrap_or_else(|_| data_dir.join("downloads").to_string_lossy().to_string());
    let aidoku_index_url = String::new();
    let aidoku_enabled = false;
    let aidoku_cache_path = std::env::var("MANATAN_AIDOKU_CACHE")
        .unwrap_or_else(|_| data_dir.join("aidoku").to_string_lossy().to_string());
    let local_manga_path = std::env::var("MANATAN_LOCAL_MANGA_PATH")
        .unwrap_or_else(|_| default_local_manga_dir.to_string_lossy().to_string());
    let local_anime_path = std::env::var("MANATAN_LOCAL_ANIME_PATH")
        .unwrap_or_else(|_| default_local_anime_dir.to_string_lossy().to_string());
    let manatan_config = ManatanServerConfig {
        host: "127.0.0.1".to_string(),
        port: APP_HTTP_PORT,
        java_runtime_url: manatan_runtime_url,
        webview_enabled: false,
        aidoku_index_url,
        aidoku_enabled,
        aidoku_cache_path,
        db_path: manatan_db_path,
        migrate_path: manatan_migrate_path,
        tracker_remote_search,
        tracker_search_ttl_seconds,
        downloads_path,
        local_manga_path,
        local_anime_path,
    };

    let manatan_state = build_state(manatan_config).await?;
    let manatan_router = build_router_without_cors(manatan_state);
    let fork_router = fork_api::router(fork_api::ForkApiState::load(&data_dir));

    let ocr_router = manatan_ocr_server::create_router(data_dir.clone());
    let yomitan_router = manatan_yomitan_server::create_router(data_dir.clone());
    let audio_router = manatan_audio_server::create_router(data_dir.clone());

    let app = Router::new()
        .route("/api/system/version", get(current_version_handler))
        .nest("/api/fork", fork_router)
        .nest_service("/api/ocr", ocr_router)
        .nest_service("/api/yomitan", yomitan_router)
        .nest_service("/api/audio", audio_router)
        .merge(manatan_router)
        .fallback(serve_react_app);

    let listener = TcpListener::bind(("127.0.0.1", APP_HTTP_PORT)).await?;
    info!("Web server listening on loopback:{APP_HTTP_PORT}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn serve_react_app(uri: Uri) -> impl IntoResponse {
    let Some(webui_dir) = WEBUI_DIR.get() else {
        return (StatusCode::NOT_FOUND, "404 - WebUI assets not configured").into_response();
    };
    let path_str = uri.path().trim_start_matches('/');
    // `Path::join` does not normalise `..`, so reject traversal explicitly.
    let traversal = path_str.split('/').any(|part| part == "..");

    if !path_str.is_empty() && !traversal {
        let file_path = webui_dir.join(path_str);

        if file_path.starts_with(webui_dir) && file_path.is_file() {
            if let Ok(content) = tokio_fs::read(&file_path).await {
                let mime = mime_guess::from_path(&file_path).first_or_octet_stream();
                // Vite emits content-hashed file names under assets/, so they
                // can be cached for good; everything else must revalidate so an
                // app update is picked up immediately.
                let cache_control = if path_str.starts_with("assets/") {
                    "public, max-age=31536000, immutable"
                } else {
                    "no-cache, no-store, must-revalidate"
                };
                return (
                    [
                        (axum::http::header::CONTENT_TYPE, mime.as_ref()),
                        (axum::http::header::CACHE_CONTROL, cache_control),
                    ],
                    content,
                )
                    .into_response();
            }
        }
    }

    let index_path = webui_dir.join("index.html");
    if let Ok(html_string) = tokio_fs::read_to_string(index_path).await {
        let fixed_html = html_string.replace("<head>", "<head><base href=\"/\" />");
        return (
            [
                (axum::http::header::CONTENT_TYPE, "text/html"),
                (
                    axum::http::header::CACHE_CONTROL,
                    "no-cache, no-store, must-revalidate",
                ),
            ],
            fixed_html,
        )
            .into_response();
    }

    let webui_dir_display = webui_dir.display().to_string();
    (
        StatusCode::NOT_FOUND,
        format!(
            "404 - WebUI assets not found. Expected index.html under: {}",
            webui_dir_display
        ),
    )
        .into_response()
}

fn start_background_services(app: AndroidApp, files_dir: PathBuf) {
    // Certain runtime artifacts (notably native libs) must stay in internal storage.
    let internal_runtime_dir = app
        .internal_data_path()
        .unwrap_or_else(|| files_dir.clone());

    let apk_time = get_apk_update_time(&app).unwrap_or(i64::MAX);
    let marker = internal_runtime_dir.join(".extracted_apk_time");

    let last_time: i64 = fs::read_to_string(&marker)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let jre_root = internal_runtime_dir.join("jre");

    // Prefer shared-root app-data for the WebUI, but fall back to internal if extraction fails.
    let webui_primary = files_dir.join("webui");
    let webui_fallback = internal_runtime_dir.join("webui");

    if apk_time > last_time {
        info!("Extracting assets (APK updated)...");

        if jre_root.exists() {
            fs::remove_dir_all(&jre_root).ok();
        }
        if webui_primary.exists() {
            fs::remove_dir_all(&webui_primary).ok();
        }
        if webui_fallback.exists() {
            fs::remove_dir_all(&webui_fallback).ok();
        }

        // Extract WebUI first so the Rust server can serve it even if the JRE fails.
        let mut webui_extracted = false;
        if fs::create_dir_all(&webui_primary).is_ok() {
            if install_webui(&app, &webui_primary).is_ok() {
                webui_extracted = true;
            }
        }
        if !webui_extracted {
            warn!(
                "WebUI extraction to {} failed; falling back to internal {}",
                webui_primary.display(),
                webui_fallback.display()
            );
            fs::create_dir_all(&webui_fallback).ok();
            if let Err(e) = install_webui(&app, &webui_fallback) {
                error!("WebUI extraction failed (fallback): {:?}", e);
            } else {
                webui_extracted = true;
            }
        }

        if let Err(e) = install_jre(&app, &internal_runtime_dir) {
            error!("JRE extraction failed: {:?}", e);
            // Continue; WebUI may still be usable.
        }

        fs::write(&marker, apk_time.to_string()).ok();
        info!("Extraction complete");
    } else {
        info!("Assets up-to-date, skipping extraction");

        let has_webui = webui_primary.join("index.html").exists()
            || webui_primary.join("webui").join("index.html").exists()
            || webui_fallback.join("index.html").exists()
            || webui_fallback.join("webui").join("index.html").exists();

        if !has_webui {
            info!("WebUI assets missing; re-extracting...");
            if webui_primary.exists() {
                fs::remove_dir_all(&webui_primary).ok();
            }
            if webui_fallback.exists() {
                fs::remove_dir_all(&webui_fallback).ok();
            }

            let mut webui_extracted = false;
            if fs::create_dir_all(&webui_primary).is_ok() {
                if install_webui(&app, &webui_primary).is_ok() {
                    webui_extracted = true;
                }
            }
            if !webui_extracted {
                warn!(
                    "WebUI extraction to {} failed; falling back to internal {}",
                    webui_primary.display(),
                    webui_fallback.display()
                );
                fs::create_dir_all(&webui_fallback).ok();
                if let Err(e) = install_webui(&app, &webui_fallback) {
                    error!("WebUI extraction failed (fallback): {:?}", e);
                }
            }
        }
    }

    // Create 'bin' directory to satisfy Suwayomi's directory scanner
    let bin_dir = internal_runtime_dir.join("bin");
    if bin_dir.exists() {
        fs::remove_dir_all(&bin_dir).ok();
    }
    fs::create_dir_all(&bin_dir).expect("Failed to create bin directory");
    let jar_path = bin_dir.join("Suwayomi-Server.jar");

    let tachidesk_data = resolve_tachidesk_data_dir_with_migration(&app, &files_dir);
    let tmp_dir = internal_runtime_dir.join("tmp");

    if !tachidesk_data.exists() {
        let _ = fs::create_dir_all(&tachidesk_data);
    }

    let tachi_webui_dir = tachidesk_data.join("webUI");
    if let Err(e) = fs::create_dir_all(&tachi_webui_dir) {
        error!("Failed to create tachidesk/webUI dir: {:?}", e);
    } else {
        let revision_file = tachi_webui_dir.join("revision");
        if let Err(e) = fs::write(&revision_file, "r2643") {
            error!("Failed to write revision file: {:?}", e);
        } else {
            info!("✅ Created revision file: r2643");
        }
    }

    if tmp_dir.exists() {
        let _ = fs::remove_dir_all(&tmp_dir);
    }
    if let Err(e) = fs::create_dir_all(&tmp_dir) {
        error!("Failed to create temp dir: {:?}", e);
        return;
    }
    let _ = copy_single_asset(&app, "Suwayomi-Server.jar", &jar_path);

    let lib_jli_path = find_file_in_dir(&jre_root, "libjli.so");
    if lib_jli_path.is_none() {
        error!("libjli.so missing");
        return;
    }
    let lib_jli_path = lib_jli_path.unwrap();

    let lib_jvm_path = find_file_in_dir(&jre_root, "libjvm.so");
    if lib_jvm_path.is_none() {
        error!("libjvm.so missing");
        return;
    }
    let lib_jvm_path = lib_jvm_path.unwrap();

    unsafe {
        info!("Loading JRE libraries...");

        let _lib_jli = libloading::os::unix::Library::open(
            Some(&lib_jli_path),
            libloading::os::unix::RTLD_NOW | libloading::os::unix::RTLD_GLOBAL,
        )
        .expect("Failed to load libjli.so");

        let lib_jvm = libloading::os::unix::Library::open(
            Some(&lib_jvm_path),
            libloading::os::unix::RTLD_NOW | libloading::os::unix::RTLD_GLOBAL,
        )
        .expect("Failed to load libjvm.so");

        // Preload libs
        let lib_base_dir = lib_jli_path.parent().unwrap();

        let libs_to_preload = [
            "libverify.so",
            "libjava.so",
            "libnet.so",
            "libnio.so",
            "libawt.so",
            "libawt_headless.so",
            "libjawt.so",
        ];

        for name in libs_to_preload {
            let p = lib_base_dir.join(name);
            if p.exists() {
                trace!("Preloading library: {}", name);
                if let Ok(_l) = libloading::os::unix::Library::open(
                    Some(&p),
                    libloading::os::unix::RTLD_NOW | libloading::os::unix::RTLD_GLOBAL,
                ) {
                    trace!("Loaded {}", name);
                }
            } else {
                trace!("Library not found, skipping preload: {}", name);
            }
        }

        let jar_path_abs = jar_path.canonicalize().unwrap_or(jar_path.clone());
        trace!("Classpath: {:?}", jar_path_abs);
        let mut options_vec = Vec::new();

        options_vec.push(format!("-Djava.class.path={}", jar_path_abs.display()));
        options_vec.push(format!("-Djava.home={}", jre_root.display()));
        options_vec.push(format!("-Djava.library.path={}", lib_base_dir.display()));
        options_vec.push(format!("-Djava.io.tmpdir={}", tmp_dir.display()));

        options_vec.push("-Djava.net.preferIPv4Stack=true".to_string());
        options_vec.push("-Djava.net.preferIPv6Addresses=false".to_string());
        options_vec.push("-Dos.name=Linux".to_string());
        options_vec.push("-Djava.vm.name=OpenJDK".to_string());
        options_vec.push("-Xmx512m".to_string());
        options_vec.push("-Xms256m".to_string());
        options_vec.push("-XX:TieredStopAtLevel=1".to_string());
        options_vec.push("-Dsuwayomi.tachidesk.config.server.webUIEnabled=false".to_string());
        options_vec.push("-Dsuwayomi.tachidesk.config.server.kcefEnable=false".to_string());
        options_vec.push("-Dsuwayomi.tachidesk.config.server.enableCookieApi=true".to_string());
        options_vec.push(
            "-Dsuwayomi.tachidesk.config.server.initialOpenInBrowserEnabled=false".to_string(),
        );
        options_vec.push("-Dsuwayomi.tachidesk.config.server.systemTrayEnabled=false".to_string());
        options_vec.push(
            "-Dsuwayomi.tachidesk.config.server.rootDir={}"
                .to_string()
                .replace("{}", &tachidesk_data.to_string_lossy()),
        );

        let config_marker = files_dir.join(".config_local_source_v1");
        let server_conf_exists = tachidesk_data.join("server.conf").exists();

        if !config_marker.exists() {
            info!(
                "Configuration check: Existing User = {}",
                server_conf_exists
            );

            if let Some(shared_root) = resolve_manatan_shared_root_with_migration(&app) {
                let local_sources_dir = shared_root.join("local-sources");
                let backend_compat_dir = shared_root.join(".backend-compat-unused");
                let local_manga_dir = shared_root.join("local-manga");

                info!(
                    "Ensuring local source directory exists: {}",
                    local_sources_dir.display()
                );
                let _ = std::fs::create_dir_all(&local_sources_dir);
                info!(
                    "Ensuring private-backend compatibility directory exists: {}",
                    backend_compat_dir.display()
                );
                let _ = std::fs::create_dir_all(&backend_compat_dir);
                info!(
                    "Ensuring local manga directory exists: {}",
                    local_manga_dir.display()
                );
                let _ = std::fs::create_dir_all(&local_manga_dir);

                ensure_nomedia(&local_sources_dir);
                ensure_nomedia(&backend_compat_dir);
                ensure_nomedia(&local_manga_dir);

                if !server_conf_exists {
                    info!("Fresh install detected: Setting localSourcePath flag.");
                    options_vec.push(format!(
                        "-Dsuwayomi.tachidesk.config.server.localSourcePath={}",
                        local_sources_dir.display()
                    ));
                    options_vec.push(format!(
                        "-Dsuwayomi.tachidesk.config.server.localAnimeSourcePath={}",
                        backend_compat_dir.display()
                    ));

                    // --- IMPORTANT: Create pending marker HERE ---
                    // This ensures we only patch server.conf if this specific code block runs.
                    let pending_marker = files_dir.join(".pending_local_source_config");
                    let _ = File::create(&pending_marker);
                } else {
                    info!("Legacy update detected: NOT setting localSourcePath flag.");
                }

                if let Err(e) = std::fs::write(&config_marker, "configured") {
                    error!("Failed to write config marker: {:?}", e);
                }
            } else {
                warn!("Shared storage root unavailable; skipping localSourcePath configuration.");
                let _ = std::fs::write(&config_marker, "configured");
            }
        } else {
            info!("Config marker exists. Skipping localSourcePath configuration.");
        }

        let mut jni_options: Vec<jni::sys::JavaVMOption> = options_vec
            .iter()
            .map(|s| {
                let cstr = CString::new(s.as_str()).unwrap();
                jni::sys::JavaVMOption {
                    optionString: cstr.into_raw(),
                    extraInfo: std::ptr::null_mut(),
                }
            })
            .collect();

        info!("Creating JVM with {} options", jni_options.len());

        let create_vm_fn = lib_jvm
            .get::<JniCreateJavaVM>(b"JNI_CreateJavaVM\0")
            .unwrap();
        let mut vm_args = jni::sys::JavaVMInitArgs {
            version: JNI_VERSION_1_6,
            nOptions: jni_options.len() as i32,
            options: jni_options.as_mut_ptr(),
            ignoreUnrecognized: 1,
        };

        info!("Calling JNI_CreateJavaVM...");
        let mut jvm: *mut jni::sys::JavaVM = std::ptr::null_mut();
        let mut env: *mut c_void = std::ptr::null_mut();
        let result = create_vm_fn(&mut jvm, &mut env, &mut vm_args as *mut _ as *mut c_void);

        if result != 0 {
            error!("Failed to create Java VM: {}", result);
            return;
        }
        trace!("JVM Created Successfully");

        let jvm_wrapper = JavaVM::from_raw(jvm).unwrap();
        let mut env = jvm_wrapper.attach_current_thread().unwrap();

        info!("Finding Main Class...");
        let jar_file_cls = env.find_class("java/util/jar/JarFile").unwrap();
        let mid_jar_init = env
            .get_method_id(&jar_file_cls, "<init>", "(Ljava/lang/String;)V")
            .unwrap();
        let mid_get_manifest = env
            .get_method_id(&jar_file_cls, "getManifest", "()Ljava/util/jar/Manifest;")
            .unwrap();
        let jar_path_str = env.new_string(jar_path_abs.to_str().unwrap()).unwrap();

        let jar_obj = match env.new_object_unchecked(
            &jar_file_cls,
            mid_jar_init,
            &[JValue::Object(&jar_path_str).as_jni()],
        ) {
            Ok(o) => o,
            Err(e) => {
                error!("Error opening JAR: {:?}", e);
                let _ = env.exception_describe();
                return;
            }
        };

        let manifest_obj = env
            .call_method_unchecked(jar_obj, mid_get_manifest, ReturnType::Object, &[])
            .unwrap()
            .l()
            .unwrap();
        let manifest_cls = env.find_class("java/util/jar/Manifest").unwrap();
        let mid_get_attrs = env
            .get_method_id(
                manifest_cls,
                "getMainAttributes",
                "()Ljava/util/jar/Attributes;",
            )
            .unwrap();
        let attrs_obj = env
            .call_method_unchecked(manifest_obj, mid_get_attrs, ReturnType::Object, &[])
            .unwrap()
            .l()
            .unwrap();
        let attrs_cls = env.find_class("java/util/jar/Attributes").unwrap();
        let mid_get_val = env
            .get_method_id(
                attrs_cls,
                "getValue",
                "(Ljava/lang/String;)Ljava/lang/String;",
            )
            .unwrap();
        let key_str = env.new_string("Main-Class").unwrap();
        let main_class_jstr = env
            .call_method_unchecked(
                attrs_obj,
                mid_get_val,
                ReturnType::Object,
                &[JValue::Object(&key_str).as_jni()],
            )
            .unwrap()
            .l()
            .unwrap();

        let main_class_name: String = env.get_string(&main_class_jstr.into()).unwrap().into();
        // FIX 1: Trim whitespace, just in case the Manifest has hidden spaces
        let main_class_path = main_class_name.trim().replace(".", "/");
        info!("Found Main: '{}'", main_class_path);

        // --- REPLACE THE CRASHING BLOCK WITH THIS ---

        // 1. Try to find the Main Class safely
        let main_class = match env.find_class(&main_class_path) {
            Ok(cls) => cls,
            Err(e) => {
                error!(
                    "❌ CRITICAL: JVM could not load Main Class: {}",
                    main_class_path
                );
                // This prints the actual Java error (e.g. ClassNotFoundException) to the logs
                let _ = env.exception_describe();
                return;
            }
        };

        // 2. Try to find the 'main' method safely
        let main_method_id = match env.get_static_method_id(
            &main_class,
            "main",
            "([Ljava/lang/String;)V",
        ) {
            Ok(mid) => mid,
            Err(e) => {
                error!(
                    "❌ CRITICAL: Found class, but could not find 'static void main(String[] args)'"
                );
                let _ = env.exception_describe();
                return;
            }
        };

        // 3. Create the arguments array safely
        let empty_str_array = match env.new_object_array(0, "java/lang/String", JObject::null()) {
            Ok(arr) => arr,
            Err(e) => {
                error!("❌ CRITICAL: Failed to create args array");
                let _ = env.exception_describe();
                return;
            }
        };

        info!("Invoking Main...");
        if let Err(e) = env.call_static_method_unchecked(
            &main_class,
            main_method_id,
            ReturnType::Primitive(Primitive::Void),
            &[JValue::Object(&empty_str_array).as_jni()],
        ) {
            error!("Crash in Main: {:?}", e);
            let _ = env.exception_describe();
        }
    }
}

fn install_webui(app: &AndroidApp, target_dir: &Path) -> std::io::Result<()> {
    let filename = CString::new("webui.tar").unwrap();

    let asset = app
        .asset_manager()
        .open(&filename)
        .ok_or(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "webui.tar missing in assets",
        ))?;

    let mut archive = Archive::new(BufReader::new(asset));
    // Shared storage can reject chmod/mtime operations; do not preserve metadata.
    archive.set_preserve_permissions(false);
    archive.set_preserve_mtime(false);
    archive.unpack(target_dir)?;
    info!("WebUI extracted successfully to {:?}", target_dir);
    Ok(())
}

fn install_jre(app: &AndroidApp, target_dir: &Path) -> std::io::Result<()> {
    let filename = CString::new("jre.tar.gz").unwrap();

    let asset = app
        .asset_manager()
        .open(&filename)
        .ok_or(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "jre.tar.gz missing",
        ))?;

    let decoder = GzDecoder::new(BufReader::new(asset));
    let mut archive = Archive::new(decoder);
    archive.set_preserve_permissions(false);
    archive.set_preserve_mtime(false);
    archive.unpack(target_dir)?;
    Ok(())
}

fn copy_single_asset(
    app: &AndroidApp,
    asset_name: &str,
    target_path: &Path,
) -> std::io::Result<()> {
    let c_path = CString::new(asset_name).unwrap();
    if let Some(mut asset) = app.asset_manager().open(&c_path) {
        let mut out = File::create(target_path)?;
        std::io::copy(&mut asset, &mut out)?;
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            asset_name,
        ))
    }
}

fn find_file_in_dir(dir: &Path, filename: &str) -> Option<PathBuf> {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = find_file_in_dir(&path, filename) {
                    return Some(found);
                }
            } else if let Some(name) = path.file_name() {
                if name == filename {
                    return Some(path);
                }
            }
        }
    }
    None
}

fn ensure_battery_unrestricted(app: &AndroidApp) {
    use jni::objects::{JObject, JValue};

    let vm_ptr = app.vm_as_ptr() as *mut jni::sys::JavaVM;
    let vm = unsafe { JavaVM::from_raw(vm_ptr).unwrap() };
    let mut env = vm.attach_current_thread().unwrap();

    let activity_ptr = app.activity_as_ptr() as jni::sys::jobject;
    let context = unsafe { JObject::from_raw(activity_ptr) };

    let pkg_name_jstr = env
        .call_method(&context, "getPackageName", "()Ljava/lang/String;", &[])
        .unwrap()
        .l()
        .unwrap();
    let pkg_name_string: String = env.get_string((&pkg_name_jstr).into()).unwrap().into();

    let power_service_str = env.new_string("power").unwrap();
    let power_manager = env
        .call_method(
            &context,
            "getSystemService",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            &[JValue::Object(&power_service_str)],
        )
        .unwrap()
        .l()
        .unwrap();

    let is_ignoring = env
        .call_method(
            &power_manager,
            "isIgnoringBatteryOptimizations",
            "(Ljava/lang/String;)Z",
            &[JValue::Object(&pkg_name_jstr)],
        )
        .unwrap()
        .z()
        .unwrap();

    if is_ignoring {
        info!("Battery optimization is already unrestricted.");
        return;
    }

    info!("Requesting removal of battery optimizations...");

    let action_str = env
        .new_string("android.settings.REQUEST_IGNORE_BATTERY_OPTIMIZATIONS")
        .unwrap();

    let intent_class = env.find_class("android/content/Intent").unwrap();
    let intent = env
        .new_object(
            &intent_class,
            "(Ljava/lang/String;)V",
            &[JValue::Object(&action_str)],
        )
        .unwrap();

    let uri_class = env.find_class("android/net/Uri").unwrap();
    let uri_str = env
        .new_string(format!("package:{}", pkg_name_string))
        .unwrap();
    let uri = env
        .call_static_method(
            &uri_class,
            "parse",
            "(Ljava/lang/String;)Landroid/net/Uri;",
            &[JValue::Object(&uri_str)],
        )
        .unwrap()
        .l()
        .unwrap();

    let _ = env
        .call_method(
            &intent,
            "setData",
            "(Landroid/net/Uri;)Landroid/content/Intent;",
            &[JValue::Object(&uri)],
        )
        .unwrap();

    let _ = env
        .call_method(
            &context,
            "startActivity",
            "(Landroid/content/Intent;)V",
            &[JValue::Object(&intent)],
        )
        .unwrap();

    info!("Battery exemption dialog requested.");
}

fn get_package_name(env: &mut jni::JNIEnv, context: &JObject) -> jni::errors::Result<String> {
    let package_jstr_obj = env
        .call_method(context, "getPackageName", "()Ljava/lang/String;", &[])?
        .l()?;

    let package_jstr: JString = package_jstr_obj.into();

    let rust_string: String = env.get_string(&package_jstr)?.into();

    Ok(rust_string)
}
fn supports_vulkan(app: &AndroidApp) -> bool {
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM).unwrap() };
    let mut env = vm.attach_current_thread().unwrap();
    let context = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };
    let pm = env
        .call_method(
            &context,
            "getPackageManager",
            "()Landroid/content/pm/PackageManager;",
            &[],
        )
        .unwrap()
        .l()
        .unwrap();
    let pm_class = env.find_class("android/content/pm/PackageManager").unwrap();
    let feature_str = env
        .get_static_field(
            &pm_class,
            "FEATURE_VULKAN_HARDWARE_VERSION",
            "Ljava/lang/String;",
        )
        .unwrap()
        .l()
        .unwrap();
    let vulkan_1_1_version_code = 0x401000;
    let supported = env
        .call_method(
            &pm,
            "hasSystemFeature",
            "(Ljava/lang/String;I)Z",
            &[
                JValue::Object(&feature_str),
                JValue::Int(vulkan_1_1_version_code),
            ],
        )
        .unwrap()
        .z()
        .unwrap_or(false);
    info!("Vulkan 1.1+ hardware support detected: {}", supported);
    supported
}
fn get_android_sdk_version(app: &AndroidApp) -> i32 {
    let vm_ptr = app.vm_as_ptr() as *mut jni::sys::JavaVM;
    let vm = unsafe { JavaVM::from_raw(vm_ptr).unwrap() };
    let mut env = vm.attach_current_thread().unwrap();

    let version_cls = env
        .find_class("android/os/Build$VERSION")
        .expect("Failed to find Build$VERSION");
    let sdk_int = env
        .get_static_field(version_cls, "SDK_INT", "I")
        .expect("Failed to get SDK_INT")
        .i()
        .unwrap_or(0);

    sdk_int
}

fn check_and_request_permissions(app: &AndroidApp) {
    // 1. Initialize JNI Environment and Context
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM).unwrap() };
    let mut env = vm.attach_current_thread().unwrap();

    // The AndroidApp context is the activity (jobject) itself.
    let context = unsafe { JObject::from_raw(app.activity_as_ptr() as jobject) };

    // Get the package name using JNI
    let pkg_name = match get_package_name(&mut env, &context) {
        Ok(name) => name,
        Err(e) => {
            info!("Failed to get package name via JNI: {:?}", e);
            "dev.local.mangareader".to_string()
        }
    };
    info!("Using Package Name: {}", pkg_name);

    // 2. Get Android SDK Version
    let version_cls = env.find_class("android/os/Build$VERSION").unwrap();
    let sdk_int = env
        .get_static_field(version_cls, "SDK_INT", "I")
        .unwrap()
        .i()
        .unwrap();

    info!("Detected Android SDK: {}", sdk_int);

    if sdk_int >= 33 {
        let notif_perm = env
            .new_string("android.permission.POST_NOTIFICATIONS")
            .unwrap();

        let check_res = env
            .call_method(
                &context,
                "checkSelfPermission",
                "(Ljava/lang/String;)I",
                &[JValue::Object(&notif_perm)],
            )
            .unwrap()
            .i()
            .unwrap();

        if check_res != 0 {
            // 0 = PERMISSION_GRANTED, -1 = PERMISSION_DENIED
            info!("Requesting Notification Permissions (Android 13+)...");

            let string_cls = env.find_class("java/lang/String").unwrap();
            let perms_array = env
                .new_object_array(1, string_cls, JObject::null())
                .unwrap();

            env.set_object_array_element(&perms_array, 0, notif_perm)
                .unwrap();

            // Request code 102 for notifications
            let _ = env.call_method(
                &context,
                "requestPermissions",
                "([Ljava/lang/String;I)V",
                &[JValue::Object(&perms_array), JValue::Int(102)],
            );
        } else {
            info!("Notification permissions already granted.");
        }
    }

    if sdk_int >= 30 {
        // --- Android 11+ (SDK 30+) Logic: Manage All Files ---
        let env_cls = env.find_class("android/os/Environment").unwrap();
        let is_manager = env
            .call_static_method(env_cls, "isExternalStorageManager", "()Z", &[])
            .unwrap()
            .z()
            .unwrap();

        if !is_manager {
            info!("Requesting Android 11+ All Files Access...");
            let uri_cls = env.find_class("android/net/Uri").unwrap();

            // Construct "package:com.your.package"
            let uri_str = env.new_string(format!("package:{}", pkg_name)).unwrap();

            let uri = env
                .call_static_method(
                    uri_cls,
                    "parse",
                    "(Ljava/lang/String;)Landroid/net/Uri;",
                    &[JValue::Object(&uri_str)],
                )
                .unwrap()
                .l()
                .unwrap();

            let intent_cls = env.find_class("android/content/Intent").unwrap();
            let action = env
                .new_string("android.settings.MANAGE_APP_ALL_FILES_ACCESS_PERMISSION")
                .unwrap();

            let intent = env
                .new_object(
                    intent_cls,
                    "(Ljava/lang/String;Landroid/net/Uri;)V",
                    &[JValue::Object(&action), JValue::Object(&uri)],
                )
                .unwrap();

            let flags = 0x10000000; // FLAG_ACTIVITY_NEW_TASK
            let _ = env.call_method(
                &intent,
                "addFlags",
                "(I)Landroid/content/Intent;",
                &[JValue::Int(flags)],
            );

            let _ = env.call_method(
                &context,
                "startActivity",
                "(Landroid/content/Intent;)V",
                &[JValue::Object(&intent)],
            );
        }
    } else {
        // --- Android 8.0 - 10 (SDK 26-29) Logic: Standard Permissions ---

        let perm_string = env
            .new_string("android.permission.WRITE_EXTERNAL_STORAGE")
            .unwrap();

        // Check if already granted
        let check_res = env
            .call_method(
                &context,
                "checkSelfPermission",
                "(Ljava/lang/String;)I",
                &[JValue::Object(&perm_string)],
            )
            .unwrap()
            .i()
            .unwrap();

        if check_res != 0 {
            info!("Requesting Legacy Storage Permissions (SDK < 30)...");

            let string_cls = env.find_class("java/lang/String").unwrap();

            let perms_array = env
                .new_object_array(2, string_cls, JObject::null())
                .unwrap();
            let write_perm = env
                .new_string("android.permission.WRITE_EXTERNAL_STORAGE")
                .unwrap();
            let read_perm = env
                .new_string("android.permission.READ_EXTERNAL_STORAGE")
                .unwrap();

            env.set_object_array_element(&perms_array, 0, write_perm)
                .unwrap();
            env.set_object_array_element(&perms_array, 1, read_perm)
                .unwrap();

            // Call activity.requestPermissions(String[], int)
            let _ = env.call_method(
                &context,
                "requestPermissions",
                "([Ljava/lang/String;I)V",
                &[JValue::Object(&perms_array), JValue::Int(101)],
            );
        } else {
            info!("Legacy Storage Permissions already granted.");
        }
    }
}

fn acquire_wifi_lock(app: &AndroidApp) {
    use jni::objects::{JObject, JValue};

    info!("Acquiring WifiLock...");
    let vm_ptr = app.vm_as_ptr() as *mut jni::sys::JavaVM;
    let vm = unsafe { JavaVM::from_raw(vm_ptr).unwrap() };
    let mut env = vm.attach_current_thread().unwrap();

    let activity_ptr = app.activity_as_ptr() as jni::sys::jobject;
    let context = unsafe { JObject::from_raw(activity_ptr) };

    // 1. Get WifiManager
    let wifi_service_str = env.new_string("wifi").unwrap();
    let wifi_manager = env
        .call_method(
            &context,
            "getSystemService",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            &[JValue::Object(&wifi_service_str)],
        )
        .unwrap()
        .l()
        .unwrap();

    // 2. Create Lock (Mode 3 = WIFI_MODE_FULL_HIGH_PERF)
    let tag = env.new_string("Manatan:WifiLock").unwrap();
    let wifi_lock = env
        .call_method(
            &wifi_manager,
            "createWifiLock",
            "(ILjava/lang/String;)Landroid/net/wifi/WifiManager$WifiLock;",
            &[JValue::Int(3), JValue::Object(&tag)],
        )
        .unwrap()
        .l()
        .unwrap();

    // 3. Acquire
    let _ = env.call_method(&wifi_lock, "acquire", "()V", &[]);

    // 4. Release Reference (Java keeps the lock object alive)
    let _ = env.new_global_ref(&wifi_lock).unwrap();

    info!("✅ WifiLock Acquired!");
}

fn acquire_wake_lock(app: &AndroidApp) {
    use jni::objects::{JObject, JValue};

    info!("Acquiring Partial WakeLock...");
    let vm_ptr = app.vm_as_ptr() as *mut jni::sys::JavaVM;
    let vm = unsafe { JavaVM::from_raw(vm_ptr).unwrap() };
    let mut env = vm.attach_current_thread().unwrap();

    let activity_ptr = app.activity_as_ptr() as jni::sys::jobject;
    let context = unsafe { JObject::from_raw(activity_ptr) };

    let power_service_str = env.new_string("power").unwrap();
    let power_manager = env
        .call_method(
            &context,
            "getSystemService",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            &[JValue::Object(&power_service_str)],
        )
        .unwrap()
        .l()
        .unwrap();

    let tag = env.new_string("Manatan:CpuLock").unwrap();
    let wake_lock = env
        .call_method(
            &power_manager,
            "newWakeLock",
            "(ILjava/lang/String;)Landroid/os/PowerManager$WakeLock;",
            &[JValue::Int(1), JValue::Object(&tag)],
        )
        .unwrap()
        .l()
        .unwrap();

    // 3. Acquire
    let _ = env.call_method(&wake_lock, "acquire", "()V", &[]);

    let _ = env.new_global_ref(&wake_lock).unwrap();

    info!("✅ Partial WakeLock Acquired!");
}
// Add this helper function for getting last update time
fn get_apk_update_time(app: &AndroidApp) -> Option<i64> {
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as *mut _).ok()? };
    let mut env = vm.attach_current_thread().ok()?; // ← Add `mut` here
    let ctx = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };

    let pkg = env
        .call_method(&ctx, "getPackageName", "()Ljava/lang/String;", &[])
        .ok()?
        .l()
        .ok()?;
    let pm = env
        .call_method(
            &ctx,
            "getPackageManager",
            "()Landroid/content/pm/PackageManager;",
            &[],
        )
        .ok()?
        .l()
        .ok()?;
    let info = env
        .call_method(
            &pm,
            "getPackageInfo",
            "(Ljava/lang/String;I)Landroid/content/pm/PackageInfo;",
            &[(&pkg).into(), 0.into()],
        )
        .ok()?
        .l()
        .ok()?;

    env.get_field(&info, "lastUpdateTime", "J").ok()?.j().ok()
}

#[derive(Serialize)]
struct VersionResponse {
    version: String,
    variant: String,
    update_status: String,
}

async fn current_version_handler() -> impl IntoResponse {
    let version = env!("CARGO_PKG_VERSION");
    #[cfg(feature = "native_webview")]
    let variant = "native-webview";
    #[cfg(not(feature = "native_webview"))]
    let variant = "browser";

    let update_status = "disabled-for-fork".to_string();

    Json(VersionResponse {
        version: version.to_string(),
        variant: variant.to_string(),
        update_status,
    })
}

fn update_server_conf_local_source(app: &AndroidApp, files_dir: &Path) {
    let pending_marker = files_dir.join(".pending_local_source_config");

    let is_fresh_install = pending_marker.exists();
    if is_fresh_install {
        info!("🔄 Attempting to patch server.conf with localSourcePath (Fresh Install)...");
    } else {
        info!("🔄 Checking server.conf for legacy localSourcePath...");
    }

    let Some(shared_root) = resolve_manatan_shared_root_with_migration(app) else {
        warn!("Shared storage root unavailable; skipping server.conf localSourcePath patch.");
        return;
    };

    let local_sources_dir = shared_root.join("local-sources");
    if let Err(err) = std::fs::create_dir_all(&local_sources_dir) {
        error!("Failed to create local sources dir: {err:?}");
    }

    let tachidesk_data_dir = resolve_tachidesk_data_dir_with_migration(app, files_dir);

    // 2. Read server.conf
    let conf_path = tachidesk_data_dir.join("server.conf");
    if !conf_path.exists() {
        warn!(
            "server.conf not found at {:?}, skipping patch. (Server might not have created it yet?)",
            conf_path
        );
        return;
    }

    let content = match fs::read_to_string(&conf_path) {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to read server.conf: {:?}", e);
            return;
        }
    };

    // 3. Update Lines
    let mut new_lines = Vec::new();
    let mut patched = false;

    let desired_sources = local_sources_dir.to_string_lossy().to_string();
    let desired_anime = shared_root
        .join(".backend-compat-unused")
        .to_string_lossy()
        .to_string();

    for line in content.lines() {
        if line.trim().starts_with("server.localSourcePath =") {
            let should_patch = is_fresh_install;

            if should_patch {
                new_lines.push(format!(
                    "server.localSourcePath = \"{}\" # Autoconfigured by Manatan",
                    desired_sources
                ));
                patched = true;
            } else {
                new_lines.push(line.to_string());
            }
        } else if line.trim().starts_with("server.localAnimeSourcePath =") {
            let should_patch = is_fresh_install;

            if should_patch {
                new_lines.push(format!(
                    "server.localAnimeSourcePath = \"{}\" # Autoconfigured by Manatan",
                    desired_anime
                ));
                patched = true;
            } else {
                new_lines.push(line.to_string());
            }
        } else {
            new_lines.push(line.to_string());
        }
    }

    if patched {
        let new_content = new_lines.join("\n");
        if let Err(e) = fs::write(&conf_path, new_content) {
            error!("Failed to write server.conf: {:?}", e);
            return;
        }
        info!("✅ Patched server.conf local source paths");

        // Remove marker so we don't do this again (fresh install only)
        if is_fresh_install {
            let _ = fs::remove_file(&pending_marker);
        }
    } else {
        warn!("No local source path changes needed");
    }
}
