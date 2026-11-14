package org.junit;

import java.util.Arrays;
import java.util.Objects;

/** JUnit 4's assertions the host's unit tests use: a stand-in for this machine (no JUnit here). */
public final class Assert {
    private Assert() {}

    public static void fail(String message) {
        throw new AssertionError(message);
    }

    public static void assertTrue(boolean condition) {
        if (!condition) fail("expected true");
    }

    public static void assertTrue(String message, boolean condition) {
        if (!condition) fail(message);
    }

    public static void assertFalse(boolean condition) {
        if (condition) fail("expected false");
    }

    public static void assertNull(Object value) {
        if (value != null) fail("expected null, got " + value);
    }

    public static void assertNotNull(Object value) {
        if (value == null) fail("expected a value");
    }

    public static void assertEquals(Object expected, Object actual) {
        if (!Objects.equals(expected, actual)) fail("expected <" + expected + "> but was <" + actual + ">");
    }

    public static void assertEquals(long expected, long actual) {
        if (expected != actual) fail("expected <" + expected + "> but was <" + actual + ">");
    }

    public static void assertArrayEquals(byte[] expected, byte[] actual) {
        if (!Arrays.equals(expected, actual)) fail("arrays differ");
    }
}
