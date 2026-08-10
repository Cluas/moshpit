#!/usr/bin/env python3
"""Convert the hand-written docs pages into Starlight Markdown.

One-shot migration tool, kept in the repo because the conversion is not
lossless-by-luck: the old pages carry structures Markdown has no syntax for
(the ASCII `div.wire` diagrams, the comparison tables' per-cell classes) and
this file records exactly what happens to each of them. If a page comes out
wrong, fix it here and re-run rather than hand-patching the output — there are
32 pages and hand-patching does not survive a second pass.

    scripts/html-to-markdown.py            # write into marketing/site-next
    scripts/html-to-markdown.py --check    # report only, write nothing

What it does with the shapes that actually appear (surveyed, not guessed):
  h2 + p.sub        → frontmatter title + description
  p.eyebrow         → dropped; Starlight's sidebar section is the same signal
  h3/h4             → ##/###
  p.txt             → paragraph
  ul.list / ol      → list
  b / i / code / a  → inline markdown
  pre               → fenced block, language guessed from the content
  details+summary   → Starlight <details> passthrough (valid in MDX-less .md)
  table             → GitHub table; cell classes become ✓/✗ text so the
                      comparison pages keep their meaning without CSS
  div.wire          → Starlight :::note when it holds prose (all 14 in the
                      docs do), fenced block when it holds a drawing
  comments          → dropped; the pages use them as section rules
  img               → Markdown image, absolute /path kept (public/ serves it)
  div.dpager        → dropped; Starlight generates prev/next itself
"""
from __future__ import annotations

import argparse
import pathlib
import re
import sys

try:
    from bs4 import BeautifulSoup, Comment, NavigableString, Tag
except ModuleNotFoundError:  # pragma: no cover — an environment problem, not a bug
    sys.exit(
        "beautifulsoup4 is missing from this interpreter.\n"
        f"  running: {sys.executable}\n"
        "  on this machine it lives in python3.11:\n"
        "    /opt/homebrew/bin/python3.11 scripts/html-to-markdown.py"
    )

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "marketing" / "site"
DST = ROOT / "marketing" / "site-next" / "src" / "content" / "docs"

# Old URL → new content slug. The docs-*.html basenames map 1:1 onto the
# sidebar entries in astro.config.mjs; keeping the same leaf means /docs/mosh
# still resolves after the switch.
SKIP_CLASSES = {"dpager", "eyebrow", "dnav", "dsearch"}


def inline(node) -> str:
    """Render inline content, preserving emphasis, code and links."""
    if isinstance(node, Comment):
        return ""
    if isinstance(node, NavigableString):
        text = str(node)
        # Collapse the source's hand-wrapped lines; Markdown re-wraps anyway.
        return re.sub(r"\s+", " ", text)
    if not isinstance(node, Tag):
        return ""

    kids = "".join(inline(c) for c in node.children)
    name = node.name
    if name in {"b", "strong"}:
        return emphasis(kids.strip(), "**", "b")
    if name in {"i", "em"}:
        return emphasis(kids.strip(), "*", "i")
    if name == "code":
        body = kids.strip()
        # A backtick inside would break the span; the old pages never do it,
        # but assert rather than emit broken Markdown if that changes.
        assert "`" not in body, f"backtick inside <code>: {body!r}"
        return f"`{body}`"
    if name == "a":
        href = node.get("href", "")
        return f"[{kids.strip()}]({href})" if href else kids
    if name == "br":
        return "  \n"
    if name == "img":
        return image(node)
    if name == "kbd":
        return f"`{kids.strip()}`"
    return kids


def emphasis(text: str, marker: str, tag: str) -> str:
    """Bold or italic, in whichever syntax actually renders.

    CommonMark only lets a `**` run close when it is "right-flanking", and a run
    preceded by punctuation and followed by a letter is not. Chinese prose hits
    that constantly, because a bold clause ends with 。or ：and the next
    character is a hanzi with no space between them:

        **它是 tmux 的另一种选择，不是替代品。**两者是独立的服务器

    That renders with four visible asterisks and no bold — 42 times across these
    pages, which is how it was found. The same trap exists in English
    (`**Note.**Something`), just harder to hit because English puts a space
    there.

    The condition is decidable from the content alone: if the text ends with an
    alphanumeric, the closing run is preceded by a letter and closes whatever
    follows it. Otherwise fall back to inline HTML, which Markdown allows and
    which has no flanking rules to fail.
    """
    if not text:
        return ""
    if text[0].isalnum() and text[-1].isalnum():
        return f"{marker}{text}{marker}"
    return f"<{tag}>{text}</{tag}>"


