#!/usr/bin/env bash
# Render docs/design/*.md into the folder the local HTTP server publishes, so
# they can be read on a phone next to the build they describe.
#
#   ./scripts/publish-design-docs.sh          # render + (re)start the server
#   ./scripts/publish-design-docs.sh --no-serve
#
# The server is plain HTTP on the LAN — fine for a build and some prose, not
# for anything you wouldn't say out loud on the office network.

set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

OUT="build/serve"
PORT="${OFFHOOK_SERVE_PORT:-8787}"
mkdir -p "$OUT/docs"

command -v pandoc >/dev/null || { echo "✘ pandoc not installed (brew install pandoc)" >&2; exit 1; }

# Dark, mobile-first, and self-contained: no CDN, no fonts to fetch, because
# this gets read on a phone that may only have the LAN.
cat > "$OUT/docs/style.css" <<'CSS'
:root { color-scheme: dark; }
* { box-sizing: border-box; }
body {
  font: 16px/1.7 -apple-system, "PingFang SC", system-ui, sans-serif;
  max-width: 46rem; margin: 0 auto; padding: 1.5rem 1.1rem 5rem;
  background: #0b0d10; color: #e6e9ee; overflow-wrap: break-word;
}
a { color: #9b8cff; }
h1, h2, h3, h4 { line-height: 1.3; margin: 2.2rem 0 .8rem; }
h1 { font-size: 1.6rem; border-bottom: 1px solid #232833; padding-bottom: .5rem; }
h2 { font-size: 1.28rem; color: #cfd4de; }
h3 { font-size: 1.08rem; color: #cfd4de; }
code {
  font: 13px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace;
  background: #171a1f; padding: .15rem .4rem; border-radius: 5px;
}
pre {
  background: #12151a; border: 1px solid #232833; border-radius: 10px;
  padding: .9rem 1rem; overflow-x: auto;
}
pre code { background: none; padding: 0; font-size: 12.5px; }
/* Tables carry most of the meaning in these docs; let them scroll rather
   than squeeze columns into unreadable slivers on a phone. */
.table-wrap { overflow-x: auto; margin: 1rem 0; }
table { border-collapse: collapse; width: 100%; font-size: 14px; }
th, td { border: 1px solid #232833; padding: .5rem .65rem; text-align: left; vertical-align: top; }
th { background: #161a20; color: #cfd4de; white-space: nowrap; }
blockquote {
  margin: 1.2rem 0; padding: .75rem 1rem;
  border-left: 3px solid #6c5ce7; background: #14161c; border-radius: 0 8px 8px 0;
}
blockquote p:first-child { margin-top: 0; }
blockquote p:last-child { margin-bottom: 0; }
hr { border: none; border-top: 1px solid #232833; margin: 2.5rem 0; }
del { color: #6b7280; }
.nav { font-size: 13px; color: #8b94a3; margin-bottom: 1.5rem; }
.nav a { margin-right: 1rem; }
CSS

render() {   # render <src.md> <dst.html> <title>
  pandoc "$1" -f gfm -t html5 --standalone --metadata title="$3" \
    --css style.css --wrap=none \
    --include-before-body=<(printf '<div class=nav><a href="../">← build</a><a href="./">all docs</a></div>') \
    -o "$2"
  # pandoc emits bare <table>; wrap each so wide tables scroll on their own
  # instead of pushing the whole page sideways.
  python3 - "$2" <<'PY'
import re, sys, pathlib
p = pathlib.Path(sys.argv[1])
s = p.read_text()
s = s.replace("<table>", '<div class="table-wrap"><table>').replace("</table>", "</table></div>")
p.write_text(s)
PY
}

echo "▶ Rendering docs/design/*.md"
LINKS=""
for md in docs/design/*.md; do
  base="$(basename "$md" .md)"
  # -E: BSD sed has no `\+` in basic regex, so the `#` prefix survived.
  title="$(head -1 "$md" | sed -E 's/^#+[[:space:]]*//')"
  render "$md" "$OUT/docs/$base.html" "$title"
  echo "  · $base.html — $title"
  LINKS="$LINKS<li><a href=\"$base.html\">$title</a> <small>$base.md</small></li>"
done

cat > "$OUT/docs/index.html" <<HTML
<!doctype html><meta charset=utf-8><meta name=viewport content="width=device-width,initial-scale=1">
<title>Offhook design docs</title><link rel=stylesheet href=style.css>
<div class=nav><a href="../">← build</a></div>
<h1>Design docs</h1>
<ul>$LINKS</ul>
<p><small>Rendered from <code>docs/design/</code> at $(date '+%Y-%m-%d %H:%M'). Re-run
<code>scripts/publish-design-docs.sh</code> after editing.</small></p>
HTML

if [ "${1:-}" != "--no-serve" ]; then
  if ! curl -sf "http://127.0.0.1:$PORT/" >/dev/null 2>&1; then
    echo "▶ Starting HTTP server on :$PORT"
    ( cd "$OUT" && nohup python3 -m http.server "$PORT" --bind 0.0.0.0 > /tmp/offhook-serve.log 2>&1 & )
    sleep 1
  fi
  IP="$(ipconfig getifaddr en0 2>/dev/null || ipconfig getifaddr en1 2>/dev/null || echo 127.0.0.1)"
  echo
  echo "✓ http://$IP:$PORT/docs/"
fi
