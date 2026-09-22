# Upstream sync 保守ガイド

このディレクトリは、`longbridge/gpui-kit` から分岐した本 fork の独自仕様を、次回の
upstream sync で意味単位に移植するための正本です。アプリ利用者向けの移行例は
リポジトリ直下の `APP_MIGRATION_CONTEXT.md` にありますが、実装の差分判定では本書群を
優先してください。

## 固定した比較対象

| 記号 | commit | 用途 |
| --- | --- | --- |
| B | `7f6d92327936fbab7994a35c86328d793acc060d` | 直近同期時の upstream 基点 |
| F | `d12bb0f9fa551ae41cab65c3ea08d3649102a727` | この文書が説明する fork 実装 |

監査時点では `F` が `HEAD`、作業ツリーは clean、`git merge-base B F` は `B` だった。
範囲 `B..F` は 12 commit、87 files、112,055 insertions、283 deletions である。この行数の
大半は追跡済みの VitePress cache と参考用 Djot 文書であり、Rust 実装の規模を表さない。

比較を再現するときは可動ブランチ名を使わない。

```text
git rev-parse B^{commit} F^{commit}
git merge-base B F
git diff --name-status -M B F
git diff --numstat B F
git log --reverse --oneline B..F
git diff --no-ext-diff --no-textconv --no-color --find-renames=50% --diff-algorithm=myers --unified=3 B F -- <path>
```

上の `B` と `F` は実行時に表の full SHA へ置き換える。ソース根拠は
`git show F:path/to/file` で取得できる。GitHub に commit が存在することは前提にしない。

## 文書の使い分け

- [DIFFERENCES.md](DIFFERENCES.md): `FORK-001` から `FORK-009` までの仕様契約、依存関係、
  実装箇所、移植方法、検証方法。
- [INVENTORY.md](INVENTORY.md): 87差分ファイルの全件台帳、実装ファイルの変更範囲、
  12 commit の最終仕様への対応。
- この文書: 次回 sync の作業順、判定ルール、固定 `F` に対する検証記録。

## 次回 sync の実行手順

### 1. 三つの commit を固定する

既存の `B` と `F` に加え、新しい upstream commit を `U` として full SHA で記録する。
fetch 直後のブランチ名や `HEAD` を後続コマンドへ埋め込まない。作業ツリーが `F` と
異なる状態で `F` の回帰試験を行う場合は、`F` の隔離 worktree を作り、試験記録へ
実際の checkout SHA と clean 状態を残す。

### 2. 差分 ID ごとに判定する

各 `FORK-*` を次のいずれかへ分類し、`U` のソース、テスト、release note の根拠を付ける。

| 判定 | 条件 |
| --- | --- |
| 保持 | upstream に同等の契約がなく、fork の利用要件が存続する |
| 適応 | upstream に統合点はあるが、fork の契約を満たす変更が必要 |
| 吸収済み | 公開 API だけでなく境界・失敗動作・テストまで同等である |
| 廃止 | 利用要件を取り下げ、互換性への影響を明示した |

名前やコード断片が似ているだけでは「吸収済み」にしない。既存 API へ移した場合は
呼び出し側の置換方法も記録する。

### 3. 依存順に移植する

1. `FORK-001`: grammar dependency、Cargo feature、lockfile、言語 registry。
2. `FORK-003`: `gpui-base` の snapshot、revision、presentation の契約。
3. `FORK-002`: foreground、windowed、background、injection parse と世代管理。
4. `FORK-004`: 可変行高、wrap/fold、token、caret/selection/IME/scroll の共通 geometry。
5. `FORK-005`・`FORK-006`・`FORK-007`: MarkedEditor、表操作、theme。
6. `FORK-008`: Story と自動テスト。

ファイルパスではなく、[DIFFERENCES.md](DIFFERENCES.md) に記載した責務とシンボルから
新しい統合先を探す。旧 `crates/ui` や旧 input ファイルをディレクトリごと復元しない。

### 4. 検証して台帳を更新する

差分 ID ごとにテストと手動シナリオを実行し、結果を commit・OS・toolchain・feature と
ともに記録する。`git range-diff` は commit 対応の補助に使えるが、パッチ一致だけで
仕様維持と判定しない。完了時に `B`、`F`、新しい移植後 commit、各 ID の判定、未解決
事項を三文書すべてへ反映する。

## 固定 F の検証記録

