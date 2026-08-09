#!/usr/bin/env python3
"""Turn the loose docs pages into a documentation SITE.

The pages themselves are hand-written; what a reader actually judges a docs
site by is the chrome around them — a sidebar that shows the whole map, a
table of contents for the page they are on, prev/next so they can read it
like a book, and search. Injecting that here (rather than pasting it into
twenty files) means the nav can never drift out of sync with itself, and
adding a page is one line in NAV below.

Idempotent: it strips any chrome it previously injected before injecting
again, so run it as often as you like.

    python3 scripts/build-docs.py
"""
from __future__ import annotations

import html
import json
import pathlib
import re

SITE = pathlib.Path(__file__).resolve().parent.parent / "marketing" / "site"

# ── the map ──────────────────────────────────────────────────────────────
# (section title EN, section title zh, [(slug, title EN, title zh), …])
# `slug` is the English filename without .html; the zh page is "<slug>.zh.html".
NAV: list[tuple[str, str, list[tuple[str, str, str]]]] = [
    ("Start", "开始", [
        ("docs-intro", "What Offhook does", "Offhook 是做什么的"),
        ("docs-setup", "Set up a connection", "配置一个连接"),
        ("docs-first-session", "Run your first session", "跑通第一个会话"),
    ]),
    ("Connections", "连接", [
        ("docs-keys", "SSH keys and host trust", "SSH 密钥与主机信任"),
        ("docs-mosh", "Mosh and roaming", "Mosh 与漫游"),
        ("guide-remote-access", "Reach your machine from anywhere", "在任何地方连上你的机器"),
    ]),
    ("Multiplexers", "多路复用器", [
        ("docs-multiplexers", "Choosing tmux or herdr", "选 tmux 还是 herdr"),
        ("docs-tmux", "Using tmux", "使用 tmux"),
        ("docs-herdr", "Using herdr", "使用 herdr"),
    ]),
    ("Agents", "Agent", [
        ("docs-agents", "Agent status and the lock screen", "Agent 状态与锁屏"),
        ("docs-worktrees", "Isolated tasks with git worktrees", "用 git worktree 开隔离任务"),
        ("guide-claude-code", "Run Claude Code from your iPhone", "用 iPhone 跑 Claude Code"),
    ]),
    ("Input", "输入", [
        ("docs-keyboard", "Keyboard and shortcuts", "键盘与快捷键"),
        ("docs-scrolling", "Scrolling and scrollback", "滚动与回滚"),
        ("docs-clipboard", "Clipboard and paste", "剪贴板与粘贴"),
        ("docs-ime", "CJK and IME input", "中日韩与输入法"),
        # No voice-input page on purpose: the setting exists but ships
        # disabled and labelled "Coming soon". Documenting a toggle the user
        # cannot turn on is how a docs site starts lying.
    ]),
    ("Appearance", "外观", [
        ("docs-appearance", "Themes, fonts and icons", "主题、字体与图标"),
    ]),
    ("Help", "帮助", [
        ("docs-troubleshooting", "Troubleshooting", "排障"),
        ("compare", "How Offhook compares", "和别的比怎么样"),
    ]),
]

BEGIN, END = "<!--DOCSHELL:BEGIN-->", "<!--DOCSHELL:END-->"
HUB_BEGIN, HUB_END = "<!--DOCSHUB:BEGIN-->", "<!--DOCSHUB:END-->"
TOC_BEGIN, TOC_END = "<!--DOCTOC:BEGIN-->", "<!--DOCTOC:END-->"
NEXT_BEGIN, NEXT_END = "<!--DOCNEXT:BEGIN-->", "<!--DOCNEXT:END-->"


def flat(zh: bool) -> list[tuple[str, str]]:
    """Reading order: every page, in sidebar order."""
    out = []
    for _en, _zh, pages in NAV:
        for slug, t_en, t_zh in pages:
            out.append((slug, t_zh if zh else t_en))
    return out


def page_href(slug: str, zh: bool) -> str:
    """The URL a page is SERVED at, which is not its filename.

    nginx maps /docs/herdr onto docs-herdr.html (see marketing/site/nginx.conf).
    The files keep their flat names so they stay openable straight off disk;
    every link the site emits uses the pretty path, because a documentation
    site whose URLs end in .html reads like a zipped folder.
    """
    prefix = "/zh" if zh else ""
    if slug in ("docs", "guides", "compare"):
        return f"{prefix}/{slug}"
    if slug.startswith("docs-"):
        return f"{prefix}/docs/{slug[len('docs-'):]}"
    if slug.startswith("guide-"):
        return f"{prefix}/guide/{slug[len('guide-'):]}"
    return f"{prefix}/{slug}"


