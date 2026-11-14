package com.cluas.moshpit;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import java.nio.charset.StandardCharsets;
import java.security.GeneralSecurityException;

import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;

import org.junit.Test;

/** The keychain's sealing, with a software AES key in place of the Keystore's. */
public class SecretBoxTest {
    private static SecretKey key() throws Exception {
        KeyGenerator generator = KeyGenerator.getInstance("AES");
        generator.init(256);
        return generator.generateKey();
    }

    private static final byte[] SECRET = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3Blbg==\n".getBytes(StandardCharsets.UTF_8);

    @Test
    public void aSealedSecretOpensForItsAccount() throws Exception {
        SecretKey key = key();
        byte[] box = SecretBox.seal(key, "moshpit.secret.1", SECRET);
        assertEquals(SecretBox.VERSION, box[0]);
        assertEquals(1 + SecretBox.IV_BYTES + SECRET.length + 16, box.length);
        assertArrayEquals(SECRET, SecretBox.open(key, "moshpit.secret.1", box));
    }

    @Test
    public void twoSealsOfOneSecretDiffer() throws Exception {
        SecretKey key = key();
        byte[] a = SecretBox.seal(key, "a", SECRET), b = SecretBox.seal(key, "a", SECRET);
        assertTrue("a fresh IV each time", !java.util.Arrays.equals(a, b));
    }

    @Test
    public void anItemMovedToAnotherAccountDoesNotOpen() throws Exception {
        SecretKey key = key();
        byte[] box = SecretBox.seal(key, "moshpit.secret.1", SECRET);
        refused(key, "moshpit.secret.2", box);
    }

    @Test
    public void aTamperedOrForeignItemDoesNotOpen() throws Exception {
        SecretKey key = key();
        byte[] box = SecretBox.seal(key, "a", SECRET);
        byte[] flipped = box.clone();
        flipped[flipped.length - 1] ^= 1;
        refused(key, "a", flipped);
        refused(key(), "a", box);
        byte[] versioned = box.clone();
        versioned[0] = 2;
        refused(key, "a", versioned);
        refused(key, "a", new byte[5]);
        refused(key, "a", null);
    }

    @Test
    public void anEmptySecretSealsToo() throws Exception {
        SecretKey key = key();
        assertArrayEquals(new byte[0], SecretBox.open(key, "a", SecretBox.seal(key, "a", new byte[0])));
    }

    private static void refused(SecretKey key, String account, byte[] box) {
        try {
            SecretBox.open(key, account, box);
            fail("opened");
        } catch (GeneralSecurityException expected) {
            // As it should.
        }
    }
}