実行環境は Windows x86_64、`rustc 1.97.0-nightly (ad3a598ca 2026-05-03)`、
`cargo 1.97.0-nightly (4f9b52075 2026-05-01)`。対象 checkout は `F`、作業ツリーは試験開始時
clean。依存取得を発生させないため `--offline --locked` を付けた。

### 成功した unit test

| command の test filter | 件数 | 対応 |
| --- | ---: | --- |
| `cargo +nightly test -p gpui-component --features tree-sitter-languages --lib --locked --offline highlighter::` | 28 | FORK-001, 002, 007 |
| 同上 `input::marked_editor` | 7 | FORK-002, 005, 006 |
| 同上 `input::table_format` | 3 | FORK-006 |
| `cargo +nightly test -p gpui-base --lib --locked --offline test_document_revision` | 1 | FORK-003 |
| 同上 `test_presentation_` | 2 | FORK-003, 004 |
| 同上 `inline_token` | 6 | FORK-004 |
| 同上 `vertical_layout` | 1 | FORK-004 |
| 同上 `text_wrapper::tests` | 13 | FORK-004 |
| 同上 `fold_map::tests` | 2 | FORK-004 |
| 同上 `test_auto_close` / `smart_indent` / `search_` / `multi_cursor` | 8 / 3 / 7 / 22 | 共通 editor 回帰 |

すべて0件ではないことと、`0 failed` を確認した。Windows linker の情報 warning は出たが、
テスト失敗ではない。

### 成功した構成確認

以下はそれぞれ独立した invocation とした。特に kit の3言語を同時に有効化せず、feature
unification で転送漏れが隠れないようにした。

```text
cargo +nightly check -p gpui-component --features tree-sitter-languages --locked --offline
cargo +nightly check -p gpui-component --no-default-features --locked --offline
cargo +nightly check -p gpui-kit --no-default-features --features tree-sitter-asciidoc --locked --offline
cargo +nightly check -p gpui-kit --no-default-features --features tree-sitter-asciidoc-inline --locked --offline
cargo +nightly check -p gpui-kit --no-default-features --features tree-sitter-djot --locked --offline
cargo +nightly check -p gpui-component-story --locked --offline
```

6構成すべて成功した。Story check は2分53秒で完了したが、これは native UI の操作・描画を
目視検証した結果ではない。

### 未検証を成功扱いしない項目

- native Story の目視操作。ビルド成功は caret、selection、pointer、IME、scroll、theme
  切替の視覚的な正しさを証明しない。
- `wasm32-unknown-unknown`。この nightly には target が導入されていない。なお
  `--no-default-features` は Tree-sitter 無効構成の確認であり、WASM の確認ではない。
- Linux、macOS と Metal rendering test。
- 解析時間、描画時間、cache hit rate の性能測定。ソース中の timeout や cache は仕様上の
  上限・無効化規則であり、実測性能値ではない。

WASM を確認する場合は target を用意し、次を固定 `F` の隔離 checkout で実行する。

```text
cargo +nightly check -p gpui-component-story-web --target wasm32-unknown-unknown --locked
```

### native 手動確認チェックリスト

固定した入力、ウィンドウ寸法、light/dark theme を記録して次を確認する。

- Markdown、Djot、AsciiDoc の見出し level ごとに倍率・前後 spacing が変わる。
- code block 内の `# fake heading` と `| fake | table |` が見出し・表操作として扱われない。
- 日本語を含む pipe table と AsciiDoc table を整形し、一回の undo で完全に戻る。CRLF を
  保持する要件があるアプリでは、F が LF へ正規化する現状を受容できるか別に判定する。
- read-only では入力と Format button が無効で、selection、copy、search は利用できる。
- 600行以上の文書を連続編集し、古い highlight、fold、heading metrics、Format button が
  遅れて復活しない。
- 拡大見出しの前後で soft wrap、上下移動、multi-cursor、pointer hit、selection、IME候補、
  fold icon、scroll anchor が同じ行位置を使う。
- light/dark 切替で code-block 背景が更新され、scroll位置と行高は変わらない。
- Tree-sitter 無効構成では fallback の見出し/fence表示だけが働き、table action は出ない。

## 完了判定

次回 sync は、INVENTORY の全ファイルが分類済みで、各移植対象が差分 ID、実装根拠、
移植判定、検証結果へ双方向に辿れ、未検証事項に再現手順があるとき完了とする。失敗や
未実施は作業を隠す理由にせず、結果と次の確認方法を記録する。
