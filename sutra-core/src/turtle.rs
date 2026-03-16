//! Minimal Turtle (.ttl) parser.
//!
//! Parses a subset of Turtle syntax sufficient for most ontology files:
//! - @prefix declarations
//! - @base declarations
//! - Triple statements with prefixed names
//! - Semicolon (;) for predicate lists
//! - Comma (,) for object lists
//! - Typed literals and language-tagged literals
//! - Blank nodes ([] syntax and _: syntax)
//!
//! This is NOT a full Turtle parser — for complete compliance, use
//! Oxigraph's oxttl crate. This handles the 80% case for OWL ontologies
//! exported from Protégé.

use std::collections::HashMap;

/// Parse Turtle text into a list of (subject, predicate, object) string triples.
pub fn parse_turtle(input: &str) -> Vec<(String, String, String)> {
    let mut parser = TurtleParser::new(input);
    parser.parse()
}

struct TurtleParser<'a> {
    input: &'a str,
    pos: usize,
    prefixes: HashMap<String, String>,
    base: Option<String>,
    blank_counter: usize,
}

impl<'a> TurtleParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            pos: 0,
            prefixes: HashMap::new(),
            base: None,
            blank_counter: 0,
        }
    }

    fn parse(&mut self) -> Vec<(String, String, String)> {
        let mut triples = Vec::new();
        self.skip_ws();

        while self.pos < self.input.len() {
            self.skip_ws();
            if self.pos >= self.input.len() {
                break;
            }

            // @prefix
            if self.starts_with("@prefix") {
                self.pos += 7;
                self.skip_ws();
                let prefix = self.read_until(':');
                self.pos += 1; // skip ':'
                self.skip_ws();
                let iri = self.parse_iri_ref();
                self.skip_ws();
                if self.peek() == Some('.') {
                    self.pos += 1;
                }
                self.prefixes.insert(prefix, iri);
                continue;
            }

            // PREFIX (SPARQL style)
            if self.starts_with("PREFIX") || self.starts_with("prefix") {
                self.pos += 6;
                self.skip_ws();
                let prefix = self.read_until(':');
                self.pos += 1;
                self.skip_ws();
                let iri = self.parse_iri_ref();
                self.prefixes.insert(prefix, iri);
                continue;
            }

            // @base
            if self.starts_with("@base") {
                self.pos += 5;
                self.skip_ws();
                self.base = Some(self.parse_iri_ref());
                self.skip_ws();
                if self.peek() == Some('.') {
                    self.pos += 1;
                }
                continue;
            }

            // Comment
            if self.peek() == Some('#') {
                self.skip_line();
                continue;
            }

            // Triple statement
            if let Some(subject) = self.parse_node() {
                self.skip_ws();
                // Predicate-object list
                loop {
                    self.skip_ws();
                    if self.peek() == Some('.') {
                        self.pos += 1;
                        break;
                    }

                    let predicate = match self.parse_node() {
                        Some(p) => {
                            if p == "a" {
                                "http://www.w3.org/1999/02/22-rdf-syntax-ns#type".to_string()
                            } else {
                                p
                            }
                        }
                        None => break,
                    };

                    // Object list
                    loop {
                        self.skip_ws();
                        if let Some(object) = self.parse_object() {
                            triples.push((subject.clone(), predicate.clone(), object));
                        } else {
                            break;
                        }

                        self.skip_ws();
                        if self.peek() == Some(',') {
                            self.pos += 1; // more objects
                        } else {
                            break;
                        }
                    }

                    self.skip_ws();
                    if self.peek() == Some(';') {
                        self.pos += 1; // more predicates
                    } else {
                        if self.peek() == Some('.') {
                            self.pos += 1;
                        }
                        break;
                    }
                }
            } else {
                // Skip unrecognized content
                self.pos += 1;
            }
        }

        triples
    }

    fn parse_node(&mut self) -> Option<String> {
        self.skip_ws();
        match self.peek()? {
            '<' => Some(self.parse_iri_ref()),
            '_' => Some(self.parse_blank_node()),
            '[' => Some(self.parse_anon_blank()),
            'a' if self.is_keyword_a() => {
                self.pos += 1;
                Some("a".to_string())
            }
            c if c.is_alphabetic() => Some(self.parse_prefixed_name()),
            _ => None,
        }
    }

    fn parse_object(&mut self) -> Option<String> {
        self.skip_ws();
        match self.peek()? {
            '"' => Some(self.parse_literal()),
            '\'' => Some(self.parse_literal()),
            '<' => Some(self.parse_iri_ref()),
            '_' => Some(self.parse_blank_node()),
            '[' => Some(self.parse_anon_blank()),
            c if c.is_ascii_digit() || c == '+' || c == '-' => Some(self.parse_number()),
            c if c.is_alphabetic() => {
                let name = self.parse_prefixed_name();
                if name == "true" || name == "false" {
                    return Some(name);
                }
                Some(name)
            }
            _ => None,
        }
    }

    fn parse_iri_ref(&mut self) -> String {
        if self.peek() != Some('<') {
            return String::new();
        }
        self.pos += 1;
        let start = self.pos;
        while self.pos < self.input.len() && self.input.as_bytes()[self.pos] != b'>' {
            self.pos += 1;
        }
        let iri = self.input[start..self.pos].to_string();
        if self.pos < self.input.len() {
            self.pos += 1;
        }
        iri
    }

    fn parse_prefixed_name(&mut self) -> String {
        let start = self.pos;
        while self.pos < self.input.len() {
            let c = self.input.as_bytes()[self.pos] as char;
            if c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == ':' {
                self.pos += 1;
            } else {
                break;
            }
        }
        let name = &self.input[start..self.pos];
        // Expand prefix
        if let Some(colon_pos) = name.find(':') {
            let prefix = &name[..colon_pos];
            let local = &name[colon_pos + 1..];
            if let Some(base) = self.prefixes.get(prefix) {
                return format!("{}{}", base, local);
            }
        }
        name.to_string()
    }

    fn parse_blank_node(&mut self) -> String {
        let start = self.pos;
        self.pos += 2; // skip _:
        while self.pos < self.input.len() {
            let c = self.input.as_bytes()[self.pos] as char;
            if c.is_alphanumeric() || c == '_' || c == '-' || c == '.' {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.input[start..self.pos].to_string()
    }

    fn parse_anon_blank(&mut self) -> String {
        self.pos += 1; // skip [
        self.skip_ws();
        if self.peek() == Some(']') {
            self.pos += 1;
        }
        self.blank_counter += 1;
        format!("_:anon{}", self.blank_counter)
    }

    fn parse_literal(&mut self) -> String {
        let quote = self.input.as_bytes()[self.pos] as char;
        self.pos += 1;

        // Check for triple-quoted string
        let triple_quoted = self.pos + 1 < self.input.len()
            && self.input.as_bytes()[self.pos] == quote as u8
            && self.input.as_bytes()[self.pos + 1] == quote as u8;
        if triple_quoted {
            self.pos += 2;
        }

        let start = self.pos;
        if triple_quoted {
            while self.pos + 2 < self.input.len() {
                if self.input.as_bytes()[self.pos] == quote as u8
                    && self.input.as_bytes()[self.pos + 1] == quote as u8
                    && self.input.as_bytes()[self.pos + 2] == quote as u8
                {
                    break;
                }
                self.pos += 1;
            }
            let value = &self.input[start..self.pos];
            self.pos += 3; // skip closing """
            return self.finish_literal(value);
        }

        while self.pos < self.input.len() {
            let c = self.input.as_bytes()[self.pos];
            if c == b'\\' {
                self.pos += 2;
                continue;
            }
            if c == quote as u8 {
                break;
            }
            self.pos += 1;
        }
        let value = &self.input[start..self.pos];
        if self.pos < self.input.len() {
            self.pos += 1; // skip closing quote
        }
        self.finish_literal(value)
    }

    fn finish_literal(&mut self, value: &str) -> String {
        // Check for datatype or language tag
        if self.pos + 1 < self.input.len()
            && self.input.as_bytes()[self.pos] == b'^'
            && self.input.as_bytes()[self.pos + 1] == b'^'
        {
            self.pos += 2;
            let dt = if self.peek() == Some('<') {
                self.parse_iri_ref()
            } else {
                self.parse_prefixed_name()
            };
            return format!("\"{}\"^^<{}>", value, dt);
        }
        if self.peek() == Some('@') {
            self.pos += 1;
            let start = self.pos;
            while self.pos < self.input.len() {
                let c = self.input.as_bytes()[self.pos] as char;
                if c.is_alphanumeric() || c == '-' {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            let lang = &self.input[start..self.pos];
            return format!("\"{}\"@{}", value, lang);
        }
        format!("\"{}\"", value)
    }

    fn parse_number(&mut self) -> String {
        let start = self.pos;
        if self.peek() == Some('+') || self.peek() == Some('-') {
            self.pos += 1;
        }
        while self.pos < self.input.len() && self.input.as_bytes()[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        if self.peek() == Some('.') {
            self.pos += 1;
            while self.pos < self.input.len() && self.input.as_bytes()[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
        }
        self.input[start..self.pos].to_string()
    }

    fn is_keyword_a(&self) -> bool {
        if self.pos + 1 >= self.input.len() {
            return false;
        }
        let next = self.input.as_bytes()[self.pos + 1] as char;
        next == ' ' || next == '\t' || next == '\n' || next == '\r'
    }

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn starts_with(&self, s: &str) -> bool {
        self.input[self.pos..].starts_with(s)
    }

    fn skip_ws(&mut self) {
        while self.pos < self.input.len() {
            let c = self.input.as_bytes()[self.pos];
            if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' {
                self.pos += 1;
            } else if c == b'#' {
                self.skip_line();
            } else {
                break;
            }
        }
    }

    fn skip_line(&mut self) {
        while self.pos < self.input.len() && self.input.as_bytes()[self.pos] != b'\n' {
            self.pos += 1;
        }
        if self.pos < self.input.len() {
            self.pos += 1;
        }
    }

    fn read_until(&mut self, ch: char) -> String {
        let start = self.pos;
        while self.pos < self.input.len() && self.input.as_bytes()[self.pos] != ch as u8 {
            self.pos += 1;
        }
        self.input[start..self.pos].trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_turtle() {
        let ttl = r#"
@prefix ex: <http://example.org/> .
ex:Alice ex:knows ex:Bob .
"#;
        let triples = parse_turtle(ttl);
        assert_eq!(triples.len(), 1);
        assert_eq!(triples[0].0, "http://example.org/Alice");
        assert_eq!(triples[0].1, "http://example.org/knows");
        assert_eq!(triples[0].2, "http://example.org/Bob");
    }

    #[test]
    fn parse_rdf_type_shorthand() {
        let ttl = r#"
@prefix ex: <http://example.org/> .
ex:Alice a ex:Person .
"#;
        let triples = parse_turtle(ttl);
        assert_eq!(triples.len(), 1);
        assert_eq!(
            triples[0].1,
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#type"
        );
    }

    #[test]
    fn parse_semicolon_list() {
        let ttl = r#"
@prefix ex: <http://example.org/> .
ex:Alice ex:name "Alice" ;
         ex:age "30" .
"#;
        let triples = parse_turtle(ttl);
        assert_eq!(triples.len(), 2);
    }

    #[test]
    fn parse_comma_list() {
        let ttl = r#"
@prefix ex: <http://example.org/> .
ex:Alice ex:knows ex:Bob , ex:Charlie .
"#;
        let triples = parse_turtle(ttl);
        assert_eq!(triples.len(), 2);
    }

    #[test]
    fn parse_language_tagged_literal() {
        let ttl = r#"
@prefix ex: <http://example.org/> .
ex:Alice ex:name "Alice"@en .
"#;
        let triples = parse_turtle(ttl);
        assert_eq!(triples[0].2, "\"Alice\"@en");
    }

    #[test]
    fn parse_typed_literal() {
        let ttl = r#"
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
ex:Alice ex:age "30"^^xsd:integer .
"#;
        let triples = parse_turtle(ttl);
        assert!(triples[0].2.contains("integer"));
    }
}
