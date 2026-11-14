package com.cluas.moshpit.term;

import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.graphics.Typeface;
import android.util.Log;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.ViewGroup;
import android.view.inputmethod.InputMethodManager;

import com.termux.terminal.TerminalSession;
import com.termux.terminal.TerminalSessionClient;
import com.termux.view.TerminalView;
import com.termux.view.TerminalViewClient;

import java.util.HashMap;
import java.util.Map;

/**
 * The host's table of terminal views, by stream (`docs/nsh/DESIGN.md` §3.2: 终端视图常驻). NSH destroys a native
 * component's view when its element goes away (a page popped, a screen rebuilt); the terminal must keep its contents
 * and scrollback, so the view, its emulator and its byte channel live here, one per pane's stream, and a component
 * only borrows the view. A stream's terminal goes when the core says the pane is done for good
 * ({@link #release}, from {@code Platform.releaseStream}).
 */
public final class TerminalViews {
    private TerminalViews() {}

    /** One pane's terminal. */
    public static final class Terminal {
        public final long stream;
        public final TerminalView view;
        public final TerminalSession session;
        public final ByteChannelTransport transport;
        /** The colors the screen asked for (background, foreground, cursor, ANSI 0–15), applied once the emulator exists. */
        int[] colors;

        Terminal(Context context, long stream) {
            this.stream = stream;
            // The session first (its emulator comes with the view's first size), then the view.
            transport = new ByteChannelTransport(stream);
            session = new TerminalSession(transport, 5000, new SessionClient(this));
            view = new TerminalView(context, null);
            view.setTerminalViewClient(new ViewClient(this));
            // The text size before the typeface: it makes the renderer the typeface goes to.
            wear();
            view.setTypeface(Typeface.MONOSPACE);
            view.setFocusable(true);
            view.setFocusableInTouchMode(true);
            view.setKeepScreenOn(true);
            view.attachSession(session);
        }

        /** The terminals' look: its text size now, its colors once the emulator exists. */
        void wear() {
            colors = lookColors;
            view.setTextSize(Math.round(android.util.TypedValue.applyDimension(android.util.TypedValue.COMPLEX_UNIT_SP,
                    lookSize, view.getResources().getDisplayMetrics())));
            applyColors();
        }

        void applyColors() {
            if (colors == null || session.getEmulator() == null) return;
            int[] current = session.getEmulator().mColors.mCurrentColors;
            current[com.termux.terminal.TextStyle.COLOR_INDEX_BACKGROUND] = colors[0];
            current[com.termux.terminal.TextStyle.COLOR_INDEX_FOREGROUND] = colors[1];
            current[com.termux.terminal.TextStyle.COLOR_INDEX_CURSOR] = colors[2];
            for (int i = 0; i < 16 && i + 3 < colors.length; i++) current[i] = colors[i + 3];
            view.setBackgroundColor(colors[0]);
            view.invalidate();
        }

        public void focus() {
            view.requestFocus();
            InputMethodManager imm = (InputMethodManager) view.getContext().getSystemService(Context.INPUT_METHOD_SERVICE);
            if (imm != null) imm.showSoftInput(view, 0);
        }

        /**
         * The screen's text (not its scrollback), one line per row as drawn (a shell's prompt pads its row to the
         * edge, which reads as a wrapped line), trailing blanks dropped.
         */
        public String screenText() {
            com.termux.terminal.TerminalEmulator emulator = session.getEmulator();
            if (emulator == null) return "";
            String rows = emulator.getScreen().getSelectedText(0, 0, emulator.mColumns, emulator.mRows - 1, false, false);
            StringBuilder out = new StringBuilder();
            for (String row : rows.split("\n", -1)) {
                int end = row.length();
                while (end > 0 && row.charAt(end - 1) == ' ') end--;
                out.append(row, 0, end).append('\n');
            }
            return out.toString();
        }
    }

