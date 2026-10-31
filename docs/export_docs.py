#!/usr/bin/env python3
"""Convert Markdown documentation into Hugo-compatible content.

Reads all ``.md`` files from a source directory (default ``./docs``) and
writes them to ``content/docs/`` with generated Hugo front matter.

Rules:
  * Top-level docs are categorized by filename keywords:
      - readme / intro / overview / install / setup / configure -> getting-started/
      - usage / guide / tutorial                                -> guides/
      - api / reference / docs                                  -> reference/
    Unmatched files fall back to reference/ (glossaries and developer
    docs read like reference material).
  * ``book/`` is treated as a book: its structure is preserved under
    ``content/docs/book/``. The book README becomes the section landing
    page (``_index.md``), chapters and appendices keep their filenames,
    and non-Markdown assets (e.g. ``labs/**/*.mod``, ``manifest.toml``)
    are copied verbatim so relative links from chapters keep working.
  * Front matter per file:
      title:   "<Project Name> - <Extracted Title>"
      weight:  10 + (category_index * 5)   # index of the file within its category
      type:    docs
      draft:   false
      lastmod: source file mtime in UTC, YYYY-MM-DDTHH:MM:SS+00:00

The first ``# `` heading is used as the extracted title and stripped from
the body so Hugo does not render the title twice.

Usage:
    python3 docs/export_docs.py [--source ./docs] [--output content/docs]
                                [--project "Tyu Programming Language"]
"""

from __future__ import annotations

import argparse
import re
import shutil
from datetime import datetime, timezone
from pathlib import Path

# Categorization rules, checked in order; first match wins.
CATEGORY_RULES: list[tuple[str, Path, tuple[str, ...]]] = [
    ("getting-started", Path("getting-started"),
     ("readme", "intro", "overview", "install", "setup", "configure")),
    ("guides", Path("guides"),
     ("usage", "guide", "tutorial")),
    ("reference", Path("reference"),
     ("api", "reference", "docs")),
]

FALLBACK_CATEGORY = ("reference", Path("reference"))

# Section landing pages, ordered for the Hugo nav. The book needs no entry:
# its README becomes content/docs/book/_index.md directly.
SECTION_PAGES: list[tuple[Path, str, int]] = [
    (Path("getting-started"), "Getting Started", 5),
    (Path("guides"), "Guides", 15),
    (Path("reference"), "Reference", 25),
]

HEADING_RE = re.compile(r"^#\s+(.+?)\s*$", re.MULTILINE)
FRONT_MATTER_RE = re.compile(r"\A---\s*\n.*?\n---\s*\n", re.DOTALL)


def extract_title(body: str, fallback: str) -> tuple[str, str]:
    """Return (title, body_without_leading_h1)."""
    match = HEADING_RE.search(body)
    if match and body.lstrip().startswith("#"):
        title = match.group(1).strip()
        # Drop the H1 (and surrounding blank lines) from the body.
        body = HEADING_RE.sub("", body, count=1).lstrip("\n")
    else:
        title = fallback
    return title, body


def title_from_filename(stem: str) -> str:
    """'ch01-the-rigor-is-the-engine' -> 'Ch01 The Rigor Is The Engine'."""
    words = re.split(r"[-_]+", stem)
    return " ".join(w.capitalize() if not w.isdigit() else w.upper() for w in words if w)


def categorize(filename: str) -> tuple[str, Path]:
    name = filename.lower()
    for _, folder, keywords in CATEGORY_RULES:
        if any(k in name for k in keywords):
            return (_, folder)
    return FALLBACK_CATEGORY


def yaml_str(value: str) -> str:
    escaped = value.replace("\\", "\\\\").replace('"', '\\"')
    return f'"{escaped}"'


def front_matter(title: str, weight: int, lastmod: str) -> str:
    return (
        "---\n"
        f"title: {yaml_str(title)}\n"
        f"weight: {weight}\n"
        "type: docs\n"
        "draft: false\n"
        f"lastmod: {lastmod}\n"
        "---\n\n"
    )


