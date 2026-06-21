//! The COS (Carousel Object System) object model — the typed value tree every
//! PDF body object parses into.
//!
//! A PDF file is a graph of eight primitive object types (ISO 32000-1 §7.3):
//! booleans, numbers (integer/real), strings, names, arrays, dictionaries, the
//! null object, plus *streams* (a dictionary followed by raw bytes) and
//! *indirect references* (`n g R`). [`Object`] is that closed set; [`Dictionary`]
//! preserves key insertion order so re-serialization is deterministic.

/// An indirect-object reference: object number + generation (`num gen R`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjRef {
    pub num: u32,
    pub gen: u16,
}

impl ObjRef {
    /// Construct a reference from its object and generation numbers.
    pub fn new(num: u32, gen: u16) -> Self {
        ObjRef { num, gen }
    }
}

/// A PDF dictionary: an order-preserving map from name keys to objects.
///
/// PDF dictionaries are logically unordered, but keeping insertion order makes
/// parser output reproducible and easier to diff in tests.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Dictionary(Vec<(String, Object)>);

impl Dictionary {
    /// An empty dictionary.
    pub fn new() -> Self {
        Dictionary(Vec::new())
    }

    /// Append a key/value pair. A duplicate key keeps the last value (PDF
    /// readers take the most recent), so callers may push freely.
    pub fn insert(&mut self, key: impl Into<String>, value: Object) {
        let key = key.into();
        if let Some(slot) = self.0.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = value;
        } else {
            self.0.push((key, value));
        }
    }

    /// Look up a value by key.
    pub fn get(&self, key: &str) -> Option<&Object> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// True when the dictionary has no entries.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The number of entries.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Iterate over the (key, value) pairs in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Object)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }
}

/// A stream object: a dictionary plus the raw (still filter-encoded) body bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct Stream {
    pub dict: Dictionary,
    /// Raw bytes between `stream` and `endstream`, before any filter is applied.
    pub data: Vec<u8>,
}

/// A COS object: the closed set of PDF value types.
#[derive(Debug, Clone, PartialEq)]
pub enum Object {
    Null,
    Boolean(bool),
    Integer(i64),
    Real(f64),
    /// A string's decoded bytes (literal `(...)` or hex `<...>`).
    String(Vec<u8>),
    /// A name without its leading `/` (e.g. `Type`, `Pages`).
    Name(String),
    Array(Vec<Object>),
    Dictionary(Dictionary),
    Stream(Stream),
    Reference(ObjRef),
}

impl Object {
    /// The integer value, if this is an [`Object::Integer`].
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Object::Integer(n) => Some(*n),
            _ => None,
        }
    }

    /// The numeric value as `f64` (accepts both integer and real).
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Object::Integer(n) => Some(*n as f64),
            Object::Real(r) => Some(*r),
            _ => None,
        }
    }

    /// The boolean value, if this is an [`Object::Boolean`].
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Object::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// The name text (without `/`), if this is an [`Object::Name`].
    pub fn as_name(&self) -> Option<&str> {
        match self {
            Object::Name(n) => Some(n.as_str()),
            _ => None,
        }
    }

    /// The decoded string bytes, if this is an [`Object::String`].
    pub fn as_string(&self) -> Option<&[u8]> {
        match self {
            Object::String(s) => Some(s.as_slice()),
            _ => None,
        }
    }

    /// The element slice, if this is an [`Object::Array`].
    pub fn as_array(&self) -> Option<&[Object]> {
        match self {
            Object::Array(a) => Some(a.as_slice()),
            _ => None,
        }
    }

    /// The dictionary, if this is an [`Object::Dictionary`] or [`Object::Stream`]
    /// (a stream's dictionary is returned for either).
    pub fn as_dict(&self) -> Option<&Dictionary> {
        match self {
            Object::Dictionary(d) => Some(d),
            Object::Stream(s) => Some(&s.dict),
            _ => None,
        }
    }

    /// The stream, if this is an [`Object::Stream`].
    pub fn as_stream(&self) -> Option<&Stream> {
        match self {
            Object::Stream(s) => Some(s),
            _ => None,
        }
    }

    /// The reference, if this is an [`Object::Reference`].
    pub fn as_reference(&self) -> Option<ObjRef> {
        match self {
            Object::Reference(r) => Some(*r),
            _ => None,
        }
    }

    /// True when this is the null object.
    pub fn is_null(&self) -> bool {
        matches!(self, Object::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn objref_fields() {
        let r = ObjRef::new(12, 3);
        assert_eq!(r.num, 12);
        assert_eq!(r.gen, 3);
    }

    #[test]
    fn dictionary_insert_get_and_dedup() {
        let mut d = Dictionary::new();
        assert!(d.is_empty());
        d.insert("Type", Object::Name("Page".into()));
        d.insert("Count", Object::Integer(1));
        assert_eq!(d.len(), 2);
        // Duplicate key overwrites in place, length unchanged.
        d.insert("Count", Object::Integer(9));
        assert_eq!(d.len(), 2);
        assert_eq!(d.get("Count").unwrap().as_integer(), Some(9));
        assert_eq!(d.get("Type").unwrap().as_name(), Some("Page"));
        assert!(d.get("Missing").is_none());
        let keys: Vec<&str> = d.iter().map(|(k, _)| k).collect();
        assert_eq!(keys, ["Type", "Count"]);
    }

    #[test]
    fn accessors_match_their_variant() {
        assert_eq!(Object::Integer(5).as_integer(), Some(5));
        assert_eq!(Object::Integer(5).as_f64(), Some(5.0));
        assert_eq!(Object::Real(2.5).as_f64(), Some(2.5));
        assert_eq!(Object::Boolean(true).as_bool(), Some(true));
        assert_eq!(Object::Name("X".into()).as_name(), Some("X"));
        assert_eq!(Object::String(b"hi".to_vec()).as_string(), Some(&b"hi"[..]));
        assert_eq!(
            Object::Reference(ObjRef::new(1, 0)).as_reference(),
            Some(ObjRef::new(1, 0))
        );
        assert!(Object::Null.is_null());
    }

    #[test]
    fn array_and_dict_accessors() {
        let arr = Object::Array(vec![Object::Integer(1), Object::Integer(2)]);
        assert_eq!(arr.as_array().unwrap().len(), 2);
        let mut d = Dictionary::new();
        d.insert("K", Object::Integer(1));
        let dict = Object::Dictionary(d.clone());
        assert_eq!(dict.as_dict().unwrap().len(), 1);
        let stream = Object::Stream(Stream {
            dict: d,
            data: vec![1, 2, 3],
        });
        assert_eq!(stream.as_dict().unwrap().len(), 1);
        assert_eq!(stream.as_stream().unwrap().data, vec![1, 2, 3]);
    }

    #[test]
    fn accessors_reject_wrong_variant() {
        let n = Object::Null;
        assert!(n.as_integer().is_none());
        assert!(n.as_f64().is_none());
        assert!(n.as_bool().is_none());
        assert!(n.as_name().is_none());
        assert!(n.as_string().is_none());
        assert!(n.as_array().is_none());
        assert!(n.as_dict().is_none());
        assert!(n.as_stream().is_none());
        assert!(n.as_reference().is_none());
        assert!(!Object::Integer(0).is_null());
    }
}
