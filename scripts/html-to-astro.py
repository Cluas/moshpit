#!/usr/bin/env python3
"""Wrap the marketing pages in the shared Astro layout.

The docs became Markdown because they are prose. These pages are not: the hero,
the phone galleries and the comparison tables are bespoke markup that exists to
look a specific way. Converting them to Markdown would throw the design away,
and re-expressing them as components mid-migration would change the markup and
the plumbing at the same time, leaving no way to tell which one broke a layout.

So this does the one thing that is safe and still removes the duplication: it
lifts each page's unique body out from between the shared header and footer and
emits an .astro file that renders exactly those bytes inside
layouts/Marketing.astro. Visual risk is nil — the markup is unchanged — and the
48 copies of the chrome collapse into one file.

    scripts/html-to-astro.py [--check]
"""
from __future__ import annotations

import argparse
import html
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "marketing" / "site"
DST = ROOT / "marketing" / "site-next" / "src" / "pages"

# source basename → (astro path, locale, url path used for hreflang + nav).
# A trailing False marks a page with no counterpart in the other language: the
# layout then drops the language toggle and the hreflang alternate, because both
# would point at a page nobody wrote.
PAGES = [
    ("index.html", "index.astro", "en", ""),
    ("zh.html", "zh.astro", "zh", ""),
    ("compare.html", "compare.astro", "en", "compare"),
    ("compare.zh.html", "zh/compare.astro", "zh", "compare"),
    ("pricing.html", "pricing.astro", "en", "pricing"),
    ("pricing.zh.html", "zh/pricing.astro", "zh", "pricing"),
    ("guides.html", "guides.astro", "en", "guides"),
    ("guides.zh.html", "zh/guides.astro", "zh", "guides"),
    ("guide-claude-code.html", "guide/claude-code.astro", "en", "guide/claude-code"),
    ("guide-claude-code.zh.html", "zh/guide/claude-code.astro", "zh", "guide/claude-code"),
    ("guide-remote-access.html", "guide/remote-access.astro", "en", "guide/remote-access"),
    ("guide-remote-access.zh.html", "zh/guide/remote-access.astro", "zh", "guide/remote-access"),
    ("privacy.html", "privacy.astro", "en", "privacy", False),
    ("support.html", "support.astro", "en", "support", False),
]


def meta(raw: str, name: str) -> str:
    m = re.search(rf'<meta name="{name}" content="(.*?)">', raw, re.S)
    return html.unescape(m.group(1)) if m else ""


def body_of(raw: str) -> str:
    """Everything between the shared header and the shared footer."""
    start = raw.find("</header>")
    assert start != -1, "no <header> to cut after"
    start = raw.find("</div>", start) + len("</div>")
    end = raw.find("<footer")
    assert end > start, "no <footer> to cut before"
    body = raw[start:end]
    # The gallery script lived inline after the footer on the old pages; the
    # layout owns it now. Page-specific scripts (docs search) stay put.
    body = re.sub(r"<script>\s*//\s*Gallery.*?</script>", "", body, flags=re.S)
    return body.strip("\n")


def as_template_literal(body: str) -> str:
    """Escape the body for a JS template literal.

    The bodies go through `set:html` rather than straight into the template,
    because an .astro template is JSX-like: the `{"type":"worktree_created"}`
    JSON inside the guides' <pre> blocks parses as an expression and the build
    fails. Handing Astro an opaque string means no character in these pages can
    be reinterpreted — which is exactly the guarantee a mechanical migration
    wants, and it costs nothing here (no inner components, global CSS, and the
    images are already static files in public/).
    """
    return body.replace("\\", "\\\\").replace("`", "\\`").replace("${", "\\${")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()

    for name, out_rel, lang, url, *rest in PAGES:
        translated = rest[0] if rest else True
        src = SRC / name
        if not src.exists():
            print(f"  ! missing {name}")
            continue
        raw = src.read_text()
        m = re.search(r"<title>(.*?)</title>", raw, re.S)
        title = html.unescape(m.group(1)).strip() if m else "Moshpit"
        desc = meta(raw, "description")
        body = body_of(raw)

        # A relative path has to climb out of pages/ as well as any locale
        # subdirectory, and getting that arithmetic wrong only shows up as an
        # unresolved import at build time. The tsconfig alias removes the sum.
        import_path = "@layouts/Marketing.astro"

        # ensure_ascii=False is load-bearing. An .astro attribute is HTML-ish,
        # not a JS string literal, so a `—` escape is never decoded — the
        # tab would read "Moshpit — your agents…" and every Chinese page
        # would be nothing but escapes in its <title> and og: tags. Emit the
        # real characters and let the file's UTF-8 carry them.
        def attr(s: str) -> str:
            return json.dumps(s, ensure_ascii=False)
        text = "\n".join(
            [
                "---",
                f"import Marketing from '{import_path}';",
                "",
                "// Raw markup, lifted verbatim from the page this replaces.",
                "const body = `" + as_template_literal(body) + "`;",
                "---",
                "",
                "<Marketing",
                f"  title={attr(title)}",
                f"  description={attr(desc)}",
                f"  path={attr(url)}",
                f'  lang="{lang}"',
                *([] if translated else ["  translated={false}"]),
                ">",
                "  <Fragment set:html={body} />",
                "</Marketing>",
                "",
            ]
        )
        out = DST / out_rel
        if not args.check:
            out.parent.mkdir(parents=True, exist_ok=True)
            out.write_text(text)
        verb = "would write" if args.check else "wrote"
        print(f"  {verb} {out.relative_to(ROOT)}  ({len(body.splitlines())} body lines)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
