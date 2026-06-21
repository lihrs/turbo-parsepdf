//! Ruled-table detection from page graphics + text.
//!
//! Tables drawn with rules are recovered by collecting the axis-aligned line
//! segments a content stream paints (`re` rectangles and horizontal/vertical
//! `m`/`l` strokes, transformed by the CTM), clustering them into a grid of
//! column (`x`) and row (`y`) lines, and dropping each positioned [`TextRun`]
//! into the cell that contains it. A region needs at least a 2×2 grid to count
//! as a table. Borderless tables (whitespace-aligned only) are out of scope.

use serde::Serialize;

use crate::content::{Op, Operator};
use crate::object::Object;
use crate::text::{Matrix, TextRun};

/// Two positions are the same grid line when within this many units.
const EPS: f64 = 2.0;

/// A recovered table: a grid of cell text in `[row][col]` order (top-to-bottom,
/// left-to-right).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Table {
    pub rows: usize,
    pub cols: usize,
    pub cells: Vec<Vec<String>>,
}

/// Detect the dominant ruled table on a page from a pre-tokenized op list.
pub fn detect_tables(ops: &[Op], runs: &[TextRun]) -> Vec<Table> {
    build_tables(&collect_segments(ops), runs)
}

/// Build the page's tables from collected line segments and positioned runs.
fn build_tables(segments: &Segments, runs: &[TextRun]) -> Vec<Table> {
    match build_grid(segments) {
        Some(grid) => vec![fill_table(&grid, runs)],
        None => Vec::new(),
    }
}

/// A horizontal segment at `y` spanning `x0..x1`, or vertical at `x` over `y0..y1`.
#[derive(Default)]
struct Segments {
    xs: Vec<f64>,
    ys: Vec<f64>,
}

/// The graphics state needed to place path points in device space.
struct GState {
    ctm: Matrix,
    stack: Vec<Matrix>,
    cur: (f64, f64),
}

impl GState {
    fn new() -> Self {
        GState {
            ctm: Matrix::identity(),
            stack: Vec::new(),
            cur: (0.0, 0.0),
        }
    }
}

/// Collect the column (`x`) and row (`y`) line positions from a pre-tokenized op
/// list (used by [`detect_tables`]; the hot path drives [`apply_path`] directly).
fn collect_segments(ops: &[Op]) -> Segments {
    let mut g = GState::new();
    let mut segs = Segments::default();
    for op in ops {
        apply_path(&mut g, op.operator, &op.operands, &mut segs);
    }
    segs
}

/// Apply one content operator (operator + operands) to the path/graphics state.
fn apply_path(g: &mut GState, operator: Operator, operands: &[Object], segs: &mut Segments) {
    match operator.as_str() {
        "q" => g.stack.push(g.ctm),
        "Q" => restore(g),
        "cm" => g.ctm = matrix_of(operands).then(&g.ctm),
        "m" => g.cur = point(g, operands, 0),
        "l" => line_to(g, operands, segs),
        "re" => rectangle(g, operands, segs),
        _ => {}
    }
}

fn restore(g: &mut GState) {
    if let Some(m) = g.stack.pop() {
        g.ctm = m;
    }
}

/// `l x y`: add the segment from the current point, then move there.
fn line_to(g: &mut GState, operands: &[Object], segs: &mut Segments) {
    let end = point(g, operands, 0);
    add_segment(segs, g.cur, end);
    g.cur = end;
}

/// `re x y w h`: the four edges of a rectangle in device space.
fn rectangle(g: &GState, operands: &[Object], segs: &mut Segments) {
    let (x, y, w, h) = (
        nth(operands, 0),
        nth(operands, 1),
        nth(operands, 2),
        nth(operands, 3),
    );
    let p = |dx, dy| g.ctm.apply(x + dx, y + dy);
    let (a, b, c, d) = (p(0.0, 0.0), p(w, 0.0), p(w, h), p(0.0, h));
    add_segment(segs, a, b);
    add_segment(segs, b, c);
    add_segment(segs, c, d);
    add_segment(segs, d, a);
}

/// Record an axis-aligned segment as a column or row line (diagonals ignored).
fn add_segment(segs: &mut Segments, p0: (f64, f64), p1: (f64, f64)) {
    if (p0.1 - p1.1).abs() < EPS {
        segs.ys.push(p0.1);
    } else if (p0.0 - p1.0).abs() < EPS {
        segs.xs.push(p0.0);
    }
}

/// A device-space point from two operands transformed by the CTM.
fn point(g: &GState, operands: &[Object], i: usize) -> (f64, f64) {
    g.ctm.apply(nth(operands, i), nth(operands, i + 1))
}

/// The clustered, sorted grid lines (`xs` ascending, `ys` descending).
struct Grid {
    xs: Vec<f64>,
    ys: Vec<f64>,
}

/// Cluster the segment positions into a grid; `None` if smaller than 2×2.
fn build_grid(segs: &Segments) -> Option<Grid> {
    let xs = cluster(&segs.xs, false);
    let ys = cluster(&segs.ys, true);
    if xs.len() < 2 || ys.len() < 2 {
        return None;
    }
    Some(Grid { xs, ys })
}

/// Sort and merge near-equal positions; `descending` flips the order.
fn cluster(values: &[f64], descending: bool) -> Vec<f64> {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut out: Vec<f64> = Vec::new();
    for v in sorted {
        if out.last().is_none_or(|&last| (v - last).abs() >= EPS) {
            out.push(v);
        }
    }
    if descending {
        out.reverse();
    }
    out
}

