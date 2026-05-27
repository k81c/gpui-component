(heading) @title

(heading
  (marker) @punctuation.special)

(thematic_break) @punctuation.special

((emphasis) @emphasis
  (#set! highlight.allow-overlap))

((strong) @emphasis.strong
  (#set! highlight.allow-overlap))

[
  (code_block)
  (raw_block)
] @text.literal.block

(verbatim) @text.literal

[
  (link_text)
] @link_text

[
  (autolink)
  (inline_link_destination)
  (link_destination)
] @link_uri
