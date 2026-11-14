package com.cluas.moshpit;

import android.app.Activity;
import android.util.Log;
import android.view.View;
import android.view.ViewGroup;
import android.widget.FrameLayout;

import com.cluas.moshpit.ui.Components;
import com.longbridge.nsh.NshView;

import java.util.ArrayList;
import java.util.List;

/**
 * The page stack: one {@code NshView} per page, the top one in front, as the Rust controller pushes and pops them
 * ({@link HostPlatform#push}, {@link HostPlatform#pop}). A pushed page slides in from the end; a popped one slides
 * out and its view is destroyed, then the controller hears it is gone ({@link Core#gone}) and closes its channel.
 * Main thread only.
 */
final class Pages {
    private static final long SLIDE_MS = 220;

    private static final class Page {
        final long surface;
        final NshView view;

        Page(long surface, NshView view) {
            this.surface = surface;
            this.view = view;
        }
    }

    private final Activity activity;
    private final FrameLayout root;
    private final List<Page> stack = new ArrayList<>();

    Pages(Activity activity, FrameLayout root) {
        this.activity = activity;
        this.root = root;
    }

    /** A new page on top running surface {@code surface}. */
    void push(long surface, boolean animated) {
        NshView view = new NshView(activity);
        view.setErrorListener(message -> Log.e(MainActivity.TAG, "nsh: " + message));
        Components.register(view, Core::keyField, surface);
        view.setBackgroundColor(0xFF05090A);
        root.addView(view, new FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        stack.add(new Page(surface, view));
        Core.prepare(surface);
        view.runRustApp("app");
        if (animated && stack.size() > 1) {
            view.setTranslationX(root.getWidth());
            view.animate().translationX(0).setDuration(SLIDE_MS).start();
        }
        Log.i(MainActivity.TAG, "page " + surface + " pushed (" + stack.size() + " deep)");
    }

    /** Takes page {@code surface} away. */
    void pop(long surface, boolean animated) {
        Page page = null;
        for (Page p : stack) if (p.surface == surface) page = p;
        if (page == null) return;
        stack.remove(page);
        final Page gone = page;
        Runnable remove = () -> {
            root.removeView(gone.view);
            gone.view.destroy();
            Core.gone(gone.surface);
        };
        if (animated && root.getWidth() > 0) {
            gone.view.animate().translationX(root.getWidth()).setDuration(SLIDE_MS).withEndAction(remove).start();
        } else {
            remove.run();
        }
        Log.i(MainActivity.TAG, "page " + surface + " popped (" + stack.size() + " left)");
    }

    /** The top page's view, if any. */
    NshView top() {
        return stack.isEmpty() ? null : stack.get(stack.size() - 1).view;
    }

    /** Every page goes (the activity is going away). */
    void clear() {
        for (Page page : stack) {
            root.removeView(page.view);
            page.view.destroy();
            Core.gone(page.surface);
        }
        stack.clear();
    }

    int depth() {
        return stack.size();
    }

    View rootView() {
        return root;
    }
}
