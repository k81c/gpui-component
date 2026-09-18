//! Structured editor facade for Markdown, Djot and AsciiDoc documents.
//!
//! `MarkedEditorState` deliberately owns an ordinary [`EditorState`].  Syntax
//! parsing, selection, folding and undo therefore continue to use the upstream
//! editor implementation while this facade adds document-aware actions.

use std::{cell::RefCell, ops::Range, rc::Rc};

use gpui::{
    App, AppContext as _, Entity, IntoElement, ParentElement as _, RenderOnce, SharedString,
    StyleRefinement, Styled, Subscription, Window, div, px,
};

use crate::button::{Button, ButtonVariants as _};
use crate::highlighter::MarkedSyntaxSnapshot;
use crate::{ActiveTheme as _, Sizable};
use crate::{IconName, StyledExt as _};

use super::table_format;
use super::{Editor, EditorState};
use gpui_base::input::{
    BackgroundSpan, HighlightStyleResolver as _, InputContent, InputPresentationDecorator,
    LinePresentation,
};

/// Options controlling a [`MarkedEditorState`].
#[derive(Clone, Debug)]
pub struct MarkedEditorOptions {
    pub language: SharedString,
    pub readonly: bool,
    pub table_actions: bool,
}

impl Default for MarkedEditorOptions {
    fn default() -> Self {
        Self {
            language: "markdown".into(),
            readonly: false,
            table_actions: true,
        }
    }
}

/// State for a document-aware editor.
pub struct MarkedEditorState {
    editor: Entity<EditorState>,
    options: MarkedEditorOptions,
    presentation: Rc<RefCell<MarkedPresentation>>,
    _subscription: Subscription,
}

impl MarkedEditorState {
    /// Convenience constructor for the common language-only case.
    pub fn with_language(
        language: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        Self::new(
            MarkedEditorOptions {
                language: language.into(),
                ..Default::default()
            },
            window,
            cx,
        )
    }

    /// Create a marked editor and install the shared Tree-sitter highlighter.
    pub fn new(
        options: MarkedEditorOptions,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let language = options.language.clone();
        let readonly = options.readonly;
        let presentation = Rc::new(RefCell::new(MarkedPresentation::default()));
        let presentation_for_editor = presentation.clone();
        let editor = cx.new(|cx| {
            let mut state = EditorState::new(window, cx)
                .language(language.clone())
                .default_value("");
            state.ensure_highlighter_factory(crate::highlighter::input_highlighter_factory());
            state.set_readonly(readonly, cx);
            let decorator: Rc<dyn InputPresentationDecorator> =
                Rc::new(MarkedDecorator(presentation_for_editor));
            state.set_presentation_decorator(Some(decorator), cx);
            state
        });
        let subscription = cx.observe(&editor, |state, editor, cx| {
            let value = editor.read(cx).value();
            let snapshot = editor
                .read(cx)
                .highlighter_snapshot::<MarkedSyntaxSnapshot>();
            state.presentation.borrow_mut().rebuild(
                &value,
                state.options.language.as_ref(),
                snapshot.as_deref(),
            );
            cx.notify();
        });
        Self {
            editor,
            options,
            presentation,
            _subscription: subscription,
        }
    }

    pub fn editor(&self) -> &Entity<EditorState> {
        &self.editor
    }

    pub fn options(&self) -> &MarkedEditorOptions {
        &self.options
    }

    pub fn set_value(
        &mut self,
        value: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.set_content(InputContent::new(value.into()), window, cx);
    }

    /// Replace the document while preserving any atomic inline tokens attached
    /// to it. Like [`Self::set_value`], this resets the editor history.
    pub fn set_content(
        &mut self,
        content: InputContent,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let value = content.text().clone();
        self.presentation
            .borrow_mut()
            .rebuild(&value, self.options.language.as_ref(), None);
        self.editor.update(cx, |editor, cx| {
            editor.set_value(content, window, cx);
        });
    }

