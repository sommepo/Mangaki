package dev.local.mangareader;

import android.content.Context;
import android.content.Intent;
import android.content.res.AssetFileDescriptor;
import android.database.Cursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.DocumentsContract;
import android.util.Base64;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.Enumeration;
import java.util.List;
import java.util.Locale;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

/**
 * Read-only scanner for Storage Access Framework document trees.
 *
 * <p>JNI contract (called from fork_api.rs): {@link #scan}, {@link #readPage},
 * {@link #openArchive(Context, String)} and {@link #releaseGrant}. Keep their names and
 * signatures stable.
 *
 * <p>Traversal issues exactly one {@code ContentResolver.query} per directory with the
 * projection it needs (id, name, MIME type); it never uses {@code DocumentFile.listFiles()},
 * whose per-child {@code getName()}/{@code isDirectory()} calls each cost another provider IPC.
 */
public final class DocumentScanner {
    private static final int MAX_DEPTH = 8;
    private static final int MAX_DOCUMENTS = 100_000;
    private static final int MAX_PAGE_BYTES = 64 * 1024 * 1024;
    /** Sorts path separators before every printable character so "Vol 1/Ch 2" precedes "Vol 1 extra". */
    private static final String SORT_SEPARATOR = "\u0001";
    private static final String[] CHILD_PROJECTION = {
            DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME,
            DocumentsContract.Document.COLUMN_MIME_TYPE,
    };
    private static final char[] HEX = "0123456789abcdef".toCharArray();

    private DocumentScanner() {}

    private static final class Node {
        final String name;
        final Uri uri;
        final boolean directory;

        Node(String name, Uri uri, boolean directory) {
            this.name = name;
            this.uri = uri;
            this.directory = directory;
        }
    }

    private static final class Volume {
        final String title;
        final String locator;
        final List<String> pages;
        final String archiveUri;
        /** Relative path inside the series, used to keep nested chapters in reading order. */
        final String sortKey;

        Volume(String title, String locator, List<String> pages, String archiveUri, String sortKey) {
            this.title = title;
            this.locator = locator;
            this.pages = pages;
            this.archiveUri = archiveUri;
            this.sortKey = sortKey;
        }
    }

    public static String scan(Context context, String treeUriValue) throws Exception {
        try {
            return scanTree(context, Uri.parse(treeUriValue));
        } catch (SecurityException error) {
            // A revoked grant is not an empty library: report it so the UI can offer re-authorisation.
            throw new SecurityException("Folder access is no longer granted; reconnect this folder", error);
        }
    }

    private static String scanTree(Context context, Uri treeUri) throws Exception {
        String rootId = DocumentsContract.getTreeDocumentId(treeUri);
        Uri rootUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, rootId);
        String rootName = displayName(context, rootUri, "Manga");
        int[] visited = new int[]{0};
        List<Node> children = children(context, treeUri, rootId, visited);
        JSONArray series = new JSONArray();

        List<Volume> rootVolumes = volumesIn(context, treeUri, rootUri, rootName, children, visited, 0, false, "");
        if (!rootVolumes.isEmpty()) {
            series.put(seriesJson(rootName, rootUri.toString(), rootVolumes));
        }

        for (Node child : children) {
            if (!child.directory) continue;
            String childId = DocumentsContract.getDocumentId(child.uri);
            List<Node> descendants = children(context, treeUri, childId, visited);
            List<Volume> volumes = volumesIn(context, treeUri, child.uri, child.name, descendants, visited, 1, true, "");
            if (!volumes.isEmpty()) {
                series.put(seriesJson(child.name, child.uri.toString(), volumes));
            }
        }

