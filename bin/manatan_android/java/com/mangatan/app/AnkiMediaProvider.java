package dev.local.mangareader;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.content.Context;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;

import java.io.File;
import java.io.FileNotFoundException;
import java.io.IOException;
import java.util.Locale;

/**
 * Minimal read-only FileProvider for handing mined media to AnkiDroid.
 *
 * <p>AnkiDroid's media API copies a file from a {@code content://} URI that the caller has
 * granted it read access to. Files live in this app's private cache, are shared through a
 * per-file grant, and are deleted as soon as AnkiDroid has copied them, so nothing is ever
 * written to shared storage (MediaStore), which also required Android 10+ for RELATIVE_PATH.
 */
public final class AnkiMediaProvider extends ContentProvider {
    static final String AUTHORITY = "dev.local.mangareader.ankimedia";
    private static final String DIRECTORY = "anki-media";

    static File directory(Context context) throws IOException {
        File dir = new File(context.getCacheDir(), DIRECTORY);
        if (!dir.isDirectory() && !dir.mkdirs()) throw new IOException("Could not create the media cache");
        return dir;
    }

    static Uri uriFor(File file) {
        return new Uri.Builder().scheme("content").authority(AUTHORITY).appendPath(file.getName()).build();
    }

    static boolean isAudioName(String name) {
        String lower = name.toLowerCase(Locale.ROOT);
        return lower.endsWith(".mp3") || lower.endsWith(".aac") || lower.endsWith(".m4a")
                || lower.endsWith(".wav") || lower.endsWith(".webm") || lower.endsWith(".ogg")
                || lower.endsWith(".flac");
    }

    /**
     * AnkiDroid derives the stored file's extension from this MIME type, so the mapping matches
     * the one previously written to MediaStore exactly.
     */
    static String mimeTypeFor(String name) {
        String lower = name.toLowerCase(Locale.ROOT);
        if (isAudioName(lower)) {
            if (lower.endsWith(".m4a")) return "audio/mp4";
            if (lower.endsWith(".wav")) return "audio/wav";
            if (lower.endsWith(".ogg")) return "audio/ogg";
            return "audio/mpeg";
        }
        if (lower.endsWith(".jpg") || lower.endsWith(".jpeg")) return "image/jpeg";
        if (lower.endsWith(".webp")) return "image/webp";
        return "image/png";
    }

    @Override
    public boolean onCreate() {
        return true;
    }

    private File resolve(Uri uri) throws FileNotFoundException {
        String name = uri.getLastPathSegment();
        if (name == null || name.isEmpty() || name.contains("/") || name.startsWith(".")
                || uri.getPathSegments().size() != 1) {
            throw new FileNotFoundException("Unknown media");
        }
        Context context = getContext();
        if (context == null) throw new FileNotFoundException("Provider is not attached");
        try {
            File file = new File(directory(context), name);
            if (!file.getCanonicalFile().getParentFile().equals(directory(context).getCanonicalFile())
                    || !file.isFile()) {
                throw new FileNotFoundException("Unknown media");
            }
            return file;
        } catch (IOException error) {
            throw new FileNotFoundException("Unknown media");
        }
    }

    @Override
    public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
        if (mode == null || !mode.equals("r")) throw new SecurityException("Media is read-only");
        return ParcelFileDescriptor.open(resolve(uri), ParcelFileDescriptor.MODE_READ_ONLY);
    }

    @Override
    public String getType(Uri uri) {
        String name = uri.getLastPathSegment();
        return name == null ? null : mimeTypeFor(name);
    }

    @Override
    public Cursor query(Uri uri, String[] projection, String selection, String[] selectionArgs, String sortOrder) {
        File file;
        try {
            file = resolve(uri);
        } catch (FileNotFoundException error) {
            return null;
        }
        String[] columns = projection != null ? projection
                : new String[]{OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE};
        MatrixCursor cursor = new MatrixCursor(columns, 1);
        Object[] row = new Object[columns.length];
        for (int i = 0; i < columns.length; i++) {
            if (OpenableColumns.DISPLAY_NAME.equals(columns[i])) row[i] = file.getName();
            else if (OpenableColumns.SIZE.equals(columns[i])) row[i] = file.length();
        }
        cursor.addRow(row);
        return cursor;
    }

    @Override
    public Uri insert(Uri uri, ContentValues values) {
        throw new UnsupportedOperationException("Media is read-only");
    }

    @Override
    public int delete(Uri uri, String selection, String[] selectionArgs) {
        throw new UnsupportedOperationException("Media is read-only");
    }

    @Override
    public int update(Uri uri, ContentValues values, String selection, String[] selectionArgs) {
        throw new UnsupportedOperationException("Media is read-only");
    }
}
