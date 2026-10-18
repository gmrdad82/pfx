# Asset sources

`pfx asset` finds and fetches HDRIs, textures, models and model packs from public CC0 sources and from accounts you configure, checks every download, refuses licences you have not allowed, and records each asset's licence facts in a `.pitofx` beside its files. `pfx credits` then lists the credits of every asset a scene uses.

The crate behind it is `pfx-source`: the `Source` trait, the sources, the HTTPS client, the zip reader, the licence policy, the settings and the credits, for an app or an editor to link on its own.

## Commands

```
pfx asset search [words...] [--source <name>] [--kind hdri|texture|model|pack] [--limit N] [--allow <kind>]... [--json]
pfx asset get <source:id> [--out <dir>] [--res <r>] [--format <f>] [--from <file.zip>] [--allow nc|nd|editorial|custom]... [--json]
pfx asset list [--in <dir>] [--json]
pfx asset sources [--json]
pfx asset settings [--stdin] [--json]
pfx credits <scene.pitofx> [--json]
```

## Sources

| Source | What | How |
|---|---|---|
| `polyhaven` | HDRIs, textures and models, CC0 | Poly Haven's public API; no key. Every request carries the user agent `pfx/<version>`, and content listed from the live API shows "Powered by Poly Haven", as its API terms ask. |
| `ambientcg` | materials, HDRIs and models, CC0 | ambientCG's API v3; no key. Downloads are zips, unpacked into the asset's folder. |
| `kenney` | model packs, CC0 | A built-in catalog: each pack's download URL, size and sha256 are pinned in pfx, so a pack that changed upstream is refused rather than taken. |
| `quaternius` | model packs, CC0 | A built-in catalog of pack pages. Their downloads come from a shared cloud folder with no stable file URL or checksum, so pfx cannot script them: download the pack by hand from the page `pfx asset search --source quaternius` lists, then `pfx asset get quaternius:<pack> --from <file.zip>`. |
| an account | Sketchfab's downloadable models | Configured in pfx's settings, below. Searching needs no token; the download link asks for the account's API token, read at run time. |

`pfx asset search` with no `--source` asks every source; a catalog lists every pack when no words are given. A result whose licence your settings would refuse is still listed, marked with the `--allow` it needs.

## Get an asset

```
pfx asset get polyhaven:<id> --res 2k
pfx asset get ambientcg:<id> --format png
pfx asset get kenney:nature-kit --out tmp/packs/nature
```

Without `--out` the asset goes to pfx's cache, under `assets/<source>/<id>/`, where `pfx asset list` finds it and code finds it by reference (`pfx_source::fetch::locate(cache, "polyhaven:<id>")`). With `--out` the folder you name is the asset's folder.

Every file is downloaded to a temporary name beside its place, checked, and only then renamed into place, so a broken download never leaves a half file: the size and MD5 Poly Haven lists, the size ambientCG lists, the pinned sha256 and size of a catalog pack. A zip is unpacked entry by entry, each entry's CRC-32 checked; an entry whose path would leave the asset's folder refuses the whole archive. Fetching an asset that is already there and whose files still match their sha256 downloads nothing.

The defaults: HDRIs as `.hdr` at 1k, textures as each map's `.jpg` at 1k, Poly Haven models as split `.gltf` with their buffer and textures at their relative paths, ambientCG as the 1K JPG zip, account models as `.glb` (a `.gltf` zip when no `.glb` is offered). `--res` and `--format` pick others; asking for what a source does not offer names what it does.

## Licences

Every asset carries an SPDX licence id, checked against pfx's SPDX table. Licences that are non-commercial (`nc`, the `-NC` Creative Commons licences), no-derivatives (`nd`, the `-ND` ones), editorial (`editorial`, `LicenseRef-Editorial`) or a custom `LicenseRef-` (`custom`) are refused unless you pass `--allow <kind>` or allow the kind in your settings. An `OR` expression is taken when any side is allowed; an `AND` needs every id allowed. A licence is refused before anything is downloaded and before an account's token is read.

## The record

Each fetched asset's folder holds its files and `asset.pitofx`, a `.pitofx` whose FACT chunk is TOML:

```toml
licence = "CC0-1.0"
credit = "\"<name>\" by <author> (<page>) is licensed under CC0 1.0"
author = "<author>"
source = "<page>"

[asset]
from = "polyhaven"
id = "<id>"
name = "<name>"
kind = "hdri"
pfx = "<pfx version>"

[files]
"<id>_1k.hdr" = "<sha256>"
```

`licence`, `credit`, `author` and `source` sit at the top as they do in an imported model's `.pitofx`, so both read alike. A zip's own name and sha256 are kept as `archive` and `archive_sha256`. The credit line names the title, the author, the source and the licence; ambientCG's is the line its licence page suggests.

## Credits

```
pfx credits scene.pitofx
pfx credits scene.pitofx --json
```

`pfx scene save` packs every file a recipe reads into the scene's `.pitofx`, and with them the `asset.pitofx` of each fetched asset whose files it reads (found in the file's folder or up to three folders above it). `pfx credits` reads the scene's `.pitofx` and lists one credit line per asset, ready for a credits card: each fetched asset's record, each embedded model `.pitofx` with its licence facts, and each library object's licence from the scene's facts. The same credit used by several files or objects is listed once; `--json` gives each one's name, licence, credit, author, source, `from` (its `source:id`) and what in the scene uses it. pfx's own original library objects need no credit and are left out.

An asset that records no licence, or a licence that is not an SPDX expression, is refused by name, one line each on stderr, and the command exits 1 without printing credits.

## Settings and accounts

pfx's own settings are a `.pitofx` (`settings.pitofx`, a CONF chunk of TOML) in pfx's settings folder: `PFX_SETTINGS` names the file, else `PFX_CONFIG` the folder, else `$XDG_CONFIG_HOME/pfx` or `~/.config/pfx` on Linux, `~/Library/Application Support/pfx` on macOS and `%APPDATA%\pfx` on Windows. `pfx asset settings` prints them as TOML; `pfx asset settings --stdin` reads TOML on stdin, checks it and writes the `.pitofx`.

```toml
[licences]
allow = ["nc"]
only = ["CC0-1.0", "CC-BY-4.0", "CC-BY-NC-4.0"]

[[account]]
name = "models"
kind = "sketchfab"
token_env = "MODELS_API_TOKEN"
```

`allow` unlocks licence kinds for every command; `only`, when given, takes only the licences it lists. An account names its token by an environment variable (`token_env`) or a command that prints it on its first line (`token_cmd = ["<program>", "<arg>", ...]`, run without a shell). pfx reads the token only when it downloads, sends it only on the download-link request, and never stores, logs or prints it; a settings file with a token in it is refused. An account's name is its source name, as `pfx asset get models:<model id>`.