    private static final Map<Long, Terminal> terminals = new HashMap<>();
    /** The stream each terminal page's slot shows, by the page's surface. */
    private static final Map<Long, Long> shown = new HashMap<>();
    /** The slots on screen, by their page's surface. */
    private static final Map<Long, java.util.List<TerminalComponent>> slots = new HashMap<>();
    /** The sticky ctrl is armed for the next key (the bar's ctrl chip). Any thread writes it. */
    public static volatile boolean ctrlArmed;
    /** The terminals' look (Settings › Theme and Font Size): background, foreground, cursor, ANSI 0–15. */
    private static int[] lookColors;
    private static float lookSize = 12;

    /** Every terminal, and those made from now on, wears this look. Main thread. */
    public static void look(int[] colors, float fontSizePt) {
        lookColors = colors;
        lookSize = fontSizePt;
        for (Terminal terminal : terminals.values()) terminal.wear();
    }
    /** The terminal shown last (the test hooks' "current"). */
    private static Terminal current;

    /** Stream {@code stream}'s terminal, made on first use. Main thread. */
    public static Terminal of(Context context, long stream) {
        Terminal terminal = terminals.get(stream);
        if (terminal == null) {
            terminal = new Terminal(context.getApplicationContext() != null ? context : context, stream);
            terminals.put(stream, terminal);
            Log.i(ByteChannelTransport.TAG, "terminal for stream " + stream);
        }
        current = terminal;
        return terminal;
    }

    /** Page {@code surface}'s slot shows stream {@code stream} from now on. Main thread. */
    public static void show(Context context, long surface, long stream) {
        shown.put(surface, stream);
        for (TerminalComponent slot : new java.util.ArrayList<>(slots(surface))) slot.attach(stream);
    }

    static long shownBy(long surface) {
        Long stream = shown.get(surface);
        return stream == null ? 0 : stream;
    }

    static java.util.List<TerminalComponent> slots(long surface) {
        return slots.computeIfAbsent(surface, s -> new java.util.ArrayList<>());
    }

    /** Raise or put away the keyboard of the terminal on screen. */
    public static void keyboard(boolean show) {
        Terminal terminal = current;
        if (terminal == null) return;
        if (show) {
            terminal.focus();
        } else {
            InputMethodManager imm = (InputMethodManager) terminal.view.getContext().getSystemService(Context.INPUT_METHOD_SERVICE);
            if (imm != null) imm.hideSoftInputFromWindow(terminal.view.getWindowToken(), 0);
        }
    }

    /** The clipboard into the terminal on screen, as a paste. */
    public static void paste() {
        Terminal terminal = current;
        if (terminal != null && terminal.session.getEmulator() != null) {
            ClipboardManager clipboard = (ClipboardManager) terminal.view.getContext().getSystemService(Context.CLIPBOARD_SERVICE);
            if (clipboard == null || !clipboard.hasPrimaryClip()) return;
            CharSequence text = clipboard.getPrimaryClip().getItemAt(0).coerceToText(terminal.view.getContext());
            if (text != null) terminal.session.getEmulator().paste(text.toString());
        }
    }

    /** The terminal on screen last, if any. */
    public static Terminal current() {
        return current;
    }

    /** Stream {@code stream}'s terminal is done for good: its channel is let go and its view dropped. Main thread. */
    public static void release(long stream) {
        Terminal terminal = terminals.remove(stream);
        if (terminal == null) return;
        terminal.session.finishIfRunning();
        if (terminal.view.getParent() instanceof ViewGroup) ((ViewGroup) terminal.view.getParent()).removeView(terminal.view);
        if (current == terminal) current = null;
        shown.values().removeIf(s -> s == stream);
        Log.i(ByteChannelTransport.TAG, "terminal for stream " + stream + " released");
    }

    /** What the terminal view asks of its host. */
    private static final class ViewClient implements TerminalViewClient {
        private final Terminal terminal;
        private int columns, rows;

        ViewClient(Terminal terminal) {
            this.terminal = terminal;
        }

