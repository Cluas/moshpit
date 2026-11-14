package com.cluas.moshpit;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import android.util.Log;
import android.view.KeyEvent;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;

import com.cluas.moshpit.term.TerminalProbe;
import com.cluas.moshpit.term.TerminalViews;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Locale;

/**
 * The test hooks of a debuggable build, for android/test.sh and android/live.sh over
 * {@code adb shell am broadcast -a com.cluas.moshpit.TEST}. Answers go to logcat under {@code MoshpitTest}.
 * <p>
 * {@code --es json '{"cmd": …}'} goes to the controller ({@link Core#debug}); {@code --es cmd …} is the view's:
 * {@code screen} (the terminal's rows), {@code focus}, {@code keys} (a key with a repeat count, as a held key sends
 * it), {@code ime} (an input method's composition on the view's own InputConnection), {@code echo} (keystroke to
 * echo latency), {@code probe-reset} / {@code probe} (output counted for a burst), {@code dismiss} (the dialogs),
 * {@code pick-path} (the document picker's answer: a file of the app's own storage).
 */
final class DebugReceiver extends BroadcastReceiver {
    static final String ACTION = "com.cluas.moshpit.TEST";
    static final String TAG = "MoshpitTest";

    private final MainActivity activity;
    private final Handler main = new Handler(Looper.getMainLooper());

    private DebugReceiver(MainActivity activity) {
        this.activity = activity;
    }

    static DebugReceiver register(MainActivity activity) {
        DebugReceiver receiver = new DebugReceiver(activity);
        IntentFilter filter = new IntentFilter(ACTION);
        if (Build.VERSION.SDK_INT >= 33) activity.registerReceiver(receiver, filter, Context.RECEIVER_EXPORTED);
        else activity.registerReceiver(receiver, filter);
        Log.i(TAG, "test hooks ready");
        return receiver;
    }

    @Override
    public void onReceive(Context context, Intent intent) {
        String json = intent.getStringExtra("json");
        if (json != null) {
            String answer = Core.debug(json);
            Log.i(TAG, "answer " + answer);
            if (json.contains("\"answer-all\"")) activity.dismissDialogs();
            return;
        }
        String cmd = intent.getStringExtra("cmd");
        if (cmd == null) return;
        TerminalViews.Terminal terminal = TerminalViews.current();
        switch (cmd) {
            case "dismiss":
                activity.dismissDialogs();
                Log.i(TAG, "dismissed");
                break;
            case "focus":
                if (terminal != null) terminal.focus();
                Log.i(TAG, "focus " + (terminal != null && terminal.view.hasFocus()));
                break;
            case "screen":
                if (terminal == null) {
                    Log.i(TAG, "screen none");
                    break;
                }
                for (String row : terminal.screenText().split("\n")) Log.i(TAG, "screen| " + row);
                Log.i(TAG, "screen end stream=" + terminal.stream);
                break;
            case "keys":
                keys(terminal, intent.getIntExtra("code", KeyEvent.KEYCODE_DEL), intent.getIntExtra("repeat", 0));
                break;
            case "ime":
                ime(terminal);
                break;
            case "echo":
                echo(terminal, intent.getIntExtra("count", 40), intent.getIntExtra("gap", 150));
                break;
            case "probe-reset":
                if (terminal != null) TerminalProbe.reset(terminal);
                Log.i(TAG, "probe reset");
                break;
            case "pick-path": {
                // The document picker's answer without its UI: a file in the app's own storage.
                java.io.File file = new java.io.File(intent.getStringExtra("path"));
                long token = activity.lastPickToken;
                new Thread(() -> Core.picked(token, activity.read(android.net.Uri.fromFile(file)))).start();
                Log.i(TAG, "picked " + file + " for " + token);
                break;
            }
            case "probe":
                Log.i(TAG, "probe " + (terminal == null ? "none" : TerminalProbe.report(terminal)));
                break;
            default:
                Log.w(TAG, "no hook " + cmd);
        }
    }

    /** A key held down: its down events with repeat counts 0…{@code repeat}, then its up. */
    private void keys(TerminalViews.Terminal terminal, int code, int repeat) {
        if (terminal == null) return;
        long down = SystemClock.uptimeMillis();
        for (int i = 0; i <= repeat; i++) {
            terminal.view.dispatchKeyEvent(new KeyEvent(down, SystemClock.uptimeMillis(), KeyEvent.ACTION_DOWN, code, i));
        }
        terminal.view.dispatchKeyEvent(new KeyEvent(down, SystemClock.uptimeMillis(), KeyEvent.ACTION_UP, code, 0));
        Log.i(TAG, "keys " + code + " x" + (repeat + 1));
    }

    /** An IME's composition, as a Chinese pinyin keyboard sends it: only the commits reach the core. */
    private void ime(TerminalViews.Terminal terminal) {
        if (terminal == null) return;
        InputConnection ic = terminal.view.onCreateInputConnection(new EditorInfo());
        if (ic == null) {
            Log.i(TAG, "IME none");
            return;
        }
        ic.setComposingText("n", 1);
        ic.setComposingText("ni", 1);
        ic.setComposingText("nihao", 1);
        ic.commitText("你好", 1);
        ic.setComposingText("m", 1);
        ic.commitText("ma", 1);
        ic.setComposingText("s", 1);
        ic.setComposingText("sh", 1);
        ic.commitText("是", 1);
        ic.finishComposingText();
        Log.i(TAG, "IME done");
    }

    /**
     * Keystroke-to-echo latency: {@code count} single keys typed at a shell prompt, {@code gap} ms apart, each timed
     * from the write into the byte channel to the drain that brought its echo back; then Ctrl-U clears the line.
     */
    private void echo(TerminalViews.Terminal terminal, int count, int gap) {
        if (terminal == null) return;
        TerminalProbe.reset(terminal);
        Runnable[] step = new Runnable[1];
        int[] sent = {0};
        step[0] = () -> {
            if (sent[0] >= count) {
                terminal.session.write(new byte[] {0x15}, 0, 1);
                main.postDelayed(() -> {
                    List<Long> micros = new ArrayList<>(TerminalProbe.echoes(terminal));
                    Collections.sort(micros);
                    Log.i(TAG, "echo " + summary(micros));
                }, 500);
                return;
            }
            sent[0]++;
            byte key = (byte) ('a' + sent[0] % 26);
            terminal.session.write(new byte[] {key}, 0, 1);
            main.postDelayed(step[0], gap);
        };
        main.post(step[0]);
    }

    static String summary(List<Long> sorted) {
        if (sorted.isEmpty()) return "n=0";
        return String.format(Locale.ROOT, "n=%d p50=%.2fms p95=%.2fms max=%.2fms", sorted.size(),
                sorted.get(sorted.size() / 2) / 1000.0,
                sorted.get(Math.min(sorted.size() - 1, (int) Math.ceil(sorted.size() * 0.95) - 1)) / 1000.0,
                sorted.get(sorted.size() - 1) / 1000.0);
    }
}
