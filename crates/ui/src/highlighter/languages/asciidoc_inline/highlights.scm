; AsciiDoc inline highlights
; ノード名は tree-sitter-asciidoc-inline 0.7.0 の NODE_TYPES から確認済み。
;
; AsciiDoc の強調記法:
;   bold (constrained):   *word*   / unconstrained: **word**  → ノード名: emphasis
;   italic (constrained): _word_   / unconstrained: __word__  → ノード名: ltalic (grammarのタイポ)
;   monospace:            `word`                              → ノード名: monospace
;   highlight:            #word#                              → ノード名: highlight

; bold: *word* / **word**
((emphasis) @emphasis.strong
  (#set! highlight.allow-overlap))

; italic: _word_ / __word__
; ("ltalic" は tree-sitter-asciidoc-inline grammar 内のタイポがそのままノード名)
((ltalic) @emphasis
  (#set! highlight.allow-overlap))

; monospace inline code: `word`
(monospace) @text.literal

; highlight: #word#
(highlight) @text.literal

; autolink URL
(autolink) @link_uri

; cross-reference
(xref) @link_text
