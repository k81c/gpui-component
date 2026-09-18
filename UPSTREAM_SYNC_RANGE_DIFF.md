# Upstream sync range-diff record

## Compared ranges

- Old baseline: `20d65289d7e4d2573456b8ed907a73876d224bd1`
- Old fork tip: `0108a8b634db9e25d745d8c40f8ae526c45e33fc`
- New baseline: `7f6d92327936fbab7994a35c86328d793acc060d`
- New branch: `codex/sync-upstream-7f6d9232`

Run the patch-level comparison with:

```text
git range-diff --no-color 20d65289d7e4d2573456b8ed907a73876d224bd1..0108a8b634db9e25d745d8c40f8ae526c45e33fc 7f6d92327936fbab7994a35c86328d793acc060d..codex/sync-upstream-7f6d9232
```

## Recorded alignment

```text
 1: d060e763 !  1: f169f6a1 chore: restore fork metadata after upstream sync
 2: b27a8e03 !  2: 6c6dd344 feat: reapply marked editor and syntax extensions on upstream
 3: 3d77b07f =  3: 1a3d524e fix: preserve table rows around passthrough lines
 4: b2ff1f5d !  4: e56f2738 feat: integrate marked editor vertical presentation layout
 5: ef026a37 !  5: 923499da fix: complete marked editor parser parity
 6: 0108a8b6 =  6: f3d1e5ed docs: add marked editor app migration context
 -: -------- >  7: 52c18a92 fix: adapt marked editor to upstream inline tokens
 -: -------- >  8: 15757428 feat: add marked editor verification story
 -: -------- >  9: 55a7c944 fix: make marked editor fill relative height
 -: -------- > 10: 681d857c test: cover marked editor format contract
```

`=` entries remained patch-equivalent. `!` entries were deliberately adapted to the new upstream:

- Metadata keeps upstream ignore entries and updates repository guidance to the `base` / `component` / `kit` split.
- The editor export merge retains upstream `SyntaxContext`, atomic inline token modules, and language configuration while adding marked-document APIs.
- Vertical scrolling uses upstream wrapped-row and byte-offset handling, then applies the fork's per-line presentation map.
- Foreground, background, windowed, and injection parsing resolve parsers through `LanguageRegistry::parser()`, including factory-only languages.
- New commits cover atomic token geometry under variable line height, a dedicated Story, the preserved relative-height patch, and focused contract tests.

This file records the semantic alignment; the command above is authoritative for the complete patch-level range-diff.
