package com.cluas.moshpit;

import android.app.AlertDialog;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.net.Uri;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.os.UserManager;
import android.text.InputType;
import android.util.Log;
import android.widget.EditText;
import android.widget.Toast;

import com.cluas.moshpit.term.TerminalViews;

/**
 * What the Rust controller asks of the host (crates/android/src/platform.rs's {@code Platform}, through
 * crates/android/src/android/java.rs). Called on any thread: the preferences and the keychain answer at once (both
 * are thread-safe), everything that shows something hops to the main thread and returns.
 * <p>
 * Never throws into Rust except where the Rust side reads the exception as the error ({@link #secretLoad},
 * {@link #hardwareGenerate}, {@link #hardwareSign}).
 */
final class HostPlatform {
    private final MainActivity activity;
    private final Context app;
    private final Handler main = new Handler(Looper.getMainLooper());
    /** The store's preferences (`UserDefaults.standard` on iOS), as {@code crates/android/src/prefs.rs} encodes them. */
    private final SharedPreferences prefs;
    private final Secrets secrets;

    HostPlatform(MainActivity activity) {
        this.activity = activity;
        app = activity.getApplicationContext();
        prefs = app.getSharedPreferences("moshpit.defaults", Context.MODE_PRIVATE);
        secrets = new Secrets(app);
    }

    // --- Preferences ---

    String prefGet(String key) {
        return prefs.getString(key, null);
    }

    void prefSet(String key, String value) {
        SharedPreferences.Editor editor = prefs.edit();
        if (value == null) editor.remove(key);
        else editor.putString(key, value);
        editor.apply();
    }

    boolean userUnlocked() {
        if (Build.VERSION.SDK_INT < 24) return true;
        UserManager users = app.getSystemService(UserManager.class);
        return users == null || users.isUserUnlocked();
    }

    // --- The keychain ---

    String secretSave(String account, byte[] secret, boolean biometry) {
        return secrets.save(account, secret);
    }

    byte[] secretLoad(String account) throws Exception {
        return secrets.load(account);
    }

    String secretDelete(String account) {
        return secrets.delete(account);
    }

    boolean secretExists(String account) {
        return secrets.exists(account);
    }

    // --- Hardware keys ---

    byte[] hardwareGenerate(String alias) throws Exception {
        return HardwareKeys.generate(alias);
    }

    byte[] hardwareSign(String alias, byte[] data) throws Exception {
        return HardwareKeys.sign(alias, data);
    }

    void hardwareDelete(String alias) {
        HardwareKeys.delete(alias);
    }

    // --- The page stack ---

    void push(long surface, boolean animated) {
        main.post(() -> activity.pages().push(surface, animated));
    }

    void pop(long surface, boolean animated) {
        main.post(() -> activity.pages().pop(surface, animated));
    }

    void finish() {
        main.post(() -> activity.moveTaskToBack(true));
    }

    // --- The system ---

    void copyText(String text) {
        main.post(() -> {
            ClipboardManager clipboard = (ClipboardManager) app.getSystemService(Context.CLIPBOARD_SERVICE);
            if (clipboard != null) clipboard.setPrimaryClip(ClipData.newPlainText("Moshpit", text));
        });
    }

    void shareText(String text) {
        main.post(() -> {
            Intent send = new Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, text);
            activity.startActivity(Intent.createChooser(send, null));
        });
    }

    void openUrl(String url) {
        main.post(() -> {
            try {
                activity.startActivity(new Intent(Intent.ACTION_VIEW, Uri.parse(url)));
            } catch (Exception e) {
                Log.w(MainActivity.TAG, "nothing opens " + url + ": " + e);
            }
        });
    }

    void toast(String text) {
        Log.i(MainActivity.TAG, "toast: " + text);
        main.post(() -> Toast.makeText(app, text, Toast.LENGTH_SHORT).show());
    }

    void notify(String channel, String tag, String title, String body, String link) {
        main.post(() -> Notifier.post(app, channel, tag, title, body, link));
    }

    void confirm(long token, String title, String message, String ok, boolean destructive) {
        Log.i(MainActivity.TAG, "dialog " + token + ": " + title);
        main.post(() -> {
            if (activity.isFinishing()) {
                Core.answer(token, false, null);
                return;
            }
            AlertDialog dialog = new AlertDialog.Builder(activity, android.R.style.Theme_DeviceDefault_Dialog_Alert)
                    .setTitle(title)
                    .setMessage(message)
                    .setPositiveButton(ok, (d, w) -> Core.answer(token, true, null))
                    .setNegativeButton(android.R.string.cancel, (d, w) -> Core.answer(token, false, null))
                    .setOnCancelListener(d -> Core.answer(token, false, null))
                    .create();
            dialog.show();
            if (destructive) dialog.getButton(AlertDialog.BUTTON_POSITIVE).setTextColor(0xFFFF6B6B);
            activity.showing(dialog);
        });
    }

    void prompt(long token, String title, String text, String ok, boolean multiline) {
        Log.i(MainActivity.TAG, "dialog " + token + ": " + title);
        main.post(() -> {
            EditText field = new EditText(activity);
            if (multiline) {
                // A paste of several lines (a theme's JSON): monospace, a few lines tall, no suggestions.
                field.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_MULTI_LINE
                        | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
                field.setTypeface(android.graphics.Typeface.MONOSPACE);
                field.setMinLines(4);
                field.setMaxLines(10);
                field.setGravity(android.view.Gravity.TOP | android.view.Gravity.START);
            } else {
                field.setInputType(InputType.TYPE_CLASS_TEXT);
            }
            field.setText(text);
            AlertDialog dialog = new AlertDialog.Builder(activity, android.R.style.Theme_DeviceDefault_Dialog_Alert)
                    .setTitle(title)
                    .setView(field)
                    .setPositiveButton(ok, (d, w) -> Core.answer(token, true, field.getText().toString()))
                    .setNegativeButton(android.R.string.cancel, (d, w) -> Core.answer(token, false, null))
                    .setOnCancelListener(d -> Core.answer(token, false, null))
                    .create();
            dialog.show();
            activity.showing(dialog);
        });
    }

    // --- Files ---

    /** The system's document picker; the file picked goes to {@code Core.picked(token, …)}. */
    void pickFile(long token) {
        main.post(() -> activity.pickFile(token));
    }

    /** The key field {@code kind} of the page on top shows {@code text}. */
    void fillKeyField(String kind, String text) {
        main.post(() -> com.cluas.moshpit.ui.Components.fillKeyField(kind, text));
    }

    void releaseStream(long stream) {
        main.post(() -> TerminalViews.release(stream));
    }

    // --- The terminal page's slot ---

    void showStream(long surface, long stream) {
        main.post(() -> TerminalViews.show(activity, surface, stream));
    }

    void terminalLook(String[] colors, double fontSize) {
        int[] parsed = new int[colors.length];
        for (int i = 0; i < colors.length; i++) {
            Integer c = com.cluas.moshpit.ui.Colors.parse(colors[i]);
            parsed[i] = c == null ? 0xFF000000 : c;
        }
        main.post(() -> TerminalViews.look(parsed, (float) fontSize));
    }

    void keyboard(boolean show) {
        main.post(() -> TerminalViews.keyboard(show));
    }

    void setCtrl(boolean armed) {
        TerminalViews.ctrlArmed = armed;
    }

    void paste() {
        main.post(TerminalViews::paste);
    }
}
