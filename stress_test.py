"""
SutraDB Stress Test — 1M embeddings + heavy SPARQL queries.

Generates a synthetic knowledge graph with:
- 100K entities across 10 types
- ~500K triples (type assertions, relationships, properties)
- 1M embeddings (1024-dim, 10 per entity for label+aliases)
- Clustered embedding space (entities of same type cluster together)

Then runs heavy-duty SPARQL queries including:
- Full table scans
- Multi-hop traversals
- Vector similarity searches
- "Wormhole" traversals (graph → vector → graph)
"""

import requests
import json
import time
import random
import math
import sys
import io

if hasattr(sys.stdout, 'buffer'):
    sys.stdout.reconfigure(encoding='utf-8', errors='replace')

ENDPOINT = "http://localhost:3030"
DIMENSIONS = 128  # Use 128-dim for faster stress test (not 1024)
NUM_ENTITIES = 100_000
NUM_EMBEDDINGS_TARGET = 1_000_000
EMBEDDINGS_PER_ENTITY = 10  # label + 9 aliases = 10 per entity

# Entity types and their cluster centers in embedding space
ENTITY_TYPES = [
    ("Person", [1.0, 0.0, 0.0, 0.0]),
    ("Mountain", [0.0, 1.0, 0.0, 0.0]),
    ("City", [0.0, 0.0, 1.0, 0.0]),
    ("Country", [0.0, 0.0, 0.0, 1.0]),
    ("River", [0.7, 0.7, 0.0, 0.0]),
    ("University", [0.0, 0.7, 0.7, 0.0]),
    ("Company", [0.0, 0.0, 0.7, 0.7]),
    ("Language", [0.7, 0.0, 0.0, 0.7]),
    ("Species", [0.5, 0.5, 0.5, 0.0]),
    ("Book", [0.0, 0.5, 0.5, 0.5]),
]

# Predicates for relationships between types
RELATIONSHIPS = [
    ("Person", "livesIn", "City"),
    ("Person", "bornIn", "Country"),
    ("Person", "speaks", "Language"),
    ("Person", "worksAt", "University"),
    ("Person", "worksAt", "Company"),
    ("Person", "wrote", "Book"),
    ("Person", "knows", "Person"),
    ("Mountain", "locatedIn", "Country"),
    ("City", "locatedIn", "Country"),
    ("River", "flowsThrough", "Country"),
    ("River", "flowsThrough", "City"),
    ("University", "locatedIn", "City"),
    ("Company", "locatedIn", "City"),
    ("Company", "locatedIn", "Country"),
    ("Book", "writtenIn", "Language"),
]

NS = "http://stress.sutradb.dev/"


def health_check():
    try:
        r = requests.get(f"{ENDPOINT}/health", timeout=5)
        return r.status_code == 200
    except:
        return False


def generate_vector(type_idx, entity_idx, variation=0):
    """Generate a clustered vector for an entity of a given type."""
    random.seed(type_idx * 1000000 + entity_idx * 100 + variation)
    center = ENTITY_TYPES[type_idx][1]

    # Extend center to full dimensions with type-based pattern
    vec = []
    for d in range(DIMENSIONS):
        base = center[d % len(center)]
        noise = random.gauss(0, 0.15)
        variation_offset = random.gauss(0, 0.05) * (variation + 1)
        vec.append(base + noise + variation_offset)

    # Normalize
    norm = math.sqrt(sum(v * v for v in vec))
    if norm > 0:
        vec = [v / norm for v in vec]
    return vec


