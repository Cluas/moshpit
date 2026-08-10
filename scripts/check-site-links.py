#!/usr/bin/env python3
"""Fail if any root-absolute link in a built site points at a file that is not there.

Run against the Astro output before it is baked into an image:

    scripts/check-site-links.py marketing/site-next/dist

This exists because the migration off flat HTML moved every URL through a
different mechanism at once — Astro's routing, nginx's `try_files $uri $uri.html`
and Starlight's own link resolution. Two whole sections (/docs and /zh/docs, 36
links) pointed at pages nobody had generated yet, and nothing in the build
complained: Astro does not resolve `href` strings, and nginx cannot warn about a
file it was never asked for until a reader asks.

Resolution mirrors the nginx config, so a pass here means a pass in production:
  /x        → /x, /x.html, or /x/index.html
  /x.html   → the file itself
Relative links are skipped on purpose. They resolve against the current
directory, which is a different question, and the pages do not use them.
"""
from __future__ import annotations

import collections
import pathlib
import re
import sys

LINK = re.compile(r'(?:href|src)="(/[^"#?]*)')


def servable(dist: pathlib.Path) -> set[str]:
    """Every path nginx would answer with a 200, given this directory."""
    urls = {"/"}
    for f in dist.rglob("*"):
        if not f.is_file():
            continue
        url = "/" + str(f.relative_to(dist))
        urls.add(url)
        if url.endswith(".html"):
            urls.add(url[: -len(".html")])  # try_files $uri.html
            if url.endswith("/index.html"):
                urls.add(url[: -len("index.html")].rstrip("/") or "/")
    return urls


def main() -> int:
    if len(sys.argv) != 2:
        sys.exit(f"usage: {sys.argv[0]} <dist-dir>")
    dist = pathlib.Path(sys.argv[1])
    if not dist.is_dir():
        sys.exit(f"not a directory: {dist}")

    urls = servable(dist)
    broken: collections.Counter[str] = collections.Counter()
    example: dict[str, str] = {}
    pages = 0
    for page in sorted(dist.rglob("*.html")):
        pages += 1
        for match in LINK.finditer(page.read_text()):
            url = match.group(1)
            if url not in urls and url.rstrip("/") not in urls:
                broken[url] += 1
                example.setdefault(url, str(page.relative_to(dist)))

    if not broken:
        print(f"  {pages} pages, every internal link resolves")
        return 0
    for url, count in broken.most_common():
        print(f"  BROKEN {url}  ({count}×, e.g. {example[url]})")
    print(f"\n{len(broken)} distinct broken links across {pages} pages")
    return 1


if __name__ == "__main__":
    sys.exit(main())
