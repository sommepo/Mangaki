package dev.local.mangareader;

import android.content.ContentValues;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.pm.PackageManager;
import android.database.Cursor;
import android.net.Uri;
import android.os.SystemClock;
import android.text.TextUtils;
import android.util.Base64;
import android.util.Log;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.BufferedInputStream;
import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.math.BigInteger;
import java.net.HttpURLConnection;
import java.net.InetSocketAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.SecureRandom;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.regex.Pattern;

/**
 * Loopback-only AnkiConnect-compatible bridge onto AnkiDroid's public ContentProvider API
 * (FlashCardsContract). Every request runs on a worker thread, never the main thread.
 */
public class AnkiBridge {
    private static final String TAG = "AnkiBridge";

    // Server State
    private static volatile Context appContext;
    private static volatile boolean serverRunning = false;
    private static ServerSocket serverSocket;
    private static final ExecutorService EXEC = Executors.newFixedThreadPool(4);
    private static final String TOKEN_PREFS = "manga_reader_anki_bridge";
    private static final String TOKEN_KEY = "token";
    private static final int PORT = 18765;
    private static volatile String cachedToken;

    public static synchronized String getBridgeToken(Context context) {
        if (cachedToken != null) return cachedToken;
        SharedPreferences preferences = context.getSharedPreferences(TOKEN_PREFS, Context.MODE_PRIVATE);
        String token = preferences.getString(TOKEN_KEY, null);
        if (token == null) {
            byte[] bytes = new byte[32];
            new SecureRandom().nextBytes(bytes);
            token = Base64.encodeToString(bytes, Base64.NO_WRAP | Base64.URL_SAFE);
            preferences.edit().putString(TOKEN_KEY, token).commit();
        }
        cachedToken = token;
        return token;
    }

    // API Constants
    private static final String ANKI_PACKAGE = "com.ichi2.anki";
    private static final String ANKI_PERMISSION = "com.ichi2.anki.permission.READ_WRITE_DATABASE";
    private static final String AUTHORITY = "com.ichi2.anki.flashcards";
    private static final Uri BASE_URI = Uri.parse("content://" + AUTHORITY);
    private static final Uri NOTES_URI = Uri.withAppendedPath(BASE_URI, "notes");
    private static final Uri NOTES_V2_URI = Uri.withAppendedPath(BASE_URI, "notes_v2");
    private static final Uri MODELS_URI = Uri.withAppendedPath(BASE_URI, "models");
    private static final Uri DECKS_URI = Uri.withAppendedPath(BASE_URI, "decks");
    private static final Uri MEDIA_URI = Uri.withAppendedPath(BASE_URI, "media");

    // Columns
    private static final String NOTE_ID = "_id";
    private static final String NOTE_MID = "mid";
    private static final String NOTE_FLDS = "flds";
    private static final String NOTE_TAGS = "tags";
    private static final String NOTE_CSUM = "csum";

    private static final String MODEL_ID = "_id";
    private static final String MODEL_NAME = "name";
    private static final String MODEL_FIELD_NAMES = "field_names";

    private static final String DECK_ID = "deck_id";
    private static final String DECK_NAME = "deck_name";

    private static final String MEDIA_FILE_URI = "file_uri";
    private static final String MEDIA_PREFERRED_NAME = "preferred_name";

    private static final String FIELD_SEPARATOR = "\u001f";
    private static final int MAX_MEDIA_BYTES = 50 * 1024 * 1024;

    // HTML Utils
    private static final Pattern STYLE_TAG = Pattern.compile("(?s)<style.*?>.*?</style>");
    private static final Pattern SCRIPT_TAG = Pattern.compile("(?s)<script.*?>.*?</script>");
    private static final Pattern HTML_TAG = Pattern.compile("<.*?>");
    private static final Pattern IMG_TAG = Pattern.compile("<img src=[\"']?([^\"'>]+)[\"']? ?/?>");
    private static final Pattern VALID_AUDIO_EXT = Pattern.compile("(?i).*\\.(mp3|aac|m4a|wav|webm|ogg|flac)$");
    private static final Pattern WHITESPACE = Pattern.compile("\\s+");

    /** Anki search keywords that are not note fields; queries using them go to AnkiDroid's own search. */
    private static final Set<String> SEARCH_KEYWORDS = new HashSet<>(java.util.Arrays.asList(
            "tag", "added", "rated", "is", "card", "flag", "prop", "nid", "cid", "mid", "edited",
            "introduced", "resched", "dupe", "re", "nc", "w", "preset", "has-cd", "has-rd",
            "deck", "note", "notetype"));

    // ========================================================================
    // SERVER MANAGEMENT
    // ========================================================================

    public static void startAnkiConnectServer(Context context) {
        appContext = context.getApplicationContext();
        synchronized (AnkiBridge.class) {
            if (serverRunning) return;
            serverRunning = true;
        }

        new Thread(() -> {
            try (ServerSocket socket = new ServerSocket()) {
                // Rebinding right after a process restart must not fail on a TIME_WAIT socket.
                socket.setReuseAddress(true);
                // This compatibility bridge is app-private and never exposed to the LAN.
                socket.bind(new InetSocketAddress("127.0.0.1", PORT));
                serverSocket = socket;
                Log.i(TAG, "Anki bridge listening on loopback:" + PORT);

                while (serverRunning) {
                    try {
                        Socket client = socket.accept();
                        EXEC.execute(() -> handleAnkiRequest(client));
                    } catch (IOException e) {
                        if (!serverRunning || socket.isClosed()) break;
                        Log.w(TAG, "Accept failed; continuing", e);
                    }
                }
            } catch (Exception e) {
                Log.e(TAG, "AnkiConnect server start error", e);
            } finally {
                serverSocket = null;
                // Allow a later startAnkiConnectServer() (e.g. the next MangatanActivity) to retry.
                serverRunning = false;
            }
        }, "AnkiConnect-Server").start();
    }

    public static void stopAnkiConnectServer() {
        serverRunning = false;
        try {
            ServerSocket socket = serverSocket;
            if (socket != null) socket.close();
        } catch (Exception e) {
            Log.e(TAG, "Error stopping server", e);
        }
    }