def generate_and_load_data():
    """Generate synthetic data and load into SutraDB."""
    session = requests.Session()

    print("=" * 60)
    print("SUTRADB STRESS TEST — 1M EMBEDDINGS")
    print("=" * 60)

    # Step 1: Declare vector predicate
    print("\n[1/4] Declaring vector predicate...")
    r = session.post(f"{ENDPOINT}/vectors/declare", json={
        "predicate": f"{NS}hasEmbedding",
        "dimensions": DIMENSIONS,
        "m": 16,
        "ef_construction": 100,
        "metric": "cosine",
    })
    if r.status_code == 200:
        print(f"  Declared: {DIMENSIONS}-dim, M=16, ef_construction=100")
    else:
        print(f"  Already declared (or error: {r.status_code}), continuing...")

    # Step 2: Generate and insert triples
    print(f"\n[2/4] Generating triples for {NUM_ENTITIES:,} entities...")
    entities_per_type = NUM_ENTITIES // len(ENTITY_TYPES)

    # Build entity registry
    entity_registry = {}  # type_name -> list of entity IRIs
    for type_idx, (type_name, _) in enumerate(ENTITY_TYPES):
        entity_registry[type_name] = []
        for i in range(entities_per_type):
            iri = f"{NS}{type_name.lower()}/{i}"
            entity_registry[type_name].append(iri)

    # Generate type assertion triples
    total_triples = 0
    batch_lines = []
    batch_size = 10000

    def flush_batch():
        nonlocal total_triples
        if not batch_lines:
            return
        body = "\n".join(batch_lines)
        r = session.post(f"{ENDPOINT}/triples", data=body,
                         headers={"Content-Type": "application/n-triples"}, timeout=120)
        r.raise_for_status()
        result = r.json()
        total_triples += result.get("inserted", 0)
        batch_lines.clear()

    # Type assertions: entity rdf:type Type
    print("  Inserting type assertions...")
    for type_name, entities in entity_registry.items():
        type_iri = f"{NS}type/{type_name}"
        for iri in entities:
            batch_lines.append(f'<{iri}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <{type_iri}> .')
            if len(batch_lines) >= batch_size:
                flush_batch()
    flush_batch()
    print(f"  Type assertions: {total_triples:,}")

    # Relationship triples
    print("  Inserting relationship triples...")
    random.seed(42)
    rel_count_before = total_triples
    for subj_type, pred_name, obj_type in RELATIONSHIPS:
        pred_iri = f"{NS}pred/{pred_name}"
        subj_entities = entity_registry[subj_type]
        obj_entities = entity_registry[obj_type]

        # Each entity gets 1-3 relationships of each type
        for subj in subj_entities:
            num_rels = random.randint(1, 3)
            for _ in range(num_rels):
                obj = random.choice(obj_entities)
                batch_lines.append(f'<{subj}> <{pred_iri}> <{obj}> .')
                if len(batch_lines) >= batch_size:
                    flush_batch()
    flush_batch()
    print(f"  Relationship triples: {total_triples - rel_count_before:,}")

    # Label triples
    print("  Inserting label triples...")
    label_count_before = total_triples
    for type_name, entities in entity_registry.items():
        for i, iri in enumerate(entities):
            label = f'"{type_name} #{i}"'
            batch_lines.append(f'<{iri}> <{NS}pred/label> {label} .')
            if len(batch_lines) >= batch_size:
                flush_batch()
    flush_batch()
    print(f"  Label triples: {total_triples - label_count_before:,}")

    print(f"  TOTAL TRIPLES: {total_triples:,}")

    # Step 3: Insert embeddings
    print(f"\n[3/4] Inserting {NUM_EMBEDDINGS_TARGET:,} embeddings ({DIMENSIONS}-dim)...")
    vec_start = time.time()
    vec_count = 0
    vec_errors = 0

    for type_idx, (type_name, _) in enumerate(ENTITY_TYPES):
        entities = entity_registry[type_name]
        for entity_idx, iri in enumerate(entities):
            for variation in range(EMBEDDINGS_PER_ENTITY):
                # Use a unique subject IRI per embedding variation
                if variation == 0:
                    vec_iri = iri
                else:
                    vec_iri = f"{iri}/alias/{variation}"

                vec = generate_vector(type_idx, entity_idx, variation)

                try:
                    r = session.post(f"{ENDPOINT}/vectors", json={
                        "predicate": f"{NS}hasEmbedding",
                        "subject": vec_iri,
                        "vector": vec,
                    }, timeout=30)
                    r.raise_for_status()
                    vec_count += 1
                except Exception as e:
                    vec_errors += 1

                if vec_count % 10000 == 0:
                    elapsed = time.time() - vec_start
                    rate = vec_count / elapsed if elapsed > 0 else 0
                    print(f"  {vec_count:,}/{NUM_EMBEDDINGS_TARGET:,} vectors ({rate:.0f}/sec, errors: {vec_errors})")

    vec_elapsed = time.time() - vec_start
    print(f"  TOTAL VECTORS: {vec_count:,} in {vec_elapsed:.1f}s ({vec_count/vec_elapsed:.0f}/sec)")
    if vec_errors:
        print(f"  Errors: {vec_errors}")

    print(f"\n[4/4] Data generation complete.")
    print(f"  Triples: {total_triples:,}")
    print(f"  Vectors: {vec_count:,}")

    return total_triples, vec_count


