//! A group of rows (`docs/nsh/UI-DESIGN.md` §4.6): an optional header, the
//! items on a card, an optional footer.
//!
//! The section draws the separators between its items, each starting where
//! the item under it says its content starts ([`Inset`]): at the content
//! (16 pt) or past the leading lane (56 pt). When any of its rows has
//! something in the leading lane, every row keeps the lane, so the titles
//! of a section start on one line (§4.5: a missing slot keeps its place).

use nsh_core::prelude::*;

use crate::ui::{
    feedback::separator,
    row::{GAP, LEADING, PADDING_X, Row},
    tokens::{number, token},
    typography::{TextToken, eyebrow},
};

/// Where a separator above an item starts, from the card's leading edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Inset {
    /// At the content: 16 pt (a row's padding).
    Content,
    /// Past the leading lane: 56 pt (padding, lane and gap).
    Lead,
}

impl Inset {
    pub fn points(self) -> f32 {
        match self {
            Inset::Content => PADDING_X,
            Inset::Lead => PADDING_X + LEADING + GAP,
        }
    }
}

/// How a section sets its items off (§4.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SectionVariant {
    /// On a card: `card-fill`, `radius-lg`, the card edge the owner picked
    /// (`card-border`, `card-blur`). Settings, forms.
    Grouped,
    /// No card: a preview, an empty state.
    Plain,
    /// A card with a head of its own over its rows, a separator between
    /// (Home's connection card).
    Card,
}

/// What a section holds, in order.
enum Item {
    // Both boxed: a row and an element are hundreds of bytes each.
    Row(Box<Row>),
    Other(Box<AnyElement>, Inset),
}

/// A section ([`section`]).
pub struct Section {
    header: Option<SharedString>,
    footer: Option<SharedString>,
    head: Option<AnyElement>,
    items: Vec<Item>,
    variant: SectionVariant,
    test_id: Option<SharedString>,
}

/// A grouped section, empty until rows are added.
pub fn section() -> Section {
    Section {
        header: None,
        footer: None,
        head: None,
        items: Vec::new(),
        variant: SectionVariant::Grouped,
        test_id: None,
    }
}

impl Section {
    pub fn variant(mut self, variant: SectionVariant) -> Self {
        self.variant = variant;
        self
    }

    /// The section's title over its card: an eyebrow (a heading to screen
    /// readers).
    pub fn header(mut self, header: impl Into<SharedString>) -> Self {
        self.header = Some(header.into());
        self
    }

    /// What the section's settings do, under its card: at most two lines
    /// at the default text size.
    pub fn footer(mut self, footer: impl Into<SharedString>) -> Self {
        self.footer = Some(footer.into());
        self
    }

    /// A [`SectionVariant::Card`]'s head, over its rows.
    pub fn head(mut self, head: impl IntoElement) -> Self {
        self.head = Some(head.into_element());
        self
    }

    /// A row; the separator above it starts where its content does.
    pub fn row(mut self, row: Row) -> Self {
        self.items.push(Item::Row(Box::new(row)));
        self
    }

    /// Rows, in order.
    pub fn rows(mut self, rows: impl IntoIterator<Item = Row>) -> Self {
        self.items
            .extend(rows.into_iter().map(|row| Item::Row(Box::new(row))));
        self
    }

    /// Something else on the card (a field), the separator above it at
    /// `inset`.
    pub fn item(mut self, item: impl IntoElement, inset: Inset) -> Self {
        self.items
            .push(Item::Other(Box::new(item.into_element()), inset));
        self
    }

    pub fn test_id(mut self, id: &str) -> Self {
        self.test_id = Some(SharedString::from(id.to_owned()));
        self
    }
}

impl IntoElement for Section {
    fn into_element(self) -> AnyElement {
        let Section {
            header,
            footer,
            head,
            items,
            variant,
            test_id,
        } = self;
        // One spine for the section's titles.
        let lane = items.iter().any(|item| match item {
            Item::Row(row) => row.inset() == Inset::Lead,
            Item::Other(_, inset) => *inset == Inset::Lead,
        });
        let mut body = view().w_full();
        let mut first = true;
        if let Some(head) = head.filter(|_| variant == SectionVariant::Card) {
            body = body.child(head);
            first = false;
        }
        for item in items {
            let (element, inset) = match item {
                Item::Row(row) => {
                    let row = if lane { row.reserve_leading() } else { *row };
                    let inset = row.inset();
                    (row.into_element(), inset)
                }
                Item::Other(element, inset) => (*element, inset),
            };
            if !first {
                body = body.child(separator(inset.points()));
            }
            first = false;
            body = body.child(element);
        }
        let body = match variant {
            SectionVariant::Plain => body,
            SectionVariant::Grouped | SectionVariant::Card => body
                .bg(move || token("card-fill"))
                .rounded(px(number("radius-lg")))
                .border_curve(BorderCurve::Continuous)
                .border(px(number("card-border-width")))
                .border_color(move || token("card-border"))
                .backdrop_blur(px(number("card-blur")))
                // A row's press fills it inside the card's corners.
                .overflow_hidden(),
        };
        let mut section = view().w_full().gap(px(8.0));
        if let Some(header) = header {
            section = section.child(
                view()
                    .ps(px(PADDING_X))
                    .child(eyebrow(text(header)).role(AccessibilityRole::Header)),
            );
        }
        section = section.child(body);
        if let Some(footer) = footer {
            section = section.child(
                view().px(px(PADDING_X)).child(
                    TextToken::Footnote
                        .apply(text(footer))
                        .text_color(move || token("muted-foreground")),
                ),
            );
        }
        if let Some(id) = &test_id {
            section = section.test_id(id);
        }
        section.into_element()
    }
}
