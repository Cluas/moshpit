//! The selection and search models, on the emulator's stable row numbers
//! (a row keeps its number while output scrolls it up, and the number of
//! a row that left the history is below the first one still kept).

use wezterm_term::StableRowIndex;

use crate::SelectKind;

/// A row's cells — (column, width, text) — and whether the row continues
/// on the next one.
pub(crate) type RowCells = (Vec<(usize, usize, String)>, bool);

/// A cell position.
pub(crate) type Point = (StableRowIndex, usize);

#[derive(Debug, Clone)]
pub(crate) struct Selection {
    pub kind: SelectKind,
    anchor: (Point, bool),
    head: (Point, bool),
    /// What a long press selected (a link): kept whole however the finger
    /// moves, the selection growing from it (B478).
    span: Option<((Point, bool), (Point, bool))>,
    /// The emulator's sequence number when the selection was made or
    /// moved: a row under it that changed since means its text is gone,
    /// unless it still reads as it did then (`rows`, B480).
    pub seqno: usize,
    /// Its rows' text and wrap marks when it was made or moved.
    pub rows: Vec<(StableRowIndex, (String, bool))>,
}

/// The selected cells: from `start` to `end`, inclusive; a block takes the
/// same columns on every row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Range {
    pub start: Point,
    pub end: Point,
    pub block: Option<(usize, usize)>,
}

impl Range {
    /// Whether the cell at `col` (`width` columns) on `row` is selected.
    pub fn contains(&self, row: StableRowIndex, col: usize, width: usize) -> bool {
        if row < self.start.0 || row > self.end.0 {
            return false;
        }
        let last = col + width - 1;
        if let Some((a, b)) = self.block {
            return last >= a && col <= b;
        }
        let after_start = row > self.start.0 || last >= self.start.1;
        let before_end = row < self.end.0 || col <= self.end.1;
        after_start && before_end
    }
}

/// Whether `(row, col)` lies between `a` and `b` (inclusive, reading order).
pub(crate) fn in_range(a: Point, b: Point, row: StableRowIndex, col: usize) -> bool {
    (row, col) >= a && (row, col) <= b
}

/// Characters that end a word for a double tap (alacritty's and the fork's
/// set): spaces and the punctuation that surrounds paths and URLs, and the
/// punctuation of CJK text.
fn is_word(s: &str) -> bool {
    !s.chars()
        .all(|c| c.is_whitespace() || ",│`|:\"' ()[]{}<>\t".contains(c) || is_cjk_punctuation(c))
}

/// The punctuation of CJK prose: the full-width forms (，：；！？（）), the
/// CJK brackets, quotes and marks (、。「」《》【】), and the dashes, quotes
/// and ellipses of general punctuation (—— “” ……). A word ran through
/// them (B696): a path tapped in an agent's Chinese came with the comma
/// and the sentence after it. Full-width letters and digits, `＿` and the
/// middle dot of names (·) stay in a word.
fn is_cjk_punctuation(c: char) -> bool {
    matches!(c,
        '\u{2010}'..='\u{2027}'
        | '\u{2030}'..='\u{205E}'
        | '\u{3000}'..='\u{3004}'
        | '\u{3008}'..='\u{3020}'
        | '\u{3030}'
        | '\u{303D}'
        | '\u{FE10}'..='\u{FE19}'
        | '\u{FE30}'..='\u{FE4F}'
        | '\u{FF01}'..='\u{FF0F}'
        | '\u{FF1A}'..='\u{FF20}'
        | '\u{FF3B}'..='\u{FF3E}'
        | '\u{FF40}'
        | '\u{FF5B}'..='\u{FF65}')
}

impl Selection {
    pub fn new(kind: SelectKind, at: Point, right_half: bool, seqno: usize) -> Self {
        Self {
            kind,
            anchor: (at, right_half),
            head: (at, right_half),
            span: None,
            seqno,
            rows: Vec::new(),
        }
    }

    /// Row `row`'s text and wrap mark as the selection last saw them (the
    /// rows are noted in order).
    pub fn read(&self, row: StableRowIndex) -> Option<&(String, bool)> {
        self.rows
            .binary_search_by_key(&row, |(r, _)| *r)
            .ok()
            .map(|k| &self.rows[k].1)
    }

    pub fn extend(&mut self, to: Point, right_half: bool) {
        self.head = (to, right_half);
    }

