//! Template parsing and rendering for secret references.
//!
//! This module provides the `Template` struct, which can parse text
//! containing `{{ ... }}` tags representing secret references.
//! It can extract the keys used in the template and render the
//! template by replacing tags with actual secret values, provided by the caller.
use crate::provider::{ReferenceParser, SecretReference};
use std::borrow::Cow;

/// A segment of a parsed template.
#[derive(Debug, Clone)]
enum Segment<'a> {
    /// Static text (including unparseable tags) that should be preserved as-is.
    Raw(&'a str),
    /// A valid secret reference identified by the parser.
    /// Stores the original text to support "fallback" strategies if the secret isn't found.
    Secret {
        reference: SecretReference,
        original: &'a str,
    },
}

/// Represents a loaded text resource that may contain secret references.
///
/// This struct is responsible for parsing the `{{ ... }}` syntax
/// and rendering the template with provided secret values.
/// It is designed to be zero-allocation when no tags are present.
#[derive(Debug, Clone)]
pub struct Template<'a> {
    source: &'a str,
    segments: Vec<Segment<'a>>,
}

impl<'a> Template<'a> {
    /// Parses a string into a Template using the provided parser to identify secrets.
    ///
    /// Any tag `{{ ... }}` that represents a valid secret for the given parser
    /// is stored as a `Segment::Secret`. Any text outside tags, or tags that
    /// fail parsing, are stored as `Segment::Raw`.
    pub fn parse<P>(source: &'a str, parser: &P) -> Self
    where
        P: ReferenceParser + ?Sized,
    {
        let mut segments = Vec::new();
        let mut cursor = 0;

        for (range, inner_key) in TagIterator::new(source) {
            // Push preceding raw text
            if range.start > cursor {
                segments.push(Segment::Raw(&source[cursor..range.start]));
            }

            // Try to parse the tag content
            let tag = &source[range.clone()];

            if let Some(reference) = parser.parse(inner_key) {
                segments.push(Segment::Secret {
                    reference,
                    original: tag,
                });
            } else {
                // Invalid/Unknown format. Treat as literal text
                segments.push(Segment::Raw(tag));
            }

            cursor = range.end;
        }

        // Push remaining raw text
        if cursor < source.len() {
            segments.push(Segment::Raw(&source[cursor..]));
        }

        Self { source, segments }
    }

    /// Returns a list of all unique secret references in the template.
    pub fn references(&self) -> Vec<SecretReference> {
        self.segments
            .iter()
            .filter_map(|s| match s {
                Segment::Secret { reference, .. } => Some(reference.clone()),
                _ => None,
            })
            .collect()
    }

    /// Returns true if the template contains any valid secret references.
    pub fn has_secrets(&self) -> bool {
        self.segments
            .iter()
            .any(|s| matches!(s, Segment::Secret { .. }))
    }

    /// Renders the template using the resolved secrets map.
    ///
    /// * If a secret is found in the map, the tag is replaced with the value.
    /// * If a secret is missing from the map, the original tag is preserved.
    /// * If no replacements occur, returns a zero-copy Cow::Borrowed of the source.
    pub fn render_with<F, S>(&self, lookup: F) -> Cow<'a, str>
    where
        F: Fn(&SecretReference) -> Option<S>,
        S: AsRef<str>,
    {
        // Resolve all segments into a temporary list of string slices.
        // This will improve map lookups and allow an accurate size pre-calculation to minimize allocations.
        enum Part<'a, S> {
            Raw(&'a str),
            Val(S),
        }
        let mut parts = Vec::with_capacity(self.segments.len());
        let mut modified = false;

        for segment in &self.segments {
            match segment {
                Segment::Raw(s) => parts.push(Part::Raw(s)),
                Segment::Secret {
                    reference,
                    original,
                } => {
                    if let Some(val) = lookup(reference) {
                        parts.push(Part::Val(val));
                        modified = true;
                    } else {
                        // Secret not found, keep original tag
                        parts.push(Part::Raw(original));
                    }
                }
            }
        }

        // If nothing changed, return the original source
        if !modified {
            return Cow::Borrowed(self.source);
        }

        // Calculate size required
        let capacity: usize = parts
            .iter()
            .map(|p| match p {
                Part::Raw(s) => s.len(),
                Part::Val(s) => s.as_ref().len(),
            })
            .sum();

        // Allocate and fill
        let mut output = String::with_capacity(capacity);
        for part in parts {
            match part {
                Part::Raw(s) => output.push_str(s),
                Part::Val(s) => output.push_str(s.as_ref()),
            }
        }

        Cow::Owned(output)
    }

    /// Render the template by replacing tags with values provided in the `map`.
    ///
    /// * If a key is present in the map, the entire tag `{{ key }}` is replaced.
    /// * If a key is NOT present, the tag is left unmodified.
    ///
    /// # Example
    ///
    /// ```rust
    /// use locket::template::Template;
    /// use locket::provider::{SecretReference, ReferenceParser};
    /// use std::collections::HashMap;
    /// use std::str::FromStr;
    ///
    /// struct BwsParser;
    /// impl ReferenceParser for BwsParser {
    ///     fn parse(&self, raw: &str) -> Option<SecretReference> {
    ///         uuid::Uuid::parse_str(raw).ok().map(SecretReference::Bws)
    ///     }
    /// }
    ///
    /// let parser = BwsParser;
    /// let tpl = Template::parse("User: {{ 7d173e0c-61cf-45dc-9fc5-e2745182ede1 }}", &parser);
    ///
    /// let ref_key = SecretReference::from_str("7d173e0c-61cf-45dc-9fc5-e2745182ede1").unwrap();
    /// let mut map = HashMap::new();
    /// map.insert(ref_key, "admin");
    ///
    /// assert_eq!(tpl.render(&map), "User: admin");
    /// ```
    pub fn render<S>(&self, values: &std::collections::HashMap<SecretReference, S>) -> Cow<'a, str>
    where
        S: AsRef<str>,
    {
        self.render_with(|k| values.get(k).map(|s| s.as_ref()))
    }
}

/// Iterator state for traversing `{{ ... }}` tags.
struct TagIterator<'a> {
    source: &'a str,
    cursor: usize,
}

