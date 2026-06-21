//! The low-level COS byte scanner.
//!
//! A [`Lexer`] is a cursor over the raw PDF bytes that knows the PDF character
//! classes (ISO 32000-1 §7.2): whitespace, the eight delimiter bytes, and the
//! "regular" bytes that make up numbers, names and keywords. It exposes the
//! primitives the recursive-descent object parser ([`crate::cos`]) builds on —
//! whitespace/comment skipping and keyword scanning — but does not itself decide
//! what an object *is*.

/// The six PDF whitespace bytes (§7.2.3): NUL, TAB, LF, FF, CR, SPACE.
pub fn is_whitespace(b: u8) -> bool {
    matches!(b, 0x00 | 0x09 | 0x0a | 0x0c | 0x0d | 0x20)
}

/// The eight PDF delimiter bytes (§7.2.3): `( ) < > [ ] { } /` and `%`.
pub fn is_delimiter(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

/// A "regular" byte: anything that is neither whitespace nor a delimiter. These
/// form the runs that become numbers, the bodies of names, and keywords.
pub fn is_regular(b: u8) -> bool {
    !is_whitespace(b) && !is_delimiter(b)
}

/// A forward cursor over PDF bytes.
#[derive(Debug, Clone)]
pub struct Lexer<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    /// A lexer positioned at the start of `data`.
    pub fn new(data: &'a [u8]) -> Self {
        Lexer { data, pos: 0 }
    }

    /// A lexer positioned at byte offset `pos` within `data`.
    pub fn at(data: &'a [u8], pos: usize) -> Self {
        Lexer {
            data,
            pos: pos.min(data.len()),
        }
    }

    /// The full underlying buffer.
    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    /// The current byte offset.
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Move the cursor to an absolute offset (clamped to the buffer length).
    pub fn seek(&mut self, pos: usize) {
        self.pos = pos.min(self.data.len());
    }

    /// True once the cursor has reached the end of the buffer.
    pub fn at_end(&self) -> bool {
        self.pos >= self.data.len()
    }

    /// The byte at the cursor without advancing.
    pub fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    /// The byte `n` ahead of the cursor without advancing.
    pub fn peek_at(&self, n: usize) -> Option<u8> {
        self.data.get(self.pos + n).copied()
    }

    /// Return the byte at the cursor and advance past it.
    pub fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    /// Advance the cursor by `n` bytes (clamped to the buffer length).
    pub fn advance(&mut self, n: usize) {
        self.seek(self.pos + n);
    }

    /// Skip past whitespace and `%`-to-end-of-line comments.
    pub fn skip_whitespace(&mut self) {
        while let Some(b) = self.peek() {
            if b == b'%' {
                self.skip_comment();
            } else if is_whitespace(b) {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    /// Skip a `%` comment through the next end-of-line.
    fn skip_comment(&mut self) {
        while let Some(b) = self.peek() {
            self.pos += 1;
            if b == b'\n' || b == b'\r' {
                break;
            }
        }
    }

    /// Read a run of regular bytes as a keyword/identifier (e.g. `obj`, `R`,
    /// `true`, `stream`). Returns `None` at a delimiter/whitespace/EOF.
    pub fn read_keyword(&mut self) -> Option<&'a [u8]> {
        let start = self.pos;
        while self.peek().is_some_and(is_regular) {
            self.pos += 1;
        }
        if self.pos == start {
            None
        } else {
            Some(&self.data[start..self.pos])
        }
    }

    /// If the bytes at the cursor equal `kw`, consume them and return true.
    /// Requires a non-regular byte (or EOF) after, so `stream` does not match a
    /// prefix of `streamfoo`.
    pub fn eat_keyword(&mut self, kw: &[u8]) -> bool {
        let end = self.pos + kw.len();
        if self.data.get(self.pos..end) != Some(kw) {
            return false;
        }
        if self.data.get(end).copied().is_some_and(is_regular) {
            return false;
        }
        self.pos = end;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_classes() {
        assert!(is_whitespace(b' '));
        assert!(is_whitespace(0));
        assert!(!is_whitespace(b'a'));
        assert!(is_delimiter(b'<'));
        assert!(is_delimiter(b'%'));
        assert!(!is_delimiter(b'a'));
        assert!(is_regular(b'a'));
        assert!(!is_regular(b' '));
        assert!(!is_regular(b'/'));
    }

    #[test]
    fn cursor_basics() {
        let mut lx = Lexer::new(b"abc");
        assert_eq!(lx.pos(), 0);
        assert_eq!(lx.peek(), Some(b'a'));
        assert_eq!(lx.peek_at(2), Some(b'c'));
        assert_eq!(lx.bump(), Some(b'a'));
        assert_eq!(lx.bump(), Some(b'b'));
        assert!(!lx.at_end());
        assert_eq!(lx.bump(), Some(b'c'));
        assert!(lx.at_end());
        assert_eq!(lx.bump(), None);
        assert_eq!(lx.peek(), None);
        assert_eq!(lx.data(), b"abc");
    }

    #[test]
    fn seek_and_advance_clamp() {
        let mut lx = Lexer::at(b"abcdef", 2);
        assert_eq!(lx.pos(), 2);
        lx.advance(2);
        assert_eq!(lx.pos(), 4);
        lx.seek(99);
        assert_eq!(lx.pos(), 6);
        assert!(lx.at_end());
        // `at` past the end clamps too.
        assert_eq!(Lexer::at(b"ab", 10).pos(), 2);
    }

    #[test]
    fn skips_whitespace_and_comments() {
        let mut lx = Lexer::new(b"  % a comment\r\n\t 42");
        lx.skip_whitespace();
        assert_eq!(lx.peek(), Some(b'4'));
    }

    #[test]
    fn comment_at_eof_terminates() {
        let mut lx = Lexer::new(b"  %trailing");
        lx.skip_whitespace();
        assert!(lx.at_end());
    }

    #[test]
    fn reads_keyword_runs() {
        let mut lx = Lexer::new(b"obj 1");
        assert_eq!(lx.read_keyword(), Some(&b"obj"[..]));
        lx.skip_whitespace();
        assert_eq!(lx.read_keyword(), Some(&b"1"[..]));
        // At a delimiter there is no keyword.
        let mut lx2 = Lexer::new(b"/Name");
        assert_eq!(lx2.read_keyword(), None);
    }

    #[test]
    fn eat_keyword_requires_boundary() {
        let mut lx = Lexer::new(b"stream\ndata");
        assert!(lx.eat_keyword(b"stream"));
        assert_eq!(lx.peek(), Some(b'\n'));
        // No prefix match into a longer regular run.
        let mut lx2 = Lexer::new(b"streamfoo");
        assert!(!lx2.eat_keyword(b"stream"));
        assert_eq!(lx2.pos(), 0);
        // No match at all.
        let mut lx3 = Lexer::new(b"xyz");
        assert!(!lx3.eat_keyword(b"obj"));
    }
}
