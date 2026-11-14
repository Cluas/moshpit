package com.cluas.moshpit.term;

import java.util.ArrayList;
import java.util.List;
import java.util.Locale;

/** What the test hooks read of a terminal's transport: output counted since a reset, and echo times. */
public final class TerminalProbe {
    private TerminalProbe() {}

    public static void reset(TerminalViews.Terminal terminal) {
        terminal.transport.resetProbe();
    }

    /** Write-to-echo-drained times since the reset, in microseconds. */
    public static List<Long> echoes(TerminalViews.Terminal terminal) {
        synchronized (terminal.transport.echoMicros) {
            return new ArrayList<>(terminal.transport.echoMicros);
        }
    }

    /** Bytes since the reset, between the first and the last, and the channel's counters. */
    public static String report(TerminalViews.Terminal terminal) {
        ByteChannelTransport t = terminal.transport;
        long span = t.lastByteAt - t.firstByteAt;
        double mbps = span > 0 ? t.probeBytes / 1e6 / (span / 1000.0) : 0;
        return String.format(Locale.ROOT, "bytes=%d span=%dms rate=%.2fMB/s parse=%dms drains=%d slices=%d %s",
                t.probeBytes, span, mbps, t.parseNanos / 1_000_000, t.drains, t.slices, t.counters());
    }
}
