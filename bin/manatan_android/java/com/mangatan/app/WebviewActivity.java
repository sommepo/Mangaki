package dev.local.mangareader;

import android.app.Activity;
import android.content.ActivityNotFoundException;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.Color;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import android.view.Gravity;
import android.view.KeyEvent;
import android.view.PixelCopy;
import android.view.View;
import android.view.ViewGroup;
import android.view.ViewParent;
import android.view.Window;
import android.view.inputmethod.InputMethodManager;
import android.webkit.JavascriptInterface;
import android.webkit.ConsoleMessage;
import android.webkit.ValueCallback;
import android.webkit.WebChromeClient;
import android.webkit.WebResourceError;
import android.webkit.WebResourceRequest;
import android.webkit.RenderProcessGoneDetail;
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

public class WebviewActivity extends Activity {
    private static volatile boolean visible;
    private WebView myWebView;
    private OnBackInvokedCallback backInvokedCallback;
    private int localLoadRetryCount = 0;
    private boolean runtimeRecoveryStarted = false;
    private long lastBackNavigationAt = 0L;
    private String targetUrl = "http://127.0.0.1:14568";
    
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

        Intent intent = getIntent();
        if (intent.hasExtra("TARGET_URL")) {
            targetUrl = intent.getStringExtra("TARGET_URL");
        }
        
        if (!isForkLocalUrl(Uri.parse(targetUrl))) {
            Log.w("MangaReader", "Rejected non-local privileged WebView target");
            targetUrl = "http://127.0.0.1:14568";
        }

        requestWindowFeature(Window.FEATURE_NO_TITLE);
        setImmersiveMode(false);

