//! SPARQL 1.1 parser (subset).
//!
//! Parses SELECT queries with basic graph patterns (BGPs), PREFIX declarations,
//! LIMIT/OFFSET, and full IRI syntax. Hand-rolled recursive descent.

use std::collections::HashMap;

use crate::error::{Result, SparqlError};

/// A parsed SPARQL query.
#[derive(Debug, Clone)]
pub struct Query {
    /// PREFIX declarations.
    pub prefixes: HashMap<String, String>,
    /// Variables to project (empty = SELECT *).
    pub projection: Vec<String>,
    /// Whether this is SELECT DISTINCT.
    pub distinct: bool,
    /// The WHERE clause patterns.
    pub patterns: Vec<Pattern>,
    /// LIMIT clause.
    pub limit: Option<usize>,
    /// OFFSET clause.
    pub offset: Option<usize>,
}

/// A pattern in the WHERE clause.
#[derive(Debug, Clone)]
pub enum Pattern {
    /// A triple pattern: subject, predicate, object.
    Triple {
        subject: Term,
        predicate: Term,
        object: Term,
    },
    /// OPTIONAL { patterns }
    Optional(Vec<Pattern>),
    /// FILTER(expression)
    Filter(FilterExpr),
}

/// A term in a triple pattern.
#[derive(Debug, Clone, PartialEq)]
pub enum Term {
    /// A variable: ?name
    Variable(String),
    /// A full IRI: <http://example.org/foo>
    Iri(String),
    /// A prefixed name: foaf:name
    PrefixedName { prefix: String, local: String },
    /// A string literal: "hello"
    Literal(String),
    /// A typed literal: "42"^^<http://www.w3.org/2001/XMLSchema#integer>
    TypedLiteral { value: String, datatype: String },
    /// An integer literal: 42
    IntegerLiteral(i64),
    /// The special token `a` (shorthand for rdf:type)
    A,
}

/// A filter expression (simplified).
#[derive(Debug, Clone)]
pub enum FilterExpr {
    /// ?var = term
    Equals(Term, Term),
    /// ?var != term
    NotEquals(Term, Term),
    /// ?var < term
    LessThan(Term, Term),
    /// ?var > term
    GreaterThan(Term, Term),
    /// bound(?var)
    Bound(String),
    /// !bound(?var)
    NotBound(String),
}

/// Parse a SPARQL query string into a Query AST.
pub fn parse(input: &str) -> Result<Query> {
    let mut parser = Parser::new(input);
    parser.parse_query()
}

struct Parser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn parse_query(&mut self) -> Result<Query> {
        let mut prefixes = HashMap::new();
        let mut distinct = false;

        self.skip_whitespace();

        // Parse PREFIX declarations
        while self.peek_keyword("PREFIX") {
            self.expect_keyword("PREFIX")?;
            let prefix = self.parse_prefix_name()?;
            let iri = self.parse_iri_ref()?;
            prefixes.insert(prefix, iri);
            self.skip_whitespace();
        }

        // Parse SELECT
        self.expect_keyword("SELECT")?;

        // Check for DISTINCT
        if self.peek_keyword("DISTINCT") {
            self.expect_keyword("DISTINCT")?;
            distinct = true;
        }

        // Parse projection
        let projection = self.parse_projection()?;

        // Parse WHERE
        self.expect_keyword("WHERE")?;
        self.expect_char('{')?;

        let patterns = self.parse_patterns()?;

        self.expect_char('}')?;

        // Parse solution modifiers
        let mut limit = None;
        let mut offset = None;

        self.skip_whitespace();
        while self.pos < self.input.len() {
            if self.peek_keyword("LIMIT") {
                self.expect_keyword("LIMIT")?;
                limit = Some(self.parse_integer()? as usize);
            } else if self.peek_keyword("OFFSET") {
                self.expect_keyword("OFFSET")?;
                offset = Some(self.parse_integer()? as usize);
            } else {
                break;
            }
            self.skip_whitespace();
        }