    /// Format the table containing `range` as one undoable UTF-8 edit.
    pub fn format_table_at(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let source = {
            let editor = self.editor.read(cx);
            if !editor.is_editable() || !editor.highlighter_ready() {
                return false;
            }
            let text = editor.text();
            if range.start > range.end
                || range.end > text.len()
                || !text.is_char_boundary(range.start)
                || !text.is_char_boundary(range.end)
            {
                return false;
            }
            text.slice(range.clone()).to_string()
        };
        let Some(formatted) = table_format::format_table(&source) else {
            return false;
        };
        if formatted == source {
            return false;
        }
        self.editor.update(cx, |editor, cx| {
            editor.replace_utf8_range(range, &formatted, window, cx);
        });
        true
    }

    pub fn table_ranges(&self, cx: &App) -> Vec<Range<usize>> {
        let editor = self.editor.read(cx);
        if !editor.is_editable() || !editor.highlighter_ready() {
            return Vec::new();
        }
        let Some(snapshot) = editor.highlighter_snapshot::<MarkedSyntaxSnapshot>() else {
            return Vec::new();
        };
        let text = editor.text();
        snapshot
            .table_ranges
            .iter()
            .filter(|range| {
                range.start <= range.end
                    && range.end <= text.len()
                    && text.is_char_boundary(range.start)
                    && text.is_char_boundary(range.end)
                    && table_format::format_table(&text.slice((*range).clone()).to_string())
                        .is_some()
            })
            .cloned()
            .collect()
    }
}

#[derive(Default)]
struct MarkedPresentation {
    headings: Vec<Option<u8>>,
    backgrounds: Vec<Range<usize>>,
    code_block_background: Option<gpui::Hsla>,
}

impl MarkedPresentation {
    fn rebuild(&mut self, text: &str, language: &str, snapshot: Option<&MarkedSyntaxSnapshot>) {
        self.headings.clear();
        self.backgrounds.clear();
        if let Some(snapshot) = snapshot {
            self.headings.clone_from(&snapshot.heading_levels);
            self.backgrounds.clone_from(&snapshot.code_block_ranges);
            return;
        }
        let mut offset = 0;
        let mut in_fence = false;
        let mut fence_start = None;
        for line in text.split_inclusive('\n') {
            let trimmed = line.trim_start();
            let level = if language.eq_ignore_ascii_case("asciidoc")
                || language.eq_ignore_ascii_case("adoc")
            {
                let count = trimmed.chars().take_while(|c| *c == '=').count();
                (count > 0 && trimmed.as_bytes().get(count) == Some(&b' ')).then_some(count as u8)
            } else {
                let count = trimmed.chars().take_while(|c| *c == '#').count();
                (count > 0 && trimmed.as_bytes().get(count) == Some(&b' ')).then_some(count as u8)
            };
            self.headings.push(level);
            let fence =
                trimmed.starts_with("```") || trimmed.starts_with("~~~") || trimmed == "----";
            if fence {
                if in_fence {
                    if let Some(start) = fence_start.take() {
                        self.backgrounds.push(start..offset + line.len());
                    }
                    in_fence = false;
                } else {
                    in_fence = true;
                    fence_start = Some(offset);
                }
            } else if in_fence && fence_start.is_none() {
                fence_start = Some(offset);
            }
            offset += line.len();
        }
        if let Some(start) = fence_start {
            self.backgrounds.push(start..text.len());
        }
    }
}

struct MarkedDecorator(Rc<RefCell<MarkedPresentation>>);

impl InputPresentationDecorator for MarkedDecorator {
    fn line_presentation(&self, line: usize, mut default: LinePresentation) -> LinePresentation {
        let level = self.0.borrow().headings.get(line).copied().flatten();
        if let Some(level) = level {
            let base_line_height = default.line_height;
            let scale = match level {
                1 => 1.8,
                2 => 1.55,
                3 => 1.35,
                4 => 1.2,
                _ => 1.1,
            };
            default.font_size *= scale;
            default.line_height = (base_line_height * scale).max(base_line_height * 1.15);
            default.spacing_before = if level <= 2 {
                base_line_height * 0.35
            } else {
                base_line_height * 0.2
            };
            default.spacing_after = base_line_height * 0.1;
        }
        default
    }

