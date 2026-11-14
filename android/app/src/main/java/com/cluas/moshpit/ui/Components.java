package com.cluas.moshpit.ui;

import android.content.Context;
import android.content.res.ColorStateList;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Paint;
import android.graphics.Typeface;
import android.text.Editable;
import android.text.InputType;
import android.text.TextWatcher;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.Menu;
import android.view.View;
import android.view.ViewGroup;
import android.widget.EditText;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.PopupMenu;
import android.widget.ProgressBar;
import android.widget.SeekBar;
import android.widget.Switch;
import android.widget.TextView;

import com.cluas.moshpit.term.TerminalComponent;
import com.longbridge.nsh.NshComponent;
import com.longbridge.nsh.NshComponentContext;
import com.longbridge.nsh.NshView;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;

/**
 * The host's native components the shared screens place (`moshpit/…`, the iOS host's
 * {@code NSHHostComponents}): Android's own controls where the screens want the platform's (a switch, a slider, a
 * menu), the terminal, and the private key's fields, whose text goes to the core and never to a screen.
 */
public final class Components {
    /** Where the private key's fields send what is typed: {@code Core.keyField}. */
    public interface KeyFields {
        void changed(String kind, String text);
    }

    private Components() {}

    /** Registers every component on {@code view}, the page of surface {@code surface}. */
    public static void register(NshView view, KeyFields keys, long surface) {
        view.registerComponent("moshpit/terminal", c -> new TerminalComponent(c, surface));
        view.registerComponent("moshpit/symbol", SymbolComponent::new);
        view.registerComponent("moshpit/activity-indicator", ActivityIndicatorComponent::new);
        view.registerComponent("moshpit/switch", SwitchComponent::new);
        view.registerComponent("moshpit/segmented", SegmentedComponent::new);
        view.registerComponent("moshpit/slider", SliderComponent::new);
        view.registerComponent("moshpit/menu", c -> new MenuComponent(c, false));
        view.registerComponent("moshpit/action-menu", c -> new MenuComponent(c, true));
        view.registerComponent("moshpit/progress", ProgressComponent::new);
        view.registerComponent("moshpit/color-well", ColorWellComponent::new);
        view.registerComponent("moshpit/key-pem", c -> new KeyFieldComponent(c, keys, "pem"));
        view.registerComponent("moshpit/key-passphrase", c -> new KeyFieldComponent(c, keys, "passphrase"));
        view.registerComponent("moshpit/connection-password", c -> new KeyFieldComponent(c, keys, "connection-password"));
        view.registerComponent("moshpit/connection-pem", c -> new KeyFieldComponent(c, keys, "connection-pem"));
        view.registerComponent("moshpit/app-icon-mark", AppIconMarkComponent::new);
        view.registerComponent("moshpit/signal-grid", SignalGridComponent::new);
        // The terminal screen's (ui/src/terminal).
        view.registerComponent("moshpit/glass", GlassComponent::new);
        view.registerComponent("moshpit/glow", GlowComponent::new);
        view.registerComponent("moshpit/pane-loader", PaneLoaderComponent::new);
        view.registerComponent("moshpit/sparkline", SparklineComponent::new);
        view.registerComponent("moshpit/thumbnail", c -> new NothingComponent(c));
    }

    /** The private key's field ({@code pem}) or its passphrase's on the page on top shows {@code text}. Main thread. */
    public static void fillKeyField(String kind, String text) {
        KeyFieldComponent.fill(kind, text);
    }

    /** {@code moshpit/glass}: the bar's glass (iOS: {@code UIGlassEffect}), a frosted fill and a hairline here. */
    static final class GlassComponent implements NshComponent {
        private final View glass;
        private String shape = "capsule";

        GlassComponent(NshComponentContext context) {
            glass = new View(context.getContext()) {
                @Override
                protected void onSizeChanged(int w, int h, int oldw, int oldh) {
                    super.onSizeChanged(w, h, oldw, oldh);
                    paint();
                }
            };
            paint();
        }