def lastmod_of(path: Path) -> str:
    mtime = datetime.fromtimestamp(path.stat().st_mtime, tz=timezone.utc)
    return mtime.strftime("%Y-%m-%dT%H:%M:%S+00:00")


def detect_project_name(source: Path) -> str:
    """Project name from the README next to the source dir, else dir name."""
    for candidate in (source.parent / "README.md", Path("README.md")):
        if candidate.is_file():
            match = HEADING_RE.search(candidate.read_text(encoding="utf-8"))
            if match:
                return match.group(1).strip()
    return source.resolve().parent.name


def book_sort_key(path: Path) -> tuple:
    """README first, then chapters in numeric order, then appendices."""
    stem = path.stem.lower()
    if stem == "readme":
        return (0, 0, stem)
    chapter = re.match(r"ch(\d+)", stem)
    if chapter:
        return (1, int(chapter.group(1)), stem)
    appendix = re.match(r"appendix-([a-z])", stem)
    if appendix:
        return (2, ord(appendix.group(1)), stem)
    return (3, 0, stem)


def convert_file(src: Path, dest: Path, project: str, weight: int) -> None:
    text = src.read_text(encoding="utf-8")
    text = FRONT_MATTER_RE.sub("", text, count=1)  # drop any existing front matter
    title, body = extract_title(text, title_from_filename(src.stem))
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_text(
        front_matter(f"{project} - {title}", weight, lastmod_of(src)) + body,
        encoding="utf-8",
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--source", type=Path, default=Path("docs"),
                        help="directory containing the Markdown sources")
    parser.add_argument("--output", type=Path, default=Path("content/docs"),
                        help="Hugo content directory to write into")
    parser.add_argument("--project", type=str, default=None,
                        help="project name for titles (default: detected from README)")
    args = parser.parse_args()

    source = args.source.resolve()
    output = args.output.resolve()
    if not source.is_dir():
        raise SystemExit(f"source directory not found: {source}")

    project = args.project or detect_project_name(source)
    book_src = source / "book"

    # --- Top-level docs: categorize, then weight = 10 + index * 5 ---------
    by_category: dict[Path, list[Path]] = {}
    for md in sorted(source.rglob("*.md")):
        if book_src in md.parents or md == book_src:
            continue  # handled below as the book
        _, folder = categorize(md.name)
        by_category.setdefault(folder, []).append(md)

    converted = 0
    for folder, files in by_category.items():
        for index, md in enumerate(sorted(files, key=lambda p: p.name.lower())):
            dest = output / folder / f"{md.stem.lower()}.md"
            convert_file(md, dest, project, 10 + index * 5)
            converted += 1
            print(f"  {md.relative_to(source)}  ->  {dest.relative_to(output.parent)}")

    # --- The book: structure preserved, chapters ordered, labs copied -----
    if book_src.is_dir():
        book_mds = sorted(book_src.rglob("*.md"), key=book_sort_key)
        for index, md in enumerate(book_mds):
            rel = md.relative_to(book_src)
            if md.parent == book_src and md.stem.lower() == "readme":
                dest = output / "book" / "_index.md"  # section landing page
            else:
                dest = output / "book" / rel
            convert_file(md, dest, project, 10 + index * 5)
            converted += 1
            print(f"  {md.relative_to(source)}  ->  {dest.relative_to(output.parent)}")

        # Non-Markdown book assets (labs/*.mod, manifest.toml, ...) so that
        # relative links from chapters keep resolving under Hugo.
        for asset in book_src.rglob("*"):
            if asset.is_file() and asset.suffix != ".md":
                dest = output / "book" / asset.relative_to(book_src)
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(asset, dest)
                print(f"  {asset.relative_to(source)}  ->  copied")

    # --- Section landing pages for the nav --------------------------------
    for folder, title, weight in SECTION_PAGES:
        dest = output / folder / "_index.md"
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(
            front_matter(f"{project} - {title}", weight, lastmod_of(source))
            + f"Browse the {title.lower()} pages in this section.\n",
            encoding="utf-8",
        )

    print(f"\n{converted} Markdown file(s) converted into {output}")


if __name__ == "__main__":
    main()
