//! Edits to a person's JSON file that leave the rest of it as they wrote
//! it (B375).
//!
//! A hook install or removal used to write the whole agent config back as
//! Foundation formats it: every key sorted, every line re-indented, the
//! person's own layout gone for one hook entry's sake. [`splice`] takes the
//! file as it is and the value it should now hold, and rewrites only what
//! differs: a member or an array item equal to what was there keeps its
//! exact bytes, a container that changed is rebuilt from its kept pieces
//! with the separators, indentation and `:` spacing the file already uses,
//! and what is new is written in that style. The result is parsed again
//! and must equal the value asked for; if it doesn't, or the file is
//! anything the strict reader here doesn't take (a fragment, a trailing
//! comma), the caller writes the value whole as before.

use moshpit_seal::foundation::json::{self, Dialect, Style, Value};

/// `existing` rewritten to hold `target`, as little changed as can be;
/// `None` when that can't be done (the caller then writes `target` whole).
pub fn splice(existing: &str, target: &Value) -> Option<String> {
    let bytes = existing.as_bytes();
    let mut parser = Spans { bytes, at: 0 };
    parser.blank();
    let root = parser.node()?;
    parser.blank();
    if parser.at != bytes.len() {
        return None;
    }
    let style = Layout::of(existing, &root);
    let text = reconcile(existing, &root, target, &style, "")?;
    let out = format!("{}{text}{}", &existing[..root.start], &existing[root.end..]);
    let parsed = json::parse(
        out.trim().as_bytes(),
        Dialect::Serialization {
            fragments_allowed: false,
        },
    )
    .ok()?;
    (canonical(&parsed) == canonical(target)).then_some(out)
}

fn canonical(value: &Value) -> String {
    json::write_serialization(
        value,
        Style {
            pretty: false,
            sorted_keys: true,
            escape_slashes: false,
        },
    )
}

/// A value's place in the text.
#[derive(Debug)]
struct Node {
    start: usize,
    end: usize,
    kind: Kind,
}

#[derive(Debug)]
enum Kind {
    Scalar,
    /// Each member: its key (decoded), where the member starts (its key's
    /// opening quote), and its value.
    Object(Vec<(String, usize, Node)>),
    Array(Vec<Node>),
}

/// The file's habits: the indent of one level, what sits between a key and
/// its value, and between two items on one line.
struct Layout {
    unit: String,
    colon: String,
    inline: String,
}

impl Layout {
    fn of(text: &str, root: &Node) -> Self {
        let mut unit = "  ".to_owned();
        let mut colon = ": ".to_owned();
        let mut inline = None;
        if let Kind::Object(members) = &root.kind
            && let Some((_, key_start, value)) = members.first()
        {
            let lead = &text[root.start + 1..*key_start];
            if let Some(line) = lead.rsplit('\n').next()
                && lead.contains('\n')
                && !line.is_empty()
            {
                unit = line.to_owned();
            }
            let key_end = key_end(text, *key_start);
            colon = text[key_end..value.start].to_owned();
            if let Some((_, next, _)) = members.get(1)
                && !lead.contains('\n')
            {
                inline = Some(text[value.end..*next].to_owned());
            }
        }
        // A file written tight (`"a":1`) is written tight on one line too.
        let inline =
            inline.unwrap_or_else(|| if colon.ends_with(' ') { ", " } else { "," }.to_owned());
        Layout {
            unit,
            colon,
            inline,
        }
    }
}

/// Where the key string starting at `start` ends (after its closing quote).
fn key_end(text: &str, start: usize) -> usize {
    let bytes = text.as_bytes();
    let mut at = start + 1;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => at += 2,
            b'"' => return at + 1,
            _ => at += 1,
        }
    }
    at
}

