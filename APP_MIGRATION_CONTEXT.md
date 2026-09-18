# MarkedEditor 利用アプリ移行コンテキスト

この文書は、この fork を利用する GPUI アプリを同期後 API へ移行する担当者または
コーディングエージェントへ、そのままコンテキストとして渡すための資料です。公開
サイト向けのコンポーネント説明ではなく、移行時の判断基準、API 対応、検証条件を
一つに固定することを目的とします。

## 対象と確認済み基準

- upstream 基準: `longbridge/gpui-kit` の `7f6d92327936fbab7994a35c86328d793acc060d`
- upstream HEAD 確認日: 2026-09-19（実装開始時に `upstream/main` を fetch 後に固定）
- fork ブランチ: `codex/sync-upstream-7f6d9232`
- 対象 package: `gpui-base` / `gpui-component` / `gpui-kit` 0.6.2
- 主対象: native の Markdown、Djot、AsciiDoc 編集画面

この確認はソース比較、Rust のユニットテスト、native Story と Story Web のビルドに
基づきます。専用の `MarkedEditor` Story は Markdown、Djot、AsciiDoc、editable / read-only、
atomic inline token、長い折り返し、表、コードブロック、高さ制約を収録します。実アプリの
テーマ、ウィンドウ寸法、入力データを使った目視確認は別途必要であり、Story のビルド成功
だけを対話・描画の証明とはみなしません。

## エージェント向け実行契約

利用アプリを変更するときは、次を守ってください。

1. 通常の一行入力や Textarea を一括置換しないでください。旧
   `InputState::marked_editor(...)` を使っていた構造化文書エディタだけが対象です。
2. `crates/ui`、旧 `input/element.rs`、旧 `text/node.rs` を参照または復元しないで
   ください。同期後の所有先は `crates/component` と `crates/base` です。
3. アプリは `Entity<MarkedEditorState>` を View に保持し、render ごとに state を
   作り直さないでください。イベント購読の `Subscription` も View に保持します。
4. `gpui-kit` を使う場合は `gpui_kit::init(cx)`、`gpui-component` を直接使う場合は
   `gpui_component::init(cx)` を、最初のコンポーネント生成より前に一度だけ呼びます。
5. 表範囲は独自に文字列検索せず、解析完了後の `table_ranges(cx)` を使います。
   返る range は UTF-8 byte range です。
6. 解析中に `table_ranges(cx)` が空になるのは正常です。空を「文書に表がない」という
   永続状態として保存しないでください。
7. fork 側に不具合があるという実測がない限り、利用アプリ内に二重の Tree-sitter
   parser、独自 debounce、独自見出しレイアウトを追加しないでください。

## Cargo 設定

upstream が推奨する統合 crate を使う例です。placeholder は公開済みの fork URL と
同期コミットへ置換してください。

```toml
[dependencies]
gpui-kit = { git = "<fork-repository-url>", rev = "<published-sync-commit>", features = [
    "tree-sitter-markdown",
    "tree-sitter-asciidoc",
    "tree-sitter-djot",
] }
```

組み込みの全言語を必要とする場合だけ、個別 feature の代わりに
`tree-sitter-languages` を使います。コードフェンス内の Rust なども強調する場合は、
対応する `tree-sitter-rust` などを追加してください。

`gpui-component` を直接依存するアプリでは、同じ feature 名をその dependency に指定
します。その場合、以下の `gpui_kit::component::...` は
`gpui_component::...` に読み替えます。

## 旧 API から新 API への対応