def plain(node) -> str:
    """Inline content as text, with no Markdown syntax.

    Frontmatter `description` ends up in three places that all render it
    literally: <meta name="description">, the og: tags, and Starlight's
    LinkCard. A `[label](/href)` written there reaches the reader as brackets
    and a URL, so descriptions keep the words and drop the markup.
    """
    if isinstance(node, Comment):
        return ""
    if isinstance(node, NavigableString):
        return re.sub(r"\s+", " ", str(node))
    if not isinstance(node, Tag):
        return ""
    if node.name == "br":
        return " "
    if node.name == "img":
        return ""
    return "".join(plain(c) for c in node.children)


def image(node: Tag) -> str:
    src = node.get("src", "")
    alt = (node.get("alt") or "").replace("]", "")
    return f"![{alt}]({src})"


def fence(text: str) -> str:
    """Wrap pre-formatted text, guessing a language for syntax highlighting."""
    body = text.strip("\n")
    lang = ""
    if re.search(r"^\s*[$#>]\s|\b(ssh|tmux|brew|apt|git|herdr|mosh)\b", body[:200]):
        lang = "sh"
    return f"```{lang}\n{body}\n```"


def table(node: Tag) -> str:
    rows: list[list[str]] = []
    for tr in node.find_all("tr"):
        cells = tr.find_all(["th", "td"])
        out = []
        for c in cells:
            text = "".join(inline(k) for k in c.children).strip()
            classes = c.get("class") or []
            # The comparison pages encode the answer in a class, not the text:
            # td.us / td.yes is "we do", td.no is "we do not". Without this the
            # converted table reads as a column of blanks.
            if not text:
                if "no" in classes:
                    text = "—"
                elif {"us", "yes"} & set(classes):
                    text = "✓"
            out.append(text.replace("|", "\\|"))
        if out:
            rows.append(out)
    if not rows:
        return ""
    width = max(len(r) for r in rows)
    rows = [r + [""] * (width - len(r)) for r in rows]
    head, *body = rows
    sep = ["---"] * width
    lines = ["| " + " | ".join(head) + " |", "| " + " | ".join(sep) + " |"]
    lines += ["| " + " | ".join(r) + " |" for r in body]
    return "\n".join(lines)


def cell(node: Tag, depth: int) -> list[str]:
    """One card from the old design's grids: a badge, a title, an explanation.

        <div class="cell">
          <span class="ic">Global</span>
          <h4>history-limit</h4>
          <p>tmux set -g history-limit 50000 runs before every attach…</p>

    The badge is not decoration — `Global`, `Per window`, `Transient`,
    `mosh only` are the scope of the thing being described, and the first pass
    at this conversion emitted them as bare one-word paragraphs floating above
    the heading, which is how they were noticed.

    They fold into the explanation as a bold lead-in rather than into the
    heading: a heading of "history-limit — Global" would put the badge in the
    anchor (#history-limit--global) and in the table of contents, where it is
    noise. As a lead-in it reads the way it was drawn in both languages —
    "**Global.** tmux set -g …" and "**尺寸。**直连客户端的 resize …".
    """
    badge = node.find("span", class_="ic")
    label = plain(badge).strip() if badge else ""
    if badge:
        badge.extract()

    chunks: list[str] = []
    for child in node.children:
        chunks += block(child, depth)
    if not label:
        return chunks

    # CJK gets its own full stop; a Latin period after a hanzi looks like a typo.
    stop = "。" if any("一" <= c <= "鿿" for c in label) else "."
    lead = emphasis(f"{label}{stop}", "**", "b")
    for i, chunk in enumerate(chunks):
        if not chunk.startswith("#"):
            chunks[i] = f"{lead}{'' if stop == '。' else ' '}{chunk}"
            return chunks
    # A card with a title and no prose: keep the badge rather than drop it.
    return chunks + [lead]


