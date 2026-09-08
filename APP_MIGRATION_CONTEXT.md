# MarkedEditor 利用アプリ移行コンテキスト

この文書は、この fork を利用する GPUI アプリを同期後 API へ移行する担当者または
コーディングエージェントへ、そのままコンテキストとして渡すための資料です。公開
サイト向けのコンポーネント説明ではなく、移行時の判断基準、API 対応、検証条件を
一つに固定することを目的とします。

## 対象と確認済み基準

- upstream 基準: `longbridge/gpui-kit` の `20d65289d7e4d2573456b8ed907a73876d224bd1`
- fork ブランチ: `codex/sync-upstream-20d65289`
- 機能移植の確認基準コミット: `ef026a37` 以降
- 対象 package: `gpui-kit` / `gpui-component` 0.6.0
- 主対象: native の Markdown、Djot、AsciiDoc 編集画面

この確認はソース比較、Rust のユニットテスト、native Story と Story Web のビルドに
基づきます。実アプリのテーマ、ウィンドウ寸法、入力データを使った目視確認までは
このリポジトリ内では実施していません。専用の `MarkedEditor` Story もまだないため、
Story の成功はコンパイル互換の確認であり、対話・描画の証明ではありません。

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
| コードブロック背景 | active theme の `text.literal.block` を使用し、未定義時は muted color へ fallback |
| 表整形 | Markdown / Djot pipe table と AsciiDoc table、UTF-8 byte range 検証、単一 undo |
| rich `TextView` | upstream の code-block callback と highlighter identity cache を利用。旧 `text/node.rs` は復元していない |

意図的に移植していないものは、旧 API shim、旧 `InputState` 内の marked mode、旧 300 ms
遅延 task、移設前の Dock / Text / Input ファイルです。

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

次は確認基準コミットまでの機能を含む working tree で成功しています。

```text
cargo +nightly fmt --all -- --check
cargo +nightly check -p gpui-component --features tree-sitter-languages
cargo +nightly check -p gpui-kit --features tree-sitter-asciidoc,tree-sitter-djot
cargo +nightly check -p gpui-component-story
cargo +nightly check -p gpui-component-story-web
cargo +nightly test -p gpui-component --features tree-sitter-languages highlighter:: --lib
cargo +nightly test -p gpui-component --features tree-sitter-languages input::marked_editor --lib
cargo +nightly test -p gpui-component --features tree-sitter-languages input::table_format --lib
cargo +nightly test -p gpui-base vertical_layout --lib
cargo +nightly test -p gpui-base text_wrapper --lib
```

確認環境の stable `rustc 1.94.0` では、依存する `gpui-pre 0.3.2` が
`std::hint::cold_path` を使うため `E0658` で停止します。fork の今回の差分ではなく依存
toolchain の制約です。確認には `rustc 1.97.0-nightly` を使いました。

Story Web は build できますが、tree-sitter 無効構成では native と同じ構造解析を提供
しません。WASM を本番対象にする場合、表ボタンを必須要件にせず、WASM 向け parser
戦略を別途決めてください。

## 完了条件

利用アプリの移行は、Cargo feature、初期化、state 型、render、イベント購読、値の
読み書き、表整形の call site を新 API へ変更し、上のアプリ側受け入れ確認を終えた
時点で完了です。fork の `main` 付け替え、同期ブランチの push、利用アプリの依存 rev
更新は別作業であり、この文書だけでは実行済みとみなしません。
