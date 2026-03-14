//! Minimal N-Triples line parser.
//!
//! Parses a single line of N-Triples format and returns the subject, predicate,
//! and object as raw strings. This does not intern terms — the caller is
//! responsible for interning via `TermDictionary`.

/// Parse a single N-Triples line into (subject, predicate, object) strings.
///
/// Returns `None` for blank lines, comment lines, and malformed lines.
///
/// Supported forms:
/// - IRI references: `<http://...>`
/// - Plain string literals: `"value"`
/// - Typed literals: `"value"^^<datatype>`
/// - Language-tagged literals: `"value"@en`
pub fn parse_ntriples_line(line: &str) -> Option<(String, String, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }

    let mut pos = 0;
    let bytes = line.as_bytes();

    // Parse subject (must be an IRI)
    let subject = parse_iri(bytes, &mut pos)?;
    skip_whitespace(bytes, &mut pos);

    // Parse predicate (must be an IRI)
    let predicate = parse_iri(bytes, &mut pos)?;
    skip_whitespace(bytes, &mut pos);

    // Parse object (IRI or literal)
    let object = if pos < bytes.len() && bytes[pos] == b'<' {
        parse_iri(bytes, &mut pos)?
    } else if pos < bytes.len() && bytes[pos] == b'"' {
        parse_literal(bytes, &mut pos)?
    } else {
        return None;
    };

    // Skip optional whitespace and trailing '.'
    skip_whitespace(bytes, &mut pos);
    if pos < bytes.len() && bytes[pos] == b'.' {
        // valid terminator
    }

    Some((subject, predicate, object))
}

/// Parse an IRI enclosed in angle brackets. Advances `pos` past the closing `>`.
fn parse_iri(bytes: &[u8], pos: &mut usize) -> Option<String> {
    if *pos >= bytes.len() || bytes[*pos] != b'<' {
        return None;
    }
    *pos += 1; // skip '<'
    let start = *pos;
    while *pos < bytes.len() && bytes[*pos] != b'>' {
        *pos += 1;
    }
    if *pos >= bytes.len() {
        return None;
    }
    let iri = std::str::from_utf8(&bytes[start..*pos]).ok()?;
    *pos += 1; // skip '>'
    Some(iri.to_string())
}

/// Parse a literal value starting with `"`. Handles typed and language-tagged literals.
///
/// Returns the full literal representation:
/// - Plain: `"value"`
/// - Typed: `"value"^^<datatype>` (returns the raw string including datatype IRI)
/// - Language-tagged: `"value"@lang`
fn parse_literal(bytes: &[u8], pos: &mut usize) -> Option<String> {
    if *pos >= bytes.len() || bytes[*pos] != b'"' {
        return None;
    }
    *pos += 1; // skip opening '"'
    let start = *pos;

    // Find closing quote, handling escape sequences
    while *pos < bytes.len() {
        if bytes[*pos] == b'\\' {
            *pos += 2; // skip escaped character
            continue;
        }
        if bytes[*pos] == b'"' {
            break;
        }
        *pos += 1;
    }
    if *pos >= bytes.len() {
        return None;
    }

    let value = std::str::from_utf8(&bytes[start..*pos]).ok()?;
    *pos += 1; // skip closing '"'

    // Check for datatype or language tag
    if *pos < bytes.len() && bytes[*pos] == b'^' {
        // Typed literal: ^^<datatype>
        if *pos + 1 < bytes.len() && bytes[*pos + 1] == b'^' {
            *pos += 2; // skip '^^'
            let datatype = parse_iri(bytes, pos)?;
            return Some(format!("\"{}\"^^<{}>", value, datatype));
        }
    } else if *pos < bytes.len() && bytes[*pos] == b'@' {
        // Language-tagged literal: @lang
        *pos += 1; // skip '@'
        let lang_start = *pos;
        while *pos < bytes.len()
            && bytes[*pos] != b' '
            && bytes[*pos] != b'\t'
            && bytes[*pos] != b'.'
        {
            *pos += 1;
        }
        let lang = std::str::from_utf8(&bytes[lang_start..*pos]).ok()?;
        return Some(format!("\"{}\"@{}", value, lang));
    }

    // Plain literal
    Some(format!("\"{}\"", value))
}

fn skip_whitespace(bytes: &[u8], pos: &mut usize) {
    while *pos < bytes.len() && (bytes[*pos] == b' ' || bytes[*pos] == b'\t') {
        *pos += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_triple() {
        let line = r#"<http://example.org/s> <http://example.org/p> <http://example.org/o> ."#;
        let result = parse_ntriples_line(line).unwrap();
        assert_eq!(result.0, "http://example.org/s");
        assert_eq!(result.1, "http://example.org/p");
        assert_eq!(result.2, "http://example.org/o");
    }

    #[test]
    fn parse_string_literal() {
        let line = r#"<http://example.org/s> <http://example.org/p> "hello world" ."#;
        let result = parse_ntriples_line(line).unwrap();
        assert_eq!(result.2, "\"hello world\"");
    }

    #[test]
    fn parse_typed_literal() {
        let line = r#"<http://example.org/s> <http://example.org/p> "42"^^<http://www.w3.org/2001/XMLSchema#integer> ."#;
        let result = parse_ntriples_line(line).unwrap();
        assert_eq!(
            result.2,
            "\"42\"^^<http://www.w3.org/2001/XMLSchema#integer>"
        );
    }

    #[test]
    fn parse_language_tagged_literal() {
        let line = r#"<http://example.org/s> <http://example.org/p> "hello"@en ."#;
        let result = parse_ntriples_line(line).unwrap();
        assert_eq!(result.2, "\"hello\"@en");
    }

    #[test]
    fn skip_blank_line() {
        assert!(parse_ntriples_line("").is_none());
        assert!(parse_ntriples_line("   ").is_none());
    }

    #[test]
    fn skip_comment() {
        assert!(parse_ntriples_line("# this is a comment").is_none());
    }

    #[test]
    fn skip_malformed() {
        assert!(parse_ntriples_line("not a triple").is_none());
        assert!(parse_ntriples_line("<incomplete").is_none());
    }

    #[test]
    fn parse_escaped_literal() {
        let line = r#"<http://example.org/s> <http://example.org/p> "say \"hello\"" ."#;
        let result = parse_ntriples_line(line).unwrap();
        assert_eq!(result.2, r#""say \"hello\"""#);
    }

    #[test]
    fn parse_no_trailing_dot() {
        // Some serializers omit the trailing dot; we should still parse
        let line = r#"<http://example.org/s> <http://example.org/p> <http://example.org/o>"#;
        let result = parse_ntriples_line(line).unwrap();
        assert_eq!(result.0, "http://example.org/s");
    }

    #[test]
    fn parse_integer_literal_value() {
        let line = r#"<http://example.org/s> <http://example.org/p> "100"^^<http://www.w3.org/2001/XMLSchema#integer> ."#;
        let result = parse_ntriples_line(line).unwrap();
        // Verify the typed literal is correctly parsed
        assert!(result.2.contains("XMLSchema#integer"));
    }
}