def block(node, depth: int = 0) -> list[str]:
    """Render a block-level node into Markdown chunks."""
    if isinstance(node, Comment):
        # Comment subclasses NavigableString, so without this the check below
        # treats a comment as text. The old pages use them as section rules —
        # `<!-- ══ 键盘 ══ -->` — and 30 of those were being published as
        # paragraphs of ASCII in the middle of the prose.
        return []
    if isinstance(node, NavigableString):
        text = re.sub(r"\s+", " ", str(node)).strip()
        return [text] if text else []
    if not isinstance(node, Tag):
        return []

    classes = set(node.get("class") or [])
    if classes & SKIP_CLASSES:
        return []

    name = node.name
    if name in {"script", "style", "header", "footer", "aside", "nav"}:
        return []
    if name == "h2":
        return []  # consumed as the page title
    if name in {"h3", "h4", "h5"}:
        level = {"h3": "##", "h4": "###", "h5": "####"}[name]
        return [f"{level} {''.join(inline(c) for c in node.children).strip()}"]
    if name == "p":
        text = "".join(inline(c) for c in node.children).strip()
        return [text] if text else []
    if name in {"ul", "ol"}:
        out = []
        for i, li in enumerate(node.find_all("li", recursive=False), 1):
            bullet = f"{i}." if name == "ol" else "-"
            text = "".join(inline(c) for c in li.children).strip()
            out.append(f"{bullet} {text}")
        return ["\n".join(out)] if out else []
    if name == "pre":
        return [fence(node.get_text())]
    if name == "table":
        t = table(node)
        return [t] if t else []
    if name == "details":
        summary = node.find("summary")
        label = "".join(inline(c) for c in summary.children).strip() if summary else "Details"
        if summary:
            summary.extract()
        inner = []
        for c in node.children:
            inner += block(c, depth + 1)
        body = "\n\n".join(inner)
        return [f"<details>\n<summary>{label}</summary>\n\n{body}\n\n</details>"]
    if name == "img":
        return [image(node)]
    if "cell" in classes:
        return cell(node, depth)
    if "wire" in classes:
        # `wire` means two different things in this design. On the marketing
        # pages it is ASCII art, and fencing it is the only way to keep the
        # drawing. In the docs it is a "here is where this stops" callout built
        # from <p> and <ul> — surveyed across all 32 pages, every single one of
        # the 14 is prose and not one is a drawing. Fencing those rendered
        # paragraphs of Chinese as if they were code, complete with the source's
        # hand-wrapped indentation.
        if node.find(["p", "ul", "ol", "li"]):
            inner: list[str] = []
            for child in node.children:
                inner += block(child, depth + 1)
            body = "\n\n".join(c for c in inner if c.strip())
            return [f":::note\n{body}\n:::"]
        return [fence(node.get_text())]

    # Any other container: recurse.
    out = []
    for c in node.children:
        out += block(c, depth)
    return out


def convert(path: pathlib.Path) -> tuple[str, str, str]:
    raw = path.read_text()
    raw = re.sub(r"<!--DOCSHELL:BEGIN-->.*?<!--DOCSHELL:END-->", "", raw, flags=re.S)
    raw = re.sub(r"<!--DOCTOC:BEGIN-->.*?<!--DOCTOC:END-->", "", raw, flags=re.S)
    raw = re.sub(r"<!--DOCNEXT:BEGIN-->.*?<!--DOCNEXT:END-->", "", raw, flags=re.S)
    soup = BeautifulSoup(raw, "html.parser")

    body = soup.select_one(".dbody") or soup.select_one("body")
    h2 = body.find("h2")
    title = "".join(inline(c) for c in h2.children).strip() if h2 else path.stem
    sub = body.select_one("p.sub")
    desc = plain(sub).strip() if sub else ""
    if sub:
        sub.extract()

    chunks: list[str] = []
    for node in body.children:
        chunks += block(node)
    md = "\n\n".join(c for c in chunks if c.strip())
    md = re.sub(r"\n{3,}", "\n\n", md).strip()
    return title, desc, md


def yaml_quote(s: str) -> str:
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()

    pages = sorted(SRC.glob("docs-*.html"))
    written = 0
    for page in pages:
        stem = page.name.removesuffix(".html")
        zh = stem.endswith(".zh")
        slug = stem.removesuffix(".zh").removeprefix("docs-")
        if slug == "docs":            # the hub page — Starlight generates it
            continue
        title, desc, md = convert(page)
        out_dir = DST / ("zh/docs" if zh else "docs")
        out = out_dir / f"{slug}.md"
        front = ["---", f"title: {yaml_quote(title)}"]
        if desc:
            front.append(f"description: {yaml_quote(desc)}")
        front.append("---")
        text = "\n".join(front) + "\n\n" + md + "\n"
        status = "would write" if args.check else "wrote"
        if not args.check:
            out_dir.mkdir(parents=True, exist_ok=True)
            out.write_text(text)
        written += 1
        print(f"  {status} {out.relative_to(ROOT)}  ({len(md.splitlines())} lines)")
    print(f"\n{written} pages")


if __name__ == "__main__":
    sys.exit(main())