def page_file(slug: str, zh: bool) -> str:
    """The file on disk, for reading during the build."""
    return f"{slug}.zh.html" if zh else f"{slug}.html"


def sidebar(current: str, zh: bool) -> str:
    rows = []
    for sec_en, sec_zh, pages in NAV:
        rows.append(f'<p class="dsec">{html.escape(sec_zh if zh else sec_en)}</p>')
        for slug, t_en, t_zh in pages:
            title = html.escape(t_zh if zh else t_en)
            cls = ' class="on"' if slug == current else ""
            aria = ' aria-current="page"' if slug == current else ""
            rows.append(f'<a href="{page_href(slug, zh)}"{cls}{aria}>{title}</a>')
    home = "/zh" if zh else "/"
    top = "文档" if zh else "Docs"
    # ⌘K alone is invisible to anyone who does not already know it, and a phone
    # has no key to press. The button is the affordance; the hint just teaches
    # the shortcut to people who will want it later.
    search = (
        '  <button class="dsearchbtn" type="button" data-docsearch>'
        f'<span>{"搜索文档…" if zh else "Search the docs…"}</span>'
        '<kbd>⌘K</kbd></button>\n'
    )
    return (
        f'{BEGIN}\n<aside class="dnav" id="dnav">\n'
        f'  <a class="dhome" href="{home}">← {"回到首页" if zh else "Back to the app"}</a>\n'
        f'  <p class="dtitle">{top}</p>\n'
        f'{search}'
        f'  <nav>{"".join(rows)}</nav>\n'
        f'</aside>\n{END}'
    )


def toc(body: str, zh: bool) -> str:
    """On-page contents, built from the h3s the author already wrote.

    The anchor has to be worked out the same way anchor_headings() does, or the
    contents point at ids that were never assigned. A heading that arrived with
    its own id keeps it — that is how in-page links from elsewhere stay valid —
    and numbering one by position regardless produced a link to #s19 on a page
    where that heading was id="new-phone" and #s19 existed nowhere.
    """
    heads = re.findall(r'<h3([^>]*)>(.*?)</h3>', body, re.S)
    items = []
    for i, (attrs, raw) in enumerate(heads):
        text = re.sub(r"<[^>]+>", "", raw).strip()
        if not text:
            continue
        existing = re.search(r'id="([^"]+)"', attrs)
        items.append((existing.group(1) if existing else f"s{i + 1}", text))
    if len(items) < 2:
        return f"{TOC_BEGIN}{TOC_END}"
    links = "".join(
        f'<a href="#{anchor}">{html.escape(text)}</a>' for anchor, text in items
    )
    label = "本页内容" if zh else "On this page"
    return (
        f'{TOC_BEGIN}\n<aside class="dtoc">\n  <p class="dtoclabel">{label}</p>\n'
        f'  <nav>{links}</nav>\n</aside>\n{TOC_END}'
    )


def anchor_headings(body: str) -> str:
    """Give each h3 the id the TOC points at."""
    n = [0]

    def sub(m):
        n[0] += 1
        attrs = m.group(1)
        if "id=" in attrs:
            return m.group(0)
        return f'<h3{attrs} id="s{n[0]}">{m.group(2)}</h3>'

    return re.sub(r'<h3([^>]*)>(.*?)</h3>', sub, body, flags=re.S)


def prevnext(slug: str, zh: bool) -> str:
    order = flat(zh)
    idx = next((i for i, (s, _) in enumerate(order) if s == slug), None)
    if idx is None:
        return f"{NEXT_BEGIN}{NEXT_END}"
    parts = []
    if idx > 0:
        s, t = order[idx - 1]
        parts.append(
            f'<a class="dprev" href="{page_href(s, zh)}">'
            f'<span>{"上一页" if zh else "Previous"}</span>{html.escape(t)}</a>'
        )
    else:
        parts.append("<span></span>")
    if idx < len(order) - 1:
        s, t = order[idx + 1]
        parts.append(
            f'<a class="dnext" href="{page_href(s, zh)}">'
            f'<span>{"下一页" if zh else "Next"}</span>{html.escape(t)}</a>'
        )
    return f'{NEXT_BEGIN}\n<div class="dpager">{"".join(parts)}</div>\n{NEXT_END}'


