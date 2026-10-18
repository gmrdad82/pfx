# Icons

The editor's eighteen icon glyphs (U+F002 to U+F201, listed in `chars.txt`), cut from the JetBrains Mono Nerd Font.

- Taken from: Nerd Fonts 3.5.1 (https://github.com/ryanoasis/nerd-fonts, tag `v3.5.1`), release asset `JetBrainsMono.tar.xz` (https://github.com/ryanoasis/nerd-fonts/releases/download/v3.5.1/JetBrainsMono.tar.xz, sha256 `04d5e8f903693f9dd13e16f867e994834e681eb3c72c0d337a770dcda09010cf`), file `JetBrainsMonoNerdFont-Regular.ttf` (sha256 `1c680e8cde9fcf8b88a5605ce8d1fb94dd3fb15841f7ca7bf4c55664855e5611`, 2,571,596 bytes): JetBrains Mono patched with the Nerd Fonts glyphs.
- The glyphs are Font Awesome Free's (https://fontawesome.com), as Nerd Fonts carries them in `src/glyphs/font-awesome/`.
- Licences: the patched font is under the SIL Open Font License 1.1 (`OFL.txt`, JetBrains Mono's); Font Awesome Free's fonts are under the SIL Open Font License 1.1 with the Reserved Font Name "Font Awesome" (`FontAwesome-LICENSE.txt`, from nerd-fonts `v3.5.1`, `src/glyphs/font-awesome/LICENSE.txt`). The subset keeps the patched font's own names and does not use the reserved name.

| File | sha256 | Bytes |
|---|---|---|
| `Icons.ttf` | `dc331e056a04ed8e804a670609d034ea0aad9ef94a7e361f7b4a58a494d381b3` | 7,836 |
| `OFL.txt` | `30f0c136e3c88e422d0791acd97238870f9054a9729bc34cf2ff0d4ed8cac4ad` | |
| `FontAwesome-LICENSE.txt` | `de784a808496d49f5f80dd9d1d6e9a51cff712cfbc339e7c25155631ae3cdb5d` | |

Subset with `pfx font subset --font JetBrainsMonoNerdFont-Regular.ttf --out Icons.ttf --chars chars.txt --no-closure`. The theme installs it last in every family, so the icons draw beside any face.
