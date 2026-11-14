package com.cluas.moshpit.ui;

import android.app.AlertDialog;
import android.content.Context;
import android.graphics.Color;
import android.graphics.drawable.GradientDrawable;
import android.text.Editable;
import android.text.InputFilter;
import android.text.InputType;
import android.text.TextWatcher;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.View;
import android.widget.EditText;
import android.widget.GridLayout;
import android.widget.LinearLayout;
import android.widget.SeekBar;
import android.widget.TextView;

/**
 * The system color picker's stand-in (iOS: {@code UIColorWell}'s {@code UIColorPickerViewController}, which
 * Android has no counterpart of): a dialog with the color's preview and its code, hue, saturation and brightness
 * sliders, a field for {@code #RRGGBB}, and a grid of starting points. No opacity, as on iOS: a palette entry has
 * none. Done hands the color back as {@code #RRGGBB}; Cancel, nothing.
 */
public final class ColorPicker {
    /** Where the picked color goes. */
    public interface Picked {
        void color(String hex);
    }

    /** The grid: the terminal palettes' usual hues, light to dark, and grays. */
    static final int[] PRESETS = {
        0xFFFF5F57, 0xFFFF9F43, 0xFFFFD93D, 0xFF6BCB77, 0xFF4D96FF, 0xFF6C6BEF, 0xFFC86BFA, 0xFF53DCC9,
        0xFFB33A3A, 0xFFB86E1F, 0xFFB39B1F, 0xFF3D8B4A, 0xFF2D5FB3, 0xFF4847A8, 0xFF8A45B3, 0xFF2E9C8C,
        0xFFFFFFFF, 0xFFD0D0D0, 0xFFA0A0A0, 0xFF707070, 0xFF484848, 0xFF282828, 0xFF101010, 0xFF000000,
    };

    private ColorPicker() {}

    /** {@code #RRGGBB} of {@code argb} (its alpha dropped). */
    public static String hex(int argb) {
        return String.format("#%06X", argb & 0xFFFFFF);
    }

    /** The color {@code #RRGGBB} (or {@code RRGGBB}) names, or null. */
    public static Integer parse(String text) {
        if (text == null) return null;
        String t = text.trim();
        if (!t.startsWith("#")) t = "#" + t;
        return t.length() == 7 ? Colors.parse(t) : null;
    }

    /** Shows the picker over {@code context}'s activity, starting at {@code start}; the dialog, to dismiss. */
    public static AlertDialog show(Context context, int start, String title, Picked picked) {
        ColorPicker.Dialog dialog = new ColorPicker.Dialog(context, start, title, picked);
        dialog.show();
        return dialog.dialog;
    }

    private static final class Dialog {
        private final Context context;
        private final float[] hsv = new float[3];
        private final View preview;
        private final TextView code;
        private final SeekBar hue;
        private final SeekBar saturation;
        private final SeekBar brightness;
        private final EditText field;
        private final AlertDialog dialog;
        /** The sliders or the field are being set from the color: no echo. */
        private boolean setting;
        /** A color given exactly (the start, a preset, a typed code): kept as it is, not rounded through HSV. */
        private Integer exact;

        Dialog(Context context, int start, String title, Picked picked) {
            this.context = context;
            exact = start | 0xFF000000;
            Color.colorToHSV(start, hsv);
            LinearLayout column = new LinearLayout(context);
            column.setOrientation(LinearLayout.VERTICAL);
            int pad = px(20);
            column.setPadding(pad, px(8), pad, 0);

            LinearLayout head = new LinearLayout(context);
            head.setGravity(Gravity.CENTER_VERTICAL);
            preview = new View(context);
            head.addView(preview, new LinearLayout.LayoutParams(px(56), px(56)));
            code = new TextView(context);
            code.setTypeface(android.graphics.Typeface.MONOSPACE);
            code.setTextSize(TypedValue.COMPLEX_UNIT_SP, 18);
            code.setTextColor(Color.WHITE);
            code.setPadding(px(16), 0, 0, 0);
            head.addView(code);
            column.addView(head);

            hue = slider(column, "Hue", 360);
            saturation = slider(column, "Saturation", 100);
            brightness = slider(column, "Brightness", 100);

            field = new EditText(context);
            field.setTypeface(android.graphics.Typeface.MONOSPACE);
            field.setTextColor(Color.WHITE);
            field.setHint("#RRGGBB");
            field.setSingleLine(true);
            field.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
                    | InputType.TYPE_TEXT_FLAG_CAP_CHARACTERS);
            field.setFilters(new InputFilter[] {new InputFilter.LengthFilter(7)});
            field.setContentDescription("Hex color");
            field.addTextChangedListener(new TextWatcher() {
                @Override public void beforeTextChanged(CharSequence s, int a, int b, int c) {}
                @Override public void onTextChanged(CharSequence s, int a, int b, int c) {}
                @Override public void afterTextChanged(Editable s) {
                    if (setting) return;
                    Integer typed = parse(s.toString());
                    if (typed != null) {
                        exact = typed;
                        Color.colorToHSV(typed, hsv);
                        refresh(false);
                    }
                }
            });
            column.addView(field, new LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT,
                    LinearLayout.LayoutParams.WRAP_CONTENT));

