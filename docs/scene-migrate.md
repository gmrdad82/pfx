# Import a TOML file

A scene or a recipe can be written as plain TOML in any text editor and then imported into a `.pitofx` ([pfx-file.md](pfx-file.md#the-user-file)), the document pfx opens. `pfx scene migrate` imports a `.scene.toml` once into a `.pitofx` beside it, with every file it reads; the `.toml` is left as it was and never written again:

```sh
pfx scene migrate scenes/room.scene.toml
```

```text
imported scenes/room.scene.toml into scenes/room.pitofx
```

- `pfx scene migrate <folder>` imports every `*.scene.toml` and recipe in the folder and below it (skipping `target/`, `tmp/` and hidden folders), each into its own `.pitofx`; one imported before is kept and said so.
- `--dry-run` says what it would import and writes nothing.
- A `.pitofx` that already exists beside the file is refused: the import happens once, and edits go to the `.pitofx`. A `.pitofx` given to `pfx scene migrate` is refused too, since it is a document already.
- A recipe (`recipe = 1`) is imported the same way, with its includes and files; `--bare` keeps only the recipe and the files it reads, and the build comes the first time a command needs it.
- Every file of a scene starts with `format = 1` ([scene-format.md](scene-format.md#format)); a file that names no format, or an older one, is refused with `old-format` and the import writes nothing. A newer format than the crate knows is refused, naming the version it knows (`newer-format`).
- An HDRI the recipe's `[sky]` reads from inside the recipe's folder is not packed, because the `.pitofx` is written in that folder and reads it from the disk beside it. The import says so: `kept assets/sky/softbox.hdr beside the .pitofx, which reads it from the disk there`. Copy it with the `.pitofx` to render elsewhere: a `.pitofx` copied away from it refuses to render with `sky.path: …`. An HDRI outside the folder is packed, as every file from outside it is (`packed ../sky/softbox.hdr from outside scenes`). Saving the document elsewhere from the window packs it. `--dry-run` names the files it would import, not what each one keeps or packs.

Every other command given a `.toml` refuses with one line naming the import: `pfx: room.scene.toml is a plain TOML file; import it once with `pfx scene migrate room.scene.toml`, which writes room.pitofx beside it`.

After the import, `pfx scene print <x.pitofx>` shows the TOML inside it, `pfx scene write` replaces a file from stdin, and the editor edits it in its TOML panel or inspector.
