//! Layout reconstruction: positioned [`TextRun`]s → reading-order lines.
//!
//! PDF text is placed glyph-by-glyph with no notion of lines or words. This
//! groups runs into lines by clustering their baselines (`y`, top-down since PDF
//! `y` grows upward), orders each line left-to-right, and joins the runs —
//! inserting a space where the horizontal gap between consecutive runs is wide
//! enough to be a word break. A page with no runs is flagged `needs_ocr` (it is
//! likely a scanned image; OCR is out of scope).

use std::cmp::Ordering;

use serde::Serialize;

use crate::image::ParsedImage;
use crate::tables::Table;
use crate::text::TextRun;

/// One reconstructed line of text at its origin.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Line {
    pub text: String,
    pub x: f64,
    pub y: f64,
}

/// A page's reconstructed text, geometry, tables, and images.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PageText {
    pub width: f64,
    pub height: f64,
    pub lines: Vec<Line>,
    pub needs_ocr: bool,
    #[serde(default)]
    pub tables: Vec<Table>,
    #[serde(default)]
    pub images: Vec<ParsedImage>,
}

impl PageText {
    /// The page's text as newline-joined lines.
    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Reconstruct a page's text lines from its runs and media box `[x0 y0 x1 y1]`.
/// Tables and images are filled in later by [`crate::Document::extract`].
pub fn layout_page(runs: &[TextRun], media_box: [f64; 4]) -> PageText {
    PageText {
        width: (media_box[2] - media_box[0]).abs(),
        height: (media_box[3] - media_box[1]).abs(),
        lines: build_lines(runs),
        needs_ocr: runs.is_empty(),
        tables: Vec::new(),
        images: Vec::new(),
    }
}

/// Cluster runs into reading-order lines.
fn build_lines(runs: &[TextRun]) -> Vec<Line> {
    let mut order: Vec<&TextRun> = runs.iter().collect();
    order.sort_by(|a, b| cmp_run(a, b));
    let mut lines = Vec::new();
    let mut current: Vec<&TextRun> = Vec::new();
    for run in order {
        if line_break(&current, run) {
            lines.push(make_line(&current));
            current.clear();
        }
        current.push(run);
    }
    if !current.is_empty() {
        lines.push(make_line(&current));
    }
    lines
}

/// Order runs top-to-bottom (`y` descending), then left-to-right (`x`).
fn cmp_run(a: &TextRun, b: &TextRun) -> Ordering {
    b.y.partial_cmp(&a.y)
        .unwrap_or(Ordering::Equal)
        .then_with(|| a.x.partial_cmp(&b.x).unwrap_or(Ordering::Equal))
}

/// True when `run` starts a new line relative to the current cluster.
fn line_break(current: &[&TextRun], run: &TextRun) -> bool {
    match current.first() {
        None => false,
        Some(first) => {
            if (first.y - run.y).abs() > line_tolerance(first, run) {
                return true;
            }
            // When baselines are close but the run starts substantially to
            // the left of where the current line left off, treat it as a new
            // line.  This catches line wraps in CJK PDFs whose tight line
            // spacing keeps y-differences within tolerance, as well as math
            // exam options where braces and body text share similar Y but the
            // X gap is enormous (full column width → clear new line).
            if let Some(last) = current.last() {
                let prev_end = last.x + last.width;
                let em = first.font_size.max(run.font_size);
                let reset_threshold = em * 1.2;

                // A very large leftward jump (e.g. end-of-line to next-line
                // start) always signals a new line, even when Y is nearly
                // identical — mixed font sizes can inflate line_tolerance
                // beyond the actual inter-line spacing.
                if prev_end - run.x > em * 2.0 {
                    return true;
                }

                // Moderate leftward jump combined with a small Y shift also
                // indicates a new line (e.g. separate equations).
                let y_diff = (first.y - run.y).abs();
                let has_y_shift = y_diff > 0.1 && y_diff <= line_tolerance(first, run);

                if prev_end - run.x > reset_threshold && has_y_shift {
                    return true;
                }
            }
            false
        }
    }
}

/// The baseline tolerance below which two runs share a line.
/// Uses 0.4× the smaller of the two font sizes — using max inflated the
/// tolerance when math-symbol fonts (often larger) appeared beside body text,
/// causing tight lines (6 pt spacing) to merge.
fn line_tolerance(a: &TextRun, b: &TextRun) -> f64 {
    (a.font_size.min(b.font_size) * 0.4).max(1.0)
}

/// Join a line's runs, inserting spaces at wide gaps.
fn make_line(runs: &[&TextRun]) -> Line {
    let mut text = String::new();
    let mut prev_end: Option<f64> = None;
    for run in runs {
        if should_space(prev_end, &text, run) {
            text.push(' ');
        }
        text.push_str(&run.text);
        prev_end = Some(run.x + run.width);
    }
    Line {
        text,
        x: runs[0].x,
        y: runs[0].y,
    }
}

/// Whether a space should separate the previous run from `run`.
fn should_space(prev_end: Option<f64>, text: &str, run: &TextRun) -> bool {
    let Some(pe) = prev_end else {
        return false;
    };
    if text.ends_with(' ') || run.text.starts_with(' ') {
        return false;
    }
    // A tight char (math operator / bracket) belongs adjacent to its neighbor
    // at normal spacing - this keeps "1≤x≤3" and "A={x|...}" tight. But when
    // the gap is very wide the producer clearly intended a word break, so still
    // space (e.g. "2x+3 = 7", where '=' sits far from '7').
    if ends_with_tight_char(text) || starts_with_tight_char(&run.text) {
        return run.x - pe > 0.5 * run.font_size.max(1.0);
    }
    gap_is_wide(pe, run)
}

/// A gap wider than a fraction of the em is a word break.
/// Uses 0.3em threshold for better handling of math symbols and tighter layouts.
fn gap_is_wide(prev_end: f64, run: &TextRun) -> bool {
    run.x - prev_end > 0.3 * run.font_size.max(1.0)
}

/// Characters that typically don't have space after them (math/punctuation).
fn ends_with_tight_char(text: &str) -> bool {
    text.ends_with(|c: char| {
        matches!(c,
            '(' | '[' | '{' | '<' | '|' | '\u{FF5C}' | // Brackets and bars (including fullwidth)
            '=' | '+' | '-' | '*' | '/' | '×' | '÷' | // Math operators
            '≤' | '≥' | '≠' | '≈' | // Comparisons
            '∈' | '∉' | '⊂' | '⊃' | '⊆' | '⊇' | '∩' | '∪' | '∅' | // Set operations
            '∂' | '∫' | '∑' | '∏' | '√' // Calculus
        )
    })
}

/// Characters that typically don't have space before them.
fn starts_with_tight_char(text: &str) -> bool {
    text.starts_with(|c: char| {
        matches!(c,
            ')' | ']' | '}' | '>' | '|' | '\u{FF5C}' |
            ',' | '.' | ';' | ':' | '!' | '?' |
            '≤' | '≥' | '∈' | '∉'
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, x: f64, y: f64, width: f64) -> TextRun {
        TextRun {
            text: text.into(),
            x,
            y,
            width,
            font_size: 10.0,
            font: "F1".into(),
        }
    }

    #[test]
    fn groups_runs_into_lines_top_down() {
        let runs = vec![
            run("world", 50.0, 700.0, 30.0),
            run("Hello", 10.0, 700.0, 30.0),
            run("second", 10.0, 680.0, 40.0),
        ];
        let page = layout_page(&runs, [0.0, 0.0, 200.0, 800.0]);
        assert_eq!(page.lines.len(), 2);
        // Same-baseline runs merge, ordered by x, with a gap space.
        assert_eq!(page.lines[0].text, "Hello world");
        assert_eq!(page.lines[1].text, "second");
        assert_eq!(page.text(), "Hello world\nsecond");
        assert_eq!((page.width, page.height), (200.0, 800.0));
        assert!(!page.needs_ocr);
    }

    #[test]
    fn adjacent_runs_have_no_space() {
        // Second run starts right where the first ends → no inserted space.
        let runs = vec![run("foo", 10.0, 100.0, 18.0), run("bar", 28.0, 100.0, 18.0)];
        let page = layout_page(&runs, [0.0, 0.0, 100.0, 200.0]);
        assert_eq!(page.lines[0].text, "foobar");
    }

    #[test]
    fn existing_spaces_not_doubled() {
        let runs = vec![
            run("foo ", 10.0, 100.0, 24.0),
            run("bar", 50.0, 100.0, 18.0),
        ];
        let page = layout_page(&runs, [0.0, 0.0, 100.0, 200.0]);
        assert_eq!(page.lines[0].text, "foo bar");
    }

    #[test]
    fn empty_page_needs_ocr() {
        let page = layout_page(&[], [0.0, 0.0, 612.0, 792.0]);
        assert!(page.needs_ocr);
        assert!(page.lines.is_empty());
        assert_eq!(page.text(), "");
    }

    #[test]
    fn helpers_handle_edges() {
        // line_break with empty current is false.
        assert!(!line_break(&[], &run("x", 0.0, 0.0, 1.0)));
        // should_space with no previous run is false.
        assert!(!should_space(None, "", &run("x", 0.0, 0.0, 1.0)));
        // gap exactly zero is not wide.
        assert!(!gap_is_wide(10.0, &run("x", 10.0, 0.0, 1.0)));
        assert!(gap_is_wide(10.0, &run("x", 30.0, 0.0, 1.0)));
        // NaN-free ordering fallback.
        assert_eq!(
            cmp_run(&run("a", 0.0, 5.0, 1.0), &run("b", 0.0, 5.0, 1.0)),
            Ordering::Equal
        );
    }

    #[test]
    fn x_reset_detects_line_break_when_y_close() {
        // Two runs on the same logical line: y close, x monotonic.
        let a = run("Hello", 10.0, 700.0, 30.0);
        let b = run("world", 42.0, 699.0, 30.0);
        assert!(!line_break(&[&a], &b)); // still same line

        // Title at x=50 ends at 350, next line at x=50 starts far to the
        // left of the previous line end → x-reset breaks the line even
        // though y-difference (3pt) is within tolerance (5pt).
        let title = run("A long title that spans far", 50.0, 700.0, 300.0);
        let body = run("Body text", 50.0, 697.0, 60.0);
        assert!(line_break(&[&title], &body));
    }

    #[test]
    fn x_reset_splits_tight_cjk_lines() {
        // Simulate a Chinese exam paper where title and section header
        // have close baselines but clearly belong on different lines.
        let title = run("2020年新高考全国I卷（山东卷）数学", 50.0, 700.0, 250.0);
        let header = run("一、选择题", 50.0, 695.0, 60.0);
        let layout = layout_page(&[title, header], [0.0, 0.0, 600.0, 800.0]);
        assert_eq!(layout.lines.len(), 2);
        assert_eq!(layout.lines[0].text, "2020年新高考全国I卷（山东卷）数学");
        assert_eq!(layout.lines[1].text, "一、选择题");
    }

    #[test]
    fn math_symbols_no_unwanted_spaces() {
        // Set notation: A={x|1≤x≤3} should not have spaces around brackets or operators
        let runs = vec![
            run("A", 10.0, 700.0, 8.0),
            run("=", 18.5, 700.0, 6.0),
            run("{", 25.0, 700.0, 4.0),
            run("x", 29.5, 700.0, 6.0),
            run("|", 36.0, 700.0, 3.0),
            run("1", 39.5, 700.0, 6.0),
            run("≤", 46.0, 700.0, 6.0),
            run("x", 52.5, 700.0, 6.0),
            run("≤", 59.0, 700.0, 6.0),
            run("3", 65.5, 700.0, 6.0),
            run("}", 72.0, 700.0, 4.0),
        ];
        let page = layout_page(&runs, [0.0, 0.0, 200.0, 800.0]);
        assert_eq!(page.lines.len(), 1);
        // Should be tight: A={x|1≤x≤3}
        // NO spaces around =, {, |, ≤, }
        assert_eq!(page.lines[0].text, "A={x|1≤x≤3}");
    }

    #[test]
    fn math_formulas_with_appropriate_spacing() {
        // Mathematical expression with both tight and spaced elements
        let runs = vec![
            run("2", 10.0, 700.0, 6.0),
            run("x", 17.0, 700.0, 6.0),  // Small gap after number
            run("+", 24.0, 700.0, 6.0),
            run("3", 31.0, 700.0, 6.0),
            run("=", 45.0, 700.0, 6.0),  // Wider gap before =
            run("7", 59.0, 700.0, 6.0),
        ];
        let page = layout_page(&runs, [0.0, 0.0, 200.0, 800.0]);
        assert_eq!(page.lines.len(), 1);
        // Should have space around = but not around + (tight layout)
        // With 0.3em threshold, most gaps stay tight
        assert_eq!(page.lines[0].text, "2x+3 = 7");
    }

    #[test]
    fn separate_lines_for_multiple_equations() {
        // Two equations on separate lines with Y difference of 10pt
        let runs = vec![
            run("A", 50.0, 700.0, 8.0),
            run("=", 58.5, 700.0, 6.0),
            run("{x|1≤x≤3}", 65.0, 700.0, 70.0),
            run("B", 50.0, 690.0, 8.0),  // 10pt below, same X start
            run("=", 58.5, 690.0, 6.0),
            run("{x|2<x<4}", 65.0, 690.0, 70.0),
        ];
        let page = layout_page(&runs, [0.0, 0.0, 200.0, 800.0]);
        assert_eq!(page.lines.len(), 2);
        assert_eq!(page.lines[0].text, "A={x|1≤x≤3}");
        assert_eq!(page.lines[1].text, "B={x|2<x<4}");
    }
}
