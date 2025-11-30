use std::borrow::Cow;
use std::collections::HashSet;

/// Represents a loaded text resource that may contain secret references.
///
/// This struct is responsible for parsing the `{{ ... }}` syntax
/// and rendering the template with provided secret values.
#[derive(Debug, Clone, Copy)]
pub struct Template<'a> {
    source: &'a str,
}

impl<'a> Template<'a> {
    /// Create a new Template from a string slice.
    pub fn new(source: &'a str) -> Self {
        Self { source }
    }

    /// Returns true if the template contains any `{{ ... }}` tags.
    pub fn has_tags(&self) -> bool {
        self.iter_tags()
            .any(|(_, key)| !key.trim().is_empty())
    }

    /// Scans the template and returns a unique set of secret reference keys found within tags.
    ///
    /// The keys are returned trimmed of leading/trailing whitespace.
    /// i.e. `{{ op://vault/item/field }}` -> `op://vault/item/field`
    pub fn keys(&self) -> HashSet<&'a str> {
        let mut keys = HashSet::new();
        for (_, inner) in self.iter_tags() {
            keys.insert(inner.trim());
        }
        keys
    }

    /// Render the template by replacing tags with values provided in the `map`.
    ///
    /// * If a key is present in the map, the entire tag `{{ key }}` is replaced by the value.
    /// * If a key is NOT present in the map, the tag is left strictly unmodified
    pub fn render<S>(&self, values: &std::collections::HashMap<String, S>) -> Cow<'a, str>
    where
        S: AsRef<str>,
    {
        let mut output: Option<String> = None;
        let mut last_idx = 0;

        for (range, inner) in self.iter_tags() {
            let key = inner.trim();
            if key.is_empty() { continue; }

            if output.is_none() {
                if let Some(val) = values.get(key) {
                    // Match found. Diverge from the original source.
                    // Initialize the buffer and catch up.
                    let mut s = String::with_capacity(self.source.len());
                    
                    // Push everything from the start up to this tag
                    s.push_str(&self.source[0..range.start]);
                    s.push_str(val.as_ref());
                    
                    last_idx = range.end;
                    output = Some(s);
                }
                // No match found.
                // Ignore this tag and leave it as part of the
                // original string, avoiding allocation.
                continue;
            }

            // output is Some, we must render.
            let out = output.as_mut().unwrap();

            // Append text between the last processed tag and this one
            out.push_str(&self.source[last_idx..range.start]);

            match values.get(key) {
                Some(val) => out.push_str(val.as_ref()),
                None => out.push_str(&self.source[range.clone()]),
            }

            last_idx = range.end;
        }

        match output {
            Some(mut s) => {
                // Append any remaining text after the last tag
                if last_idx < self.source.len() {
                    s.push_str(&self.source[last_idx..]);
                }
                Cow::Owned(s)
            }
            // If output is still None, it means we either found no tags,
            // or found tags that didn't need replacing. Return original.
            None => Cow::Borrowed(self.source),
        }
    }

    /// Internal iterator over tags.
    fn iter_tags(&self) -> impl Iterator<Item = (std::ops::Range<usize>, &'a str)> + '_ {
        TagIterator {
            source: self.source,
            cursor: 0,
        }
    }
}

/// Iterator state for traversing `{{ ... }}` tags.
struct TagIterator<'a> {
    source: &'a str,
    cursor: usize,
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
        // If not found, '?' returns None immediately, ending the iterator.
        let start_offset = remainder.find("{{")?;
        let tag_start = self.cursor + start_offset;

        // Find end "}}" after the start
        // We are guaranteed that tag_start + 2 is valid because "{{" was found.
        let rest = &self.source[tag_start + 2..];
        
        if let Some(end_offset) = rest.find("}}") {
            let tag_end = tag_start + 2 + end_offset + 2; // +2 for {{, +2 for }}

            // Update cursor for next iteration (maintain state)
            self.cursor = tag_end;

            // Extract content
            let inner = &self.source[tag_start + 2..tag_end - 2];
            return Some((tag_start..tag_end, inner));
        }
        
        // No closing tag found
        // Treat as end of valid stream. The 'Template::render' logic will 
        // handle the leftover text (including the unclosed "{{") as raw string.
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn extract_keys_simple() {
        let tpl = Template::new("Host: {{ op://db/host }}");
        let keys = tpl.keys();
        assert_eq!(keys.len(), 1);
        assert!(keys.contains("op://db/host"));
    }

    #[test]
    fn extract_keys_multiple_deduplicated() {
        let tpl = Template::new("User: {{ user }}\nPass: {{ pass }}\nAgain: {{ user }}");
        let keys = tpl.keys();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains("user"));
        assert!(keys.contains("pass"));
    }

    #[test]
    fn render_replaces_known_keys() {
        let tpl = Template::new("A={{a}}, B={{ b }}");
        let mut map = HashMap::new();
        map.insert("a".to_string(), "1");
        map.insert("b".to_string(), "2");

        let rendered = tpl.render(&map);
        assert_eq!(rendered, "A=1, B=2");
    }

    #[test]
    fn render_ignores_unknown_keys() {
        let raw = "Valid={{ a }}, Missing={{ b }}";
        let tpl = Template::new(raw);
        let mut map = HashMap::new();
        map.insert("a".to_string(), "1");

        let rendered = tpl.render(&map);
        assert_eq!(rendered, "Valid=1, Missing={{ b }}");
    }

    #[test]
    fn handle_broken_tags() {
        // Should ignore unclosed tags
        let tpl = Template::new("Start {{ broken end");
        assert!(tpl.keys().is_empty());
        assert_eq!(tpl.render(&HashMap::<String, String>::new()), "Start {{ broken end");
    }

    #[test]
    fn handle_whitespace_in_tags() {
        let tpl = Template::new("Value: {{ op://vault/item/field with whitespace }}");
        let keys = tpl.keys();
        assert_eq!(keys.len(), 1);
        assert!(keys.contains("op://vault/item/field with whitespace"));
    }

    #[test]
    fn test_has_tags() {
        let tpl_with = Template::new("Value: {{ key }}");
        assert!(tpl_with.has_tags());

        let tpl_without = Template::new("Just some text");
        assert!(!tpl_without.has_tags());

        let tpl_broken = Template::new("Unclosed {{ tag");
        assert!(!tpl_broken.has_tags());

        let tpl_empty = Template::new("{{}}");
        assert!(!tpl_empty.has_tags());

        let tpl_wrong = Template::new("}} key {{");
        assert!(!tpl_wrong.has_tags());
    }

    #[test]
    fn handle_adjacent_tags() {
        let tpl = Template::new("{{a}}{{b}}");
        let keys = tpl.keys();
        assert!(keys.contains("a"));
        assert!(keys.contains("b"));
    }

    #[test]
    fn no_alloc_if_no_tags() {
        let tpl = Template::new("Just some config");
        let map: HashMap<String, String> = HashMap::new();

        // Cow should be Borrowed
        match tpl.render(&map) {
            Cow::Borrowed(s) => assert_eq!(s, "Just some config"),
            Cow::Owned(_) => panic!("Should not allocate for tagless string"),
        }
    }
}