    private static void handleAnkiRequest(Socket client) {
        final int HEADER_LIMIT = 32 * 1024;     // 32 KB
        final int BODY_LIMIT = 5_000_000;       // 5 MB (safe upper bound for our use)
        try {
            client.setSoTimeout(10_000); // 10s socket read timeout
            // Buffered: headers are scanned byte by byte, which would otherwise be one syscall per byte.
            InputStream rawIn = new BufferedInputStream(client.getInputStream(), 16 * 1024);
            OutputStream rawOut = client.getOutputStream();

            // --- Read headers  ---
            ByteArrayOutputStream headerBuffer = new ByteArrayOutputStream();
            byte[] last4 = new byte[4];
            int idx = 0;
            int b;
            while (true) {
                b = rawIn.read();
                if (b == -1) {
                    Log.w(TAG, "EOF while reading headers");
                    return;
                }
                headerBuffer.write(b);
                last4[idx++ % 4] = (byte) b;

                if (idx >= 4) {
                    if (last4[(idx - 4) & 3] == '\r' &&
                            last4[(idx - 3) & 3] == '\n' &&
                            last4[(idx - 2) & 3] == '\r' &&
                            last4[(idx - 1) & 3] == '\n') {
                        break;
                    }
                }
                if (idx >= 2) {
                    if (last4[(idx - 2) & 3] == '\n' &&
                            last4[(idx - 1) & 3] == '\n') {
                        break;
                    }
                }

                if (headerBuffer.size() > HEADER_LIMIT) {
                    writeResponse(rawOut, 413, error("Headers too large"), true);
                    return;
                }
            }

            // decode headers using ISO-8859-1 per HTTP spec
            String headerStr = new String(headerBuffer.toByteArray(), StandardCharsets.ISO_8859_1);
            String[] lines = headerStr.split("\r?\n");

            if (lines.length == 0) {
                writeResponse(rawOut, 400, error("Invalid request"), true);
                return;
            }

            // Parse request line: METHOD PATH VERSION
            String[] requestParts = lines[0].split(" ");
            String method = requestParts.length > 0 ? requestParts[0].trim() : "GET";

            // Build header map
            Map<String, String> hdrs = new HashMap<>();
            for (int i = 1; i < lines.length; i++) {
                String line = lines[i];
                if (line == null || line.length() == 0) break;
                int colon = line.indexOf(':');
                if (colon <= 0) continue;
                String name = line.substring(0, colon).trim().toLowerCase(Locale.ROOT);
                String value = line.substring(colon + 1).trim();
                hdrs.put(name, value);
            }

            // Handle Transfer-Encoding
            String transferEnc = hdrs.get("transfer-encoding");
            if (transferEnc != null && transferEnc.toLowerCase(Locale.ROOT).contains("chunk")) {
                writeResponse(rawOut, 501, error("Chunked transfer-encoding not supported"), true);
                return;
            }

            // Handle Expect: 100-continue
            String expect = hdrs.get("expect");
            if (expect != null && expect.equalsIgnoreCase("100-continue")) {
                try {
                    rawOut.write("HTTP/1.1 100 Continue\r\n\r\n".getBytes(StandardCharsets.US_ASCII));
                    rawOut.flush();
                } catch (Exception e) {
                    Log.w(TAG, "Failed to send 100-continue", e);
                }
            }

            // Parse Content-Length
            int contentLength = 0;
            String cl = hdrs.get("content-length");
            if (cl != null) {
                try {
                    contentLength = Integer.parseInt(cl.trim());
                } catch (NumberFormatException ignored) {
                    writeResponse(rawOut, 400, error("Invalid Content-Length"), true);
                    return;
                }
            }

            // OPTIONS preflight (CORS)
            if ("OPTIONS".equalsIgnoreCase(method)) {
                writeResponse(rawOut, 200, "", true);
                return;
            }

            // Only POST is allowed for API calls
            if (!"POST".equalsIgnoreCase(method)) {
                writeResponse(rawOut, 405, error("Method not allowed"), true);
                return;
            }
            String presented = hdrs.get("x-manga-reader-bridge");
            if (presented == null || !MessageDigest.isEqual(
                    getBridgeToken(appContext).getBytes(StandardCharsets.UTF_8),
                    presented.getBytes(StandardCharsets.UTF_8))) {
                writeResponse(rawOut, 403, error("Forbidden"), true);
                return;
            }

            // Sanity checks on Content-Length
            if (contentLength < 0) {
                writeResponse(rawOut, 400, error("Invalid Content-Length"), true);
                return;
            }
            if (contentLength > BODY_LIMIT) {
                writeResponse(rawOut, 413, error("Body too large"), true);
                return;
            }

            // Read body (exactly contentLength bytes)
            String jsonRequest = "{}";
            if (contentLength > 0) {
                byte[] body = new byte[contentLength];
                int totalRead = 0;
                while (totalRead < contentLength) {
                    int r = rawIn.read(body, totalRead, contentLength - totalRead);
                    if (r == -1) {
                        writeResponse(rawOut, 400, error("Unexpected EOF while reading body"), true);
                        return;
                    }
                    totalRead += r;
                }
                // decode body as UTF-8 (handles Japanese and other Unicode)
                jsonRequest = new String(body, 0, totalRead, StandardCharsets.UTF_8);
            }

            // Process and respond
            String jsonResponse = processRequest(appContext, jsonRequest);
            writeResponse(rawOut, 200, jsonResponse, true);

        } catch (java.net.SocketTimeoutException ste) {
            // read timed out — log and drop the connection
            Log.w(TAG, "Request timed out");
        } catch (Exception e) {
            Log.e(TAG, "Request Error", e);
            try {
                // Try to return 500 error to client
                OutputStream out = client.getOutputStream();
                writeResponse(out, 500, error(e.getMessage()), true);
            } catch (Exception ignored) {
            }
        } finally {
            try {
                client.close();
            } catch (Exception ignored) {
            }
        }
    }