    /// Its ends, in order: where it began, where it reaches, and the two of
    /// the span a long press kept.
    pub fn ends(&self) -> Vec<Point> {
        let mut ends = vec![self.anchor.0, self.head.0];
        if let Some((a, b)) = self.span {
            ends.extend([a.0, b.0]);
        }
        ends
    }

    /// The same selection with its ends at `ends` (in [`Selection::ends`]'
    /// order), each keeping the half of its cell it was in: where a reflow
    /// put its text (B572).
    pub fn moved(&self, ends: &[Point]) -> Selection {
        let mut moved = self.clone();
        if let [anchor, head, rest @ ..] = ends {
            moved.anchor.0 = *anchor;
            moved.head.0 = *head;
            if let (Some(span), [a, b]) = (moved.span.as_mut(), rest) {
                span.0.0 = *a;
                span.1.0 = *b;
            }
        }
        moved
    }

    /// Keeps what is selected now whole from here on: a finger resting on
    /// it (a long press reports its small moves) leaves it as it is, and one
    /// that slides past either end grows the selection from the far end.
    pub fn keep_whole(&mut self) {
        let (a, b) = if self.head.0 < self.anchor.0 {
            (self.head, self.anchor)
        } else {
            (self.anchor, self.head)
        };
        self.span = Some((a, b));
    }

    /// The cells selected now. `cells(row)` reads a row (`None`: not kept).
    pub fn range(&self, cells: impl Fn(StableRowIndex) -> Option<RowCells>) -> Option<Range> {
        let (mut a, mut b) = match self.span {
            // The span, and the finger past either end of it.
            Some((start, end)) => (
                if self.head.0 < start.0 {
                    self.head
                } else {
                    start
                },
                if self.head.0 > end.0 { self.head } else { end },
            ),
            None => (self.anchor, self.head),
        };
        if b.0 < a.0 {
            std::mem::swap(&mut a, &mut b);
        }
        match self.kind {
            SelectKind::Cells => {
                // Cell edges: a touch in a cell's right half starts after it
                // and ends on it; one in the left half ends before it.
                let mut start = a.0;
                if a.1 {
                    start.1 += 1;
                }
                let mut end = b.0;
                if !b.1 {
                    if end.1 > 0 {
                        end.1 -= 1;
                    } else if end.0 > start.0 {
                        end = (end.0 - 1, usize::MAX >> 1);
                    } else {
                        return None;
                    }
                }
                (start <= end).then_some(Range {
                    start,
                    end,
                    block: None,
                })
            }
            SelectKind::Block => {
                let (c0, c1) = (a.0.1.min(b.0.1), a.0.1.max(b.0.1));
                Some(Range {
                    start: (a.0.0, c0),
                    end: (b.0.0, c1),
                    block: Some((c0, c1)),
                })
            }
            // A semantic selection is made as cells or words.
            SelectKind::Word | SelectKind::Semantic => {
                let start = word_edge(&cells, a.0, false);
                let end = word_edge(&cells, b.0, true);
                Some(Range {
                    start,
                    end,
                    block: None,
                })
            }
            SelectKind::Line => {
                let mut first = a.0.0;
                while cells(first - 1).is_some_and(|(_, wrapped)| wrapped) {
                    first -= 1;
                }
                let mut last = b.0.0;
                while cells(last).is_some_and(|(_, wrapped)| wrapped) {
                    last += 1;
                }
                Some(Range {
                    start: (first, 0),
                    end: (last, usize::MAX >> 1),
                    block: None,
                })
            }
        }
    }
}

/// The first (or last) cell of the word at `p`, across wrapped rows.
fn word_edge(
    cells: &impl Fn(StableRowIndex) -> Option<RowCells>,
    p: Point,
    forward: bool,
) -> Point {
    let Some((row_cells, _)) = cells(p.0) else {
        return p;
    };
    let Some(at) = row_cells
        .iter()
        .position(|(i, w, _)| (*i..*i + *w).contains(&p.1))
    else {
        return p;
    };
    if !is_word(&row_cells[at].2) {
        return (p.0, row_cells[at].0);
    }
    let (mut row, mut cells_now, mut k) = (p.0, row_cells, at);
    loop {
        if forward {
            if k + 1 < cells_now.len() {
                if !is_word(&cells_now[k + 1].2) {
                    break;
                }
                k += 1;
            } else {
                let wrapped = cells(row).is_some_and(|(_, w)| w);
                match cells(row + 1) {
                    Some((next, _)) if wrapped && next.first().is_some_and(|c| is_word(&c.2)) => {
                        row += 1;
                        cells_now = next;
                        k = 0;
                    }
                    _ => break,
                }
            }
        } else if k > 0 {
            if !is_word(&cells_now[k - 1].2) {
                break;
            }
            k -= 1;
        } else {
            match cells(row - 1) {
                Some((prev, true)) if prev.last().is_some_and(|c| is_word(&c.2)) => {
                    row -= 1;
                    k = prev.len() - 1;
                    cells_now = prev;
                }
                _ => break,
            }
        }
    }
    let (i, w, _) = &cells_now[k];
    (row, if forward { i + w - 1 } else { *i })
}