def strip(marker_begin: str, marker_end: str, text: str) -> str:
    return re.sub(
        re.escape(marker_begin) + ".*?" + re.escape(marker_end), "", text, flags=re.S
    )


def lede(slug: str, zh: bool) -> str:
    """The first sentence of a page's own intro, used as its card on the hub."""
    path = SITE / page_file(slug, zh)
    if not path.exists():
        return ""
    body = path.read_text().split(END)[-1]
    m = re.search(r'<p class="sub">(.*?)</p>', body, re.S)
    if not m:
        return ""
    text = re.sub(r"<[^>]+>", "", m.group(1))
    text = re.sub(r"\s+", " ", text).strip()
    first = re.split(r"(?<=[.。])\s", text)[0]
    return html.escape(first)


def docs_hub(zh: bool) -> str:
    """The /docs index, generated from NAV.

    It used to be hand-written, and drifted: it listed twelve of the nineteen
    pages the sidebar carries, grouped them four ways against the sidebar's
    seven, and gave the same destination a different eyebrow and a different
    description depending on which grid it appeared in. Generating it means the
    hub cannot disagree with the sidebar, and the eyebrow is the section name
    every time rather than a word chosen per card.
    """
    out = [HUB_BEGIN]
    for sec_en, sec_zh, pages in NAV:
        section = sec_zh if zh else sec_en
        out.append(f"<h3>{html.escape(section)}</h3>")
        out.append('<div class="grid">')
        for slug, t_en, t_zh in pages:
            title = html.escape(t_zh if zh else t_en)
            out.append(
                '<div class="cell">'
                f'<span class="ic">{html.escape(section)}</span>'
                f'<h4><a href="{page_href(slug, zh)}">{title}</a></h4>'
                f"<p>{lede(slug, zh)}</p>"
                "</div>"
            )
        out.append("</div>")
    out.append(HUB_END)
    return "\n".join(out)


def build_index() -> None:
    """A search index small enough to ship inline with the page.

    Client-side because there is no server to ask — the same reason the app
    has no account. Titles plus the first slice of each section's prose is
    enough for "which page was that on", which is what docs search is for.
    """
    entries = []
    for _sec_en, _sec_zh, pages in NAV:
        for slug, t_en, t_zh in pages:
            for zh in (False, True):
                path = SITE / page_file(slug, zh)
                if not path.exists():
                    continue
                raw = path.read_text()
                # Index the CONTENT only. The injected sidebar lists every page
                # title, so indexing the raw page made every page match every
                # query — search that returns everything is search that returns
                # nothing.
                body = raw
                for a, b in ((BEGIN, END), (TOC_BEGIN, TOC_END), (NEXT_BEGIN, NEXT_END)):
                    body = strip(a, b, body)
                body = re.sub(r"<header.*?</header>", " ", body, flags=re.S)
                body = re.sub(r"<footer.*?</footer>", " ", body, flags=re.S)
                body = re.sub(r"<script.*?</script>", " ", body, flags=re.S)
                body = re.sub(r"<style.*?</style>", " ", body, flags=re.S)
                text = re.sub(r"<[^>]+>", " ", body)
                text = html.unescape(re.sub(r"\s+", " ", text)).strip()
                entries.append({
                    "u": page_href(slug, zh),
                    "t": t_zh if zh else t_en,
                    "l": "zh" if zh else "en",
                    "b": text[:2000],
                })
    (SITE / "assets" / "docs-index.json").write_text(
        json.dumps(entries, ensure_ascii=False, separators=(",", ":"))
    )
    print(f"  search index: {len(entries)} entries")


# href/src that points at a file rather than a route, and is not already
# absolute. Anything matching has to be rooted or it breaks at depth.
RELATIVE_ASSET = re.compile(
    r'\b(href|src)="(?!/|\#|https?:|data:|mailto:|\./)'
    r'([^"]+\.(?:css|js|png|jpe?g|svg|webp|json|ico)(?:\?[^"]*)?)"'
)


