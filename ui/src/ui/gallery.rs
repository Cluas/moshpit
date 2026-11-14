//! The `ui` layer's gallery (`docs/nsh/UI-DESIGN.md` §4.20): every
//! component in its variants, sizes and states, in the surface's
//! appearance (the host runs it light with `{"light": true}`). A new
//! component is reviewed here before a screen uses it. It is a debug
//! screen (`ui-gallery`): the host opens it from the hidden core switches.
//!
//! Presses on the gallery's live controls count up in its header, so its
//! tests can tell a press from none.

use nsh_core::prelude::*;

use crate::ui::{
    button::{IconButtonSize, IconVariant, Size, Variant, button, icon_button},
    control::{option, segmented, select, switch},
    empty::{EmptyVariant, empty, loaded, skeleton_list, skeleton_row},
    feedback::{SpinnerSize, separator, spinner},
    field::{
        FieldLayout, SecretShown, field, number_input, search_field, secret_field, text_input,
    },
    icon::{IconSize, Tone, icon},
    key::{ChipVariant, KeyVariant, chip, content_key, key, symbol_key},
    mark::{Face, MarkSize, agent_mark, host_mark},
    notice::{BannerTone, banner, toasts},
    row::{RowTone, row as list_row},
    section::{Inset, SectionVariant, section as group},
    status::{BadgeSize, BadgeVariant, DotSize, Status, badge, status_dot},
    swatch::cursor_colors,
    tokens::{number, token},
    typography::{TextToken, eyebrow},
};

/// The gallery's props: the surface it runs on.
#[derive(Clone, PartialEq, Props)]
pub struct ScreenProps {
    /// The surface's id.
    pub surface: Prop<u64>,
}

/// The test ids of the controls whose presses count.
pub const PRESSABLE: &[&str] = &[
    "ui-gallery-press-primary",
    "ui-gallery-press-secondary",
    "ui-gallery-press-outline",
    "ui-gallery-press-ghost",
    "ui-gallery-press-destructive",
    "ui-gallery-press-sm",
    "ui-gallery-press-md",
    "ui-gallery-press-lg",
    "ui-gallery-press-icon",
    "ui-gallery-press-bar",
    "ui-gallery-press-bar-commit",
    "ui-gallery-press-icon-glass",
    "ui-gallery-press-icon-ghost",
    "ui-gallery-press-icon-tinted",
    "ui-gallery-press-icon-sm",
    "ui-gallery-press-icon-bar",
    "ui-gallery-row-nav",
    "ui-gallery-row-subtitle",
    "ui-gallery-row-action",
    "ui-gallery-row-destructive",
    "ui-gallery-row-detail",
    "ui-gallery-press-empty",
    "ui-gallery-press-empty-inline",
    "ui-gallery-key-esc",
    "ui-gallery-key-ctrl",
    "ui-gallery-key-image",
    "ui-gallery-key-keyboard",
    "ui-gallery-key-content",
    "ui-gallery-chip-default",
    "ui-gallery-chip-custom",
    "ui-gallery-chip-toggle",
    "ui-gallery-chip-x",
    "ui-gallery-chip-symbol",
    "ui-gallery-press-icon-remove",
    "ui-gallery-press-icon-add",
    "ui-gallery-swatch-accent",
    "ui-gallery-swatch-teal",
    "ui-gallery-swatch-green",
    "ui-gallery-swatch-white",
];

/// The gallery's choice rows: test id and title.
pub const CHOICES: &[(&str, &str)] = &[
    ("ui-gallery-row-check-auto", "Automatic"),
    ("ui-gallery-row-check-light", "Light"),
    ("ui-gallery-row-check-dark", "Dark"),
];

