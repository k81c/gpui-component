use std::{
    any::Any,
    cell::{Cell, RefCell},
    ops::Range,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use gpui::{HighlightStyle, SharedString, Task};
use gpui_base::input::{
    EditorState, FoldRange, HighlightStyleResolver, InputEdit as BaseInputEdit, InputHighlighter,
    InputHighlighterFactory, RopeExt as _,
};
use ropey::{LineType, Rope};
use tree_sitter::{InputEdit, ParseOptions, Point};

use super::{
    LanguageRegistry, MarkedSyntaxSnapshot, SyntaxHighlightUpdate, SyntaxHighlighter, WindowedTree,
};

pub(crate) fn input_highlighter_factory() -> InputHighlighterFactory {
    Rc::new(|language| {
        LanguageRegistry::singleton().has_parser(language).then(|| {
            Box::new(TreeSitterInputHighlighter::new(language)) as Box<dyn InputHighlighter>
        })
    })
}

struct TreeSitterInputHighlighter {
    inner: Rc<RefCell<SyntaxHighlighter>>,
    parse_task: Rc<RefCell<Option<Task<()>>>>,
    windowed_parse_task: Rc<RefCell<Option<Task<()>>>>,
    snapshot_task: Rc<RefCell<Option<Task<()>>>>,
    ready: Rc<Cell<bool>>,
    snapshot: Rc<RefCell<Option<Rc<MarkedSyntaxSnapshot>>>>,
    snapshot_pending: Rc<Cell<bool>>,
    full_parse_pending: Rc<Cell<bool>>,
    request_generation: Rc<Cell<u64>>,
}

struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

impl TreeSitterInputHighlighter {
    fn new(language: &str) -> Self {
        Self {
            inner: Rc::new(RefCell::new(SyntaxHighlighter::new(language))),
            parse_task: Rc::new(RefCell::new(None)),
            windowed_parse_task: Rc::new(RefCell::new(None)),
            snapshot_task: Rc::new(RefCell::new(None)),
            ready: Rc::new(Cell::new(false)),
            snapshot: Rc::new(RefCell::new(None)),
            snapshot_pending: Rc::new(Cell::new(false)),
            full_parse_pending: Rc::new(Cell::new(false)),
            request_generation: Rc::new(Cell::new(0)),
        }
    }
}

impl SyntaxHighlighter {
    pub(crate) fn update_input(
        &mut self,
        edit: Option<BaseInputEdit>,
        text: &Rope,
        timeout: Option<Duration>,
    ) -> bool {
        self.update(edit.map(to_tree_sitter_edit), text, timeout)
    }

    fn update_input_with_status(
        &mut self,
        edit: Option<BaseInputEdit>,
        text: &Rope,
        timeout: Option<Duration>,
    ) -> SyntaxHighlightUpdate {
        self.update_with_status(edit.map(to_tree_sitter_edit), text, timeout)
    }
}

impl InputHighlighter for TreeSitterInputHighlighter {
    fn language(&self) -> SharedString {
        self.inner.borrow().language().clone()
    }

    fn is_ready(&self) -> bool {
        self.ready.get()
    }

    fn document_snapshot(&self) -> Option<Rc<dyn Any>> {
        self.snapshot
            .borrow()
            .as_ref()
            .cloned()
            .map(|snapshot| snapshot as Rc<dyn Any>)
    }

    fn document_snapshot_pending(&self) -> bool {
        self.snapshot_pending.get()
    }

    fn update(
        &mut self,
        edit: Option<BaseInputEdit>,
        text: &Rope,
        folding: bool,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<EditorState>,
    ) {
        const SYNC_PARSE_TIMEOUT: Duration = Duration::from_millis(2);
        const SYNC_PARSE_MAX_BYTES: usize = 256 * 1024;
        const PARSE_DEBOUNCE: Duration = Duration::from_millis(150);

        let marked_snapshot_supported = matches!(
            self.inner.borrow().language().as_ref(),
            "markdown" | "djot" | "asciidoc"
        );
        let generation = self.request_generation.get().wrapping_add(1);
        self.request_generation.set(generation);
        self.snapshot.borrow_mut().take();
        self.snapshot_pending.set(marked_snapshot_supported);
        self.snapshot_task.borrow_mut().take();

        let edit_start_byte = edit.as_ref().map(|edit| edit.start_byte).unwrap_or(0);
        // Capture reusable injection trees before the foreground update clears
        // stale renderable layers. Exact unchanged ranges remain reusable by
        // the background parse without exposing stale highlights meanwhile.
        let injection_data = self.inner.borrow().injection_parse_data();
        self.ready.set(false);
        let reuse_pending_tree =
            self.full_parse_pending.get() && self.inner.borrow().text().eq(text);
        let status = if reuse_pending_tree {
            SyntaxHighlightUpdate::TimedOut
        } else {
            let mut highlighter = self.inner.borrow_mut();
            if text.len() > SYNC_PARSE_MAX_BYTES {
                highlighter.edit_tree(edit.map(to_tree_sitter_edit), text);
                SyntaxHighlightUpdate::TimedOut
            } else {
                highlighter.update_input_with_status(edit, text, Some(SYNC_PARSE_TIMEOUT))
            }
        };
        self.full_parse_pending
            .set(status == SyntaxHighlightUpdate::TimedOut);
        if marked_snapshot_supported && status != SyntaxHighlightUpdate::TimedOut {
            let language = self.inner.borrow().language().clone();
            let tree = self.inner.borrow().tree().cloned();
            let snapshot = self.snapshot.clone();
            let snapshot_pending = self.snapshot_pending.clone();
            let request_generation = self.request_generation.clone();
            let text = text.clone();
            let cancel = Arc::new(AtomicBool::new(false));
            let task = cx.spawn_in(window, async move |entity, cx| {
                let _cancel_guard = CancelOnDrop(cancel.clone());
                let result = cx
                    .background_executor()
                    .spawn({
                        let cancel = cancel.clone();
                        async move {
                            SyntaxHighlighter::marked_syntax_snapshot_from(
                                &language,
                                &text,
                                tree.as_ref(),
                                Some(&cancel),
                            )
                        }
                    })
                    .await;
                if request_generation.get() != generation {
                    return;
                }
                snapshot_pending.set(false);
                *snapshot.borrow_mut() = result.map(Rc::new);
                let _ = entity.update(cx, |_, cx| cx.notify());
            });
            self.snapshot_task.borrow_mut().replace(task);
        }
        if status == SyntaxHighlightUpdate::Complete {
            self.ready.set(true);
            self.parse_task.borrow_mut().take();
            self.windowed_parse_task.borrow_mut().take();
            return;
        }

        let highlighter = self.inner.clone();
        let parse_task = self.parse_task.clone();
        let windowed_parse_task = self.windowed_parse_task.clone();
        let ready = self.ready.clone();
        let snapshot = self.snapshot.clone();
        let snapshot_pending = self.snapshot_pending.clone();
        let full_parse_pending = self.full_parse_pending.clone();
        let request_generation = self.request_generation.clone();
        let language = highlighter.borrow().language().clone();
        let old_tree = highlighter.borrow().tree().cloned();
        let pre_parsed_tree = (status == SyntaxHighlightUpdate::PendingInjections)
            .then(|| highlighter.borrow().tree().cloned())
            .flatten();
        let needs_snapshot = marked_snapshot_supported && status == SyntaxHighlightUpdate::TimedOut;
        let full_tree_revision = highlighter.borrow().full_tree_revision();
        let parse_window = (status == SyntaxHighlightUpdate::TimedOut)
            .then(|| compute_parse_window(text, edit_start_byte, None))
            .flatten();
        let text = text.clone();
        let text_for_apply = text.clone();

        if let Some(parse_window) = parse_window {
            let highlighter = highlighter.clone();
            let language = language.clone();
            let text = text.clone();
            let text_for_apply = text.clone();
            let windowed_request_generation = request_generation.clone();
            let cancel = Arc::new(AtomicBool::new(false));
            let task = cx.spawn_in(window, async move |entity, cx| {
                let _cancel_guard = CancelOnDrop(cancel.clone());
                let parse_cancel = cancel.clone();
                let window_range = parse_window.clone();
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let (mut parser, grammar) =
                            LanguageRegistry::singleton().parser(&language).ok()?;
                        parser.set_language(&grammar).ok()?;
                        let start_point = text.offset_to_point(window_range.start);
                        let end_point = text.offset_to_point(window_range.end);
                        let included_range = tree_sitter::Range {
                            start_byte: window_range.start,
                            end_byte: window_range.end,
                            start_point: Point::new(start_point.row, start_point.column),
                            end_point: Point::new(end_point.row, end_point.column),
                        };
                        parser.set_included_ranges(&[included_range]).ok()?;
                        let mut progress = |_: &tree_sitter::ParseState| {
                            if parse_cancel.load(Ordering::Relaxed) {
                                std::ops::ControlFlow::Break(())
                            } else {
                                std::ops::ControlFlow::Continue(())
                            }
                        };
                        let options = ParseOptions::new().progress_callback(&mut progress);
                        let tree = parser.parse_with_options(
                            &mut |offset, _| {
                                if offset >= text.len() {
                                    ""
                                } else {
                                    let (chunk, chunk_byte_ix) = text.chunk(offset);
                                    &chunk[offset - chunk_byte_ix..]
                                }
                            },
                            None,
                            Some(options),
                        )?;
                        (!parse_cancel.load(Ordering::Relaxed)).then_some(WindowedTree {
                            byte_range: window_range,
                            tree,
                        })
                    })
                    .await;
                if windowed_request_generation.get() != generation {
                    return;
                }
                if let Some(windowed_tree) = result {
                    let applied = highlighter.borrow_mut().apply_windowed_tree(
                        windowed_tree,
                        &text_for_apply,
                        full_tree_revision,
                    );
                    if applied {
                        let _ = entity.update(cx, |_, cx| cx.notify());
                    }
                }
            });
            windowed_parse_task.borrow_mut().replace(task);
        } else {
            windowed_parse_task.borrow_mut().take();
        }

        let cancel = Arc::new(AtomicBool::new(false));

        let task = cx.spawn_in(window, async move |entity, cx| {
            let _cancel_guard = CancelOnDrop(cancel.clone());
            cx.background_executor().timer(PARSE_DEBOUNCE).await;

            let parse_cancel = cancel.clone();
            let parse_text = text.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let (mut parser, grammar) =
                        LanguageRegistry::singleton().parser(&language).ok()?;
                    parser.set_language(&grammar).ok()?;
                    let mut progress = |_: &tree_sitter::ParseState| {
                        if parse_cancel.load(Ordering::Relaxed) {
                            std::ops::ControlFlow::Break(())
                        } else {
                            std::ops::ControlFlow::Continue(())
                        }
                    };
                    let options = ParseOptions::new().progress_callback(&mut progress);
                    let tree = if let Some(tree) = pre_parsed_tree {
                        tree
                    } else {
                        parser.parse_with_options(
                            &mut |offset, _| {
                                if offset >= parse_text.len() {
                                    ""
                                } else {
                                    let (chunk, chunk_byte_ix) = parse_text.chunk(offset);
                                    &chunk[offset - chunk_byte_ix..]
                                }
                            },
                            old_tree.as_ref(),
                            Some(options),
                        )?
                    };
                    if parse_cancel.load(Ordering::Relaxed) {
                        return None;
                    }
                    let marked_snapshot = if needs_snapshot {
                        Some(SyntaxHighlighter::marked_syntax_snapshot_from(
                            &language,
                            &parse_text,
                            Some(&tree),
                            Some(&parse_cancel),
                        )?)
                    } else {
                        None
                    };
                    let folds = if folding {
                        extract_fold_ranges_with_cancel(&tree, Some(&parse_cancel))?
                    } else {
                        Vec::new()
                    };
                    Some((tree, folds, marked_snapshot))
                })
                .await;

            if request_generation.get() != generation {
                return;
            }
            if let Some((tree, folds, marked_snapshot)) = result {
                let tree_for_injections = tree.clone();
                let applied_revision = {
                    let mut highlighter = highlighter.borrow_mut();
                    highlighter
                        .apply_background_tree(tree, &text_for_apply, Vec::new())
                        .then(|| highlighter.full_tree_revision())
                };
                if let Some(applied_revision) = applied_revision {
                    ready.set(true);
                    full_parse_pending.set(false);
                    if let Some(marked_snapshot) = marked_snapshot {
                        snapshot_pending.set(false);
                        *snapshot.borrow_mut() = Some(Rc::new(marked_snapshot));
                    }
                    let _ = entity.update(cx, |state, cx| {
                        state.apply_highlighter_fold_candidates(folds, cx);
                    });

                    let Some(injection_data) = injection_data else {
                        return;
                    };
                    if cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    let injection_cancel = cancel.clone();
                    let injection_text = text.clone();
                    let injections = cx
                        .background_executor()
                        .spawn(async move {
                            SyntaxHighlighter::compute_injection_layers_with_cancel(
                                injection_data,
                                &tree_for_injections,
                                &injection_text,
                                Some(&injection_cancel),
                            )
                        })
                        .await;
                    if request_generation.get() != generation || cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    let applied = highlighter.borrow_mut().apply_background_injections(
                        &text_for_apply,
                        applied_revision,
                        injections,
                    );
                    if applied {
                        let _ = entity.update(cx, |_, cx| cx.notify());
                    }
                    return;
                }
            }
            if needs_snapshot {
                snapshot_pending.set(false);
            }
            full_parse_pending.set(false);
            let _ = entity.update(cx, |_, cx| cx.notify());
        });
        parse_task.borrow_mut().replace(task);
    }

    fn styles(
        &self,
        range: &Range<usize>,
        resolver: &dyn HighlightStyleResolver,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        self.inner.borrow().styles(range, resolver)
    }
    fn fold_ranges(&self, _: &Rope) -> Vec<FoldRange> {
        self.inner
            .borrow()
            .tree()
            .map(extract_fold_ranges)
            .unwrap_or_default()
    }

    fn fold_ranges_for_edit(&self, range: Range<usize>, _: &Rope) -> Vec<FoldRange> {
        self.inner
            .borrow()
            .tree()
            .map(|tree| extract_fold_ranges_in_range(tree, range))
            .unwrap_or_default()
    }
}

