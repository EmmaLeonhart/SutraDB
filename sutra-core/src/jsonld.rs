//! Minimal JSON-LD parser.
//!
//! Parses a subset of JSON-LD into RDF triples. Handles:
//! - @context with prefix mappings
//! - @id for subject IRIs
//! - @type for rdf:type
//! - @value / @language for literals
//! - Simple property → value pairs
//!
//! For full JSON-LD compliance, use a dedicated library.

use std::collections::HashMap;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// Parse JSON-LD text into a list of (subject, predicate, object) string triples.
pub fn parse_jsonld(input: &str) -> Vec<(String, String, String)> {
    let parsed: serde_json::Value = match serde_json::from_str(input) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let mut triples = Vec::new();
    let context = extract_context(&parsed);

    match &parsed {
        serde_json::Value::Object(obj) => {
            process_node(obj, &context, &mut triples);
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                if let serde_json::Value::Object(obj) = item {
                    process_node(obj, &context, &mut triples);
                }
            }
        }
        _ => {}
    }

    triples
}

fn extract_context(value: &serde_json::Value) -> HashMap<String, String> {
    let mut ctx = HashMap::new();
    if let Some(context) = value.get("@context") {
        match context {
            serde_json::Value::Object(obj) => {
                for (key, val) in obj {
                    if let serde_json::Value::String(uri) = val {
                        ctx.insert(key.clone(), uri.clone());
                    }
                }
            }
            serde_json::Value::String(uri) => {
                ctx.insert(String::new(), uri.clone());
            }
            _ => {}
        }
    }
    ctx
}

fn expand_iri(term: &str, context: &HashMap<String, String>) -> String {
    if term.starts_with("http://") || term.starts_with("https://") {
        return term.to_string();
    }
    if let Some(colon) = term.find(':') {
        let prefix = &term[..colon];
        let local = &term[colon + 1..];
        if let Some(base) = context.get(prefix) {
            return format!("{}{}", base, local);
        }
    }
    if let Some(base) = context.get("@vocab") {
        return format!("{}{}", base, term);
    }
    term.to_string()
}

fn process_node(
    obj: &serde_json::Map<String, serde_json::Value>,
    context: &HashMap<String, String>,
    triples: &mut Vec<(String, String, String)>,
) {
    let subject = obj
        .get("@id")
        .and_then(|v| v.as_str())
        .map(|s| expand_iri(s, context))
        .unwrap_or_else(|| format!("_:node{}", triples.len()));

    // @type
    if let Some(type_val) = obj.get("@type") {
        match type_val {
            serde_json::Value::String(t) => {
                triples.push((
                    subject.clone(),
                    RDF_TYPE.to_string(),
                    expand_iri(t, context),
                ));
            }
            serde_json::Value::Array(types) => {
                for t in types {
                    if let serde_json::Value::String(t) = t {
                        triples.push((
                            subject.clone(),
                            RDF_TYPE.to_string(),
                            expand_iri(t, context),
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    // Other properties
    for (key, value) in obj {
        if key.starts_with('@') {
            continue; // Skip JSON-LD keywords
        }

        let predicate = expand_iri(key, context);

        match value {
            serde_json::Value::String(s) => {
                if s.starts_with("http://") || s.starts_with("https://") {
                    triples.push((subject.clone(), predicate, s.clone()));
                } else {
                    triples.push((subject.clone(), predicate, format!("\"{}\"", s)));
                }
            }
            serde_json::Value::Number(n) => {
                triples.push((subject.clone(), predicate, format!("\"{}\"", n)));
            }
            serde_json::Value::Bool(b) => {
                triples.push((subject.clone(), predicate, format!("\"{}\"", b)));
            }
            serde_json::Value::Object(inner) => {
                // Check for @value / @id
                if let Some(val) = inner.get("@value") {
                    let val_str = val.as_str().unwrap_or("").to_string();
                    if let Some(lang) = inner.get("@language") {
                        let lang_str = lang.as_str().unwrap_or("");
                        triples.push((
                            subject.clone(),
                            predicate,
                            format!("\"{}\"@{}", val_str, lang_str),
                        ));
                    } else if let Some(dt) = inner.get("@type") {
                        let dt_str = expand_iri(dt.as_str().unwrap_or(""), context);
                        triples.push((
                            subject.clone(),
                            predicate,
                            format!("\"{}\"^^<{}>", val_str, dt_str),
                        ));
                    } else {
                        triples.push((subject.clone(), predicate, format!("\"{}\"", val_str)));
                    }
                } else if let Some(id) = inner.get("@id") {
                    let id_str = expand_iri(id.as_str().unwrap_or(""), context);
                    triples.push((subject.clone(), predicate, id_str));
                } else {
                    // Nested node
                    process_node(inner, context, triples);
                }
            }
            serde_json::Value::Array(arr) => {
                for item in arr {
                    match item {
                        serde_json::Value::String(s) => {
                            triples.push((
                                subject.clone(),
                                predicate.clone(),
                                expand_iri(s, context),
                            ));
                        }
                        serde_json::Value::Object(inner) => {
                            if let Some(id) = inner.get("@id") {
                                let id_str = expand_iri(id.as_str().unwrap_or(""), context);
                                triples.push((subject.clone(), predicate.clone(), id_str));
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_jsonld() {
        let json = r#"{
            "@context": {
                "ex": "http://example.org/",
                "name": "http://example.org/name"
            },
            "@id": "http://example.org/Alice",
            "@type": "ex:Person",
            "name": "Alice"
        }"#;
        let triples = parse_jsonld(json);
        assert!(triples.len() >= 2);
        // Should have rdf:type and name
        assert!(triples.iter().any(|(_, p, _)| p.contains("type")));
        assert!(triples.iter().any(|(_, p, _)| p.contains("name")));
    }

    #[test]
    fn parse_jsonld_array() {
        let json = r#"[
            {"@id": "http://example.org/a", "http://example.org/p": "hello"},
            {"@id": "http://example.org/b", "http://example.org/p": "world"}
        ]"#;
        let triples = parse_jsonld(json);
        assert_eq!(triples.len(), 2);
    }
}