def process(slug: str, zh: bool) -> bool:
    path = SITE / page_file(slug, zh)
    if not path.exists():
        return False
    s = path.read_text()

    # Idempotence: remove anything a previous run injected.
    for a, b in ((BEGIN, END), (TOC_BEGIN, TOC_END), (NEXT_BEGIN, NEXT_END)):
        s = strip(a, b, s)
    s = re.sub(r'<div class="dwrap[^"]*">\s*', "", s)
    s = s.replace("<!--/dwrap-->", "")
    s = re.sub(r'\s*<div class="dbody">|</div><!--dbody-->', "", s)

    s = anchor_headings(s)

    # Wrap the page's content in the three-column shell. Most pages are one
    # <section class="band doc">, but a few authors used several plain .band
    # sections — take everything from the first content shell to the last, so
    # a multi-section page keeps all of it.
    starts = [mm.start() for mm in re.finditer(r'<div class="shell">\s*<section class="band', s)]
    if not starts:
        print(f"  ! {path.name}: no content section found, skipped")
        return False
    first = starts[0]
    last_close = s.rfind("</section>")
    end = s.find("</div>", last_close)
    if last_close < first or end < 0:
        print(f"  ! {path.name}: could not bound the content, skipped")
        return False
    end += len("</div>")
    block = s[first:end]

    class _M:
        pass
    m = _M(); m.start = lambda: first; m.end = lambda: end
    # A page whose content is a wide comparison table cannot also carry the
    # contents rail: five columns need ~884px and the reading column offers
    # 736, so the last column fell off the edge and the table scrolled
    # sideways — on the one page where the table IS the argument. Such pages
    # give up the rail and take its width instead.
    wide = any(
        len(re.findall(r"<th\b", head)) >= 5
        for head in re.findall(r"<thead[^>]*>(.*?)</thead>", block, re.S)
    )
    shell = (
        ('<div class="dwrap notoc">\n' if wide else '<div class="dwrap">\n')
        + sidebar(slug, zh) + "\n"
        + '<div class="dbody">\n'
        + block + "\n"
        + prevnext(slug, zh) + "\n"
        + "</div><!--dbody-->\n"
        + ("" if wide else toc(block, zh) + "\n")
        + "</div><!--/dwrap-->"
    )
    s = s[: m.start()] + shell + s[m.end():]

    # The search modal + its script, once per page.
    if "docs-search.js" not in s:
        s = s.replace(
            "</body>",
            '<div class="dsearch" id="dsearch" hidden style="display:none">\n'
            '  <div class="dsearchbox" role="dialog" aria-modal="true" aria-label="Search docs">\n'
            f'    <input id="dq" type="search" autocomplete="off" placeholder="{"搜索文档…" if zh else "Search the docs…"}">\n'
            '    <div id="dres"></div>\n'
            "  </div>\n</div>\n"
            '<script src="/docs-search.js?v=15" defer></script>\n</body>',
        )

    # Pages are served at pretty paths (/docs/herdr), so a relative asset
    # reference resolves against /docs/ and 404s — which once shipped every
    # docs page with no stylesheet at all. Normalise on every build so a
    # hand-written page cannot reintroduce it.
    s = RELATIVE_ASSET.sub(lambda m: f'{m.group(1)}="/{m.group(2)}"', s)

    path.write_text(s)
    return True


def build_hub() -> None:
    """Replace the card grids on /docs with generated ones."""
    for zh in (False, True):
        path = SITE / page_file("docs", zh)
        if not path.exists():
            continue
        s = path.read_text()
        block = docs_hub(zh)
        if HUB_BEGIN in s:
            s = re.sub(
                re.escape(HUB_BEGIN) + ".*?" + re.escape(HUB_END), block, s, flags=re.S
            )
        else:
            # First run: replace everything between the intro and the closing
            # section with the generated hub.
            m = re.search(r'(<p class="sub">.*?</p>)(.*?)(\s*</section>)', s, re.S)
            if not m:
                print(f"  ! could not find the hub body in {path.name}")
                continue
            s = s[: m.end(1)] + "\n" + block + m.group(3) + s[m.end(3):]
        path.write_text(s)
        print(f"  hub regenerated: {path.name}")


def main() -> None:
    print("Building the docs shell…")
    build_hub()
    done = 0
    for _sec_en, _sec_zh, pages in NAV:
        for slug, _t_en, _t_zh in pages:
            for zh in (False, True):
                if process(slug, zh):
                    done += 1
    print(f"  chrome injected into {done} pages")
    build_index()


if __name__ == "__main__":
    main()