    fn background_spans(&self, range: &Range<usize>) -> Vec<BackgroundSpan> {
        let Some(color) = self.0.borrow().code_block_background else {
            return Vec::new();
        };
        self.0
            .borrow()
            .backgrounds
            .iter()
            .filter_map(|background| {
                let start = background.start.max(range.start);
                let end = background.end.min(range.end);
                (start < end).then(|| BackgroundSpan {
                    range: start..end,
                    color,
                })
            })
            .collect()
    }
}

/// A styled marked editor with optional table-format actions.
#[derive(IntoElement)]
pub struct MarkedEditor {
    state: Entity<MarkedEditorState>,
    style: StyleRefinement,
}

impl MarkedEditor {
    pub fn new(state: &Entity<MarkedEditorState>) -> Self {
        Self {
            state: state.clone(),
            style: StyleRefinement::default(),
        }
    }
}

impl Styled for MarkedEditor {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for MarkedEditor {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = self.state.read(cx);
        let code_block_background = cx
            .theme()
            .highlight_theme
            .style("text.literal.block")
            .and_then(|style| style.background_color)
            .unwrap_or_else(|| cx.theme().muted.opacity(0.35));
        state.presentation.borrow_mut().code_block_background = Some(code_block_background);
        let editor = state.editor.clone();
        let options = state.options.clone();
        let ranges = if options.table_actions && !options.readonly {
            state.table_ranges(cx)
        } else {
            Vec::new()
        };
        let marked_state = self.state.clone();

        let mut root = div()
            .relative()
            .size_full()
            .child(
                Editor::new(&editor)
                    .readonly(options.readonly)
                    .h(gpui::relative(1.)),
            );
        for (index, range) in ranges.into_iter().enumerate() {
            let Some(bounds) = editor.read(cx).range_to_bounds(&range) else {
                continue;
            };
            let editor_origin = editor.read(cx).input_bounds().origin;
            let action_state = marked_state.clone();
            let action_range = range.clone();
            let button = Button::new(format!("marked-editor-table-{index}"))
                .icon(IconName::Check)
                .label("Format")
                .small()
                .ghost()
                .on_click(move |_, window, cx| {
                    action_state.update(cx, |state, cx| {
                        state.format_table_at(action_range.clone(), window, cx);
                    });
                });
            root = root.child(
                div()
                    .absolute()
                    .right(px(4.))
                    .top((bounds.origin.y - editor_origin.y).max(px(0.)))
                    .child(button),
            );
        }
        root.refine_style(&self.style)
    }
}

#[cfg(test)]
mod tests {
    use super::MarkedPresentation;
    use gpui::px;
    use gpui_base::input::{InputPresentationDecorator, LinePresentation};

    #[test]
    fn marked_presentation_extracts_headings_and_fences() {
        let mut presentation = MarkedPresentation::default();
        presentation.rebuild("# Title\nbody\n```\ncode\n```\n", "markdown", None);
        assert_eq!(presentation.headings[0], Some(1));
        assert_eq!(presentation.headings[1], None);
        assert_eq!(presentation.backgrounds.len(), 1);
        let decorator =
            super::MarkedDecorator(std::rc::Rc::new(std::cell::RefCell::new(presentation)));
        let line = decorator.line_presentation(
            0,
            LinePresentation {
                font_size: px(10.),
                line_height: px(15.),
                spacing_before: px(0.),
                spacing_after: px(0.),
            },
        );
        assert!(line.font_size > px(10.));
        assert!(line.line_height > px(15.));
        assert_eq!(line.spacing_before, px(5.25));
        assert_eq!(line.spacing_after, px(1.5));
    }
}
