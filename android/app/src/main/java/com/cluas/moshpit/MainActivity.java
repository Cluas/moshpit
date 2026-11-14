package com.cluas.moshpit;

import android.app.Activity;
import android.app.Dialog;
import android.content.Intent;
import android.content.pm.ApplicationInfo;
import android.content.pm.PackageInfo;
import android.net.ConnectivityManager;
import android.net.Network;
import android.os.Build;
import android.os.Bundle;
import android.util.Log;
import android.view.View;
import android.widget.FrameLayout;
import android.window.OnBackInvokedCallback;
import android.window.OnBackInvokedDispatcher;

import java.util.ArrayList;
import java.util.List;

/**
 * Moshpit's Android host (`docs/nsh/DESIGN.md` §3.2, 宿主): one activity holding the page stack ({@link Pages}) the
 * Rust controller drives. It starts the controller ({@link Core#start}) with the platform it calls back
 * ({@link HostPlatform}), and tells it what the system does: back, foreground and background, network changes, the
 * unlock, text shared to the app.
 * <p>
 * Edge to edge, as NSH's own activity is: the screens get the system bars and the keyboard as their safe area.
 */
public final class MainActivity extends Activity {
    static final String TAG = "MoshpitView";

    private Pages pages;
    private HostPlatform platform;
    private DebugReceiver debug;
    private OnBackInvokedCallback backCallback;
    private ConnectivityManager.NetworkCallback network;
    private final List<Dialog> dialogs = new ArrayList<>();

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        edgeToEdge();
        Notifier.createChannels(this);
        FrameLayout root = new FrameLayout(this);
        root.setBackgroundColor(0xFF05090A);
        setContentView(root);
        pages = new Pages(this, root);
        platform = new HostPlatform(this);
        boolean debuggable = (getApplicationInfo().flags & ApplicationInfo.FLAG_DEBUGGABLE) != 0;
        Core.start(platform, versionLine(), deviceLabel(), debuggable);
        if (debuggable) debug = DebugReceiver.register(this);
        if (Build.VERSION.SDK_INT >= 33) {
            backCallback = this::onBack;
            getOnBackInvokedDispatcher().registerOnBackInvokedCallback(OnBackInvokedDispatcher.PRIORITY_DEFAULT, backCallback);
        }
        watchNetwork();
        handleShare(getIntent());
    }

    Pages pages() {
        return pages;
    }

    /** The request codes of the document picker: the token the controller waits under, by request. */
    private final java.util.Map<Integer, Long> picks = new java.util.HashMap<>();
    private int nextPick = 0x4D00;

    /**
     * The Storage Access Framework's picker (any file: a key has no type the system knows); the file's bytes go to
     * the controller under {@code token} ({@link #onActivityResult}). The test hooks may answer it instead
     * ({@link DebugReceiver}).
     */
    void pickFile(long token) {
        Intent open = new Intent(Intent.ACTION_OPEN_DOCUMENT)
                .addCategory(Intent.CATEGORY_OPENABLE)
                .setType("*/*");
        int request = nextPick++;
        picks.put(request, token);
        lastPickToken = token;
        try {
            startActivityForResult(open, request);
        } catch (Exception e) {
            Log.w(TAG, "no document picker: " + e);
            picks.remove(request);
            Core.picked(token, null);
        }
    }

    /** The token of the picker shown last (the test hooks answer it). */
    long lastPickToken;

    /** What may be read of a picked file (a key or a theme is a few kilobytes). */
    static final int PICK_LIMIT = 1 << 20;

    @Override
    protected void onActivityResult(int request, int result, Intent data) {
        Long token = picks.remove(request);
        if (token == null) {
            super.onActivityResult(request, result, data);
            return;
        }
        android.net.Uri uri = result == RESULT_OK && data != null ? data.getData() : null;
        if (uri == null) {
            Core.picked(token, null);
            return;
        }
        // Read off the main thread: a provider may be slow (a cloud drive).
        new Thread(() -> Core.picked(token, read(uri)), "moshpit-pick").start();
    }

    /** The file at {@code uri}, up to {@link #PICK_LIMIT} bytes and one more (so the controller sees it is too big). */
    byte[] read(android.net.Uri uri) {
        try (java.io.InputStream in = getContentResolver().openInputStream(uri)) {
            if (in == null) return null;
            java.io.ByteArrayOutputStream out = new java.io.ByteArrayOutputStream();
            byte[] buffer = new byte[8192];
            int n;
            while ((n = in.read(buffer)) > 0) {
                out.write(buffer, 0, n);
                if (out.size() > PICK_LIMIT) break;
            }
            Log.i(TAG, "picked a file: " + out.size() + " bytes");
            return out.toByteArray();
        } catch (Exception e) {
            Log.w(TAG, "the picked file could not be read: " + e);
            return null;
        }
    }

    /** A dialog the platform shows, for the test hooks to dismiss. */
    void showing(Dialog dialog) {
        dialogs.removeIf(d -> !d.isShowing());
        dialogs.add(dialog);
    }

    void dismissDialogs() {
        for (Dialog d : dialogs) if (d.isShowing()) d.dismiss();
        dialogs.clear();
    }

    @SuppressWarnings("deprecation")
    private void edgeToEdge() {
        if (Build.VERSION.SDK_INT >= 30) {
            getWindow().setDecorFitsSystemWindows(false);
        } else {
            getWindow().getDecorView().setSystemUiVisibility(View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                    | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION);
        }
        getWindow().setStatusBarColor(0);
        getWindow().setNavigationBarColor(0);
    }

    private String versionLine() {
        try {
            PackageInfo info = getPackageManager().getPackageInfo(getPackageName(), 0);
            long code = Build.VERSION.SDK_INT >= 28 ? info.getLongVersionCode() : info.versionCode;
            return "v" + info.versionName + " (" + code + ")";
        } catch (Exception e) {
            return "";
        }
    }

    private static String deviceLabel() {
        String model = Build.MODEL;
        return model == null || model.isEmpty() ? "Android" : model;
    }

    /** The system's back: an NSH modal first, then the controller (which pops a page, or leaves from Home). */
    private void onBack() {
        android.view.View top = pages.top();
        if (top instanceof com.longbridge.nsh.NshView && ((com.longbridge.nsh.NshView) top).handleBack()) return;
        if (!Core.back()) moveTaskToBack(true);
    }

    /** Android 12 and older: the app gets the back button first. */
    @Override
    @SuppressWarnings("deprecation")
    public void onBackPressed() {
        onBack();
    }

    @Override
    protected void onStart() {
        super.onStart();
        Core.lifecycle(true);
    }

    @Override
    protected void onStop() {
        Core.lifecycle(false);
        super.onStop();
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        handleShare(intent);
    }

    /** Text shared from another app (ACTION_SEND text/plain): to the controller's share queue. */
    private void handleShare(Intent intent) {
        if (intent == null || !Intent.ACTION_SEND.equals(intent.getAction())) return;
        CharSequence text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT);
        if (text == null) return;
        Log.i(TAG, "share: " + text.length() + " chars");
        Core.share(text.toString());
        setIntent(new Intent(this, MainActivity.class));
    }

    private void watchNetwork() {
        ConnectivityManager connectivity = getSystemService(ConnectivityManager.class);
        if (connectivity == null || Build.VERSION.SDK_INT < 24) return;
        network = new ConnectivityManager.NetworkCallback() {
            private Network current;

            @Override
            public void onAvailable(Network n) {
                if (current != null && !current.equals(n)) Core.networkChanged();
                current = n;
            }

            @Override
            public void onLost(Network n) {
                if (n.equals(current)) current = null;
            }
        };
        connectivity.registerDefaultNetworkCallback(network);
    }

    @Override
    protected void onDestroy() {
        if (debug != null) unregisterReceiver(debug);
        if (backCallback != null && Build.VERSION.SDK_INT >= 33) {
            getOnBackInvokedDispatcher().unregisterOnBackInvokedCallback(backCallback);
        }
        ConnectivityManager connectivity = getSystemService(ConnectivityManager.class);
        if (network != null && connectivity != null) connectivity.unregisterNetworkCallback(network);
        dismissDialogs();
        pages.clear();
        super.onDestroy();
    }
}
