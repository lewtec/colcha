use std::fmt;

use serde::Serialize;

use crate::error::Error;

const MAX_SEGMENTS: usize = 32;
const MAX_SEGMENT_LEN: usize = 64;

/// Canonical configuration path: an ordered list of lower-case segments.
///
/// The root path has no segments. Every other path has 1 to 32 segments.
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

    /// Parse a dotted path. ASCII letters are normalized to lower case.
    pub fn parse(source: &str) -> Result<Path, Error> {
        if source.is_empty() {
            return Err(Error::path(source, "empty path"));
        }
        let segments: Vec<String> = source
            .split('.')
            .map(|segment| segment.to_ascii_lowercase())
            .collect();
        Path::from_segments(segments)
    }

    /// Build a path from already normalized segments.
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

    /// Append one normalized segment.
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
        f.write_str(&self.segments.join("/"))
    }
}

fn validate_segment(segment: &str) -> Result<(), Error> {
    let mut chars = segment.chars();
    let Some(first) = chars.next() else {
        return Err(Error::path(segment, "empty segment"));
    };
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return Err(Error::path(
            segment,
            "a segment starts with a lower-case letter or a digit",
        ));
    }
    if segment.len() > MAX_SEGMENT_LEN {
        return Err(Error::path(
            segment,
            format!("a segment has at most {MAX_SEGMENT_LEN} bytes"),
        ));
    }
    if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
        return Err(Error::path(
            segment,
            "a segment contains only lower-case letters, digits, '_' and '-'",
        ));
    }
    Ok(())
}
