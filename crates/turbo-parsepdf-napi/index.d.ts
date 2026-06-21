// Hand-maintained type surface for the turbo-parsepdf N-API addon. Mirrors the
// `#[napi]` exports in `src/lib.rs` and the core `ExtractedDoc` JSON shape (see
// schema/turbo-parsepdf.doc.schema.json).

/** A reconstructed line of text at its device-space origin. */
export interface Line {
  text: string;
  x: number;
  y: number;
}

/** A recovered ruled table: cell text in [row][col] order. */
export interface Table {
  rows: number;
  cols: number;
  cells: string[][];
}

/**
 * An extracted image XObject's metadata (bytes are not marshaled to JS).
 * Keys are snake_case to match the JSON schema produced by `parseToJson`.
 */
export interface ParsedImage {
  name: string;
  format: "Jpeg" | "Jpeg2000" | "Ccitt" | "Jbig2" | "Raw";
  width: number;
  height: number;
  bits_per_component: number;
  color_space: string;
}

/** One page's reconstructed content and geometry. */
export interface PageText {
  width: number;
  height: number;
  lines: Line[];
  needs_ocr: boolean;
  tables: Table[];
  images: ParsedImage[];
}

/** The extracted content of a whole document. */
export interface ExtractedDoc {
  version: string;
  pages: PageText[];
}

/** Parse a PDF buffer and return the structured extracted document. */
export function parse(data: Buffer, password?: string): ExtractedDoc;

/** Parse a PDF buffer and return its extracted form as pretty JSON. */
export function parseToJson(data: Buffer, password?: string): string;

/** Parse a PDF buffer and render it to a standalone HTML document. */
export function parseToHtml(data: Buffer, password?: string): string;

/** Parse a PDF buffer and render it to Markdown. */
export function parseToMarkdown(data: Buffer, password?: string): string;
