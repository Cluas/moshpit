//! A Markdown document in the app's look: NSH's `TextView` (CommonMark and
//! GitHub's tables, task lists, alerts, footnotes) with its parts in the
//! app's tokens and its text at the body's size, following the user's text
//! size. Selectable; a pressed link is the page's to handle.

use nsh_base::{TextViewLinkPressEvent, TextViewPart, TextViewParts, text_view};
use nsh_core::prelude::*;

use crate::ui::{tokens::token, typography::TextToken};

/// `markdown` laid out; `on_link` hears the destination of a pressed link
/// as written (`https://…`, `docs/setup.md`, `#anchor`).
pub fn document(
    markdown: impl Fn() -> SharedString + 'static,
    on_link: impl Fn(String) + 'static,
) -> impl IntoElement {
    provide_context(parts());
    text_view()
        .markdown(markdown)
        .selectable(true)
        .w_full()
        .gap_4()
        .text_size(move || px(TextToken::Body.size(font_scale())))
        .text_color(move || token("foreground"))
        .on_link_press(move |event: &TextViewLinkPressEvent| on_link(event.url.to_string()))
}

/// The parts in the app's tokens: headings in the foreground, quotes and
/// table heads muted, links in the readable accent, code on the muted wash,
/// lines in the border.
fn parts() -> TextViewParts {
    TextViewParts::new()
        .part(TextViewPart::Heading, |s| s.text_color(token("foreground")))
        .part(TextViewPart::Blockquote, |s| {
            s.border_color(token("border-strong"))
                .text_color(token("muted-foreground"))
        })
        .part(TextViewPart::Link, |s| s.text_color(token("primary-text")))
        .part(TextViewPart::Code, |s| s.bg(token("muted")))
        .part(TextViewPart::CodeBlock, |s| {
            s.bg(token("muted")).border_color(token("border"))
        })
        .part(TextViewPart::Table, |s| s.border_color(token("border")))
        .part(TextViewPart::TableCell, |s| s.border_color(token("border")))
        .part(TextViewPart::TableHeaderCell, |s| {
            s.text_color(token("muted-foreground"))
        })
        .part(TextViewPart::Rule, |s| s.border_color(token("border")))
}
