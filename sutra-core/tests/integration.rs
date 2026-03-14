//! Integration tests for sutra-core: TermDictionary + TripleStore together.

use sutra_core::*;

#[test]
fn full_roundtrip_with_dictionary() {
    let mut dict = TermDictionary::new();
    let mut store = TripleStore::new();

    // Intern some IRIs
    let alice = dict.intern("http://example.org/Alice");
    let bob = dict.intern("http://example.org/Bob");
    let knows = dict.intern("http://example.org/knows");
    let name = dict.intern("http://example.org/name");

    // Use inline literal for a name (as string ID for now)
    let alice_name = dict.intern("\"Alice\"");

    // Insert triples
    store.insert(Triple::new(alice, knows, bob)).unwrap();
    store.insert(Triple::new(alice, name, alice_name)).unwrap();

    // Query: what does Alice know?
    let results = store.find_by_subject_predicate(alice, knows);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].object, bob);

    // Resolve back to strings
    assert_eq!(
        dict.resolve(results[0].object),
        Some("http://example.org/Bob")
    );
}

#[test]
fn rdf_star_quoted_triples() {
    let mut dict = TermDictionary::new();
    let mut store = TripleStore::new();

    let alice = dict.intern("http://example.org/Alice");
    let bob = dict.intern("http://example.org/Bob");
    let knows = dict.intern("http://example.org/knows");
    let confidence = dict.intern("http://example.org/confidence");

    // Create the base triple and get its quoted ID
    store.insert(Triple::new(alice, knows, bob)).unwrap();
    let qt_id = quoted_triple_id(alice, knows, bob);

    // Annotate the quoted triple with confidence (RDF-star)
    let conf_value = inline_integer(91).unwrap();
    store
        .insert(Triple::new(qt_id, confidence, conf_value))
        .unwrap();

    // Query: what annotations exist on the quoted triple?
    let annotations = store.find_by_subject(qt_id);
    assert_eq!(annotations.len(), 1);
    assert_eq!(annotations[0].predicate, confidence);
    assert_eq!(decode_inline_integer(annotations[0].object), Some(91));
}

#[test]
fn inline_literals_in_triples() {
    let mut dict = TermDictionary::new();
    let mut store = TripleStore::new();

    let alice = dict.intern("http://example.org/Alice");
    let age = dict.intern("http://example.org/age");
    let active = dict.intern("http://example.org/active");

    let age_val = inline_integer(30).unwrap();
    let active_val = inline_boolean(true);

    store.insert(Triple::new(alice, age, age_val)).unwrap();
    store
        .insert(Triple::new(alice, active, active_val))
        .unwrap();

    // Query Alice's age
    let age_triples = store.find_by_subject_predicate(alice, age);
    assert_eq!(age_triples.len(), 1);
    assert!(is_inline(age_triples[0].object));
    assert_eq!(decode_inline_integer(age_triples[0].object), Some(30));

    // Query Alice's active status
    let active_triples = store.find_by_subject_predicate(alice, active);
    assert_eq!(active_triples.len(), 1);
    assert_eq!(decode_inline_boolean(active_triples[0].object), Some(true));
}

#[test]
fn bulk_insert_and_query() {
    let mut dict = TermDictionary::new();
    let mut store = TripleStore::new();

    let rdf_type = dict.intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
    let person = dict.intern("http://example.org/Person");
    let knows = dict.intern("http://example.org/knows");

    // Create 100 people
    let mut people = Vec::new();
    for i in 0..100 {
        let p = dict.intern(&format!("http://example.org/person/{}", i));
        store.insert(Triple::new(p, rdf_type, person)).unwrap();
        people.push(p);
    }

    // Each person knows the next
    for i in 0..99 {
        store
            .insert(Triple::new(people[i], knows, people[i + 1]))
            .unwrap();
    }

    assert_eq!(store.len(), 199); // 100 type + 99 knows

    // All instances of Person
    let persons = store
        .find_by_predicate(rdf_type)
        .into_iter()
        .filter(|t| t.object == person)
        .count();
    assert_eq!(persons, 100);

    // Person 50's outgoing knows edges
    let friends = store.find_by_subject_predicate(people[50], knows);
    assert_eq!(friends.len(), 1);
    assert_eq!(friends[0].object, people[51]);

    // Who knows person 50? (reverse traversal via OSP index)
    let known_by = store.find_by_object(people[50]);
    assert_eq!(known_by.len(), 1);
    assert_eq!(known_by[0].subject, people[49]);
}

#[test]
fn remove_and_reinsert() {
    let mut store = TripleStore::new();
    let t = Triple::new(1, 2, 3);

    store.insert(t).unwrap();
    assert!(store.contains(&t));

    store.remove(&t);
    assert!(!store.contains(&t));
    assert_eq!(store.len(), 0);

    // Reinsert should work
    store.insert(t).unwrap();
    assert!(store.contains(&t));
    assert_eq!(store.len(), 1);
}

#[test]
fn index_consistency_after_operations() {
    let mut store = TripleStore::new();

    store.insert(Triple::new(1, 10, 100)).unwrap();
    store.insert(Triple::new(1, 10, 200)).unwrap();
    store.insert(Triple::new(2, 10, 100)).unwrap();
    store.insert(Triple::new(2, 20, 300)).unwrap();

    // Remove one triple
    store.remove(&Triple::new(1, 10, 100));

    // SPO index: subject 1 should have 1 triple
    assert_eq!(store.find_by_subject(1).len(), 1);

    // POS index: predicate 10 should have 2 triples (was 3, removed 1)
    assert_eq!(store.find_by_predicate(10).len(), 2);

    // OSP index: object 100 should have 1 triple (was 2, removed 1)
    assert_eq!(store.find_by_object(100).len(), 1);
}
