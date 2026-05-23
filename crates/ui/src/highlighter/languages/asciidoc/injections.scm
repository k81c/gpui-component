; AsciiDoc block → asciidoc_inline injection
; `line` は paragraph / list items / admonition / heading など
; インライン内容を持つ全ノードに共通する子ノード。
; listing_block_body は line を含まないためコードブロックへの誤注入は起きない。
; injection.combined により全 line 範囲を1回のパースにまとめる。
((line) @injection.content
  (#set! injection.language "asciidoc_inline")
  (#set! injection.combined))