/// The text of `node` made to hold `target`; `indent`: the indent of the
/// line `node` starts on.
fn reconcile(
    text: &str,
    node: &Node,
    target: &Value,
    style: &Layout,
    indent: &str,
) -> Option<String> {
    let slice = &text[node.start..node.end];
    // Equal as JSON (an object's members in any order): kept as written.
    // Our own hooks refreshed are equal to what a refresh wrote before, so
    // a refresh changes nothing in the file.
    if value_of(slice).is_some_and(|v| canonical(&v) == canonical(target)) {
        return Some(slice.to_owned());
    }
    match (&node.kind, target) {
        (Kind::Object(members), Value::Object(wanted)) => {
            let shape = Shape::of(
                text,
                node,
                members.iter().map(|(_, s, n)| (*s, n.end)),
                indent,
                style,
            );
            let mut pieces = Vec::new();
            let mut seen: Vec<&str> = Vec::new();
            for (key, key_start, value) in members {
                if seen.contains(&key.as_str()) {
                    // A key written twice: the reader keeps the first, so
                    // the rest stay as they were.
                    pieces.push(text[*key_start..value.end].to_owned());
                    continue;
                }
                seen.push(key);
                let Some(want) = wanted.get(key) else {
                    continue;
                };
                let head = &text[*key_start..value.start];
                pieces.push(format!(
                    "{head}{}",
                    reconcile(text, value, want, style, &shape.inner)?
                ));
            }
            for (key, want) in wanted.iter() {
                if !seen.contains(&key) {
                    pieces.push(format!(
                        "{}{}{}",
                        quoted(key),
                        style.colon,
                        render(want, &shape.inner, style, shape.multiline)
                    ));
                }
            }
            Some(shape.build('{', '}', &pieces))
        }
        (Kind::Array(items), Value::Array(wanted)) => {
            let shape = Shape::of(
                text,
                node,
                items.iter().map(|n| (n.start, n.end)),
                indent,
                style,
            );
            let values: Vec<Option<String>> = items
                .iter()
                .map(|n| value_of(&text[n.start..n.end]).map(|v| canonical(&v)))
                .collect();
            let mut pieces = Vec::new();
            let mut next = 0;
            for want in wanted {
                let want_text = canonical(want);
                match (next..items.len())
                    .find(|&k| values[k].as_deref() == Some(want_text.as_str()))
                {
                    Some(k) => {
                        pieces.push(text[items[k].start..items[k].end].to_owned());
                        next = k + 1;
                    }
                    None => pieces.push(render(want, &shape.inner, style, shape.multiline)),
                }
            }
            Some(shape.build('[', ']', &pieces))
        }
        _ => Some(render(target, indent, style, true)),
    }
}

fn value_of(slice: &str) -> Option<Value> {
    json::parse(
        slice.as_bytes(),
        Dialect::Serialization {
            fragments_allowed: true,
        },
    )
    .ok()
}

fn quoted(key: &str) -> String {
    json::write_serialization(
        &Value::String(key.to_owned()),
        Style {
            pretty: false,
            sorted_keys: false,
            escape_slashes: false,
        },
    )
}

/// How a container lays out its pieces, as it does now.
struct Shape {
    /// After the opening bracket, before the first piece.
    lead: String,
    /// Between two pieces (the comma included).
    sep: String,
    /// After the last piece, before the closing bracket.
    trail: String,
    /// The indent of a piece's line.
    inner: String,
    multiline: bool,
}

impl Shape {
    fn of(
        text: &str,
        node: &Node,
        mut pieces: impl Iterator<Item = (usize, usize)>,
        indent: &str,
        style: &Layout,
    ) -> Self {
        let first = pieces.next();
        let second = pieces.next();
        let last = pieces.last().or(second).or(first);
        match (first, second) {
            (Some((start, end)), next) => {
                let lead = text[node.start + 1..start].to_owned();
                let trail = text[last.expect("a piece").1..node.end - 1].to_owned();
                let multiline = lead.contains('\n');
                let inner = if multiline {
                    lead.rsplit('\n').next().unwrap_or("").to_owned()
                } else {
                    String::new()
                };
                let sep = match next {
                    Some((after, _)) => text[end..after].to_owned(),
                    None if multiline => format!(",\n{inner}"),
                    None if lead.is_empty() => ",".to_owned(),
                    None => ", ".to_owned(),
                };
                Shape {
                    lead,
                    sep,
                    trail,
                    inner,
                    multiline,
                }
            }
            (None, _) => {
                // An empty container takes after the level it sits in.
                let inner = format!("{indent}{}", style.unit);
                Shape {
                    lead: format!("\n{inner}"),
                    sep: format!(",\n{inner}"),
                    trail: format!("\n{indent}"),
                    inner,
                    multiline: true,
                }
            }
        }
    }

    fn build(&self, open: char, close: char, pieces: &[String]) -> String {
        if pieces.is_empty() {
            return format!("{open}{close}");
        }
        format!(
            "{open}{}{}{}{close}",
            self.lead,
            pieces.join(&self.sep),
            self.trail
        )
    }
}

/// `value` written in the file's style; `indent`: its line's indent.
fn render(value: &Value, indent: &str, style: &Layout, multiline: bool) -> String {
    let inner = format!("{indent}{}", style.unit);
    let (lead, sep, trail) = if multiline {
        (
            format!("\n{inner}"),
            format!(",\n{inner}"),
            format!("\n{indent}"),
        )
    } else {
        (String::new(), style.inline.clone(), String::new())
    };
    match value {
        Value::Array(items) if !items.is_empty() => {
            let parts: Vec<String> = items
                .iter()
                .map(|v| render(v, &inner, style, multiline))
                .collect();
            format!("[{lead}{}{trail}]", parts.join(&sep))
        }
        Value::Object(object) if !object.is_empty() => {
            let parts: Vec<String> = object
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{}{}{}",
                        quoted(k),
                        style.colon,
                        render(v, &inner, style, multiline)
                    )
                })
                .collect();
            format!("{{{lead}{}{trail}}}", parts.join(&sep))
        }
        _ => json::write_serialization(
            value,
            Style {
                pretty: false,
                sorted_keys: false,
                escape_slashes: false,
            },
        ),
    }
}

