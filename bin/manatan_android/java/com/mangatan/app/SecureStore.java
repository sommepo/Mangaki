package dev.local.mangareader;

import android.content.Context;
import android.content.SharedPreferences;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.Base64;

import java.nio.charset.StandardCharsets;
import java.security.GeneralSecurityException;
import java.security.KeyStore;
import javax.crypto.AEADBadTagException;
import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;

/**
 * Small Android-Keystore-backed credential store. Values never leave app-private storage, and the
 * preference file is excluded from Auto Backup and device transfer (res/xml backup rules) because
 * the hardware-bound key never travels with it.
 *
 * <p>JNI contract (fork_api.rs): {@link #put}, {@link #get}, {@link #delete}.
 */
public final class SecureStore {
    private static final String KEY_ALIAS = "manga_reader_fork_source_credentials_v1";
    private static final String PREFS = "manga_reader_fork_secrets";
    private static final String TRANSFORMATION = "AES/GCM/NoPadding";
    private static final int TAG_BITS = 128;

    private static SecretKey cachedKey;

    private SecureStore() {}

    /**
     * Synchronized so two first-use callers cannot both generate the key: the second
     * generateKey() would replace the first and make anything encrypted with it unreadable.
     */
    private static synchronized SecretKey key() throws Exception {
        if (cachedKey != null) return cachedKey;
        KeyStore keyStore = KeyStore.getInstance("AndroidKeyStore");
        keyStore.load(null);
        KeyStore.Entry entry = keyStore.containsAlias(KEY_ALIAS) ? keyStore.getEntry(KEY_ALIAS, null) : null;
        if (entry instanceof KeyStore.SecretKeyEntry) {
            cachedKey = ((KeyStore.SecretKeyEntry) entry).getSecretKey();
            return cachedKey;
        }
        KeyGenerator generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
        generator.init(new KeyGenParameterSpec.Builder(KEY_ALIAS,
                KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .setRandomizedEncryptionRequired(true)
                .build());
        cachedKey = generator.generateKey();
        return cachedKey;
    }

    public static void put(Context context, String id, String value) throws Exception {
        if (id == null || value == null) throw new IllegalArgumentException("Credential id and value are required");
        Cipher cipher = Cipher.getInstance(TRANSFORMATION);
        // The keystore chooses a fresh random IV per encryption (randomized encryption required).
        cipher.init(Cipher.ENCRYPT_MODE, key());
        byte[] ciphertext = cipher.doFinal(value.getBytes(StandardCharsets.UTF_8));
        String encoded = Base64.encodeToString(cipher.getIV(), Base64.NO_WRAP) + "."
                + Base64.encodeToString(ciphertext, Base64.NO_WRAP);
        // commit(): the caller reports success to the user, so the write must be durable now.
        if (!prefs(context).edit().putString(id, encoded).commit()) {
            throw new IllegalStateException("Could not save the credential");
        }
    }

    public static String get(Context context, String id) throws Exception {
        String encoded = prefs(context).getString(id, null);
        if (encoded == null) return null;
        String[] parts = encoded.split("\\.", 2);
        if (parts.length != 2) throw new IllegalStateException("Invalid encrypted credential record");
        SecretKey key = key();
        try {
            Cipher cipher = Cipher.getInstance(TRANSFORMATION);
            cipher.init(Cipher.DECRYPT_MODE, key,
                    new GCMParameterSpec(TAG_BITS, Base64.decode(parts[0], Base64.NO_WRAP)));
            return new String(cipher.doFinal(Base64.decode(parts[1], Base64.NO_WRAP)), StandardCharsets.UTF_8);
        } catch (AEADBadTagException error) {
            throw new IllegalStateException("Saved credentials can no longer be decrypted; please sign in again", error);
        } catch (IllegalArgumentException | GeneralSecurityException error) {
            throw new IllegalStateException("Saved credentials are unreadable; please sign in again", error);
        }
    }

    public static void delete(Context context, String id) {
        prefs(context).edit().remove(id).commit();
    }

    private static SharedPreferences prefs(Context context) {
        return context.getApplicationContext().getSharedPreferences(PREFS, Context.MODE_PRIVATE);
    }
}