        private void paint() {
            android.graphics.drawable.GradientDrawable fill = new android.graphics.drawable.GradientDrawable();
            fill.setColor(0x26FFFFFF);
            fill.setStroke(Math.max(1, Math.round(dp(glass.getContext(), 0.5f))), 0x33FFFFFF);
            if ("circle".equals(shape)) {
                fill.setShape(android.graphics.drawable.GradientDrawable.OVAL);
            } else {
                fill.setCornerRadius(Math.min(glass.getWidth(), glass.getHeight()) / 2f);
            }
            glass.setBackground(fill);
        }

        @Override public View view() { return glass; }

        @Override
        public void setProp(String name, Object value) {
            if ("shape".equals(name) && value != null) {
                shape = String.valueOf(value);
                paint();
            }
        }
    }

    /** {@code moshpit/glow}: a radial wash of {@code color} around {@code center} (fractions), {@code radius} dp. */
    static final class GlowComponent implements NshComponent {
        private final GlowView glow;

        GlowComponent(NshComponentContext context) {
            glow = new GlowView(context.getContext());
        }

        @Override public View view() { return glow; }

        @Override
        public void setProp(String name, Object value) {
            switch (name) {
                case "color": {
                    Integer c = color(value);
                    if (c != null) glow.color = c;
                    break;
                }
                case "center":
                    if (value instanceof List && ((List<?>) value).size() == 2) {
                        glow.cx = (float) number(((List<?>) value).get(0), 0.5);
                        glow.cy = (float) number(((List<?>) value).get(1), 0.5);
                    }
                    break;
                case "radius": glow.radius = (float) number(value, 300); break;
                default: return;
            }
            glow.invalidate();
        }

        static final class GlowView extends View {
            int color = 0x2053DCC9;
            float cx = 0.5f, cy = 0.5f, radius = 300;
            private final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);

            GlowView(Context context) {
                super(context);
            }

