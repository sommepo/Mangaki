package dev.local.mangareader;

import android.app.NativeActivity;
import android.content.Intent;
import android.net.Uri;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import android.widget.Toast;

public class MangatanActivity extends NativeActivity {
    private static volatile boolean runtimeHostAlive;
    private final Handler webviewHandler = new Handler(Looper.getMainLooper());
    private final Runnable ensureWebviewVisible = () -> {
        if (!isFinishing() && !WebviewActivity.isVisible()) {
            launchMainWebview();
        }
    };

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        runtimeHostAlive = true;
        handleLaunchIntent(getIntent());
        AnkiBridge.startAnkiConnectServer(getApplicationContext());
    }

    public static boolean isRuntimeHostAlive() {
        return runtimeHostAlive;
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        if (!handleLaunchIntent(intent)) {
            launchMainWebview();
        }
    }

    @Override
    protected void onResume() {
        super.onResume();
        webviewHandler.removeCallbacks(ensureWebviewVisible);
        webviewHandler.postDelayed(ensureWebviewVisible, 250);
    }

    @Override
    protected void onPause() {
        webviewHandler.removeCallbacks(ensureWebviewVisible);
        super.onPause();
    }

    private void launchMainWebview() {
        Intent webIntent = new Intent(this, WebviewActivity.class);
        String recoveryUrl = getSharedPreferences("runtime_recovery", MODE_PRIVATE)
                .getString("target_url", null);
        if (recoveryUrl != null) {
            getSharedPreferences("runtime_recovery", MODE_PRIVATE)
                    .edit()
                    .remove("target_url")
                    .apply();
        }
        webIntent.putExtra("TARGET_URL", recoveryUrl == null ? "http://127.0.0.1:14568" : recoveryUrl);
        webIntent.addFlags(Intent.FLAG_ACTIVITY_REORDER_TO_FRONT);
        startActivity(webIntent);
    }

    private boolean handleLaunchIntent(Intent intent) {
        if (intent == null || intent.getData() == null) return false;
        
        Uri data = intent.getData();
        if ("mangareaderfork".equals(data.getScheme())
                && "launch".equals(data.getHost())) {
            String targetUrl = data.getQueryParameter("url");
            
            if (targetUrl != null && !targetUrl.isEmpty()) {
                Log.i("Manatan", "🚀 Shim Launch: " + targetUrl);
                Toast.makeText(this, "Opening Reader...", Toast.LENGTH_SHORT).show();
                
                Intent webIntent = new Intent(this, WebviewActivity.class);
                webIntent.putExtra("TARGET_URL", targetUrl);
                
                // --- CRITICAL FLAG ---
                // Tells WebviewActivity this came from the Shim, so it should minimize on close
                webIntent.putExtra("FROM_SHIM", true); 
                
                webIntent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TOP);
                startActivity(webIntent);
                return true;
            }
        }
        return false;
    }

    @Override
    public void onDestroy() {
        webviewHandler.removeCallbacks(ensureWebviewVisible);
        runtimeHostAlive = false;
        super.onDestroy();
    }
}
