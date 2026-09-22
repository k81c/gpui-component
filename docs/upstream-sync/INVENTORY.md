# 固定差分の全件台帳

対象は `B=7f6d92327936fbab7994a35c86328d793acc060d` から
`F=d12bb0f9fa551ae41cab65c3ea08d3649102a727`。`git diff --name-only -M B F` の87件を
この文書で全件分類する。`+/-` は `git diff --numstat`、hunk 数と旧→新範囲は
`--unified=0 --find-renames=50% --diff-algorithm=myers` で取得した。

分類は次の意味を持つ。

- **実装**: 次回 sync で差分 ID の契約として移植または吸収判定する。
- **依存・設定**: 実装と一緒に整合させる manifest、lock、theme、export。
- **テスト・Story**: 自動回帰または手動確認 fixture として移植する。
- **保守資料**: runtime へ入れず、必要性を別に判断する。
- **生成物・参考資料**: runtime 移植から除外する。削除の判断はこの台帳とは別に行う。

## 実装・依存・テスト: 33 files

`対応 hunk` はそのファイルの全 hunk を割り当てた数である。追加ファイルは diff 上1 hunk
なので、`主要範囲` に公開 API、状態遷移、helper、test の内部区分を併記した。

| path | +/- | hunk | 差分 ID | 主要範囲または役割 |
| --- | ---: | ---: | --- | --- |
| `Cargo.lock` | 32/0 | 4/4 | FORK-001 | B 3817,3825,10662,10742 → F 3818,3828,10666-10685,10766-10774。3 grammar の解決を固定 |
| `crates/base/src/input/base/element.rs` | 453/109 | 106/106 | FORK-004 | F 412-419 cache入力、1066-1210 metrics/map/cache/anchor、1465-2216 layout/paint、2559-3348 prepaint/paint/hit/IME/selection |
| `crates/base/src/input/base/layout.rs` | 115/1 | 6/6 | FORK-004 | B 3-47 → F 3-161。cache key、`VerticalLayoutMap`、`LastLayout`、test |
| `crates/base/src/input/base/movement.rs` | 14/7 | 1/1 | FORK-004 | B 153-159 → F 153-166。上下移動を可変 geometry へ接続 |
| `crates/base/src/input/base/state.rs` | 431/35 | 46/46 | FORK-003,004,008 | F 353,419-420 state、716/752 init、847-907 API、1284-1289/2812 revision、2587-2667・3089-3150・3616-3638・3835-3868・4326-4412 integration、4578-4803 test |
| `crates/base/src/input/base/token_presentation.rs` | 2/2 | 2/2 | FORK-004 | B/F 117,121。token cache key を表示 metrics と整合 |
| `crates/base/src/input/editor/display_map/display_map.rs` | 73/8 | 18/18 | FORK-004 | F 32-301 geometry revision propagation、420-448 geometry/inline/visible wrap query |
| `crates/base/src/input/editor/display_map/fold_map.rs` | 91/14 | 7/7 | FORK-004,008 | F 56-59 dirty、119-132 candidate置換、206-208 counts、262-276/311-328 rebuild、375-420 test |
| `crates/base/src/input/editor/display_map/text_wrapper.rs` | 101/13 | 26/26 | FORK-004,008 | F 243-280 setter result、373-427 inline metrics、772-960 per-row hit layout、1404-1428/1720-1744 test |
| `crates/base/src/input/editor/display_map/wrap_map.rs` | 16/8 | 5/5 | FORK-004 | F 119-160。layout変更のbool伝播と inline metrics |
| `crates/base/src/input/editor/highlighting.rs` | 78/2 | 4/4 | FORK-003 | F 23-69 snapshot API、103-159 presentation public contract |
| `crates/base/src/input/editor/indent.rs` | 10/7 | 9/9 | FORK-004 | B 120-156 → F 119-159。indent guide y/height を vertical map へ統合 |
| `crates/base/src/input/mod.rs` | 4/2 | 1/1 | FORK-003 | B 92-93 → F 92-95。presentation 型の re-export |
| `crates/component/Cargo.toml` | 8/0 | 6/6 | FORK-001 | F 25,34,64-65,73,148-149,157。3 feature と dependency |
| `crates/component/src/highlighter/highlighter.rs` | 362/14 | 27/27 | FORK-002,007,008 | F 22-270 limits/data helper、404-708 snapshot/cache、781-1218 parse/injection apply、1321-1326 cache、1590-1644 test |
| `crates/component/src/highlighter/input_adapter.rs` | 398/57 | 30/30 | FORK-002,008 | F 36-73 task state、78-109 adapter API、122-405 update state machine、433-569 window/fold/cancel/test |
| `crates/component/src/highlighter/languages.rs` | 99/0 | 11/11 | FORK-001,008 | F 9-33 enum、107-132 name、202-224 display/lookup、290-293 injections、425-460 grammar config、704-790 test |
| `crates/component/src/highlighter/languages/asciidoc/highlights.scm` | 20/0 | 1/1 | FORK-001,007 | F 1-20 title、marker、listing/literal captures |
| `crates/component/src/highlighter/languages/asciidoc/injections.scm` | 4/0 | 1/1 | FORK-001 | F 1-4 inline combined injection |
| `crates/component/src/highlighter/languages/asciidoc_inline/highlights.scm` | 10/0 | 1/1 | FORK-001 | F 1-10 emphasis、literal、link captures |
| `crates/component/src/highlighter/languages/djot/highlights.scm` | 22/0 | 1/1 | FORK-001,007 | F 1-22 heading、emphasis、code/raw、link captures |
| `crates/component/src/highlighter/languages/markdown/highlights.scm` | 4/3 | 4/4 | FORK-007 | B 15-53 → F 16-54。heading marker と code block capture |
| `crates/component/src/highlighter/registry.rs` | 7/1 | 6/6 | FORK-007 | F 15-52 name、166-167 serde、230 background、239 conversion、288 lookup |
| `crates/component/src/highlighter/wasm_stub.rs` | 7/0 | 1/1 | FORK-002 | F 13-19 snapshot shape stub。Tree-sitter無効時の型整合で、parse機能ではない |
| `crates/component/src/input/marked_editor.rs` | 823/0 | 1/1 | FORK-005,006,008 | F 1-26 ownership、28-57 public state、58-291 lifecycle/API/table freshness、293-468 presentation/helper、470-533 element、535-823 tests |
| `crates/component/src/input/mod.rs` | 3/0 | 3/3 | FORK-005,006 | F 4 module、18 table module、56 public marked editor export |
| `crates/component/src/input/table_format.rs` | 214/0 | 1/1 | FORK-006,008 | F 1-49 lexer helper、51-122 pipe、124-177 AsciiDoc、179-185 dispatch、187-214 tests |
| `crates/component/src/theme/default-theme.json` | 8/0 | 2/2 | FORK-007 | F 192-195 light、407-410 dark `text.literal.block` |
| `crates/kit/Cargo.toml` | 3/0 | 2/2 | FORK-001 | F 32-33 AsciiDoc、42 Djot feature forwarding |
| `crates/story/src/gallery.rs` | 1/0 | 1/1 | FORK-008 | F 92 navigation entry |
| `crates/story/src/lib.rs` | 1/0 | 1/1 | FORK-008 | F 738 Story registration |
| `crates/story/src/stories/marked_editor_story.rs` | 180/0 | 1/1 | FORK-008 | F 1-180 fixture、state、controls、render |
| `crates/story/src/stories/mod.rs` | 2/0 | 2/2 | FORK-008 | F 41 module、118 export |