            @Override
            protected void onDraw(Canvas canvas) {
                float r = dp(getContext(), radius);
                float x = cx * getWidth(), y = cy * getHeight();
                paint.setShader(new android.graphics.RadialGradient(x, y, Math.max(1, r), color, color & 0x00FFFFFF,
                        android.graphics.Shader.TileMode.CLAMP));
                canvas.drawRect(0, 0, getWidth(), getHeight(), paint);
            }
        }
    }

    /** {@code moshpit/pane-loader}: the poster's loader, the platform's spinner in the accent. */
    static final class PaneLoaderComponent implements NshComponent {
        private final FrameLayout container;
        private final ProgressBar spinner;

        PaneLoaderComponent(NshComponentContext context) {
            Context c = context.getContext();
            container = new FrameLayout(c);
            spinner = new ProgressBar(c, null, android.R.attr.progressBarStyleLarge);
            spinner.setIndeterminate(true);
            int size = Math.round(dp(c, 56));
            container.addView(spinner, new FrameLayout.LayoutParams(size, size, Gravity.CENTER));
        }

        @Override public View view() { return container; }

        @Override
        public void setProp(String name, Object value) {
            if ("accent".equals(name)) {
                Integer c = color(value);
                if (c != null) spinner.setIndeterminateTintList(ColorStateList.valueOf(c));
            }
        }
    }

    /** {@code moshpit/sparkline}: mosh's round trips, a line through {@code samples}. */
    static final class SparklineComponent implements NshComponent {
        private final LineView line;

        SparklineComponent(NshComponentContext context) {
            line = new LineView(context.getContext());
        }

        @Override public View view() { return line; }

        @Override
        public void setProp(String name, Object value) {
            switch (name) {
                case "samples":
                    line.samples.clear();
                    if (value instanceof List) for (Object v : (List<?>) value) line.samples.add(number(v, 0));
                    break;
                case "color": {
                    Integer c = color(value);
                    if (c != null) line.paint.setColor(c);
                    break;
                }
                case "lineWidth": line.paint.setStrokeWidth(dp(line.getContext(), (float) number(value, 1))); break;
                default: return;
            }
            line.invalidate();
        }

        static final class LineView extends View {
            final List<Double> samples = new ArrayList<>();
            final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);

            LineView(Context context) {
                super(context);
                paint.setStyle(Paint.Style.STROKE);
                paint.setColor(Color.WHITE);
            }

            @Override
            protected void onDraw(Canvas canvas) {
                if (samples.size() < 2) return;
                double max = 1;
                for (double v : samples) max = Math.max(max, v);
                android.graphics.Path path = new android.graphics.Path();
                for (int i = 0; i < samples.size(); i++) {
                    float x = getWidth() * i / (float) (samples.size() - 1);
                    float y = (float) (getHeight() * (1 - samples.get(i) / max));
                    if (i == 0) path.moveTo(x, y);
                    else path.lineTo(x, y);
                }
                canvas.drawPath(path, paint);
            }
        }
    }

    /** A component Android has nothing for yet (the image panel's thumbnails): an empty view. */
    static final class NothingComponent implements NshComponent {
        private final View view;

        NothingComponent(NshComponentContext context) {
            view = new View(context.getContext());
        }

        @Override public View view() { return view; }

        @Override public void setProp(String name, Object value) {}
    }

    /** {@code #RRGGBB} or {@code #RRGGBBAA} (the screens' order: alpha last), as ARGB. */
    static Integer color(Object value) {
        return Colors.parse(value);
    }

    static double number(Object value, double fallback) {
        return value instanceof Number ? ((Number) value).doubleValue() : fallback;
    }

    static float dp(Context context, float value) {
        return TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, value, context.getResources().getDisplayMetrics());
    }

    /** The props every control takes: its test identifier and label. */
    static boolean accessibility(String name, Object value, View view) {
        if ("label".equals(name)) {
            view.setContentDescription(value == null ? null : String.valueOf(value));
            return true;
        }
        return "identifier".equals(name);
    }

    /** Reports {@code view}'s own size, measured unconstrained, in dp. */
    static void reportSize(NshComponentContext context, View view) {
        view.measure(View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED),
                View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED));
        float density = view.getResources().getDisplayMetrics().density;
        context.setSize(view.getMeasuredWidth() / density, view.getMeasuredHeight() / density);
    }

    /** {@code moshpit/symbol}: an SF Symbol's stand-in glyph ({@link Symbols}). */
    static final class SymbolComponent implements NshComponent {
        private final NshComponentContext context;
        private final TextView label;
        private String name = "";
        private float pointSize = 17;

        SymbolComponent(NshComponentContext context) {
            this.context = context;
            label = new TextView(context.getContext());
            label.setGravity(Gravity.CENTER);
            label.setIncludeFontPadding(false);
            label.setTextColor(Color.WHITE);
            label.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
        }

        @Override public View view() { return label; }

        @Override
        public void setProp(String prop, Object value) {
            switch (prop) {
                case "name": name = value == null ? "" : String.valueOf(value); break;
                case "pointSize": pointSize = (float) number(value, 17); break;
                case "weight":
                    label.setTypeface(Typeface.create(Typeface.DEFAULT,
                            "semibold".equals(value) || "bold".equals(value) ? Typeface.BOLD : Typeface.NORMAL));
                    break;
                case "color": {
                    Integer c = color(value);
                    if (c != null) label.setTextColor(c);
                    return;
                }
                case "palette": {
                    if (value instanceof List && !((List<?>) value).isEmpty()) {
                        Integer c = color(((List<?>) value).get(0));
                        if (c != null) label.setTextColor(c);
                    }
                    return;
                }
                default: return;
            }
            label.setText(Symbols.glyph(name));
            label.setTextSize(TypedValue.COMPLEX_UNIT_DIP, pointSize);
            reportSize(context, label);
        }
    }

    /** {@code moshpit/activity-indicator}: the platform's spinner. */
    static final class ActivityIndicatorComponent implements NshComponent {
        private final FrameLayout container;
        private final ProgressBar spinner;

        ActivityIndicatorComponent(NshComponentContext context) {
            Context c = context.getContext();
            container = new FrameLayout(c);
            spinner = new ProgressBar(c, null, android.R.attr.progressBarStyleSmall);
            spinner.setIndeterminate(true);
            int size = Math.round(dp(c, 18));
            container.addView(spinner, new FrameLayout.LayoutParams(size, size, Gravity.CENTER));
        }

        @Override public View view() { return container; }

        @Override
        public void setProp(String name, Object value) {
            if ("color".equals(name)) {
                Integer c = color(value);
                if (c != null) spinner.setIndeterminateTintList(ColorStateList.valueOf(c));
            } else if ("scale".equals(name)) {
                float scale = (float) number(value, 1);
                spinner.setScaleX(scale);
                spinner.setScaleY(scale);
            }
        }
    }

    /** {@code moshpit/switch}. */
    static final class SwitchComponent implements NshComponent {
        private final Switch control;
        private boolean setting;

        SwitchComponent(NshComponentContext context) {
            control = new Switch(context.getContext());
            control.setOnCheckedChangeListener((button, on) -> {
                if (!setting) context.emit("change", on);
            });
            reportSize(context, control);
        }

        @Override public View view() { return control; }

        @Override
        public void setProp(String name, Object value) {
            if (accessibility(name, value, control)) return;
            switch (name) {
                case "on":
                    setting = true;
                    control.setChecked(Boolean.TRUE.equals(value));
                    setting = false;
                    break;
                case "tint": {
                    Integer c = color(value);
                    if (c != null) {
                        control.setThumbTintList(ColorStateList.valueOf(Color.WHITE));
                        control.setTrackTintList(new ColorStateList(
                                new int[][] {{android.R.attr.state_checked}, {}},
                                new int[] {c, 0x40FFFFFF}));
                    }
                    break;
                }
                case "enabled": control.setEnabled(!Boolean.FALSE.equals(value)); break;
                default: break;
            }
        }
    }

    /** {@code moshpit/segmented}: a row of segments, one picked. */
    static final class SegmentedComponent implements NshComponent {
        private final NshComponentContext context;
        private final LinearLayout row;
        private final List<TextView> segments = new ArrayList<>();
        private int selected = -1;

        SegmentedComponent(NshComponentContext context) {
            this.context = context;
            row = new LinearLayout(context.getContext());
            row.setOrientation(LinearLayout.HORIZONTAL);
            android.graphics.drawable.GradientDrawable ground = new android.graphics.drawable.GradientDrawable();
            ground.setColor(0x26FFFFFF);
            ground.setCornerRadius(dp(context.getContext(), 8));
            row.setBackground(ground);
        }

        @Override public View view() { return row; }

        @Override
        public void setProp(String name, Object value) {
            if (accessibility(name, value, row)) return;
            if ("items".equals(name) && value instanceof List) {
                row.removeAllViews();
                segments.clear();
                Context c = context.getContext();
                int index = 0;
                for (Object item : (List<?>) value) {
                    final int at = index++;
                    TextView segment = new TextView(c);
                    segment.setText(String.valueOf(item));
                    segment.setTextColor(Color.WHITE);
                    segment.setTextSize(TypedValue.COMPLEX_UNIT_DIP, 13);
                    segment.setGravity(Gravity.CENTER);
                    int pad = Math.round(dp(c, 10));
                    segment.setPadding(pad, Math.round(dp(c, 6)), pad, Math.round(dp(c, 6)));
                    segment.setOnClickListener(v -> {
                        select(at);
                        context.emit("change", at);
                    });
                    row.addView(segment, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.MATCH_PARENT, 1));
                    segments.add(segment);
                }
                select(selected);
                reportSize(context, row);
            } else if ("selected".equals(name)) {
                select((int) number(value, -1));
            }
        }

        private void select(int index) {
            selected = index;
            for (int i = 0; i < segments.size(); i++) {
                android.graphics.drawable.GradientDrawable chip = new android.graphics.drawable.GradientDrawable();
                chip.setColor(i == index ? 0x66FFFFFF : 0);
                chip.setCornerRadius(dp(context.getContext(), 7));
                segments.get(i).setBackground(chip);
            }
        }
    }

    /** {@code moshpit/slider}: stepped, as SwiftUI's. */
    static final class SliderComponent implements NshComponent {
        private static final int TICKS = 1000;
        private final NshComponentContext context;
        private final SeekBar control;
        private double min = 0, max = 1, step = 0, told = Double.NaN;
        private boolean setting;

        SliderComponent(NshComponentContext context) {
            this.context = context;
            control = new SeekBar(context.getContext());
            control.setMax(TICKS);
            control.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
                @Override public void onProgressChanged(SeekBar bar, int progress, boolean fromUser) {
                    if (setting || !fromUser) return;
                    double value = min + (max - min) * progress / TICKS;
                    if (step > 0) value = Math.round(value / step) * step;
                    if (value == told) return;
                    told = value;
                    context.emit("change", value);
                }
                @Override public void onStartTrackingTouch(SeekBar bar) {}
                @Override public void onStopTrackingTouch(SeekBar bar) {}
            });
            reportSize(context, control);
        }

        @Override public View view() { return control; }

        @Override
        public void setProp(String name, Object value) {
            if (accessibility(name, value, control)) return;
            switch (name) {
                case "value":
                    told = number(value, min);
                    setting = true;
                    control.setProgress(max > min ? (int) Math.round((told - min) / (max - min) * TICKS) : 0);
                    setting = false;
                    break;
                case "min": min = number(value, 0); break;
                case "max": max = number(value, 1); break;
                case "step": step = number(value, 0); break;
                case "tint": {
                    Integer c = color(value);
                    if (c != null) {
                        control.setProgressTintList(ColorStateList.valueOf(c));
                        control.setThumbTintList(ColorStateList.valueOf(c));
                    }
                    break;
                }
                default: break;
            }
        }
    }

    /**
     * {@code moshpit/menu} and {@code moshpit/action-menu}: a clear button over its box (the screen draws the row)
     * that opens the platform's pop-up menu; {@code select} with the item's id.
     */
    static final class MenuComponent implements NshComponent {
        private final View button;
        private List<?> items = new ArrayList<>();

        MenuComponent(NshComponentContext context, boolean actions) {
            button = new View(context.getContext());
            button.setBackgroundColor(Color.TRANSPARENT);
            button.setOnClickListener(v -> {
                PopupMenu popup = new PopupMenu(v.getContext(), v, Gravity.END);
                Menu menu = popup.getMenu();
                List<String> ids = new ArrayList<>();
                int group = 0;
                Object lastGroup = null;
                for (Object item : items) {
                    if (!(item instanceof Map)) continue;
                    Map<?, ?> entry = (Map<?, ?>) item;
                    Object id = entry.get("id"), title = entry.get("title");
                    if (id == null || title == null) continue;
                    if (actions && entry.get("group") != null && !entry.get("group").equals(lastGroup)) {
                        lastGroup = entry.get("group");
                        group++;
                    }
                    String text = String.valueOf(title);
                    if (entry.get("subtitle") != null) text += " — " + entry.get("subtitle");
                    android.view.MenuItem added = menu.add(group, ids.size(), ids.size(), text);
                    added.setEnabled(!Boolean.TRUE.equals(entry.get("disabled")));
                    ids.add(String.valueOf(id));
                }
                popup.setOnMenuItemClickListener(picked -> {
                    context.emit("select", ids.get(picked.getItemId()));
                    return true;
                });
                popup.show();
            });
        }

        @Override public View view() { return button; }

        @Override
        public void setProp(String name, Object value) {
            if (accessibility(name, value, button)) return;
            if ("items".equals(name) && value instanceof List) items = (List<?>) value;
        }
    }

    /** {@code moshpit/progress}. */
    static final class ProgressComponent implements NshComponent {
        private final ProgressBar bar;

        ProgressComponent(NshComponentContext context) {
            bar = new ProgressBar(context.getContext(), null, android.R.attr.progressBarStyleHorizontal);
            bar.setMax(1000);
            reportSize(context, bar);
        }

        @Override public View view() { return bar; }

        @Override
        public void setProp(String name, Object value) {
            if (accessibility(name, value, bar)) return;
            if ("value".equals(name)) bar.setProgress((int) Math.round(number(value, 0) * 1000));
            else if ("tint".equals(name)) {
                Integer c = color(value);
                if (c != null) bar.setProgressTintList(ColorStateList.valueOf(c));
            }
        }
    }

    /** {@code moshpit/color-well}: the color, as a swatch (picking one is not on Android yet). */
    /**
     * {@code moshpit/color-well}: the color as a ringed dot (iOS: {@code UIColorWell}); a tap opens the host's color
     * picker ({@link ColorPicker}) and each color picked there is emitted as {@code change} ({@code #RRGGBB}).
     */
    static final class ColorWellComponent implements NshComponent {
        private final View swatch;
        private int color = Color.BLACK;
        private String label = "Color";
        /** The screen's identifier, as the window hierarchy's resource id (the emulator runs find the well by it). */
        private String identifier;

        ColorWellComponent(NshComponentContext context) {
            swatch = new View(context.getContext()) {
                @Override
                public void onInitializeAccessibilityNodeInfo(android.view.accessibility.AccessibilityNodeInfo info) {
                    super.onInitializeAccessibilityNodeInfo(info);
                    if (identifier != null) info.setViewIdResourceName(identifier);
                }
            };
            swatch.setClickable(true);
            swatch.setOnClickListener(v -> {
                if (open != null && open.isShowing()) return;
                open = ColorPicker.show(v.getContext(), color, label, hex -> {
                    Integer picked = ColorPicker.parse(hex);
                    if (picked != null) paint(picked);
                    context.emit("change", hex);
                });
            });
            // The picker goes with its well's page (a back, the page popped).
            swatch.addOnAttachStateChangeListener(new View.OnAttachStateChangeListener() {
                @Override public void onViewAttachedToWindow(View v) {}

                @Override
                public void onViewDetachedFromWindow(View v) {
                    if (open != null && open.isShowing()) open.dismiss();
                    open = null;
                }
            });
            paint(color);
        }

        /** The picker this well opened, while it shows. */
        private android.app.AlertDialog open;

        private void paint(int c) {
            color = c;
            android.graphics.drawable.GradientDrawable well = new android.graphics.drawable.GradientDrawable();
            well.setShape(android.graphics.drawable.GradientDrawable.OVAL);
            well.setColor(c);
            well.setStroke(Math.round(dp(swatch.getContext(), 2)), Color.WHITE);
            swatch.setBackground(well);
        }

        @Override public View view() { return swatch; }

        @Override
        public void setProp(String name, Object value) {
            if ("label".equals(name) && value != null) label = String.valueOf(value);
            if ("identifier".equals(name)) identifier = value == null ? null : String.valueOf(value);
            if (accessibility(name, value, swatch)) return;
            if (!"color".equals(name)) return;
            Integer c = color(value);
            if (c != null) paint(c);
        }
    }

    /**
     * {@code moshpit/key-pem} and {@code moshpit/key-passphrase}: the private key's text and its passphrase, in
     * the platform's own text fields; what is typed goes to the core ({@link KeyFields}), never to the screen.
     * {@code moshpit/connection-password} and {@code moshpit/connection-pem}: the connection form's password and
     * pasted key, the same way; the screen hears only {@code filled} (whether the field holds anything), and a
     * field made again (the method switched back) starts from what was typed.
     */
    static final class KeyFieldComponent implements NshComponent {
        private final NshComponentContext context;
        private final EditText field;
        private final boolean multiline;

        /** The field of each kind made last (the page on top's): where a key read from a file goes. */
        private static final Map<String, java.lang.ref.WeakReference<KeyFieldComponent>> LIVE = new java.util.HashMap<>();
        /** What the connection form's fields hold, kept for a field of the kind made again. Main thread. */
        private static final Map<String, String> HELD = new java.util.HashMap<>();

        /** Field {@code kind} (made last) shows {@code text}, and tells the core as typing would. Main thread. */
        public static void fill(String kind, String text) {
            if (kind.startsWith("connection-")) HELD.put(kind, text);
            java.lang.ref.WeakReference<KeyFieldComponent> ref = LIVE.get(kind);
            KeyFieldComponent live = ref == null ? null : ref.get();
            if (live != null) live.field.setText(text);
        }

        KeyFieldComponent(NshComponentContext context, KeyFields keys, String kind) {
            this.context = context;
            LIVE.put(kind, new java.lang.ref.WeakReference<>(this));
            multiline = "pem".equals(kind) || "connection-pem".equals(kind);
            final boolean connection = kind.startsWith("connection-");
            field = new EditText(context.getContext());
            field.setTextColor(Color.WHITE);
            field.setBackgroundColor(Color.TRANSPARENT);
            field.setTypeface(Typeface.MONOSPACE);
            field.setGravity(Gravity.TOP | Gravity.START);
            field.setTextSize(TypedValue.COMPLEX_UNIT_DIP, multiline ? 12 : 15);
            field.setInputType(multiline
                    ? InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_MULTI_LINE | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
                    : InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_PASSWORD);
            field.setImportantForAutofill(View.IMPORTANT_FOR_AUTOFILL_NO);
            if (connection) field.setText(HELD.getOrDefault(kind, ""));
            field.addTextChangedListener(new TextWatcher() {
                private Boolean told;
                @Override public void beforeTextChanged(CharSequence s, int start, int count, int after) {}
                @Override public void onTextChanged(CharSequence s, int start, int before, int count) {}
                @Override public void afterTextChanged(Editable s) {
                    String text = s.toString();
                    keys.changed(kind, text);
                    if (connection) {
                        HELD.put(kind, text);
                        boolean filled = multiline ? !text.trim().isEmpty() : !text.isEmpty();
                        if (told == null || told != filled) {
                            told = filled;
                            context.emit("filled", filled);
                        }
                    }
                    if (multiline) report();
                }
            });
            report();
        }

        private void report() {
            float density = field.getResources().getDisplayMetrics().density;
            int lines = Math.max(multiline ? 5 : 1, field.getLineCount());
            float height = lines * field.getLineHeight() / density + 16;
            context.setSize(field.getWidth() / density, height);
        }

        @Override public View view() { return field; }

        @Override
        public void setProp(String name, Object value) {
            if (accessibility(name, value, field)) return;
            switch (name) {
                case "fontSize": field.setTextSize(TypedValue.COMPLEX_UNIT_DIP, (float) number(value, 12)); report(); break;
                case "textColor": {
                    Integer c = color(value);
                    if (c != null) field.setTextColor(c);
                    break;
                }
                case "placeholder": field.setHint(value == null ? null : String.valueOf(value)); break;
                // "trailing" beside a label (B319), "leading" under one.
                case "alignment":
                    field.setGravity(Gravity.TOP | ("trailing".equals(value) ? Gravity.END : Gravity.START));
                    break;
                default: break;
            }
        }
    }

    /** {@code moshpit/app-icon-mark}: the app's icon. */
    static final class AppIconMarkComponent implements NshComponent {
        private final ImageView image;

        AppIconMarkComponent(NshComponentContext context) {
            image = new ImageView(context.getContext());
            image.setImageDrawable(context.getContext().getApplicationInfo().loadIcon(context.getContext().getPackageManager()));
            image.setScaleType(ImageView.ScaleType.FIT_CENTER);
        }

        @Override public View view() { return image; }

        @Override public void setProp(String name, Object value) {}
    }

    /** {@code moshpit/signal-grid}: Home's empty-state grid, faint lines 24 dp apart. */
    static final class SignalGridComponent implements NshComponent {
        private final GridView grid;

        SignalGridComponent(NshComponentContext context) {
            grid = new GridView(context.getContext());
        }

        @Override public View view() { return grid; }

        @Override
        public void setProp(String name, Object value) {
            if ("color".equals(name)) {
                Integer c = color(value);
                if (c != null) grid.paint.setColor(c);
            } else if ("lineWidth".equals(name)) {
                grid.paint.setStrokeWidth(dp(grid.getContext(), (float) number(value, 1)));
            }
            grid.invalidate();
        }

        static final class GridView extends View {
            final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);

            GridView(Context context) {
                super(context);
                paint.setColor(0x14FFFFFF);
                paint.setStrokeWidth(dp(context, 1));
            }

            @Override
            protected void onDraw(Canvas canvas) {
                float step = dp(getContext(), 24);
                for (float x = 0; x <= getWidth(); x += step) canvas.drawLine(x, 0, x, getHeight(), paint);
                for (float y = 0; y <= getHeight(); y += step) canvas.drawLine(0, y, getWidth(), y, paint);
            }
        }
    }
}