        FrameLayout layout = new FrameLayout(this);
        layout.setLayoutParams(new FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT));

        myWebView = new WebView(this);
        myWebView.setBackgroundColor(Color.BLACK);
        layout.addView(myWebView, new FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT));

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
            layout.addView(exitBtn, btnParams);
        }

        setContentView(layout);

        WebSettings webSettings = myWebView.getSettings();
        webSettings.setJavaScriptEnabled(true);
        webSettings.setDomStorageEnabled(true);
        webSettings.setAllowFileAccess(false);
        webSettings.setAllowContentAccess(true);
        
        String userAgent = webSettings.getUserAgentString();
        webSettings.setUserAgentString(userAgent + " MangaReaderForkNative ManatanNative");

        myWebView.setWebViewClient(new WebViewClient() {
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
                        Log.e("Manatan", "Intent parse failed", e);
                    }
                    return true;
                }
                
                // 2. BLOCK SHIM in Native App
                if (urlString.contains("/api/v1/webview")) {
                    String fragment = uri.getFragment(); 
                    if (fragment != null && !fragment.isEmpty()) {
                        Log.i("Manatan", "Native Intercept: Skipping Shim for " + fragment);
                        launchReader(fragment);
                        return true; 
                    }
                }

                if (isForkLocalUrl(uri)) {
                    return false;
                }

                // Keep untrusted remote pages outside the privileged WebView that owns
                // the Android bridge. Source protocols are handled by native clients.
                try {
                    Intent browserIntent = new Intent(Intent.ACTION_VIEW, uri);
                    browserIntent.addCategory(Intent.CATEGORY_BROWSABLE);
                    startActivity(browserIntent);
                } catch (Exception error) {
                    Log.e("MangaReader", "Could not open external link", error);
                }
                return true;
            }

            @Override
            public void onPageFinished(WebView view, String url) {
                super.onPageFinished(view, url);
                Log.i("MangaReader", "WebView loaded " + url);
                // MODIFIED: No sync here. 
                // This prevents the toast from appearing when the webview first opens.
            }

            @Override
            public void onReceivedError(WebView view, WebResourceRequest request, WebResourceError error) {
                Log.e("MangaReader", "WebView request failed: " + request.getUrl() + " — " + error.getDescription());
                if (request.isForMainFrame() && isForkLocalUrl(request.getUrl())) {
                    localLoadRetryCount += 1;
                    if (!MangatanActivity.isRuntimeHostAlive() || localLoadRetryCount >= 8) {
                        requestRuntimeRecovery();
                        return;
                    }
                    view.loadDataWithBaseURL(
                            targetUrl,
                            "<html><body style='margin:0;background:#090909;color:#ddd;font:16px sans-serif;display:grid;place-items:center;height:100vh'>Loading your library…</body></html>",
                            "text/html",
                            "UTF-8",
                            null);
                    new Handler(Looper.getMainLooper()).postDelayed(
                            () -> {
                                if (!isFinishing() && myWebView != null) myWebView.loadUrl(targetUrl);
                            },
                            Math.min(4000, 500L * localLoadRetryCount));
                }
            }

            @Override
            public boolean onRenderProcessGone(WebView view, RenderProcessGoneDetail detail) {
                String currentUrl = view.getUrl();
                String recoveryUrl = isForkLocalUrl(Uri.parse(currentUrl == null ? "" : currentUrl))
                        ? currentUrl
                        : targetUrl;
                Log.e("MangaReader", "WebView renderer exited; rebuilding reader surface");
                destroyWebView(view);
                Intent recoveryIntent = new Intent(WebviewActivity.this, WebviewActivity.class);
                recoveryIntent.putExtra("TARGET_URL", recoveryUrl);
                recoveryIntent.addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP);
                finish();
                startActivity(recoveryIntent);
                return true;
            }
        });

        myWebView.setWebChromeClient(new WebChromeClient() {
            @Override
            public boolean onConsoleMessage(ConsoleMessage message) {
                Log.e("MangaReaderConsole", message.messageLevel() + " " + message.sourceId() + ":" + message.lineNumber() + " " + message.message());
                return true;
            }

            @Override
            public boolean onShowFileChooser(WebView webView, ValueCallback<Uri[]> filePathCallback, WebChromeClient.FileChooserParams fileChooserParams) {
                if (uploadMessage != null) {
                    uploadMessage.onReceiveValue(null);
                    uploadMessage = null;
                }
                uploadMessage = filePathCallback;
                Intent intent = fileChooserParams.createIntent();
                try {
                    startActivityForResult(intent, FILECHOOSER_RESULTCODE);
                } catch (ActivityNotFoundException e) {
                    uploadMessage = null;
                    return false;
                }
                return true;
            }
        });

        NativeCaptureBridge nativeBridge = new NativeCaptureBridge();
        myWebView.addJavascriptInterface(nativeBridge, "ManatanNative");
        myWebView.addJavascriptInterface(nativeBridge, "MangaReaderNative");

        myWebView.loadUrl(targetUrl);

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            backInvokedCallback = this::handleBackNavigation;
            getOnBackInvokedDispatcher().registerOnBackInvokedCallback(
                    OnBackInvokedDispatcher.PRIORITY_DEFAULT,
                    backInvokedCallback);
        }
    }

    private void sendCaptureCallback(String callbackId, String dataUrl) {
        if (myWebView == null) return;
        final String safeId = JSONObject.quote(callbackId);
        final String safeData = dataUrl == null ? "null" : JSONObject.quote(dataUrl);
        final String script =
            "window.__manatanNativeCaptureCallback && window.__manatanNativeCaptureCallback(" + safeId + "," + safeData + ");";
        runOnUiThread(() -> myWebView.evaluateJavascript(script, null));
    }

    private Bitmap cropBitmap(Bitmap source, float x, float y, float width, float height, float dpr) {
        if (source == null) return null;
        if (width <= 0 || height <= 0) return source;
        int left = Math.max(0, Math.round(x * dpr));
        int top = Math.max(0, Math.round(y * dpr));
        int cropWidth = Math.min(source.getWidth() - left, Math.round(width * dpr));
        int cropHeight = Math.min(source.getHeight() - top, Math.round(height * dpr));
        if (cropWidth <= 0 || cropHeight <= 0) return source;
        return Bitmap.createBitmap(source, left, top, cropWidth, cropHeight);
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
            Log.e("Manatan", "Failed to encode bitmap", e);
            return null;
        }
    }

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
                if (!active) {
                    JobForegroundService.stop(getApplicationContext());
                    return;
                }
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU
                        && checkSelfPermission("android.permission.POST_NOTIFICATIONS") != PackageManager.PERMISSION_GRANTED) {
                    requestPermissions(
                            new String[]{"android.permission.POST_NOTIFICATIONS"},
                            NOTIFICATION_PERMISSION_REQUEST);
                }
                try {
                    JobForegroundService.update(getApplicationContext(), title, completed, total);
                } catch (Exception error) {
                    Log.e("MangaReader", "Could not start background-job notification", error);
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
                startActivityForResult(intent, FOLDER_PICKER_RESULTCODE);
            });
        }

        @JavascriptInterface
        public void openExternalUrl(final String url) {
            if (url == null || url.isEmpty()) return;

            runOnUiThread(() -> {
                try {
                    Intent browserIntent = new Intent(Intent.ACTION_VIEW, Uri.parse(url));
                    browserIntent.addCategory(Intent.CATEGORY_BROWSABLE);
                    startActivity(browserIntent);
                } catch (ActivityNotFoundException e) {
                    Log.e("Manatan", "No browser found for OAuth URL", e);
                    Toast.makeText(WebviewActivity.this, "No browser available for Google sign-in", Toast.LENGTH_SHORT).show();
                } catch (Exception e) {
                    Log.e("Manatan", "Failed to open OAuth URL in external browser", e);
                }
            });
        }

        @JavascriptInterface
        public void captureFrame(final String callbackId, final String payloadJson) {
            if (myWebView == null) {
                sendCaptureCallback(callbackId, null);
                return;
            }

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
                Log.e("Manatan", "Failed to parse capture payload", e);
            }

            final float cropX = x;
            final float cropY = y;
            final float cropWidth = width;
            final float cropHeight = height;
            final float devicePixelRatio = dpr;
            final float qualityFinal = quality;

            runOnUiThread(() -> {
                try {
                    int widthPx = Math.max(1, myWebView.getWidth());
                    int heightPx = Math.max(1, myWebView.getHeight());
                    Bitmap bitmap = Bitmap.createBitmap(widthPx, heightPx, Bitmap.Config.ARGB_8888);
                    Handler handler = new Handler(Looper.getMainLooper());

                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                        PixelCopy.request(getWindow(), bitmap, result -> {
                            if (result != PixelCopy.SUCCESS) {
                                Log.e("Manatan", "PixelCopy failed: " + result);
                                bitmap.recycle();
                                sendCaptureCallback(callbackId, null);
                                return;
                            }

                            Bitmap cropped = cropBitmap(bitmap, cropX, cropY, cropWidth, cropHeight, devicePixelRatio);
                            String dataUrl = encodeBitmapToDataUrl(cropped, qualityFinal);
                            if (cropped != bitmap) {
                                cropped.recycle();
                            }
                            bitmap.recycle();
                            sendCaptureCallback(callbackId, dataUrl);
                        }, handler);
                    } else {
                        Canvas canvas = new Canvas(bitmap);
                        myWebView.draw(canvas);
                        Bitmap cropped = cropBitmap(bitmap, cropX, cropY, cropWidth, cropHeight, devicePixelRatio);
                        String dataUrl = encodeBitmapToDataUrl(cropped, qualityFinal);
                        if (cropped != bitmap) {
                            cropped.recycle();
                        }
                        bitmap.recycle();
                        sendCaptureCallback(callbackId, dataUrl);
                    }
                } catch (Exception e) {
                    Log.e("Manatan", "Native capture failed", e);
                    sendCaptureCallback(callbackId, null);
                }
            });
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
                    Log.e("Manatan", "Failed to launch save dialog", e);
                    clearPendingExport();
                    Toast.makeText(WebviewActivity.this, "No document provider is available", Toast.LENGTH_SHORT).show();
                }
            });
        }
    }

    private void setImmersiveMode(boolean enabled) {
        int flags = View.SYSTEM_UI_FLAG_LAYOUT_STABLE;
        if (enabled) {
            flags |= View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                    | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                    | View.SYSTEM_UI_FLAG_FULLSCREEN
                    | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                    | View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY;
        }
        getWindow().getDecorView().setSystemUiVisibility(flags);
    }

    private void sendFolderCallback(String callbackId, String uri) {
        if (myWebView == null || callbackId == null) return;
        String script = "window.mangaReaderFolderPicked && window.mangaReaderFolderPicked("
                + JSONObject.quote(callbackId) + ","
                + (uri == null ? "null" : JSONObject.quote(uri)) + ");";
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

    private void clearPendingExport() {
        pendingExportFilename = null;
        pendingExportMimeType = null;
        pendingExportContent = null;
    }

    private void launchReader(String url) {
        Intent webIntent = new Intent(WebviewActivity.this, WebviewActivity.class);
        webIntent.putExtra("TARGET_URL", url);
        webIntent.putExtra("FROM_SHIM", false); // Native launch
        startActivity(webIntent);
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

        Log.w("MangaReader", "Embedded server unavailable; recreating runtime host");
        Intent recoveryIntent = new Intent(this, MangatanActivity.class);
        recoveryIntent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TASK);
        startActivity(recoveryIntent);
    }

    private void handleBackNavigation() {
        long now = android.os.SystemClock.elapsedRealtime();
        if (now - lastBackNavigationAt < 750L) return;
        lastBackNavigationAt = now;

        if (myWebView == null) {
            handleSmartClose();
            return;
        }
        try {
            myWebView.evaluateJavascript(
                "(() => {"
                    + "if (window.__handleNativeBack && window.__handleNativeBack()) return true;"
                    + "if (window.location.pathname.startsWith('/fork/reader/')) {"
                    + "window.location.assign('/library'); return true;"
                    + "}"
                    + "return false;"
                    + "})()",
                value -> {
                    if ("true".equals(value)) return;
                    if (myWebView.canGoBack()) myWebView.goBack();
                    else handleSmartClose();
                });
        } catch (Exception error) {
            if (myWebView.canGoBack()) myWebView.goBack();
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
    }

    @Override
    protected void onResume() {
        super.onResume();
        visible = true;
        if (!MangatanActivity.isRuntimeHostAlive()) {
            requestRuntimeRecovery();
            return;
        }
        if (myWebView != null) {
            myWebView.onResume();
            myWebView.resumeTimers();
        }
    }

    @Override
    protected void onPause() {
        if (myWebView != null) {
            myWebView.onPause();
            myWebView.pauseTimers();
        }
        super.onPause();
    }

    @Override
    protected void onStop() {
        visible = false;
        super.onStop();
    }

    private void destroyWebView(WebView view) {
        if (view == null) return;
        try {
            view.stopLoading();
            view.onPause();
            view.removeJavascriptInterface("ManatanNative");
            view.removeJavascriptInterface("MangaReaderNative");
            ViewParent parent = view.getParent();
            if (parent instanceof ViewGroup) ((ViewGroup) parent).removeView(view);
            view.removeAllViews();
            view.destroy();
        } catch (Exception error) {
            Log.e("MangaReader", "Could not fully release WebView", error);
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
            if (resultCode != RESULT_OK || data == null || data.getData() == null) {
                clearPendingExport();
                return;
            }

            Uri uri = data.getData();
            try (OutputStream outputStream = getContentResolver().openOutputStream(uri)) {
                if (outputStream == null) {
                    throw new IllegalStateException("No output stream returned for export URI");
                }

                byte[] bytes = (pendingExportContent == null ? "" : pendingExportContent).getBytes(StandardCharsets.UTF_8);
                outputStream.write(bytes);
                outputStream.flush();

                String savedName = pendingExportFilename != null ? pendingExportFilename : "file";
                Toast.makeText(this, "Saved " + savedName, Toast.LENGTH_SHORT).show();
            } catch (Exception e) {
                Log.e("Manatan", "Failed to write exported file", e);
                Toast.makeText(this, "Failed to save file", Toast.LENGTH_SHORT).show();
            } finally {
                clearPendingExport();
            }
        } else if (requestCode == FOLDER_PICKER_RESULTCODE) {
            String callbackId = pendingFolderCallbackId;
            pendingFolderCallbackId = null;
            if (resultCode != RESULT_OK || data == null || data.getData() == null) {
                sendFolderCallback(callbackId, null);
                return;
            }
            Uri uri = data.getData();
            int flags = data.getFlags() & (Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
            try {
                getContentResolver().takePersistableUriPermission(uri, flags & Intent.FLAG_GRANT_READ_URI_PERMISSION);
                sendFolderCallback(callbackId, uri.toString());
            } catch (SecurityException error) {
                Log.e("MangaReader", "Could not persist folder permission", error);
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
        }
    }
}
