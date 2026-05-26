use crate::highlighter::{HighlightTheme, LanguageRegistry};
use rustc_hash::FxHashMap;

use anyhow::{Context, Result, anyhow};
use gpui::{HighlightStyle, SharedString};

use ropey::{ChunkCursor, LineType, Rope};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{
    collections::BTreeSet,
    ops::{ControlFlow, Range},
    usize,
};
use tree_sitter::{
    InputEdit, ParseOptions, Parser, Point, Query, QueryCursor, StreamingIterator, Tree,
};

/// When a node spans more than this many bytes beyond the requested query
/// range, we recurse into its children instead of querying it directly.
const LARGE_NODE_THRESHOLD: usize = 8 * 1024;

/// A syntax highlighter that supports incremental parsing, multiline text,
/// and caching of highlight results.
#[allow(unused)]
pub struct SyntaxHighlighter {
    language: SharedString,
    query: Option<Query>,
    /// The full injections query. This is used to build injection layers during parsing.
    injections_query: Option<Arc<Query>>,
    injection_queries: FxHashMap<SharedString, Query>,

    locals_pattern_index: usize,
    highlights_pattern_index: usize,
    // highlight_indices: Vec<Option<Highlight>>,
    non_local_variable_patterns: Vec<bool>,
    injection_content_capture_index: Option<u32>,
    injection_language_capture_index: Option<u32>,
    local_scope_capture_index: Option<u32>,
    local_def_capture_index: Option<u32>,
    local_def_value_capture_index: Option<u32>,
    local_ref_capture_index: Option<u32>,

    /// The last parsed source text.
    text: Rope,
    parser: Parser,
    /// The last parsed tree.
    tree: Option<Tree>,

    /// Parsed injection trees.
    /// These are built once in update() and queried multiple times in match_styles().
    injection_layers: Vec<InjectionLayer>,
    /// A tree parsed over a limited window for fast initial highlighting.
    /// Preferred over `tree` when the query range falls within its `byte_range`.
    /// Cleared when a full tree is applied via `apply_background_tree`.
    windowed_tree: Option<WindowedTree>,
    /// Incremented each time a full (non-windowed) tree is successfully applied.
    /// `apply_windowed_tree` refuses to apply if this has changed since the
    /// windowed parse was spawned, preventing a stale partial result from
    /// overwriting a more-complete full tree.
    full_tree_revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyntaxHighlightUpdate {
    Complete,
    PendingInjections,
    TimedOut,
}

/// A syntax tree parsed over a limited byte range for fast initial highlighting
/// while a full background parse is pending. Cleared when a complete tree is applied.
pub(crate) struct WindowedTree {
    /// The byte range of the document that this tree covers.
    pub(crate) byte_range: Range<usize>,
    pub(crate) tree: Tree,
}

/// A parsed injection layer.
/// Stores the parsed tree and the ranges it covers.
pub(crate) struct InjectionLayer {
    pub(crate) language_name: SharedString,
    pub(crate) byte_range: Range<usize>,
    pub(crate) tree: Tree,
}

/// Data needed to compute injection layers on a background thread.
pub(crate) struct InjectionParseData {
    pub(crate) query: Arc<Query>,
    pub(crate) content_capture_index: Option<u32>,
    pub(crate) language_capture_index: Option<u32>,
    /// Old injection trees that can be reused when the injected ranges are unchanged.
    pub(crate) old_layers: Vec<ReusableInjectionLayer>,
    /// The edit that produced the current text, used to incrementally update
    /// old injection trees before comparing their included ranges.
    pub(crate) edit: InputEdit,
}

pub(crate) struct ReusableInjectionLayer {
    pub(crate) language_name: SharedString,
    pub(crate) tree: Tree,
}

struct TextProvider<'a>(&'a Rope);
struct ByteChunks<'a> {
    cursor: ChunkCursor<'a>,
    node_start: usize,
    node_end: usize,
    at_first: bool,
}
impl<'a> tree_sitter::TextProvider<&'a [u8]> for TextProvider<'a> {
    type I = ByteChunks<'a>;

    fn text(&mut self, node: tree_sitter::Node) -> Self::I {
        let range = node.byte_range();
        let cursor = self.0.chunk_cursor_at(range.start);

        ByteChunks {
            cursor,
            node_start: range.start,
            node_end: range.end,
            at_first: true,
        }
    }
}

impl<'a> Iterator for ByteChunks<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if !self.at_first {
            if !self.cursor.next() {
                return None;
            }
        }
        self.at_first = false;

        let chunk_byte_start = self.cursor.byte_offset();
        if chunk_byte_start >= self.node_end {
            return None;
        }

        let chunk = self.cursor.chunk().as_bytes();

        // Slice the chunk to only include bytes within the node's range.
        let start_in_chunk = self.node_start.saturating_sub(chunk_byte_start);
        let end_in_chunk = (self.node_end - chunk_byte_start).min(chunk.len());

        if start_in_chunk >= end_in_chunk {
            return None;
        }

        Some(&chunk[start_in_chunk..end_in_chunk])
    }
}

#[derive(Debug, Default, Clone)]
struct HighlightSummary {
    count: usize,
    start: usize,
    end: usize,
    min_start: usize,
    max_end: usize,
}

/// The highlight item, the range is offset of the token in the tree.
#[derive(Debug, Default, Clone)]
struct HighlightItem {
    /// The byte range of the highlight in the text.
    range: Range<usize>,
    /// The highlight name, like `function`, `string`, `comment`, etc.
    name: SharedString,
}

impl HighlightItem {
    pub fn new(range: Range<usize>, name: impl Into<SharedString>) -> Self {
        Self {
            range,
            name: name.into(),
        }
    }
}

impl sum_tree::Item for HighlightItem {
    type Summary = HighlightSummary;
    fn summary(&self, _cx: &()) -> Self::Summary {
        HighlightSummary {
            count: 1,
            start: self.range.start,
            end: self.range.end,
            min_start: self.range.start,
            max_end: self.range.end,
        }
    }
}

impl sum_tree::Summary for HighlightSummary {
    type Context<'a> = &'a ();
    fn zero(_: Self::Context<'_>) -> Self {
        HighlightSummary {
            count: 0,
            start: usize::MIN,
            end: usize::MAX,
            min_start: usize::MAX,
            max_end: usize::MIN,
        }
    }

    fn add_summary(&mut self, other: &Self, _: Self::Context<'_>) {
        self.min_start = self.min_start.min(other.min_start);
        self.max_end = self.max_end.max(other.max_end);
        self.start = other.start;
        self.end = other.end;
        self.count += other.count;
    }
}

impl<'a> sum_tree::Dimension<'a, HighlightSummary> for usize {
    fn zero(_: &()) -> Self {
        0
    }

    fn add_summary(&mut self, _: &'a HighlightSummary, _: &()) {}
}

impl<'a> sum_tree::Dimension<'a, HighlightSummary> for Range<usize> {
    fn zero(_: &()) -> Self {
        Default::default()
    }

    fn add_summary(&mut self, summary: &'a HighlightSummary, _: &()) {
        self.start = summary.start;
        self.end = summary.end;
    }
}

impl SyntaxHighlighter {
    /// Create a new SyntaxHighlighter for the given language.
    pub fn new(lang: &str) -> Self {
        match Self::build_for_language(&lang) {
            Ok(result) => result,
            Err(err) => {
                tracing::warn!(
                    "SyntaxHighlighter init failed, fallback to use `text`, {}",
                    err
                );
                Self::build_for_language("text").unwrap()
            }
        }
    }

