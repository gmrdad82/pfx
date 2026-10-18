# JetBrains Mono

- Upstream: https://github.com/JetBrains/JetBrainsMono
- Release: `v2.304`, asset `JetBrainsMono-2.304.zip` (https://github.com/JetBrains/JetBrainsMono/releases/download/v2.304/JetBrainsMono-2.304.zip, sha256 `6f6376c6ed2960ea8a963cd7387ec9d76e3f629125bc33d1fdcd7eb7012f7bbf`)
- Licence: SIL Open Font License 1.1, `OFL.txt` beside this note, from the same archive

| File | Taken from the archive | Upstream sha256 | Subset sha256 | Bytes |
|---|---|---|---|---|
| `JetBrainsMono-Regular.ttf` | `fonts/ttf/JetBrainsMono-Regular.ttf` | `a0bf60ef0f83c5ed4d7a75d45838548b1f6873372dfac88f71804491898d138f` | `874cdc4556e000b7e1179e69c5b6c2b1ea88f01657450090fdfdf5faa539d10f` | 273,900 to 177,844 |
| `JetBrainsMono-Bold.ttf` | `fonts/ttf/JetBrainsMono-Bold.ttf` | `5590990c82e097397517f275f430af4546e1c45cff408bde4255dad142479dcb` | `ed06711b6499323ac7b2b10479d4a331745419a4ea8dd837717724c5b4a42ec2` | 277,828 to 180,924 |
| `OFL.txt` | `OFL.txt` | `30f0c136e3c88e422d0791acd97238870f9054a9729bc34cf2ff0d4ed8cac4ad` | unchanged | |

Subset with `pfx font subset --font <upstream ttf> --out <ttf> --chars chars.txt`: Latin (U+0020 to U+024F), Greek, Cyrillic, general punctuation, super- and subscripts, currency, letterlike symbols, arrows, maths operators, technical symbols, box drawing and blocks, geometric shapes, miscellaneous symbols and dingbats, with the layout features' closure.