| 旧 fork API / 構造 | 同期後 API / 処理 |
| --- | --- |
| `InputState::new(...).marked_editor(language)` | `MarkedEditorState::with_language(language, window, cx)`、または options 付き `MarkedEditorState::new(...)` |
| `Entity<InputState>` を marked editor として保持 | `Entity<MarkedEditorState>` を保持 |
| `Input::new(&state)` で marked editor を描画 | `MarkedEditor::new(&state)` |
| `InputState::format_table_at(range, ...)` | `MarkedEditorState::format_table_at(range, ...)` |
| token 付き文書の復元 | `MarkedEditorState::set_content(InputContent, ...)` |
| marked editor の値・選択・検索・LSP API | `marked.read(cx).editor().clone()` で内部 `Entity<EditorState>` を取得して利用 |
| `InputState` からの `InputEvent` 購読 | 内部 `Entity<EditorState>` の `InputEvent` を購読 |
| 行頭検索による見出し・表判定 | `MarkedEditorState` が共有 Tree-sitter parser の snapshot を利用するため、アプリ実装は削除 |
| 独自の 300 ms parse task | 移植しない。共有 highlighter の 2 ms foreground / 150 ms debounce / background parse を利用 |

互換 shim として旧 `InputState::marked_editor(...)` を復活させないでください。通常の
`InputState`、`TextareaState`、`EditorState` は引き続き別の用途で有効です。

## 最小実装例

```rust
use gpui_kit::{
    Context, Entity, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, Window, div, px,
};
use gpui_kit::component::input::{
    InputEvent, MarkedEditor, MarkedEditorOptions, MarkedEditorState,
};

struct DocumentView {
    marked: Entity<MarkedEditorState>,
    source: SharedString,
    _subscriptions: Vec<Subscription>,
}

impl DocumentView {
    fn new(initial: impl Into<SharedString>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let initial = initial.into();
        let marked = cx.new(|cx| {
            MarkedEditorState::new(
                MarkedEditorOptions {
                    language: "markdown".into(),
                    readonly: false,
                    table_actions: true,
                },
                window,
                cx,
            )
        });
        marked.update(cx, |state, cx| {
            state.set_value(initial.clone(), window, cx);
        });

        let editor = marked.read(cx).editor().clone();
        let subscription = cx.subscribe(
            &editor,
            |this, editor, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.source = editor.read(cx).value();
                    cx.notify();
                }
            },
        );

        Self {
            marked,
            source: initial,
            _subscriptions: vec![subscription],
        }
    }
}

impl Render for DocumentView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(MarkedEditor::new(&self.marked).h(px(480.)))
    }
}
```

アプリ起動時の初期化は、View の生成より前です。

```rust
gpui_kit::application().run(|cx| {
    gpui_kit::init(cx);
    // この後で Window、Root、DocumentView を生成する。
});
```

最上位 View は従来どおり `gpui_kit::component::Root` で包んでください。

## 値と Editor API へのアクセス

`MarkedEditorState` は編集エンジンを複製せず、内部に upstream の `EditorState` を一つ
保持します。値の読み書きや検索などは次のように扱います。

```rust
let editor = marked.read(cx).editor().clone();
let current = editor.read(cx).value();

marked.update(cx, |state, cx| {
    state.set_value(new_source, window, cx);
});

editor.update(cx, |state, cx| {
    state.open_search(false, cx);
});
```

初期値も `MarkedEditorState::set_value` 経由で設定してください。内部 editor へ直接
`set_value` しても observer は追従しますが、facade 経由なら解析完了前の見出し表示も
即時更新されます。

atomic inline token を含む文書は `InputContent` を組み立て、`set_content` で復元します。
token の byte range は本文と一致し、UTF-8 文字境界上になければなりません。見出し行の
token もその行の拡大された line height で測定・配置されます。

## 表整形

`table_actions: true` かつ editable で解析が最新なら、`MarkedEditor` が解析木で認識した
表に Format ボタンを重ねて表示します。ボタン位置は editor の
`range_to_bounds` に基づきます。

プログラムから整形する場合も、解析済み range をそのまま渡します。

```rust
let range = marked.read(cx).table_ranges(cx).into_iter().next();
if let Some(range) = range {
    marked.update(cx, |state, cx| {
        let changed = state.format_table_at(range, window, cx);
        // changed == true のときだけ本文が変更された。
    });
}
```

`format_table_at` は、read-only、解析未完了、範囲外、UTF-8 文字境界外、不正な表、
変更不要の表では `false` を返します。成功時の置換は一つの undo 操作です。

