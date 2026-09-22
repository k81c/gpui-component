# Fork 仕様差分

この文書の「upstream」は基点 `B=7f6d92327936fbab7994a35c86328d793acc060d`、
「fork」は `F=d12bb0f9fa551ae41cab65c3ea08d3649102a727` を指す。行番号は F のファイルに
対する目安であり、根拠の取得には `git show F:path` を使う。

## 差分 ID と依存関係

| ID | 契約 | 依存 |
| --- | --- | --- |
| FORK-001 | AsciiDoc／AsciiDoc Inline／Djot の grammar と feature | なし |
| FORK-002 | 共有 Tree-sitter parser の非同期更新と構造 snapshot | FORK-001, 003 |
| FORK-003 | parser 非依存の base editor 公開 seam | なし |
| FORK-004 | 可変行高を全 editor geometry に適用 | FORK-003 |
| FORK-005 | `MarkedEditor` facade と表示状態 | FORK-002, 004, 007 |
| FORK-006 | 構文木で検出した表の安全な整形 | FORK-002, 005 |
| FORK-007 | code block の semantic background theme | FORK-001, 003 |
| FORK-008 | native Story と focused regression tests | FORK-001〜007 |
| FORK-009 | fork の開発補助資料・生成物 | runtime 依存なし |

## FORK-001: 言語 grammar と feature

### 基点との差

B の組み込み言語集合に AsciiDoc、AsciiDoc Inline、Djot はない。F は
`gpui-component` に次を追加し、`tree-sitter-languages` に AsciiDoc と Djot を含め、
`gpui-kit` から同名 feature を転送する。

| feature | 有効になる grammar | lockfile |
| --- | --- | --- |
| `tree-sitter-asciidoc` | AsciiDoc と AsciiDoc Inline | crates.io `0.7.0` / `0.7.0` |
| `tree-sitter-asciidoc-inline` | AsciiDoc Inline のみ | crates.io `0.7.0` |
| `tree-sitter-djot` | Djot | Git `759a61896ccb2200a4becec4443e768638a21d58`、crate version `2.0.0` |

`Language::AsciiDoc` の正規名は `asciidoc`、`AsciiDocInline` は `asciidoc_inline`、
`Djot` は `djot`。AsciiDoc は injection query で各 `line` を
`asciidoc_inline` の combined injection として解析する。highlight query は AsciiDoc の
title/listing/literal、inline の emphasis/monospace/link、Djot の heading/emphasis/code/link
の一部を semantic capture へ写す。これは文法全体のレンダラーではない。

### 移植契約

- component manifest、kit の feature forwarding、lockfile、`Language` の列挙・名前・
  grammar config、4 query ファイルを一組で移植する。
- `Language::from_name` は feature が無効なら該当言語を返さない。この feature gating を
  alias の常時登録へ変えない。
- AsciiDoc 本体 feature は inline grammar も有効化する。inline 単独 feature は本体を
  有効化しない。
- grammar 更新時は query の node 名を再検証する。`marked_language_queries_are_valid` が
  grammar ごとに query construction を通すことを確認する。

### 根拠と検証

- `crates/component/Cargo.toml:24-74,148-158`
- `crates/kit/Cargo.toml:29-43`
- `Cargo.lock` の3 package entry
- `crates/component/src/highlighter/languages.rs:5-137,202-304,401-460,704-790`
- `crates/component/src/highlighter/languages/{asciidoc,asciidoc_inline,djot}`
- 検証: component 全言語 check、kit の3 feature の独立 check、highlighter test 28件。

## FORK-002: 非同期解析と構造 snapshot

### 基点との差

B の syntax highlighter に、MarkedEditor が同じ parse tree を共有する構造 snapshot と、
大きい文書・injection に対する更新段階を追加した。snapshot は行ごとの heading level、
table の UTF-8 byte range、code block の byte range を持つ。コードブロック内に見える
`# heading` や pipe table は構造 node から除外される。

更新順序は次のとおり。

1. 更新ごとに generation を増やし、既存 snapshot を外して、Markdown／Djot／AsciiDoc
   だけ `snapshot_pending=true` にする。
2. 256 KiB 以下は foreground full parse を最大2 ms実行する。完了時も構造 snapshot
   の走査は background task へ渡す。256 KiB 超は foreground full parse を行わない。