fn compute_parse_window(
    text: &Rope,
    edit_byte: usize,
    visible_byte_range: Option<Range<usize>>,
) -> Option<Range<usize>> {
    const WINDOW_MARGIN_LINES: usize = 300;
    let total_lines = text.len_lines(LineType::LF);
    if total_lines < WINDOW_MARGIN_LINES * 2 {
        return None;
    }

    let edit_line = text.offset_to_point(edit_byte.min(text.len())).row;
    let (visible_start, visible_end) = visible_byte_range
        .map(|range| {
            (
                text.offset_to_point(range.start.min(text.len())).row,
                text.offset_to_point(range.end.min(text.len())).row,
            )
        })
        .unwrap_or((edit_line, edit_line));
    let raw_start = edit_line
        .min(visible_start)
        .saturating_sub(WINDOW_MARGIN_LINES);
    let raw_end =
        (edit_line.max(visible_end) + WINDOW_MARGIN_LINES).min(total_lines.saturating_sub(1));

    let start_line = (0..=raw_start)
        .rev()
        .find(|line| text.slice_line(*line).to_string().trim().is_empty())
        .unwrap_or(0);
    let end_line = (raw_end..total_lines)
        .find(|line| text.slice_line(*line).to_string().trim().is_empty())
        .unwrap_or(total_lines.saturating_sub(1));
    let end_byte = if end_line + 1 >= total_lines {
        text.len()
    } else {
        text.line_start_offset(end_line + 1)
    };
    Some(text.line_start_offset(start_line)..end_byte)
}