    private static void writeResponse(OutputStream out, int code, String body, boolean cors) throws Exception {
        byte[] bytes = body.getBytes(StandardCharsets.UTF_8);
        String status = (code == 200) ? "OK" : "Error";

        StringBuilder h = new StringBuilder();
        h.append("HTTP/1.1 ").append(code).append(" ").append(status).append("\r\n");
        h.append("Content-Type: application/json; charset=utf-8\r\n");

        if (cors) {
            h.append("Access-Control-Allow-Origin: http://127.0.0.1:14568\r\n");
            h.append("Access-Control-Allow-Methods: POST, OPTIONS\r\n");
            h.append("Access-Control-Allow-Headers: Content-Type, X-Manga-Reader-Bridge\r\n");
            h.append("Access-Control-Allow-Private-Network: true\r\n");
            h.append("Vary: Origin\r\n");
        }

        h.append("Content-Length: ").append(bytes.length).append("\r\n");
        // One request per connection: tell keep-alive clients not to reuse the socket.
        h.append("Connection: close\r\n");
        h.append("\r\n");

        out.write(h.toString().getBytes(StandardCharsets.UTF_8));
        out.write(bytes);
        out.flush();
    }

    // ========================================================================
    // LOGIC
    // ========================================================================

    public static String processRequest(Context context, String jsonRequest) {
        try {
            JSONObject request = new JSONObject(jsonRequest);
            if (!request.has("action")) return error("Missing action");

            String action = request.getString("action");
            JSONObject params = request.optJSONObject("params");
            if (params == null) params = new JSONObject();

            Object result;
            switch (action) {
                case "version":
                    result = 6;
                    break;
                case "requestPermission":
                    result = permission(context);
                    break;
                case "multi":
                    result = handleMulti(context, params);
                    break;
                default:
                    requireAnkiAccess(context);
                    result = runAnkiAction(context, action, params);
                    if (result == UNKNOWN_ACTION) return error("Unknown action: " + action);
            }
            return success(result);
        } catch (SecurityException e) {
            return error("AnkiDroid permission was denied. Allow Mangaki to access AnkiDroid and try again.");
        } catch (Exception e) {
            return error(e.getMessage());
        }
    }

    private static final Object UNKNOWN_ACTION = new Object();

    private static Object runAnkiAction(Context context, String action, JSONObject params) throws Exception {
        switch (action) {
            case "deckNames":
                return deckNames(context);
            case "modelNames":
                return modelNames(context);
            case "modelFieldNames":
                return modelFieldNames(context, params);
            case "findNotes":
                return findNotes(context, params);
            case "guiBrowse":
                return guiBrowse(context, params);
            case "addNote":
                return addNote(context, params);
            case "updateNoteFields":
                updateNote(context, params);
                return JSONObject.NULL;
            case "notesInfo":
                return notesInfo(context, params);
            case "canAddNotes":
                return canAddNotes(context, params);
            case "storeMediaFile":
                return storeMediaFile(context, params);
            default:
                return UNKNOWN_ACTION;
        }
    }

    private static boolean isAnkiInstalled(Context ctx) {
        return ctx.getPackageManager().resolveContentProvider(AUTHORITY, 0) != null;
    }

    private static boolean hasAnkiPermission(Context ctx) {
        return ctx.checkSelfPermission(ANKI_PERMISSION) == PackageManager.PERMISSION_GRANTED;
    }

    /** Fail with an actionable message instead of silently returning empty deck/model lists. */
    private static void requireAnkiAccess(Context ctx) throws Exception {
        if (!isAnkiInstalled(ctx)) throw new Exception("AnkiDroid is not installed on this device.");
        if (!hasAnkiPermission(ctx)) {
            throw new Exception("AnkiDroid permission has not been granted. Enable on-device Anki in Settings to allow access.");
        }
    }

    private static JSONObject permission(Context ctx) throws Exception {
        JSONObject r = new JSONObject();
        r.put("permission", isAnkiInstalled(ctx) && hasAnkiPermission(ctx) ? "granted" : "denied");
        r.put("requireApikey", false);
        r.put("version", 6);
        return r;
    }

    // ------------------------------------------------------------------------
    // Model / deck catalogue (short-lived cache: one provider scan instead of one per note)
    // ------------------------------------------------------------------------

    private static final long CATALOG_TTL_MS = 5_000;

    private static final class ModelInfo {
        final long id;
        final String name;
        final String[] fields;

        ModelInfo(long id, String name, String[] fields) {
            this.id = id;
            this.name = name;
            this.fields = fields;
        }
    }

    private static List<ModelInfo> modelCatalog;
    private static long modelCatalogAt;
    private static Map<Long, String> deckCatalog;
    private static long deckCatalogAt;

    private static synchronized List<ModelInfo> models(Context ctx, boolean refresh) throws Exception {
        long now = SystemClock.elapsedRealtime();
        if (!refresh && modelCatalog != null && now - modelCatalogAt < CATALOG_TTL_MS) return modelCatalog;
        List<ModelInfo> loaded = new ArrayList<>();
        try (Cursor c = ctx.getContentResolver().query(
                MODELS_URI, new String[]{MODEL_ID, MODEL_NAME, MODEL_FIELD_NAMES}, null, null, null)) {
            if (c == null) throw new Exception("AnkiDroid did not return its note types. Is AnkiDroid installed?");
            while (c.moveToNext()) {
                loaded.add(new ModelInfo(c.getLong(0), c.getString(1), parseFieldsInternal(c.getString(2))));
            }
        }
        modelCatalog = loaded;
        modelCatalogAt = now;
        return loaded;
    }

    private static synchronized Map<Long, String> decks(Context ctx, boolean refresh) throws Exception {
        long now = SystemClock.elapsedRealtime();
        if (!refresh && deckCatalog != null && now - deckCatalogAt < CATALOG_TTL_MS) return deckCatalog;
        Map<Long, String> loaded = new java.util.LinkedHashMap<>();
        try (Cursor c = ctx.getContentResolver().query(DECKS_URI, new String[]{DECK_ID, DECK_NAME}, null, null, null)) {
            if (c == null) throw new Exception("AnkiDroid did not return its decks. Is AnkiDroid installed?");
            int idIdx = c.getColumnIndex(DECK_ID);
            int nameIdx = c.getColumnIndex(DECK_NAME);
            if (idIdx < 0 || nameIdx < 0) throw new Exception("AnkiDroid returned an unexpected deck list");
            while (c.moveToNext()) loaded.put(c.getLong(idIdx), c.getString(nameIdx));
        }
        deckCatalog = loaded;
        deckCatalogAt = now;
        return loaded;
    }