3. foreground が timeout した大規模文書では、600行以上に限り編集行と viewport の
   前後300行を空行境界へ広げた windowed parse を先行する。
4. foreground が injection 待ちなら、150 ms debounce 後に既に得た full tree を再利用して
   fold と injection を進める。timeout なら同じ debounce 後に background full parse を行い、
   fold と必要な構造 snapshot を計算して適用する。foreground が complete ならこの
   debounced task は作らない。
5. full tree 適用後、injection を background で計算し、text と full-tree revision が
   一致する場合だけ適用する。

task drop による cancel flag、generation、text equality、tree revision の全部が stale
result の防壁になる。新しい request の generation と異なる window/full/injection 結果は
適用しない。失敗・cancel 時は pending flag を下ろして通知するが、古い snapshot を
current として復活させない。

injection は最大4096 ranges、合計512 KiB、動的言語名64 bytes、non-combined parse 512、
各 injection parse 20 msを上限とする。上限は処理量の制御であり、応答時間の実測値では
ない。match cache は highlight revision と byte range を key に theme 非依存の capture
結果を再利用し、theme 解決は `styles` 呼び出し時に行う。

### 移植契約

- `InputHighlighter::document_snapshot` と `document_snapshot_pending` を通して base を
  Tree-sitter 型へ依存させない。
- foreground tree、window tree、full tree、injection layer を別の鮮度として扱う。
  window tree を full tree revision の代わりにしない。
- parser は静的 enum から直接生成せず `LanguageRegistry::parser()` で取得し、factory-only
  language も同じ経路に通す。
- snapshot の走査と injection parse でも cancel を検査する。新しい編集後に古い fold、
  table range、injection highlight を適用しない。

`tree-sitter` feature が無効な構成では `input_highlighter_factory()` は常に `None` を返す。
このとき highlighter は ready 扱いだが snapshot はなく pending でもないため、MarkedEditor
は行頭 marker と fence の fallback 表示を使い、構造 table range は公開しない。
`wasm_stub.rs` の `MarkedSyntaxSnapshot` は無効構成で module の型を成立させる内部 stub で、
WASM に parser 機能を与えない。native の no-feature と WASM は同じ無効経路を通り得るが、
native no-feature check は WASM target のビルド検証ではない。

### 根拠と検証

- `crates/component/src/highlighter/highlighter.rs:22-28,59-120,176-270,531-708,781-1218,1321-1509`
- `crates/component/src/highlighter/input_adapter.rs:32-570`
- 検証: `marked_snapshot_*`、parse window、injection limit/reuse、factory-only language、
  `latest_large_document_wins_over_cancelled_snapshot` を含む highlighter 28件と
  MarkedEditor 7件。

## FORK-003: base editor の公開 seam

### 公開 API

B にない parser 非依存の `gpui-base::input` API を F が追加する。既存 highlighter には
default method が効き、decorator を設定しない通常 editor の動作は変えないため、API は
追加的である。

- `InputHighlighter::{is_ready, document_snapshot, document_snapshot_pending}`。既定実装は
  ready、snapshotなし、pending=false なので既存実装を壊さない。
- `LinePresentation { font_size, line_height, spacing_before, spacing_after }`。
- `BackgroundSpan { range, color }`。
- `InputPresentationDecorator`。`line_metrics_revision()` の既定値 `None` は毎回保守的に
  再計算する意味を持つ。`Some(revision)` は同じ revision の metrics を再利用できる。
- editor state の `set_presentation_decorator`、`presentation_decorator`、
  `highlighter_ready`、typed `highlighter_snapshot<T>`、
  `highlighter_snapshot_pending`、`document_revision`、`replace_utf8_range`。

typed snapshot は `Rc<dyn Any>` を downcast し、型不一致なら `None`。document revision は
文書の置換と edit/history 適用で wrapping increment し、単なる通知や presentation/theme
変更では増やさない。`replace_utf8_range` は UTF-8 byte boundary と range を検証して通常の
edit/undo 経路へ渡す。

### 移植契約

この seam は通常の Input、Textarea、Editor にも存在する共通経路である。MarkedEditor
専用型を base に入れたり、base から component の Tree-sitter 型を import したりしない。
既存 highlighter の default method と、decorator なしの従来 geometry を維持する。

### 根拠と検証

