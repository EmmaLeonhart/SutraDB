//! SPARQL 1.1 parser (subset).
//!
//! Parses SELECT queries with basic graph patterns (BGPs), PREFIX declarations,
//! LIMIT/OFFSET, and full IRI syntax. Hand-rolled recursive descent.

use std::collections::HashMap;

use crate::error::{Result, SparqlError};

/// The type of SPARQL query.
#[derive(Debug, Clone, PartialEq)]
pub enum QueryType {
    Select,
    Ask,
    InsertData,
    DeleteData,
    Construct,
    Describe,
}

/// An aggregate expression in the projection.
#[derive(Debug, Clone)]
pub struct Aggregate {
    /// The aggregate function: COUNT, SUM, AVG, MIN, MAX.
    pub function: AggregateFunction,
    /// The variable or * being aggregated.
    pub argument: AggregateArg,
    /// The alias: (COUNT(*) AS ?count) → "count".
    pub alias: String,
    /// Whether DISTINCT is used inside the aggregate.
    pub distinct: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AggregateFunction {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

#[derive(Debug, Clone)]
pub enum AggregateArg {
    Variable(String),
    Star,
}

/// A parsed SPARQL query.
#[derive(Debug, Clone)]
pub struct Query {
    /// PREFIX declarations.
    pub prefixes: HashMap<String, String>,
    /// Query type (SELECT or ASK).
    pub query_type: QueryType,
    /// Variables to project (empty = SELECT *).
    pub projection: Vec<String>,
    /// Aggregate expressions in the projection.
    pub aggregates: Vec<Aggregate>,
    /// Whether this is SELECT DISTINCT.
    pub distinct: bool,
    /// The WHERE clause patterns.
    pub patterns: Vec<Pattern>,
    /// GROUP BY variables.
    pub group_by: Vec<String>,
    /// HAVING filter (applied after GROUP BY).
    pub having: Option<FilterExpr>,
    /// CONSTRUCT template patterns (only for CONSTRUCT queries).
    pub construct_template: Vec<Pattern>,
    /// ORDER BY clauses.
    pub order_by: Vec<OrderClause>,
    /// LIMIT clause.
    pub limit: Option<usize>,
    /// OFFSET clause.
    pub offset: Option<usize>,
}

/// An ORDER BY clause entry.
#[derive(Debug, Clone)]
pub struct OrderClause {
    pub variable: String,
    pub descending: bool,
    pub vector_score: Option<VectorScoreExpr>,
}

/// A VECTOR_SCORE expression used in ORDER BY.
#[derive(Debug, Clone)]
pub struct VectorScoreExpr {
    pub subject: Term,
    pub predicate: Term,
    pub query_vector: Vec<f32>,
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
    /// VECTOR_SIMILAR(?var predicate "vector"^^sutra:f32vec, threshold)
    /// Optional hints: ef:=N, k:=N
    VectorSimilar {
        subject: Term,
        predicate: Term,
        query_vector: Vec<f32>,
        threshold: f32,
        ef_search: Option<usize>,
        top_k: Option<usize>,
    },
    /// UNION { ... } { ... }
    Union(Vec<Vec<Pattern>>),
    /// BIND(expr AS ?var)
    Bind { expression: Term, variable: String },
    /// VALUES ?var { val1 val2 ... }
    Values { variable: String, values: Vec<Term> },
    /// Subquery: { SELECT ... WHERE { ... } }
    Subquery(Box<Query>),
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
    /// A vector literal: "0.1 0.2 0.3"^^sutra:f32vec
    VectorLiteral(Vec<f32>),
    /// The special token `a` (shorthand for rdf:type)
    A,
    /// An RDF-star quoted triple: << ?s ?p ?o >>
    QuotedTriple {
        subject: Box<Term>,
        predicate: Box<Term>,
        object: Box<Term>,
    },
    /// A property path: predicate+ (one or more), predicate* (zero or more),
    /// or pred1/pred2 (sequence).
    Path {
        base: Box<Term>,
        modifier: PathModifier,
    },
}

/// Property path modifier.
#[derive(Debug, Clone, PartialEq)]
pub enum PathModifier {
    /// + (one or more)
    OneOrMore,
    /// * (zero or more)
    ZeroOrMore,
    /// ? (zero or one)
    ZeroOrOne,
    /// / (sequence of two predicates)
    Sequence(Box<Term>),
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
    /// NOT EXISTS { patterns }
    NotExists(Vec<Pattern>),
    /// EXISTS { patterns }
    Exists(Vec<Pattern>),
    /// expr && expr
    And(Box<FilterExpr>, Box<FilterExpr>),
    /// expr || expr
    Or(Box<FilterExpr>, Box<FilterExpr>),
    /// !expr
    Not(Box<FilterExpr>),
    /// CONTAINS(?var, "text")
    Contains(Term, Term),
    /// STRSTARTS(?var, "text")
    StrStarts(Term, Term),
    /// STRENDS(?var, "text")
    StrEnds(Term, Term),
    /// REGEX(?var, "pattern")
    Regex(Term, Term),
    /// LANG(?var) = "en"
    LangEquals(String, String),
    /// isIRI(?var) / isURI(?var)
    IsIri(String),
    /// isLiteral(?var)
    IsLiteral(String),
    /// LANG(?var) = "en" or LANGMATCHES(LANG(?var), "en")
    LangMatches(String, String),
    /// STR(?var) — cast to string for comparison
    StrEquals(String, Term),
    /// DATATYPE(?var) = <xsd:integer> etc.
    DatatypeEquals(String, String),
    /// ?var >= term
    GreaterThanOrEqual(Term, Term),
    /// ?var <= term
    LessThanOrEqual(Term, Term),
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

        // Determine query type
        let query_type = if self.peek_keyword("ASK") {
            self.expect_keyword("ASK")?;
            QueryType::Ask
        } else if self.peek_keyword("INSERT") {
            self.expect_keyword("INSERT")?;
            self.skip_whitespace();
            self.expect_keyword("DATA")?;
            QueryType::InsertData
        } else if self.peek_keyword("DELETE") {
            self.expect_keyword("DELETE")?;
            self.skip_whitespace();
            self.expect_keyword("DATA")?;
            QueryType::DeleteData
        } else if self.peek_keyword("CONSTRUCT") {
            self.expect_keyword("CONSTRUCT")?;
            QueryType::Construct
        } else if self.peek_keyword("DESCRIBE") {
            self.expect_keyword("DESCRIBE")?;
            QueryType::Describe
        } else {
            self.expect_keyword("SELECT")?;
            QueryType::Select
        };

        let mut projection = Vec::new();
        let mut aggregates = Vec::new();

        if query_type == QueryType::Select {
            // Check for DISTINCT
            if self.peek_keyword("DISTINCT") {
                self.expect_keyword("DISTINCT")?;
                distinct = true;
            }

            // Parse projection (may include aggregates)
            let (proj, aggs) = self.parse_projection_with_aggregates()?;
            projection = proj;
            aggregates = aggs;
        }

        // CONSTRUCT: parse template, then WHERE
        let mut construct_template = Vec::new();
        if query_type == QueryType::Construct {
            self.skip_whitespace();
            self.expect_char('{')?;
            construct_template = self.parse_patterns()?;
            self.expect_char('}')?;
        }

        // DESCRIBE: parse the resource term as a single-variable projection
        if query_type == QueryType::Describe {
            self.skip_whitespace();
            if self.peek_char() == Some('?') {
                projection = vec![self.parse_variable_name()?];
            } else {
                let term = self.parse_term()?;
                if let Term::Iri(iri) = &term {
                    projection = vec![iri.clone()];
                }
            }
        }

        // For INSERT DATA / DELETE DATA, go straight to the { } block
        if query_type == QueryType::InsertData || query_type == QueryType::DeleteData {
            self.skip_whitespace();
            self.expect_char('{')?;
            let patterns = self.parse_patterns()?;
            self.expect_char('}')?;

            return Ok(Query {
                prefixes,
                query_type,
                projection: vec![],
                aggregates: vec![],
                distinct: false,
                patterns,
                group_by: vec![],
                having: None,
                construct_template: vec![],
                order_by: vec![],
                limit: None,
                offset: None,
            });
        }

        // Parse WHERE (optional keyword for ASK)
        self.skip_whitespace();
        if self.peek_keyword("WHERE") {
            self.expect_keyword("WHERE")?;
        }
        self.expect_char('{')?;

        let patterns = self.parse_patterns()?;

        self.expect_char('}')?;

        // Parse solution modifiers
        let mut group_by = Vec::new();
        let mut order_by = Vec::new();
        let mut limit = None;
        let mut offset = None;

        self.skip_whitespace();
        if self.peek_keyword("GROUP") {
            self.expect_keyword("GROUP")?;
            self.expect_keyword("BY")?;
            self.skip_whitespace();
            while self.peek_char() == Some('?') {
                group_by.push(self.parse_variable_name()?);
                self.skip_whitespace();
            }
        }

        let mut having = None;
        self.skip_whitespace();
        if self.peek_keyword("HAVING") {
            self.expect_keyword("HAVING")?;
            self.skip_whitespace();
            having = Some(self.parse_filter()?);
        }

        self.skip_whitespace();
        if self.peek_keyword("ORDER") {
            self.expect_keyword("ORDER")?;
            self.expect_keyword("BY")?;
            order_by = self.parse_order_by()?;
        }

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
            query_type,
            projection,
            aggregates,
            distinct,
            patterns,
            group_by,
            having,
            construct_template,
            order_by,
            limit,
            offset,
        })
    }

    fn parse_projection_with_aggregates(&mut self) -> Result<(Vec<String>, Vec<Aggregate>)> {
        self.skip_whitespace();
        if self.peek_char() == Some('*') {
            self.pos += 1;
            return Ok((vec![], vec![]));
        }

        let mut vars = Vec::new();
        let mut aggregates = Vec::new();

        loop {
            self.skip_whitespace();
            if self.peek_char() == Some('?') {
                vars.push(self.parse_variable_name()?);
            } else if self.peek_char() == Some('(') {
                // Could be an aggregate: (COUNT(*) AS ?count)
                let saved_pos = self.pos;
                if let Ok(agg) = self.parse_aggregate_projection() {
                    vars.push(agg.alias.clone());
                    aggregates.push(agg);
                } else {
                    self.pos = saved_pos;
                    break;
                }
            } else {
                break;
            }
        }

        if vars.is_empty() && aggregates.is_empty() {
            return Err(self.error("expected variable, aggregate, or * in SELECT"));
        }

        Ok((vars, aggregates))
    }

    fn parse_aggregate_projection(&mut self) -> Result<Aggregate> {
        self.expect_char('(')?;
        self.skip_whitespace();

        // Parse function name
        let func = if self.peek_keyword("COUNT") {
            self.expect_keyword("COUNT")?;
            AggregateFunction::Count
        } else if self.peek_keyword("SUM") {
            self.expect_keyword("SUM")?;
            AggregateFunction::Sum
        } else if self.peek_keyword("AVG") {
            self.expect_keyword("AVG")?;
            AggregateFunction::Avg
        } else if self.peek_keyword("MIN") {
            self.expect_keyword("MIN")?;
            AggregateFunction::Min
        } else if self.peek_keyword("MAX") {
            self.expect_keyword("MAX")?;
            AggregateFunction::Max
        } else {
            return Err(self.error("expected aggregate function"));
        };

        self.expect_char('(')?;
        self.skip_whitespace();

        let mut agg_distinct = false;
        if self.peek_keyword("DISTINCT") {
            self.expect_keyword("DISTINCT")?;
            agg_distinct = true;
            self.skip_whitespace();
        }

        let arg = if self.peek_char() == Some('*') {
            self.pos += 1;
            AggregateArg::Star
        } else if self.peek_char() == Some('?') {
            AggregateArg::Variable(self.parse_variable_name()?)
        } else {
            return Err(self.error("expected * or variable in aggregate"));
        };

        self.skip_whitespace();
        self.expect_char(')')?; // close inner parens
        self.skip_whitespace();

        // AS ?alias
        self.expect_keyword("AS")?;
        self.skip_whitespace();
        let alias = self.parse_variable_name()?;
        self.skip_whitespace();
        self.expect_char(')')?; // close outer parens

        Ok(Aggregate {
            function: func,
            argument: arg,
            alias,
            distinct: agg_distinct,
        })
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
            } else if self.peek_keyword("VECTOR_SIMILAR") {
                let vs = self.parse_vector_similar()?;
                patterns.push(vs);
                self.skip_whitespace();
                if self.peek_char() == Some('.') {
                    self.pos += 1;
                }
            } else if self.peek_keyword("BIND") {
                self.expect_keyword("BIND")?;
                self.expect_char('(')?;
                self.skip_whitespace();
                let expr = self.parse_term()?;
                self.skip_whitespace();
                self.expect_keyword("AS")?;
                self.skip_whitespace();
                let var = self.parse_variable_name()?;
                self.skip_whitespace();
                self.expect_char(')')?;
                patterns.push(Pattern::Bind {
                    expression: expr,
                    variable: var,
                });
                self.skip_whitespace();
                if self.peek_char() == Some('.') {
                    self.pos += 1;
                }
            } else if self.peek_keyword("VALUES") {
                self.expect_keyword("VALUES")?;
                self.skip_whitespace();
                let var = self.parse_variable_name()?;
                self.skip_whitespace();
                self.expect_char('{')?;
                self.skip_whitespace();
                let mut values = Vec::new();
                while self.peek_char() != Some('}') && self.pos < self.input.len() {
                    values.push(self.parse_term()?);
                    self.skip_whitespace();
                }
                self.expect_char('}')?;
                patterns.push(Pattern::Values {
                    variable: var,
                    values,
                });
                self.skip_whitespace();
                if self.peek_char() == Some('.') {
                    self.pos += 1;
                }
            } else if self.peek_char() == Some('{') {
                // Check if this is a subquery: { SELECT ... }
                let saved = self.pos;
                self.pos += 1; // skip '{'
                self.skip_whitespace();
                if self.peek_keyword("SELECT") {
                    // Parse the inner SELECT as a full query
                    let inner_query = self.parse_query()?;
                    self.skip_whitespace();
                    self.expect_char('}')?;
                    patterns.push(Pattern::Subquery(Box::new(inner_query)));
                    self.skip_whitespace();
                    if self.peek_char() == Some('.') {
                        self.pos += 1;
                    }
                    continue;
                }
                self.pos = saved; // Not a subquery, parse as group

                // Sub-group, possibly followed by UNION
                let first_group = self.parse_group()?;
                self.skip_whitespace();
                if self.peek_keyword("UNION") {
                    let mut branches = vec![first_group];
                    while self.peek_keyword("UNION") {
                        self.expect_keyword("UNION")?;
                        let branch = self.parse_group()?;
                        branches.push(branch);
                        self.skip_whitespace();
                    }
                    patterns.push(Pattern::Union(branches));
                } else {
                    // Just a sub-group, flatten its patterns
                    patterns.extend(first_group);
                }
                self.skip_whitespace();
                if self.peek_char() == Some('.') {
                    self.pos += 1;
                }
            } else {
                // Triple pattern (possibly with property path)
                let subject = self.parse_term()?;
                self.skip_whitespace();
                let mut predicate = self.parse_term()?;
                // Check for property path modifiers: +, *, ?, /
                match self.peek_char() {
                    Some('+') => {
                        self.pos += 1;
                        predicate = Term::Path {
                            base: Box::new(predicate),
                            modifier: PathModifier::OneOrMore,
                        };
                    }
                    Some('*') => {
                        self.pos += 1;
                        predicate = Term::Path {
                            base: Box::new(predicate),
                            modifier: PathModifier::ZeroOrMore,
                        };
                    }
                    Some('/') => {
                        self.pos += 1;
                        self.skip_whitespace();
                        let next_pred = self.parse_term()?;
                        predicate = Term::Path {
                            base: Box::new(predicate),
                            modifier: PathModifier::Sequence(Box::new(next_pred)),
                        };
                    }
                    _ => {}
                }
                self.skip_whitespace();
                // Check for ? modifier (after whitespace skip since ? could be a variable)
                // Only apply ? if immediately after predicate (no space)
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

    fn parse_group(&mut self) -> Result<Vec<Pattern>> {
        self.expect_char('{')?;
        let patterns = self.parse_patterns()?;
        self.expect_char('}')?;
        Ok(patterns)
    }

    fn parse_vector_similar(&mut self) -> Result<Pattern> {
        self.expect_keyword("VECTOR_SIMILAR")?;
        self.expect_char('(')?;

        let subject = self.parse_term()?;
        self.skip_whitespace();
        let predicate = self.parse_term()?;
        self.skip_whitespace();

        // Parse the vector literal: "0.1 0.2 0.3"^^<sutra:f32vec>
        let query_vector = self.parse_vector_literal_value()?;
        self.skip_whitespace();

        self.expect_char(',')?;
        self.skip_whitespace();

        let threshold = self.parse_float()?;

        // Parse optional hints: , ef:=N or , k:=N
        let mut ef_search = None;
        let mut top_k = None;
        self.skip_whitespace();
        while self.peek_char() == Some(',') {
            self.pos += 1; // consume ','
            self.skip_whitespace();
            if self.peek_keyword("ef") {
                self.expect_keyword("ef")?;
                self.expect_char(':')?;
                self.expect_char('=')?;
                ef_search = Some(self.parse_integer()? as usize);
            } else if self.peek_keyword("k") {
                self.expect_keyword("k")?;
                self.expect_char(':')?;
                self.expect_char('=')?;
                top_k = Some(self.parse_integer()? as usize);
            } else {
                return Err(self.error("expected 'ef' or 'k' hint in VECTOR_SIMILAR"));
            }
            self.skip_whitespace();
        }

        self.expect_char(')')?;

        Ok(Pattern::VectorSimilar {
            subject,
            predicate,
            query_vector,
            threshold,
            ef_search,
            top_k,
        })
    }

    /// Parse a vector literal string and its datatype, returning the parsed f32 values.
    /// Expects: "0.1 0.2 0.3"^^<http://sutra.dev/f32vec> or "0.1 0.2 0.3"^^<sutra:f32vec>
    fn parse_vector_literal_value(&mut self) -> Result<Vec<f32>> {
        self.skip_whitespace();
        self.expect_char('"')?;
        let start = self.pos;
        while self.pos < self.input.len() && self.input.as_bytes()[self.pos] as char != '"' {
            self.pos += 1;
        }
        let value = self.input[start..self.pos].to_string();
        self.expect_char('"')?;

        // Expect ^^
        if self.input.get(self.pos..self.pos + 2) != Some("^^") {
            return Err(self.error("expected ^^ after vector literal string"));
        }
        self.pos += 2;

        // Parse the datatype IRI
        self.skip_whitespace();
        let datatype = if self.peek_char() == Some('<') {
            self.parse_iri_ref()?
        } else {
            // Try prefixed name like sutra:f32vec
            let term = self.parse_prefixed_name()?;
            match term {
                Term::PrefixedName { prefix, local } => format!("{}:{}", prefix, local),
                _ => return Err(self.error("expected IRI or prefixed name for vector datatype")),
            }
        };

        // Validate the datatype
        if datatype != "http://sutra.dev/f32vec" && datatype != "sutra:f32vec" {
            return Err(self.error(&format!("expected sutra:f32vec datatype, got {}", datatype)));
        }

        // Parse the vector values
        Self::parse_vector_string(&value).map_err(|msg| self.error(&msg))
    }

    fn parse_vector_string(s: &str) -> std::result::Result<Vec<f32>, String> {
        s.split_whitespace()
            .map(|v| {
                v.parse::<f32>()
                    .map_err(|e| format!("invalid vector component '{}': {}", v, e))
            })
            .collect()
    }

    fn parse_float(&mut self) -> Result<f32> {
        self.skip_whitespace();
        let start = self.pos;
        if self.peek_char() == Some('-') {
            self.pos += 1;
        }
        while self.pos < self.input.len() {
            let ch = self.input.as_bytes()[self.pos] as char;
            if ch.is_ascii_digit() || ch == '.' {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.input[start..self.pos]
            .parse::<f32>()
            .map_err(|_| self.error("expected floating point number"))
    }

    fn parse_order_by(&mut self) -> Result<Vec<OrderClause>> {
        let mut clauses = Vec::new();
        loop {
            self.skip_whitespace();
            if self.pos >= self.input.len() {
                break;
            }
            // Check if the next token is a solution modifier keyword rather than an order clause
            if self.peek_keyword("LIMIT") || self.peek_keyword("OFFSET") {
                break;
            }

            let descending;
            if self.peek_keyword("ASC") {
                self.expect_keyword("ASC")?;
                descending = false;
                self.expect_char('(')?;
            } else if self.peek_keyword("DESC") {
                self.expect_keyword("DESC")?;
                descending = true;
                self.expect_char('(')?;
            } else if self.peek_char() == Some('?') {
                // Bare variable, default ASC
                let var = self.parse_variable_name()?;
                clauses.push(OrderClause {
                    variable: var,
                    descending: false,
                    vector_score: None,
                });
                continue;
            } else {
                break;
            }

            self.skip_whitespace();
            // Check for VECTOR_SCORE inside the parens
            if self.peek_keyword("VECTOR_SCORE") {
                self.expect_keyword("VECTOR_SCORE")?;
                self.expect_char('(')?;
                let subject = self.parse_term()?;
                self.skip_whitespace();
                let predicate = self.parse_term()?;
                self.skip_whitespace();
                let query_vector = self.parse_vector_literal_value()?;
                self.expect_char(')')?; // close VECTOR_SCORE
                self.expect_char(')')?; // close ASC/DESC

                // Use the subject variable name as the clause variable
                let variable = match &subject {
                    Term::Variable(name) => name.clone(),
                    _ => "__vector_score__".to_string(),
                };

                clauses.push(OrderClause {
                    variable,
                    descending,
                    vector_score: Some(VectorScoreExpr {
                        subject,
                        predicate,
                        query_vector,
                    }),
                });
            } else {
                // Regular variable inside ASC/DESC
                let var = self.parse_variable_name()?;
                self.expect_char(')')?;
                clauses.push(OrderClause {
                    variable: var,
                    descending,
                    vector_score: None,
                });
            }
        }
        Ok(clauses)
    }

    fn parse_term(&mut self) -> Result<Term> {
        self.skip_whitespace();
        match self.peek_char() {
            Some('?') => {
                let name = self.parse_variable_name()?;
                Ok(Term::Variable(name))
            }
            Some('<') => {
                // Check for RDF-star quoted triple: << ?s ?p ?o >>
                if self.pos + 1 < self.input.len() && self.input.as_bytes()[self.pos + 1] == b'<' {
                    self.pos += 2; // skip '<<'
                    self.skip_whitespace();
                    let s = self.parse_term()?;
                    self.skip_whitespace();
                    let p = self.parse_term()?;
                    self.skip_whitespace();
                    let o = self.parse_term()?;
                    self.skip_whitespace();
                    // Expect >>
                    if self.pos + 1 < self.input.len()
                        && self.input.as_bytes()[self.pos] == b'>'
                        && self.input.as_bytes()[self.pos + 1] == b'>'
                    {
                        self.pos += 2;
                    } else {
                        return Err(self.error("expected >> to close quoted triple"));
                    }
                    return Ok(Term::QuotedTriple {
                        subject: Box::new(s),
                        predicate: Box::new(p),
                        object: Box::new(o),
                    });
                }
                let iri = self.parse_iri_ref()?;
                Ok(Term::Iri(iri))
            }
            Some('"') => self.parse_string_literal(),
            Some(c) if c.is_ascii_digit() || c == '-' => {
                let n = self.parse_integer()?;
                Ok(Term::IntegerLiteral(n))
            }
            Some(':') => {
                // Empty prefix: :localName
                self.parse_prefixed_name()
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
        self.skip_whitespace();

        // FILTER NOT EXISTS { ... } (no parentheses)
        if self.peek_keyword("NOT") {
            self.expect_keyword("NOT")?;
            self.skip_whitespace();
            self.expect_keyword("EXISTS")?;
            self.skip_whitespace();
            let patterns = self.parse_group()?;
            return Ok(FilterExpr::NotExists(patterns));
        }

        // FILTER EXISTS { ... } (no parentheses)
        if self.peek_keyword("EXISTS") {
            self.expect_keyword("EXISTS")?;
            self.skip_whitespace();
            let patterns = self.parse_group()?;
            return Ok(FilterExpr::Exists(patterns));
        }

        self.expect_char('(')?;
        self.skip_whitespace();

        // FILTER(NOT EXISTS { ... })
        if self.peek_keyword("NOT") {
            self.expect_keyword("NOT")?;
            self.skip_whitespace();
            self.expect_keyword("EXISTS")?;
            self.skip_whitespace();
            let patterns = self.parse_group()?;
            self.skip_whitespace();
            self.expect_char(')')?;
            return Ok(FilterExpr::NotExists(patterns));
        }

        // FILTER(EXISTS { ... })
        if self.peek_keyword("EXISTS") {
            self.expect_keyword("EXISTS")?;
            self.skip_whitespace();
            let patterns = self.parse_group()?;
            self.skip_whitespace();
            self.expect_char(')')?;
            return Ok(FilterExpr::Exists(patterns));
        }

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
            // General negation: !(expr)
            let inner = self.parse_filter_inner()?;
            self.skip_whitespace();
            self.expect_char(')')?;
            return Ok(FilterExpr::Not(Box::new(inner)));
        }

        // String functions: CONTAINS, STRSTARTS, STRENDS, REGEX
        if self.peek_keyword("CONTAINS") {
            return self.parse_two_arg_string_filter("CONTAINS", FilterExpr::Contains);
        }
        if self.peek_keyword("STRSTARTS") {
            return self.parse_two_arg_string_filter("STRSTARTS", FilterExpr::StrStarts);
        }
        if self.peek_keyword("STRENDS") {
            return self.parse_two_arg_string_filter("STRENDS", FilterExpr::StrEnds);
        }
        if self.peek_keyword("REGEX") {
            return self.parse_two_arg_string_filter("REGEX", FilterExpr::Regex);
        }
        if self.peek_keyword("LANGMATCHES") {
            self.expect_keyword("LANGMATCHES")?;
            self.expect_char('(')?;
            self.skip_whitespace();
            // Expect LANG(?var)
            self.expect_keyword("LANG")?;
            self.expect_char('(')?;
            let var = self.parse_variable_name()?;
            self.expect_char(')')?;
            self.skip_whitespace();
            self.expect_char(',')?;
            self.skip_whitespace();
            let lang_term = self.parse_term()?;
            let lang = match &lang_term {
                Term::Literal(s) => s.clone(),
                _ => return Err(self.error("LANGMATCHES expects a string literal")),
            };
            self.skip_whitespace();
            self.expect_char(')')?;
            self.skip_whitespace();
            self.expect_char(')')?;
            return Ok(FilterExpr::LangMatches(var, lang));
        }
        if self.peek_keyword("LANG") {
            self.expect_keyword("LANG")?;
            self.expect_char('(')?;
            let var = self.parse_variable_name()?;
            self.expect_char(')')?;
            self.skip_whitespace();
            let op = self.parse_comparison_op()?;
            self.skip_whitespace();
            let lang_term = self.parse_term()?;
            let lang = match &lang_term {
                Term::Literal(s) => s.clone(),
                _ => return Err(self.error("LANG() comparison expects a string literal")),
            };
            self.skip_whitespace();
            self.expect_char(')')?;
            if op == "=" {
                return Ok(FilterExpr::LangMatches(var, lang));
            }
            return Err(self.error("LANG() only supports = comparison"));
        }
        if self.peek_keyword("DATATYPE") {
            self.expect_keyword("DATATYPE")?;
            self.expect_char('(')?;
            let var = self.parse_variable_name()?;
            self.expect_char(')')?;
            self.skip_whitespace();
            let op = self.parse_comparison_op()?;
            self.skip_whitespace();
            let dt_term = self.parse_term()?;
            let dt = match &dt_term {
                Term::Iri(s) => s.clone(),
                Term::PrefixedName { prefix, local } => format!("{}:{}", prefix, local),
                _ => return Err(self.error("DATATYPE() comparison expects an IRI")),
            };
            self.skip_whitespace();
            self.expect_char(')')?;
            if op == "=" {
                return Ok(FilterExpr::DatatypeEquals(var, dt));
            }
            return Err(self.error("DATATYPE() only supports = comparison"));
        }
        if self.peek_keyword("STR")
            && !self.peek_keyword("STRSTARTS")
            && !self.peek_keyword("STRENDS")
        {
            self.expect_keyword("STR")?;
            self.expect_char('(')?;
            let var = self.parse_variable_name()?;
            self.expect_char(')')?;
            self.skip_whitespace();
            let op = self.parse_comparison_op()?;
            self.skip_whitespace();
            let val = self.parse_term()?;
            self.skip_whitespace();
            self.expect_char(')')?;
            if op == "=" {
                return Ok(FilterExpr::StrEquals(var, val));
            }
            return Err(self.error("STR() only supports = comparison"));
        }
        if self.peek_keyword("isIRI") || self.peek_keyword("isURI") {
            let kw = if self.peek_keyword("isIRI") {
                "isIRI"
            } else {
                "isURI"
            };
            self.expect_keyword(kw)?;
            self.expect_char('(')?;
            let var = self.parse_variable_name()?;
            self.expect_char(')')?;
            self.expect_char(')')?;
            return Ok(FilterExpr::IsIri(var));
        }
        if self.peek_keyword("isLiteral") {
            self.expect_keyword("isLiteral")?;
            self.expect_char('(')?;
            let var = self.parse_variable_name()?;
            self.expect_char(')')?;
            self.expect_char(')')?;
            return Ok(FilterExpr::IsLiteral(var));
        }

        // Parse a comparison expression, then check for && / ||
        let expr = self.parse_comparison_expr()?;
        self.skip_whitespace();

        // Check for boolean connectives
        if self.remaining().starts_with("&&") {
            self.pos += 2;
            self.skip_whitespace();
            let right = self.parse_filter_inner()?;
            self.skip_whitespace();
            self.expect_char(')')?;
            return Ok(FilterExpr::And(Box::new(expr), Box::new(right)));
        }
        if self.remaining().starts_with("||") {
            self.pos += 2;
            self.skip_whitespace();
            let right = self.parse_filter_inner()?;
            self.skip_whitespace();
            self.expect_char(')')?;
            return Ok(FilterExpr::Or(Box::new(expr), Box::new(right)));
        }

        self.expect_char(')')?;
        Ok(expr)
    }

    /// Parse a filter expression without the outer parens (for recursive use).
    fn parse_filter_inner(&mut self) -> Result<FilterExpr> {
        self.skip_whitespace();
        if self.peek_keyword("bound") {
            self.expect_keyword("bound")?;
            self.expect_char('(')?;
            let var = self.parse_variable_name()?;
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
                return Ok(FilterExpr::NotBound(var));
            }
            let inner = self.parse_filter_inner()?;
            return Ok(FilterExpr::Not(Box::new(inner)));
        }
        self.parse_comparison_expr()
    }

    fn parse_comparison_expr(&mut self) -> Result<FilterExpr> {
        let left = self.parse_term()?;
        self.skip_whitespace();

        let op = self.parse_comparison_op()?;
        self.skip_whitespace();

        let right = self.parse_term()?;

        match op.as_str() {
            "=" => Ok(FilterExpr::Equals(left, right)),
            "!=" => Ok(FilterExpr::NotEquals(left, right)),
            "<" => Ok(FilterExpr::LessThan(left, right)),
            ">" => Ok(FilterExpr::GreaterThan(left, right)),
            ">=" => Ok(FilterExpr::GreaterThanOrEqual(left, right)),
            "<=" => Ok(FilterExpr::LessThanOrEqual(left, right)),
            _ => Err(self.error(&format!("unknown operator: {}", op))),
        }
    }

    fn parse_two_arg_string_filter(
        &mut self,
        keyword: &str,
        ctor: impl FnOnce(Term, Term) -> FilterExpr,
    ) -> Result<FilterExpr> {
        self.expect_keyword(keyword)?;
        self.expect_char('(')?;
        self.skip_whitespace();
        let arg1 = self.parse_term()?;
        self.skip_whitespace();
        self.expect_char(',')?;
        self.skip_whitespace();
        let arg2 = self.parse_term()?;
        self.skip_whitespace();
        self.expect_char(')')?;
        self.skip_whitespace();
        self.expect_char(')')?;
        Ok(ctor(arg1, arg2))
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
                if self.peek_char() == Some('=') {
                    self.pos += 1;
                    Ok("<=".to_string())
                } else {
                    Ok("<".to_string())
                }
            }
            Some('>') => {
                self.pos += 1;
                if self.peek_char() == Some('=') {
                    self.pos += 1;
                    Ok(">=".to_string())
                } else {
                    Ok(">".to_string())
                }
            }
            _ => Err(self.error("expected comparison operator")),
        }
    }

    fn remaining(&self) -> &str {
        &self.input[self.pos..]
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
        // Handle empty prefix case (e.g., :localName)
        if self.peek_char() == Some(':') {
            let prefix = String::new();
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
            // Check if it's a prefixed name or full IRI
            self.skip_whitespace();
            if self.peek_char() == Some('<') {
                let datatype = self.parse_iri_ref()?;
                if datatype == "http://sutra.dev/f32vec" {
                    let vec = Self::parse_vector_string(&value).map_err(|msg| self.error(&msg))?;
                    Ok(Term::VectorLiteral(vec))
                } else {
                    Ok(Term::TypedLiteral { value, datatype })
                }
            } else {
                // Try prefixed name
                let saved_pos = self.pos;
                match self.parse_prefixed_name() {
                    Ok(Term::PrefixedName {
                        ref prefix,
                        ref local,
                    }) if prefix == "sutra" && local == "f32vec" => {
                        let vec =
                            Self::parse_vector_string(&value).map_err(|msg| self.error(&msg))?;
                        Ok(Term::VectorLiteral(vec))
                    }
                    Ok(Term::PrefixedName { prefix, local }) => Ok(Term::TypedLiteral {
                        value,
                        datatype: format!("{}:{}", prefix, local),
                    }),
                    _ => {
                        self.pos = saved_pos;
                        let datatype = self.parse_iri_ref()?;
                        Ok(Term::TypedLiteral { value, datatype })
                    }
                }
            }
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

    #[test]
    fn parse_vector_similar_with_threshold() {
        let q = parse(
            r#"SELECT ?doc WHERE { VECTOR_SIMILAR(?doc :hasEmbedding "0.1 0.2 0.3"^^<http://sutra.dev/f32vec>, 0.85) }"#,
        )
        .unwrap();
        assert_eq!(q.patterns.len(), 1);
        match &q.patterns[0] {
            Pattern::VectorSimilar {
                subject,
                predicate,
                query_vector,
                threshold,
                ef_search,
                top_k,
            } => {
                assert_eq!(*subject, Term::Variable("doc".to_string()));
                assert_eq!(
                    *predicate,
                    Term::PrefixedName {
                        prefix: String::new(),
                        local: "hasEmbedding".to_string()
                    }
                );
                assert_eq!(query_vector, &[0.1f32, 0.2, 0.3]);
                assert!((threshold - 0.85).abs() < f32::EPSILON);
                assert!(ef_search.is_none());
                assert!(top_k.is_none());
            }
            _ => panic!("expected VectorSimilar pattern"),
        }
    }

    #[test]
    fn parse_vector_similar_with_ef_hint() {
        let q = parse(
            r#"SELECT ?doc WHERE { VECTOR_SIMILAR(?doc :hasEmbedding "0.1 0.2"^^<http://sutra.dev/f32vec>, 0.9, ef:=200) }"#,
        )
        .unwrap();
        match &q.patterns[0] {
            Pattern::VectorSimilar {
                ef_search, top_k, ..
            } => {
                assert_eq!(*ef_search, Some(200));
                assert!(top_k.is_none());
            }
            _ => panic!("expected VectorSimilar pattern"),
        }
    }

    #[test]
    fn parse_vector_similar_with_k_hint() {
        let q = parse(
            r#"SELECT ?doc WHERE { VECTOR_SIMILAR(?doc :hasEmbedding "0.1 0.2"^^<http://sutra.dev/f32vec>, 0.9, k:=10) }"#,
        )
        .unwrap();
        match &q.patterns[0] {
            Pattern::VectorSimilar {
                top_k, ef_search, ..
            } => {
                assert_eq!(*top_k, Some(10));
                assert!(ef_search.is_none());
            }
            _ => panic!("expected VectorSimilar pattern"),
        }
    }

    #[test]
    fn parse_order_by_asc_desc() {
        let q = parse("SELECT ?s ?name WHERE { ?s ?p ?name } ORDER BY ASC(?name)").unwrap();
        assert_eq!(q.order_by.len(), 1);
        assert_eq!(q.order_by[0].variable, "name");
        assert!(!q.order_by[0].descending);
        assert!(q.order_by[0].vector_score.is_none());

        let q2 = parse("SELECT ?s ?name WHERE { ?s ?p ?name } ORDER BY DESC(?name)").unwrap();
        assert_eq!(q2.order_by.len(), 1);
        assert_eq!(q2.order_by[0].variable, "name");
        assert!(q2.order_by[0].descending);
    }

    #[test]
    fn parse_order_by_vector_score() {
        let q = parse(
            r#"SELECT ?doc WHERE { ?doc ?p ?o } ORDER BY DESC(VECTOR_SCORE(?doc :hasEmbedding "0.1 0.2 0.3"^^<http://sutra.dev/f32vec>))"#,
        )
        .unwrap();
        assert_eq!(q.order_by.len(), 1);
        assert!(q.order_by[0].descending);
        let vs = q.order_by[0].vector_score.as_ref().unwrap();
        assert_eq!(vs.subject, Term::Variable("doc".to_string()));
        assert_eq!(vs.query_vector, vec![0.1f32, 0.2, 0.3]);
    }

    #[test]
    fn parse_union() {
        let q = parse(
            "SELECT ?s WHERE { \
             { ?s a :Person } UNION { ?s a :Organization } \
             }",
        )
        .unwrap();
        assert_eq!(q.patterns.len(), 1);
        match &q.patterns[0] {
            Pattern::Union(branches) => {
                assert_eq!(branches.len(), 2);
                assert_eq!(branches[0].len(), 1);
                assert_eq!(branches[1].len(), 1);
            }
            _ => panic!("expected Union pattern"),
        }
    }

    #[test]
    fn parse_vector_literal_in_triple() {
        let q = parse(
            r#"SELECT ?s WHERE { ?s :hasEmbedding "0.5 -0.3 1.0"^^<http://sutra.dev/f32vec> }"#,
        )
        .unwrap();
        if let Pattern::Triple { object, .. } = &q.patterns[0] {
            assert_eq!(*object, Term::VectorLiteral(vec![0.5, -0.3, 1.0]));
        } else {
            panic!("expected triple pattern");
        }
    }
}