    /// Build the highlighter for the given language.
    ///
    /// https://github.com/tree-sitter/tree-sitter/blob/v0.26.8/crates/highlight/src/highlight.rs#L339
    fn build_for_language(lang: &str) -> Result<Self> {
        let Some(config) = LanguageRegistry::singleton().language(&lang) else {
            return Err(anyhow!(
                "language {:?} is not registered in `LanguageRegistry`",
                lang
            ));
        };

        let mut parser = Parser::new();
        parser
            .set_language(&config.language)
            .context("parse set_language")?;

        // Concatenate the query strings, keeping track of the start offset of each section.
        let mut query_source = String::new();
        query_source.push_str(&config.injections);
        let locals_query_offset = query_source.len();
        query_source.push_str(&config.locals);
        let highlights_query_offset = query_source.len();
        query_source.push_str(&config.highlights);

        // Construct a single query by concatenating the three query strings, but record the
        // range of pattern indices that belong to each individual string.
        let mut query = Query::new(&config.language, &query_source).context("new query")?;

        let mut locals_pattern_index = 0;
        let mut highlights_pattern_index = 0;
        for i in 0..(query.pattern_count()) {
            let pattern_offset = query.start_byte_for_pattern(i);
            if pattern_offset < highlights_query_offset {
                if pattern_offset < highlights_query_offset {
                    highlights_pattern_index += 1;
                }
                if pattern_offset < locals_query_offset {
                    locals_pattern_index += 1;
                }
            }
        }

        // Separate combined injection patterns into their own query.
        // Combined injections (e.g., PHP's HTML text nodes) collect all matching
        // ranges and parse them as a single document, so that opening/closing
        // tags across injection boundaries are correctly matched.
        let combined_injections_query = if !config.injections.is_empty() {
            if let Ok(mut ciq) = Query::new(&config.language, &config.injections) {
                let mut has_combined_query = false;
                // Scan the injection query's own patterns for injection.combined.
                // (Previously this scanned the highlights query, which never contains
                // injection.combined for languages like asciidoc that keep highlights
                // and injections in separate .scm files.)
                for pattern_index in 0..ciq.pattern_count() {
                    let settings = ciq.property_settings(pattern_index);
                    if settings.iter().any(|s| &*s.key == "injection.combined") {
                        has_combined_query = true;
                    } else {
                        ciq.disable_pattern(pattern_index);
                    }
                }
                // Also disable injection patterns from the main highlights query
                // to avoid duplicate processing.
                for pattern_index in 0..locals_pattern_index {
                    let settings = query.property_settings(pattern_index);
                    if settings.iter().any(|s| &*s.key == "injection.combined") {
                        query.disable_pattern(pattern_index);
                    }
                }
                if has_combined_query {
                    Some(Arc::new(ciq))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // Injection layers are computed separately during parsing, so do not
        // emit injection captures from the main highlight query.
        for pattern_index in 0..locals_pattern_index {
            query.disable_pattern(pattern_index);
        }

        // Find all of the highlighting patterns that are disabled for nodes that
        // have been identified as local variables.
        let non_local_variable_patterns = (0..query.pattern_count())
            .map(|i| {
                query
                    .property_predicates(i)
                    .iter()
                    .any(|(prop, positive)| !*positive && prop.key.as_ref() == "local")
            })
            .collect();

        // Store the numeric ids for all of the special captures.
        let injection_content_capture_index = combined_injections_query.as_ref().and_then(|q| {
            q.capture_names()
                .iter()
                .position(|name| *name == "injection.content")
                .map(|i| i as u32)
        });
        let injection_language_capture_index = combined_injections_query.as_ref().and_then(|q| {
            q.capture_names()
                .iter()
                .position(|name| *name == "injection.language")
                .map(|i| i as u32)
        });
        let mut local_def_capture_index = None;
        let mut local_def_value_capture_index = None;
        let mut local_ref_capture_index = None;
        let mut local_scope_capture_index = None;
        for (i, name) in query.capture_names().iter().enumerate() {
            let i = Some(i as u32);
            match *name {
                "local.definition" => local_def_capture_index = i,
                "local.definition-value" => local_def_value_capture_index = i,
                "local.reference" => local_ref_capture_index = i,
                "local.scope" => local_scope_capture_index = i,
                _ => {}
            }
        }

        let mut injection_queries = FxHashMap::default();
        for inj_language in config.injection_languages.iter() {
            if let Some(inj_config) = LanguageRegistry::singleton().language(&inj_language) {
                match Query::new(&inj_config.language, &inj_config.highlights) {
                    Ok(q) => {
                        injection_queries.insert(inj_config.name.clone(), q);
                    }
                    Err(e) => {
                        tracing::error!(
                            "failed to build injection query for {:?}: {:?}",
                            inj_config.name,
                            e
                        );
                    }
                }
            }
        }

        // let highlight_indices = vec![None; query.capture_names().len()];

        Ok(Self {
            language: config.name.clone(),
            query: Some(query),
            injections_query: combined_injections_query,
            injection_queries,

            locals_pattern_index,
            highlights_pattern_index,
            non_local_variable_patterns,
            injection_content_capture_index,
            injection_language_capture_index,
            local_scope_capture_index,
            local_def_capture_index,
            local_def_value_capture_index,
            local_ref_capture_index,
            text: Rope::new(),
            parser,
            tree: None,
            injection_layers: Vec::new(),
            windowed_tree: None,
            full_tree_revision: 0,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.text.len() == 0
    }

    /// Get the parsed tree (if available)
    pub fn tree(&self) -> Option<&Tree> {
        self.tree.as_ref()
    }

    /// Returns the language name for this highlighter.
    pub fn language(&self) -> &SharedString {
        &self.language
    }

    /// Returns a reference to the current text.
    pub fn text(&self) -> &Rope {
        &self.text
    }

    /// Returns heading levels by buffer line for markup languages.
    ///
    /// Each line maps to `Some(1..=6)` when the line is recognized as a heading,
    /// otherwise `None`.
    pub fn heading_levels(&self) -> Vec<Option<u8>> {
        let mut levels = vec![None; self.text.len_lines(LineType::LF)];
        let Some(tree) = self.tree.as_ref() else {
            return levels;
        };

        let mut stack = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            let child_count = node.child_count();
            for i in (0..child_count).rev() {
                if let Some(child) = node.child(i as u32) {
                    stack.push(child);
                }
            }

            let Some(level) = Self::detect_heading_level(&self.language, &self.text, &node) else {
                continue;
            };
            let row = node.start_position().row;
            if row < levels.len() {
                levels[row] = Some(level);
            }
        }

        levels
    }

    pub(crate) fn heading_levels_in_rows(&self, rows: Range<usize>) -> Vec<(usize, Option<u8>)> {
        let line_count = self.text.len_lines(LineType::LF);
        let start = rows.start.min(line_count);
        let end = rows.end.min(line_count).max(start);
        let mut levels = vec![None; end.saturating_sub(start)];
        let Some(tree) = self.tree.as_ref() else {
            return (start..end).map(|row| (row, None)).collect();
        };

        let mut stack = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            let node_start = node.start_position().row;
            let node_end = node.end_position().row;
            if node_end < start || node_start >= end {
                continue;
            }

            let child_count = node.child_count();
            for i in (0..child_count).rev() {
                if let Some(child) = node.child(i as u32) {
                    stack.push(child);
                }
            }

            let Some(level) = Self::detect_heading_level(&self.language, &self.text, &node) else {
                continue;
            };
            if (start..end).contains(&node_start) {
                levels[node_start - start] = Some(level);
            }
        }

        (start..end).zip(levels).collect()
    }

    fn detect_heading_level(lang: &str, text: &Rope, node: &tree_sitter::Node) -> Option<u8> {
        let kind = node.kind();
        let byte_range = node.start_byte()..node.end_byte();
        let source = text.slice(byte_range).to_string();
        let first_line = source.lines().next().unwrap_or_default().trim_start();

        match lang {
            "markdown" => {
                if kind == "atx_heading" {
                    return Self::count_heading_marker_prefix(first_line, '#');
                }
                if kind == "setext_heading" {
                    let mut lines = source.lines();
                    let _title = lines.next();
                    let marker = lines.next().unwrap_or_default().trim();
                    if marker.starts_with('=') {
                        return Some(1);
                    }
                    if marker.starts_with('-') {
                        return Some(2);
                    }
                }
            }
            "asciidoc" => {
                return match kind {
                    "document_title" => Some(1),
                    "title1" => Some(2),
                    "title2" => Some(3),
                    "title3" => Some(4),
                    "title4" => Some(5),
                    "title5" => Some(6),
                    _ => None,
                };
            }
            "djot" => {
                if kind.contains("heading") || kind == "section" {
                    return Self::count_heading_marker_prefix(first_line, '#');
                }
            }
            _ => {}
        }

        if kind.contains("heading") {
            Self::count_heading_marker_prefix(first_line, '#')
                .or_else(|| Self::count_heading_marker_prefix(first_line, '='))
        } else {
            None
        }
    }

    fn count_heading_marker_prefix(line: &str, marker: char) -> Option<u8> {
        let count = line.chars().take_while(|ch| *ch == marker).count();
        if !(1..=6).contains(&count) {
            return None;
        }
        let next = line.chars().nth(count);
        if matches!(next, Some(' ' | '\t') | None) {
            Some(count as u8)
        } else {
            None
        }
    }

    /// Highlight the given text, returning a map from byte ranges to highlight captures.
    ///
    /// Uses incremental parsing by `edit` to efficiently update the highlighter's state.
    /// When `timeout` is `Some`, aborts if parsing exceeds the given duration
    /// and returns `false`. On timeout the old tree is preserved so highlighting
    /// still works with stale data, but `self.text` is updated so that the
    /// caller can send the current text to a background parse.
    /// When `timeout` is `None`, parsing runs to completion and always returns `true`.
    /// Returns true if this language has injection queries
    /// (i.e. it embeds other languages such as asciidoc_inline in asciidoc).
    pub fn has_injections(&self) -> bool {
        self.injections_query.is_some()
    }

    pub(crate) fn update_with_status(
        &mut self,
        edit: Option<InputEdit>,
        text: &Rope,
        timeout: Option<Duration>,
    ) -> SyntaxHighlightUpdate {
        if self.text.eq(text) {
            return SyntaxHighlightUpdate::Complete;
        }

        let edit = edit.unwrap_or(InputEdit {
            start_byte: 0,
            old_end_byte: 0,
            new_end_byte: text.len(),
            start_position: Point::new(0, 0),
            old_end_position: Point::new(0, 0),
            new_end_position: Point::new(0, 0),
        });

        let mut old_tree = self
            .tree
            .take()
            .unwrap_or(self.parser.parse("", None).unwrap());
        old_tree.edit(&edit);

        let mut timed_out = false;
        let start = Instant::now();
        let mut progress = |_: &tree_sitter::ParseState| -> ControlFlow<()> {
            let Some(budget) = timeout else {
                return ControlFlow::Continue(());
            };

            if start.elapsed() > budget {
                timed_out = true;
                return ControlFlow::Break(()); // Cancel execution
            }

            ControlFlow::Continue(())
        };

        let options = ParseOptions::new().progress_callback(&mut progress);
        let new_tree = self.parser.parse_with_options(
            &mut move |offset, _| {
                if offset >= text.len() {
                    ""
                } else {
                    let (chunk, chunk_byte_ix) = text.chunk(offset);
                    &chunk[offset - chunk_byte_ix..]
                }
            },
            Some(&old_tree),
            Some(options),
        );

        if timed_out || new_tree.is_none() {
            // Restore the old tree (already has edit() applied) so highlighting
            // continues with stale but byte-shifted data.
            self.tree = Some(old_tree);
            self.text = text.clone();
            // The windowed tree (from a previous Phase 1 parse) has NOT had
            // edit() applied, so its byte ranges are stale relative to the new
            // text.  Keeping it would cause match_styles to return un-shifted
            // highlight ranges (highlights appear before their actual text).
            // Clear it; Phase 1 will produce a fresh windowed tree shortly.
            self.windowed_tree = None;
            // Apply the edit to injection layer trees to keep their byte
            // positions in sync with the shifted text.  Without this, injection
            // highlights (e.g. asciidoc _emphasis_) appear at the pre-insertion
            // position until Phase 2 completes.
            for layer in &mut self.injection_layers {
                layer.tree.edit(&edit);
                layer.byte_range = Self::shift_byte_range(&layer.byte_range, &edit);
            }
            return SyntaxHighlightUpdate::TimedOut;
        }

        let new_tree = new_tree.unwrap();
        self.tree = Some(new_tree.clone());
        self.text = text.clone();

        if timeout.is_none() {
            self.parse_injection_layers(&new_tree);
            return SyntaxHighlightUpdate::Complete;
        }

        if self.has_injections() {
            // Injection parsing is deferred to a background thread to avoid
            // blocking the main thread on every keystroke.
            // Stale injection trees queried against new text produce incorrect
            // highlights (wrong emphasis ranges, etc.), so clear them and wait
            // for the background parse to restore correct layers.
            self.injection_layers.clear();
            // self.tree is now a fresh sync parse (correct byte positions).
            // The windowed_tree (from a previous Phase 1) was NOT updated with
            // the current edit, so its byte ranges are stale.  match_styles
            // prefers windowed_tree over self.tree when the query range fits,
            // which would produce un-shifted highlight ranges.  Clear it;
            // self.tree is already correct for main-language highlights.
            self.windowed_tree = None;
            // Signal the caller to dispatch a background parse.
            SyntaxHighlightUpdate::PendingInjections
        } else {
            self.parse_injection_layers(&new_tree);
            SyntaxHighlightUpdate::Complete
        }
    }

    pub fn update(
        &mut self,
        edit: Option<InputEdit>,
        text: &Rope,
        timeout: Option<Duration>,
    ) -> bool {
        matches!(
            self.update_with_status(edit, text, timeout),
            SyntaxHighlightUpdate::Complete
        )
    }

    /// Returns the data needed to compute injection layers on a background thread.
    /// Returns `None` if this language has no injections.
    pub(crate) fn injection_parse_data(&self, edit: InputEdit) -> Option<InjectionParseData> {
        let query = self.injections_query.clone()?;
        Some(InjectionParseData {
            query,
            content_capture_index: self.injection_content_capture_index,
            language_capture_index: self.injection_language_capture_index,
            old_layers: self
                .injection_layers
                .iter()
                .map(|layer| ReusableInjectionLayer {
                    language_name: layer.language_name.clone(),
                    tree: layer.tree.clone(),
                })
                .collect(),
            edit,
        })
    }

    /// Compute injection layers from a freshly-parsed main tree.
    /// This is pure computation with no side effects and is safe to run on a
    /// background thread.
    ///
    /// `scope`: when `Some`, the QueryCursor is restricted to that byte range so
    /// that only injection ranges within the viewport window are collected.
    /// This makes the injection parse fast for large documents (e.g. asciidoc).
    /// Injection highlights outside the scope will be absent until a full
    /// (scope=None) parse completes.
    pub(crate) fn compute_injection_layers(
        data: InjectionParseData,
        tree: &Tree,
        text: &Rope,
        scope: Option<Range<usize>>,
    ) -> Vec<InjectionLayer> {
        fn sort_ranges(ranges: &mut [tree_sitter::Range]) {
            ranges.sort_unstable_by(|a, b| {
                a.start_byte
                    .cmp(&b.start_byte)
                    .then_with(|| a.end_byte.cmp(&b.end_byte))
            });
        }

        let root_node = tree.root_node();
        let mut cursor = QueryCursor::new();
        // Restrict the query to the visible window when a scope is provided.
        // For combined injections (e.g. asciidoc_inline), this avoids scanning
        // the entire document on every keystroke.
        if let Some(ref s) = scope {
            cursor.set_byte_range(s.clone());
        }
        let mut matches = cursor.matches(&data.query, root_node, TextProvider(text));

        let mut combined_ranges: FxHashMap<SharedString, Vec<tree_sitter::Range>> =
            FxHashMap::default();
        let mut new_layers = Vec::new();
        while let Some(query_match) = matches.next() {
            let mut language_name: Option<SharedString> = None;
            let mut combined = false;
            for prop in data.query.property_settings(query_match.pattern_index) {
                match prop.key.as_ref() {
                    "injection.language" => {
                        language_name = prop
                            .value
                            .as_ref()
                            .map(|v| SharedString::from(v.to_string()));
                    }
                    "injection.combined" => combined = true,
                    _ => {}
                }
            }

            // Captured language names are left for a follow-up so this change
            // can focus on fixed-language injections.
            if language_name.is_none()
                && query_match
                    .captures
                    .iter()
                    .any(|cap| Some(cap.index) == data.language_capture_index)
            {
                continue;
            }

            let Some(language_name) = language_name else {
                continue;
            };

            let mut ranges = query_match
                .captures
                .iter()
                .filter(|cap| Some(cap.index) == data.content_capture_index)
                .map(|capture| capture.node.range())
                .collect::<Vec<_>>();

            if ranges.is_empty() {
                continue;
            }
            sort_ranges(&mut ranges);

            if combined {
                combined_ranges
                    .entry(language_name.clone())
                    .or_default()
                    .extend(ranges);
            } else {
                let old_tree = Self::reusable_injection_tree(&data, &language_name, &ranges);
                if let Some(layer) =
                    Self::parse_injection_layer(&language_name, ranges, old_tree.as_ref(), text)
                {
                    new_layers.push(layer);
                }
            }
        }

        for (language_name, mut ranges) in combined_ranges {
            if ranges.is_empty() {
                continue;
            }
            sort_ranges(&mut ranges);

            let old_tree = Self::reusable_injection_tree(&data, &language_name, &ranges);

            if let Some(layer) =
                Self::parse_injection_layer(&language_name, ranges, old_tree.as_ref(), text)
            {
                new_layers.push(layer);
            }
        }
        new_layers.sort_by_key(|layer| layer.byte_range.start);
        new_layers
    }

    fn reusable_injection_tree(
        data: &InjectionParseData,
        language_name: &SharedString,
        ranges: &[tree_sitter::Range],
    ) -> Option<Tree> {
        fn range_key(ranges: &[tree_sitter::Range]) -> Vec<(usize, usize)> {
            ranges.iter().map(|r| (r.start_byte, r.end_byte)).collect()
        }

        let queried = range_key(ranges);
        data.old_layers
            .iter()
            .filter(|layer| layer.language_name == *language_name)
            .find_map(|old_layer| {
                if range_key(&old_layer.tree.included_ranges()) == queried {
                    return Some(old_layer.tree.clone());
                }

                let mut edited_tree = old_layer.tree.clone();
                edited_tree.edit(&data.edit);
                if range_key(&edited_tree.included_ranges()) == queried {
                    Some(edited_tree)
                } else {
                    None
                }
            })
    }

    /// Parse one injection layer over the given included ranges.
    /// Reuses the previous tree only when the language and byte ranges still match.
    fn parse_injection_layer(
        language_name: &SharedString,
        ranges: Vec<tree_sitter::Range>,
        old_tree: Option<&Tree>,
        text: &Rope,
    ) -> Option<InjectionLayer> {
        fn bounding_byte_range(ranges: &[tree_sitter::Range]) -> Option<Range<usize>> {
            let start = ranges.iter().map(|r| r.start_byte).min()?;
            let end = ranges.iter().map(|r| r.end_byte).max()?;
            Some(start..end)
        }
        let config = LanguageRegistry::singleton().language(language_name)?;
        let mut parser = Parser::new();
        parser.set_language(&config.language).ok()?;
        parser.set_included_ranges(&ranges).ok()?;

        let new_tree = parser.parse_with_options(
            &mut |offset, _| {
                if offset >= text.len() {
                    ""
                } else {
                    let (chunk, chunk_byte_ix) = text.chunk(offset);
                    &chunk[offset - chunk_byte_ix..]
                }
            },
            old_tree,
            None,
        )?;

        let byte_range = bounding_byte_range(&ranges)?;
        Some(InjectionLayer {
            language_name: language_name.clone(),
            byte_range,
            tree: new_tree,
        })
    }

    /// Apply a tree that was parsed on a background thread.
    ///
    /// `injection_layers` must also be pre-computed in the background via
    /// [`compute_injection_layers`] to avoid blocking the main thread.
    ///
    /// Returns `true` if the tree was applied, `false` if the text no longer
    /// matches (i.e. the user typed during the background parse).
    pub(crate) fn apply_background_tree(
        &mut self,
        tree: Tree,
        text: &Rope,
        injection_layers: Vec<InjectionLayer>,
    ) -> bool {
        // Only apply if the text still matches what was parsed.
        if !self.text.eq(text) {
            return false;
        }

        self.tree = Some(tree);
        self.injection_layers = injection_layers;
        // A complete tree supersedes any windowed result.
        self.windowed_tree = None;
        self.full_tree_revision += 1;
        true
    }

    /// Apply a windowed tree parsed on a background thread.
    ///
    /// Only accepted when:
    /// - the text still matches, and
    /// - no full tree has been applied since this windowed parse was spawned
    ///   (guarded by `expected_full_tree_revision`).
    ///
    /// Returns `true` if the windowed tree was applied.
    pub(crate) fn apply_windowed_tree(
        &mut self,
        windowed: WindowedTree,
        text: &Rope,
        expected_full_tree_revision: u64,
    ) -> bool {
        if !self.text.eq(text) {
            return false;
        }
        if self.full_tree_revision != expected_full_tree_revision {
            // A full parse finished after this windowed parse was spawned;
            // the windowed result is now redundant.
            return false;
        }
        self.windowed_tree = Some(windowed);
        true
    }

    /// Returns the current `full_tree_revision` counter.
    /// Callers capture this before spawning a windowed parse and pass it back
    /// to `apply_windowed_tree` to guard against races.
    pub(crate) fn full_tree_revision(&self) -> u64 {
        self.full_tree_revision
    }

    /// Shift a byte range by a tree-sitter edit.
    /// Positions before the edit start are unchanged; positions inside the
    /// replaced region are clamped to the new end; positions after are shifted
    /// by the net byte delta.
    ///
    /// Retained for the planned incremental injection update (changed-ranges
    /// based layer reuse). Not currently called.
    #[allow(dead_code)]
    fn shift_byte_range(range: &Range<usize>, edit: &InputEdit) -> Range<usize> {
        let delta: isize = edit.new_end_byte as isize - edit.old_end_byte as isize;
        let shift = |pos: usize| -> usize {
            if pos <= edit.start_byte {
                pos
            } else if pos <= edit.old_end_byte {
                edit.new_end_byte
            } else {
                (pos as isize + delta).max(0) as usize
            }
        };
        shift(range.start)..shift(range.end)
    }

    /// Parse injection layers after the main tree is updated.
    /// pattern: parse once in update, query many times in render.
    fn parse_injection_layers(&mut self, tree: &Tree) {
        // Internal call: no real edit available, so use a no-op edit.
        // This path is only taken for languages without injections or
        // on initial load (before any user edits).
        let no_op_edit = InputEdit {
            start_byte: 0,
            old_end_byte: 0,
            new_end_byte: 0,
            start_position: tree_sitter::Point::new(0, 0),
            old_end_position: tree_sitter::Point::new(0, 0),
            new_end_position: tree_sitter::Point::new(0, 0),
        };
        let Some(data) = self.injection_parse_data(no_op_edit) else {
            self.injection_layers.clear();
            return;
        };
        self.injection_layers = Self::compute_injection_layers(data, tree, &self.text.clone(), None);
    }

    /// Match the visible ranges of nodes in the Tree for highlighting.
    fn match_styles(&self, range: Range<usize>) -> Vec<HighlightItem> {
        let mut highlights = vec![];

        // Prefer the windowed tree when the query range is fully inside its
        // byte_range.  It is a freshly-parsed partial tree so it gives correct
        // results faster than waiting for the full background parse.
        let active_tree: &Tree = if let Some(wt) = &self.windowed_tree {
            if wt.byte_range.start <= range.start && range.end <= wt.byte_range.end {
                &wt.tree
            } else if let Some(t) = &self.tree {
                t
            } else {
                return highlights;
            }
        } else if let Some(t) = &self.tree {
            t
        } else {
            return highlights;
        };

        let Some(query) = &self.query else {
            return highlights;
        };

        let root_node = active_tree.root_node();
        let source = &self.text;

        // Query pre-parsed injection layers.
        let mut last_layer_start = 0;
        for layer in &self.injection_layers {
            debug_assert!(layer.byte_range.start >= last_layer_start);
            last_layer_start = layer.byte_range.start;

            if layer.byte_range.end <= range.start {
                continue;
            }

            // Layers are sorted by start byte in compute_injection_layers.
            if layer.byte_range.start >= range.end {
                break;
            }

            let Some(query) = self.injection_queries.get(&layer.language_name) else {
                tracing::debug!(
                    "missing highlight query for injection language {:?}",
                    layer.language_name
                );
                continue;
            };

            let mut query_cursor = QueryCursor::new();
            query_cursor.set_byte_range(range.clone());

            let mut matches =
                query_cursor.matches(query, layer.tree.root_node(), TextProvider(&self.text));

            let mut last_end = 0usize;
            while let Some(m) = matches.next() {
                let allow_overlapping_captures = query
                    .property_settings(m.pattern_index)
                    .iter()
                    .any(|prop| prop.key.as_ref() == "highlight.allow-overlap");

                for cap in m.captures {
                    let node_range = cap.node.start_byte()..cap.node.end_byte();

                    if !allow_overlapping_captures && node_range.start < last_end {
                        continue;
                    }

                    if let Some(highlight_name) = query.capture_names().get(cap.index as usize) {
                        if !allow_overlapping_captures {
                            last_end = node_range.end;
                        }
                        highlights.push(HighlightItem::new(
                            node_range,
                            SharedString::from(highlight_name.to_string()),
                        ));
                    }
                }
            }
        }

        let query_nodes = collect_query_nodes(root_node, &range);

        for query_node in &query_nodes {
            let mut query_cursor = QueryCursor::new();
            query_cursor.set_byte_range(range.clone());

            let mut matches = query_cursor.matches(&query, *query_node, TextProvider(&source));

            while let Some(query_match) = matches.next() {
                for cap in query_match.captures {
                    let node = cap.node;

                    let Some(highlight_name) = query.capture_names().get(cap.index as usize) else {
                        continue;
                    };

                    let node_range: Range<usize> = node.start_byte()..node.end_byte();
                    let highlight_name = SharedString::from(highlight_name.to_string());

                    // Merge near range and same highlight name
                    let last_item = highlights.last();
                    let last_range = last_item.map(|item| &item.range).unwrap_or(&(0..0));
                    let last_highlight_name = last_item.map(|item| item.name.clone());

                    if last_range == &node_range {
                        // case:
                        // last_range: 213..220, last_highlight_name: Some("property")
                        // last_range: 213..220, last_highlight_name: Some("string")
                        highlights.push(HighlightItem::new(
                            node_range,
                            last_highlight_name.unwrap_or(highlight_name),
                        ));
                    } else {
                        highlights.push(HighlightItem::new(node_range, highlight_name.clone()));
                    }
                }
            }
        }

        // DO NOT REMOVE THIS PRINT, it's useful for debugging
        // for item in highlights {
        //     println!("item: {:?}", item);
        // }

        highlights
    }

    /// Returns the syntax highlight styles for a range of text.
    ///
    /// `ime_marked_range`: if `Some`, bytes in that range are excluded from
    /// highlights.  IME composition text is syntactically meaningless and
    /// must not be treated as e.g. emphasis delimiters.
    pub fn styles(
        &self,
        range: &Range<usize>,
        theme: &HighlightTheme,
        ime_marked_range: Option<Range<usize>>,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        let mut styles = vec![];
        let start_offset = range.start;

        let highlights = self.match_styles(range.clone());

        for item in highlights {
            let node_range = &item.range;
            let name = &item.name;

            // Skip highlights that overlap the IME composition range.
            // The composition text is syntactically meaningless; treating its
            // bytes as delimiters produces false emphasis/strong spans.
            if let Some(ref ime) = ime_marked_range {
                if node_range.start < ime.end && node_range.end > ime.start {
                    continue;
                }
            }

            // Clip the node range to the requested range.
            // `range.start` / `range.end` can fall inside a multi-byte character
            // (e.g. 3-byte CJK or emoji) when the visible-byte range is
            // computed from pixel positions.  Passing a non-char-boundary byte
            // index to DirectWrite's layout_line causes a panic in str slicing.
            // Walk backward from the byte until we land on a UTF-8 char boundary
            // (i.e. a byte that is NOT a continuation byte 0x80..=0xBF).
            let snap_to_char_boundary = |byte: usize| -> usize {
                let mut pos = byte.min(self.text.len());
                // Walk backward until pos points at a char boundary.
                // A UTF-8 continuation byte has the bit pattern 10xx_xxxx.
                // We look at the byte *at* pos (not before it).
                while pos > 0 && pos < self.text.len() {
                    let (chunk, chunk_start) = self.text.chunk(pos);
                    let byte_in_chunk = pos - chunk_start;
                    if byte_in_chunk < chunk.len()
                        && (chunk.as_bytes()[byte_in_chunk] & 0xC0) == 0x80
                    {
                        pos -= 1;
                    } else {
                        break;
                    }
                }
                pos
            };
            let clipped_start =
                snap_to_char_boundary(node_range.start.max(range.start));
            let clipped_end =
                snap_to_char_boundary(node_range.end.min(range.end));
            let mut node_range = clipped_start..clipped_end;
            if node_range.start > node_range.end {
                node_range.end = node_range.start;
            }

            styles.push((node_range, theme.style(name.as_ref()).unwrap_or_default()));
        }

        // If the matched styles is empty, return a default range.
        if styles.is_empty() {
            return vec![(start_offset..range.end, HighlightStyle::default())];
        }

        // Snap the total range endpoints to char boundaries before passing to
        // unique_styles, which uses them as sweep-line split points.
        let snapped_range = {
            let snap_to_char_boundary = |byte: usize| -> usize {
                let mut pos = byte.min(self.text.len());
                while pos > 0 && pos < self.text.len() {
                    let (chunk, chunk_start) = self.text.chunk(pos);
                    let byte_in_chunk = pos - chunk_start;
                    if byte_in_chunk < chunk.len()
                        && (chunk.as_bytes()[byte_in_chunk] & 0xC0) == 0x80
                    {
                        pos -= 1;
                    } else {
                        break;
                    }
                }
                pos
            };
            snap_to_char_boundary(range.start)..snap_to_char_boundary(range.end)
        };
        let styles = unique_styles(&snapped_range, styles);

        // NOTE: DO NOT remove this comment, it is used for debugging.
        // for style in &styles {
        //     println!("---- style: {:?} - {:?}", style.0, style.1.color);
        // }
        // println!("--------------------------------");

        styles
    }
}

/// To merge intersection ranges, let the subsequent range cover
/// the previous overlapping range and split the previous range.
///
/// From:
///
/// AA
///   BBB
///    CCCCC
///      DD
///         EEEE
///
/// To:
///
/// AABCCDDCEEEE
pub(crate) fn unique_styles(
    total_range: &Range<usize>,
    styles: Vec<(Range<usize>, HighlightStyle)>,
) -> Vec<(Range<usize>, HighlightStyle)> {
    if styles.is_empty() {
        return styles;
    }

    // Create intervals: (position, is_start, style_index)
    let mut intervals: Vec<(usize, bool, usize)> = Vec::with_capacity(styles.len() * 2 + 2);
    for (i, (range, _)) in styles.iter().enumerate() {
        intervals.push((range.start, true, i));
        intervals.push((range.end, false, i));
    }

    intervals.push((total_range.start, true, usize::MAX));
    intervals.push((total_range.end, false, usize::MAX));

    // Sort by position, with ends before starts at same position
    // This ensures we close ranges before opening new ones at the same position
    intervals.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    // Track significant intervals (where style ranges end) for merging decisions
    let mut significant_intervals: BTreeSet<usize> = BTreeSet::new();
    for (range, _) in &styles {
        significant_intervals.insert(range.end);
    }

    let mut result: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    let mut active_styles: Vec<usize> = Vec::new();
    let mut last_pos = total_range.start;

    for (pos, is_start, style_idx) in intervals {
        // Skip total_range boundaries in active set management
        let is_boundary = style_idx == usize::MAX;

        if pos > last_pos {
            let interval = last_pos..pos;
            let combined_style = if active_styles.is_empty() {
                HighlightStyle::default()
            } else {
                let mut combined = HighlightStyle::default();
                for &idx in &active_styles {
                    merge_highlight_style(&mut combined, &styles[idx].1);
                }
                combined
            };
            result.push((interval, combined_style));
        }

        if !is_boundary {
            if is_start {
                active_styles.push(style_idx);
            } else {
                active_styles.retain(|&i| i != style_idx);
            }
        }

        last_pos = pos;
    }

    // Merge adjacent ranges with the same style, but not across significant boundaries
    let mut merged: Vec<(Range<usize>, HighlightStyle)> = Vec::with_capacity(result.len());
    for (range, style) in result {
        if let Some((last_range, last_style)) = merged.last_mut() {
            if last_range.end == range.start
                && *last_style == style
                && !significant_intervals.contains(&range.start)
            {
                // Merge adjacent ranges with same style, but not across significant boundaries
                last_range.end = range.end;
                continue;
            }
        }
        merged.push((range, style));
    }

    merged
}

/// Walk the tree and collect nodes suitable for querying, skipping subtrees
/// that fall entirely outside the byte range. Nodes much larger than the
/// query range are recursed into so that `QueryCursor` only visits the
/// relevant portion of the tree.
fn collect_query_nodes<'a>(
    root: tree_sitter::Node<'a>,
    range: &Range<usize>,
) -> Vec<tree_sitter::Node<'a>> {
    let mut nodes = Vec::new();
    collect_query_nodes_inner(root, range, &mut nodes);
    if nodes.is_empty() {
        nodes.push(root);
    }
    nodes
}

fn collect_query_nodes_inner<'a>(
    node: tree_sitter::Node<'a>,
    range: &Range<usize>,
    out: &mut Vec<tree_sitter::Node<'a>>,
) {
    // Skip nodes entirely outside the range.
    if node.end_byte() <= range.start || node.start_byte() >= range.end {
        return;
    }

    let node_span = node.end_byte() - node.start_byte();
    let range_span = range.end - range.start;

    // Use `goto_first_child_for_byte` to seek directly to the first
    // overlapping child instead of iterating all children from the start.
    if node_span > range_span + LARGE_NODE_THRESHOLD && node.child_count() > 0 {
        let mut cursor = node.walk();
        if cursor.goto_first_child_for_byte(range.start).is_some() {
            loop {
                let child = cursor.node();
                if child.start_byte() >= range.end {
                    break;
                }
                collect_query_nodes_inner(child, range, out);
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
        return;
    }

    out.push(node);
}

/// Merge other style (Other on top)
fn merge_highlight_style(style: &mut HighlightStyle, other: &HighlightStyle) {
    if let Some(color) = other.color {
        style.color = Some(color);
    }
    if let Some(font_weight) = other.font_weight {
        style.font_weight = Some(font_weight);
    }
    if let Some(font_style) = other.font_style {
        style.font_style = Some(font_style);
    }
    if let Some(background_color) = other.background_color {
        style.background_color = Some(background_color);
    }
    if let Some(underline) = other.underline {
        style.underline = Some(underline);
    }
    if let Some(strikethrough) = other.strikethrough {
        style.strikethrough = Some(strikethrough);
    }
    if let Some(fade_out) = other.fade_out {
        style.fade_out = Some(fade_out);
    }
}

#[cfg(test)]
mod tests {
    use gpui::Hsla;
    use tree_sitter::{Parser, Query, QueryCursor};

    use super::*;
    use crate::Colorize as _;

    fn color_style(color: Hsla) -> HighlightStyle {
        let mut style = HighlightStyle::default();
        style.color = Some(color);
        style
    }

    #[cfg(any(feature = "tree-sitter-languages", feature = "tree-sitter-asciidoc"))]
    fn has_highlight_covering(
        highlights: &[HighlightItem],
        source: &str,
        text: &str,
        highlight_name: &str,
    ) -> bool {
        let start = source.find(text).expect("text should exist in source");
        let end = start + text.len();
        highlights.iter().any(|item| {
            item.name.as_ref() == highlight_name
                && item.range.start <= start
                && item.range.end >= end
        })
    }

    #[track_caller]
    fn assert_unique_styles(
        range: Range<usize>,
        left: Vec<(Range<usize>, HighlightStyle)>,
        right: Vec<(Range<usize>, HighlightStyle)>,
    ) {
        fn color_name(c: Option<Hsla>) -> String {
            match c {
                Some(c) => {
                    if c == gpui::red() {
                        "red".to_string()
                    } else if c == gpui::green() {
                        "green".to_string()
                    } else if c == gpui::blue() {
                        "blue".to_string()
                    } else {
                        c.to_hex()
                    }
                }
                None => "clean".to_string(),
            }
        }

        let left = unique_styles(&range, left);
        if left.len() != right.len() {
            println!("\n---------------------------------------------");
            for (range, style) in left.iter() {
                println!("({:?}, {})", range, color_name(style.color));
            }
            println!("---------------------------------------------");
            panic!("left {} styles, right {} styles", left.len(), right.len());
        }
        for (left, right) in left.into_iter().zip(right) {
            if left.1.color != right.1.color || left.0 != right.0 {
                panic!(
                    "\n left: ({:?}, {})\nright: ({:?}, {})\n",
                    left.0,
                    color_name(left.1.color),
                    right.0,
                    color_name(right.1.color)
                );
            }
        }
    }

    #[test]
    #[cfg(feature = "tree-sitter-languages")]
    fn test_html_style_injects_css_highlights() {
        let html = r#"<style>
.card { color: #336699; }
</style>
"#;

        let rope = Rope::from_str(html);
        let mut highlighter = SyntaxHighlighter::new("html");
        highlighter.update(None, &rope, None);

        let highlights = highlighter.match_styles(0..html.len());

        assert!(
            has_highlight_covering(&highlights, html, "color", "property"),
            "CSS property names inside style elements should be highlighted"
        );
        assert!(
            has_highlight_covering(&highlights, html, "#336699", "string.special"),
            "CSS color values inside style elements should be highlighted"
        );
    }

    #[test]
    #[cfg(feature = "tree-sitter-languages")]
    fn test_html_script_injects_javascript_highlights() {
        let html = r#"<script>
const answer = 42;
console.log(answer);
</script>
"#;

        let rope = Rope::from_str(html);
        let mut highlighter = SyntaxHighlighter::new("html");
        highlighter.update(None, &rope, None);

        let highlights = highlighter.match_styles(0..html.len());

        assert!(
            has_highlight_covering(&highlights, html, "const", "keyword"),
            "JavaScript keywords inside script elements should be highlighted"
        );
        assert!(
            has_highlight_covering(&highlights, html, "answer", "variable"),
            "JavaScript identifiers inside script elements should be highlighted"
        );
    }

    #[test]
    #[cfg(feature = "tree-sitter-languages")]
    fn test_php_combined_injection_closing_tags() {
        let php_code = r#"<?php
$x = 1;
?>
<html>
<body>
  <h1><?php echo "Hello"; ?></h1>
  <ul>
    <?php foreach ($items as $item): ?>
      <li><?php echo $item; ?></li>
    <?php endforeach; ?>
  </ul>
</body>
</html>
"#;

        let rope = Rope::from_str(php_code);
        let mut highlighter = SyntaxHighlighter::new("php");
        highlighter.update(None, &rope, None);

        let full_range = 0..php_code.len();
        let highlights = highlighter.match_styles(full_range);

        // Verify all closing HTML tags are highlighted
        let closing_tags = ["</h1>", "</li>", "</ul>", "</body>", "</html>"];
        for tag in closing_tags {
            let pos = php_code.find(tag).unwrap();
            let tag_name_start = pos + 2; // after "</"
            let tag_name_end = tag_name_start + tag.len() - 3; // before ">"

            let has_highlight = highlights
                .iter()
                .any(|item| item.range.start <= tag_name_start && item.range.end >= tag_name_end);

            assert!(
                has_highlight,
                "closing tag {} at byte {} should be highlighted",
                tag, pos
            );
        }
    }

    #[test]
    #[cfg(feature = "tree-sitter-languages")]
    fn test_highlight_allow_overlap_property_combines_nested_captures() {
        let markdown = "This has ***bold and italic*** and **bold _with_ italic** text.";
        let rope = Rope::from_str(markdown);
        let mut highlighter = SyntaxHighlighter::new("markdown");
        highlighter.update(None, &rope, None);

        let styles = highlighter.styles(&(0..markdown.len()), &HighlightTheme::default_dark());
        for text in ["bold and italic", "with"] {
            let start = markdown.find(text).unwrap();
            let end = start + text.len();

            assert!(
                styles.iter().any(|(range, style)| {
                    range.start <= start
                        && range.end >= end
                        && style.font_weight == Some(gpui::FontWeight::BOLD)
                        && style.font_style == Some(gpui::FontStyle::Italic)
                }),
                "{text:?} should combine bold and italic styles"
            );
        }

        let highlights = highlighter.match_styles(0..markdown.len());
        let delimiter_start = markdown.find("_with_").unwrap();
        let delimiter_end = delimiter_start + "_".len();

        assert!(
            highlights.iter().any(|item| {
                item.name.as_ref() == "punctuation.delimiter"
                    && item.range.start <= delimiter_start
                    && item.range.end >= delimiter_end
            }),
            "overlap-enabled captures should not hide nested delimiter highlights"
        );
    }

    #[test]
    fn test_unique_styles() {
        let red = color_style(gpui::red());
        let green = color_style(gpui::green());
        let blue = color_style(gpui::blue());
        let clean = HighlightStyle::default();

        assert_unique_styles(
            0..65,
            vec![
                (2..10, clean),
                (2..10, clean),
                (5..11, red),
                (2..6, clean),
                (10..15, green),
                (15..30, clean),
                (29..35, blue),
                (35..40, green),
                (45..60, blue),
            ],
            vec![
                (0..5, clean),
                (5..6, red),
                (6..10, red),
                (10..11, green),
                (11..15, green),
                (15..29, clean),
                (29..30, blue),
                (30..35, blue),
                (35..40, green),
                (40..45, clean),
                (45..60, blue),
                (60..65, clean),
            ],
        );
    }

    #[allow(dead_code)]
    fn parse_tree(lang: &str, source: &str) -> Tree {
        let config = LanguageRegistry::singleton()
            .language(lang)
            .unwrap_or_else(|| panic!("language config not found: {lang}"));
        let mut parser = Parser::new();
        parser.set_language(&config.language).unwrap();
        parser.parse(source, None).unwrap()
    }

    #[allow(dead_code)]
    fn capture_rows_with_first_matching_query(
        lang: &str,
        source: &str,
        candidates: &[&str],
        capture_name: &str,
    ) -> Vec<usize> {
        let config = LanguageRegistry::singleton()
            .language(lang)
            .unwrap_or_else(|| panic!("language config not found: {lang}"));
        let tree = parse_tree(lang, source);
        let rope = Rope::from_str(source);

        for query_source in candidates {
            let Ok(query) = Query::new(&config.language, query_source) else {
                continue;
            };
            let capture_index = query
                .capture_names()
                .iter()
                .position(|name| *name == capture_name)
                .map(|ix| ix as u32)
                .unwrap_or(0);

            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(&query, tree.root_node(), TextProvider(&rope));
            let mut rows = Vec::new();

            while let Some(query_match) = matches.next() {
                for cap in query_match.captures {
                    if cap.index == capture_index {
                        rows.push(cap.node.start_position().row);
                    }
                }
            }

            if !rows.is_empty() {
                rows.sort_unstable();
                rows.dedup();
                return rows;
            }
        }

        Vec::new()
    }

    #[test]
    #[cfg(feature = "tree-sitter-markdown")]
    fn test_markdown_parse_and_capture_headings() {
        let source = "# H1\nText\n## H2\nUnder\n---\n```\n# not\n```\n";
        let tree = parse_tree("markdown", source);
        assert!(!tree.root_node().has_error());

        let rows = capture_rows_with_first_matching_query(
            "markdown",
            source,
            &[
                "[(atx_heading) (setext_heading)] @heading",
                "[(atx_h1_marker) (atx_h2_marker) (setext_h2_underline)] @heading",
            ],
            "heading",
        );
        assert_eq!(rows, vec![0, 2, 3]);
    }

    #[test]
    #[cfg(feature = "tree-sitter-markdown")]
    fn test_heading_levels_in_rows_limits_result_rows() {
        let source = "# A\nText\n## B\nText\n### C\n";
        let rope = Rope::from_str(source);
        let mut highlighter = SyntaxHighlighter::new("markdown");
        assert!(highlighter.update(None, &rope, None));

        assert_eq!(
            highlighter.heading_levels_in_rows(1..4),
            vec![(1, None), (2, Some(2)), (3, None)]
        );
    }

    #[test]
    #[cfg(feature = "tree-sitter-asciidoc")]
    fn test_update_with_status_reports_pending_injections() {
        let source = "= A\n\nThis has *bold* text.\n";
        let rope = Rope::from_str(source);
        let mut highlighter = SyntaxHighlighter::new("asciidoc");

        assert_eq!(
            highlighter.update_with_status(None, &rope, Some(Duration::from_millis(2))),
            SyntaxHighlightUpdate::PendingInjections
        );
        assert!(highlighter.tree().is_some());
    }

    #[test]
    #[cfg(not(target_family = "wasm"))]
    fn test_reusable_injection_tree_matches_shifted_non_combined_range() {
        let old_source = "  [1,2]";
        let old_range = tree_sitter::Range {
            start_byte: 2,
            end_byte: old_source.len(),
            start_point: Point::new(0, 2),
            end_point: Point::new(0, old_source.len()),
        };
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_json::LANGUAGE.into())
            .expect("JSON parser should load");
        parser
            .set_included_ranges(&[old_range])
            .expect("included range should be valid");
        let old_tree = parser
            .parse(old_source, None)
            .expect("old JSON should parse");
        let edit = InputEdit {
            start_byte: 0,
            old_end_byte: 0,
            new_end_byte: 2,
            start_position: Point::new(0, 0),
            old_end_position: Point::new(0, 0),
            new_end_position: Point::new(0, 2),
        };
        let new_range = tree_sitter::Range {
            start_byte: 4,
            end_byte: old_source.len() + 2,
            start_point: Point::new(0, 4),
            end_point: Point::new(0, old_source.len() + 2),
        };
        let language_name = SharedString::from("json");
        let query = Arc::new(
            Query::new(
                &tree_sitter_json::LANGUAGE.into(),
                "(document) @injection.content",
            )
            .expect("query should compile"),
        );
        let data = InjectionParseData {
            query,
            content_capture_index: None,
            language_capture_index: None,
            old_layers: vec![ReusableInjectionLayer {
                language_name: language_name.clone(),
                tree: old_tree,
            }],
            edit,
        };

        let reused =
            SyntaxHighlighter::reusable_injection_tree(&data, &language_name, &[new_range])
                .expect("shifted range should reuse the old tree");
        assert_eq!(reused.included_ranges()[0].start_byte, new_range.start_byte);
    }

    #[test]
    #[cfg(feature = "tree-sitter-asciidoc")]
    fn test_asciidoc_parse_and_capture_headings() {
        let source = "= Title\n\n== Section\nParagraph\n";
        let tree = parse_tree("asciidoc", source);
        assert!(!tree.root_node().has_error());

        let rows = capture_rows_with_first_matching_query(
            "asciidoc",
            source,
            &["[(document_title) (title1) (title2) (title3) (title4) (title5)] @heading"],
            "heading",
        );
        assert_eq!(rows, vec![0, 2]);
    }

    #[test]
    #[cfg(feature = "tree-sitter-djot")]
    fn test_djot_parse_and_capture_headings() {
        let source = "# Title\nBody\n## Sub\n```\n# not heading\n```\n";
        let tree = parse_tree("djot", source);
        assert!(!tree.root_node().has_error());

        let rows = capture_rows_with_first_matching_query(
            "djot",
            source,
            &["[(heading) (section)] @heading", "[(atx_heading)] @heading"],
            "heading",
        );
        assert_eq!(rows, vec![0, 2]);
    }

    #[test]
    #[cfg(all(
        feature = "tree-sitter-markdown",
        feature = "tree-sitter-asciidoc",
        feature = "tree-sitter-djot"
    ))]
    fn test_heading_levels_alignment_multilang() {
        let markdown = Rope::from_str("# A\nB\n## C\n");
        let mut md = SyntaxHighlighter::new("markdown");
        assert!(md.update(None, &markdown, None));
        assert_eq!(md.heading_levels(), vec![Some(1), None, Some(2), None]);

        let asciidoc = Rope::from_str("= A\n\n== C\n");
        let mut adoc = SyntaxHighlighter::new("asciidoc");
        assert_eq!(adoc.language().as_ref(), "asciidoc");
        assert!(adoc.update(None, &asciidoc, None));
        assert_eq!(adoc.heading_levels(), vec![Some(1), None, Some(2), None]);

        let djot = Rope::from_str("# A\nB\n## C\n");
        let mut dj = SyntaxHighlighter::new("djot");
        assert_eq!(dj.language().as_ref(), "djot");
        assert!(dj.update(None, &djot, None));
        assert_eq!(dj.heading_levels(), vec![Some(1), None, Some(2), None]);
    }

