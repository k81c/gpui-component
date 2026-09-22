use gpui_kit::component::{
    input::{InlineToken, InputContent, MarkedEditor, MarkedEditorOptions, MarkedEditorState},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled, Window,
    px,
};

use crate::section;

const MARKDOWN: &str = r#"# Marked editor

Ask @guide to review this document. This deliberately long paragraph wraps across several visual rows so vertical movement, selection, pointer hit testing and scrolling can be checked with a heading-sized line above it.

| Feature | Status |
| --- | --- |
| Variable line height | Ready |
| Table formatting | Select Format |

```rust
fn main() {
    println!("variable-height code block");
}
```
"#;

const DJOT: &str = r#"# Djot heading

Djot content uses the same structured editor facade and syntax snapshot path.

| feature | status |
| ------- | ------ |
| parser factory | runtime |

``` rust
let answer = 42;
```
"#;

const ASCIIDOC: &str = r#"= AsciiDoc heading

The AsciiDoc grammar and aliases use the same runtime parser registry.

[cols="1,1"]
|===
|Feature |Status

|Headings
|Ready

|Code blocks
|Ready
|===

[source,rust]
----
let answer = 42;
----
"#;

const CONSTRAINED: &str = r#"# Height constraint

This editor is intentionally short. Scroll through wrapped text, move the caret vertically, select across the heading boundary, and verify the IME candidate anchor follows the caret.

## Second heading

The bottom lines should remain reachable without the editor escaping its parent height.
"#;

pub struct MarkedEditorStory {
    markdown: Entity<MarkedEditorState>,
    readonly: Entity<MarkedEditorState>,
    djot: Entity<MarkedEditorState>,
    asciidoc: Entity<MarkedEditorState>,
    constrained: Entity<MarkedEditorState>,
}

impl super::Story for MarkedEditorStory {
    fn title() -> &'static str {
        "Marked Editor"
    }

    fn description() -> &'static str {
        "Structured Markdown, Djot and AsciiDoc editing with variable-height presentation."
    }

    fn closable() -> bool {
        false
    }

    fn new_view(window: &mut Window, cx: &mut App) -> Entity<impl Render> {
        Self::view(window, cx)
    }
}

impl MarkedEditorStory {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::new(window, cx))
    }

    fn state(
        language: &'static str,
        readonly: bool,
        value: &'static str,
        inline_token: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<MarkedEditorState> {
        cx.new(|cx| {
            let mut state = MarkedEditorState::new(
                MarkedEditorOptions {
                    language: language.into(),
                    readonly,
                    table_actions: true,
                },
                window,
                cx,
            );
            if inline_token {
                let start = value.find("@guide").expect("sample token is present");
                let content = InputContent::new(value)
                    .with_token(
                        start..start + "@guide".len(),
                        InlineToken::new("person:guide", "@guide").with_label("Guide"),
                    )
                    .expect("sample token range is valid");
                state.set_content(content, window, cx);
            } else {
                state.set_value(value, window, cx);
            }
            state
        })
    }

    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            markdown: Self::state("markdown", false, MARKDOWN, true, window, cx),
            readonly: Self::state("markdown", true, MARKDOWN, false, window, cx),
            djot: Self::state("djot", false, DJOT, false, window, cx),
            asciidoc: Self::state("asciidoc", false, ASCIIDOC, false, window, cx),
            constrained: Self::state("markdown", false, CONSTRAINED, false, window, cx),
        }
    }
}

impl Render for MarkedEditorStory {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_4()
            .child(
                section("Markdown — editable")
                    .description(
                        "Heading scaling, an atomic inline token, long wrapping, a table action and a fenced code block.",
                    )
                    .child(MarkedEditor::new(&self.markdown).h(px(420.))),
            )
            .child(
                section("Markdown — read-only")
                    .description("Selection and copy remain available while edits and table actions are disabled.")
                    .child(MarkedEditor::new(&self.readonly).h(px(240.))),
            )
            .child(
                section("Djot")
                    .description("Djot heading, table and fenced-code snapshot extraction.")
                    .child(MarkedEditor::new(&self.djot).h(px(300.))),
            )
            .child(
                section("AsciiDoc")
                    .description("AsciiDoc heading, table and source-block grammar integration.")
                    .child(MarkedEditor::new(&self.asciidoc).h(px(340.))),
            )
            .child(
                section("Constrained height")
                    .description("A focused case for relative height, wrap, navigation, IME anchoring and scroll.")
                    .child(MarkedEditor::new(&self.constrained).h(px(180.))),
            )
    }
}
