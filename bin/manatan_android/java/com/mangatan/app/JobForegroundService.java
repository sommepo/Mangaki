package dev.local.mangareader;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.IBinder;
import android.util.Log;

/**
 * User-visible progress for OCR and offline-download jobs (Android "dataSync" foreground service).
 *
 * <p>The first update starts the service; later progress ticks only re-post the notification, so
 * no further start requests are made while the app may be in the background (Android 12+ rejects
 * those). Android 15 limits dataSync services to six hours a day; {@link #onTimeout(int, int)}
 * stops cleanly instead of letting the system crash the app. Work itself lives in the Rust job
 * store and resumes from its checkpoints when the app is reopened.
 */
public final class JobForegroundService extends Service {
    public static final String ACTION_UPDATE = "dev.local.mangareader.action.UPDATE_JOBS";
    public static final String ACTION_STOP = "dev.local.mangareader.action.STOP_JOBS";
    public static final String EXTRA_TITLE = "title";
    public static final String EXTRA_COMPLETED = "completed";
    public static final String EXTRA_TOTAL = "total";
    private static final String CHANNEL_ID = "manga_reader_background_jobs";
    private static final int NOTIFICATION_ID = 14569;
    private static final String TAG = "MangaReaderJobs";

    /** True between a successful startForeground() and onDestroy(). */
    private static volatile boolean foreground;
    /** True after startForegroundService() until the service has entered the foreground. */
    private static volatile boolean startPending;

    public static void update(Context context, String title, int completed, int total) {
        Context app = context.getApplicationContext();
        if (foreground) {
            NotificationManager manager = app.getSystemService(NotificationManager.class);
            if (manager != null) {
                ensureChannel(manager);
                manager.notify(NOTIFICATION_ID, buildNotification(app, title, completed, total));
            }
            return;
        }
        Intent intent = new Intent(app, JobForegroundService.class)
                .setAction(ACTION_UPDATE)
                .putExtra(EXTRA_TITLE, title)
                .putExtra(EXTRA_COMPLETED, completed)
                .putExtra(EXTRA_TOTAL, total);
        startPending = true;
        try {
            app.startForegroundService(intent);
        } catch (RuntimeException error) {
            // e.g. ForegroundServiceStartNotAllowedException while backgrounded (Android 12+).
            startPending = false;
            throw error;
        }
    }

    public static void stop(Context context) {
        // Never start a service just to stop it: an idle app must not create one, and a
        // background startService() throws IllegalStateException on Android 8+.
        if (!foreground && !startPending) return;
        Context app = context.getApplicationContext();
        try {
            app.startService(new Intent(app, JobForegroundService.class).setAction(ACTION_STOP));
        } catch (RuntimeException error) {
            app.stopService(new Intent(app, JobForegroundService.class));
        }
    }

    @Override
    public void onCreate() {
        super.onCreate();
        NotificationManager manager = getSystemService(NotificationManager.class);
        if (manager != null) ensureChannel(manager);
    }

    private static void ensureChannel(NotificationManager manager) {
        if (manager.getNotificationChannel(CHANNEL_ID) != null) return;
        NotificationChannel channel = new NotificationChannel(
                CHANNEL_ID,
                "OCR and offline downloads",
                NotificationManager.IMPORTANCE_LOW);
        channel.setDescription("Progress for manga OCR and offline page downloads");
        channel.setShowBadge(false);
        manager.createNotificationChannel(channel);
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        boolean stopRequested = intent != null && ACTION_STOP.equals(intent.getAction());
        if (stopRequested && !startPending) {
            stopNow();
            return START_NOT_STICKY;
        }

        String title = intent == null ? null : intent.getStringExtra(EXTRA_TITLE);
        int completed = intent == null ? 0 : intent.getIntExtra(EXTRA_COMPLETED, 0);
        int total = intent == null ? 0 : intent.getIntExtra(EXTRA_TOTAL, 0);
        try {
            // A startForegroundService() call must always be answered with startForeground(),
            // even when a stop request overtook it, or Android raises an ANR-style crash.
            Notification notification = buildNotification(this, title, completed, total);
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC);
            } else {
                startForeground(NOTIFICATION_ID, notification);
            }
            foreground = true;
        } catch (RuntimeException error) {
            // Android 14+ refuses dataSync when not allowed (e.g. the daily limit was used up).
            Log.w(TAG, "Could not enter the foreground for background jobs", error);
            stopRequested = true;
        } finally {
            startPending = false;
        }
        if (stopRequested) stopNow();
        // Not sticky: after process death the job store resumes when the app is opened again.
        return START_NOT_STICKY;
    }

    /** Android 15+: the dataSync time budget is exhausted; the service must stop within seconds. */
    @Override
    public void onTimeout(int startId, int fgsType) {
        Log.w(TAG, "Background-job time limit reached; progress resumes when Mangaki is reopened");
        stopNow();
    }

    private void stopNow() {
        foreground = false;
        stopForeground(STOP_FOREGROUND_REMOVE);
        stopSelf();
    }

    @Override
    public void onDestroy() {
        foreground = false;
        startPending = false;
        super.onDestroy();
    }

    private static Notification buildNotification(Context context, String title, int completed, int total) {
        // Bring the existing task to the front exactly as it was (reader page included) rather
        // than clearing it, which CLEAR_TOP on the runtime host would do.
        Intent openIntent = new Intent(context, WebviewActivity.class)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_REORDER_TO_FRONT);
        PendingIntent pendingIntent = PendingIntent.getActivity(
                context,
                0,
                openIntent,
                PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
        String safeTitle = title == null || title.isEmpty() ? "Mangaki background work" : title;
        int boundedCompleted = Math.max(0, completed);
        Notification.Builder builder = new Notification.Builder(context, CHANNEL_ID)
                .setSmallIcon(android.R.drawable.stat_sys_download)
                .setContentTitle(safeTitle)
                .setContentText(total > 0 ? boundedCompleted + " / " + total + " pages" : boundedCompleted + " pages")
                .setContentIntent(pendingIntent)
                .setOnlyAlertOnce(true)
                .setOngoing(true)
                .setShowWhen(false)
                .setCategory(Notification.CATEGORY_PROGRESS);
        if (total > 0) builder.setProgress(total, Math.min(boundedCompleted, total), false);
        else builder.setProgress(0, 0, true);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            builder.setForegroundServiceBehavior(Notification.FOREGROUND_SERVICE_IMMEDIATE);
        }
        return builder.build();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }
}
