//! Display-only padding for rounded inline highlights. Source offsets remain
//! authoritative for selection, copying and links; padding never enters a buffer.
use std::ops::Range;

use gpui::{
    App, Bounds, HighlightStyle, Hsla, Pixels, SharedString, TextLayout, Window, fill, point, px,
};

#[derive(Clone)]
pub(super) struct InlineBackground {
    pub source: SharedString,
    pub text: SharedString,
    pub highlights: Vec<(Range<usize>, HighlightStyle)>,
    pads: Vec<(Range<usize>, usize)>,
    backgrounds: Vec<(Range<usize>, Hsla)>,
    radius: Pixels,
}

impl InlineBackground {
    pub fn new(
        source: SharedString,
        highlights: &[(Range<usize>, HighlightStyle)],
        radius: Pixels,
    ) -> Option<Self> {
        if radius <= px(0.) {
            return None;
        }
        let mut spans: Vec<(Range<usize>, Hsla)> = Vec::new();
        for (range, style) in highlights {
            let Some(color) = style
                .background_color
                .filter(|color| !color.is_transparent())
            else {
                continue;
            };
            if range.is_empty() || range.end > source.len() {
                continue;
            }
            if let Some((previous, previous_color)) = spans.last_mut() {
                if previous.end == range.start && *previous_color == color {
                    previous.end = range.end;
                    continue;
                }
            }
            spans.push((range.clone(), color));
        }
        if spans.is_empty() {
            return None;
        }
        let mut result = Self {
            source: source.clone(),
            text: SharedString::default(),
            highlights: Vec::new(),
            pads: Vec::new(),
            backgrounds: Vec::new(),
            radius,
        };
        let mut text = String::with_capacity(source.len() + spans.len() * 4);
        let (mut highlight_ix, mut span_ix, mut background_start) = (0, 0, 0);
        for (offset, ch) in source.char_indices() {
            while highlight_ix < highlights.len() && highlights[highlight_ix].0.end <= offset {
                highlight_ix += 1;
            }
            let style = highlights
                .get(highlight_ix)
                .filter(|(range, _)| range.contains(&offset))
                .map(|(_, style)| *style)
                .unwrap_or_default();
            if spans
                .get(span_ix)
                .is_some_and(|(span, _)| span.start == offset)
            {
                background_start = text.len();
                result.pad(&mut text, offset, style);
            }
            result.push(&mut text, ch, style);
            let end = offset + ch.len_utf8();
            if let Some((span, color)) = spans.get(span_ix) {
                if span.end == end {
                    result.pad(&mut text, end, style);
                    result
                        .backgrounds
                        .push((background_start..text.len(), *color));
                    span_ix += 1;
                }
            }
        }
        result.text = text.into();
        Some(result)
    }

    fn push(&mut self, text: &mut String, ch: char, mut style: HighlightStyle) {
        // GPUI's rectangular run background is replaced by one rounded quad per
        // visual line, including the actual shaped padding advance.
        style.background_color = None;
        let start = text.len();
        text.push(ch);
        if let Some((range, previous)) = self.highlights.last_mut() {
            if *previous == style {
                range.end = text.len();
                return;
            }
        }
        self.highlights.push((start..text.len(), style));
    }

    fn pad(&mut self, text: &mut String, source: usize, style: HighlightStyle) {
        let start = text.len();
        // Nonbreaking space reserves real font-relative width without creating a
        // new wrap opportunity between the badge edge and its first/last glyph.
        self.push(text, '\u{a0}', style);
        self.pads.push((start..text.len(), source));
    }

    pub fn source_offset(&self, display: usize) -> usize {
        let mut removed = 0;
        for (range, source) in &self.pads {
            if display < range.start {
                break;
            }
            if display <= range.end {
                return *source;
            }
            removed += range.len();
        }
        display.saturating_sub(removed).min(self.source.len())
    }

    pub fn display_offset(&self, source: usize) -> usize {
        source
            + self
                .pads
                .iter()
                .filter(|(_, offset)| *offset <= source)
                .map(|(range, _)| range.len())
                .sum::<usize>()
    }

    pub fn source_range(&self, range: Range<usize>) -> Range<usize> {
        self.source_offset(range.start)..self.source_offset(range.end)
    }

    pub fn slice(&self, range: Range<usize>) -> Self {
        let source_range = self.source_range(range.clone());
        let clip = |span: &Range<usize>| {
            let start = span.start.max(range.start);
            let end = span.end.min(range.end);
            (start < end)
                .then_some(start.saturating_sub(range.start)..end.saturating_sub(range.start))
        };
        Self {
            source: self.source[source_range.clone()].to_owned().into(),
            text: self.text[range.clone()].to_owned().into(),
            highlights: self
                .highlights
                .iter()
                .filter_map(|(span, style)| clip(span).map(|span| (span, *style)))
                .collect(),
            pads: self
                .pads
                .iter()
                .filter_map(|(span, source)| {
                    clip(span).map(|span| (span, source.saturating_sub(source_range.start)))
                })
                .collect(),
            backgrounds: self
                .backgrounds
                .iter()
                .filter_map(|(span, color)| clip(span).map(|span| (span, *color)))
                .collect(),
            radius: self.radius,
        }
    }

    pub fn paint(&self, layout: &TextLayout, window: &mut Window, _cx: &mut App) {
        let height = layout.line_height();
        let inset = px(2.).min(height * 0.1);
        for (range, color) in &self.backgrounds {
            let mut row: Option<Bounds<Pixels>> = None;
            for (relative, ch) in self.text[range.clone()].char_indices() {
                let offset = range.start + relative;
                let Some(pos) = layout.position_for_index(offset) else {
                    continue;
                };
                let right = layout
                    .position_for_index(offset + ch.len_utf8())
                    .filter(|next| next.y == pos.y)
                    .map_or(pos.x + height * 0.4, |next| next.x);
                let cell = Bounds::from_corners(
                    point(pos.x, pos.y + inset),
                    point(right, pos.y + height - inset),
                );
                if row
                    .as_ref()
                    .is_some_and(|row| row.origin.y != cell.origin.y)
                {
                    window.paint_quad(fill(row.take().unwrap(), *color).corner_radii(self.radius));
                }
                row = Some(row.map_or(cell, |row| row.union(&cell)));
            }
            if let Some(row) = row {
                window.paint_quad(fill(row, *color).corner_radii(self.radius));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn padding_preserves_unicode_source_offsets_and_fragment_copy() {
        let source: SharedString = "前 `中文🙂` 后".into();
        let start = source.find('`').unwrap();
        let end = source.rfind('`').unwrap() + 1;
        let background = InlineBackground::new(
            source.clone(),
            &[(
                start..end,
                HighlightStyle {
                    background_color: Some(gpui::black()),
                    ..Default::default()
                },
            )],
            px(5.),
        )
        .unwrap();
        for offset in source
            .char_indices()
            .map(|(offset, _)| offset)
            .chain([source.len()])
        {
            assert_eq!(
                background.source_offset(background.display_offset(offset)),
                offset
            );
        }
        assert_eq!(
            background.source_range(0..background.text.len()),
            0..source.len()
        );
        let split = background.display_offset(start + 1);
        let left = background.slice(0..split);
        let right = background.slice(split..background.text.len());
        assert_eq!(format!("{}{}", left.source, right.source), source.as_ref());
        assert!(
            background
                .highlights
                .iter()
                .all(|(_, style)| style.background_color.is_none())
        );
        assert!(InlineBackground::new(source, &[], px(5.)).is_none());
    }
}
