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

  /// Short display name for an IRI (last segment after # or /).
  static String shortName(String iri) {
    final cleaned = iri.replaceAll(RegExp(r'[<>]'), '');
    final hashIdx = cleaned.lastIndexOf('#');
    if (hashIdx >= 0) return cleaned.substring(hashIdx + 1);
    final slashIdx = cleaned.lastIndexOf('/');
    if (slashIdx >= 0) return cleaned.substring(slashIdx + 1);
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
