# Jost

- Upstream: https://github.com/indestructible-type/Jost at `35f141c970538f1ed0f235789c19156b3ce2a762`, as Google Fonts ships it: https://github.com/google/fonts, `ofl/jost/`, commit `6e4b84c976cadb3c49a40fd9a1c203e4f7fcf2da`
- Font: the variable `Jost[wght].ttf`, one `wght` axis from 100 to 900 (default 400), designed by Owen Earl
- Licence: SIL Open Font License 1.1, no Reserved Font Name, `OFL.txt` beside this note, from the same commit

| File | Taken from | Upstream sha256 | Subset sha256 | Bytes |
|---|---|---|---|---|
| `Jost[wght].ttf` | `ofl/jost/Jost[wght].ttf` | `6343b70971000b04c5d401c96ae08ce371086135e999d5e1e1413039c0213076` | `305a90a02e14c4868aff887b657aadb2e952678a6708ac8775aa7eed48d272c8` | 134,996 to 86,208 |
| `OFL.txt` | `ofl/jost/OFL.txt` | `1af3438a4d5f0ed2bdbc5751a5a67ebf6d537334161184b7fbb68503ef0ea0c5` | unchanged | |

Subset with `pfx font subset --font <upstream ttf> --out <ttf> --chars chars.txt`: Basic Latin, Latin-1, Latin Extended-A, dashes, quotes and the ellipsis, four arrows, and the euro, trade mark and minus signs. The variation axis and its tables stay; the theme reads the font at two weights. Text outside the subset falls back to JetBrains Mono.