- `crates/base/src/input/editor/highlighting.rs:23-159`
- `crates/base/src/input/base/state.rs:796-924,1284-1289,2812,3839-3868`
- `crates/base/src/input/mod.rs:92-97`
- 検証: document revision 1件、presentation 2件、table format の stale revision test。

## FORK-004: 可変行高と editor geometry

### 基点との差

B は概ね一様な行高を前提にする。F は各 logical line の `LinePresentation` から
wrap row の高さと前後 spacing を作り、`VerticalLayoutMap` の prefix sum を共通の座標源に
する。fold 後の visible wrap row count を使うため、折り畳まれた行の高さは extent へ
残らない。

同じ map を scroll extent、visible range、行配置、gutter/fold icon、caret、selection、
diagnostic/background、pointer hit testing、上下移動、IME bounds、indent guide に使用する。
atomic inline token は表示行の font size と line height で再測定され、wrap・click・編集履歴
は upstream の token contract を保つ。

`PresentationLayoutCache` の key は document revision、display map geometry revision、
wrap width、default metrics、decorator identity、decorator の optional metrics revision を
含む。fold candidate の置換や、総 wrap row 数が同じでも行ごとの wrap 分布が変わる場合は
geometry revision／fold projection を更新する。metrics 変更時は表示中の先頭 logical line
を anchor に scroll offset を補正する。

### 不変条件と制限

- decorator なしでは upstream と同じ default metrics を返す。
- background span だけの変更は line metrics revision を増やさない。
- geometry cache は document text が同じという理由だけで再利用しない。
- `VerticalLayoutMap` は描画の見た目だけでなく入力座標の契約である。一部の paint path
  だけへ可変行高を足してはならない。

### 根拠と検証

- `crates/base/src/input/base/layout.rs:6-161`
- `crates/base/src/input/base/element.rs:404-421,1066-1210,1451-1660,1764-2216,2559-3348`
- `crates/base/src/input/base/movement.rs:153-166`
- `crates/base/src/input/editor/display_map/{display_map,fold_map,text_wrapper,wrap_map}.rs`
- `crates/base/src/input/base/token_presentation.rs`、`indent.rs`
- 検証: presentation 2件、inline token 6件、vertical map 1件、text wrapper 13件、fold 2件、
  auto-close 8件、smart-indent 3件、search 7件、multi-cursor 22件。

## FORK-005: MarkedEditor facade

### 公開 API と状態

B には `MarkedEditor` facade と表操作 API がない。F の `MarkedEditorOptions` の既定値は
`language="markdown"`、`readonly=false`、
`table_actions=true`。`MarkedEditorState::with_language`／`new` は通常の `EditorState` を一つ
所有し、component の highlighter factory と presentation decorator を設定する。公開操作は
`editor`、`options`、`set_value`、`set_content`、`format_table_at`、`table_ranges`。
描画要素は `MarkedEditor::new` で、内部 `Editor` の高さを親に対する100%にする。

`set_content` は presentation を pending にし、fallback revision と表 range を無効化して、
内部 editor の `set_value` へ渡す。この時点で新しい見出しを同期的に構築しない。snapshot
受信時は heading/code/table を更新し、snapshot pending 中は既存 presentation と表操作を
無効化する。highlighter が snapshot を提供せず pending でもない場合だけ、行頭 marker と
fence の軽量 fallback を document revision ごとに構築する。

heading scale は level 1〜4が 1.8 / 1.55 / 1.35 / 1.2、それ以外が1.1。line height は
scale 後と基準の1.15倍の大きい方、前 spacing は level 1〜2が基準行高の0.35、その他が
0.2、後 spacing は0.1。これは F の具体値であり、upstream が typography API を持った場合も
表示契約として比較する。

アプリは `Entity<MarkedEditorState>` と subscription を View に保持する。入力イベント、
検索、selection、LSP などは `editor().clone()` で得る内部 `Entity<EditorState>` を使う。
通常の `InputState`／Textarea を一括置換しない。初期化は `gpui_kit::init(cx)` または
`gpui_component::init(cx)` を state 作成前に行う。

### 根拠と検証

- `crates/component/src/input/marked_editor.rs:1-533`
- `crates/component/src/input/mod.rs:1-57`
- 検証: MarkedEditor 7件、Story build。native の pointer/IME/theme/scroll 目視は未実施。

## FORK-006: 表 range と整形

### 構文と出力

