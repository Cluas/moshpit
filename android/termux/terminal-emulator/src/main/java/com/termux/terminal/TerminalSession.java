package com.termux.terminal;

import java.util.UUID;

/**
 * A terminal session: a {@link TerminalEmulator} coupled to a {@link Transport} that carries its bytes.
 * <p>
 * Moshpit's version of Termux's class. Termux runs a local process on a pseudo-terminal, reads it on a thread of
 * its own and hands the output to the main thread one message per read. Here the bytes come from and go to the
 * app's core (a byte channel per pane, see the Moshpit spike's {@code ByteChannelTransport}): the transport calls
 * {@link #append} on the main thread, in order, and the session writes keystrokes, pastes and the emulator's
 * replies to the transport synchronously. There is no process, no pty and no thread here.
 * <p>
 * The public surface {@code TerminalView} and the text selection use is Termux's.
 */
public final class TerminalSession extends TerminalOutput {

    /** Carries a session's bytes: what the user types to the core, what the core sends to the emulator. */
    public interface Transport {
        /**
         * The session's emulator exists (its first size is known): output may start. The transport hands it to
         * {@link TerminalSession#append} on the main thread from now on, in order.
         */
        void start(TerminalSession session);

        /** Bytes for the other end (keystrokes, a paste, a reply of the emulator), in order, on the main thread. */
        void write(byte[] data, int offset, int count);

        /** The emulator's size changed (and on start, before {@link #start}). */
        void resize(int columns, int rows, int cellWidthPixels, int cellHeightPixels);

        /** The session is done: let go of the channel. */
        void close();
    }

    public final String mHandle = UUID.randomUUID().toString();

    TerminalEmulator mEmulator;

    /** Buffer to write translate code points into utf8 before writing to the transport */
    private final byte[] mUtf8InputBuffer = new byte[5];

    /** Callback which gets notified when a session finishes or changes title. */
    TerminalSessionClient mClient;

    /** Set by the application for user identification of session, not by terminal. */
    public String mSessionName;

    private final Transport mTransport;
    private final Integer mTranscriptRows;
    private boolean mClosed;

    public TerminalSession(Transport transport, Integer transcriptRows, TerminalSessionClient client) {
        this.mTransport = transport;
        this.mTranscriptRows = transcriptRows;
        this.mClient = client;
    }

    /**
     * @param client The {@link TerminalSessionClient} interface implementation to allow
     *               for communication between {@link TerminalSession} and its client.
     */
    public void updateTerminalSessionClient(TerminalSessionClient client) {
        mClient = client;

        if (mEmulator != null)
            mEmulator.updateTerminalSessionClient(client);
    }

    /** Tell the transport the new size and reflow or initialize the emulator. */
    public void updateSize(int columns, int rows, int cellWidthPixels, int cellHeightPixels) {
        if (mClosed) return;
        if (mEmulator == null) {
            mEmulator = new TerminalEmulator(this, columns, rows, cellWidthPixels, cellHeightPixels, mTranscriptRows, mClient);
            mTransport.resize(columns, rows, cellWidthPixels, cellHeightPixels);
            mTransport.start(this);
        } else {
            mEmulator.resize(columns, rows, cellWidthPixels, cellHeightPixels);
            mTransport.resize(columns, rows, cellWidthPixels, cellHeightPixels);
        }
    }

    /** The terminal title as set through escape sequences or null if none set. */
    public String getTitle() {
        return (mEmulator == null) ? null : mEmulator.getTitle();
    }

    /**
     * Output of the other end, for the emulator: called by the transport on the main thread, in order. Call
     * {@link #notifyScreenUpdate()} once after a run of appends.
     */
    public void append(byte[] data, int length) {
        if (mEmulator != null && length > 0) mEmulator.append(data, length);
    }

    /** Write data to the other end. */
    @Override
    public void write(byte[] data, int offset, int count) {
        if (!mClosed && count > 0) mTransport.write(data, offset, count);
    }

    /** Write the Unicode code point to the terminal encoded in UTF-8. */
    public void writeCodePoint(boolean prependEscape, int codePoint) {
        if (codePoint > 1114111 || (codePoint >= 0xD800 && codePoint <= 0xDFFF)) {
            // 1114111 (= 2**16 + 1024**2 - 1) is the highest code point, [0xD800,0xDFFF] is the surrogate range.
            throw new IllegalArgumentException("Invalid code point: " + codePoint);
        }

        int bufferPosition = 0;
        if (prependEscape) mUtf8InputBuffer[bufferPosition++] = 27;

        if (codePoint <= /* 7 bits */0b1111111) {
            mUtf8InputBuffer[bufferPosition++] = (byte) codePoint;
        } else if (codePoint <= /* 11 bits */0b11111111111) {
            /* 110xxxxx leading byte with leading 5 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b11000000 | (codePoint >> 6));
            /* 10xxxxxx continuation byte with following 6 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b10000000 | (codePoint & 0b111111));
        } else if (codePoint <= /* 16 bits */0b1111111111111111) {
            /* 1110xxxx leading byte with leading 4 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b11100000 | (codePoint >> 12));
            /* 10xxxxxx continuation byte with following 6 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b10000000 | ((codePoint >> 6) & 0b111111));
            /* 10xxxxxx continuation byte with following 6 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b10000000 | (codePoint & 0b111111));
        } else { /* We have checked codePoint <= 1114111 above, so we have max 21 bits = 0b111111111111111111111 */
            /* 11110xxx leading byte with leading 3 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b11110000 | (codePoint >> 18));
            /* 10xxxxxx continuation byte with following 6 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b10000000 | ((codePoint >> 12) & 0b111111));
            /* 10xxxxxx continuation byte with following 6 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b10000000 | ((codePoint >> 6) & 0b111111));
            /* 10xxxxxx continuation byte with following 6 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b10000000 | (codePoint & 0b111111));
        }
        write(mUtf8InputBuffer, 0, bufferPosition);
    }

    public TerminalEmulator getEmulator() {
        return mEmulator;
    }

    /** Notify the {@link #mClient} that the screen has changed. */
    public void notifyScreenUpdate() {
        mClient.onTextChanged(this);
    }

    /** Reset state for terminal emulator state. */
    public void reset() {
        mEmulator.reset();
        notifyScreenUpdate();
    }

    /** Let go of the transport; the emulator keeps what it shows. */
    public void finishIfRunning() {
        if (mClosed) return;
        mClosed = true;
        mTransport.close();
    }

    @Override
    public void titleChanged(String oldTitle, String newTitle) {
        mClient.onTitleChanged(this);
    }

    public synchronized boolean isRunning() {
        return !mClosed;
    }

    /** There is no local process: always 0. */
    public synchronized int getExitStatus() {
        return 0;
    }

    @Override
    public void onCopyTextToClipboard(String text) {
        mClient.onCopyTextToClipboard(this, text);
    }

    @Override
    public void onPasteTextFromClipboard() {
        mClient.onPasteTextFromClipboard(this);
    }

    @Override
    public void onBell() {
        mClient.onBell(this);
    }

    @Override
    public void onColorsChanged() {
        mClient.onColorsChanged(this);
    }

    /** There is no local process: always 0. */
    public int getPid() {
        return 0;
    }

    /** There is no local process: the working directory is the other end's, unknown here. */
    public String getCwd() {
        return null;
    }

}
