package com.cluas.moshpit.term;

/**
 * The view's end of a pane's byte channel (crates/stream), through JNI into {@code libnsh.so}
 * (crates/android/src/android/stream.rs): the hot path, no JSON, no polling.
 * <p>
 * Output: {@link #attach} gives the core a waker ({@link ByteChannelTransport#wake}), called on any thread at most
 * once until the view has drained the queue. {@link #read} says what it read: {@code n >= 0} bytes and no mark
 * (drained when fewer than the buffer), or {@code -(n + 1)}: {@code n} bytes and then a mark, which the core handles,
 * so read on. Input: {@link #write} and {@link #resize} append to the core's input, synchronously, in the order they
 * are called.
 */
public final class ByteChannel {
    private ByteChannel() {}

    /** Makes {@code transport.wake()} (a {@link ByteChannelTransport}) the waker of {@code stream}; false if the stream is gone. */
    static native boolean attach(long stream, Object transport);

    /** Drops the waker: once this returns it is not called again. */
    static native void detach(long stream);

    /** Moves up to {@code buffer.length} bytes of output into {@code buffer}, oldest first (see the class). */
    static native int read(long stream, byte[] buffer);

    /** Appends {@code count} bytes at {@code offset} to the input after everything written before. */
    public static native boolean write(long stream, byte[] data, int offset, int count);

    /** The view's size in cells, after the bytes written so far. */
    static native boolean resize(long stream, int columns, int rows);

    /** The output's counters: pushes, pushed, read, queued, peak queued, wakes, drains; null if gone. */
    public static native long[] stats(long stream);

    /** What a read returned: the bytes it moved. */
    static int bytesOf(int read) {
        return read >= 0 ? read : -read - 1;
    }

    /** Whether a read stopped at a mark (read on). */
    static boolean marked(int read) {
        return read < 0;
    }
}
