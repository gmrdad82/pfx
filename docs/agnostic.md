# The agnostic check

pfx is generic: no app or game puts its names, colours, fonts or assets in it. Each product owns and packs its own and only uses the engine. `tests/agnostic.rs` holds the check; the gate runs it with the rest of the root package's tests, in about a second.

## What it checks

It reads `git ls-files` and skips `prompts/`, `CONVENTIONS.md`, `HANDOVER.md`, `ROADMAP.md`, `AGENTS.md` and `tests/agnostic/`. Over every other tracked file (its name and, for text files, every line) it counts the patterns of `tests/agnostic/deny.txt`: product names, a product's font family and a product's colours. The file's header says how each pattern matches, which are word-bounded and how a generic phrase is allowed.

Today's hits are the baseline in `tests/agnostic/baseline.txt`, one line per file with its count per label. The baseline only shrinks, because the product recipes, desks and presets are still moving out. The check fails when:

- a file outside the baseline has a hit;
- a baselined file's count for a label rises, or the file gains a label it did not have;
- a tracked font, image, SVG or audio file is missing from `tests/agnostic/fixtures.txt`, or its sha256 no longer matches.

## Shrinking the baseline

When counts fall the check still passes, prints how far the baseline can shrink and the command to rewrite it:

```
tests/agnostic/update.sh
```

It rewrites `baseline.txt` from the tracked files and refuses while anything has risen. Commit the smaller file with the work that removed the hits. `tests/agnostic/update.sh --grow` writes the baseline whatever it finds; it is for creating the file, and a diff that adds lines or raises a number is a review failure.

## Fixture files

`tests/agnostic/fixtures.txt` lists each asset file as `sha256  path  source`. The source names where it came from and starts with one of:

- `generated:` the engine script or tool that made it;
- `OFL:` or `CC0:` the open source it was taken from, with its licence;
- `product:` a copy of a product's file that goes out in wave D. No new one may be added.

The failure for an unlisted file prints the line to add. A file that was dropped but is still listed is noted, and its line should go.

## When a product needs something

A product's need becomes a generic feature: the engine takes the value (a colour, a font, a glyph set, a recipe) as a parameter and the product passes its own. The engine's tests, examples and docs use neutral fixtures and the fonts it owns, never a product's.