    private static ModelInfo findModel(Context ctx, String name) throws Exception {
        for (int attempt = 0; attempt < 2; attempt++) {
            ModelInfo caseInsensitive = null;
            for (ModelInfo model : models(ctx, attempt > 0)) {
                if (model.name == null) continue;
                if (model.name.equals(name)) return model;
                if (caseInsensitive == null && model.name.equalsIgnoreCase(name)) caseInsensitive = model;
            }
            if (caseInsensitive != null) return caseInsensitive;
        }
        throw new Exception("Model '" + name + "' not found.");
    }

    private static ModelInfo findModel(Context ctx, long id) throws Exception {
        for (int attempt = 0; attempt < 2; attempt++) {
            for (ModelInfo model : models(ctx, attempt > 0)) if (model.id == id) return model;
        }
        return null;
    }

    private static String deckName(Context ctx, long deckId) throws Exception {
        String name = decks(ctx, false).get(deckId);
        return name != null ? name : decks(ctx, true).get(deckId);
    }

    private static JSONArray deckNames(Context ctx) throws Exception {
        JSONArray result = new JSONArray();
        for (String name : decks(ctx, true).values()) if (name != null) result.put(name);
        return result;
    }

    private static JSONArray modelNames(Context ctx) throws Exception {
        JSONArray result = new JSONArray();
        for (ModelInfo model : models(ctx, true)) if (model.name != null) result.put(model.name);
        return result;
    }

    private static JSONArray modelFieldNames(Context ctx, JSONObject params) throws Exception {
        String targetName = params.getString("modelName");
        String[] best = null;
        for (ModelInfo model : models(ctx, false)) {
            if (!targetName.equals(model.name)) continue;
            if (best == null || model.fields.length > best.length) best = model.fields;
        }
        JSONArray result = new JSONArray();
        if (best != null) for (String field : best) result.put(field);
        return result;
    }

    // ------------------------------------------------------------------------
    // findNotes
    // ------------------------------------------------------------------------

    /** The subset of Anki search syntax the mining UI sends: deck:, note: and one "Field:value". */
    private static final class SimpleQuery {
        String deck;
        String note;
        String field;
        String value;
    }

    /** Splits an Anki search into terms, honouring "quoted terms" and key:"quoted values". */
    private static List<String> searchTerms(String query) {
        List<String> terms = new ArrayList<>();
        StringBuilder current = new StringBuilder();
        boolean quoted = false;
        boolean inTerm = false;
        for (int i = 0; i < query.length(); i++) {
            char c = query.charAt(i);
            if (c == '\\' && i + 1 < query.length()) {
                current.append(c).append(query.charAt(++i));
                inTerm = true;
            } else if (c == '"') {
                quoted = !quoted;
                inTerm = true;
            } else if (!quoted && Character.isWhitespace(c)) {
                if (inTerm) terms.add(current.toString());
                current.setLength(0);
                inTerm = false;
            } else {
                current.append(c);
                inTerm = true;
            }
        }
        if (quoted) return null;
        if (inTerm) terms.add(current.toString());
        return terms;
    }

    private static SimpleQuery parseSimpleQuery(String query) {
        List<String> terms = searchTerms(query);
        if (terms == null || terms.isEmpty()) return null;
        SimpleQuery parsed = new SimpleQuery();
        for (String term : terms) {
            // Escapes, wildcards, negation and grouping need Anki's real search engine.
            if (term.isEmpty() || term.indexOf('\\') >= 0 || term.startsWith("-") || term.startsWith("(")
                    || term.equalsIgnoreCase("or") || term.equalsIgnoreCase("and")) {
                return null;
            }
            int colon = term.indexOf(':');
            if (colon <= 0 || colon == term.length() - 1) return null;
            String key = term.substring(0, colon);
            String value = term.substring(colon + 1);
            String lowerKey = key.toLowerCase(Locale.ROOT);
            if (lowerKey.equals("deck")) {
                if (parsed.deck != null || value.equalsIgnoreCase("current") || value.equalsIgnoreCase("filtered")) return null;
                parsed.deck = value;
            } else if (lowerKey.equals("note")) {
                if (parsed.note != null || value.indexOf('*') >= 0 || value.indexOf('_') >= 0) return null;
                parsed.note = value;
            } else if (SEARCH_KEYWORDS.contains(lowerKey) || parsed.field != null
                    || value.indexOf('*') >= 0 || value.indexOf('_') >= 0) {
                return null;
            } else {
                parsed.field = key;
                parsed.value = value;
            }
        }
        return parsed.field == null ? null : parsed;
    }

    /**
     * Fast duplicate lookup for "Field:value" searches. AnkiDroid indexes only a checksum of each
     * note's first field, so the checksum path is used when the searched field is the first field
     * of every candidate note type; any other query is passed to AnkiDroid's own search engine.
     */
    private static JSONArray findNotes(Context ctx, JSONObject params) throws Exception {
        String query = params.getString("query");
        JSONArray ids = new JSONArray();
        SimpleQuery parsed = parseSimpleQuery(query);
        Set<Long> modelIds = parsed == null ? null : checksumModels(ctx, parsed);
        if (modelIds == null) {
            try (Cursor c = ctx.getContentResolver().query(NOTES_URI, new String[]{NOTE_ID}, query, null, null)) {
                if (c != null) while (c.moveToNext()) ids.put(c.getLong(0));
            }
            return ids;
        }
        if (modelIds.isEmpty()) return ids;

        long csum = fieldChecksum(parsed.value);
        Map<Long, List<Long>> deckCache = new HashMap<>();
        try (Cursor c = ctx.getContentResolver().query(NOTES_V2_URI, new String[]{NOTE_ID, NOTE_MID},
                NOTE_CSUM + "=?", new String[]{String.valueOf(csum)}, null)) {
            if (c != null) {
                while (c.moveToNext()) {
                    long nid = c.getLong(0);
                    if (!modelIds.contains(c.getLong(1))) continue;
                    if (parsed.deck != null && !noteInDeck(ctx, nid, parsed.deck, true, deckCache)) continue;
                    ids.put(nid);
                }
            }
        }
        return ids;
    }

