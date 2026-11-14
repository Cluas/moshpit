package com.cluas.moshpit;

import android.content.Context;
import android.content.SharedPreferences;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.Base64;
import android.util.Log;

import java.security.KeyStore;

import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;

/**
 * The app's keychain on Android (`crates/store`'s {@code Keychain}): secrets by account — passwords, private keys,
 * the trusted host keys — each sealed by {@link SecretBox} with an AES-256 key that lives in the Android Keystore and
 * never leaves it, the sealed bytes in the app's private {@code SharedPreferences} file {@code moshpit.keychain}.
 * <p>
 * Not yet: items behind the person's biometrics ({@code requireBiometry}, Face ID on iOS) need a second key with
 * {@code setUserAuthenticationRequired} and a {@code BiometricPrompt}, which needs a device with a lock screen; they
 * are sealed with the one key for now.
 */
final class Secrets {
    static final String ALIAS = "moshpit.keychain.v1";
    private static final String FILE = "moshpit.keychain";

    private final SharedPreferences items;

    Secrets(Context context) {
        items = context.getSharedPreferences(FILE, Context.MODE_PRIVATE);
    }

    private static synchronized SecretKey key() throws Exception {
        KeyStore store = KeyStore.getInstance("AndroidKeyStore");
        store.load(null);
        KeyStore.Entry entry = store.getEntry(ALIAS, null);
        if (entry instanceof KeyStore.SecretKeyEntry) return ((KeyStore.SecretKeyEntry) entry).getSecretKey();
        KeyGenerator generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
        generator.init(new KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build());
        return generator.generateKey();
    }

    /** Null when saved; else why not (the item as it was). */
    String save(String account, byte[] secret) {
        try {
            byte[] box = SecretBox.seal(key(), account, secret);
            boolean written = items.edit().putString(account, Base64.encodeToString(box, Base64.NO_WRAP)).commit();
            return written ? null : "the keychain file was not written";
        } catch (Exception e) {
            Log.w(MainActivity.TAG, "keychain save failed: " + e);
            return e.toString();
        }
    }

    /** The item's bytes, or null when there is none; throws when it can't be opened. */
    byte[] load(String account) throws Exception {
        String stored = items.getString(account, null);
        if (stored == null) return null;
        return SecretBox.open(key(), account, Base64.decode(stored, Base64.NO_WRAP));
    }

    String delete(String account) {
        return items.edit().remove(account).commit() ? null : "the keychain file was not written";
    }

    boolean exists(String account) {
        return items.contains(account);
    }
}
