# .pitofx in git

<p align="center"><img src="images/git.gif" width="800" alt="A terminal: git init a repository, pfx new street.pitofx --from street writes a starter recipe and git add stages it; pfx set street.pitofx sun.hour=7.5 changes one value; git diff prints only Binary files differ; pfx git setup writes the .gitattributes line and the repository's textconv driver; git diff again prints the change as TOML, the recipe's hash in the metadata and the one line hour = 19.13 becoming hour = 7.5."></p>

A `.pitofx` is a compressed container, so git sees it as binary and `git diff` says only `Binary files differ`. pfx gives git a text view of it instead: a [textconv diff driver](https://git-scm.com/docs/gitattributes#_performing_text_diffs_of_binary_files) that prints the document as TOML. The file in the repository stays the `.pitofx`; only what `git diff`, `git log -p` and `git show` print changes.

## Set it up

`pfx new <game>` writes a `.gitattributes` with the line below into the game it scaffolds and, when the folder is already a git repository, sets the repository's own git config too. For any other repository, run once from inside it:

```sh
pfx git setup [folder] [--json]
```

It writes two things and nothing else:

- `.gitattributes` at the repository's top gets `*.pitofx diff=pitofx`, appended when no line for `*.pitofx` names `diff=pitofx` yet. Commit it, so every clone tags its `.pitofx` files the same way.
- The repository's local git config (`.git/config`, never the global one) gets `diff.pitofx.textconv = pfx scene print --all` and `diff.pitofx.binary = false`. Git keeps local config out of commits, so each clone runs `pfx git setup` once; without it, git falls back to `Binary files differ`.

Run again, it changes nothing and says so (`already there`). Outside a git repository it refuses in one line, `pfx git setup: <folder>: not inside a git repository (git init first)`, and exits 1. `--json` prints one object: `repo`, `gitattributes` (`line`, `changed`) and `config` (each `key`, `value`, `changed`). Git must be on `PATH`, and so must `pfx`, since git runs the textconv command by that name.

Set up by hand, the same two steps are:

```sh
echo '*.pitofx diff=pitofx' >> .gitattributes
git config diff.pitofx.textconv 'pfx scene print --all'
```

## What the diff shows

`pfx scene print <file.pitofx> --all` prints every entry of the document, in an order that depends only on what it holds, never on where its chunks sit in the file:

- each entry starts with a `# <TAG> <name>` line;
- the TOML entries print whole: the scene's metadata (`SMET`), the recipe (`RCPE`), each scene file (`SCNE`), each `.toml` file the document holds (`FILE`, so materials and shared rigs show) and the builder's overrides (`OVRD`);
- every other entry prints one line, its raw size and the CRC-32 its chunk table holds: packed meshes, images and other files, the build (`GLTF`, `GBIN`, `FACT`, `NODE`) and the thumbnail (`THMB`), as `# FILE meshes/crate.glb: 94580 bytes, crc32 1c0ffee5`;
- entries sort by kind in the order above, then by name; tags pfx does not know come last.

So an edit made in place, which appends the changed entry and leaves dead bytes behind ([Editing in place](pfx-file.md#editing-in-place)), a compact, and a save that writes the chunks in another order all print the same text for the same content. A packed file changes the diff by its one line; its bytes never reach the terminal, and nothing is decompressed to list it, so `git diff` on a document holding gigabytes of meshes costs what its TOML costs.

```diff
diff --git a/content/first.pitofx b/content/first.pitofx
--- a/content/first.pitofx
+++ b/content/first.pitofx
@@ -14,7 +14,7 @@ entry = "content/first.scene.toml"
 [[object]]
 name = "title"
-at = [0.0, 1.2, 0.0]
+at = [0.0, 1.6, 0.0]
```

## Merges

Git still merges a `.pitofx` as a binary file: when both branches changed the same document, it keeps the current branch's copy and reports a conflict. Pick a side with `git checkout --ours` or `--theirs`, then carry the other side's edits over with `pfx scene print` and `pfx scene write`, or with the editor.