def run_queries():
    """Run heavy-duty SPARQL queries and report performance."""
    session = requests.Session()

    print("\n" + "=" * 60)
    print("QUERY STRESS TESTS")
    print("=" * 60)

    results = []

    def run_query(name, query, expect_rows=None):
        print(f"\n--- {name} ---")
        print(f"  Query: {query[:120]}...")
        start = time.time()
        try:
            r = session.get(f"{ENDPOINT}/sparql", params={"query": query}, timeout=300)
            elapsed = time.time() - start
            if r.status_code != 200:
                print(f"  ERROR: HTTP {r.status_code}: {r.text[:200]}")
                results.append({"name": name, "status": "ERROR", "time": elapsed, "rows": 0})
                return
            data = r.json()
            rows = len(data["results"]["bindings"])
            print(f"  Result: {rows:,} rows in {elapsed:.3f}s")
            if expect_rows is not None and rows != expect_rows:
                print(f"  WARNING: expected {expect_rows} rows")
            results.append({"name": name, "status": "OK", "time": elapsed, "rows": rows})

            # Show first 3 results
            for b in data["results"]["bindings"][:3]:
                vals = {k: v.get("value", "?")[:60] for k, v in b.items()}
                print(f"    {vals}")
        except Exception as e:
            elapsed = time.time() - start
            print(f"  ERROR: {e}")
            results.append({"name": name, "status": "ERROR", "time": elapsed, "rows": 0})

    # ── Query 1: Full scan count ──
    run_query(
        "Full table scan (all triples)",
        "SELECT * WHERE { ?s ?p ?o } LIMIT 100"
    )

    # ── Query 2: Type-specific scan ──
    run_query(
        "All Person entities",
        f"SELECT ?person WHERE {{ ?person <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <{NS}type/Person> }} LIMIT 100"
    )

    # ── Query 3: 2-hop traversal ──
    run_query(
        "2-hop: Person → livesIn → City → locatedIn → Country",
        f"SELECT ?person ?city ?country WHERE {{ ?person <{NS}pred/livesIn> ?city . ?city <{NS}pred/locatedIn> ?country }} LIMIT 50"
    )

    # ── Query 4: 3-hop traversal ──
    run_query(
        "3-hop: Person → worksAt → Company → locatedIn → City → locatedIn → Country",
        f"SELECT ?person ?company ?city ?country WHERE {{ ?person <{NS}pred/worksAt> ?company . ?company <{NS}pred/locatedIn> ?city . ?city <{NS}pred/locatedIn> ?country }} LIMIT 50"
    )

    # ── Query 5: Vector similarity (cold) ──
    # Generate a query vector in the "Person" cluster
    query_vec = generate_vector(0, 0, 0)
    vec_str = " ".join(f"{v:.6f}" for v in query_vec)
    run_query(
        "VECTOR_SIMILAR: Find entities similar to Person#0",
        f'SELECT ?entity WHERE {{ VECTOR_SIMILAR(?entity <{NS}hasEmbedding> "{vec_str}"^^<http://sutra.dev/f32vec>, 0.7) }} LIMIT 20'
    )

    # ── Query 6: Vector similarity in Mountain cluster ──
    query_vec = generate_vector(1, 50, 0)
    vec_str = " ".join(f"{v:.6f}" for v in query_vec)
    run_query(
        "VECTOR_SIMILAR: Find entities similar to Mountain#50",
        f'SELECT ?entity WHERE {{ VECTOR_SIMILAR(?entity <{NS}hasEmbedding> "{vec_str}"^^<http://sutra.dev/f32vec>, 0.7) }} LIMIT 20'
    )

    # ── Query 7: WORMHOLE — Graph → Vector → Graph ──
    # Find people, then vector-search for similar entities, then check what type those are
    query_vec = generate_vector(0, 500, 0)
    vec_str = " ".join(f"{v:.6f}" for v in query_vec)
    run_query(
        "WORMHOLE: Graph→Vector→Graph (type→vectorSearch→type check)",
        f'SELECT ?entity ?type WHERE {{ VECTOR_SIMILAR(?entity <{NS}hasEmbedding> "{vec_str}"^^<http://sutra.dev/f32vec>, 0.8) . ?entity <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> ?type }} LIMIT 20'
    )

    # ── Query 8: WORMHOLE — Start in vector space, traverse graph ──
    query_vec = generate_vector(2, 100, 0)  # City cluster
    vec_str = " ".join(f"{v:.6f}" for v in query_vec)
    run_query(
        "WORMHOLE: Vector→Graph (vectorSearch→locatedIn→country)",
        f'SELECT ?city ?country WHERE {{ VECTOR_SIMILAR(?city <{NS}hasEmbedding> "{vec_str}"^^<http://sutra.dev/f32vec>, 0.85) . ?city <{NS}pred/locatedIn> ?country }} LIMIT 20'
    )

    # ── Query 9: WORMHOLE — Graph → Vector → Graph → Graph ──
    query_vec = generate_vector(0, 0, 0)  # Person cluster
    vec_str = " ".join(f"{v:.6f}" for v in query_vec)
    run_query(
        "WORMHOLE: Graph→Vector→Graph→Graph (type+vector→livesIn→locatedIn)",
        f'SELECT ?person ?city ?country WHERE {{ ?person <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <{NS}type/Person> . VECTOR_SIMILAR(?person <{NS}hasEmbedding> "{vec_str}"^^<http://sutra.dev/f32vec>, 0.8) . ?person <{NS}pred/livesIn> ?city . ?city <{NS}pred/locatedIn> ?country }} LIMIT 20'
    )

    # ── Query 10: UNION + Vector ──
    query_vec = generate_vector(0, 0, 0)
    vec_str = " ".join(f"{v:.6f}" for v in query_vec)
    run_query(
        "UNION + VECTOR: People who wrote books OR work at universities, similar to query",
        f'SELECT ?person WHERE {{ {{ ?person <{NS}pred/wrote> ?book }} UNION {{ ?person <{NS}pred/worksAt> ?uni . ?uni <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <{NS}type/University> }} . VECTOR_SIMILAR(?person <{NS}hasEmbedding> "{vec_str}"^^<http://sutra.dev/f32vec>, 0.7) }} LIMIT 20'
    )

    # ── Query 11: FILTER + Vector ──
    run_query(
        "DISTINCT entities by type",
        f"SELECT DISTINCT ?type WHERE {{ ?entity <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> ?type }} LIMIT 20"
    )

    # ── Query 12: Large result set ──
    run_query(
        "Large result: all Person→knows→Person relationships",
        f"SELECT ?a ?b WHERE {{ ?a <{NS}pred/knows> ?b }} LIMIT 1000"
    )

    # ── Summary ──
    print("\n" + "=" * 60)
    print("RESULTS SUMMARY")
    print("=" * 60)
    print(f"{'Query':<65} {'Status':<8} {'Time':>8} {'Rows':>8}")
    print("-" * 95)
    for r in results:
        print(f"{r['name'][:65]:<65} {r['status']:<8} {r['time']:>7.3f}s {r['rows']:>8,}")

    total_time = sum(r["time"] for r in results)
    ok_count = sum(1 for r in results if r["status"] == "OK")
    print("-" * 95)
    print(f"Total: {ok_count}/{len(results)} queries succeeded, {total_time:.3f}s total query time")

    return results


if __name__ == "__main__":
    if not health_check():
        print("ERROR: SutraDB not running at", ENDPOINT)
        sys.exit(1)

    total_triples, vec_count = generate_and_load_data()
    query_results = run_queries()

    # Write report
    report = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "data": {
            "entities": NUM_ENTITIES,
            "triples": total_triples,
            "vectors": vec_count,
            "dimensions": DIMENSIONS,
            "entity_types": len(ENTITY_TYPES),
        },
        "queries": query_results,
    }
    with open("stress_test_report.json", "w") as f:
        json.dump(report, f, indent=2)
    print(f"\nReport saved to stress_test_report.json")
