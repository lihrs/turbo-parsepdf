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
        Some(first) => (first.y - run.y).abs() > line_tolerance(first, run),
    }
}

/// The baseline tolerance below which two runs share a line.
fn line_tolerance(a: &TextRun, b: &TextRun) -> f64 {
    (a.font_size.max(b.font_size) * 0.5).max(1.0)
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
    gap_is_wide(pe, run) && !text.ends_with(' ') && !run.text.starts_with(' ')
}

/// A gap wider than a fraction of the em is a word break.
fn gap_is_wide(prev_end: f64, run: &TextRun) -> bool {
    run.x - prev_end > 0.2 * run.font_size.max(1.0)
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
}