        Ok(Query {
            prefixes,
            projection,
            distinct,
            patterns,
            limit,
            offset,
        })
    }

    fn parse_projection(&mut self) -> Result<Vec<String>> {
        self.skip_whitespace();
        if self.peek_char() == Some('*') {
            self.pos += 1;
            return Ok(vec![]);
        }

        let mut vars = Vec::new();
        while self.peek_char() == Some('?') {
            vars.push(self.parse_variable_name()?);
            self.skip_whitespace();
        }

        if vars.is_empty() {
            return Err(self.error("expected variable or * in SELECT"));
        }

        Ok(vars)
    }

    fn parse_patterns(&mut self) -> Result<Vec<Pattern>> {
        let mut patterns = Vec::new();

        loop {
            self.skip_whitespace();
            if self.peek_char() == Some('}') {
                break;
            }

            if self.peek_keyword("OPTIONAL") {
                self.expect_keyword("OPTIONAL")?;
                self.expect_char('{')?;
                let inner = self.parse_patterns()?;
                self.expect_char('}')?;
                patterns.push(Pattern::Optional(inner));
                self.skip_whitespace();
                if self.peek_char() == Some('.') {
                    self.pos += 1;
                }
            } else if self.peek_keyword("FILTER") {
                self.expect_keyword("FILTER")?;
                let expr = self.parse_filter()?;
                patterns.push(Pattern::Filter(expr));
                self.skip_whitespace();
                if self.peek_char() == Some('.') {
                    self.pos += 1;
                }
            } else {
                // Triple pattern
                let subject = self.parse_term()?;
                self.skip_whitespace();
                let predicate = self.parse_term()?;
                self.skip_whitespace();
                let object = self.parse_term()?;
                self.skip_whitespace();

                // Consume the period if present
                if self.peek_char() == Some('.') {
                    self.pos += 1;
                }

                patterns.push(Pattern::Triple {
                    subject,
                    predicate,
                    object,
                });
            }
        }

        Ok(patterns)
    }

    fn parse_term(&mut self) -> Result<Term> {
        self.skip_whitespace();
        match self.peek_char() {
            Some('?') => {
                let name = self.parse_variable_name()?;
                Ok(Term::Variable(name))
            }
            Some('<') => {
                let iri = self.parse_iri_ref()?;
                Ok(Term::Iri(iri))
            }
            Some('"') => self.parse_string_literal(),
            Some(c) if c.is_ascii_digit() || c == '-' => {
                let n = self.parse_integer()?;
                Ok(Term::IntegerLiteral(n))
            }
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                // Could be 'a' (rdf:type) or a prefixed name
                let word = self.peek_word();
                if word == "a"
                    && self
                        .input
                        .get(self.pos + 1..self.pos + 2)
                        .is_none_or(|c| c.starts_with(|ch: char| !ch.is_ascii_alphanumeric()))
                {
                    self.pos += 1;
                    Ok(Term::A)
                } else {
                    self.parse_prefixed_name()
                }
            }
            _ => Err(self.error("expected term (variable, IRI, literal, or prefixed name)")),
        }
    }

    fn parse_filter(&mut self) -> Result<FilterExpr> {
        self.expect_char('(')?;
        self.skip_whitespace();

        // Check for bound/!bound
        if self.peek_keyword("bound") {
            self.expect_keyword("bound")?;
            self.expect_char('(')?;
            let var = self.parse_variable_name()?;
            self.expect_char(')')?;
            self.expect_char(')')?;
            return Ok(FilterExpr::Bound(var));
        }

        if self.peek_char() == Some('!') {
            self.pos += 1;
            self.skip_whitespace();
            if self.peek_keyword("bound") {
                self.expect_keyword("bound")?;
                self.expect_char('(')?;
                let var = self.parse_variable_name()?;
                self.expect_char(')')?;
                self.expect_char(')')?;
                return Ok(FilterExpr::NotBound(var));
            }
        }

        let left = self.parse_term()?;
        self.skip_whitespace();

        let op = self.parse_comparison_op()?;
        self.skip_whitespace();

        let right = self.parse_term()?;
        self.skip_whitespace();
        self.expect_char(')')?;

        match op.as_str() {
            "=" => Ok(FilterExpr::Equals(left, right)),
            "!=" => Ok(FilterExpr::NotEquals(left, right)),
            "<" => Ok(FilterExpr::LessThan(left, right)),
            ">" => Ok(FilterExpr::GreaterThan(left, right)),
            _ => Err(self.error(&format!("unknown operator: {}", op))),
        }
    }

    fn parse_comparison_op(&mut self) -> Result<String> {
        match self.peek_char() {
            Some('=') => {
                self.pos += 1;
                Ok("=".to_string())
            }
            Some('!') => {
                self.pos += 1;
                self.expect_char('=')?;
                Ok("!=".to_string())
            }
            Some('<') => {
                self.pos += 1;
                Ok("<".to_string())
            }
            Some('>') => {
                self.pos += 1;
                Ok(">".to_string())
            }
            _ => Err(self.error("expected comparison operator")),
        }
    }

    fn parse_variable_name(&mut self) -> Result<String> {
        self.expect_char('?')?;
        let start = self.pos;
        while self.pos < self.input.len() {
            let ch = self.input.as_bytes()[self.pos] as char;
            if ch.is_ascii_alphanumeric() || ch == '_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == start {
            return Err(self.error("expected variable name after ?"));
        }
        Ok(self.input[start..self.pos].to_string())
    }

    fn parse_iri_ref(&mut self) -> Result<String> {
        self.skip_whitespace();
        self.expect_char('<')?;
        let start = self.pos;
        while self.pos < self.input.len() && self.input.as_bytes()[self.pos] as char != '>' {
            self.pos += 1;
        }
        let iri = self.input[start..self.pos].to_string();
        self.expect_char('>')?;
        Ok(iri)
    }

    fn parse_prefix_name(&mut self) -> Result<String> {
        self.skip_whitespace();
        let start = self.pos;
        while self.pos < self.input.len() {
            let ch = self.input.as_bytes()[self.pos] as char;
            if ch == ':' {
                let name = self.input[start..self.pos].to_string();
                self.pos += 1; // consume ':'
                return Ok(name);
            }
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                self.pos += 1;
            } else {
                break;
            }
        }
        Err(self.error("expected prefix name followed by ':'"))
    }

    fn parse_prefixed_name(&mut self) -> Result<Term> {
        let start = self.pos;
        while self.pos < self.input.len() {
            let ch = self.input.as_bytes()[self.pos] as char;
            if ch == ':' {
                let prefix = self.input[start..self.pos].to_string();
                self.pos += 1;
                let local_start = self.pos;
                while self.pos < self.input.len() {
                    let ch = self.input.as_bytes()[self.pos] as char;
                    if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                let local = self.input[local_start..self.pos].to_string();
                return Ok(Term::PrefixedName { prefix, local });
            }
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                self.pos += 1;
            } else {
                break;
            }
        }
        Err(self.error("expected prefixed name (prefix:local)"))
    }

    fn parse_string_literal(&mut self) -> Result<Term> {
        self.expect_char('"')?;
        let start = self.pos;
        while self.pos < self.input.len() && self.input.as_bytes()[self.pos] as char != '"' {
            if self.input.as_bytes()[self.pos] == b'\\' {
                self.pos += 1; // skip escape
            }
            self.pos += 1;
        }
        let value = self.input[start..self.pos].to_string();
        self.expect_char('"')?;

        // Check for typed literal ^^
        if self.input.get(self.pos..self.pos + 2) == Some("^^") {
            self.pos += 2;
            let datatype = self.parse_iri_ref()?;
            Ok(Term::TypedLiteral { value, datatype })
        } else {
            Ok(Term::Literal(value))
        }
    }

    fn parse_integer(&mut self) -> Result<i64> {
        self.skip_whitespace();
        let start = self.pos;
        if self.peek_char() == Some('-') {
            self.pos += 1;
        }
        while self.pos < self.input.len()
            && (self.input.as_bytes()[self.pos] as char).is_ascii_digit()
        {
            self.pos += 1;
        }
        self.input[start..self.pos]
            .parse::<i64>()
            .map_err(|_| self.error("expected integer"))
    }

    // --- Helpers ---

    fn skip_whitespace(&mut self) {
        while self.pos < self.input.len() {
            let ch = self.input.as_bytes()[self.pos] as char;
            if ch.is_ascii_whitespace() {
                self.pos += 1;
            } else if ch == '#' {
                // Skip comment to end of line
                while self.pos < self.input.len() && self.input.as_bytes()[self.pos] != b'\n' {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    fn peek_char(&self) -> Option<char> {
        self.input.as_bytes().get(self.pos).map(|&b| b as char)
    }

    fn peek_word(&self) -> &str {
        let start = self.pos;
        let mut end = self.pos;
        while end < self.input.len() && (self.input.as_bytes()[end] as char).is_ascii_alphanumeric()
        {
            end += 1;
        }
        &self.input[start..end]
    }

    fn peek_keyword(&mut self, keyword: &str) -> bool {
        self.skip_whitespace();
        let upper = self.input.get(self.pos..self.pos + keyword.len());
        if let Some(s) = upper {
            if s.eq_ignore_ascii_case(keyword) {
                // Make sure it's not part of a longer word
                let next = self
                    .input
                    .as_bytes()
                    .get(self.pos + keyword.len())
                    .map(|&b| b as char);
                return next.is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_');
            }
        }
        false
    }

    fn expect_keyword(&mut self, keyword: &str) -> Result<()> {
        self.skip_whitespace();
        if self.peek_keyword(keyword) {
            self.pos += keyword.len();
            Ok(())
        } else {
            Err(self.error(&format!("expected '{}'", keyword)))
        }
    }

    fn expect_char(&mut self, expected: char) -> Result<()> {
        self.skip_whitespace();
        if self.peek_char() == Some(expected) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.error(&format!(
                "expected '{}', got {:?}",
                expected,
                self.peek_char()
            )))
        }
    }

    fn error(&self, message: &str) -> SparqlError {
        SparqlError::Parse {
            position: self.pos,
            message: message.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_select_star() {
        let q = parse("SELECT * WHERE { ?s ?p ?o }").unwrap();
        assert!(q.projection.is_empty()); // * = empty
        assert_eq!(q.patterns.len(), 1);
    }

    #[test]
    fn parse_select_variables() {
        let q = parse("SELECT ?name ?age WHERE { ?person ?p ?name }").unwrap();
        assert_eq!(q.projection, vec!["name", "age"]);
    }

    #[test]
    fn parse_with_prefix() {
        let q = parse(
            "PREFIX foaf: <http://xmlns.com/foaf/0.1/> \
             SELECT ?name WHERE { ?person foaf:name ?name }",
        )
        .unwrap();
        assert_eq!(q.prefixes["foaf"], "http://xmlns.com/foaf/0.1/");
        if let Pattern::Triple { predicate, .. } = &q.patterns[0] {
            assert_eq!(
                *predicate,
                Term::PrefixedName {
                    prefix: "foaf".to_string(),
                    local: "name".to_string()
                }
            );
        } else {
            panic!("expected triple pattern");
        }
    }

    #[test]
    fn parse_with_iri() {
        let q =
            parse("SELECT ?o WHERE { <http://example.org/Alice> <http://example.org/knows> ?o }")
                .unwrap();
        assert_eq!(q.patterns.len(), 1);
        if let Pattern::Triple { subject, .. } = &q.patterns[0] {
            assert_eq!(*subject, Term::Iri("http://example.org/Alice".to_string()));
        }
    }

    #[test]
    fn parse_a_shorthand() {
        let q = parse("SELECT ?s WHERE { ?s a foaf:Person }").unwrap();
        if let Pattern::Triple { predicate, .. } = &q.patterns[0] {
            assert_eq!(*predicate, Term::A);
        }
    }

    #[test]
    fn parse_multiple_patterns() {
        let q = parse(
            "SELECT ?name WHERE { \
             ?person a foaf:Person . \
             ?person foaf:name ?name \
             }",
        )
        .unwrap();
        assert_eq!(q.patterns.len(), 2);
    }

    #[test]
    fn parse_limit_offset() {
        let q = parse("SELECT * WHERE { ?s ?p ?o } LIMIT 10 OFFSET 5").unwrap();
        assert_eq!(q.limit, Some(10));
        assert_eq!(q.offset, Some(5));
    }

    #[test]
    fn parse_distinct() {
        let q = parse("SELECT DISTINCT ?s WHERE { ?s ?p ?o }").unwrap();
        assert!(q.distinct);
    }

    #[test]
    fn parse_filter() {
        let q = parse("SELECT ?s WHERE { ?s ?p ?o . FILTER(?o = 42) }").unwrap();
        assert_eq!(q.patterns.len(), 2);
        assert!(matches!(q.patterns[1], Pattern::Filter(_)));
    }

    #[test]
    fn parse_optional() {
        let q = parse(
            "SELECT ?s ?name WHERE { \
             ?s ?p ?o . \
             OPTIONAL { ?s foaf:name ?name } \
             }",
        )
        .unwrap();
        assert_eq!(q.patterns.len(), 2);
        assert!(matches!(q.patterns[1], Pattern::Optional(_)));
    }

    #[test]
    fn parse_integer_literal() {
        let q = parse("SELECT ?s WHERE { ?s ex:age 42 }").unwrap();
        if let Pattern::Triple { object, .. } = &q.patterns[0] {
            assert_eq!(*object, Term::IntegerLiteral(42));
        }
    }

    #[test]
    fn parse_string_literal() {
        let q = parse(r#"SELECT ?s WHERE { ?s ex:name "Alice" }"#).unwrap();
        if let Pattern::Triple { object, .. } = &q.patterns[0] {
            assert_eq!(*object, Term::Literal("Alice".to_string()));
        }
    }

    #[test]
    fn parse_error_on_invalid() {
        assert!(parse("INVALID QUERY").is_err());
        assert!(parse("SELECT WHERE { }").is_err());
    }
}
