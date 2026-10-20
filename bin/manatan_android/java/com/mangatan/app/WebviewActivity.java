package dev.local.mangareader;

import android.app.Activity;
import android.content.ActivityNotFoundException;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.content.res.Configuration;
import android.graphics.Bitmap;
import android.graphics.Color;
import android.graphics.Insets;
import android.graphics.Rect;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import android.util.Log;
import android.view.Gravity;
import android.view.KeyEvent;
import android.view.PixelCopy;
import android.view.View;
import android.view.ViewGroup;
import android.view.ViewParent;
import android.view.Window;
import android.view.WindowInsets;
import android.view.inputmethod.InputMethodManager;
import android.webkit.ConsoleMessage;
import android.webkit.JavascriptInterface;
import android.webkit.RenderProcessGoneDetail;
import android.webkit.ValueCallback;
import android.webkit.WebChromeClient;
import android.webkit.WebResourceError;
import android.webkit.WebResourceRequest;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.window.OnBackInvokedCallback;
import android.window.OnBackInvokedDispatcher;
import android.widget.Button;
import android.widget.FrameLayout;
import android.widget.Toast;
import android.util.Base64;

import org.json.JSONObject;

import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.util.Locale;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

public class WebviewActivity extends Activity {
    private static final String DEFAULT_URL = "http://127.0.0.1:14568";
    private static final String TAG = "MangaReader";
    private static final String STATE_FOLDER_CALLBACK = "pending_folder_callback";
    private static final String STATE_PAGE_URL = "page_url";
    /** Encoding captures and writing exports must never block the UI thread. */
    private static final ExecutorService BACKGROUND = Executors.newSingleThreadExecutor(runnable -> {
        Thread thread = new Thread(runnable, "MangaReader-io");
        thread.setPriority(Thread.NORM_PRIORITY - 1);
        return thread;
    });

    private static volatile boolean visible;
    private final Handler mainHandler = new Handler(Looper.getMainLooper());
    private FrameLayout rootLayout;
    private WebView myWebView;
    private OnBackInvokedCallback backInvokedCallback;
    private int localLoadRetryCount = 0;
    private long lastLocalLoadErrorAt = 0L;
    private boolean runtimeRecoveryStarted = false;
    private long lastBackNavigationAt = 0L;
    private int rendererLossCount = 0;
    private long lastRendererLossAt = 0L;
    private boolean started = false;
    private boolean immersive = false;
    private boolean notificationPermissionRequested = false;
    private String targetUrl = DEFAULT_URL;

    // Flags
    private ValueCallback<Uri[]> uploadMessage;
    public final static int FILECHOOSER_RESULTCODE = 100;
    public final static int SAVEFILE_RESULTCODE = 101;
    public final static int FOLDER_PICKER_RESULTCODE = 102;
    private static final int ANKI_PERMISSION_REQUEST = 999;
    private static final int NOTIFICATION_PERMISSION_REQUEST = 1000;
    private String pendingExportFilename;
    private String pendingExportMimeType;
    private String pendingExportContent;
    private String pendingFolderCallbackId;

    public static boolean isVisible() {
        return visible;
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        if (savedInstanceState != null) {
            pendingFolderCallbackId = savedInstanceState.getString(STATE_FOLDER_CALLBACK);
        }

        String requested = getIntent().getStringExtra("TARGET_URL");
        if (requested != null) targetUrl = requested;

        if (!isForkLocalUrl(Uri.parse(targetUrl))) {
            Log.w(TAG, "Rejected non-local privileged WebView target");
            targetUrl = DEFAULT_URL;
        }

        requestWindowFeature(Window.FEATURE_NO_TITLE);
        setImmersiveMode(false);

        rootLayout = new FrameLayout(this);
        rootLayout.setBackgroundColor(surfaceColor());
        rootLayout.setLayoutParams(new FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT));
        rootLayout.setOnApplyWindowInsetsListener(this::applySystemInsets);

        attachNewWebView();

        // Close Button
        if (!targetUrl.contains(":14568")) {
            Button exitBtn = new Button(this);
            exitBtn.setText("CLOSE");
            exitBtn.setBackgroundColor(Color.RED);
            exitBtn.setTextColor(Color.WHITE);
            exitBtn.setAlpha(0.7f);
            FrameLayout.LayoutParams btnParams = new FrameLayout.LayoutParams(FrameLayout.LayoutParams.WRAP_CONTENT, FrameLayout.LayoutParams.WRAP_CONTENT);
            btnParams.gravity = Gravity.BOTTOM | Gravity.END;
            btnParams.setMargins(0, 0, 50, 50);
            exitBtn.setOnClickListener(v -> handleSmartClose());
            rootLayout.addView(exitBtn, btnParams);
        }

