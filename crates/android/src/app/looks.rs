//! The app's looks: the accent every screen wears (Settings › Accent) and
//! the terminal's color scheme (Settings › Theme), their pages' host side
//! (`AccentNSHPage.swift`, `ThemeGalleryNSHPage.swift`), and their editors'
//! (`AccentEditorNSHPage.swift`, `ThemeEditorNSHPage.swift`): the draft is
//! the controller's, kept on the editor's page; the page tells each change
//! and draws its preview from its own copy. Each color row's well is the
//! host's component (`moshpit/color-well`): UIKit's `UIColorWell` on iOS,
//! a swatch that opens the host's color picker here (`ColorPicker.java`).

use nsh_core::color::Rgba;

use moshpit_app::looks::{AccentEditor, EditorIntent, Handled, ThemeEditor};
use moshpit_store::app_themes::AppTheme as StoredAccent;
use moshpit_store::themes::{AnsiSlot, TerminalTheme, ThemeStore};
use moshpit_ui::accent::{Accent, Accents};
use moshpit_ui::theme::AppTheme;
use moshpit_ui::theme_gallery::{Palette, Themes};
use serde_json::Value;

use crate::app::{App, Core, PageKind, Question, text};
use crate::platform::Ask;

const BLACK: Rgba = Rgba {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};

/// A stored hex colour as a colour (the shared editors' rule).
pub use moshpit_app::looks::rgba;

/// The theme a screen wears for accent `accent` (`NSHScreenParams`).
pub fn app_theme(accent: &StoredAccent) -> AppTheme {
    AppTheme {
        accent: rgba(&accent.accent_hex),
        accent_pressed: rgba(&accent.accent_pressed_hex),
        screen_bg: rgba(&accent.screen_bg_hex),
        terminal_bg: rgba(&accent.terminal_bg_hex),
    }
}

fn accent_row(accent: &StoredAccent) -> Accent {
    Accent {
        id: accent.id.clone(),
        name: accent.name.clone(),
        accent: rgba(&accent.accent_hex),
        screen_bg: rgba(&accent.screen_bg_hex),
    }
}

/// `{"op": "accents", …}`.
pub fn accents_message(app: &App, selected: &str, deleting: Option<&str>) -> String {
    let all = app.accents.all();
    let accents = Accents {
        built_in: all
            .iter()
            .filter(|a| a.is_built_in)
            .map(accent_row)
            .collect(),
        custom: all
            .iter()
            .filter(|a| !a.is_built_in)
            .map(accent_row)
            .collect(),
        selected: selected.to_owned(),
        deleting: deleting.map(str::to_owned),
    };
    moshpit_ui::accent::Message::Accents(accents).to_json()
}

fn palette(theme: &TerminalTheme) -> Palette {
    let mut ansi = [BLACK; 8];
    for (slot, color) in AnsiSlot::ALL.iter().zip(ansi.iter_mut()) {
        *color = rgba(&theme.ansi(*slot));
    }
    Palette {
        id: theme.id.clone(),
        name: theme.name.clone(),
        background: rgba(&theme.background_hex),
        foreground: rgba(&theme.foreground_hex),
        cursor: rgba(&theme.cursor_hex),
        ansi,
    }
}

/// `{"op": "themes", …}`.
pub fn themes_message(app: &App, deleting: Option<&str>) -> String {
    let all = app.themes.all_themes();
    let themes = Themes {
        built_in: all.iter().filter(|t| t.is_built_in).map(palette).collect(),
        custom: all.iter().filter(|t| !t.is_built_in).map(palette).collect(),
        selected: app.settings.theme_id(),
        deleting: deleting.map(str::to_owned),
    };
    moshpit_ui::theme_gallery::Message::Themes(themes).to_json()
}

/// The terminal's colors for the view: background, foreground, cursor and
/// the sixteen ANSI colors, as `#RRGGBB`.
pub fn terminal_colors(theme: &TerminalTheme) -> Vec<String> {
    let mut colors = vec![
        format!("#{}", theme.background_hex.trim_start_matches('#')),
        format!("#{}", theme.foreground_hex.trim_start_matches('#')),
        format!("#{}", theme.cursor_hex.trim_start_matches('#')),
    ];
    for slot in AnsiSlot::ALL {
        colors.push(format!("#{}", theme.ansi(slot).trim_start_matches('#')));
    }
    for slot in AnsiSlot::ALL {
        colors.push(format!("#{}", theme.bright(slot).trim_start_matches('#')));
    }
    colors
}