    /** Note types eligible for the checksum path, empty when none has the field, or null when it cannot be used. */
    private static Set<Long> checksumModels(Context ctx, SimpleQuery parsed) throws Exception {
        Set<Long> result = new HashSet<>();
        for (ModelInfo model : models(ctx, false)) {
            if (parsed.note != null && (model.name == null || !model.name.equalsIgnoreCase(parsed.note))) continue;
            for (int i = 0; i < model.fields.length; i++) {
                if (!parsed.field.equalsIgnoreCase(model.fields[i])) continue;
                if (i != 0) return null;
                result.add(model.id);
            }
        }
        return result;
    }

    /** Anki deck search semantics: case-insensitive, includes subdecks, '*' wildcard. */
    private static boolean deckMatches(String actual, String pattern, boolean includeChildren) {
        if (actual == null) return false;
        String name = actual.toLowerCase(Locale.ROOT);
        String wanted = pattern.toLowerCase(Locale.ROOT);
        if (wanted.indexOf('*') >= 0) {
            StringBuilder regex = new StringBuilder();
            for (String part : wanted.split("\\*", -1)) {
                if (regex.length() > 0 || wanted.startsWith("*")) regex.append(".*");
                regex.append(Pattern.quote(part));
            }
            return Pattern.compile(regex.toString(), Pattern.DOTALL).matcher(name).matches();
        }
        return name.equals(wanted) || (includeChildren && name.startsWith(wanted + "::"));
    }

    private static List<Long> noteDecks(Context ctx, long noteId, Map<Long, List<Long>> cache) {
        List<Long> cached = cache.get(noteId);
        if (cached != null) return cached;
        List<Long> deckIds = new ArrayList<>();
        Uri cardsUri = Uri.withAppendedPath(NOTES_URI, noteId + "/cards");
        try (Cursor c = ctx.getContentResolver().query(cardsUri, new String[]{DECK_ID}, null, null, null)) {
            if (c != null) while (c.moveToNext()) deckIds.add(c.getLong(0));
        } catch (Exception e) {
            Log.e(TAG, "noteDecks", e);
        }
        cache.put(noteId, deckIds);
        return deckIds;
    }

    private static boolean noteInDeck(Context ctx, long noteId, String deckPattern, boolean includeChildren,
                                      Map<Long, List<Long>> cache) throws Exception {
        for (long deckId : noteDecks(ctx, noteId, cache)) {
            if (deckMatches(deckName(ctx, deckId), deckPattern, includeChildren)) return true;
        }
        return false;
    }

