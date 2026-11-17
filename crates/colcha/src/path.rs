use std::fmt;
use std::fmt::Write as _;

use serde::Serialize;

use crate::error::Error;

const MAX_SEGMENTS: usize = 32;

/// Canonical configuration path: an ordered list of case-sensitive segments.
///
/// The root path has no segments. Every other path has 1 to 32 segments.
/// A segment is any non-empty string. `Display` quotes segments that cannot be
/// written as bare words.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Path {
    segments: Vec<String>,
}

impl Path {
    /// The empty path at the root of the schema.
    pub fn root() -> Path {
        Path {
            segments: Vec::new(),
        }
    }

    /// Parse a dotted path. Quoted segments may contain dots and escapes.
    pub fn parse(source: &str) -> Result<Path, Error> {
        if source.is_empty() {
            return Err(Error::path(source, "empty path"));
        }
        let mut segments = Vec::new();
        let mut pos = 0;
        loop {
            let segment = if source[pos..].starts_with('"') {
                parse_quoted(source, &mut pos)?
            } else {
                let start = pos;
                while let Some(ch) = source[pos..].chars().next() {
                    if ch == '.' {
                        break;
                    }
                    pos += ch.len_utf8();
                }
                if start == pos {
                    return Err(Error::path(source, "empty segment"));
                }
                source[start..pos].to_string()
            };
            segments.push(segment);
            if pos == source.len() {
                break;
            }
            if !source[pos..].starts_with('.') {
                return Err(Error::path(source, "expected '.' between segments"));
            }
            pos += 1;
            if pos == source.len() {
                return Err(Error::path(source, "empty segment"));
            }
        }
        Path::from_segments(segments)
    }

    /// Build a path from segments. Segments stay case-sensitive.
    pub fn from_segments(segments: Vec<String>) -> Result<Path, Error> {
        if segments.len() > MAX_SEGMENTS {
            return Err(Error::path(
                segments.join("."),
                format!("more than {MAX_SEGMENTS} segments"),
            ));
        }
        for segment in &segments {
            validate_segment(segment)?;
        }
        Ok(Path { segments })
    }

    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    pub fn is_root(&self) -> bool {
        self.segments.is_empty()
    }

    pub fn len(&self) -> usize {
        self.segments.len()
    }

    pub fn last(&self) -> Option<&str> {
        self.segments.last().map(String::as_str)
    }

    /// Append one segment.
    pub fn child(&self, segment: &str) -> Result<Path, Error> {
        validate_segment(segment)?;
        if self.segments.len() >= MAX_SEGMENTS {
            return Err(Error::path(
                self.to_string(),
                format!("more than {MAX_SEGMENTS} segments"),
            ));
        }
        let mut segments = self.segments.clone();
        segments.push(segment.to_string());
        Ok(Path { segments })
    }

    /// True when `self` is `other` or an ancestor of `other`.
    pub fn is_prefix_of(&self, other: &Path) -> bool {
        self.segments.len() <= other.segments.len()
            && self
                .segments
                .iter()
                .zip(other.segments.iter())
                .all(|(a, b)| a == b)
    }

    pub fn strip_prefix(&self, prefix: &Path) -> Option<Path> {
        if !prefix.is_prefix_of(self) {
            return None;
        }
        Some(Path {
            segments: self.segments[prefix.segments.len()..].to_vec(),
        })
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.segments.is_empty() {
            return f.write_str("<root>");
        }
        for (index, segment) in self.segments.iter().enumerate() {
            if index > 0 {
                f.write_str(".")?;
            }
            if is_bare_segment(segment) {
                f.write_str(segment)?;
            } else {
                write_quoted(f, segment)?;
            }
        }
        Ok(())
    }
}

fn validate_segment(segment: &str) -> Result<(), Error> {
    if segment.is_empty() {
        return Err(Error::path(segment, "empty segment"));
    }
    Ok(())
}

fn is_bare_segment(segment: &str) -> bool {
    let mut chars = segment.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphanumeric() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn write_quoted(f: &mut fmt::Formatter<'_>, segment: &str) -> fmt::Result {
    f.write_char('"')?;
    for ch in segment.chars() {
        match ch {
            '"' => f.write_str("\\\"")?,
            '\\' => f.write_str("\\\\")?,
            '\n' => f.write_str("\\n")?,
            '\r' => f.write_str("\\r")?,
            '\t' => f.write_str("\\t")?,
            '\u{0008}' => f.write_str("\\b")?,
            '\u{000c}' => f.write_str("\\f")?,
            ch if ch.is_control() => {
                let code = ch as u32;
                if code <= 0xFFFF {
                    write!(f, "\\u{code:04X}")?;
                } else {
                    write!(f, "\\u{{{code:X}}}")?;
                }
            }
            ch => f.write_char(ch)?,
        }
    }
    f.write_char('"')
}

fn parse_quoted(source: &str, pos: &mut usize) -> Result<String, Error> {
    *pos += 1;
    let mut output = String::new();
    loop {
        let Some(ch) = source[*pos..].chars().next() else {
            return Err(Error::path(source, "unterminated string"));
        };
        *pos += ch.len_utf8();
        match ch {
            '"' => return Ok(output),
            '\\' => {
                let Some(escape) = source[*pos..].chars().next() else {
                    return Err(Error::path(source, "unterminated escape"));
                };
                *pos += escape.len_utf8();
                match escape {
                    '"' => output.push('"'),
                    '\\' => output.push('\\'),
                    '/' => output.push('/'),
                    'b' => output.push('\u{0008}'),
                    'f' => output.push('\u{000c}'),
                    'n' => output.push('\n'),
                    'r' => output.push('\r'),
                    't' => output.push('\t'),
                    'u' => output.push(parse_unicode_escape(source, pos)?),
                    other => {
                        return Err(Error::path(source, format!("invalid escape '\\{other}'")));
                    }
                }
            }
            '\n' => return Err(Error::path(source, "newline inside a string")),
            other => output.push(other),
        }
    }
}

fn parse_unicode_escape(source: &str, pos: &mut usize) -> Result<char, Error> {
    let (digits, width) = if source[*pos..].starts_with('{') {
        *pos += 1;
        let start = *pos;
        while let Some(ch) = source[*pos..].chars().next() {
            if !ch.is_ascii_hexdigit() {
                break;
            }
            *pos += ch.len_utf8();
        }
        let digits = &source[start..*pos];
        if digits.is_empty() || !source[*pos..].starts_with('}') {
            return Err(Error::path(source, "invalid unicode escape"));
        }
        *pos += 1;
        (digits, 6)
    } else {
        let start = *pos;
        for _ in 0..4 {
            let Some(ch) = source[*pos..].chars().next() else {
                return Err(Error::path(source, "short unicode escape"));
            };
            if !ch.is_ascii_hexdigit() {
                return Err(Error::path(source, "invalid unicode escape"));
            }
            *pos += ch.len_utf8();
        }
        (&source[start..*pos], 4)
    };
    if digits.len() > width {
        return Err(Error::path(source, "invalid unicode escape"));
    }
    let code = u32::from_str_radix(digits, 16)
        .map_err(|_| Error::path(source, "invalid unicode escape"))?;
    char::from_u32(code).ok_or_else(|| Error::path(source, "invalid unicode escape"))
}