/// The gallery's icon buttons: test id, symbol, what screen readers say,
/// look and size.
pub const ICON_BUTTONS: &[(&str, &str, &str, IconVariant, IconButtonSize)] = &[
    (
        "ui-gallery-press-icon-glass",
        "gearshape",
        "Settings",
        IconVariant::Glass,
        IconButtonSize::Md,
    ),
    (
        "ui-gallery-press-icon-ghost",
        "ellipsis",
        "More",
        IconVariant::Ghost,
        IconButtonSize::Md,
    ),
    (
        "ui-gallery-press-icon-tinted",
        "plus",
        "Add connection",
        IconVariant::Tinted,
        IconButtonSize::Md,
    ),
    (
        "ui-gallery-press-icon-sm",
        "plus",
        "New session",
        IconVariant::Tinted,
        IconButtonSize::Sm,
    ),
    (
        "ui-gallery-press-icon-bar",
        "plus",
        "Add connection",
        IconVariant::Bar,
        IconButtonSize::Md,
    ),
];

/// The gallery's fields, by test id, in the order they show: every state
/// of [`crate::ui::field`]. The first three are a chain: Return moves from
/// each to the next, and the third's Return key says Done.
pub const FIELDS: &[&str] = &[
    "ui-gallery-field-empty",
    "ui-gallery-field-filled",
    "ui-gallery-field-number",
    "ui-gallery-field-focused",
    "ui-gallery-field-invalid",
    "ui-gallery-field-disabled",
    "ui-gallery-field-stacked",
    "ui-gallery-field-secret-empty",
    "ui-gallery-field-secret-saved",
    "ui-gallery-field-secret-replacing",
];

/// The gallery's fields ([`FIELDS`]), one per row, rows drawing their own
/// insets as they do in a section's card (so the card's padding is undone
/// around them).
fn fields() -> View {
    let (name, host, port) = (element_ref(), element_ref(), element_ref());
    let rows: Vec<AnyElement> = vec![
        field(
            "Name",
            text_input()
                .placeholder("my-server")
                .element_ref(&name)
                .next(&host),
        )
        .test_id("ui-gallery-field-empty")
        .into_element(),
        field(
            "Host",
            text_input()
                .mono()
                .default_value(|| "build.example.com".to_owned())
                .element_ref(&host)
                .next(&port),
        )
        .test_id("ui-gallery-field-filled")
        .into_element(),
        field(
            "Port",
            number_input()
                .default_value(|| "22".to_owned())
                .element_ref(&port),
        )
        .test_id("ui-gallery-field-number")
        .into_element(),
        field(
            "User",
            text_input().mono().default_value(|| "dev".to_owned()),
        )
        .show_focus()
        .test_id("ui-gallery-field-focused")
        .into_element(),
        field(
            "Proxy port",
            number_input().default_value(|| "70000".to_owned()),
        )
        .error(|| Some("Port must be between 1 and 65535.".into()))
        .test_id("ui-gallery-field-invalid")
        .into_element(),
        field("SSH port", number_input().default_value(|| "22".to_owned()))
            .disabled(|| true)
            .test_id("ui-gallery-field-disabled")
            .into_element(),
        field(
            "mosh-server path",
            text_input().mono().placeholder("Automatic"),
        )
        .layout(FieldLayout::Stacked)
        .description("Only where the host's PATH lacks it.")
        .test_id("ui-gallery-field-stacked")
        .into_element(),
        secret_field(
            "Password",
            "moshpit/connection-password",
            "ui-gallery-password-empty",
        )
        .test_id("ui-gallery-field-secret-empty")
        .into_element(),
        secret_field(
            "Password",
            "moshpit/connection-password",
            "ui-gallery-password-saved",
        )
        .shown(|| SecretShown::Saved)
        .test_id("ui-gallery-field-secret-saved")
        .into_element(),
        secret_field(
            "Password",
            "moshpit/connection-password",
            "ui-gallery-password-replacing",
        )
        .shown(|| SecretShown::Replacing)
        .test_id("ui-gallery-field-secret-replacing")
        .into_element(),
    ];
    let mut list = view().mx(px(-16.0)).my(px(-16.0));
    for (i, row) in rows.into_iter().enumerate() {
        if i > 0 {
            list = list.child(separator(16.0));
        }
        list = list.child(row);
    }
    list
}

