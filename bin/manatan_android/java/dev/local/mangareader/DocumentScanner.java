package dev.local.mangareader;

import android.content.ContentResolver;
import android.content.Context;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.DocumentsContract;
import android.util.Base64;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.Enumeration;
import java.util.List;
import java.util.Locale;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

/** Read-only scanner for Storage Access Framework document trees. */
public final class DocumentScanner {
    private static final int MAX_DEPTH = 8;
    private static final int MAX_DOCUMENTS = 100_000;
    private static final int MAX_PAGE_BYTES = 64 * 1024 * 1024;

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

        Volume(String title, String locator, List<String> pages, String archiveUri) {
            this.title = title;
            this.locator = locator;
            this.pages = pages;
            this.archiveUri = archiveUri;
        }
    }

    public static String scan(Context context, String treeUriValue) throws Exception {
        Uri treeUri = Uri.parse(treeUriValue);
        String rootId = DocumentsContract.getTreeDocumentId(treeUri);
        Uri rootUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, rootId);
        String rootName = displayName(context, rootUri, "Manga");
        int[] visited = new int[]{0};
        List<Node> children = children(context, treeUri, rootId, visited);
        JSONArray series = new JSONArray();

        List<Volume> rootVolumes = volumesIn(context, treeUri, rootUri, children, visited, 0, false);
        if (!rootVolumes.isEmpty()) {
            series.put(seriesJson(rootName, rootUri.toString(), rootVolumes));
        }

        for (Node child : children) {
            if (!child.directory) continue;
            String childId = DocumentsContract.getDocumentId(child.uri);
            List<Node> descendants = children(context, treeUri, childId, visited);
            List<Volume> volumes = volumesIn(context, treeUri, child.uri, descendants, visited, 1, true);
            if (!volumes.isEmpty()) {
                series.put(seriesJson(child.name, child.uri.toString(), volumes));
            }
        }

        return new JSONObject().put("series", series).toString();
    }

    private static JSONObject seriesJson(String title, String locator, List<Volume> volumes) throws Exception {
        volumes.sort((left, right) -> naturalCompare(left.title, right.title));
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

    private static List<Volume> volumesIn(Context context, Uri treeUri, Uri parentUri,
                                           List<Node> nodes, int[] visited, int depth, boolean traverseDirectories) throws Exception {
        if (depth > MAX_DEPTH) return Collections.emptyList();
        List<Volume> volumes = new ArrayList<>();
        List<String> directPages = new ArrayList<>();
        for (Node node : nodes) {
            if (!node.directory && isImage(node.name)) directPages.add("content:" + node.uri);
            if (!node.directory && isArchive(node.name)) {
                volumes.add(new Volume(stripExtension(node.name), node.uri.toString(),
                        Collections.emptyList(), node.uri.toString()));
            }
        }
        if (!directPages.isEmpty()) {
            volumes.add(new Volume(displayName(context, parentUri, "Volume"), parentUri.toString(), directPages, null));
        }
        if (!traverseDirectories) return volumes;
        for (Node node : nodes) {
            if (!node.directory) continue;
            List<Node> childNodes = children(context, treeUri, DocumentsContract.getDocumentId(node.uri), visited);
            List<String> pages = new ArrayList<>();
            for (Node child : childNodes) if (!child.directory && isImage(child.name)) pages.add("content:" + child.uri);
            if (!pages.isEmpty()) volumes.add(new Volume(node.name, node.uri.toString(), pages, null));
            else volumes.addAll(volumesIn(context, treeUri, node.uri, childNodes, visited, depth + 1, true));
        }
        return volumes;
    }

    private static List<Node> children(Context context, Uri treeUri, String parentId, int[] visited) throws Exception {
        if (visited[0] > MAX_DOCUMENTS) throw new IllegalStateException("Folder contains too many documents");
        Uri childrenUri = DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, parentId);
        List<Node> result = new ArrayList<>();
        String[] projection = {DocumentsContract.Document.COLUMN_DOCUMENT_ID,
                DocumentsContract.Document.COLUMN_DISPLAY_NAME, DocumentsContract.Document.COLUMN_MIME_TYPE};
        try (Cursor cursor = context.getContentResolver().query(childrenUri, projection, null, null, null)) {
            if (cursor == null) throw new IllegalStateException("Folder permission is unavailable");
            while (cursor.moveToNext()) {
                if (++visited[0] > MAX_DOCUMENTS) throw new IllegalStateException("Folder contains too many documents");
                String id = cursor.getString(0);
                String name = cursor.getString(1);
                String mime = cursor.getString(2);
                result.add(new Node(name == null ? "Untitled" : name,
                        DocumentsContract.buildDocumentUriUsingTree(treeUri, id),
                        DocumentsContract.Document.MIME_TYPE_DIR.equals(mime)));
            }
        }
        result.sort((left, right) -> naturalCompare(left.name, right.name));
        return result;
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
            return readBounded(context.getContentResolver().openInputStream(Uri.parse(locator.substring(8))));
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
                return readBounded(zip.getInputStream(entry));
            }
        }
        throw new IllegalStateException("Archive page no longer exists");
    }

    public static int openArchive(Context context, String uriValue) throws Exception {
        ParcelFileDescriptor descriptor = openArchive(context, Uri.parse(uriValue));
        return descriptor.detachFd();
    }

    public static void releaseGrant(Context context, String uriValue) {
        context.getContentResolver().releasePersistableUriPermission(
                Uri.parse(uriValue), Intent.FLAG_GRANT_READ_URI_PERMISSION);
    }

    private static ParcelFileDescriptor openArchive(Context context, Uri uri) throws Exception {
        ParcelFileDescriptor descriptor = context.getContentResolver().openFileDescriptor(uri, "r");
        if (descriptor == null) throw new IllegalStateException("Archive is unavailable");
        return descriptor;
    }

    private static byte[] readBounded(InputStream input) throws Exception {
        if (input == null) throw new IllegalStateException("Page is unavailable");
        try (InputStream source = input; ByteArrayOutputStream output = new ByteArrayOutputStream()) {
            byte[] buffer = new byte[32 * 1024];
            int total = 0;
            int read;
            while ((read = source.read(buffer)) >= 0) {
                total += read;
                if (total > MAX_PAGE_BYTES) throw new IllegalStateException("Page exceeds 64 MiB safety limit");
                output.write(buffer, 0, read);
            }
            return output.toByteArray();
        }
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
        StringBuilder output = new StringBuilder();
        for (int i = 0; i < 16; i++) output.append(String.format(Locale.ROOT, "%02x", digest[i]));
        return output.toString();
    }

    private static String encode(String value) {
        return Base64.encodeToString(value.getBytes(StandardCharsets.UTF_8), Base64.URL_SAFE | Base64.NO_WRAP | Base64.NO_PADDING);
    }

    private static String decode(String value) {
        return new String(Base64.decode(value, Base64.URL_SAFE | Base64.NO_WRAP | Base64.NO_PADDING), StandardCharsets.UTF_8);
    }

    private static int naturalCompare(String left, String right) {
        int li = 0, ri = 0;
        while (li < left.length() && ri < right.length()) {
            char lc = Character.toLowerCase(left.charAt(li));
            char rc = Character.toLowerCase(right.charAt(ri));
            if (Character.isDigit(lc) && Character.isDigit(rc)) {
                int ls = li, rs = ri;
                while (li < left.length() && Character.isDigit(left.charAt(li))) li++;
                while (ri < right.length() && Character.isDigit(right.charAt(ri))) ri++;
                String ln = left.substring(ls, li).replaceFirst("^0+(?!$)", "");
                String rn = right.substring(rs, ri).replaceFirst("^0+(?!$)", "");
                int length = Integer.compare(ln.length(), rn.length());
                if (length != 0) return length;
                int number = ln.compareTo(rn);
                if (number != 0) return number;
            } else {
                if (lc != rc) return Character.compare(lc, rc);
                li++;
                ri++;
            }
        }
        return Integer.compare(left.length(), right.length());
    }
}
