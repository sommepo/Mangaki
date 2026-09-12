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

public final class JobForegroundService extends Service {
    public static final String ACTION_UPDATE = "dev.local.mangareader.action.UPDATE_JOBS";
    public static final String ACTION_STOP = "dev.local.mangareader.action.STOP_JOBS";
    public static final String EXTRA_TITLE = "title";
    public static final String EXTRA_COMPLETED = "completed";
    public static final String EXTRA_TOTAL = "total";
    private static final String CHANNEL_ID = "manga_reader_background_jobs";
    private static final int NOTIFICATION_ID = 14569;

    public static void update(Context context, String title, int completed, int total) {
        Intent intent = new Intent(context, JobForegroundService.class)
                .setAction(ACTION_UPDATE)
                .putExtra(EXTRA_TITLE, title)
                .putExtra(EXTRA_COMPLETED, completed)
                .putExtra(EXTRA_TOTAL, total);
        context.startForegroundService(intent);
    }

    public static void stop(Context context) {
        context.startService(new Intent(context, JobForegroundService.class).setAction(ACTION_STOP));
    }

    @Override
    public void onCreate() {
        super.onCreate();
        NotificationManager manager = getSystemService(NotificationManager.class);
        NotificationChannel channel = new NotificationChannel(
                CHANNEL_ID,
                "OCR and offline downloads",
                NotificationManager.IMPORTANCE_LOW);
        channel.setDescription("Progress for manga OCR and offline page downloads");
        manager.createNotificationChannel(channel);
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent != null && ACTION_STOP.equals(intent.getAction())) {
            stopForeground(STOP_FOREGROUND_REMOVE);
            stopSelf();
            return START_NOT_STICKY;
        }

        String title = intent == null ? null : intent.getStringExtra(EXTRA_TITLE);
        int completed = intent == null ? 0 : intent.getIntExtra(EXTRA_COMPLETED, 0);
        int total = intent == null ? 0 : intent.getIntExtra(EXTRA_TOTAL, 0);
        Notification notification = buildNotification(title, completed, total);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC);
        } else {
            startForeground(NOTIFICATION_ID, notification);
        }
        return START_NOT_STICKY;
    }

    private Notification buildNotification(String title, int completed, int total) {
        Intent openIntent = new Intent(this, MangatanActivity.class)
                .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP | Intent.FLAG_ACTIVITY_SINGLE_TOP);
        PendingIntent pendingIntent = PendingIntent.getActivity(
                this,
                0,
                openIntent,
                PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
        String safeTitle = title == null || title.isEmpty() ? "Mangaki background work" : title;
        Notification.Builder builder = new Notification.Builder(this, CHANNEL_ID)
                .setSmallIcon(android.R.drawable.stat_sys_download)
                .setContentTitle(safeTitle)
                .setContentText(total > 0 ? completed + " / " + total + " pages" : completed + " pages")
                .setContentIntent(pendingIntent)
                .setOnlyAlertOnce(true)
                .setOngoing(true)
                .setCategory(Notification.CATEGORY_PROGRESS);
        if (total > 0) builder.setProgress(total, Math.min(completed, total), false);
        else builder.setProgress(0, 0, true);
        return builder.build();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }
}