    #[test]
    #[cfg(feature = "tree-sitter-asciidoc")]
    fn test_asciidoc_heading_styles_all_levels() {
        // 複数 section が入れ子になる構造で全 heading が @title キャプチャされるか確認
        let source = "= Doc Title\n\n== Section One\n\nText here.\n\n=== SubSection\n\nMore text.\n\n== Section Two\n\nAnother paragraph.\n";
        let rope = Rope::from_str(source);
        let mut highlighter = SyntaxHighlighter::new("asciidoc");
        highlighter.update(None, &rope, None);

        let highlights = highlighter.match_styles(0..source.len());

        for heading_text in ["Doc Title", "Section One", "SubSection", "Section Two"] {
            assert!(
                has_highlight_covering(&highlights, source, heading_text, "title"),
                "heading {:?} should have @title highlight",
                heading_text
            );
        }
    }

    #[test]
    #[cfg(feature = "tree-sitter-asciidoc")]
    fn test_asciidoc_inline_emphasis_styles() {
        // bold と italic が injection 経由で正しくキャプチャされるか確認
        let source = "= Title\n\nThis has *bold text* and _italic text_ here.\n";
        let rope = Rope::from_str(source);
        let mut highlighter = SyntaxHighlighter::new("asciidoc");
        assert!(
            highlighter.update(None, &rope, None),
            "sync parse should complete and apply injections when timeout is None"
        );

        let highlights = highlighter.match_styles(0..source.len());

        assert!(
            has_highlight_covering(&highlights, source, "bold text", "emphasis.strong"),
            "*bold text* should have @emphasis.strong highlight"
        );
        assert!(
            has_highlight_covering(&highlights, source, "italic text", "emphasis"),
            "_italic text_ should have @emphasis highlight"
        );
    }
}