## 保守資料: 32 files

これらは runtime 差分へ cherry-pick しない。必要なら新 upstream 上で目的を再評価する。
すべて追加ファイルは全体が1 hunk、`.gitignore` だけは F 36-45 の1 hunkである。

| path | +/- | 分類・扱い |
| --- | ---: | --- |
| `.gitignore` | 10/0 | FORK-009。ccc・ローカル補助ファイルの ignore。upstream ignore と三者 merge |
| `AGENTS.md` | 243/0 | FORK-009。エージェント向け repository guidance。実コードを一次根拠に更新 |
| `APP_MIGRATION_CONTEXT.md` | 330/0 | FORK-009。a9ec以前中心のアプリ移行履歴。本正本への案内と既知差を保持 |
| `ARCHITECTURE.adoc` | 380/0 | FORK-009。別プロジェクト `acdc` の内容。gpui-kit architecture として移植しない |
| `UPSTREAM_SYNC_RANGE_DIFF.md` | 39/0 | FORK-009。681d857cまでの歴史的 range-diff record |
| `rust_programming.djot` | 40942/0 | FORK-009。コード参照なしの参考資料。runtime／Story fixture として扱わない |
| `.agents/skills/generate-component-documentation/SKILL.md` | 20/0 | FORK-009。開発 skill |
| `.agents/skills/generate-component-story/SKILL.md` | 20/0 | FORK-009。開発 skill |
| `.agents/skills/github-pull-request-description/SKILL.md` | 44/0 | FORK-009。開発 skill |
| `.agents/skills/gpui-action/SKILL.md` | 180/0 | FORK-009。GPUI guide |
| `.agents/skills/gpui-async/SKILL.md` | 177/0 | FORK-009。GPUI guide |
| `.agents/skills/gpui-context/SKILL.md` | 161/0 | FORK-009。GPUI guide |
| `.agents/skills/gpui-element/SKILL.md` | 126/0 | FORK-009。GPUI guide |
| `.agents/skills/gpui-element/references/advanced-patterns.md` | 705/0 | FORK-009。skill reference |
| `.agents/skills/gpui-element/references/api-reference.md` | 477/0 | FORK-009。skill reference |
| `.agents/skills/gpui-element/references/best-practices.md` | 546/0 | FORK-009。skill reference |
| `.agents/skills/gpui-element/references/examples.md` | 632/0 | FORK-009。skill reference |
| `.agents/skills/gpui-element/references/patterns.md` | 509/0 | FORK-009。skill reference |
| `.agents/skills/gpui-entity/SKILL.md` | 168/0 | FORK-009。GPUI guide |
| `.agents/skills/gpui-entity/references/advanced.md` | 528/0 | FORK-009。skill reference |
| `.agents/skills/gpui-entity/references/api-reference.md` | 382/0 | FORK-009。skill reference |
| `.agents/skills/gpui-entity/references/best-practices.md` | 484/0 | FORK-009。skill reference |
| `.agents/skills/gpui-entity/references/patterns.md` | 579/0 | FORK-009。skill reference |
| `.agents/skills/gpui-event/SKILL.md` | 176/0 | FORK-009。GPUI guide |
| `.agents/skills/gpui-focus-handle/SKILL.md` | 232/0 | FORK-009。GPUI guide |
| `.agents/skills/gpui-global/SKILL.md` | 204/0 | FORK-009。GPUI guide |
| `.agents/skills/gpui-layout-and-style/SKILL.md` | 177/0 | FORK-009。GPUI guide |
| `.agents/skills/gpui-style-guide/SKILL.md` | 555/0 | FORK-009。repository style guide |
| `.agents/skills/gpui-test/SKILL.md` | 94/0 | FORK-009。testing guide |
| `.agents/skills/gpui-test/examples.md` | 172/0 | FORK-009。skill reference |
| `.agents/skills/gpui-test/reference.md` | 350/0 | FORK-009。skill reference |
| `.agents/skills/new-component/SKILL.md` | 215/0 | FORK-009。component scaffolding guide |

