package dev.local.mangareader;

import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Paint;
import android.graphics.Rect;

import com.google.android.gms.tasks.Task;
import com.google.android.gms.tasks.Tasks;
import com.google.mlkit.vision.common.InputImage;
import com.google.mlkit.vision.text.Text;
import com.google.mlkit.vision.text.TextRecognition;
import com.google.mlkit.vision.text.TextRecognizer;
import com.google.mlkit.vision.text.japanese.JapaneseTextRecognizerOptions;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.concurrent.TimeUnit;
import java.util.regex.Pattern;

/**
 * JNI bridge for the bundled, on-device Japanese ML Kit recognizer.
 *
 * <p>Called from Rust worker threads (never the main thread), so the blocking
 * {@code Tasks.await} is safe here. JNI contract: {@link #recognizeJapanese(byte[])} and
 * {@link #selfTestJapanese()}.
 */
public final class OnDeviceOcr {
    /** Longest edge handed to ML Kit; larger scans are subsampled while decoding. */
    private static final int MAX_EDGE = 4096;
    /** ~16 MP keeps one decoded ARGB page at or below 64 MiB of native memory. */
    private static final long MAX_PIXELS = 16L * 1024 * 1024;
    private static final long TIMEOUT_SECONDS = 120;
    private static final Pattern WHITESPACE = Pattern.compile("\\s+");

    private static final Object RECOGNIZER_LOCK = new Object();
    private static TextRecognizer recognizer;

    private OnDeviceOcr() {}

    /**
     * Creating a recognizer loads the bundled model, which costs far more than recognising one
     * page, so a single process-wide client is reused. ML Kit queues concurrent process() calls.
     */
    private static TextRecognizer recognizer() {
        synchronized (RECOGNIZER_LOCK) {
            if (recognizer == null) {
                recognizer = TextRecognition.getClient(new JapaneseTextRecognizerOptions.Builder().build());
            }
            return recognizer;
        }
    }

    public static String recognizeJapanese(byte[] encodedImage) throws Exception {
        if (encodedImage == null || encodedImage.length == 0) {
            throw new IllegalArgumentException("The page image is empty");
        }
        BitmapFactory.Options bounds = new BitmapFactory.Options();
        bounds.inJustDecodeBounds = true;
        BitmapFactory.decodeByteArray(encodedImage, 0, encodedImage.length, bounds);
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) {
            throw new IllegalArgumentException("The page image could not be decoded");
        }
        BitmapFactory.Options options = new BitmapFactory.Options();
        options.inSampleSize = sampleSize(bounds.outWidth, bounds.outHeight);
        options.inPreferredConfig = Bitmap.Config.ARGB_8888;
        Bitmap bitmap = BitmapFactory.decodeByteArray(encodedImage, 0, encodedImage.length, options);
        if (bitmap == null) throw new IllegalArgumentException("The page image could not be decoded");
        return recognizeAndRelease(bitmap);
    }

    public static String selfTestJapanese() throws Exception {
        Bitmap bitmap = Bitmap.createBitmap(1200, 480, Bitmap.Config.ARGB_8888);
        Canvas canvas = new Canvas(bitmap);
        canvas.drawColor(Color.WHITE);
        Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
        paint.setColor(Color.BLACK);
        paint.setTextSize(132f);
        canvas.drawText("日本語テスト", 90f, 285f, paint);
        return recognizeAndRelease(bitmap);
    }

    /** Power-of-two subsampling: only oversized scans are reduced, ordinary pages decode at full size. */
    static int sampleSize(int width, int height) {
        int sample = 1;
        while (Math.max(width, height) / sample > MAX_EDGE
                || ((long) (width / sample)) * (height / sample) > MAX_PIXELS) {
            sample *= 2;
        }
        return sample;
    }

    private static String recognizeAndRelease(Bitmap bitmap) throws Exception {
        int width = bitmap.getWidth();
        int height = bitmap.getHeight();
        if (width <= 0 || height <= 0) {
            bitmap.recycle();
            throw new IllegalArgumentException("The page has invalid dimensions");
        }
        Task<Text> task = null;
        try {
            task = recognizer().process(InputImage.fromBitmap(bitmap, 0));
            Text result = Tasks.await(task, TIMEOUT_SECONDS, TimeUnit.SECONDS);
            return toJson(result, width, height);
        } finally {
            // After a timeout ML Kit may still be reading the pixels; recycling then would crash
            // natively, so an unfinished bitmap is left to the garbage collector instead.
            if (task == null || task.isComplete()) bitmap.recycle();
        }
    }

    private static String toJson(Text result, int width, int height) throws Exception {
        JSONArray blocks = new JSONArray();
        if (result == null) return blocks.toString();
        for (Text.TextBlock block : result.getTextBlocks()) {
            Rect box = block.getBoundingBox();
            String text = block.getText();
            if (box == null || text == null) continue;
            String value = WHITESPACE.matcher(text).replaceAll("");
            if (value.isEmpty()) continue;
            // Clamp both edges so a box that overhangs the page never yields x + width > 1.
            double left = clamp((double) box.left / width);
            double top = clamp((double) box.top / height);
            double right = clamp((double) box.right / width);
            double bottom = clamp((double) box.bottom / height);
            if (right <= left || bottom <= top) continue;
            JSONObject bounds = new JSONObject();
            bounds.put("x", left);
            bounds.put("y", top);
            bounds.put("width", right - left);
            bounds.put("height", bottom - top);
            JSONObject item = new JSONObject();
            item.put("text", value);
            item.put("tightBoundingBox", bounds);
            item.put("isMerged", true);
            item.put("forcedOrientation", box.height() > box.width() ? "vertical" : "horizontal");
            blocks.put(item);
        }
        return blocks.toString();
    }

    private static double clamp(double value) {
        return Math.max(0.0, Math.min(1.0, value));
    }
}
