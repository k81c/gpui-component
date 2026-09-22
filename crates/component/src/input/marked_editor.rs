//! Structured editor facade for Markdown, Djot and AsciiDoc documents.
//!
//! `MarkedEditorState` deliberately owns an ordinary [`EditorState`].  Syntax
//! parsing, selection, folding and undo therefore continue to use the upstream
//! editor implementation while this facade adds document-aware actions.

use std::{cell::RefCell, ops::Range, rc::Rc};

use gpui::{
    App, AppContext as _, Entity, IntoElement, ParentElement as _, RenderOnce, SharedString,
    StyleRefinement, Styled, Subscription, Task, Window, div, px,
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
    fallback_revision: Option<u64>,
    table_ranges: Vec<Range<usize>>,
    table_snapshot: Option<Rc<MarkedSyntaxSnapshot>>,
    table_document_revision: Option<u64>,
    table_generation: u64,
    table_task: Option<Task<()>>,
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
            let editor = editor.read(cx);
            let snapshot = editor.highlighter_snapshot::<MarkedSyntaxSnapshot>();
            if let Some(snapshot) = snapshot {
                let changed = state
                    .presentation
                    .borrow_mut()
                    .set_snapshot(snapshot.clone());
                state.fallback_revision = None;
                if changed {
                    state.table_generation = state.table_generation.wrapping_add(1);
                    let generation = state.table_generation;
                    state.table_ranges.clear();
                    state.table_snapshot = None;
                    state.table_document_revision = None;
                    state.table_task.take();
                    let text = editor.text().clone();
                    let document_revision = editor.document_revision();
                    let ranges = snapshot.table_ranges.clone();
                    let task = cx.spawn(async move |this, cx| {
                        let ranges = cx
                            .background_executor()
                            .spawn(async move { valid_table_ranges(&text, &ranges) })
                            .await;
                        let Some(this) = this.upgrade() else {
                            return;
                        };
                        let _ = this.update(cx, |state, cx| {
                            if state.table_generation == generation {
                                state.table_ranges = ranges;
                                state.table_snapshot = Some(snapshot);
                                state.table_document_revision = Some(document_revision);
                                cx.notify();
                            }
                        });
                    });
                    state.table_task = Some(task);
                }
            } else if editor.highlighter_snapshot_pending() {
                state.presentation.borrow_mut().set_pending();
                state.fallback_revision = None;
                state.invalidate_table_ranges();
            } else {
                let revision = editor.document_revision();
                if state.fallback_revision != Some(revision) {
                    state
                        .presentation
                        .borrow_mut()
                        .rebuild_fallback(editor.value().as_ref(), editor.language_name().as_ref());
                    state.fallback_revision = Some(revision);
                    state.invalidate_table_ranges();
                }
            }
            cx.notify();
        });
        Self {
            editor,
            options,
            presentation,
            fallback_revision: None,
            table_ranges: Vec::new(),
            table_snapshot: None,
            table_document_revision: None,
            table_generation: 0,
            table_task: None,
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
        self.presentation.borrow_mut().set_pending();
        self.fallback_revision = None;
        self.invalidate_table_ranges();
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
        let (range, source) = {
            let editor = self.editor.read(cx);
            if !editor.is_editable() || !editor.highlighter_ready() {
                return false;
            }
            let Some(snapshot) = editor.highlighter_snapshot::<MarkedSyntaxSnapshot>() else {
                return false;
            };
            if self.table_document_revision != Some(editor.document_revision())
                || !self
                    .table_snapshot
                    .as_ref()
                    .is_some_and(|candidate| Rc::ptr_eq(candidate, &snapshot))
            {
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
            let Some(candidate) = self.table_ranges.iter().find(|candidate| {
                candidate.start <= range.start && range.end <= candidate.end
                    || range.start <= candidate.start
                        && candidate.end <= range.end
                        && text
                            .slice(candidate.end..range.end)
                            .chars()
                            .all(|ch| matches!(ch, '\r' | '\n'))
            }) else {
                return false;
            };
            let range = candidate.clone();
            let source = text.slice(range.clone()).to_string();
            (range, source)
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
        if self.table_document_revision != Some(editor.document_revision())
            || !self
                .table_snapshot
                .as_ref()
                .is_some_and(|candidate| Rc::ptr_eq(candidate, &snapshot))
        {
            return Vec::new();
        }
        let text = editor.text();
        self.table_ranges
            .iter()
            .filter(|range| {
                range.start <= range.end
                    && range.end <= text.len()
                    && text.is_char_boundary(range.start)
                    && text.is_char_boundary(range.end)
            })
            .cloned()
            .collect()
    }

    fn invalidate_table_ranges(&mut self) {
        self.table_generation = self.table_generation.wrapping_add(1);
        self.table_ranges.clear();
        self.table_snapshot = None;
        self.table_document_revision = None;
        self.table_task.take();
    }
}

#[derive(Default)]
struct MarkedPresentation {
    snapshot: Option<Rc<MarkedSyntaxSnapshot>>,
    fallback_headings: Vec<Option<u8>>,
    fallback_backgrounds: Vec<Range<usize>>,
    code_block_background: Option<gpui::Hsla>,
    metrics_revision: u64,
}

impl MarkedPresentation {
    fn set_snapshot(&mut self, snapshot: Rc<MarkedSyntaxSnapshot>) -> bool {
        if self
            .snapshot
            .as_ref()
            .is_some_and(|current| Rc::ptr_eq(current, &snapshot))
        {
            return false;
        }
        let metrics_changed = !same_heading_metrics(self.headings(), &snapshot.heading_levels);
        self.snapshot = Some(snapshot);
        self.fallback_headings.clear();
        self.fallback_backgrounds.clear();
        if metrics_changed {
            self.metrics_revision = self.metrics_revision.wrapping_add(1);
        }
        true
    }

    fn set_pending(&mut self) -> bool {
        if self.snapshot.is_none()
            && self.fallback_headings.is_empty()
            && self.fallback_backgrounds.is_empty()
        {
            return false;
        }
        let metrics_changed = self.headings().iter().any(Option::is_some);
        self.snapshot = None;
        self.fallback_headings.clear();
        self.fallback_backgrounds.clear();
        if metrics_changed {
            self.metrics_revision = self.metrics_revision.wrapping_add(1);
        }
        true
    }

    fn rebuild_fallback(&mut self, text: &str, language: &str) {
        let previous_headings = self.headings().to_vec();
        self.snapshot = None;
        self.fallback_headings.clear();
        self.fallback_backgrounds.clear();
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
            self.fallback_headings.push(level);
            let fence =
                trimmed.starts_with("```") || trimmed.starts_with("~~~") || trimmed == "----";
            if fence {
                if in_fence {
                    if let Some(start) = fence_start.take() {
                        self.fallback_backgrounds.push(start..offset + line.len());
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
            self.fallback_backgrounds.push(start..text.len());
        }
        if !same_heading_metrics(&previous_headings, &self.fallback_headings) {
            self.metrics_revision = self.metrics_revision.wrapping_add(1);
        }
    }

    fn headings(&self) -> &[Option<u8>] {
        self.snapshot
            .as_ref()
            .map(|snapshot| snapshot.heading_levels.as_slice())
            .unwrap_or(&self.fallback_headings)
    }

    fn backgrounds(&self) -> &[Range<usize>] {
        self.snapshot
            .as_ref()
            .map(|snapshot| snapshot.code_block_ranges.as_slice())
            .unwrap_or(&self.fallback_backgrounds)
    }
}

fn same_heading_metrics(left: &[Option<u8>], right: &[Option<u8>]) -> bool {
    (0..left.len().max(right.len()))
        .all(|index| left.get(index).copied().flatten() == right.get(index).copied().flatten())
}

fn valid_table_ranges(text: &ropey::Rope, ranges: &[Range<usize>]) -> Vec<Range<usize>> {
    ranges
        .iter()
        .filter(|range| {
            range.start <= range.end
                && range.end <= text.len()
                && text.is_char_boundary(range.start)
                && text.is_char_boundary(range.end)
                && table_format::format_table(&text.slice((*range).clone()).to_string()).is_some()
        })
        .cloned()
        .collect()
}

struct MarkedDecorator(Rc<RefCell<MarkedPresentation>>);

impl InputPresentationDecorator for MarkedDecorator {
    fn line_metrics_revision(&self) -> Option<u64> {
        Some(self.0.borrow().metrics_revision)
    }

    fn line_presentation(&self, line: usize, mut default: LinePresentation) -> LinePresentation {
        let level = self.0.borrow().headings().get(line).copied().flatten();
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
            .backgrounds()
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

        let mut root = div().relative().size_full().child(
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
    use super::{MarkedEditor, MarkedEditorOptions, MarkedEditorState, MarkedPresentation};
    use crate::highlighter::MarkedSyntaxSnapshot;
    use gpui::{
        AppContext as _, Context, Entity, EntityInputHandler as _, IntoElement, ParentElement as _,
        Render, Styled as _, TestAppContext, VisualTestContext, Window, div, px,
    };
    use gpui_base::input::{InputPresentationDecorator, LinePresentation};

    struct Harness {
        marked: Entity<MarkedEditorState>,
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(MarkedEditor::new(&self.marked))
        }
    }

    #[test]
    fn marked_presentation_extracts_headings_and_fences() {
        let mut presentation = MarkedPresentation::default();
        presentation.rebuild_fallback("# Title\nbody\n```\ncode\n```\n", "markdown");
        assert_eq!(presentation.headings()[0], Some(1));
        assert_eq!(presentation.headings()[1], None);
        assert_eq!(presentation.backgrounds().len(), 1);
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

    #[test]
    fn marked_presentation_revision_only_tracks_line_metrics() {
        let mut presentation = MarkedPresentation::default();
        assert!(
            presentation.set_snapshot(std::rc::Rc::new(MarkedSyntaxSnapshot {
                heading_levels: vec![None],
                table_ranges: Vec::new(),
                code_block_ranges: vec![0..4],
            }))
        );
        assert_eq!(presentation.metrics_revision, 0);

        assert!(
            presentation.set_snapshot(std::rc::Rc::new(MarkedSyntaxSnapshot {
                heading_levels: vec![None],
                table_ranges: Vec::new(),
                code_block_ranges: vec![5..9],
            }))
        );
        assert_eq!(presentation.metrics_revision, 0);

        assert!(
            presentation.set_snapshot(std::rc::Rc::new(MarkedSyntaxSnapshot {
                heading_levels: vec![Some(2)],
                table_ranges: Vec::new(),
                code_block_ranges: Vec::new(),
            }))
        );
        assert_eq!(presentation.metrics_revision, 1);
        assert!(presentation.set_pending());
        assert_eq!(presentation.metrics_revision, 2);
    }

    #[gpui::test]
    fn readonly_marked_editor_rejects_table_format(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let mut marked = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let state = cx.new(|cx| {
                let mut state = MarkedEditorState::new(
                    MarkedEditorOptions {
                        readonly: true,
                        ..Default::default()
                    },
                    window,
                    cx,
                );
                state.set_value("| A | B |\n| --- | --- |\n| x | y |\n", window, cx);
                state
            });
            marked = Some(state.clone());
            Harness { marked: state }
        });
        let marked = marked.expect("marked editor was created");

        VisualTestContext::update(cx, |window, cx| {
            let changed = marked.update(cx, |state, cx| {
                state.format_table_at(0..usize::MAX, window, cx)
            });
            assert!(!changed);
            assert!(marked.read(cx).table_ranges(cx).is_empty());
        });
    }

    #[cfg(feature = "tree-sitter-markdown")]
    #[gpui::test]
    fn table_format_is_one_undoable_edit(cx: &mut TestAppContext) {
        use gpui_base::input::Undo;

        const SOURCE: &str = "| A | Longer |\n| --- | --- |\n| 日本 | x |\n";
        cx.update(crate::init);
        let mut marked = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let state = cx.new(|cx| {
                let mut state = MarkedEditorState::with_language("markdown", window, cx);
                state.set_value(SOURCE, window, cx);
                state
            });
            marked = Some(state.clone());
            Harness { marked: state }
        });
        let marked = marked.expect("marked editor was created");
        VisualTestContext::update(cx, |window, cx| {
            let _ = window.draw(cx);
        });
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(200));
        VisualTestContext::run_until_parked(cx);

        let editor = marked.read_with(cx, |state, _| state.editor().clone());
        VisualTestContext::update(cx, |window, cx| {
            assert!(editor.read(cx).highlighter_ready());
            assert!(marked.update(cx, |state, cx| {
                state.format_table_at(0..SOURCE.len(), window, cx)
            }));
            assert_ne!(editor.read(cx).value().as_ref(), SOURCE);
            editor.update(cx, |state, cx| state.focus(window, cx));
        });
        VisualTestContext::update(cx, |window, cx| {
            let _ = window.draw(cx);
        });
        VisualTestContext::update(cx, |window, cx| {
            window.dispatch_action(Box::new(Undo), cx);
        });
        VisualTestContext::run_until_parked(cx);
        assert_eq!(editor.read_with(cx, |state, _| state.value()), SOURCE);
    }

    #[cfg(feature = "tree-sitter-markdown")]
    #[gpui::test]
    fn stale_table_action_is_rejected_before_reparse(cx: &mut TestAppContext) {
        const SOURCE: &str = "| A | B |\n| --- | --- |\n| x | y |\n";
        cx.update(crate::init);
        let mut marked = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let state = cx.new(|cx| {
                let mut state = MarkedEditorState::with_language("markdown", window, cx);
                state.set_value(SOURCE, window, cx);
                state
            });
            marked = Some(state.clone());
            Harness { marked: state }
        });
        let marked = marked.expect("marked editor was created");
        VisualTestContext::update(cx, |window, cx| {
            let _ = window.draw(cx);
        });
        VisualTestContext::run_until_parked(cx);
        let editor = marked.read_with(cx, |state, _| state.editor().clone());

        VisualTestContext::update(cx, |window, cx| {
            editor.update(cx, |state, cx| {
                state.replace_text_in_range(Some(0..1), "X", window, cx);
            });
            assert!(!marked.update(cx, |state, cx| {
                state.format_table_at(0..SOURCE.len(), window, cx)
            }));
        });
    }

    #[cfg(feature = "tree-sitter-markdown")]
    #[gpui::test]
    fn unchanged_notifications_reuse_snapshot_and_presentation(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let mut marked = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let state = cx.new(|cx| {
                let mut state = MarkedEditorState::with_language("markdown", window, cx);
                state.set_value("# 見出し\n\n本文\n", window, cx);
                state
            });
            marked = Some(state.clone());
            Harness { marked: state }
        });
        let marked = marked.expect("marked editor was created");
        VisualTestContext::update(cx, |window, cx| {
            let _ = window.draw(cx);
        });
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(200));
        VisualTestContext::run_until_parked(cx);

        let editor = marked.read_with(cx, |state, _| state.editor().clone());
        let (first_snapshot, first_metrics_revision) = marked.read_with(cx, |state, cx| {
            (
                editor
                    .read(cx)
                    .highlighter_snapshot::<MarkedSyntaxSnapshot>()
                    .expect("snapshot"),
                state.presentation.borrow().metrics_revision,
            )
        });

        VisualTestContext::update(cx, |_, cx| {
            for _ in 0..100 {
                editor.update(cx, |_, cx| cx.notify());
            }
        });
        VisualTestContext::run_until_parked(cx);

        marked.read_with(cx, |state, cx| {
            let current_snapshot = editor
                .read(cx)
                .highlighter_snapshot::<MarkedSyntaxSnapshot>()
                .expect("snapshot");
            assert!(std::rc::Rc::ptr_eq(&first_snapshot, &current_snapshot));
            assert_eq!(
                state.presentation.borrow().metrics_revision,
                first_metrics_revision
            );
        });
    }

    #[cfg(feature = "tree-sitter-markdown")]
    #[gpui::test]
    fn latest_large_document_wins_over_cancelled_snapshot(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let body = "paragraph text\n".repeat(24_000);
        let first = format!("## Aaa\n{body}");
        let latest = format!("### Bb\n{body}");
        assert_eq!(first.len(), latest.len());

        let mut marked = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let state = cx.new(|cx| {
                let mut state = MarkedEditorState::with_language("markdown", window, cx);
                state.set_value(first.clone(), window, cx);
                state
            });
            marked = Some(state.clone());
            Harness { marked: state }
        });
        let marked = marked.expect("marked editor was created");

        VisualTestContext::update(cx, |window, cx| {
            let _ = window.draw(cx);
        });
        VisualTestContext::update(cx, |window, cx| {
            marked.update(cx, |state, cx| state.set_value(latest.clone(), window, cx));
            let _ = window.draw(cx);
        });
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(400));
        VisualTestContext::run_until_parked(cx);

        marked.read_with(cx, |state, cx| {
            let editor = state.editor().read(cx);
            assert_eq!(editor.value().as_ref(), latest);
            assert!(editor.highlighter_ready());
            assert!(!editor.highlighter_snapshot_pending());
            let snapshot = editor
                .highlighter_snapshot::<MarkedSyntaxSnapshot>()
                .expect("latest snapshot");
            assert_eq!(snapshot.heading_levels[0], Some(3));
        });
    }
}
