package com.cluas.moshpit;

/**
 * The app's Rust half ({@code libnsh.so}, crates/android): its controller's entry points, through hand-written JNI
 * (crates/android/src/android/mod.rs). The controller owns the stores, the sessions and the page stack; this side
 * shows what it asks for ({@link HostPlatform}) and tells it what the system and the person do.
 * <p>
 * Every call returns at once. A page runs this way: the controller asks {@link HostPlatform#push} for a page; the
 * host makes an {@code NshView}, calls {@link #prepare} with the page's surface and then {@code runRustApp}, which
 * calls into Rust on the same thread and runs the surface's screen.
 */
public final class Core {
    static {
        System.loadLibrary("nsh");
    }

    private Core() {}

    /** Starts the controller (again, with the pages anew, when the activity was made anew) and shows Home. */
    static native void start(HostPlatform platform, String version, String deviceLabel, boolean debuggable);

    /** The next {@code runRustApp} on this thread runs surface {@code surface}. */
    static native void prepare(long surface);

    /** Surface {@code surface}'s view is gone (destroyed). */
    static native void gone(long surface);

    /** The system's back: true when the app took it. */
    static native boolean back();

    /** A dialog's answer: OK or not, and a prompt's text. */
    static native void answer(long token, boolean ok, String text);

    /** The app came to the foreground (true) or left it. */
    static native void lifecycle(boolean foreground);

    /** The network changed (a new default route, a link back). */
    static native void networkChanged();

    /** The person unlocked the device. */
    static native void unlocked();

    /** Text shared to the app from another one. */
    static native void share(String text);

    /** The private key ({@code pem}) or its passphrase ({@code passphrase}) as typed: to Add Key, nowhere else. */
    static native void keyField(String kind, String text);

    /** The file the document picker was asked for under {@code token}: its bytes, or null (none picked, unreadable). */
    static native void picked(long token, byte[] bytes);

    /** The sticky ctrl went with a key: the bar's chip comes off. */
    public static native void ctrlUsed();

    /** A debuggable build's test hook; its answer as JSON. */
    static native String debug(String json);
}