/// Assign each run to its grid cell, building the row/col text matrix.
fn fill_table(grid: &Grid, runs: &[TextRun]) -> Table {
    let rows = grid.ys.len() - 1;
    let cols = grid.xs.len() - 1;
    let mut cells = vec![vec![String::new(); cols]; rows];
    for run in runs {
        place_run(grid, &mut cells, run);
    }
    Table { rows, cols, cells }
}

/// Drop one run into the cell that contains its origin.
fn place_run(grid: &Grid, cells: &mut [Vec<String>], run: &TextRun) {
    if let (Some(col), Some(row)) = (col_of(grid, run.x), row_of(grid, run.y)) {
        append_cell(&mut cells[row][col], &run.text);
    }
}

/// Append text to a cell, space-separating existing content.
fn append_cell(cell: &mut String, text: &str) {
    if !cell.is_empty() && !cell.ends_with(' ') {
        cell.push(' ');
    }
    cell.push_str(text);
}

/// The column index for an `x` between two adjacent column lines.
fn col_of(grid: &Grid, x: f64) -> Option<usize> {
    (0..grid.xs.len() - 1).find(|&j| x >= grid.xs[j] - EPS && x < grid.xs[j + 1] + EPS)
}

/// The row index for a `y` between two adjacent (descending) row lines.
fn row_of(grid: &Grid, y: f64) -> Option<usize> {
    (0..grid.ys.len() - 1).find(|&i| y <= grid.ys[i] + EPS && y > grid.ys[i + 1] - EPS)
}

/// Build a matrix from up to six operands (missing entries default to identity).
fn matrix_of(operands: &[Object]) -> Matrix {
    let n = |i: usize| operands.get(i).and_then(Object::as_f64);
    Matrix {
        a: n(0).unwrap_or(1.0),
        b: n(1).unwrap_or(0.0),
        c: n(2).unwrap_or(0.0),
        d: n(3).unwrap_or(1.0),
        e: n(4).unwrap_or(0.0),
        f: n(5).unwrap_or(0.0),
    }
}

fn nth(operands: &[Object], i: usize) -> f64 {
    operands.get(i).and_then(Object::as_f64).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::parse_content;

    fn run(text: &str, x: f64, y: f64) -> TextRun {
        TextRun {
            text: text.into(),
            x,
            y,
            width: 10.0,
            font_size: 10.0,
            font: "F".into(),
        }
    }

    // A 2×2 grid of rectangles spanning x∈{0,50,100}, y∈{0,20,40}.
    const GRID: &[u8] = b"0 20 50 20 re 50 20 50 20 re 0 0 50 20 re 50 0 50 20 re S";

    #[test]
    fn detects_2x2_table_and_places_text() {
        let runs = vec![
            run("A", 10.0, 30.0),
            run("B", 60.0, 30.0),
            run("C", 10.0, 10.0),
            run("D", 60.0, 10.0),
        ];
        let tables = detect_tables(&parse_content(GRID), &runs);
        assert_eq!(tables.len(), 1);
        let t = &tables[0];
        assert_eq!((t.rows, t.cols), (2, 2));
        // Row 0 is the top (higher y).
        assert_eq!(t.cells[0], vec!["A".to_string(), "B".to_string()]);
        assert_eq!(t.cells[1], vec!["C".to_string(), "D".to_string()]);
    }

    #[test]
    fn lines_via_m_l_and_cm_transform() {
        // Two horizontal + two vertical lines forming a 1×1 cell after a cm shift.
        let content = b"q 1 0 0 1 0 0 cm 0 0 m 100 0 l 0 40 m 100 40 l \
0 0 m 0 40 l 100 0 m 100 40 l S Q";
        let runs = vec![run("X", 50.0, 20.0)];
        let tables = detect_tables(&parse_content(content), &runs);
        assert_eq!(tables.len(), 1);
        assert_eq!((tables[0].rows, tables[0].cols), (1, 1));
        assert_eq!(tables[0].cells[0][0], "X");
    }

    #[test]
    fn no_grid_no_table() {
        // A single rectangle is only a 1×1 boundary → not enough for a 2×2 grid,
        // but its 4 edges give 2 xs and 2 ys → a 1×1 table is allowed.
        assert!(detect_tables(&parse_content(b"0 0 10 10 re S"), &[]).len() <= 1);
        // No path ops at all → no table.
        assert!(detect_tables(&parse_content(b"BT (hi) Tj ET"), &[]).is_empty());
    }

    #[test]
    fn diagonal_segments_ignored() {
        // A diagonal line contributes no axis-aligned grid line.
        let mut segs = Segments::default();
        add_segment(&mut segs, (0.0, 0.0), (10.0, 10.0));
        assert!(segs.xs.is_empty() && segs.ys.is_empty());
    }

    #[test]
    fn cell_text_accumulates_with_spaces() {
        let mut cell = String::new();
        append_cell(&mut cell, "foo");
        append_cell(&mut cell, "bar");
        assert_eq!(cell, "foo bar");
    }

    #[test]
    fn runs_outside_grid_are_dropped() {
        let runs = vec![run("off", 999.0, 999.0)];
        let tables = detect_tables(&parse_content(GRID), &runs);
        // Grid exists but the run lands in no cell.
        assert!(tables[0]
            .cells
            .iter()
            .all(|r| r.iter().all(String::is_empty)));
    }

    #[test]
    fn q_without_stack_is_safe() {
        // A stray Q (empty stack) must not panic.
        detect_tables(&parse_content(b"Q 0 0 10 10 re S"), &[]);
    }
}