## 生成物: 22 files

すべて `docs/.vitepress/cache/deps/` 下の追加ファイルで、FORK-009 に分類する。VitePress/Vite
が再生成する bundle または metadata であり、FORK-001〜008 のソースではない。次回 sync
では手作業で移植せず、cache を必要とする運用が確認できた場合だけ同じ toolchain で再生成
する。

| path | +/− |
| --- | ---: |
| `docs/.vitepress/cache/deps/_metadata.json` | 61/0 |
| `docs/.vitepress/cache/deps/chunk-6WSJOYDR.js` | 9952/0 |
| `docs/.vitepress/cache/deps/chunk-6WSJOYDR.js.map` | 7/0 |
| `docs/.vitepress/cache/deps/chunk-PZ5AY32C.js` | 9/0 |
| `docs/.vitepress/cache/deps/chunk-PZ5AY32C.js.map` | 7/0 |
| `docs/.vitepress/cache/deps/chunk-ZZEIC257.js` | 12705/0 |
| `docs/.vitepress/cache/deps/chunk-ZZEIC257.js.map` | 7/0 |
| `docs/.vitepress/cache/deps/lucide-vue-next.js` | 26413/0 |
| `docs/.vitepress/cache/deps/lucide-vue-next.js.map` | 7/0 |
| `docs/.vitepress/cache/deps/package.json` | 3/0 |
| `docs/.vitepress/cache/deps/vitepress___@vue_devtools-api.js` | 3821/0 |
| `docs/.vitepress/cache/deps/vitepress___@vue_devtools-api.js.map` | 7/0 |
| `docs/.vitepress/cache/deps/vitepress___@vueuse_core.js` | 589/0 |
| `docs/.vitepress/cache/deps/vitepress___@vueuse_core.js.map` | 7/0 |
| `docs/.vitepress/cache/deps/vitepress___@vueuse_integrations_useFocusTrap.js` | 1154/0 |
| `docs/.vitepress/cache/deps/vitepress___@vueuse_integrations_useFocusTrap.js.map` | 7/0 |
| `docs/.vitepress/cache/deps/vitepress___mark__js_src_vanilla__js.js` | 1667/0 |
| `docs/.vitepress/cache/deps/vitepress___mark__js_src_vanilla__js.js.map` | 7/0 |
| `docs/.vitepress/cache/deps/vitepress___minisearch.js` | 1815/0 |
| `docs/.vitepress/cache/deps/vitepress___minisearch.js.map` | 7/0 |
| `docs/.vitepress/cache/deps/vue.js` | 343/0 |
| `docs/.vitepress/cache/deps/vue.js.map` | 7/0 |