        @Override public float onScale(float scale) { return 1.0f; }
        @Override public void onSingleTapUp(MotionEvent e) { terminal.focus(); }
        @Override public boolean shouldBackButtonBeMappedToEscape() { return false; }
        @Override public boolean shouldEnforceCharBasedInput() { return false; }
        @Override public boolean shouldUseCtrlSpaceWorkaround() { return false; }
        @Override public boolean isTerminalViewSelected() { return true; }
        @Override public void copyModeChanged(boolean copyMode) {}
        @Override public boolean onKeyDown(int keyCode, KeyEvent e, TerminalSession session) { return false; }
        @Override public boolean onKeyUp(int keyCode, KeyEvent e) { return false; }
        @Override public boolean onLongPress(MotionEvent event) { return false; }
        @Override public boolean readControlKey() { return ctrlArmed; }
        @Override public boolean readAltKey() { return false; }
        @Override public boolean readShiftKey() { return false; }
        @Override public boolean readFnKey() { return false; }
        @Override
        public boolean onCodePoint(int codePoint, boolean ctrlDown, TerminalSession session) {
            if (ctrlDown && ctrlArmed) {
                // The sticky ctrl goes with this key; the bar hears it.
                ctrlArmed = false;
                com.cluas.moshpit.Core.ctrlUsed();
            }
            return false;
        }

        @Override
        public void onEmulatorSet() {
            terminal.applyColors();
            if (terminal.session.getEmulator() == null) return;
            int c = terminal.session.getEmulator().mColumns, r = terminal.session.getEmulator().mRows;
            if (c == columns && r == rows) return;
            columns = c;
            rows = r;
            Log.i(ByteChannelTransport.TAG, "stream " + terminal.stream + " size " + c + "x" + r);
        }

        @Override public void logError(String tag, String message) { Log.e(tag, message); }
        @Override public void logWarn(String tag, String message) { Log.w(tag, message); }
        @Override public void logInfo(String tag, String message) { Log.i(tag, message); }
        @Override public void logDebug(String tag, String message) { Log.d(tag, message); }
        @Override public void logVerbose(String tag, String message) { Log.v(tag, message); }
        @Override public void logStackTraceWithMessage(String tag, String message, Exception e) { Log.e(tag, message, e); }
        @Override public void logStackTrace(String tag, Exception e) { Log.e(tag, "", e); }
    }

    /** What the session tells its host. */
    private static final class SessionClient implements TerminalSessionClient {
        private final Terminal terminal;

        SessionClient(Terminal terminal) {
            this.terminal = terminal;
        }

        @Override public void onTextChanged(TerminalSession changedSession) { terminal.view.onScreenUpdated(); }
        @Override public void onTitleChanged(TerminalSession changedSession) {}
        @Override public void onSessionFinished(TerminalSession finishedSession) {}

        @Override
        public void onCopyTextToClipboard(TerminalSession session, String text) {
            ClipboardManager clipboard = (ClipboardManager) terminal.view.getContext().getSystemService(Context.CLIPBOARD_SERVICE);
            if (clipboard != null) clipboard.setPrimaryClip(ClipData.newPlainText("terminal", text));
        }

        @Override
        public void onPasteTextFromClipboard(TerminalSession session) {
            ClipboardManager clipboard = (ClipboardManager) terminal.view.getContext().getSystemService(Context.CLIPBOARD_SERVICE);
            if (clipboard == null || !clipboard.hasPrimaryClip()) return;
            CharSequence text = clipboard.getPrimaryClip().getItemAt(0).coerceToText(terminal.view.getContext());
            if (text != null && session.getEmulator() != null) session.getEmulator().paste(text.toString());
        }

        @Override public void onBell(TerminalSession session) {}
        @Override public void onColorsChanged(TerminalSession session) {}
        @Override public void onTerminalCursorStateChange(boolean state) {}
        @Override public Integer getTerminalCursorStyle() { return null; }

        @Override public void logError(String tag, String message) { Log.e(tag, message); }
        @Override public void logWarn(String tag, String message) { Log.w(tag, message); }
        @Override public void logInfo(String tag, String message) { Log.i(tag, message); }
        @Override public void logDebug(String tag, String message) { Log.d(tag, message); }
        @Override public void logVerbose(String tag, String message) { Log.v(tag, message); }
        @Override public void logStackTraceWithMessage(String tag, String message, Exception e) { Log.e(tag, message, e); }
        @Override public void logStackTrace(String tag, Exception e) { Log.e(tag, "", e); }
    }
}