        return new JSONObject().put("series", series).toString();
    }

    private static JSONObject seriesJson(String title, String locator, List<Volume> volumes) throws Exception {
        volumes.sort((left, right) -> naturalCompare(left.sortKey, right.sortKey));
        JSONArray output = new JSONArray();
        for (Volume volume : volumes) {
            JSONArray pages = new JSONArray();
            for (String page : volume.pages) pages.put(page);
            JSONObject item = new JSONObject()
                    .put("id", stableId(volume.locator))
                    .put("title", volume.title)
                    .put("pageCount", volume.pages.size())
                    .put("fingerprint", stableId(volume.locator + ":" + volume.pages.size()))
                    .put("pages", pages);
            if (volume.archiveUri != null) item.put("archiveUri", volume.archiveUri);
            output.put(item);
        }
        return new JSONObject()
                .put("id", stableId(locator))
                .put("title", title)
                .put("volumes", output);
    }

    /**
     * @param parentName display name of {@code parentUri}, already known from the parent listing
     *                   (avoids one provider query per directory).
     * @param prefix     path of intermediate folders below the series folder ("" at series level).
     */
    private static List<Volume> volumesIn(Context context, Uri treeUri, Uri parentUri, String parentName,
                                          List<Node> nodes, int[] visited, int depth,
                                          boolean traverseDirectories, String prefix) throws Exception {
        if (depth > MAX_DEPTH) return Collections.emptyList();
        List<Volume> volumes = new ArrayList<>();
        List<String> directPages = new ArrayList<>();
        for (Node node : nodes) {
            if (node.directory) continue;
            if (isImage(node.name)) {
                directPages.add("content:" + node.uri);
            } else if (isArchive(node.name)) {
                volumes.add(new Volume(qualify(prefix, stripExtension(node.name)), node.uri.toString(),
                        Collections.emptyList(), node.uri.toString(), sortKey(prefix, node.name)));
            }
        }
        if (!directPages.isEmpty()) {
            volumes.add(new Volume(parentName, parentUri.toString(), directPages, null, sortKey(prefix, "")));
        }
        if (!traverseDirectories) return volumes;
        for (Node node : nodes) {
            if (!node.directory) continue;
            List<Node> childNodes = children(context, treeUri, DocumentsContract.getDocumentId(node.uri), visited);
            List<String> pages = new ArrayList<>();
            for (Node child : childNodes) if (!child.directory && isImage(child.name)) pages.add("content:" + child.uri);
            if (!pages.isEmpty()) {
                volumes.add(new Volume(qualify(prefix, node.name), node.uri.toString(), pages, null,
                        sortKey(prefix, node.name)));
            } else {
                volumes.addAll(volumesIn(context, treeUri, node.uri, node.name, childNodes, visited,
                        depth + 1, true, qualify(prefix, node.name)));
            }
        }
        return volumes;
    }

    /** Nested chapters keep their folder context ("Vol 01 / Ch 001") so equal chapter names stay distinguishable. */
    private static String qualify(String prefix, String name) {
        return prefix.isEmpty() ? name : prefix + " / " + name;
    }

    private static String sortKey(String prefix, String name) {
        String path = prefix.replace(" / ", SORT_SEPARATOR);
        if (path.isEmpty()) return name;
        return name.isEmpty() ? path : path + SORT_SEPARATOR + name;
    }

    private static List<Node> children(Context context, Uri treeUri, String parentId, int[] visited) throws Exception {
        if (visited[0] > MAX_DOCUMENTS) throw new IllegalStateException("Folder contains too many documents");
        Uri childrenUri = DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, parentId);
        List<Node> result = new ArrayList<>();
        try (Cursor cursor = context.getContentResolver().query(childrenUri, CHILD_PROJECTION, null, null, null)) {
            if (cursor == null) throw new IllegalStateException("Folder permission is unavailable");
            while (cursor.moveToNext()) {
                if (++visited[0] > MAX_DOCUMENTS) throw new IllegalStateException("Folder contains too many documents");
                String id = cursor.getString(0);
                if (id == null) continue;
                String name = cursor.getString(1);
                if (isIgnoredName(name)) continue;
                String mime = cursor.getString(2);
                result.add(new Node(name == null ? "Untitled" : name,
                        DocumentsContract.buildDocumentUriUsingTree(treeUri, id),
                        DocumentsContract.Document.MIME_TYPE_DIR.equals(mime)));
            }
        }
        result.sort((left, right) -> naturalCompare(left.name, right.name));
        return result;
    }

    /** Hidden files (".DS_Store", macOS "._001.jpg" resource forks, ".nomedia") and "__MACOSX" are never pages. */
    private static boolean isIgnoredName(String name) {
        return name != null && (name.startsWith(".") || "__MACOSX".equals(name));
    }

    private static List<String> archivePages(Context context, Uri uri) throws Exception {
        List<String> names = new ArrayList<>();
        try (ParcelFileDescriptor descriptor = openArchive(context, uri);
             ZipFile zip = new ZipFile(new File("/proc/self/fd/" + descriptor.getFd()))) {
            Enumeration<? extends ZipEntry> entries = zip.entries();
            while (entries.hasMoreElements()) {
                ZipEntry entry = entries.nextElement();
                if (!entry.isDirectory() && isImage(entry.getName())) names.add(entry.getName());
                if (names.size() > MAX_DOCUMENTS) throw new IllegalStateException("Archive contains too many pages");
            }
        }
        names.sort(DocumentScanner::naturalCompare);
        String encodedUri = encode(uri.toString());
        List<String> result = new ArrayList<>();
        for (String name : names) result.add("zip:" + encodedUri + ":" + encode(name));
        return result;
    }

    public static byte[] readPage(Context context, String locator) throws Exception {
        if (locator.startsWith("content:")) {
            return readDocument(context, Uri.parse(locator.substring(8)));
        }
        if (!locator.startsWith("zip:")) throw new IllegalArgumentException("Unsupported page locator");
        String[] parts = locator.substring(4).split(":", 2);
        if (parts.length != 2) throw new IllegalArgumentException("Invalid archive page locator");
        Uri archive = Uri.parse(decode(parts[0]));
        String wanted = decode(parts[1]);
        try (ParcelFileDescriptor descriptor = openArchive(context, archive);
             ZipFile zip = new ZipFile(new File("/proc/self/fd/" + descriptor.getFd()))) {
            ZipEntry entry = zip.getEntry(wanted);
            if (entry != null && !entry.isDirectory()) {
                if (entry.getSize() > MAX_PAGE_BYTES) throw new IllegalStateException("Page exceeds 64 MiB safety limit");
                return readBounded(zip.getInputStream(entry), entry.getSize());
            }
        }
        throw new IllegalStateException("Archive page no longer exists");
    }

    public static int openArchive(Context context, String uriValue) throws Exception {
        ParcelFileDescriptor descriptor = openArchive(context, Uri.parse(uriValue));
        return descriptor.detachFd();
    }

    public static void releaseGrant(Context context, String uriValue) {
        try {
            context.getContentResolver().releasePersistableUriPermission(
                    Uri.parse(uriValue), Intent.FLAG_GRANT_READ_URI_PERMISSION);
        } catch (SecurityException alreadyReleased) {
            // Removing a source must stay idempotent: the grant may already have been revoked
            // by the user or the provider, which is exactly the state being requested.
        }
    }

    private static ParcelFileDescriptor openArchive(Context context, Uri uri) throws Exception {
        ParcelFileDescriptor descriptor = context.getContentResolver().openFileDescriptor(uri, "r");
        if (descriptor == null) throw new IllegalStateException("Archive is unavailable");
        return descriptor;
    }

    /**
     * Reads one page document. When the provider exposes a real file the page is read straight into
     * an exactly sized array (one copy instead of the three a growing ByteArrayOutputStream needs).
     */
    private static byte[] readDocument(Context context, Uri uri) throws Exception {
        AssetFileDescriptor asset = context.getContentResolver().openAssetFileDescriptor(uri, "r");
        if (asset == null) throw new IllegalStateException("Page is unavailable");
        try (AssetFileDescriptor ignored = asset; FileInputStream input = asset.createInputStream()) {
            long length = asset.getLength();
            if (length == AssetFileDescriptor.UNKNOWN_LENGTH) {
                length = asset.getParcelFileDescriptor().getStatSize();
            }
            return readBounded(input, length);
        }
    }

    /** @param expectedLength the size when known, or a negative value for streams of unknown size. */
    private static byte[] readBounded(InputStream input, long expectedLength) throws Exception {
        if (input == null) throw new IllegalStateException("Page is unavailable");
        if (expectedLength > MAX_PAGE_BYTES) {
            input.close();
            throw new IllegalStateException("Page exceeds 64 MiB safety limit");
        }
        try (InputStream source = input) {
            if (expectedLength < 0) return copyBounded(source, new ByteArrayOutputStream(256 * 1024), 0);
            byte[] output = new byte[(int) expectedLength];
            int total = 0;
            while (total < output.length) {
                int read = source.read(output, total, output.length - total);
                if (read < 0) return Arrays.copyOf(output, total);
                total += read;
            }
            int next = source.read();
            if (next < 0) return output;
            // The document grew after it was measured; finish with the bounded streaming path.
            ByteArrayOutputStream rest = new ByteArrayOutputStream(output.length + 64 * 1024);
            rest.write(output, 0, total);
            rest.write(next);
            return copyBounded(source, rest, total + 1L);
        }
    }

    private static byte[] copyBounded(InputStream source, ByteArrayOutputStream output, long alreadyRead) throws Exception {
        if (alreadyRead > MAX_PAGE_BYTES) throw new IllegalStateException("Page exceeds 64 MiB safety limit");
        byte[] buffer = new byte[64 * 1024];
        long total = alreadyRead;
        int read;
        while ((read = source.read(buffer)) >= 0) {
            total += read;
            if (total > MAX_PAGE_BYTES) throw new IllegalStateException("Page exceeds 64 MiB safety limit");
            output.write(buffer, 0, read);
        }
        return output.toByteArray();
    }

    private static String displayName(Context context, Uri uri, String fallback) {
        try (Cursor cursor = context.getContentResolver().query(uri,
                new String[]{DocumentsContract.Document.COLUMN_DISPLAY_NAME}, null, null, null)) {
            if (cursor != null && cursor.moveToFirst() && cursor.getString(0) != null) return cursor.getString(0);
        } catch (Exception ignored) {}
        return fallback;
    }

    private static boolean isImage(String name) {
        String value = name.toLowerCase(Locale.ROOT);
        return value.endsWith(".jpg") || value.endsWith(".jpeg") || value.endsWith(".png") || value.endsWith(".webp");
    }

    private static boolean isArchive(String name) {
        String value = name.toLowerCase(Locale.ROOT);
        return value.endsWith(".cbz") || value.endsWith(".zip");
    }

    private static String stripExtension(String name) {
        int dot = name.lastIndexOf('.');
        return dot > 0 ? name.substring(0, dot) : name;
    }

    private static String stableId(String value) throws Exception {
        byte[] digest = MessageDigest.getInstance("SHA-256").digest(value.getBytes(StandardCharsets.UTF_8));
        char[] output = new char[32];
        for (int i = 0; i < 16; i++) {
            output[i * 2] = HEX[(digest[i] >> 4) & 0x0f];
            output[i * 2 + 1] = HEX[digest[i] & 0x0f];
        }
        return new String(output);
    }

    private static String encode(String value) {
        return Base64.encodeToString(value.getBytes(StandardCharsets.UTF_8), Base64.URL_SAFE | Base64.NO_WRAP | Base64.NO_PADDING);
    }

    private static String decode(String value) {
        return new String(Base64.decode(value, Base64.URL_SAFE | Base64.NO_WRAP | Base64.NO_PADDING), StandardCharsets.UTF_8);
    }

    /**
     * Natural, case-insensitive order ("Vol 9" before "Vol 10"). Digit runs compare by numeric
     * value, including full-width digits common in Japanese names. Allocation-free (no regex per
     * comparison) and a consistent total order: remaining characters, then leading-zero count, then
     * the raw string break ties, so TimSort never sees a contract violation.
     */
    static int naturalCompare(String left, String right) {
        int leftLength = left.length();
        int rightLength = right.length();
        int li = 0;
        int ri = 0;
        int zeroTieBreak = 0;
        while (li < leftLength && ri < rightLength) {
            char lc = left.charAt(li);
            char rc = right.charAt(ri);
            int ld = Character.digit(lc, 10);
            int rd = Character.digit(rc, 10);
            if (ld >= 0 && rd >= 0) {
                int leftEnd = li;
                while (leftEnd < leftLength && Character.digit(left.charAt(leftEnd), 10) >= 0) leftEnd++;
                int rightEnd = ri;
                while (rightEnd < rightLength && Character.digit(right.charAt(rightEnd), 10) >= 0) rightEnd++;
                int leftStart = li;
                while (leftStart < leftEnd - 1 && Character.digit(left.charAt(leftStart), 10) == 0) leftStart++;
                int rightStart = ri;
                while (rightStart < rightEnd - 1 && Character.digit(right.charAt(rightStart), 10) == 0) rightStart++;
                int significantLeft = leftEnd - leftStart;
                int significantRight = rightEnd - rightStart;
                if (significantLeft != significantRight) return significantLeft < significantRight ? -1 : 1;
                for (int k = 0; k < significantLeft; k++) {
                    int a = Character.digit(left.charAt(leftStart + k), 10);
                    int b = Character.digit(right.charAt(rightStart + k), 10);
                    if (a != b) return a < b ? -1 : 1;
                }
                if (zeroTieBreak == 0) zeroTieBreak = Integer.compare(leftEnd - li, rightEnd - ri);
                li = leftEnd;
                ri = rightEnd;
            } else {
                // A digit from any script compares as its ASCII form, so mixed scripts order
                // consistently against letters and punctuation.
                char a = ld >= 0 ? (char) ('0' + ld) : Character.toLowerCase(lc);
                char b = rd >= 0 ? (char) ('0' + rd) : Character.toLowerCase(rc);
                if (a != b) return a < b ? -1 : 1;
                li++;
                ri++;
            }
        }
        if (li < leftLength) return 1;
        if (ri < rightLength) return -1;
        if (zeroTieBreak != 0) return zeroTieBreak;
        return left.compareTo(right);
    }
}
