# Contributing to pfx

Issues and pull requests are welcome at [github.com/gmrdad82/pfx](https://github.com/gmrdad82/pfx). Everyone taking part follows the [code of conduct](CODE_OF_CONDUCT.md). Report a vulnerability privately, as [SECURITY.md](SECURITY.md) says, never in a public issue.

pfx: got questions or doubts? [Ping me on Discord](https://discord.gg/XsNcn3kYzn)

## The Contributor Licence Agreement

pfx is offered under the AGPL-3.0 and under a commercial licence ([README](README.md#licence)). So that both stay possible, a pull request is merged only once its author has signed the [pfx Individual Contributor Licence Agreement](CLA.md). You keep the copyright in your work; the agreement lets Cătălin Ilinca license it as part of pfx under any licence, and always under the licence pfx uses on the day you submit it. It is adapted from the Harmony Individual Contributor License Agreement 1.0; read it before you sign.

Sign it once, in your first pull request:

1. Write this statement in the pull request's description, with your full legal name, your GitHub account and the date:

   > I have read the pfx Individual Contributor Licence Agreement, version 1.0, in CLA.md, and I agree to its terms for this Contribution and for every Contribution I Submit to pfx from now on.
   >
   > Name: <full legal name>
   > GitHub: <account>
   > Date: <YYYY-MM-DD>

2. Add one row to the table in [CLA-SIGNERS.md](CLA-SIGNERS.md), in the same pull request: your name, your GitHub account, the date and the agreement's version.

Later pull requests need neither. If you contribute for an employer, have them approve the agreement first (CLA.md, Section 3(c)).

## What a change needs

- **The fast gate green:** `bin/gate --fast` (formatting, clippy with warnings as errors, and the unit tests of the crates the branch changed). GPU tests are `#[ignore]`d; run the ones your change touches with `cargo test -p <crate> -- --ignored <filter>`.
- **The engine stays generic:** no game's or app's names, values, assets or fonts enter it ([docs/agnostic.md](docs/agnostic.md)); tests use the engine's own neutral fixtures, listed in `tests/agnostic/fixtures.txt`.
- **Only permissive code from elsewhere:** code you port or copy comes from MIT, Apache-2.0, BSD, ISC or zlib sources, media from CC0 and fonts under the SIL Open Font License; never GPL, LGPL, AGPL or other copyleft code. Credit each origin with its version or commit in [INSPIRATION.md](INSPIRATION.md), and its copyright and licence text in [NOTICE.md](NOTICE.md) where its licence asks.
- **Dependencies under permissive licences:** `cargo test --test licences` checks every dependency's licence.
- **No comments in code;** match the surrounding style.
- **A pull request message that says what pfx can now do,** in a plain sentence.
