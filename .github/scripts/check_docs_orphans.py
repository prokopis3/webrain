#!/usr/bin/env python3
"""Fail CI if a docs/*.md|*.mdx file is neither in the Mintlify navigation
(docs/docs.json) nor ignored by docs/.mintignore.

This is the enforcement arm of the `.mintignore` header comment
("the site is the pages in docs.json nav") — internal/working files dropped
into docs/ must never become publishable by accident.
"""
import fnmatch
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]  # repo root
DOCS = ROOT / "docs"


def nav_blob() -> str:
    with open(DOCS / "docs.json", encoding="utf-8") as f:
        return f.read()


def ignore_patterns() -> list:
    pats = []
    try:
        with open(DOCS / ".mintignore", encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if line and not line.startswith("#"):
                    pats.append(line)
    except FileNotFoundError:
        pass
    return pats


def is_ignored(rel: str, pats: list) -> bool:
    # Directory patterns (e.g. `adr/`) ignore everything beneath them.
    for p in pats:
        if fnmatch.fnmatch(rel, p) or fnmatch.fnmatch(rel, p.rstrip("/") + "/*"):
            return True
    return False


def main() -> int:
    blob = nav_blob()
    pats = ignore_patterns()
    orphans = []
    for f in sorted(DOCS.rglob("*")):
        if f.is_file() and f.suffix in (".md", ".mdx"):
            rel = f.relative_to(DOCS).as_posix()
            if is_ignored(rel, pats):
                continue
            # Mintlify nav references paths WITHOUT the extension.
            nav = rel[: -len(f.suffix)]
            if nav not in blob:
                orphans.append(rel)
    if orphans:
        print("docs/.mintignore enforcement: files neither in docs.json nav nor ignored:")
        for o in orphans:
            print(f"  {o}")
        print("Add each to docs/.mintignore (internal/working file) or reference it in docs/docs.json navigation.")
        return 1
    print("docs orphan check: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