## 12 commit の最終仕様への対応

| commit | 最終状態での扱い |
| --- | --- |
| `f169f6a1b52cdb71cc38c93deb32fffc29dc74f5` | FORK-009。fork metadata と保守資料 |
| `6c6dd34485cc45b567906357d1c6d710ab260a1b` | FORK-001,002,003,005,006,007 の初期移植 |
| `1a3d524e34eb87b3d132bd921f57d97c42a7117d` | FORK-006。表以外の行を元位置に保持 |
| `e56f273857d053e40ab1b970fcb59f7d559e566e` | FORK-003,004,005。可変行高と表示統合 |
| `923499da7031a9eb6dfe9aab9431fee6a2c46d91` | FORK-002。registry parser、snapshot、非同期 parse の parity |
| `f3d1e5edccf41d2a2506b030720d9769e2826158` | FORK-009。旧アプリ移行資料 |
| `52c18a9283df0e739509f9939864c02eb390827c` | FORK-004。upstream atomic inline token と可変 metrics の統合 |
| `157574280a26d312fa47f87d5806414a065e27ce` | FORK-008。MarkedEditor Story |
| `55a7c944eb97c244c0dd0746a19fae8b65689435` | FORK-005。内部 Editor の相対高さ100% |
| `681d857ce46a7056b174a32bb80312cb87eebf1d` | FORK-006,008。read-only／single undo／stale table contract test |
| `a9ec0880e49099abf634592baf92a8f1c4e418aa` | FORK-009。旧 range-diff 記録 |
| `d12bb0f9fa551ae41cab65c3ea08d3649102a727` | FORK-002〜008。snapshot generation、非同期 table validation、geometry/cache/fold/wrap の最終修正と追加 test。以前の commit の挙動はこの最終状態で読む |

旧6 commit との patch-level 対応はリポジトリ直下の `UPSTREAM_SYNC_RANGE_DIFF.md` に残す。
その表は `681d857c` までの履歴であり、F の最終仕様を網羅しない。

## 全件・被覆の再確認

```text
git diff --name-only -M B F
git diff --numstat B F
git diff --no-ext-diff --no-textconv --no-color --find-renames=50% --diff-algorithm=myers --unified=0 B F -- Cargo.lock crates
```

期待値は全87 files、内訳は base 12、component 15、kit 1、story 4、skills 26、generated
cache 22、その他 root 7。この台帳では Cargo.lock を実装側へ数えているため、表の合計は
実装・依存・テスト33、保守資料32、生成物22となる。実装表の hunk 分母合計と実際の
diff hunk 数が一致しない場合は、文書を更新するまで新しい sync の完了としない。
