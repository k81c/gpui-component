use std::{ops::Range, rc::Rc};

use gpui::{Bounds, Font, Half, Pixels, ShapedLine, TextAlign, px};

use super::{WrappingIndent, display_map::LineLayout};
use crate::input::LinePresentation;

#[derive(Clone, PartialEq)]
pub(super) struct PresentationLayoutKey {
    pub(super) document_revision: u64,
    pub(super) geometry_revision: u64,
    pub(super) decorator_identity: Option<usize>,
    pub(super) decorator_revision: Option<u64>,
    pub(super) font: Font,
    pub(super) font_size: Pixels,
    pub(super) line_height: Pixels,
    pub(super) rem_size: Pixels,
}

#[derive(Clone)]
pub(super) struct PresentationLayoutCache {
    pub(super) key: PresentationLayoutKey,
    pub(super) presentations: Rc<Vec<LinePresentation>>,
    pub(super) vertical_layout: VerticalLayoutMap,
    pub(super) presentation_hash: u64,
}

#[derive(Clone, Default)]
pub(crate) struct WhitespaceIndicators {
    pub(crate) space: ShapedLine,
    pub(crate) tab: ShapedLine,
}

/// Prefix-sum vertical geometry for every buffer line.
///
/// Folded lines have a zero height. Keeping this map alongside the shaped
/// visible lines makes scrolling, hit testing, carets and painting use the
/// same content-space coordinates.
#[derive(Clone, Default)]
pub(crate) struct VerticalLayoutMap {
    origins: Rc<Vec<Pixels>>,
    heights: Rc<Vec<Pixels>>,
    pub(crate) total_height: Pixels,
}

impl VerticalLayoutMap {
    pub(crate) fn new(heights: Vec<Pixels>) -> Self {
        let mut origins = Vec::with_capacity(heights.len());
        let mut total_height = px(0.);
        for height in &heights {
            origins.push(total_height);
            total_height += *height;
        }
        Self {
            origins: Rc::new(origins),
            heights: Rc::new(heights),
            total_height,
        }
    }

    pub(crate) fn origin_for_line(&self, line: usize) -> Pixels {
        self.origins.get(line).copied().unwrap_or(self.total_height)
    }

    pub(crate) fn height_for_line(&self, line: usize) -> Pixels {
        self.heights.get(line).copied().unwrap_or(px(0.))
    }

    pub(crate) fn line_at_y(&self, y: Pixels) -> usize {
        if self.origins.is_empty() {
            return 0;
        }
        let mut line = self
            .origins
            .partition_point(|origin| *origin <= y)
            .saturating_sub(1)
            .min(self.origins.len() - 1);
        while line + 1 < self.origins.len() && self.height_for_line(line) == px(0.) {
            line += 1;
        }
        line
    }
}

#[derive(Clone)]
pub(super) struct LastLayout {
    pub(super) document_revision: u64,
    pub(super) presentation_hash: u64,
    pub(super) visible_range: Range<usize>,
    pub(super) visible_buffer_lines: Vec<usize>,
    pub(super) visible_line_byte_offsets: Vec<usize>,
    pub(super) visible_top: Pixels,
    pub(super) visible_range_offset: Range<usize>,
    pub(super) lines: Rc<Vec<LineLayout>>,
    pub(super) line_height: Pixels,
    /// Presentations for shaped visible lines, parallel to `visible_buffer_lines`.
    pub(crate) line_presentations: Rc<Vec<LinePresentation>>,
    /// Presentations for every buffer line, used before or outside shaping.
    pub(crate) all_line_presentations: Rc<Vec<LinePresentation>>,
    pub(crate) vertical_layout: VerticalLayoutMap,
    pub(super) wrap_width: Option<Pixels>,
    pub(super) wrapping_indent: WrappingIndent,
    pub(super) line_number_width: Pixels,
    /// Width of one space in the editor font.
    ///
    /// Past the end of a line there are no glyphs to hit-test against, so this is the
    /// step used to measure how far past the end a pointer sits.
    pub(super) space_width: Pixels,
    pub(super) cursor_bounds: Option<Bounds<Pixels>>,
    pub(super) text_align: TextAlign,
    pub(super) content_width: Pixels,
}

impl LastLayout {
    pub(crate) fn line(&self, row: usize) -> Option<&LineLayout> {
        let pos = self.visible_buffer_lines.binary_search(&row).ok()?;
        self.lines.get(pos)
    }

    pub(super) fn alignment_offset(&self, line_width: Pixels) -> Pixels {
        match self.text_align {
            TextAlign::Left => px(0.),
            TextAlign::Center => (self.content_width - line_width).half().max(px(0.)),
            TextAlign::Right => (self.content_width - line_width).max(px(0.)),
        }
    }

    pub(crate) fn presentation_for_visible_index(&self, index: usize) -> LinePresentation {
        self.line_presentations
            .get(index)
            .copied()
            .or_else(|| self.line_presentations.first().copied())
            .unwrap_or(LinePresentation {
                font_size: self.line_height,
                line_height: self.line_height,
                spacing_before: px(0.),
                spacing_after: px(0.),
            })
    }

    pub(crate) fn presentation_for_buffer_line(&self, line: usize) -> Option<LinePresentation> {
        self.all_line_presentations.get(line).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_layout_maps_lines_and_folded_gaps() {
        let map = VerticalLayoutMap::new(vec![px(20.), px(0.), px(42.)]);
        assert_eq!(map.origin_for_line(0), px(0.));
        assert_eq!(map.origin_for_line(1), px(20.));
        assert_eq!(map.origin_for_line(2), px(20.));
        assert_eq!(map.total_height, px(62.));
        assert_eq!(map.line_at_y(px(0.)), 0);
        assert_eq!(map.line_at_y(px(19.)), 0);
        assert_eq!(map.line_at_y(px(20.)), 2);
        assert_eq!(map.line_at_y(px(61.)), 2);
    }
}