/// A logical line (rows joined where they wrap) for searching: its text,
/// and where each of its characters sits.
pub(crate) struct LogicalLine {
    pub text: String,
    /// (byte offset in `text`, row, column, width) of each cell's text;
    /// none from [`logical_texts`].
    starts: Vec<(usize, StableRowIndex, usize, usize)>,
    /// The first and last rows it covers.
    pub rows: (StableRowIndex, StableRowIndex),
}

impl LogicalLine {
    /// The cell holding the byte at `offset`.
    pub fn at(&self, offset: usize) -> Option<Point> {
        let k = self.starts.partition_point(|(o, _, _, _)| *o <= offset);
        let (_, row, col, _) = *self.starts.get(k.checked_sub(1)?)?;
        Some((row, col))
    }

    /// Each cell's text, in order (a wide character once).
    pub fn cells(&self) -> Vec<&str> {
        (0..self.starts.len())
            .map(|k| {
                let end = self.starts.get(k + 1).map_or(self.text.len(), |s| s.0);
                &self.text[self.starts[k].0..end]
            })
            .collect()
    }

    /// Cell `k`: its row, first column and last column (a wide character
    /// covers two).
    pub fn cell(&self, k: usize) -> Option<(StableRowIndex, usize, usize)> {
        let &(_, row, col, width) = self.starts.get(k)?;
        Some((row, col, col + width.max(1) - 1))
    }

    /// The byte offset of cell `k`'s text.
    pub fn offset(&self, k: usize) -> Option<usize> {
        self.starts.get(k).map(|s| s.0)
    }

    /// The last row the line covers.
    pub fn last_row(&self) -> Option<StableRowIndex> {
        self.starts.last().map(|s| s.1)
    }
}

/// The logical lines of rows `first..=last`.
pub(crate) fn logical_lines(
    first: StableRowIndex,
    last: StableRowIndex,
    cells: impl Fn(StableRowIndex) -> Option<RowCells>,
) -> Vec<LogicalLine> {
    lines_of(first, last, cells, true)
}

/// The logical lines' text only, without where each character sits: what
/// a search looks through before it places its matches (B734: the places
/// of 10 000 rows' cells cost a find step 20 MB, for the few lines that
/// match).
pub(crate) fn logical_texts(
    first: StableRowIndex,
    last: StableRowIndex,
    cells: impl Fn(StableRowIndex) -> Option<RowCells>,
) -> Vec<LogicalLine> {
    lines_of(first, last, cells, false)
}

fn lines_of(
    first: StableRowIndex,
    last: StableRowIndex,
    cells: impl Fn(StableRowIndex) -> Option<RowCells>,
    places: bool,
) -> Vec<LogicalLine> {
    let new_line = |row| LogicalLine {
        text: String::new(),
        starts: Vec::new(),
        rows: (row, row),
    };
    let mut out = Vec::new();
    let mut line = new_line(first);
    let mut fresh = true;
    for row in first..=last {
        let Some((row_cells, wrapped)) = cells(row) else {
            continue;
        };
        if fresh {
            line.rows.0 = row;
            fresh = false;
        }
        line.rows.1 = row;
        let mut next = 0;
        for (i, w, s) in row_cells {
            while next < i {
                if places {
                    line.starts.push((line.text.len(), row, next, 1));
                }
                line.text.push(' ');
                next += 1;
            }
            if places {
                line.starts.push((line.text.len(), row, i, w));
            }
            line.text.push_str(&s);
            next = i + w;
        }
        if !wrapped {
            out.push(std::mem::replace(&mut line, new_line(row + 1)));
            fresh = true;
        }
    }
    if !line.text.is_empty() {
        out.push(line);
    }
    out
}
