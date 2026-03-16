/// Represents an RDF triple (or RDF-star quoted triple).
class Triple {
  final String subject;
  final String predicate;
  final String object;

  /// If non-null, this triple is a quoted triple annotation.
  final Triple? quotedTriple;

  /// Whether the object is a vector literal (sutra:f32vec).
  bool get isVector =>
      object.contains('sutra:f32vec') || predicate.contains('hasEmbedding');

  /// Whether this is an RDF-star quoted triple pattern.
  bool get isQuotedTriple => quotedTriple != null;

  const Triple({
    required this.subject,
    required this.predicate,
    required this.object,
    this.quotedTriple,
  });

  factory Triple.fromSparqlRow(Map<String, dynamic> row) {
    return Triple(
      subject: _extractValue(row['s'] ?? row['subject'] ?? ''),
      predicate: _extractValue(row['p'] ?? row['predicate'] ?? ''),
      object: _extractValue(row['o'] ?? row['object'] ?? ''),
    );
  }

  static String _extractValue(dynamic val) {
    if (val is Map) return val['value']?.toString() ?? '';
    return val.toString();
  }

  /// Convert to N-Triples format for insertion.
  String toNTriples() {
    final s = _wrapIri(subject);
    final p = _wrapIri(predicate);
    final o = object.startsWith('"') ? object : _wrapIri(object);
    return '$s $p $o .';
  }

  static String _wrapIri(String iri) {
    if (iri.startsWith('<') && iri.endsWith('>')) return iri;
    if (iri.startsWith('_:')) return iri; // blank node
    return '<$iri>';
  }

  /// Known prefix abbreviations for compact display.
  static const _prefixes = {
    'http://www.w3.org/1999/02/22-rdf-syntax-ns#': 'rdf:',
    'http://www.w3.org/2000/01/rdf-schema#': 'rdfs:',
    'http://www.w3.org/2002/07/owl#': 'owl:',
    'http://www.w3.org/2001/XMLSchema#': 'xsd:',
    'http://www.wikidata.org/entity/': 'wd:',
    'http://www.wikidata.org/prop/direct/': 'wdt:',
    'http://schema.org/': 'schema:',
    'http://sutra.dev/': 'sutra:',
    'http://www.w3.org/2004/02/skos/core#': 'skos:',
    'http://xmlns.com/foaf/0.1/': 'foaf:',
    'http://purl.org/dc/terms/': 'dcterms:',
    'http://www.w3.org/2003/01/geo/wgs84_pos#': 'geo:',
  };

  /// Short display name for an IRI using known prefix abbreviations.
  static String shortName(String iri) {
    final cleaned = iri.replaceAll(RegExp(r'[<>]'), '');
    // Strip language tag from literals for display
    if (cleaned.startsWith('"')) {
      final atIdx = cleaned.lastIndexOf('"@');
      if (atIdx > 0) return cleaned.substring(1, atIdx);
      final end = cleaned.indexOf('"', 1);
      if (end > 0) return cleaned.substring(1, end);
      return cleaned;
    }
    // Try known prefixes first
    for (final entry in _prefixes.entries) {
      if (cleaned.startsWith(entry.key)) {
        return '${entry.value}${cleaned.substring(entry.key.length)}';
      }
    }
    // Fallback: last segment after # or /
    final hashIdx = cleaned.lastIndexOf('#');
    if (hashIdx >= 0) return cleaned.substring(hashIdx + 1);
    final slashIdx = cleaned.lastIndexOf('/');
    if (slashIdx >= 0 && slashIdx < cleaned.length - 1) {
      return cleaned.substring(slashIdx + 1);
    }
    return cleaned;
  }

  @override
  String toString() => '$subject $predicate $object';
}

/// Classification of a triple for view mode filtering.
enum TripleType { semantic, vector, hnswEdge }

TripleType classifyTriple(Triple t) {
  if (t.isVector) return TripleType.vector;
  if (t.predicate.contains('hnswNeighbor')) return TripleType.hnswEdge;
  return TripleType.semantic;
}