/// The search field (a list's filter) and a textarea (a field of prose
/// that grows from four lines to twelve): R15 and R16's pieces.
fn prose() -> View {
    view()
        .gap(px(12.0))
        .child(search_field("Search languages", |_| {}).test_id("ui-gallery-search"))
        .child(
            view().mx(px(-16.0)).mb(px(-16.0)).child(
                field(
                    "Message",
                    text_input()
                        .multiline(4, 12)
                        .placeholder("What broke, or what you want"),
                )
                .layout(FieldLayout::Stacked)
                .test_id("ui-gallery-field-textarea"),
            ),
        )
}

/// A section: its title (an eyebrow, on the cards' text line) over a card.
fn section(title: &'static str, body: impl IntoElement) -> View {
    view()
        .gap(px(8.0))
        .child(view().ps(px(16.0)).child(eyebrow(text(title))))
        .child(
            view()
                .bg(move || token("card-fill"))
                .rounded(px(number("radius-lg")))
                .p(px(16.0))
                .gap(px(12.0))
                .child(body),
        )
}

/// A wrapping row of things, 12 pt apart.
fn row() -> View {
    view().flex_row().flex_wrap().items_center().gap(px(12.0))
}

/// Words that change, in the muted tone.
fn caption_of(words: impl Fn() -> String + 'static) -> Text {
    TextToken::Subhead
        .apply(text(words))
        .text_color(move || token("muted-foreground"))
}

/// Words beside a thing, in the muted tone.
fn caption(words: &'static str) -> Text {
    TextToken::Subhead
        .apply(text(words))
        .text_color(move || token("muted-foreground"))
}

