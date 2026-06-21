//! Error and diagnostic types shared across the parser.
//!
//! The split mirrors the sibling `turbo-xlsx` / `turbo-html2pdf` engines: a fatal
//! fault is a [`TurboParsePdfError`] carrying a stable machine-readable
//! [`ErrorCode`]; non-fatal problems (a recovered xref, a dropped malformed
//! object) are collected as [`Lint`]s in [`Diagnostics`] and returned alongside
//! the parsed document, never thrown.

use thiserror::Error;

/// Stable machine-readable code for a fatal parse fault. The string form (the
/// variant name) crosses the N-API boundary as `TurboParsePdfError.code`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// The input did not begin with a `%PDF-` header signature.
    InvalidHeader,
    /// The cross-reference table / stream could not be located or parsed and no
    /// recovery scan succeeded.
    InvalidXref,
    /// An indirect object header (`n g obj … endobj`) was malformed.
    InvalidObject,
    /// The COS tokenizer hit a byte sequence that is not a valid object.
    UnexpectedToken,
    /// Input ended while an object/stream/structure was still open.
    UnexpectedEof,
    /// A stream's filter chain failed to decode (e.g. corrupt DEFLATE data).
    BadStream,
    /// The trailer dictionary was missing or had no usable `/Root`.
    MissingTrailer,
    /// A referenced indirect object could not be resolved.
    ObjectNotFound,
    /// A required capability is not implemented for this input.
    Unsupported,
}

impl ErrorCode {
    /// The stable string form of the code (mirrors the variant name).
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::InvalidHeader => "InvalidHeader",
            ErrorCode::InvalidXref => "InvalidXref",
            ErrorCode::InvalidObject => "InvalidObject",
            ErrorCode::UnexpectedToken => "UnexpectedToken",
            ErrorCode::UnexpectedEof => "UnexpectedEof",
            ErrorCode::BadStream => "BadStream",
            ErrorCode::MissingTrailer => "MissingTrailer",
            ErrorCode::ObjectNotFound => "ObjectNotFound",
            ErrorCode::Unsupported => "Unsupported",
        }
    }
}

/// A fatal fault produced while parsing a PDF.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{}: {message}", code.as_str())]
pub struct TurboParsePdfError {
    pub code: ErrorCode,
    pub message: String,
}

impl TurboParsePdfError {
    /// Construct an error from a code and a message.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        TurboParsePdfError {
            code,
            message: message.into(),
        }
    }
}

/// Stable machine-readable code for a non-fatal lint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintCode {
    /// The cross-reference table was unusable, so objects were located by a
    /// brute-force scan for `n g obj` markers.
    RecoveredXref,
    /// A stream's declared `/Length` disagreed with the `endstream` marker; the
    /// scanned length was used.
    LengthMismatch,
    /// A malformed indirect object was skipped during recovery.
    DroppedObject,
}

impl LintCode {
    /// The stable string form of the lint code (mirrors the variant name).
    pub fn as_str(self) -> &'static str {
        match self {
            LintCode::RecoveredXref => "RecoveredXref",
            LintCode::LengthMismatch => "LengthMismatch",
            LintCode::DroppedObject => "DroppedObject",
        }
    }
}

/// A non-fatal diagnostic collected during a parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lint {
    pub code: LintCode,
    pub message: String,
}

/// Collected non-fatal diagnostics returned alongside the document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diagnostics {
    pub lints: Vec<Lint>,
}

impl Diagnostics {
    /// Append a lint to the collection.
    pub fn push(&mut self, code: LintCode, message: impl Into<String>) {
        self.lints.push(Lint {
            code,
            message: message.into(),
        });
    }

    /// True when no lints have been collected.
    pub fn is_empty(&self) -> bool {
        self.lints.is_empty()
    }
}

/// Shorthand result type for fallible parser operations.
pub type Result<T> = std::result::Result<T, TurboParsePdfError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_round_trip_to_strings() {
        let codes = [
            ErrorCode::InvalidHeader,
            ErrorCode::InvalidXref,
            ErrorCode::InvalidObject,
            ErrorCode::UnexpectedToken,
            ErrorCode::UnexpectedEof,
            ErrorCode::BadStream,
            ErrorCode::MissingTrailer,
            ErrorCode::ObjectNotFound,
            ErrorCode::Unsupported,
        ];
        let names: Vec<&str> = codes.iter().map(|c| c.as_str()).collect();
        assert_eq!(
            names,
            [
                "InvalidHeader",
                "InvalidXref",
                "InvalidObject",
                "UnexpectedToken",
                "UnexpectedEof",
                "BadStream",
                "MissingTrailer",
                "ObjectNotFound",
                "Unsupported",
            ]
        );
    }

    #[test]
    fn lint_codes_round_trip_to_strings() {
        let codes = [
            LintCode::RecoveredXref,
            LintCode::LengthMismatch,
            LintCode::DroppedObject,
        ];
        let names: Vec<&str> = codes.iter().map(|c| c.as_str()).collect();
        assert_eq!(names, ["RecoveredXref", "LengthMismatch", "DroppedObject"]);
    }

    #[test]
    fn error_display_uses_code_and_message() {
        let e = TurboParsePdfError::new(ErrorCode::InvalidHeader, "nope");
        assert_eq!(e.to_string(), "InvalidHeader: nope");
    }

    #[test]
    fn diagnostics_push_and_empty() {
        let mut d = Diagnostics::default();
        assert!(d.is_empty());
        d.push(LintCode::RecoveredXref, "x");
        assert!(!d.is_empty());
        assert_eq!(d.lints.len(), 1);
    }
}