/// A strict JSON reader that keeps where each value sits.
struct Spans<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Spans<'_> {
    fn blank(&mut self) {
        while self.at < self.bytes.len()
            && matches!(self.bytes[self.at], b' ' | b'\t' | b'\n' | b'\r')
        {
            self.at += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> Option<()> {
        (self.bytes.get(self.at) == Some(&byte)).then(|| self.at += 1)
    }

    fn node(&mut self) -> Option<Node> {
        let start = self.at;
        let kind = match *self.bytes.get(self.at)? {
            b'{' => {
                self.at += 1;
                let mut members = Vec::new();
                self.blank();
                if self.eat(b'}').is_none() {
                    loop {
                        self.blank();
                        let key_start = self.at;
                        let key = self.string()?;
                        self.blank();
                        self.eat(b':')?;
                        self.blank();
                        let value = self.node()?;
                        members.push((key, key_start, value));
                        self.blank();
                        if self.eat(b',').is_some() {
                            continue;
                        }
                        self.eat(b'}')?;
                        break;
                    }
                }
                Kind::Object(members)
            }
            b'[' => {
                self.at += 1;
                let mut items = Vec::new();
                self.blank();
                if self.eat(b']').is_none() {
                    loop {
                        self.blank();
                        items.push(self.node()?);
                        self.blank();
                        if self.eat(b',').is_some() {
                            continue;
                        }
                        self.eat(b']')?;
                        break;
                    }
                }
                Kind::Array(items)
            }
            b'"' => {
                self.string()?;
                Kind::Scalar
            }
            _ => {
                while self.at < self.bytes.len()
                    && !matches!(
                        self.bytes[self.at],
                        b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r'
                    )
                {
                    self.at += 1;
                }
                if self.at == start {
                    return None;
                }
                Kind::Scalar
            }
        };
        Some(Node {
            start,
            end: self.at,
            kind,
        })
    }

    /// A string at the cursor, decoded by the real reader.
    fn string(&mut self) -> Option<String> {
        let start = self.at;
        self.eat(b'"')?;
        while self.at < self.bytes.len() {
            match self.bytes[self.at] {
                b'\\' => self.at += 2,
                b'"' => {
                    self.at += 1;
                    let raw = std::str::from_utf8(&self.bytes[start..self.at]).ok()?;
                    return match value_of(raw)? {
                        Value::String(s) => Some(s),
                        _ => None,
                    };
                }
                _ => self.at += 1,
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(text: &str) -> Value {
        json::parse(
            text.as_bytes(),
            Dialect::Serialization {
                fragments_allowed: false,
            },
        )
        .unwrap()
    }

    #[test]
    fn what_is_unchanged_keeps_its_bytes() {
        let text = "{\n    \"z\":   1,\n    \"a\" :[1,2]\n}\n";
        assert_eq!(splice(text, &value(text)).as_deref(), Some(text));
    }

    #[test]
    fn a_new_member_takes_the_files_layout() {
        let text = "{\n    \"z\": 1,\n    \"a\": [1, 2]\n}\n";
        let want = value("{\"z\":1,\"a\":[1,2],\"hooks\":{\"Stop\":[{\"x\":true}]}}");
        assert_eq!(
            splice(text, &want).unwrap(),
            "{\n    \"z\": 1,\n    \"a\": [1, 2],\n    \"hooks\": {\n        \"Stop\": [\n            {\n                \"x\": true\n            }\n        ]\n    }\n}\n"
        );
    }

    #[test]
    fn an_item_taken_out_leaves_its_neighbours_as_they_were() {
        let text = "{\"hooks\": {\"Stop\": [ {\"mine\":1},  {\"ours\":2} ], \"Other\": [3]}}";
        let want = value("{\"hooks\":{\"Stop\":[{\"mine\":1}],\"Other\":[3]}}");
        assert_eq!(
            splice(text, &want).unwrap(),
            "{\"hooks\": {\"Stop\": [ {\"mine\":1} ], \"Other\": [3]}}"
        );
    }

    #[test]
    fn a_member_taken_out_takes_its_comma_with_it() {
        let text = "{\n  \"a\": 1,\n  \"hooks\": {},\n  \"b\": 2\n}";
        let want = value("{\"a\":1,\"b\":2}");
        assert_eq!(
            splice(text, &want).unwrap(),
            "{\n  \"a\": 1,\n  \"b\": 2\n}"
        );
    }

    #[test]
    fn what_the_strict_reader_wont_take_is_left_to_the_caller() {
        assert_eq!(splice("{\"a\": 1,}", &value("{\"a\":1}")), None);
        assert_eq!(
            splice("[1]", &value("{\"a\":1}")),
            Some("{\n  \"a\": 1\n}".to_owned())
        );
    }
}
