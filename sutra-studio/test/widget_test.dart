import 'package:flutter_test/flutter_test.dart';
import 'package:sutra_studio/models/triple.dart';

void main() {
  test('shortName abbreviates Wikidata IRIs', () {
    expect(Triple.shortName('http://www.wikidata.org/entity/Q42'),
        'wd:Q42');
  });

  test('shortName abbreviates RDF type', () {
    expect(
        Triple.shortName(
            'http://www.w3.org/1999/02/22-rdf-syntax-ns#type'),
        'rdf:type');
  });

  test('shortName strips literal quotes', () {
    expect(Triple.shortName('"hello world"'), 'hello world');
  });

  test('shortName strips language tag', () {
    expect(Triple.shortName('"延喜式神名帳"@ja'), '延喜式神名帳');
  });

  test('shortName falls back to last path segment', () {
    expect(Triple.shortName('http://example.org/foo/bar'), 'bar');
  });
}