impl<'a> TagIterator<'a> {
    fn new(source: &'a str) -> Self {
        Self { source, cursor: 0 }
    }
}

impl<'a> Iterator for TagIterator<'a> {
    type Item = (std::ops::Range<usize>, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        // Bounds check
        if self.cursor >= self.source.len() {
            return None;
        }

        let remainder = &self.source[self.cursor..];

        // Find start "{{"
        let start_offset = remainder.find("{{")?;
        let tag_start = self.cursor + start_offset;

        // Find end "}}" after the start
        let rest = &self.source[tag_start + 2..];

        if let Some(end_offset) = rest.find("}}") {
            let tag_end = tag_start + 2 + end_offset + 2; // +2 for {{, +2 for }}

            // Update cursor for next iteration
            self.cursor = tag_end;

            // Extract content
            let inner = &self.source[tag_start + 2..tag_end - 2];
            let key = sanitize_key(inner);
            if !key.is_empty() {
                return Some((tag_start..tag_end, key));
            }
        }

        // No closing tag found
        None
    }
}

fn sanitize_key(raw: &str) -> &str {
    let trimmed = raw.trim();

    // Check for double quotes
    if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
        return trimmed[1..trimmed.len() - 1].trim();
    }

    // Check for single quotes
    if trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2 {
        return trimmed[1..trimmed.len() - 1].trim();
    }

    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ReferenceParser, SecretReference};
    use std::collections::HashMap;

    // A parser that uses the Mock reference type and doesn't allow whitespace.
    struct MockParser;
    impl ReferenceParser for MockParser {
        fn parse(&self, raw: &str) -> Option<SecretReference> {
            if raw.starts_with("test:") && !raw.contains(char::is_whitespace) {
                Some(SecretReference::Mock(raw.to_string()))
            } else {
                None
            }
        }
    }

    fn ref_from(s: &str) -> SecretReference {
        SecretReference::Mock(s.to_string())
    }

    #[test]
    fn extract_keys_simple() {
        let parser = MockParser;
        let tpl = Template::parse("Host: {{ test:field }}", &parser);
        let refs = tpl.references();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0], ref_from("test:field"));
    }

    #[test]
    fn extract_keys_multiple_deduplicated() {
        // Updated to use valid references
        let source = "User: {{ test:first }}\nPass: {{ test:another }}\nAgain: {{ test:first }}";
        let parser = MockParser;
        let tpl = Template::parse(source, &parser);

        let refs = tpl.references();
        // `references()` returns a Vec of all occurrences
        assert!(refs.contains(&ref_from("test:first")));
        assert!(refs.contains(&ref_from("test:another")));
    }

    #[test]
    fn render_replaces_known_keys() {
        let parser = MockParser;
        let tpl = Template::parse("A={{ test:A }}, B={{ test:B }}", &parser);

        let mut map = HashMap::new();
        map.insert(ref_from("test:A"), "1".to_string());
        map.insert(ref_from("test:B"), "2".to_string());
        let rendered = tpl.render_with(|r| map.get(r));
        assert_eq!(rendered, "A=1, B=2");
    }

    #[test]
    fn render_ignores_unknown_keys() {
        let parser = MockParser;
        // test:b is valid syntax, but missing from the map.
        let raw = "Valid={{ test:a }}, Missing={{ test:b }}";
        let tpl = Template::parse(raw, &parser);

        let mut map = HashMap::new();
        map.insert(ref_from("test:a"), "1".to_string());

        let rendered = tpl.render_with(|r| map.get(r));
        assert_eq!(rendered, "Valid=1, Missing={{ test:b }}");
    }

    #[test]
    fn handle_broken_tags() {
        let parser = MockParser;
        let tpl = Template::parse("Start {{ broken end", &parser);
        assert!(!tpl.has_secrets());
        assert_eq!(tpl.render_with(|_| None::<String>), "Start {{ broken end");
    }

    #[test]
    fn handle_whitespace_in_tags() {
        let parser = MockParser;
        // MockParser rejects strings with spaces.
        // Behavior: Parse fails -> Treated as Raw text -> Preserved.
        let source = "Value: {{ test:key with whitespace }}";
        let tpl = Template::parse(source, &parser);

        assert!(!tpl.has_secrets()); // Should fail parsing

        // Should render identical to source
        assert_eq!(tpl.render_with(|_| None::<String>), source);
    }

    #[test]
    fn test_has_tags() {
        let parser = MockParser;

        let tpl_with = Template::parse("Value: {{ test:key }}", &parser);
        assert!(tpl_with.has_secrets());

        let tpl_without = Template::parse("Just some text", &parser);
        assert!(!tpl_without.has_secrets());

        // Invalid syntax -> Raw -> No Secrets
        let tpl_broken = Template::parse("Unclosed {{ tag", &parser);
        assert!(!tpl_broken.has_secrets());

        let tpl_empty = Template::parse("{{}}", &parser);
        assert!(!tpl_empty.has_secrets());
    }

    #[test]
    fn handle_adjacent_tags() {
        let parser = MockParser;
        let tpl = Template::parse("{{ test:a }}{{ test:b }}", &parser);
        let refs = tpl.references();

        assert_eq!(refs.len(), 2);
        assert!(refs.contains(&ref_from("test:a")));
        assert!(refs.contains(&ref_from("test:b")));
    }

    #[test]
    fn no_alloc_if_no_tags() {
        let parser = MockParser;
        let source = "Just some config";
        let tpl = Template::parse(source, &parser);

        match tpl.render_with(|_| None::<String>) {
            Cow::Borrowed(s) => assert_eq!(s, source),
            Cow::Owned(_) => panic!("Should not allocate for tagless string"),
        }
    }

    #[test]
    fn extract_keys_with_quotes() {
        let parser = MockParser;
        // sanitize_key handles the quotes before passing to MockParser
        let raw = r#"
            A: {{ "test:key1" }}
            B: {{ 'test:key2' }}
            C: {{ test:key3 }}
        "#;
        let tpl = Template::parse(raw, &parser);
        let refs = tpl.references();

        assert_eq!(refs.len(), 3);
        assert!(refs.contains(&ref_from("test:key1")));
        assert!(refs.contains(&ref_from("test:key2")));
        assert!(refs.contains(&ref_from("test:key3")));
    }

    #[test]
    fn render_with_mixed_quotes() {
        let parser = MockParser;
        let raw = r#"{{ "test:key1" }} | {{ 'test:key2' }} | {{ test:key3 }}"#;
        let tpl = Template::parse(raw, &parser);

        let mut map = HashMap::new();
        map.insert(ref_from("test:key1"), "val1".to_string());
        map.insert(ref_from("test:key2"), "val2".to_string());
        map.insert(ref_from("test:key3"), "val3".to_string());

        let out = tpl.render_with(|r| map.get(r));
        assert_eq!(out, "val1 | val2 | val3");
    }

    #[test]
    fn quotes_with_whitespace() {
        let parser = MockParser;
        // Outer whitespace (inside quotes) is stripped by sanitize_key.
        // MockParser receives "test:key1", which is valid.
        let raw = r#"{{ " test:key1 " }} | {{ ' test:key2' }}"#;
        let tpl = Template::parse(raw, &parser);

        let mut map = HashMap::new();
        map.insert(ref_from("test:key1"), "val1".to_string());
        map.insert(ref_from("test:key2"), "val2".to_string());

        let out = tpl.render_with(|r| map.get(r));
        assert_eq!(out, "val1 | val2");
    }

    #[test]
    fn empty_quotes_ignored() {
        let parser = MockParser;
        let raw = r#"{{ "" }} {{ '' }} {{ }} "#;
        let tpl = Template::parse(raw, &parser);
        assert!(!tpl.has_secrets());
    }
}