fn to_tree_sitter_edit(edit: BaseInputEdit) -> InputEdit {
    InputEdit {
        start_byte: edit.start_byte,
        old_end_byte: edit.old_end_byte,
        new_end_byte: edit.new_end_byte,
        start_position: Point::new(edit.start_position.row, edit.start_position.column),
        old_end_position: Point::new(edit.old_end_position.row, edit.old_end_position.column),
        new_end_position: Point::new(edit.new_end_position.row, edit.new_end_position.column),
    }
}

fn extract_fold_ranges(tree: &tree_sitter::Tree) -> Vec<FoldRange> {
    extract_fold_ranges_with_cancel(tree, None).unwrap_or_default()
}

fn extract_fold_ranges_in_range(
    tree: &tree_sitter::Tree,
    byte_range: Range<usize>,
) -> Vec<FoldRange> {
    extract_fold_ranges_in_range_with_cancel(tree, byte_range, None).unwrap_or_default()
}

fn extract_fold_ranges_with_cancel(
    tree: &tree_sitter::Tree,
    cancelled: Option<&AtomicBool>,
) -> Option<Vec<FoldRange>> {
    extract_fold_ranges_in_range_with_cancel(tree, 0..usize::MAX, cancelled)
}

fn extract_fold_ranges_in_range_with_cancel(
    tree: &tree_sitter::Tree,
    byte_range: Range<usize>,
    cancelled: Option<&AtomicBool>,
) -> Option<Vec<FoldRange>> {
    let root = tree.root_node();
    let mut stack = Vec::new();
    let mut cursor = root.walk();
    stack.extend(root.named_children(&mut cursor));
    let mut ranges = Vec::new();

    while let Some(node) = stack.pop() {
        if cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return None;
        }
        if node.end_byte() <= byte_range.start || node.start_byte() >= byte_range.end {
            continue;
        }
        let start = node.start_position().row;
        let end = node.end_position().row;
        if end.saturating_sub(start) < 2 {
            continue;
        }
        ranges.push(FoldRange::new(start, end));
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor));
    }
    ranges.sort_by_key(|range| range.start_line);
    ranges.dedup_by_key(|range| range.start_line);
    Some(ranges)
}

#[cfg(test)]
mod tests {
    use super::compute_parse_window;
    use gpui_base::input::RopeExt as _;
    use ropey::Rope;

    #[test]
    fn parse_window_covers_edit_and_viewport_at_paragraph_boundaries() {
        let source = (0..900)
            .map(|line| {
                if line % 100 == 0 {
                    "\n".to_string()
                } else {
                    format!("line {line}\n")
                }
            })
            .collect::<String>();
        let text = Rope::from(source);
        let edit = text.line_start_offset(650);
        let visible = text.line_start_offset(620)..text.line_start_offset(680);

        let range = compute_parse_window(&text, edit, Some(visible.clone())).expect("window");

        assert!(range.start <= edit && edit <= range.end);
        assert!(range.start <= visible.start && visible.end <= range.end);
        assert!(range.end - range.start < text.len());
        assert!(text.slice(0..range.start).to_string().ends_with('\n'));
    }

    #[test]
    fn parse_window_skips_small_documents() {
        let text = Rope::from("one\ntwo\n");
        assert!(compute_parse_window(&text, 0, None).is_none());
    }
}