            GridLayout grid = new GridLayout(context);
            grid.setColumnCount(8);
            grid.setPadding(0, px(8), 0, px(4));
            for (int preset : PRESETS) {
                View swatch = new View(context);
                GradientDrawable dot = new GradientDrawable();
                dot.setShape(GradientDrawable.OVAL);
                dot.setColor(preset);
                dot.setStroke(px(1), 0x40FFFFFF);
                swatch.setBackground(dot);
                swatch.setContentDescription(hex(preset));
                swatch.setOnClickListener(v -> {
                    exact = preset;
                    Color.colorToHSV(preset, hsv);
                    refresh(true);
                });
                GridLayout.LayoutParams at = new GridLayout.LayoutParams();
                at.width = px(28);
                at.height = px(28);
                at.setMargins(px(3), px(3), px(3), px(3));
                grid.addView(swatch, at);
            }
            column.addView(grid);

            dialog = new AlertDialog.Builder(context, android.R.style.Theme_Material_Dialog_Alert)
                    .setTitle(title)
                    .setView(column)
                    .setPositiveButton("Done", (d, w) -> picked.color(hex(current())))
                    .setNegativeButton(android.R.string.cancel, null)
                    .create();
            refresh(true);
        }

        void show() {
            dialog.show();
        }

        private SeekBar slider(LinearLayout column, String name, int max) {
            TextView label = new TextView(context);
            label.setText(name);
            label.setTextColor(0xB3FFFFFF);
            label.setTextSize(TypedValue.COMPLEX_UNIT_SP, 13);
            label.setPadding(0, px(12), 0, 0);
            column.addView(label);
            SeekBar bar = new SeekBar(context);
            bar.setMax(max);
            bar.setContentDescription(name);
            bar.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
                @Override
                public void onProgressChanged(SeekBar s, int progress, boolean fromUser) {
                    if (!fromUser || setting) return;
                    exact = null;
                    hsv[0] = hue.getProgress();
                    hsv[1] = saturation.getProgress() / 100f;
                    hsv[2] = brightness.getProgress() / 100f;
                    refresh(true);
                }

                @Override public void onStartTrackingTouch(SeekBar s) {}
                @Override public void onStopTrackingTouch(SeekBar s) {}
            });
            column.addView(bar, new LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT,
                    LinearLayout.LayoutParams.WRAP_CONTENT));
            return bar;
        }

        private int current() {
            return exact != null ? exact : Color.HSVToColor(hsv);
        }

        /** Shows the color everywhere; {@code field}: its code into the text field too. */
        private void refresh(boolean field) {
            setting = true;
            int color = current();
            GradientDrawable well = new GradientDrawable();
            well.setShape(GradientDrawable.OVAL);
            well.setColor(color);
            well.setStroke(px(2), Color.WHITE);
            preview.setBackground(well);
            code.setText(hex(color));
            hue.setProgress(Math.round(hsv[0]));
            saturation.setProgress(Math.round(hsv[1] * 100));
            brightness.setProgress(Math.round(hsv[2] * 100));
            if (field) {
                this.field.setText(hex(color));
                this.field.setSelection(this.field.getText().length());
            }
            setting = false;
        }

        private int px(float dp) {
            return Math.round(Components.dp(context, dp));
        }
    }
}