        setContentView(rootLayout);

        // After a recreation (light/dark switch, locale change, process restore) reopen the page
        // the user was on instead of the launch URL.
        String restoredUrl = savedInstanceState == null ? null : savedInstanceState.getString(STATE_PAGE_URL);
        myWebView.loadUrl(restoredUrl != null && isForkLocalUrl(Uri.parse(restoredUrl)) ? restoredUrl : targetUrl);

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            backInvokedCallback = this::handleBackNavigation;
            getOnBackInvokedDispatcher().registerOnBackInvokedCallback(
                    OnBackInvokedDispatcher.PRIORITY_DEFAULT,
                    backInvokedCallback);
        }
    }

    /** Builds and attaches a fully configured WebView; also used to recover from a lost renderer. */
    private void attachNewWebView() {
        WebView webView = new WebView(this);
        webView.setBackgroundColor(surfaceColor());
        rootLayout.addView(webView, 0, new FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT));
        myWebView = webView;

        WebSettings webSettings = webView.getSettings();
        webSettings.setJavaScriptEnabled(true);
        webSettings.setDomStorageEnabled(true);
        // Pages come only from the loopback server; nothing needs file:// access, and file pages
        // must never reach other origins.
        webSettings.setAllowFileAccess(false);
        webSettings.setAllowFileAccessFromFileURLs(false);
        webSettings.setAllowUniversalAccessFromFileURLs(false);
        // Kept on: <input type=file> results (dictionary/backup imports) are content:// URIs.
        webSettings.setAllowContentAccess(true);
        webSettings.setMixedContentMode(WebSettings.MIXED_CONTENT_NEVER_ALLOW);
        webSettings.setJavaScriptCanOpenWindowsAutomatically(false);
        webSettings.setSupportMultipleWindows(false);
        webSettings.setGeolocationEnabled(false);
        webSettings.setCacheMode(WebSettings.LOAD_DEFAULT);

        String userAgent = webSettings.getUserAgentString();
        webSettings.setUserAgentString(userAgent + " MangaReaderForkNative ManatanNative");

        webView.setWebViewClient(new WebViewClient() {
            @Override
            public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
                Uri uri = request.getUrl();
                String urlString = uri.toString();

                // 1. Handle intent:// links (Shim -> App transition)
                if (urlString.startsWith("intent://")) {
                    try {
                        Intent intent = Intent.parseUri(urlString, Intent.URI_INTENT_SCHEME);
                        Uri deepLink = intent.getData();
                        if (deepLink != null
                                && "mangareaderfork".equals(deepLink.getScheme())) {
                            String actualUrl = deepLink.getQueryParameter("url");
                            if (actualUrl != null) {
                                launchReader(actualUrl);
                                return true;
                            }
                        }
                    } catch (Exception e) {
                        Log.e(TAG, "Intent parse failed", e);
                    }
                    return true;
                }

                // 2. BLOCK SHIM in Native App
                if (urlString.contains("/api/v1/webview")) {
                    String fragment = uri.getFragment();
                    if (fragment != null && !fragment.isEmpty()) {
                        Log.i(TAG, "Native intercept: skipping WebView shim");
                        launchReader(fragment);
                        return true;
                    }
                }

                if (isForkLocalUrl(uri)) {
                    return false;
                }

                // Remote frames never navigate inside the privileged WebView, and a sub-frame
                // (e.g. an embed in dictionary markup) must not launch a browser by itself.
                if (!request.isForMainFrame()) return true;

                // Keep untrusted remote pages outside the privileged WebView that owns
                // the Android bridge. Source protocols are handled by native clients.
                openInBrowser(uri);
                return true;
            }

            @Override
            public void onPageFinished(WebView view, String url) {
                super.onPageFinished(view, url);
                Log.i(TAG, "WebView page finished");
            }

            @Override
            public void onReceivedError(WebView view, WebResourceRequest request, WebResourceError error) {
                if (!request.isForMainFrame()) return;
                Log.e(TAG, "WebView main-frame load failed: " + error.getDescription());
                if (view != myWebView || !isForkLocalUrl(request.getUrl())) return;
                long now = SystemClock.elapsedRealtime();
                // The retry budget is for one outage, not for the whole session.
                if (now - lastLocalLoadErrorAt > 30_000L) localLoadRetryCount = 0;
                lastLocalLoadErrorAt = now;
                localLoadRetryCount += 1;
                if (!MangatanActivity.isRuntimeHostAlive() || localLoadRetryCount >= 8) {
                    requestRuntimeRecovery();
                    return;
                }
                final String retryUrl = request.getUrl().toString();
                view.loadDataWithBaseURL(
                        targetUrl,
                        "<html><body style='margin:0;background:#090909;color:#ddd;font:16px sans-serif;display:grid;place-items:center;height:100vh'>Loading your library…</body></html>",
                        "text/html",
                        "UTF-8",
                        null);
                mainHandler.postDelayed(
                        () -> {
                            if (!isFinishing() && myWebView == view) myWebView.loadUrl(retryUrl);
                        },
                        Math.min(4000, 500L * localLoadRetryCount));
            }

            @Override
            public boolean onRenderProcessGone(WebView view, RenderProcessGoneDetail detail) {
                recoverFromRendererLoss(view, detail.didCrash());
                return true;
            }
        });

        webView.setWebChromeClient(new WebChromeClient() {
            @Override
            public boolean onConsoleMessage(ConsoleMessage message) {
                // Only warnings and errors: page logs can be chatty and may contain titles or
                // looked-up text, which must not end up in logcat.
                ConsoleMessage.MessageLevel level = message.messageLevel();
                if (level == ConsoleMessage.MessageLevel.ERROR) {
                    Log.e("MangaReaderConsole", message.sourceId() + ":" + message.lineNumber() + " " + message.message());
                } else if (level == ConsoleMessage.MessageLevel.WARNING) {
                    Log.w("MangaReaderConsole", message.sourceId() + ":" + message.lineNumber() + " " + message.message());
                }
                return true;
            }

            @Override
            public boolean onShowFileChooser(WebView webView, ValueCallback<Uri[]> filePathCallback, WebChromeClient.FileChooserParams fileChooserParams) {
                if (uploadMessage != null) {
                    uploadMessage.onReceiveValue(null);
                    uploadMessage = null;
                }
                uploadMessage = filePathCallback;
                try {
                    startActivityForResult(fileChooserParams.createIntent(), FILECHOOSER_RESULTCODE);
                } catch (ActivityNotFoundException e) {
                    uploadMessage = null;
                    return false;
                }
                return true;
            }
        });

        NativeCaptureBridge nativeBridge = new NativeCaptureBridge();
        webView.addJavascriptInterface(nativeBridge, "ManatanNative");
        webView.addJavascriptInterface(nativeBridge, "MangaReaderNative");

        if (!started) {
            webView.onPause();
        }
    }

    /**
     * The renderer can be killed for memory while the app is in the background, or crash. Rebuild
     * only the WebView in place (instead of finishing and relaunching the activity) and reload the
     * page the user was on; repeated losses fall back to the library to avoid a crash loop.
     */
    private void recoverFromRendererLoss(WebView view, boolean crashed) {
        String currentUrl = view.getUrl();
        boolean wasActive = view == myWebView;
        Log.e(TAG, "WebView renderer " + (crashed ? "crashed" : "was killed") + "; rebuilding reader surface");
        destroyWebView(view);
        if (!wasActive || isFinishing() || isDestroyed()) return;

        long now = SystemClock.elapsedRealtime();
        if (now - lastRendererLossAt > 60_000L) rendererLossCount = 0;
        lastRendererLossAt = now;
        rendererLossCount += 1;

        String recoveryUrl = currentUrl != null && isForkLocalUrl(Uri.parse(currentUrl)) ? currentUrl : targetUrl;
        if (rendererLossCount > 2) recoveryUrl = DEFAULT_URL;
        attachNewWebView();
        myWebView.loadUrl(recoveryUrl);
    }

    private void sendCaptureCallback(String callbackId, String dataUrl) {
        final String safeId = JSONObject.quote(callbackId);
        final String safeData = dataUrl == null ? "null" : JSONObject.quote(dataUrl);
        final String script =
            "window.__manatanNativeCaptureCallback && window.__manatanNativeCaptureCallback(" + safeId + "," + safeData + ");";
        runOnUiThread(() -> {
            if (myWebView != null) myWebView.evaluateJavascript(script, null);
        });
    }

    private String encodeBitmapToDataUrl(Bitmap bitmap, float quality) {
        if (bitmap == null) return null;
        try {
            int qualityInt = Math.max(1, Math.min(100, Math.round(quality * 100f)));
            java.io.ByteArrayOutputStream output = new java.io.ByteArrayOutputStream();
            bitmap.compress(Bitmap.CompressFormat.JPEG, qualityInt, output);
            byte[] bytes = output.toByteArray();
            String base64 = Base64.encodeToString(bytes, Base64.NO_WRAP);
            return "data:image/jpeg;base64," + base64;
        } catch (Exception e) {
            Log.e(TAG, "Failed to encode bitmap", e);
            return null;
        }
    }

    /** JavaScript bridge. Methods run on WebView's private "JavaBridge" thread, not the UI thread. */
    private class NativeCaptureBridge {
        @JavascriptInterface
        public String getAnkiBridgeToken() {
            return AnkiBridge.getBridgeToken(getApplicationContext());
        }

        @JavascriptInterface
        public void requestAnkiPermission() {
            runOnUiThread(() -> {
                if (checkSelfPermission("com.ichi2.anki.permission.READ_WRITE_DATABASE")
                        != PackageManager.PERMISSION_GRANTED) {
                    requestPermissions(new String[]{"com.ichi2.anki.permission.READ_WRITE_DATABASE"}, ANKI_PERMISSION_REQUEST);
                } else {
                    notifyPermissionResult("anki", true);
                }
            });
        }

        @JavascriptInterface
        public void setImmersive(final boolean enabled) {
            runOnUiThread(() -> setImmersiveMode(enabled));
        }

        @JavascriptInterface
        public void updateBackgroundJobs(final boolean active, final String title, final int completed, final int total) {
            runOnUiThread(() -> {
                try {
                    if (!active) {
                        JobForegroundService.stop(getApplicationContext());
                        return;
                    }
                    // Ask once per screen: progress ticks arrive every few seconds and each
                    // permission request pauses this activity.
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU
                            && !notificationPermissionRequested
                            && checkSelfPermission("android.permission.POST_NOTIFICATIONS") != PackageManager.PERMISSION_GRANTED) {
                        notificationPermissionRequested = true;
                        requestPermissions(
                                new String[]{"android.permission.POST_NOTIFICATIONS"},
                                NOTIFICATION_PERMISSION_REQUEST);
                    }
                    JobForegroundService.update(getApplicationContext(), title, completed, total);
                } catch (Exception error) {
                    Log.e(TAG, "Could not update background-job notification", error);
                }
            });
        }

        @JavascriptInterface
        public void pickFolder(final String callbackId) {
            runOnUiThread(() -> {
                pendingFolderCallbackId = callbackId;
                Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
                intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION
                        | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION
                        | Intent.FLAG_GRANT_PREFIX_URI_PERMISSION);
                try {
                    startActivityForResult(intent, FOLDER_PICKER_RESULTCODE);
                } catch (ActivityNotFoundException error) {
                    Log.e(TAG, "No document picker is available", error);
                    pendingFolderCallbackId = null;
                    sendFolderCallback(callbackId, null);
                    Toast.makeText(getApplicationContext(), "No folder picker is available on this device", Toast.LENGTH_SHORT).show();
                }
            });
        }

        @JavascriptInterface
        public void openExternalUrl(final String url) {
            if (url == null || url.isEmpty()) return;
            final Uri uri = Uri.parse(url);
            String scheme = uri.getScheme() == null ? "" : uri.getScheme().toLowerCase(Locale.ROOT);
            // Only web links: intent:, file:, content: and other schemes could reach app
            // components or private files on behalf of page script.
            if (!scheme.equals("https") && !scheme.equals("http")) {
                Log.w(TAG, "Refused to open a non-web URL scheme: " + scheme);
                return;
            }
            runOnUiThread(() -> openInBrowser(uri));
        }

        @JavascriptInterface
        public void captureFrame(final String callbackId, final String payloadJson) {
            float x = 0f;
            float y = 0f;
            float width = 0f;
            float height = 0f;
            float dpr = 1f;
            float quality = 0.92f;
            try {
                JSONObject payload = new JSONObject(payloadJson);
                x = (float) payload.optDouble("x", 0d);
                y = (float) payload.optDouble("y", 0d);
                width = (float) payload.optDouble("width", 0d);
                height = (float) payload.optDouble("height", 0d);
                dpr = (float) payload.optDouble("dpr", 1d);
                quality = (float) payload.optDouble("quality", 0.92d);
            } catch (Exception e) {
                Log.e(TAG, "Failed to parse capture payload", e);
            }

            final float cropX = x;
            final float cropY = y;
            final float cropWidth = width;
            final float cropHeight = height;
            final float devicePixelRatio = dpr;
            final float qualityFinal = quality;

            runOnUiThread(() -> captureRegion(callbackId, cropX, cropY, cropWidth, cropHeight, devicePixelRatio, qualityFinal));
        }

        @JavascriptInterface
        public void hideKeyboard() {
            runOnUiThread(() -> {
                View view = getCurrentFocus();
                if (view != null) {
                    InputMethodManager imm = (InputMethodManager) getSystemService(INPUT_METHOD_SERVICE);
                    imm.hideSoftInputFromWindow(view.getWindowToken(), 0);
                }
            });
        }

        @JavascriptInterface
        public void saveFile(final String filename, final String mimeType, final String content) {
            runOnUiThread(() -> {
                try {
                    pendingExportFilename = filename;
                    pendingExportMimeType = mimeType;
                    pendingExportContent = content;

                    Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT);
                    intent.addCategory(Intent.CATEGORY_OPENABLE);
                    intent.setType(normalizeExportMimeType(mimeType));
                    intent.putExtra(Intent.EXTRA_TITLE, filename);
                    startActivityForResult(intent, SAVEFILE_RESULTCODE);
                } catch (Exception e) {
                    Log.e(TAG, "Failed to launch save dialog", e);
                    clearPendingExport();
                    Toast.makeText(WebviewActivity.this, "No document provider is available", Toast.LENGTH_SHORT).show();
                }
            });
        }
    }

    /**
     * Copies exactly the requested page region. The source rectangle is offset by the WebView's
     * position in the window, so captures stay aligned when the window also contains system-bar
     * areas, and only the cropped pixels are allocated. JPEG encoding runs off the UI thread.
     */
    private void captureRegion(String callbackId, float x, float y, float width, float height, float dpr, float quality) {
        WebView view = myWebView;
        if (view == null || view.getWidth() <= 0 || view.getHeight() <= 0 || isFinishing()) {
            sendCaptureCallback(callbackId, null);
            return;
        }
        try {
            int[] location = new int[2];
            view.getLocationInWindow(location);
            Rect source = new Rect(location[0], location[1],
                    location[0] + view.getWidth(), location[1] + view.getHeight());
            if (width > 0 && height > 0) {
                Rect crop = new Rect(
                        location[0] + Math.round(x * dpr),
                        location[1] + Math.round(y * dpr),
                        location[0] + Math.round((x + width) * dpr),
                        location[1] + Math.round((y + height) * dpr));
                if (crop.intersect(source)) source = crop;
            }
            Bitmap bitmap = Bitmap.createBitmap(source.width(), source.height(), Bitmap.Config.ARGB_8888);
            PixelCopy.request(getWindow(), source, bitmap, result -> {
                if (result != PixelCopy.SUCCESS) {
                    Log.e(TAG, "PixelCopy failed: " + result);
                    bitmap.recycle();
                    sendCaptureCallback(callbackId, null);
                    return;
                }
                BACKGROUND.execute(() -> {
                    String dataUrl = encodeBitmapToDataUrl(bitmap, quality);
                    bitmap.recycle();
                    sendCaptureCallback(callbackId, dataUrl);
                });
            }, mainHandler);
        } catch (Exception e) {
            Log.e(TAG, "Native capture failed", e);
            sendCaptureCallback(callbackId, null);
        }
    }

    /** Follows the system light/dark setting (the theme is DayNight on Android 10+). */
    private boolean isNightMode() {
        int night = getResources().getConfiguration().uiMode & Configuration.UI_MODE_NIGHT_MASK;
        return night == Configuration.UI_MODE_NIGHT_YES;
    }

    /** Background behind the page and under the system bars; avoids a black flash in light mode. */
    private int surfaceColor() {
        return isNightMode() ? Color.BLACK : Color.WHITE;
    }

    private void setImmersiveMode(boolean enabled) {
        immersive = enabled;
        int flags = View.SYSTEM_UI_FLAG_LAYOUT_STABLE;
        if (!isNightMode()) {
            // setSystemUiVisibility replaces every flag, so keep the light-bar appearance that
            // gives dark status/navigation icons on the light surface.
            flags |= View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR | View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR;
        }
        if (enabled) {
            flags |= View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                    | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                    | View.SYSTEM_UI_FLAG_FULLSCREEN
                    | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                    | View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY;
        }
        getWindow().getDecorView().setSystemUiVisibility(flags);
        if (rootLayout != null) rootLayout.requestApplyInsets();
    }

    /**
     * Library and settings screens sit between the system bars like ordinary Android screens; the
     * reader (immersive) uses the whole display. When the window is edge-to-edge (always on
     * Android 15+ for this target SDK) the bar and keyboard insets would otherwise cover the page.
     * Display-cutout insets are passed on in immersive mode so CSS safe-area-inset still works.
     */
    private WindowInsets applySystemInsets(View view, WindowInsets insets) {
        int left = 0;
        int top = 0;
        int right = 0;
        int bottom = 0;
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            Insets bars = insets.getInsets(WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
            Insets ime = insets.getInsets(WindowInsets.Type.ime());
            if (immersive) {
                bottom = ime.bottom;
            } else {
                left = bars.left;
                top = bars.top;
                right = bars.right;
                bottom = Math.max(bars.bottom, ime.bottom);
            }
        } else if (!immersive) {
            left = insets.getSystemWindowInsetLeft();
            top = insets.getSystemWindowInsetTop();
            right = insets.getSystemWindowInsetRight();
            bottom = insets.getSystemWindowInsetBottom();
        }
        view.setPadding(left, top, right, bottom);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            if (!immersive) return WindowInsets.CONSUMED;
            return new WindowInsets.Builder(insets)
                    .setInsets(WindowInsets.Type.systemBars(), Insets.NONE)
                    .setInsets(WindowInsets.Type.ime(), Insets.NONE)
                    .build();
        }
        return immersive ? insets : insets.consumeSystemWindowInsets();
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        // Dialogs, the keyboard and permission prompts clear the legacy immersive flags.
        if (hasFocus && immersive) setImmersiveMode(true);
    }

    private void sendFolderCallback(String callbackId, String uri) {
        if (myWebView == null || callbackId == null) return;
        String script = "window.mangaReaderFolderPicked && window.mangaReaderFolderPicked("
                + JSONObject.quote(callbackId) + ","
                + (uri == null ? "null" : JSONObject.quote(uri)) + ");";
        myWebView.evaluateJavascript(script, null);
    }

    /** Lets the page react to a permission decision (e.g. retry the AnkiDroid connection). */
    private void notifyPermissionResult(String permission, boolean granted) {
        if (myWebView == null) return;
        String script = "window.dispatchEvent(new CustomEvent('mangareadernativepermission',{detail:{permission:"
                + JSONObject.quote(permission) + ",granted:" + granted + "}}));";
        myWebView.evaluateJavascript(script, null);
    }

    private String normalizeExportMimeType(String mimeType) {
        if (mimeType == null || mimeType.isEmpty()) {
            return "application/octet-stream";
        }

        int charsetSeparator = mimeType.indexOf(';');
        if (charsetSeparator >= 0) {
            return mimeType.substring(0, charsetSeparator).trim();
        }

        return mimeType;
    }

    private boolean isForkLocalUrl(Uri uri) {
        if (uri == null || !"http".equals(uri.getScheme())) return false;
        String host = uri.getHost();
        int port = uri.getPort();
        return ("127.0.0.1".equals(host) || "localhost".equals(host))
                && (port == 14568 || port == 18765);
    }

    private static boolean isRootUrl(Uri uri) {
        String path = uri.getPath();
        return (path == null || path.isEmpty() || path.equals("/"))
                && uri.getQuery() == null && uri.getFragment() == null;
    }

    private void clearPendingExport() {
        pendingExportFilename = null;
        pendingExportMimeType = null;
        pendingExportContent = null;
    }

    private void openInBrowser(Uri uri) {
        try {
            Intent browserIntent = new Intent(Intent.ACTION_VIEW, uri);
            browserIntent.addCategory(Intent.CATEGORY_BROWSABLE);
            startActivity(browserIntent);
        } catch (ActivityNotFoundException e) {
            Log.e(TAG, "No browser available", e);
            Toast.makeText(getApplicationContext(), "No browser is available to open this link", Toast.LENGTH_SHORT).show();
        } catch (Exception e) {
            Log.e(TAG, "Could not open external link", e);
        }
    }

    /**
     * Shim/deep-link navigation. Local pages load in this WebView (stacking a second WebView
     * activity would double memory use, and with singleTop the intent was silently dropped);
     * anything else opens outside the privileged WebView.
     */
    private void launchReader(String url) {
        if (url == null || url.isEmpty()) return;
        Uri uri = Uri.parse(url);
        if (isForkLocalUrl(uri)) {
            if (myWebView != null) myWebView.loadUrl(url);
            return;
        }
        String scheme = uri.getScheme() == null ? "" : uri.getScheme().toLowerCase(Locale.ROOT);
        if (scheme.equals("http") || scheme.equals("https")) openInBrowser(uri);
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        String requested = intent == null ? null : intent.getStringExtra("TARGET_URL");
        if (requested == null || myWebView == null) return;
        Uri uri = Uri.parse(requested);
        if (!isForkLocalUrl(uri)) return;
        String current = myWebView.getUrl();
        if (requested.equals(current)) return;
        // A plain "show the app" request must not throw away the page (and reader position) the
        // user is on; only a specific destination navigates.
        if (isRootUrl(uri) && current != null && isForkLocalUrl(Uri.parse(current))) return;
        myWebView.loadUrl(requested);
    }

    private void handleSmartClose() {
        moveTaskToBack(true);
    }

    private void requestRuntimeRecovery() {
        if (runtimeRecoveryStarted || isFinishing()) return;
        runtimeRecoveryStarted = true;

        String recoveryUrl = targetUrl;
        if (myWebView != null) {
            String currentUrl = myWebView.getUrl();
            if (currentUrl != null && isForkLocalUrl(Uri.parse(currentUrl))) {
                recoveryUrl = currentUrl;
            }
        }
        getSharedPreferences("runtime_recovery", MODE_PRIVATE)
                .edit()
                .putString("target_url", recoveryUrl)
                .apply();

        Log.w(TAG, "Embedded server unavailable; recreating runtime host");
        Intent recoveryIntent = new Intent(this, MangatanActivity.class);
        recoveryIntent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TASK);
        startActivity(recoveryIntent);
    }

    private void handleBackNavigation() {
        long now = SystemClock.elapsedRealtime();
        if (now - lastBackNavigationAt < 750L) return;
        lastBackNavigationAt = now;

        final WebView view = myWebView;
        if (view == null) {
            handleSmartClose();
            return;
        }
        try {
            view.evaluateJavascript(
                "(() => {"
                    + "if (window.__handleNativeBack && window.__handleNativeBack()) return true;"
                    + "if (window.location.pathname.startsWith('/fork/reader/')) {"
                    + "window.location.assign('/library'); return true;"
                    + "}"
                    + "return false;"
                    + "})()",
                value -> {
                    if ("true".equals(value)) return;
                    // The WebView may have been replaced or destroyed while the script ran.
                    if (view == myWebView && view.canGoBack()) view.goBack();
                    else handleSmartClose();
                });
        } catch (Exception error) {
            if (view.canGoBack()) view.goBack();
            else handleSmartClose();
        }
    }

    @Override
    public void onBackPressed() {
        handleBackNavigation();
    }

    @Override
    protected void onStart() {
        super.onStart();
        visible = true;
        started = true;
        // Timers are paused/resumed in onStart/onStop, not onPause/onResume: a permission dialog,
        // the folder picker overlay or split-screen focus loss must not freeze the page while it
        // is still on screen.
        if (myWebView != null) {
            myWebView.onResume();
            myWebView.resumeTimers();
        }
    }

    @Override
    protected void onResume() {
        super.onResume();
        visible = true;
        if (!MangatanActivity.isRuntimeHostAlive()) {
            requestRuntimeRecovery();
        }
    }

    @Override
    protected void onStop() {
        visible = false;
        started = false;
        if (myWebView != null) {
            myWebView.onPause();
            // pauseTimers() is process-wide; this is the only WebView, so it stops all page
            // timers and polling while the app is in the background.
            myWebView.pauseTimers();
        }
        super.onStop();
    }

    @Override
    protected void onSaveInstanceState(Bundle outState) {
        super.onSaveInstanceState(outState);
        // Survives process death while the system folder picker is open.
        if (pendingFolderCallbackId != null) outState.putString(STATE_FOLDER_CALLBACK, pendingFolderCallbackId);
        String currentUrl = myWebView == null ? null : myWebView.getUrl();
        if (currentUrl != null && isForkLocalUrl(Uri.parse(currentUrl))) outState.putString(STATE_PAGE_URL, currentUrl);
    }

    private void destroyWebView(WebView view) {
        if (view == null) return;
        try {
            view.stopLoading();
            view.onPause();
            view.removeJavascriptInterface("ManatanNative");
            view.removeJavascriptInterface("MangaReaderNative");
            view.setWebChromeClient(null);
            ViewParent parent = view.getParent();
            if (parent instanceof ViewGroup) ((ViewGroup) parent).removeView(view);
            view.removeAllViews();
            view.destroy();
        } catch (Exception error) {
            Log.e(TAG, "Could not fully release WebView", error);
        }
        if (view == myWebView) myWebView = null;
    }

    @Override
    protected void onDestroy() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU && backInvokedCallback != null) {
            getOnBackInvokedDispatcher().unregisterOnBackInvokedCallback(backInvokedCallback);
            backInvokedCallback = null;
        }
        visible = false;
        mainHandler.removeCallbacksAndMessages(null);
        if (uploadMessage != null) {
            // A file chooser that never answers leaves the page's <input type=file> stuck.
            uploadMessage.onReceiveValue(null);
            uploadMessage = null;
        }
        destroyWebView(myWebView);
        super.onDestroy();
    }

    @Override
    public boolean onKeyDown(int keyCode, KeyEvent event) {
        if (keyCode == KeyEvent.KEYCODE_BACK) {
            handleBackNavigation();
            return true;
        }
        return super.onKeyDown(keyCode, event);
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        if (requestCode == FILECHOOSER_RESULTCODE) {
            if (uploadMessage == null) return;
            uploadMessage.onReceiveValue(WebChromeClient.FileChooserParams.parseResult(resultCode, data));
            uploadMessage = null;
        } else if (requestCode == SAVEFILE_RESULTCODE) {
            final String content = pendingExportContent;
            final String savedName = pendingExportFilename != null ? pendingExportFilename : "file";
            clearPendingExport();
            if (resultCode != RESULT_OK || data == null || data.getData() == null) return;
            final Uri uri = data.getData();
            final Context app = getApplicationContext();
            // Exports can be large; write them off the UI thread.
            BACKGROUND.execute(() -> {
                boolean saved = false;
                try (OutputStream outputStream = app.getContentResolver().openOutputStream(uri)) {
                    if (outputStream == null) {
                        throw new IllegalStateException("No output stream returned for export URI");
                    }
                    outputStream.write((content == null ? "" : content).getBytes(StandardCharsets.UTF_8));
                    outputStream.flush();
                    saved = true;
                } catch (Exception e) {
                    Log.e(TAG, "Failed to write exported file", e);
                }
                final boolean success = saved;
                mainHandler.post(() -> Toast.makeText(app,
                        success ? "Saved " + savedName : "Failed to save file",
                        Toast.LENGTH_SHORT).show());
            });
        } else if (requestCode == FOLDER_PICKER_RESULTCODE) {
            String callbackId = pendingFolderCallbackId;
            pendingFolderCallbackId = null;
            if (resultCode != RESULT_OK || data == null || data.getData() == null) {
                sendFolderCallback(callbackId, null);
                return;
            }
            Uri uri = data.getData();
            try {
                getContentResolver().takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION);
                sendFolderCallback(callbackId, uri.toString());
            } catch (RuntimeException error) {
                Log.e(TAG, "Could not persist folder permission", error);
                sendFolderCallback(callbackId, null);
            }
        } else {
            super.onActivityResult(requestCode, resultCode, data);
        }
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, String[] permissions, int[] grantResults) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults);
        if (requestCode == ANKI_PERMISSION_REQUEST) {
            boolean granted = grantResults.length > 0 && grantResults[0] == PackageManager.PERMISSION_GRANTED;
            if (!granted) {
                Toast.makeText(this, "Anki permission denied.", Toast.LENGTH_LONG).show();
            }
            notifyPermissionResult("anki", granted);
        }
    }
}