    private static JSONArray guiBrowse(Context ctx, JSONObject params) throws Exception {
        String query = params.getString("query");
        String searchQuery = query;

        if (query.startsWith("nid:")) {
            try {
                long noteId = Long.parseLong(query.substring(4).trim());
                List<Long> deckIds = noteDecks(ctx, noteId, new HashMap<>());
                if (!deckIds.isEmpty()) {
                    String name = deckName(ctx, deckIds.get(0));
                    if (name != null) searchQuery = "deck:\"" + name + "\" " + query;
                }
            } catch (Exception ignored) {}
        }

        Uri uri = Uri.parse("anki://x-callback-url/browser?search=" + Uri.encode(searchQuery));
        Intent intent = new Intent(Intent.ACTION_VIEW, uri);
        intent.setPackage(ANKI_PACKAGE);
        intent.setFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TOP);
        try {
            ctx.startActivity(intent);
        } catch (Exception e) {
            Log.w(TAG, "guiBrowse", e);
        }
        return new JSONArray();
    }

    // ------------------------------------------------------------------------
    // Duplicate policy (AnkiConnect semantics)
    // ------------------------------------------------------------------------

    private static final class DuplicatePolicy {
        final boolean allowDuplicate;
        final boolean deckScope;
        final String deckName;
        final boolean checkChildren;
        final boolean checkAllModels;

        DuplicatePolicy(JSONObject note) {
            JSONObject options = note.optJSONObject("options");
            if (options == null) options = new JSONObject();
            JSONObject scopeOptions = options.optJSONObject("duplicateScopeOptions");
            if (scopeOptions == null) scopeOptions = new JSONObject();
            allowDuplicate = options.optBoolean("allowDuplicate", false);
            deckScope = "deck".equals(options.optString("duplicateScope", "collection"));
            String overrideDeck = scopeOptions.isNull("deckName") ? null : scopeOptions.optString("deckName", null);
            deckName = overrideDeck != null ? overrideDeck : note.optString("deckName", null);
            checkChildren = scopeOptions.optBoolean("checkChildren", false);
            checkAllModels = scopeOptions.optBoolean("checkAllModels", false);
        }
    }

    private static final class Candidate {
        final long noteId;
        final long modelId;

        Candidate(long noteId, long modelId) {
            this.noteId = noteId;
            this.modelId = modelId;
        }
    }

    /** One indexed query for all checksums; returns csum -> notes having that first-field checksum. */
    private static Map<Long, List<Candidate>> notesByChecksum(Context ctx, Set<Long> checksums) {
        Map<Long, List<Candidate>> result = new HashMap<>();
        if (checksums.isEmpty()) return result;
        String in = TextUtils.join(",", checksums);
        try (Cursor c = ctx.getContentResolver().query(NOTES_V2_URI, new String[]{NOTE_ID, NOTE_MID, NOTE_CSUM},
                NOTE_CSUM + " IN (" + in + ")", null, null)) {
            if (c != null) {
                while (c.moveToNext()) {
                    long csum = c.getLong(2);
                    List<Candidate> list = result.get(csum);
                    if (list == null) result.put(csum, list = new ArrayList<>());
                    list.add(new Candidate(c.getLong(0), c.getLong(1)));
                }
            }
        }
        return result;
    }

    private static boolean isDuplicate(Context ctx, List<Candidate> candidates, long modelId,
                                       DuplicatePolicy policy, Map<Long, List<Long>> deckCache) throws Exception {
        if (candidates == null) return false;
        for (Candidate candidate : candidates) {
            if (!policy.checkAllModels && candidate.modelId != modelId) continue;
            if (policy.deckScope && policy.deckName != null
                    && !noteInDeck(ctx, candidate.noteId, policy.deckName, policy.checkChildren, deckCache)) {
                continue;
            }
            return true;
        }
        return false;
    }

    private static String firstFieldValue(JSONObject fields, ModelInfo model) {
        if (model.fields.length == 0) return "";
        return fields.optString(model.fields[0], "");
    }

    private static JSONArray canAddNotes(Context ctx, JSONObject params) throws Exception {
        JSONArray notes = params.getJSONArray("notes");
        JSONArray results = new JSONArray();
        if (notes.length() == 0) return results;

        int count = notes.length();
        ModelInfo[] noteModels = new ModelInfo[count];
        long[] checksums = new long[count];
        boolean[] empty = new boolean[count];
        Set<Long> distinct = new LinkedHashSet<>();
        for (int i = 0; i < count; i++) {
            JSONObject note = notes.getJSONObject(i);
            ModelInfo model = findModel(ctx, note.getString("modelName"));
            if (model.fields.length == 0) throw new Exception("The selected note type has no fields");
            String first = firstFieldValue(note.getJSONObject("fields"), model);
            noteModels[i] = model;
            empty[i] = stripForChecksum(first).isEmpty();
            checksums[i] = fieldChecksum(first);
            if (!empty[i]) distinct.add(checksums[i]);
        }

        Map<Long, List<Candidate>> existing = notesByChecksum(ctx, distinct);
        Map<Long, List<Long>> deckCache = new HashMap<>();
        for (int i = 0; i < count; i++) {
            if (empty[i]) {
                results.put(false);
                continue;
            }
            DuplicatePolicy policy = new DuplicatePolicy(notes.getJSONObject(i));
            boolean duplicate = !policy.allowDuplicate
                    && isDuplicate(ctx, existing.get(checksums[i]), noteModels[i].id, policy, deckCache);
            results.put(!duplicate);
        }
        return results;
    }

    // ------------------------------------------------------------------------
    // Notes
    // ------------------------------------------------------------------------

    private static long addNote(Context ctx, JSONObject params) throws Exception {
        JSONObject note = params.getJSONObject("note");
        String deckName = note.getString("deckName");
        String modelName = note.getString("modelName");
        JSONObject fields = note.getJSONObject("fields");

        long targetDeckId = findDeckId(ctx, deckName);
        ModelInfo model = findModel(ctx, modelName);
        String[] fieldNames = model.fields;
        if (fieldNames.length == 0) throw new Exception("The selected note type has no fields");

        // Reject duplicates before uploading any media, so a refused note leaves nothing behind
        // in AnkiDroid and a retried request cannot silently create a second card.
        String first = firstFieldValue(fields, model);
        DuplicatePolicy policy = new DuplicatePolicy(note);
        if (!policy.allowDuplicate && !stripForChecksum(first).isEmpty()) {
            long csum = fieldChecksum(first);
            Set<Long> one = new HashSet<>();
            one.add(csum);
            if (isDuplicate(ctx, notesByChecksum(ctx, one).get(csum), model.id, policy, new HashMap<>())) {
                throw new Exception("cannot create note because it is a duplicate");
            }
        }

        if (note.has("picture")) processMedia(ctx, fields, note.get("picture"), false);
        if (note.has("audio")) processMedia(ctx, fields, note.get("audio"), true);

        String[] vals = new String[fieldNames.length];
        for (int i = 0; i < fieldNames.length; i++) {
            vals[i] = fields.optString(fieldNames[i], "");
        }
        if (stripForChecksum(vals[0]).isEmpty()) throw new Exception("cannot create note because it is empty");

        Set<String> tags = new LinkedHashSet<>();
        tags.add("manga-reader-fork");
        JSONArray tagArr = note.optJSONArray("tags");
        if (tagArr != null) {
            for (int i = 0; i < tagArr.length(); i++) {
                // Anki tags are space separated; a tag containing spaces would split into several.
                String tag = WHITESPACE.matcher(tagArr.getString(i).trim()).replaceAll("_");
                if (!tag.isEmpty()) tags.add(tag);
            }
        }

        ContentValues cv = new ContentValues();
        cv.put(NOTE_MID, model.id);
        cv.put(NOTE_FLDS, joinFields(vals));
        cv.put(NOTE_TAGS, joinTags(tags));

        Uri result = ctx.getContentResolver().insert(NOTES_URI, cv);
        if (result == null) throw new Exception("Insert failed");

        long newNoteId = Long.parseLong(result.getLastPathSegment());

        moveCardsToDeck(ctx, newNoteId, targetDeckId);

        return newNoteId;
    }

    private static void moveCardsToDeck(Context ctx, long noteId, long targetDeckId) {
        Uri cardsUri = Uri.withAppendedPath(NOTES_URI, noteId + "/cards");
        try (Cursor c = ctx.getContentResolver().query(cardsUri, new String[]{"ord", DECK_ID}, null, null, null)) {
            if (c != null) {
                while (c.moveToNext()) {
                    int ord = c.getInt(0);
                    long currentDeckId = c.getLong(1);

                    if (currentDeckId != targetDeckId) {
                        Uri specificCardUri = Uri.withAppendedPath(cardsUri, String.valueOf(ord));
                        ContentValues values = new ContentValues();
                        values.put(DECK_ID, targetDeckId);
                        ctx.getContentResolver().update(specificCardUri, values, null, null);
                    }
                }
            }
        } catch (Exception e) {
            Log.e(TAG, "Failed to move cards to deck", e);
        }
    }

    private static void updateNote(Context ctx, JSONObject params) throws Exception {
        JSONObject note = params.getJSONObject("note");
        long noteId = note.getLong("id");
        JSONObject fields = note.getJSONObject("fields");

        Uri uri = Uri.withAppendedPath(NOTES_URI, String.valueOf(noteId));
        long modelId;
        String[] oldFields;
        try (Cursor c = ctx.getContentResolver().query(uri, new String[]{NOTE_MID, NOTE_FLDS}, null, null, null)) {
            if (c == null || !c.moveToFirst()) throw new Exception("Note " + noteId + " was not found");
            modelId = c.getLong(0);
            oldFields = splitFields(c.getString(1));
        }
        ModelInfo model = findModel(ctx, modelId);
        if (model == null) throw new Exception("Model fields not found for ID: " + modelId);

        if (note.has("picture")) processMedia(ctx, fields, note.get("picture"), false);
        if (note.has("audio")) processMedia(ctx, fields, note.get("audio"), true);

        String[] fieldNames = model.fields;
        String[] newFields = new String[fieldNames.length];
        for (int i = 0; i < fieldNames.length; i++) {
            if (fields.has(fieldNames[i])) newFields[i] = fields.getString(fieldNames[i]);
            else if (i < oldFields.length) newFields[i] = oldFields[i];
            else newFields[i] = "";
        }

        ContentValues cv = new ContentValues();
        cv.put(NOTE_FLDS, joinFields(newFields));
        ctx.getContentResolver().update(uri, cv, null, null);
    }

    private static JSONArray notesInfo(Context ctx, JSONObject params) throws Exception {
        JSONArray noteIds = params.getJSONArray("notes");
        JSONArray result = new JSONArray();

        for (int i = 0; i < noteIds.length(); i++) {
            long nid = noteIds.getLong(i);
            JSONObject noteObj = null;
            Uri uri = Uri.withAppendedPath(NOTES_URI, String.valueOf(nid));
            try (Cursor c = ctx.getContentResolver().query(uri, new String[]{NOTE_MID, NOTE_FLDS, NOTE_TAGS}, null, null, null)) {
                if (c != null && c.moveToFirst()) {
                    long mid = c.getLong(0);
                    String[] fieldVals = splitFields(c.getString(1));
                    String tags = c.getString(2);
                    ModelInfo model = findModel(ctx, mid);
                    String[] fieldNames = model == null ? new String[0] : model.fields;

                    noteObj = new JSONObject();
                    noteObj.put("noteId", nid);
                    noteObj.put("modelName", model == null || model.name == null ? "Unknown" : model.name);

                    JSONArray tagsArr = new JSONArray();
                    if (tags != null) {
                        for (String t : tags.split(" ")) {
                            if (!t.isEmpty()) tagsArr.put(t);
                        }
                    }
                    noteObj.put("tags", tagsArr);

                    JSONObject fieldsObj = new JSONObject();
                    for (int j = 0; j < fieldNames.length; j++) {
                        JSONObject fVal = new JSONObject();
                        fVal.put("value", j < fieldVals.length ? fieldVals[j] : "");
                        fVal.put("order", j);
                        fieldsObj.put(fieldNames[j], fVal);
                    }
                    noteObj.put("fields", fieldsObj);
                }
            }
            result.put(noteObj == null ? JSONObject.NULL : noteObj);
        }
        return result;
    }

    // ------------------------------------------------------------------------
    // Media
    // ------------------------------------------------------------------------

    private static void processMedia(Context ctx, JSONObject fields, Object mediaObj, boolean isAudioContext) throws Exception {
        JSONArray arr = (mediaObj instanceof JSONArray) ? (JSONArray) mediaObj : new JSONArray().put(mediaObj);

        for (int i = 0; i < arr.length(); i++) {
            JSONObject m = arr.getJSONObject(i);
            if (!m.has("filename")) continue;
            String filename = m.getString("filename");
            // Never log the payload itself: it is user content and can be megabytes of base64.
            Log.i(TAG, "Processing media: " + filename);

            if (isAudioContext) {
                if (!VALID_AUDIO_EXT.matcher(filename).matches()) {
                    if (filename.contains(".")) {
                        filename = filename.substring(0, filename.lastIndexOf('.')) + ".mp3";
                    } else {
                        filename = filename + ".mp3";
                    }
                    Log.i(TAG, "Forced audio extension: " + filename);
                }
            }

            String storedFilename = null;

            if (m.has("data")) {
                storedFilename = saveMedia(ctx, filename, m.getString("data"));
            } else if (m.has("url")) {
                storedFilename = saveMediaFromUrl(ctx, filename, m.getString("url"));
            } else if (m.has("path")) {
                storedFilename = saveMediaFromPath(ctx, filename, m.getString("path"));
            }

            if (storedFilename == null) continue;

            // Generate tag based on context
            String val;
            if (isAudioContext) {
                val = "[sound:" + storedFilename + "]";
            } else {
                val = "<img src=\"" + storedFilename + "\">";
            }

            JSONArray targets = m.optJSONArray("fields");
            if (targets != null) {
                for (int j = 0; j < targets.length(); j++) {
                    String fName = targets.getString(j);
                    String current = fields.optString(fName, "");
                    fields.put(fName, current + val);
                }
            }
        }
    }

    private static String storeMediaFile(Context ctx, JSONObject params) throws Exception {
        String filename = params.getString("filename");
        if (params.has("data")) return saveMedia(ctx, filename, params.getString("data"));
        else if (params.has("url")) return saveMediaFromUrl(ctx, filename, params.getString("url"));
        else if (params.has("path")) return saveMediaFromPath(ctx, filename, params.getString("path"));
        else throw new Exception("storeMediaFile requires 'data', 'url', or 'path'");
    }

    private static String saveMedia(Context ctx, String name, String b64) throws Exception {
        byte[] data = Base64.decode(b64, Base64.NO_WRAP);
        return saveMediaBytes(ctx, name, data);
    }

    private static String saveMediaFromUrl(Context ctx, String name, String urlString) throws Exception {
        Log.i(TAG, "Downloading media for " + name);
        URL url = new URL(urlString);
        HttpURLConnection conn = (HttpURLConnection) url.openConnection();
        try {
            conn.setRequestMethod("GET");
            conn.setConnectTimeout(30000);
            conn.setReadTimeout(30000);
            conn.setInstanceFollowRedirects(true);
            conn.setRequestProperty("User-Agent", "MangaReaderFork/0.1");

            if (conn.getResponseCode() != HttpURLConnection.HTTP_OK) {
                throw new Exception("HTTP " + conn.getResponseCode());
            }
            if (conn.getContentLengthLong() > MAX_MEDIA_BYTES) throw new Exception("Media file is too large");

            try (InputStream input = conn.getInputStream()) {
                return saveMediaBytes(ctx, name, readMediaBounded(input));
            }
        } finally {
            conn.disconnect();
        }
    }

    private static String saveMediaFromPath(Context ctx, String name, String filePath) throws Exception {
        File file = new File(filePath);
        if (!file.exists()) throw new Exception("File not found: " + filePath);
        if (file.length() > MAX_MEDIA_BYTES) throw new Exception("Media file is too large");
        try (InputStream input = new java.io.FileInputStream(file)) {
            return saveMediaBytes(ctx, name, readMediaBounded(input));
        }
    }

    private static byte[] readMediaBounded(InputStream input) throws Exception {
        ByteArrayOutputStream buffer = new ByteArrayOutputStream(64 * 1024);
        byte[] data = new byte[16 * 1024];
        long total = 0;
        int r;
        while ((r = input.read(data)) != -1) {
            total += r;
            if (total > MAX_MEDIA_BYTES) throw new Exception("Media file is too large");
            buffer.write(data, 0, r);
        }
        return buffer.toByteArray();
    }

    /**
     * Hands media to AnkiDroid through its supported API: the bytes are written to this app's
     * private cache, exposed via {@link AnkiMediaProvider} with a one-off read grant, copied by
     * AnkiDroid into its collection, and then deleted. Works on every supported Android version
     * and never places mined media in the shared photo/music library.
     */
    private static String saveMediaBytes(Context ctx, String name, byte[] data) throws Exception {
        String lowerName = name.toLowerCase(Locale.ROOT);
        String extension = AnkiMediaProvider.isAudioName(lowerName)
                ? lowerName.substring(lowerName.lastIndexOf('.'))
                : imageExtension(lowerName);
        File file = new File(AnkiMediaProvider.directory(ctx), UUID.randomUUID() + extension);
        try (FileOutputStream out = new FileOutputStream(file)) {
            out.write(data);
        }
        Uri shared = AnkiMediaProvider.uriFor(file);
        try {
            ctx.grantUriPermission(ANKI_PACKAGE, shared, Intent.FLAG_GRANT_READ_URI_PERMISSION);

            ContentValues cv = new ContentValues();
            cv.put(MEDIA_FILE_URI, shared.toString());
            cv.put(MEDIA_PREFERRED_NAME, name.replaceAll("\\.[^.]*$", ""));

            Uri res = ctx.getContentResolver().insert(MEDIA_URI, cv);
            if (res != null && res.getPath() != null) {
                return new File(res.getPath()).getName();
            }
            throw new Exception("AnkiDroid failed to copy the media");
        } finally {
            try {
                ctx.revokeUriPermission(shared, Intent.FLAG_GRANT_READ_URI_PERMISSION);
            } catch (RuntimeException ignored) {
            }
            if (!file.delete()) file.deleteOnExit();
        }
    }

    private static String imageExtension(String lowerName) {
        if (lowerName.endsWith(".jpg")) return ".jpg";
        if (lowerName.endsWith(".jpeg")) return ".jpeg";
        if (lowerName.endsWith(".webp")) return ".webp";
        return ".png";
    }

    private static JSONArray handleMulti(Context ctx, JSONObject params) throws Exception {
        JSONArray acts = params.getJSONArray("actions");
        JSONArray res = new JSONArray();
        for (int i = 0; i < acts.length(); i++) {
            String subJson = acts.getJSONObject(i).toString();
            String subRes = processRequest(ctx, subJson);
            res.put(new JSONObject(subRes));
        }
        return res;
    }

    // ------------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------------

    private static String stripForChecksum(String data) {
        if (data == null) return "";
        String cleaned = IMG_TAG.matcher(data).replaceAll(" $1 ");
        cleaned = STYLE_TAG.matcher(cleaned).replaceAll("");
        cleaned = SCRIPT_TAG.matcher(cleaned).replaceAll("");
        cleaned = HTML_TAG.matcher(cleaned).replaceAll("");
        return cleaned.replace("&nbsp;", " ").trim();
    }

    private static long fieldChecksum(String data) {
        try {
            String cleaned = stripForChecksum(data);
            MessageDigest md = MessageDigest.getInstance("SHA-1");
            byte[] hash = md.digest(cleaned.getBytes(StandardCharsets.UTF_8));
            BigInteger big = new BigInteger(1, hash);
            String hex = big.toString(16);
            while (hex.length() < 40) hex = "0" + hex;
            return Long.parseLong(hex.substring(0, 8), 16);
        } catch (Exception e) {
            return 0;
        }
    }

    private static long findDeckId(Context ctx, String targetDeckName) throws Exception {
        for (int attempt = 0; attempt < 2; attempt++) {
            for (Map.Entry<Long, String> deck : decks(ctx, attempt > 0).entrySet()) {
                if (targetDeckName.equals(deck.getValue())) return deck.getKey();
            }
        }
        throw new Exception("Deck '" + targetDeckName + "' not found. Available decks can be queried with deckNames action.");
    }

    private static String[] parseFieldsInternal(String rawData) {
        if (rawData == null) return new String[0];
        String trimmed = rawData.trim();
        List<String> result = new ArrayList<>();

        if (trimmed.startsWith("[") && trimmed.endsWith("]")) {
            try {
                JSONArray jsonArray = new JSONArray(trimmed);
                for (int i = 0; i < jsonArray.length(); i++) {
                    Object item = jsonArray.get(i);
                    if (item instanceof JSONObject) {
                        result.add(((JSONObject) item).optString("name"));
                    } else {
                        result.add(item.toString());
                    }
                }
                return result.toArray(new String[0]);
            } catch (Exception e) {
                Log.e(TAG, "Field parse error", e);
            }
        }
        return splitFields(rawData);
    }

    private static String joinFields(String[] arr) {
        return String.join(FIELD_SEPARATOR, arr);
    }

    private static String[] splitFields(String str) {
        if (str == null) return new String[0];
        return str.split(FIELD_SEPARATOR, -1);
    }

    private static String joinTags(Set<String> tags) {
        return String.join(" ", tags);
    }

    private static String success(Object result) {
        String resStr = (result == null) ? "null" : result.toString();
        if (result != null && !(result instanceof JSONObject) && !(result instanceof JSONArray) && !(result instanceof Number) && !(result instanceof Boolean)
                && result != JSONObject.NULL) {
            resStr = JSONObject.quote(result.toString());
        }
        return "{\"result\":" + resStr + ",\"error\":null}";
    }

    private static String error(String msg) {
        // JSONObject.quote escapes backslashes, newlines and control characters too; the previous
        // quote-only escaping produced invalid JSON for many exception messages.
        return "{\"result\":null,\"error\":" + JSONObject.quote(msg == null ? "Unknown error" : msg) + "}";
    }
}