## 移植済み動作

監査では、旧 fork の独自コミット群を共通祖先から意味単位で追い、次を現在の所有先
で確認しました。

| 機能 | 同期後の実装 |
| --- | --- |
| AsciiDoc / AsciiDoc Inline / Djot | component Cargo features、言語 alias、grammar、highlight / injection query。`gpui-kit` にも個別 feature を転送 |
| foreground parse 上限 | 2 ms。256 KiB 超は foreground の full parse を避ける |
| 遅延解析 | 150 ms debounce の background full parse。新しい編集で旧 task を cancel し、古い text snapshot の結果は適用しない |
| 大規模文書 | 600 行以上では編集位置の前後 300 行を空行境界へ広げた windowed parse を先行し、full parse 完了後に置換 |
| injection | background 計算、同一言語・同一 included range の tree 再利用、range / byte / parse 数上限、cancel 対応 |
| highlight cache | text/tree revision と byte range を key に theme 非依存の match 結果を再利用 |
| 構造 snapshot | 同じ Tree-sitter parse から見出し、表、コードブロック range を取得。コードブロック内の見かけ上の見出し・表は除外 |
| 見出し表示 | Markdown / Djot / AsciiDoc の heading level に応じた font size、line height、前後 spacing |
| 可変行高 | scroll extent、visible range、caret、selection、hit testing、IME、fold icon、背景、行番号、indent guide が同じ vertical map を使用 |
| atomic inline token | 行ごとの font size / line height / spacing で測定・配置・wrap・hit testing。UTF-8 編集と undo は upstream の token 履歴を維持 |
| コードブロック背景 | active theme の `text.literal.block` を使用し、未定義時は muted color へ fallback |
| 表整形 | Markdown / Djot pipe table と AsciiDoc table、UTF-8 byte range 検証、単一 undo |
| rich `TextView` | upstream の code-block callback と highlighter identity cache を利用。旧 `text/node.rs` は復元していない |

意図的に移植していないものは、旧 API shim、旧 `InputState` 内の marked mode、旧 300 ms
遅延 task、移設前の Dock / Text / Input ファイルです。

## 旧コミットから同期コミットへの対応

旧 6 コミットを一対一で機械的に再生せず、最新 upstream 上で意味単位に再構成しました。

| 旧 fork コミット | 同期ブランチの対応 | 内容 |
| --- | --- | --- |
| `d060e763` | `f169f6a1` | fork metadata と ignore 設定 |
| `b27a8e03` | `6c6dd344` | MarkedEditor、言語 feature、grammar / query |
| `3d77b07f` | `1a3d524e` | table passthrough 行保持 |
| `b2ff1f5d` | `e56f2738` | 可変行高と editor vertical presentation |
| `ef026a37` | `923499da` | parser parity、snapshot、非同期解析 |
| `0108a8b6` | `f3d1e5ed` | 利用アプリ移行コンテキスト |
| 元 worktree の未コミット高さ修正 | `55a7c944` | `MarkedEditor` 内部 `Editor` の相対高さ 100% |

最新 upstream の atomic inline token との統合は `52c18a92`、専用 Story は `15757428`
として追加しました。対応の再確認には次を使います。

```text
git range-diff 20d65289d7e4d2573456b8ed907a73876d224bd1..0108a8b634db9e25d745d8c40f8ae526c45e33fc 7f6d92327936fbab7994a35c86328d793acc060d..codex/sync-upstream-7f6d9232
```

## アプリ側の探索手順

まず利用アプリで次を検索します。

```text
marked_editor(
format_table_at(
Input::new(
Entity<InputState>
gpui_component::init
gpui_kit::init
tree-sitter-markdown
```

`Input::new` と `Entity<InputState>` の全件を変更してはいけません。
`marked_editor(...)` へ到達する state と、その state の値・イベント・表整形を扱う call
site だけを一つの移行単位にします。

## アプリ側の受け入れ確認

