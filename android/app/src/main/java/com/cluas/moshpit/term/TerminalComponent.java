package com.cluas.moshpit.term;

import android.content.Context;
import android.view.View;
import android.view.ViewGroup;
import android.widget.FrameLayout;

import com.longbridge.nsh.NshComponent;
import com.longbridge.nsh.NshComponentContext;


/**
 * The native component {@code moshpit/terminal}, the slot of the shared terminal screen (ui/src/terminal): it shows
 * the terminal of the stream the controller says its page shows ({@link TerminalViews#show}), borrowed from
 * {@link TerminalViews} (the views outlive the component, so a rebuilt screen keeps the scrollback).
 * <p>
 * Props: {@code generation} (the screen's rekey; the view is the same). Commands: {@code focus}. The core gets the
 * size on the byte channel itself.
 */
public final class TerminalComponent implements NshComponent {
    private final Context context;
    private final long surface;
    private final FrameLayout container;
    private TerminalViews.Terminal terminal;

    public TerminalComponent(NshComponentContext context, long surface) {
        this.context = context.getContext();
        this.surface = surface;
        container = new FrameLayout(this.context);
        TerminalViews.slots(surface).add(this);
        attach(TerminalViews.shownBy(surface));
    }

    @Override
    public View view() {
        return container;
    }

    /** Shows stream {@code stream}'s terminal (0: none). */
    void attach(long stream) {
        if (terminal != null && terminal.stream == stream) return;
        container.removeAllViews();
        terminal = null;
        if (stream == 0) return;
        terminal = TerminalViews.of(context, stream);
        View view = terminal.view;
        if (view.getParent() instanceof ViewGroup) ((ViewGroup) view.getParent()).removeView(view);
        container.addView(view, new FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        if (terminal.colors != null) container.setBackgroundColor(terminal.colors[0]);
        view.post(terminal::focus);
    }

    @Override
    public void setProp(String name, Object value) {
        // `generation`: the screen made its element again; the terminal is the same.
    }

    @Override
    public void command(String name, Object args) {
        if ("focus".equals(name) && terminal != null) terminal.focus();
    }

    @Override
    public void destroy() {
        // The terminal stays in the table; only this component's borrow ends.
        TerminalViews.slots(surface).remove(this);
        container.removeAllViews();
    }
}
