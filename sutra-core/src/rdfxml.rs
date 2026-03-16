//! Minimal RDF/XML parser.
//!
//! Parses a subset of RDF/XML sufficient for OWL ontology files.
//! Uses simple string-based XML parsing (no external XML crate).
//! For full RDF/XML compliance, use Oxigraph's oxrdfxml crate.

/// Parse RDF/XML text into a list of (subject, predicate, object) string triples.
pub fn parse_rdfxml(input: &str) -> Vec<(String, String, String)> {
    let mut triples = Vec::new();
    let mut base_uri = String::new();
    let mut namespaces: Vec<(String, String)> = Vec::new();

    // Extract namespaces from rdf:RDF element
    if let Some(rdf_start) = input.find("<rdf:RDF") {
        let rdf_end = input[rdf_start..].find('>').unwrap_or(0) + rdf_start;
        let header = &input[rdf_start..rdf_end];

        // Extract xmlns declarations
        let mut pos = 0;
        let header_bytes = header.as_bytes();
        while pos < header.len() {
            if header[pos..].starts_with("xmlns:") {
                pos += 6;
                let prefix_end = header[pos..].find('=').unwrap_or(0) + pos;
                let prefix = header[pos..prefix_end].to_string();
                pos = prefix_end + 1;
                // Skip quote
                if pos < header.len() && (header_bytes[pos] == b'"' || header_bytes[pos] == b'\'') {
                    let quote = header_bytes[pos];
                    pos += 1;
                    let uri_end = header[pos..].find(quote as char).unwrap_or(0) + pos;
                    let uri = header[pos..uri_end].to_string();
                    namespaces.push((prefix, uri));
                    pos = uri_end + 1;
                    continue;
                }
            } else if header[pos..].starts_with("xml:base=") {
                pos += 9;
                if pos < header.len() && (header_bytes[pos] == b'"' || header_bytes[pos] == b'\'') {
                    let quote = header_bytes[pos];
                    pos += 1;
                    let uri_end = header[pos..].find(quote as char).unwrap_or(0) + pos;
                    base_uri = header[pos..uri_end].to_string();
                    pos = uri_end + 1;
                    continue;
                }
            }
            pos += 1;
        }
    }

    let expand = |name: &str| -> String {
        if let Some(colon) = name.find(':') {
            let prefix = &name[..colon];
            let local = &name[colon + 1..];
            for (p, uri) in &namespaces {
                if p == prefix {
                    return format!("{}{}", uri, local);
                }
            }
        }
        if name.starts_with("http://") || name.starts_with("https://") {
            return name.to_string();
        }
        format!("{}{}", base_uri, name)
    };

    // Parse rdf:Description elements
    let mut search_pos = 0;
    while let Some(desc_start) = input[search_pos..].find("<rdf:Description") {
        let abs_start = search_pos + desc_start;

        // Extract rdf:about
        let desc_header_end = input[abs_start..].find('>').unwrap_or(0) + abs_start;
        let header = &input[abs_start..desc_header_end];

        let subject = if let Some(about_pos) = header.find("rdf:about=") {
            let val_start = about_pos + 11; // skip rdf:about="
            let quote = header.as_bytes()[about_pos + 10];
            let val_end = header[val_start..].find(quote as char).unwrap_or(0) + val_start;
            expand(&header[val_start..val_end])
        } else {
            format!("_:desc{}", abs_start)
        };

        // Find closing tag
        let close_tag = "</rdf:Description>";
        let desc_end = input[desc_header_end..].find(close_tag).unwrap_or(0) + desc_header_end;
        let body = &input[desc_header_end + 1..desc_end];

        // Parse property elements within the Description
        let mut prop_pos = 0;
        while prop_pos < body.len() {
            if body.as_bytes()[prop_pos] == b'<'
                && prop_pos + 1 < body.len()
                && body.as_bytes()[prop_pos + 1] != b'/'
            {
                prop_pos += 1;
                let tag_end = body[prop_pos..]
                    .find(|c: char| c == '>' || c == ' ' || c == '/')
                    .unwrap_or(0)
                    + prop_pos;
                let tag_name = &body[prop_pos..tag_end];

                if tag_name.is_empty() || tag_name.starts_with('!') || tag_name.starts_with('?') {
                    prop_pos = tag_end + 1;
                    continue;
                }

                let predicate = expand(tag_name);

                // Check for rdf:resource attribute (object is a URI)
                let attr_end = body[prop_pos..].find('>').unwrap_or(0) + prop_pos;
                let attrs = &body[prop_pos..attr_end];

                if let Some(res_pos) = attrs.find("rdf:resource=") {
                    let val_start = res_pos + 14;
                    let quote = attrs.as_bytes()[res_pos + 13];
                    let val_end = attrs[val_start..].find(quote as char).unwrap_or(0) + val_start;
                    let object = expand(&attrs[val_start..val_end]);
                    triples.push((subject.clone(), predicate, object));
                } else if !body[attr_end..].starts_with("/>") {
                    // Literal content
                    let content_start = attr_end + 1;
                    let close = format!("</{}>", tag_name);
                    if let Some(close_pos) = body[content_start..].find(&close) {
                        let content = &body[content_start..content_start + close_pos];
                        let content = content.trim();
                        if !content.is_empty() {
                            triples.push((subject.clone(), predicate, format!("\"{}\"", content)));
                        }
                        prop_pos = content_start + close_pos + close.len();
                        continue;
                    }
                }

                prop_pos = attr_end + 1;
            } else {
                prop_pos += 1;
            }
        }

        search_pos = desc_end + close_tag.len();
    }

    triples
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_rdfxml() {
        let xml = r#"<?xml version="1.0"?>
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
         xmlns:ex="http://example.org/">
  <rdf:Description rdf:about="http://example.org/Alice">
    <ex:knows rdf:resource="http://example.org/Bob"/>
    <ex:name>Alice</ex:name>
  </rdf:Description>
</rdf:RDF>"#;
        let triples = parse_rdfxml(xml);
        assert_eq!(triples.len(), 2);
        assert_eq!(triples[0].0, "http://example.org/Alice");
        assert_eq!(triples[0].1, "http://example.org/knows");
        assert_eq!(triples[0].2, "http://example.org/Bob");
        assert_eq!(triples[1].2, "\"Alice\"");
    }
}