B にはこの表整形機能がない。F は構造 snapshot が示した表 range を background task で
`format_table` に通し、実際に処理
可能な range だけを公開する。generation、snapshot の `Rc` identity、document revision、
UTF-8 boundary を再確認するため、編集前に表示された古い Format button は失敗する。

pipe table は trim 後 `|` で始まる行だけを row とし、2行未満なら拒否する。backslash の
直後と backtick 内の `|` は delimiter にしない。列数は最大 row、幅は separator row を
除く各 cell の `chars().count()` で決め、不足 cell は空として補う。全 cell が colon を
除いて hyphen の separator row は alignment colon を保持する。対象外行は同じ位置に残す。

AsciiDoc は最初と最後の `|===` の間だけを対象とし、その間でも trim 後 `|` で始まる行
だけを整形する。`split_asciidoc_row` には cell specifier を落とす分岐があるが、呼び出し側が
`|` 始まりだけを渡すため、F は `2+|value` や `^|value` で始まる行への一般的な対応を
提供していない。この helper コメントから cell specifier 対応済みと判断してはならない。

出力は `str::lines()` と `join("\n")` を使うため CRLF を LF に正規化する。入力末尾が
`\n` なら末尾 LF を一つ戻す。Unicode の幅は grapheme/display width ではなく Unicode
scalar count なので、CJK や combining mark の視覚列が揃う保証はない。

`format_table_at` は read-only、highlighter未完了、snapshotなし、revision/identity不一致、
不正range、formatter拒否、変更不要で `false`。成功時は `replace_utf8_range` 一回なので
undo 一回で戻る。`table_actions=false` はボタンを隠すが、公開 method 自体の呼び出しを
禁止しない。

### 根拠と検証

- `crates/component/src/input/table_format.rs:1-214`
- `crates/component/src/input/marked_editor.rs:96-132,199-291,404-418,491-532`
- 検証: formatter 3件、MarkedEditor の read-only、single undo、stale action test。

## FORK-007: semantic code-block background

B の theme/highlight registry にない `text.literal.block` capture と
`ThemeStyle.background_color` を F が追加し、
light/dark default theme の両方へ背景色を定義する。Markdown fenced/indented code、AsciiDoc
listing/literal、Djot code/raw block がこの capture を生成する。MarkedEditor は active
theme の `text.literal.block.background_color` を描画時に取り、未定義なら
`theme.muted.opacity(0.35)` を使う。

theme change は line metrics を変えないため geometry cache の revision を増やさず、
render 時の背景色だけ更新する。移植時は query、registry の name/serde field/conversion、
両 theme、MarkedEditor の fallback を一組で保持する。

根拠は `highlighter/registry.rs:13-292`、4言語の query、
`theme/default-theme.json:192-195,407-410`、`marked_editor.rs:491-503`。

## FORK-008: Story と回帰証拠

B にない `MarkedEditorStory` を F の native gallery に登録する。Story は Markdown／Djot／AsciiDoc、
editable/read-only、atomic token、長い wrap、table、code block、高さ制約を人が確認するための
fixture である。Story の compile は公開 API と feature integration を検査するが、GUI の
見た目・IME・pointer 操作の成功を証明しない。

自動テストは各差分 ID の根拠であり、次回 sync では削除されたテストを同等のテストへ
対応付ける。0件 filter は成功扱いしない。実行済み件数は [README.md](README.md) に記録した。

根拠は `crates/story/src/stories/marked_editor_story.rs:1-180` と gallery/module registration、
各実装ファイルの `#[cfg(test)]` block。

## FORK-009: runtime ではない差分

`.agents/skills/**`、`AGENTS.md`、`.gitignore` のローカル補助設定、`ARCHITECTURE.adoc`、
`rust_programming.djot`、`docs/.vitepress/cache/**` は F に含まれるが FORK-001〜008 の runtime
実装ではない。`ARCHITECTURE.adoc` は本文が別プロジェクト `acdc` の説明で、この repo の
`docs/ARCHITECTURE.md` と同一視しない。`rust_programming.djot` はコードから参照されて
いない。VitePress cache は生成物である。

次回 sync では台帳から消さず、保持／再生成／除外を個別に判断する。内容が大きいという
理由だけで upstream 移植パッチへ混ぜない。既存の migration・range-diff 文書も履歴資料
として保持し、現行正本への案内と既知の相違を明記する。