/// What the Accent or Theme page told the host.
pub fn on_event(app: &mut App, core: &Core, surface: u64, ev: &str, event: &Value) {
    let id = text(event, "id");
    let accent = match app.page_mut(surface).map(|p| &mut p.kind) {
        Some(PageKind::Accent { selected, deleting }) => {
            match ev {
                "select" => {
                    if let Some(id) = &id {
                        *selected = id.clone();
                    }
                }
                "delete" | "cancel-delete" => *deleting = None,
                _ => {}
            }
            Some(selected.clone())
        }
        Some(PageKind::Theme { deleting }) => {
            if matches!(ev, "delete" | "cancel-delete") {
                *deleting = None;
            }
            None
        }
        _ => return,
    };
    let set = |app: &App, name: &str, value: String| {
        if let Some(spec) = moshpit_store::settings::spec_named(name) {
            app.settings
                .set(spec, moshpit_store::settings::SettingValue::String(value));
        }
    };
    match (accent, ev) {
        (Some(_), "delete") => {
            if let Some(id) = &id {
                app.accents.delete(id);
            }
        }
        (Some(selected), "back") => {
            // The pick applies as the page goes away, as on iOS; every page
            // made from now on wears it.
            set(app, "appThemeId", selected);
            app.close(surface);
        }
        (None, "select") => {
            if let Some(id) = id {
                set(app, "themeId", id);
            }
        }
        (None, "delete") => {
            if let Some(id) = &id {
                app.themes.delete(id);
            }
        }
        (None, "back") => app.close(surface),
        (Some(_), "add") => {
            // From the accent applied now, as iOS does.
            let applied = app.accents.theme(&app.settings.app_theme_id());
            let draft = AccentEditor::new(&app.accents.make_draft(&applied));
            app.open(core, PageKind::AccentEditor(Box::new(draft)), true);
        }
        (Some(_), "edit") => {
            let custom = id.and_then(|id| {
                app.accents
                    .custom_themes()
                    .iter()
                    .find(|a| a.id == id)
                    .cloned()
            });
            if let Some(draft) = custom {
                let editor = AccentEditor::new(&draft);
                app.open(core, PageKind::AccentEditor(Box::new(editor)), true);
            }
        }
        (None, "new") => {
            let draft = app.themes.make_draft();
            open_theme_editor(app, core, draft);
        }
        (None, "edit") => {
            let custom = id.and_then(|id| {
                app.themes
                    .custom_themes()
                    .iter()
                    .find(|t| t.id == id)
                    .cloned()
            });
            if let Some(draft) = custom {
                open_theme_editor(app, core, draft);
            }
        }
        (None, "import") => {
            // The SwiftUI page's import sheet: the JSON pasted into a text
            // field (one theme or a list of them).
            app.ask(Question::ImportThemes, |token| Ask::Prompt {
                token,
                title: "Import Theme".to_owned(),
                text: String::new(),
                ok: "Import".to_owned(),
                multiline: true,
            });
        }
        _ => {}
    }
    app.refresh();
}

fn open_theme_editor(app: &mut App, core: &Core, theme: TerminalTheme) {
    // The bright rows show from the start when the theme overrides one.
    let editor = ThemeEditor::new(theme);
    app.open(core, PageKind::ThemeEditor(Box::new(editor)), true);
}

/// The person pasted theme JSON: imported as custom themes, the first one
/// picked; what went wrong otherwise, said.
pub fn import_themes(app: &mut App, json: &str) {
    match app.themes.import_themes(json) {
        Ok(imported) => {
            if let Some(first) = imported.first()
                && let Some(spec) = moshpit_store::settings::spec_named("themeId")
            {
                app.settings.set(
                    spec,
                    moshpit_store::settings::SettingValue::String(first.id.clone()),
                );
            }
        }
        Err(error) => app.platform.toast(&match error {
            moshpit_store::themes::ImportError::NotJson => "That isn't theme JSON".to_owned(),
            moshpit_store::themes::ImportError::NoThemes => "No themes in it".to_owned(),
        }),
    }
    app.refresh();
}

/// What an editor (the accent's or a theme's) told the host: what the
/// shared model makes of it.
pub fn on_editor_event(app: &mut App, core: &Core, surface: u64, _ev: &str, event: &Value) {
    let saved = match app.page_mut(surface).map(|p| &mut p.kind) {
        Some(PageKind::AccentEditor(editor)) => match editor.handle(event) {
            Handled::Changed | Handled::Unknown | Handled::Intent(EditorIntent::CopyJson) => return,
            Handled::Intent(EditorIntent::Save) => Saved::Accent(editor.accent()),
            Handled::Intent(EditorIntent::Cancel) => Saved::Nothing,
        },
        Some(PageKind::ThemeEditor(editor)) => match editor.handle(event) {
            Handled::Changed | Handled::Unknown => return,
            Handled::Intent(EditorIntent::CopyJson) => {
                let json = ThemeStore::export_json(&editor.draft);
                app.platform.copy_text(&json);
                app.platform.toast("Theme JSON copied");
                return;
            }
            Handled::Intent(EditorIntent::Save) => Saved::Theme(editor.draft.clone()),
            Handled::Intent(EditorIntent::Cancel) => Saved::Nothing,
        },
        _ => return,
    };
    let set = |app: &App, name: &str, value: String| {
        if let Some(spec) = moshpit_store::settings::spec_named(name) {
            app.settings
                .set(spec, moshpit_store::settings::SettingValue::String(value));
        }
    };
    match saved {
        Saved::Accent(accent) => {
            // Saved at once (the person pressed Save) and picked like any
            // other accent on the page under it, which applies it as it goes.
            app.accents.save(&accent);
            for page in app.pages.iter_mut() {
                if let PageKind::Accent { selected, .. } = &mut page.kind {
                    *selected = accent.id.clone();
                }
            }
        }
        Saved::Theme(theme) => {
            // Editing implies wanting it.
            let id = theme.id.clone();
            app.themes.save(theme);
            set(app, "themeId", id);
        }
        Saved::Nothing => {}
    }
    app.close(surface);
    app.refresh();
    let _ = core;
}

/// What an editor's Save (or its going away) leaves.
enum Saved {
    Accent(StoredAccent),
    Theme(TerminalTheme),
    Nothing,
}
