#!/usr/bin/env python3
"""Generate the NSH screens' translations (ui/locales/strings.json) from the
app's string catalog, Moshpit/Resources/Localizable.xcstrings: one source for
a text, whichever UI shows it.

The keys are the ones the NSH screens use: every `t!("...")` and
`plural!("...", n)` literal in ui/src. A key the catalog lacks is an error, so
a screen never ships a text no translator saw; give it to the catalog first
(scripts/gen/gen_xcstrings.py), then run this.

The output is what NSH's `Root::translations_json` reads (rust-i18n's shape):
{"<language>": {"<key>": "<text>"}} for every language the catalog has. The
key is the English text, as in the catalog (its source language). A
language without a translation of a key is left out of that language, and
NSH falls back the way rust-i18n does: to the language without its last
subtag, then to English.

- Format specifiers become rust-i18n arguments, numbered as Foundation
  numbers them: `%@` and `%lld` are `%{arg1}`, `%{arg2}`, ... in order,
  `%2$@` is `%{arg2}`, `%%` is `%`. A screen passes them as
  `t!(key, arg1 = ...)`: see ui/src/strings.rs.
- Plurals (NSH has no plural rules): a key the screens look up with
  `plural!` is written once per CLDR category, as `<key>#<category>`
  (`%lld keys#one`), in every language that has the key. A category the
  catalog has no form for takes the `other` form, as Foundation does; a
  language with a single form (ja, ko, zh) gives it to every category.
  ui/src/strings.rs picks the category with CLDR's rules.

Usage:
  scripts/gen/gen_nsh_strings.py           # write ui/locales/strings.json
  scripts/gen/gen_nsh_strings.py --check   # exit 1 if it is out of date
"""
import json
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CATALOG = os.path.join(ROOT, "Moshpit", "Resources", "Localizable.xcstrings")
UI_SRC = os.path.join(ROOT, "ui", "src")
OUT = os.path.join(ROOT, "ui", "locales", "strings.json")

# `t!("key"` / `tl!("key"` / `plural!("key"`, the key a Rust string
# literal (escapes kept for `rust_string` to read).
KEY_USE = re.compile(r'\b(t|tl|plural)!\(\s*"((?:[^"\\]|\\.)*)"')

# CLDR's plural categories.
CATEGORIES = ["zero", "one", "two", "few", "many", "other"]

# A Foundation format specifier: %@ %d %lld %f %s, with an optional
# position (`%2$@`); `%%` is a literal percent sign.
SPECIFIER = re.compile(r"%(?:(\d+)\$)?(?:@|l{0,2}[dDuUxXoO]|[fFeEgGcCsSp]|%)")


def rust_string(literal):
    """The text of a Rust string literal's body."""
    out, i = [], 0
    while i < len(literal):
        c = literal[i]
        if c != "\\":
            out.append(c)
            i += 1
            continue
        nxt = literal[i + 1]
        if nxt == "u":
            end = literal.index("}", i)
            out.append(chr(int(literal[i + 3:end], 16)))
            i = end + 1
            continue
        out.append({"n": "\n", "t": "\t", "r": "\r", "0": "\0", "\\": "\\", '"': '"', "'": "'"}[nxt])
        i += 2
    return "".join(out)


def used_keys():
    """Every key ui/src looks up, in first-use order: {key: is a plural}."""
    keys = {}
    for directory, _, files in sorted(os.walk(UI_SRC)):
        for name in sorted(files):
            if not name.endswith(".rs"):
                continue
            with open(os.path.join(directory, name), encoding="utf-8") as f:
                for match in KEY_USE.finditer(f.read()):
                    key = rust_string(match.group(2))
                    keys[key] = keys.get(key, False) or match.group(1) == "plural"
    return keys


def arguments(text):
    """`text` with its format specifiers as rust-i18n's numbered arguments."""
    count = 0

    def replace(match):
        nonlocal count
        if match.group(0) == "%%":
            return "%"
        count += 1
        return "%{arg" + (match.group(1) or str(count)) + "}"

    return SPECIFIER.sub(replace, text)


def forms(localization):
    """A localization's texts: {"": text} or, plural, {category: text}."""
    if "stringUnit" in localization:
        return {"": localization["stringUnit"]["value"]}
    variations = localization.get("variations", {})
    if "plural" in variations:
        return {category: v["stringUnit"]["value"] for category, v in variations["plural"].items()}
    raise SystemExit(f"unsupported variations {sorted(variations)}: give the screen its own key")


def generate():
    with open(CATALOG, encoding="utf-8") as f:
        catalog = json.load(f)
    source = catalog["sourceLanguage"]
    strings = catalog["strings"]
    languages = {source}
    for entry in strings.values():
        languages.update(entry.get("localizations", {}))
    out = {language: {} for language in sorted(languages)}
    missing = []
    for key, is_plural in used_keys().items():
        entry = strings.get(key)
        if entry is None:
            missing.append(key)
            continue
        localizations = entry.get("localizations", {})
        if source not in localizations:
            # The key is the source language's text.
            localizations = {**localizations, source: {"stringUnit": {"value": key}}}
        for language, localization in localizations.items():
            texts = forms(localization)
            if not is_plural:
                if "" not in texts:
                    raise SystemExit(f"{key!r} has plural forms in {language}: look it up with plural!")
                out[language][key] = arguments(texts[""])
                continue
            other = texts.get("other", texts.get(""))
            if other is None:
                raise SystemExit(f"{key!r} has no `other` form in {language}")
            for category in CATEGORIES:
                out[language][f"{key}#{category}"] = arguments(texts.get(category, other))
    if missing:
        raise SystemExit(
            "keys ui/src uses that Localizable.xcstrings lacks (add them with gen_xcstrings.py):\n  "
            + "\n  ".join(missing))
    return json.dumps(out, ensure_ascii=False, indent=2, sort_keys=True) + "\n"


def main():
    text = generate()
    if "--check" in sys.argv:
        current = open(OUT, encoding="utf-8").read() if os.path.exists(OUT) else ""
        if current != text:
            print(f"{os.path.relpath(OUT, ROOT)} is out of date: run scripts/gen/gen_nsh_strings.py")
            sys.exit(1)
        return
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8") as f:
        f.write(text)
    print(f"wrote {os.path.relpath(OUT, ROOT)}")


if __name__ == "__main__":
    main()
