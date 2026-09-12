package dev.local.mangareader;

import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Paint;
import android.graphics.Rect;

import com.google.android.gms.tasks.Tasks;
import com.google.mlkit.vision.common.InputImage;
import com.google.mlkit.vision.text.Text;
import com.google.mlkit.vision.text.TextRecognition;
import com.google.mlkit.vision.text.TextRecognizer;
import com.google.mlkit.vision.text.japanese.JapaneseTextRecognizerOptions;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.concurrent.TimeUnit;

/** JNI bridge for the bundled, on-device Japanese ML Kit recognizer. */
public final class OnDeviceOcr {
    private OnDeviceOcr() {}

    public static String recognizeJapanese(byte[] encodedImage) throws Exception {
        Bitmap bitmap = BitmapFactory.decodeByteArray(encodedImage, 0, encodedImage.length);
        if (bitmap == null) throw new IllegalArgumentException("The page image could not be decoded");
        try {
            return recognize(bitmap);
        } finally {
            bitmap.recycle();
        }
    }

    public static String selfTestJapanese() throws Exception {
        Bitmap bitmap = Bitmap.createBitmap(1200, 480, Bitmap.Config.ARGB_8888);
        Canvas canvas = new Canvas(bitmap);
        canvas.drawColor(Color.WHITE);
        Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
        paint.setColor(Color.BLACK);
        paint.setTextSize(132f);
        canvas.drawText("日本語テスト", 90f, 285f, paint);
        try {
            return recognize(bitmap);
        } finally {
            bitmap.recycle();
        }
    }

    private static String recognize(Bitmap bitmap) throws Exception {
        int width = bitmap.getWidth();
        int height = bitmap.getHeight();
        if (width <= 0 || height <= 0) throw new IllegalArgumentException("The page has invalid dimensions");

        TextRecognizer recognizer = TextRecognition.getClient(
                new JapaneseTextRecognizerOptions.Builder().build());
        try {
            Text result = Tasks.await(
                    recognizer.process(InputImage.fromBitmap(bitmap, 0)),
                    120,
                    TimeUnit.SECONDS);
            JSONArray blocks = new JSONArray();
            for (Text.TextBlock block : result.getTextBlocks()) {
                Rect box = block.getBoundingBox();
                String value = block.getText().replaceAll("\\s+", "");
                if (box == null || value.trim().isEmpty()) continue;
                JSONObject bounds = new JSONObject();
                bounds.put("x", clamp((double) box.left / width));
                bounds.put("y", clamp((double) box.top / height));
                bounds.put("width", clamp((double) box.width() / width));
                bounds.put("height", clamp((double) box.height() / height));
                JSONObject item = new JSONObject();
                item.put("text", value);
                item.put("tightBoundingBox", bounds);
                item.put("isMerged", true);
                item.put("forcedOrientation", box.height() > box.width() ? "vertical" : "horizontal");
                blocks.put(item);
            }
            return blocks.toString();
        } finally {
            recognizer.close();
        }
    }

    private static double clamp(double value) {
        return Math.max(0.0, Math.min(1.0, value));
    }
}