- Markdown、Djot、AsciiDoc を一つずつ開き、見出し倍率とコードブロック背景を確認する。
- コードブロック内の `# fake heading` と `| fake | table |` に見出し倍率や表ボタンが
  出ないことを確認する。
- 日本語を含む表を整形し、1 回の undo で元へ戻ることを確認する。
- read-only では入力と表ボタンが無効で、選択・コピー・検索は使えることを確認する。
- 長文を連続編集し、古い highlight、古い fold、遅れて戻る表ボタンが残らないことを
  確認する。
- 見出しの前後で折り返し、選択、上下移動、マウス hit testing、IME 候補位置、scroll
  が同じ行位置を使うことを確認する。
- ライト / ダークテーマ切替後にコードブロック背景が更新されることを確認する。

## リポジトリ側の検証記録

確認環境は Windows x86_64、`rustc 1.97.0-nightly (ad3a598ca 2026-05-03)`、
`cargo 1.97.0-nightly (4f9b52075 2026-05-01)` です。lockfile は固定 upstream 版を種にし、
AsciiDoc / AsciiDoc Inline / Djot の grammar 解決後はすべて `--locked` で実行しました。

成功:

```text
cargo +nightly fmt --all -- --check
cargo +nightly check -p gpui-component --features tree-sitter-languages --locked
cargo +nightly check -p gpui-component --no-default-features --locked
cargo +nightly check -p gpui-kit --features tree-sitter-asciidoc,tree-sitter-djot --locked
cargo +nightly check -p gpui-component-story --locked
cargo +nightly test -p gpui-component --features tree-sitter-languages highlighter:: --lib --locked
cargo +nightly test -p gpui-component --features tree-sitter-languages input::marked_editor --lib --locked
cargo +nightly test -p gpui-component --features tree-sitter-languages input::table_format --lib --locked
cargo +nightly test -p gpui-base inline_token --lib --locked
cargo +nightly test -p gpui-base vertical_layout --lib --locked
cargo +nightly test -p gpui-base text_wrapper --lib --locked
cargo +nightly test -p gpui-base test_auto_close --lib --locked
cargo +nightly test -p gpui-base smart_indent --lib --locked
cargo +nightly test -p gpui-base search_ --lib --locked
cargo +nightly test -p gpui-base multi_cursor --lib --locked
git diff --check
```

対象テストの結果は highlighter 26、MarkedEditor 3、table format 3、inline token 6、
vertical layout 1、text wrapper 13、auto-close 8、smart-indent 3、search 7、multi-cursor 22 の
全件成功です。`Cargo.lock` の固定 upstream との差分は上記 3 grammar と
`gpui-component` の dependency entry だけで、無関係な更新はありません。

未検証または成功扱いにしない項目:

- `cargo +nightly check -p gpui-component-story-web --target wasm32-unknown-unknown --locked`
  は nightly toolchain に `wasm32-unknown-unknown` target が未導入のため `E0463` で停止。
- native Story の目視確認（wrap、上下移動、選択、pointer hit testing、IME、scroll、
  テーマ切替、相対高さ）は、この実行環境から native window を観察できないため未実施。
- プロジェクト方針どおり全 workspace test suite は実行していない。

確認環境の stable `rustc 1.94.0` では、依存する `gpui-pre 0.3.5` が
`std::hint::cold_path` を使うため `E0658` で停止します。fork の今回の差分ではなく依存
toolchain の制約です。

Story Web は tree-sitter 無効構成では native と同じ構造解析を提供しません。WASM を
本番対象にする場合、target 導入後にコンパイルを再確認し、表ボタンを必須要件にせず、
WASM 向け parser 戦略を別途決めてください。

## 完了条件

利用アプリの移行は、Cargo feature、初期化、state 型、render、イベント購読、値の
読み書き、表整形の call site を新 API へ変更し、上のアプリ側受け入れ確認を終えた
時点で完了です。fork の `main` 付け替え、同期ブランチの push、利用アプリの依存 rev
更新は別作業であり、この文書だけでは実行済みとみなしません。