#[component]
pub fn GalleryScreen(props: ScreenProps) -> impl IntoElement {
    let _ = props.surface;
    let presses = signal(0_u32);
    let loading = signal(false);
    let choice = signal("Automatic");
    let expanded = signal(false);
    let notify = signal(true);
    let cursor = signal(0_usize);
    let mux = signal(SharedString::from("tmux"));
    let arrived = signal(false);
    let toasts = toasts();
    let banner_shown = signal(true);
    let press = move || presses.update(|n| *n += 1);
    let held = signal(0_u32);

    let variants = row()
        .child(
            button("Save")
                .variant(Variant::Primary)
                .on_press(press)
                .test_id("ui-gallery-press-primary"),
        )
        .child(
            button("Insert")
                .on_press(press)
                .test_id("ui-gallery-press-secondary"),
        )
        .child(
            button("Reconnect")
                .variant(Variant::Outline)
                .on_press(press)
                .test_id("ui-gallery-press-outline"),
        )
        .child(
            button("Cancel")
                .variant(Variant::Ghost)
                .on_press(press)
                .test_id("ui-gallery-press-ghost"),
        )
        .child(
            button("Delete…")
                .variant(Variant::Destructive)
                .on_press(press)
                .test_id("ui-gallery-press-destructive"),
        );
    let sizes = row()
        .child(
            button("Small")
                .variant(Variant::Primary)
                .size(Size::Sm)
                .on_press(press)
                .test_id("ui-gallery-press-sm"),
        )
        .child(
            button("Medium")
                .variant(Variant::Primary)
                .on_press(press)
                .test_id("ui-gallery-press-md"),
        )
        .child(
            button("Large")
                .variant(Variant::Primary)
                .size(Size::Lg)
                .on_press(press)
                .test_id("ui-gallery-press-lg"),
        )
        .child(
            button("Add")
                .icon("plus")
                .on_press(press)
                .test_id("ui-gallery-press-icon"),
        );
    let states = row()
        .child(
            button("Disabled")
                .variant(Variant::Primary)
                .disabled(|| true)
                .on_press(press)
                .test_id("ui-gallery-disabled"),
        )
        .child(
            button("Send")
                .variant(Variant::Primary)
                .loading(move || loading.get())
                .on_press(press)
                .test_id("ui-gallery-loading"),
        )
        .child(
            button("Toggle loading")
                .variant(Variant::Outline)
                .size(Size::Sm)
                .on_press(move || loading.update(|l| *l = !*l))
                .test_id("ui-gallery-toggle-loading"),
        );
    let bar = row()
        .child(
            button("Cancel")
                .variant(Variant::Bar)
                .on_press(press)
                .test_id("ui-gallery-press-bar"),
        )
        .child(
            button("Done")
                .variant(Variant::BarCommit)
                .on_press(press)
                .test_id("ui-gallery-press-bar-commit"),
        );
    let icon_buttons = row()
        .children(
            ICON_BUTTONS
                .iter()
                .map(move |&(id, symbol, label, variant, size)| {
                    icon_button(symbol, label)
                        .variant(variant)
                        .size(size)
                        .on_press(press)
                        .test_id(id)
                }),
        )
        // A symbol in the tone of what it does: off the bar, on it.
        .child(
            icon_button("minus.circle.fill", "Remove esc from toolbar")
                .size(IconButtonSize::Sm)
                .tone(Tone::Destructive)
                .on_press(press)
                .test_id("ui-gallery-press-icon-remove"),
        )
        .child(
            icon_button("plus.circle.fill", "Add ^L to toolbar")
                .size(IconButtonSize::Sm)
                .tone(Tone::Success)
                .on_press(press)
                .test_id("ui-gallery-press-icon-add"),
        );
    let badges = view()
        .gap(px(12.0))
        .child(
            row()
                .child(badge("SSH").mono())
                .child(badge("mosh").mono())
                .child(badge("Current").variant(BadgeVariant::Outline))
                .child(badge("7 of 12").variant(BadgeVariant::Primary)),
        )
        .child(
            row()
                .child(
                    badge("Saved")
                        .variant(BadgeVariant::Success)
                        .icon("checkmark"),
                )
                .child(badge("1 waiting").variant(BadgeVariant::Warning))
                .child(badge("Done").variant(BadgeVariant::Info))
                .child(badge("Failed").variant(BadgeVariant::Destructive)),
        )
        .child(
            row()
                .child(badge("ed25519").mono().size(BadgeSize::Md))
                .child(
                    badge("Secure Enclave")
                        .variant(BadgeVariant::Success)
                        .size(BadgeSize::Md),
                ),
        );
    let dot = |status: Status, words: &'static str| {
        view()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .child(status_dot(status))
            .child(caption(words))
    };
    let statuses = view()
        .gap(px(12.0))
        .child(
            row()
                .child(dot(Status::Attention, "Needs you"))
                .child(dot(Status::Working, "Working"))
                .child(dot(Status::Done, "Done"))
                .child(dot(Status::Idle, "Idle")),
        )
        .child(
            row()
                .child(dot(Status::Live, "Online"))
                .child(dot(Status::Connecting, "Connecting…"))
                .child(dot(Status::Offline, "Offline"))
                .child(dot(Status::Error, "Failed")),
        )
        .child(row().child(status_dot(Status::Live).size(DotSize::Md).label("Online")));
    // Marks (R6): a host's on a head and in a row's lane, in each state;
    // an agent's glyph in each.
    let marks = view()
        .gap(px(12.0))
        .test_id("ui-gallery-marks")
        .child(
            row()
                .child(host_mark("S").status(Status::Live))
                .child(host_mark("B").status(Status::Connecting))
                .child(host_mark("E").status(Status::Offline))
                .child(host_mark("G").status(Status::Error))
                .child(host_mark("L")),
        )
        // A host key's face in each hue, a grid of another side, an emoji,
        // and both muted offline.
        .child(row().children((0..8u8).map(|hue| {
            host_mark("F")
                .face(Some(demo_face(
                    5,
                    0x5A5Au32.rotate_left(u32::from(hue) * 3),
                    hue,
                )))
                .size(MarkSize::Sm)
                .into_element()
        })))
        .child(
            row()
                .child(
                    host_mark("W")
                        .face(Some(demo_face(5, 0x2B5D, 5)))
                        .status(Status::Live),
                )
                .child(
                    host_mark("W")
                        .face(Some(demo_face(5, 0x2B5D, 5)))
                        .status(Status::Offline),
                )
                .child(host_mark("S").face(Some(demo_face(7, 0x0F3C_A5E1, 2))))
                .child(host_mark("E").emoji(Some("🐳")).status(Status::Live))
                .child(host_mark("E").emoji(Some("🐳")).status(Status::Offline))
                .child(host_mark("E").emoji(Some("🍓")).size(MarkSize::Sm)),
        )
        .child(
            row()
                .child(host_mark("P").size(MarkSize::Sm))
                .child(host_mark("D").size(MarkSize::Sm).status(Status::Live))
                .child(agent_mark(Status::Attention))
                .child(agent_mark(Status::Working))
                .child(agent_mark(Status::Done))
                .child(agent_mark(Status::Idle)),
        );
    let leaves = view()
        .gap(px(12.0))
        .child(
            row()
                .child(icon("terminal").size(IconSize::Sm))
                .child(icon("terminal"))
                .child(icon("terminal").size(IconSize::Lg))
                .child(icon("terminal").size(IconSize::Key).tone(Tone::Key))
                .child(
                    icon("hand.raised.fill")
                        .tone(Tone::Warning)
                        .label("Needs you"),
                )
                .child(icon("checkmark.circle").tone(Tone::Info))
                .child(icon("sparkle").tone(Tone::Muted)),
        )
        .child(
            row()
                .child(spinner(SpinnerSize::Sm))
                .child(spinner(SpinnerSize::Md)),
        )
        .child(separator(0.0))
        .child(separator(40.0));
    let empties = view()
        .gap(px(12.0))
        .child(
            empty("No connections yet")
                .message("Add a host to keep your agents within reach.")
                .action(
                    button("Add Connection")
                        .variant(Variant::Primary)
                        .on_press(press)
                        .test_id("ui-gallery-press-empty"),
                )
                .test_id("ui-gallery-empty-page"),
        )
        .child(separator(0.0))
        .child(
            empty("No keys yet")
                .variant(EmptyVariant::Inline)
                .icon("key")
                .message("Generate one on this phone, or import one.")
                .action(
                    button("Generate Key")
                        .on_press(press)
                        .test_id("ui-gallery-press-empty-inline"),
                )
                .test_id("ui-gallery-empty-inline"),
        );
    let skeletons = view()
        .gap(px(12.0))
        .child(skeleton_row().test_id("ui-gallery-skeleton-row"))
        .child(
            skeleton_row()
                .leading(true)
                .test_id("ui-gallery-skeleton-leading"),
        )
        .child(
            skeleton_row()
                .subtitle(false)
                .test_id("ui-gallery-skeleton-title"),
        )
        .child(separator(0.0))
        .child(loaded(
            move || arrived.get(),
            || {
                skeleton_list(2)
                    .leading(true)
                    .test_id("ui-gallery-skeleton-list")
                    .into_element()
            },
            || {
                view()
                    .gap(px(8.0))
                    .test_id("ui-gallery-loaded-content")
                    .child(caption("devbox · 3 sessions"))
                    .child(caption("api · 1 agent working"))
                    .into_element()
            },
        ))
        .child(
            button("Toggle loaded")
                .variant(Variant::Outline)
                .size(Size::Sm)
                .on_press(move || arrived.update(|a| *a = !*a))
                .test_id("ui-gallery-toggle-loaded"),
        );
    let banners = view()
        .gap(px(12.0))
        .child(
            banner(BannerTone::Info, || "Reconnecting…".into()).test_id("ui-gallery-banner-info"),
        )
        .child(
            banner(BannerTone::Warning, || "This host's key changed.".into())
                .test_id("ui-gallery-banner-warning"),
        )
        .child(
            banner(BannerTone::Destructive, || "The session ended.".into())
                .test_id("ui-gallery-banner-destructive"),
        )
        .child(
            banner(BannerTone::Info, || {
                "Read-only: another phone has the session.".into()
            })
            .action("Take Over", press)
            .action_test_id("ui-gallery-banner-take-over")
            .test_id("ui-gallery-banner-action"),
        )
        .child(
            banner(BannerTone::Warning, || {
                "Notifications are off for Moshpit.".into()
            })
            .on_close(move || banner_shown.set(false))
            .shown(move || banner_shown.get())
            .test_id("ui-gallery-banner-close"),
        )
        .child(
            button("Show banner")
                .variant(Variant::Outline)
                .size(Size::Sm)
                .on_press(move || banner_shown.set(true))
                .test_id("ui-gallery-show-banner"),
        )
        // Floating over the terminal's own ground, as the terminal's
        // banners do (R12).
        .child(
            view()
                .p(px(12.0))
                .rounded(px(number("radius-sm")))
                .bg(move || token("terminal-bg"))
                .child(
                    banner(BannerTone::Info, || {
                        "Network changed. mosh is reconnecting.".into()
                    })
                    .floating()
                    .test_id("ui-gallery-banner-floating"),
                ),
        );
    let toast = row().child(
        button("Copy")
            .icon("doc.on.doc")
            .on_press(move || toasts.show("Copied"))
            .test_id("ui-gallery-toast-show"),
    );

    // Keys on the terminal's own ground; ⌃ arms and lets go as it is
    // pressed; one key is disabled.
    let armed = signal(false);
    let keys = view()
        .gap(px(12.0))
        .p(px(12.0))
        .rounded(px(number("radius-sm")))
        .bg(move || token("terminal-bg"))
        .child(
            row()
                .gap(px(6.0))
                .child(key("esc").on_press(press).test_id("ui-gallery-key-esc"))
                .child(
                    key("⌃")
                        .variant(KeyVariant::Modifier)
                        .armed(move || armed.get())
                        .on_press(move || {
                            armed.update(|a| *a = !*a);
                            press();
                        })
                        .test_id("ui-gallery-key-ctrl"),
                )
                .child(
                    symbol_key("photo", "Attach image")
                        .on_press(press)
                        .test_id("ui-gallery-key-image"),
                )
                .child(
                    symbol_key("keyboard.chevron.compact.down", "Hide keyboard")
                        .variant(KeyVariant::Toggle)
                        .on_press(press)
                        .test_id("ui-gallery-key-keyboard"),
                )
                // A face its caller draws (the terminal's keyboard key,
                // whose two glyphs swap), with a name of its own.
                .child(
                    content_key(
                        icon("keyboard").size(IconSize::Key).tone(Tone::Key),
                        "Show keyboard",
                    )
                    .variant(KeyVariant::Toggle)
                    .on_press(press)
                    .test_id("ui-gallery-key-content"),
                ),
        )
        .child(
            row()
                .gap(px(6.0))
                .child(
                    key("tab")
                        .disabled(|| true)
                        .on_press(press)
                        .test_id("ui-gallery-key-disabled"),
                )
                // No press handler: display only (Settings' preview of the
                // bar).
                .child(key("^C").test_id("ui-gallery-key-display")),
        );
    let picked_chip = signal(true);
    let chips = row()
        .gap(px(8.0))
        .child(
            chip("esc")
                .mono()
                .on_press(press)
                .test_id("ui-gallery-chip-default"),
        )
        .child(
            chip("deploy")
                .variant(ChipVariant::Custom)
                .on_press(press)
                .test_id("ui-gallery-chip-custom"),
        )
        .child(
            chip("build-box")
                .variant(ChipVariant::Toggle)
                .mono()
                .selected(move || picked_chip.get())
                .on_press(move || {
                    picked_chip.update(|p| *p = !*p);
                    press();
                })
                .test_id("ui-gallery-chip-toggle"),
        )
        .child(
            chip("x")
                .mono()
                .on_press(press)
                .test_id("ui-gallery-chip-x"),
        )
        .child(
            chip("Files")
                .symbol("folder")
                .on_press(press)
                .test_id("ui-gallery-chip-symbol"),
        )
        // No press handler: display only (a shortcut's keycap in its row).
        .child(chip("^L").mono().test_id("ui-gallery-chip-display"));
    // The cursor's colors (B318) for a stored id, the one shown picked
    // ringed; a fixed color like the surface's accent is not shown, and
    // the stored id under them stays what it was.
    let stored = signal("green".to_owned());
    let swatches = view()
        .gap(px(12.0))
        .child(
            cursor_colors(
                move || stored.get(),
                move |id| {
                    stored.set(id.to_owned());
                    press();
                },
            )
            .test_id("ui-gallery-swatch"),
        )
        .child(
            caption_of(move || format!("Stored: {}", stored.get()))
                .test_id("ui-gallery-swatch-stored"),
        );

    let page = scroll()
        .flex_1()
        .bg(move || token("background"))
        .test_id("ui-gallery")
        .child(
            view()
                .px(px(number("space-page")))
                .pt_safe_area_plus(px(16.0))
                .pb_safe_area_plus(px(32.0))
                .gap(px(24.0))
                .child(
                    view()
                        .gap(px(4.0))
                        .child(
                            TextToken::Title
                                .apply(text("UI gallery"))
                                .text_color(move || token("foreground"))
                                .role(AccessibilityRole::Header),
                        )
                        .child(
                            TextToken::Subhead
                                .apply(text(move || format!("Presses: {}", presses.get())))
                                .text_color(move || token("muted-foreground"))
                                .test_id("ui-gallery-presses"),
                        )
                        .child(
                            TextToken::Subhead
                                .apply(text(move || format!("Long presses: {}", held.get())))
                                .text_color(move || token("muted-foreground"))
                                .test_id("ui-gallery-held"),
                        ),
                )
                .child(section(
                    "Button",
                    view()
                        .gap(px(12.0))
                        .child(variants)
                        .child(sizes)
                        .child(states)
                        .child(bar),
                ))
                .child(section("Icon button", icon_buttons))
                .child(section("Field", fields()))
                .child(section("Search · Textarea", prose()))
                .child(section("Badge", badges))
                .child(section("Status", statuses))
                .child(section("Mark", marks))
                .child(section("Icon · Spinner · Separator", leaves))
                .child(
                    group()
                        .header("Row")
                        .footer("Lanes: leading 28, title, value, accessory. A row without a leading icon keeps the lane.")
                        .row(
                            list_row("Appearance")
                                .leading_tile("paintbrush")
                                .value("Automatic")
                                .chevron()
                                .on_press(press)
                                .test_id("ui-gallery-row-nav"),
                        )
                        .row(
                            list_row("Server binary")
                                .subtitle("/usr/local/bin/mosh-server")
                                .mono_subtitle()
                                .chevron()
                                .on_press(press)
                                .test_id("ui-gallery-row-subtitle"),
                        )
                        .row(
                            list_row("Predictive echo")
                                .leading_icon("keyboard")
                                .value("Adaptive")
                                .test_id("ui-gallery-row-value"),
                        )
                        .row(
                            list_row("Notifications")
                                .leading_icon("bell")
                                .trailing(
                                    switch(move || notify.get(), move |on| notify.set(on))
                                        .label("Notifications")
                                        .test_id("ui-gallery-switch"),
                                )
                                .test_id("ui-gallery-row-toggle"),
                        )
                        .row(
                            list_row("Reconnect")
                                .tone(RowTone::Primary)
                                .on_press(press)
                                .test_id("ui-gallery-row-action"),
                        )
                        .row(
                            list_row("Delete Connection…")
                                .tone(RowTone::Destructive)
                                .on_press(press)
                                .test_id("ui-gallery-row-destructive"),
                        )
                        .row(
                            list_row("laptop")
                                .leading_icon("key")
                                .subtitle("SHA256:Xk3vQ8pL2mN9rT4wY7zB1cD6fG0hJ5sA9fQ")
                                .mono_subtitle()
                                .detail(|| "Used for build-box, gpu-01".into())
                                .badge(badge("ED25519").mono())
                                .on_press(press)
                                .on_long_press(move || held.update(|n| *n += 1))
                                .test_id("ui-gallery-row-detail"),
                        )
                        .test_id("ui-gallery-section-rows"),
                )
                .child(
                    group()
                        .header("Choice · Disclosure · States")
                        .rows(CHOICES.iter().map(move |&(id, name)| {
                            list_row(name)
                                .check(move || choice.get() == name)
                                .on_press(move || choice.set(name))
                                .test_id(id)
                        }))
                        .row(
                            list_row("Advanced")
                                .disclosure(move || expanded.get())
                                .on_press(move || expanded.update(|e| *e = !*e))
                                .test_id("ui-gallery-row-disclosure"),
                        )
                        .row(
                            list_row("Checking the host")
                                .loading(|| true)
                                .on_press(press)
                                .test_id("ui-gallery-row-loading"),
                        )
                        .row(
                            list_row("Not available")
                                .disabled(|| true)
                                .on_press(press)
                                .test_id("ui-gallery-row-disabled"),
                        )
                        .test_id("ui-gallery-section-choice"),
                )
                .child(
                    group()
                        .header("Switch · Segmented · Select")
                        .item(
                            view().px(px(16.0)).py(px(8.0)).child(
                                segmented(
                                    ["Block", "Bar", "Underline"],
                                    move || Some(cursor.get()),
                                    move |i| cursor.set(i),
                                )
                                .label("Cursor")
                                .test_id("ui-gallery-segmented"),
                            ),
                            Inset::Content,
                        )
                        .item(
                            select(
                                "Multiplexer",
                                [
                                    option("tmux", "tmux").detail("On almost every host"),
                                    option("herdr", "herdr").detail("Knows what its agents do"),
                                    option("none", "None"),
                                ],
                                move || mux.get(),
                                move |id| mux.set(id),
                            )
                            .test_id("ui-gallery-select"),
                            Inset::Content,
                        )
                        .item(
                            select(
                                "Waiting",
                                [option("tmux", "tmux"), option("herdr", "herdr")],
                                move || mux.get(),
                                move |id| mux.set(id),
                            )
                            .disabled(|| true)
                            .test_id("ui-gallery-select-disabled"),
                            Inset::Content,
                        )
                        .test_id("ui-gallery-section-controls"),
                )
                .child(
                    group()
                        .variant(SectionVariant::Plain)
                        .header("Plain")
                        .row(list_row("Terminal size").value("80 × 24"))
                        .test_id("ui-gallery-section-plain"),
                )
                .child(section("Empty", empties))
                .child(section("Skeleton", skeletons))
                .child(section("Banner", banners))
                .child(section("Toast", toast))
                .child(section("Key", keys))
                .child(section("Chip", chips))
                .child(section("Swatch", swatches)),
        );
    // The toasts over the page, 8 pt above its bottom safe area.
    view()
        .flex_1()
        .child(page)
        .child(toasts.host().test_id("ui-gallery-toast"))
}

/// A `side`-cell face for the gallery, mirrored left to right, its left
/// half and middle from `bits`.
fn demo_face(side: usize, bits: u32, hue: u8) -> Face {
    let half = side.div_ceil(2);
    let cells = (0..side * side)
        .map(|at| {
            let (row, column) = (at / side, at % side);
            let column = column.min(side - 1 - column);
            (bits >> ((row * half + column) % 32)) & 1 == 1
        })
        .collect();
    Face {
        side: side as u8,
        cells,
        hue,
        kinds: Vec::new(),
        asleep: Vec::new(),
    }
}
