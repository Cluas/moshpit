package com.cluas.moshpit.term;

import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import android.util.Log;

import com.termux.terminal.TerminalSession;

import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicLong;

/**
 * A terminal session's transport over one pane's byte channel (gate G3's design, kept).
 * <p>
 * Output: the core's waker ({@link #wake}, any thread) posts one drain to the main thread; at most one is ever
 * pending. The drain reads until the queue is empty and appends every chunk to the emulator in order, then tells
 * the view once. A long backlog is drained in slices of {@link #SLICE_MS} so frames keep coming: a slice that ends
 * before the queue is empty posts the next one itself (the core does not wake again until a read has emptied the
 * queue), so there is still one pending drain and no polling.
 * <p>
 * Input: keystrokes, pastes and IME commits go to the core synchronously, on the main thread, as they come.
 */
public final class ByteChannelTransport implements TerminalSession.Transport {
    static final String TAG = "MoshpitView";
    /** The longest a drain runs before it lets a frame through. */
    static final long SLICE_MS = 8;

    final long stream;
    private final Handler main = new Handler(Looper.getMainLooper());
    private final byte[] buffer = new byte[64 * 1024];
    /** A drain is posted and has not started. */
    private final AtomicBoolean posted = new AtomicBoolean();
    private final Runnable drain = this::drain;
    private TerminalSession session;
    private volatile boolean closed;

    /** Counters for the test hooks: wakes on the core's threads, the rest on the main thread. */
    final AtomicLong wakes = new AtomicLong();
    long drains, slices, bytes;
    /** The time of the first and the last output byte since {@link #resetProbe} (uptime ms; 0: none). */
    long firstByteAt, lastByteAt, probeBytes;
    /** Main-thread time spent in the emulator's parser ({@code append}) since the reset, ns. */
    long parseNanos;
    /** The time of the last write whose echo has not been seen yet (0: none), and the echo times seen. */
    private long writeNanos;
    final java.util.ArrayList<Long> echoMicros = new java.util.ArrayList<>();

    ByteChannelTransport(long stream) {
        this.stream = stream;
    }

    @Override
    public void start(TerminalSession session) {
        this.session = session;
        if (!ByteChannel.attach(stream, this)) Log.w(TAG, "stream " + stream + " is gone");
    }

    /** The core has output waiting: called by the core's waker, on any thread. */
    @SuppressWarnings("unused") // Called from Rust (stream.rs).
    void wake() {
        if (closed) return;
        wakes.incrementAndGet();
        if (posted.compareAndSet(false, true)) main.post(drain);
    }

    private void drain() {
        posted.set(false);
        TerminalSession s = session;
        if (closed || s == null) return;
        drains++;
        long deadline = SystemClock.uptimeMillis() + SLICE_MS;
        long before = bytes;
        boolean empty;
        while (true) {
            int read = ByteChannel.read(stream, buffer);
            int n = ByteChannel.bytesOf(read);
            if (n > 0) {
                bytes += n;
                long now = SystemClock.uptimeMillis();
                if (firstByteAt == 0) firstByteAt = now;
                lastByteAt = now;
                probeBytes += n;
                long parsing = System.nanoTime();
                s.append(buffer, n);
                parseNanos += System.nanoTime() - parsing;
            }
            if (!ByteChannel.marked(read) && n < buffer.length) {
                empty = true;
                break;
            }
            if (SystemClock.uptimeMillis() >= deadline) {
                empty = false;
                break;
            }
        }
        slices++;
        s.notifyScreenUpdate();
        // An echo is output: a drain that only passed marks is not one.
        if (writeNanos != 0 && bytes > before) {
            long micros = (System.nanoTime() - writeNanos) / 1000;
            writeNanos = 0;
            synchronized (echoMicros) {
                echoMicros.add(micros);
            }
        }
        // Not empty: the core does not wake again until a read empties the queue, so the next slice is ours to post.
        if (!empty && posted.compareAndSet(false, true)) main.post(drain);
    }

    @Override
    public void write(byte[] data, int offset, int count) {
        if (closed) return;
        writeNanos = System.nanoTime();
        if (!ByteChannel.write(stream, data, offset, count)) Log.w(TAG, "write refused: stream " + stream + " closed");
    }

    @Override
    public void resize(int columns, int rows, int cellWidthPixels, int cellHeightPixels) {
        if (closed) return;
        ByteChannel.resize(stream, columns, rows);
    }

    @Override
    public void close() {
        closed = true;
        main.removeCallbacks(drain);
        ByteChannel.detach(stream);
    }

    /** Starts counting output afresh (the burst probe). */
    void resetProbe() {
        firstByteAt = 0;
        lastByteAt = 0;
        probeBytes = 0;
        parseNanos = 0;
        drains = 0;
        slices = 0;
        synchronized (echoMicros) {
            echoMicros.clear();
        }
    }

    String counters() {
        long[] core = ByteChannel.stats(stream);
        String rust = core == null ? "gone" : String.format(java.util.Locale.ROOT,
            "pushes=%d pushed=%d read=%d queued=%d peak=%d wakes=%d drains=%d",
            core[0], core[1], core[2], core[3], core[4], core[5], core[6]);
        return "view: wakes=" + wakes.get() + " drains=" + drains + " slices=" + slices + " bytes=" + bytes + " | core: " + rust;
    }
}
