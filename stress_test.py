"""
SutraDB Stress Test v2 — Proper data model.

All data is triples. Entities connect to each other via relationship triples
(predicate "1") and ~50% of entities also have vector triples (predicate "2")
where the object is a vector literal. Vectors don't exist outside of triples.

This tests the real architecture: graph traversal, vector search, and
wormhole queries that hop between graph and vector space.
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
DIMENSIONS = 64
NUM_ENTITIES = 50_000
VECTOR_FRACTION = 0.5  # 50% of entities get vectors

NS = "http://test.sutradb.dev/"
PRED_LINK = f"{NS}link"        # predicate "1" — entity-to-entity relationships
PRED_TYPE = f"{NS}type"        # rdf:type equivalent
PRED_VEC = f"{NS}hasEmbedding" # predicate "2" — entity-to-vector

# 5 entity types with cluster centers in vector space
TYPES = [
    ("alpha", [1.0, 0.0, 0.0, 0.0]),
    ("beta",  [0.0, 1.0, 0.0, 0.0]),
    ("gamma", [0.0, 0.0, 1.0, 0.0]),
    ("delta", [0.0, 0.0, 0.0, 1.0]),
    ("omega", [0.5, 0.5, 0.5, 0.5]),
]


def health_check():
    try:
        return requests.get(f"{ENDPOINT}/health", timeout=5).status_code == 200
    except:
        return False


def gen_vector(type_idx, entity_idx):
    """Generate a clustered vector. Same type = similar vectors."""
    random.seed(type_idx * 100000 + entity_idx)
    center = TYPES[type_idx][1]
    vec = []
    for d in range(DIMENSIONS):
        base = center[d % len(center)]
        noise = random.gauss(0, 0.1)
        vec.append(base + noise)
    norm = math.sqrt(sum(v * v for v in vec))
    return [v / norm for v in vec] if norm > 0 else vec


def generate_and_load():
    session = requests.Session()
    print("=" * 60)
    print("SUTRADB STRESS TEST v2")
    print(f"Entities: {NUM_ENTITIES:,}, Dimensions: {DIMENSIONS}")
    print(f"Vector fraction: {VECTOR_FRACTION:.0%}")
    print("=" * 60)

    # Step 1: Declare vector predicate
    print("\n[1/3] Declaring vector predicate...")
    r = session.post(f"{ENDPOINT}/vectors/declare", json={
        "predicate": PRED_VEC,
        "dimensions": DIMENSIONS,
        "m": 16,
        "ef_construction": 100,
        "metric": "cosine",
    })
    if r.status_code == 200:
        print(f"  Declared: {DIMENSIONS}-dim cosine")
    else:
        print(f"  Already declared or error ({r.status_code}), continuing...")

    # Step 2: Generate and insert triples
    print(f"\n[2/3] Generating triples...")
    entities_per_type = NUM_ENTITIES // len(TYPES)
    random.seed(42)

    # Build entity list
    entities = []
    for type_idx, (type_name, _) in enumerate(TYPES):
        for i in range(entities_per_type):
            entities.append({
                "iri": f"{NS}entity/{type_name}/{i}",
                "type_idx": type_idx,
                "type_name": type_name,
                "entity_idx": i,
                "has_vector": random.random() < VECTOR_FRACTION,
            })

    total_triples = 0
    batch_lines = []
    batch_size = 10000

    def flush():
        nonlocal total_triples
        if not batch_lines:
            return
        body = "\n".join(batch_lines)
        r = session.post(f"{ENDPOINT}/triples", data=body,
                         headers={"Content-Type": "application/n-triples"}, timeout=120)
        r.raise_for_status()
        total_triples += r.json().get("inserted", 0)
        batch_lines.clear()

    # Type triples: <entity> <type> <type_class>
    print("  Type triples...")
    for e in entities:
        type_iri = f"{NS}class/{e['type_name']}"
        batch_lines.append(f'<{e["iri"]}> <{PRED_TYPE}> <{type_iri}> .')
        if len(batch_lines) >= batch_size:
            flush()
    flush()
    print(f"    {total_triples:,} type triples")

    # Link triples: <entity> <link> <other_entity> — random connections
    print("  Link triples (random connections)...")
    link_start = total_triples
    for e in entities:
        num_links = random.randint(1, 4)
        for _ in range(num_links):
            target = random.choice(entities)
            if target["iri"] != e["iri"]:
                batch_lines.append(f'<{e["iri"]}> <{PRED_LINK}> <{target["iri"]}> .')
                if len(batch_lines) >= batch_size:
                    flush()
    flush()
    print(f"    {total_triples - link_start:,} link triples")

    print(f"  TOTAL GRAPH TRIPLES: {total_triples:,}")

    # Step 3: Insert vectors (these create triples too via POST /vectors)
    # IMPORTANT: shuffle insertion order so HNSW upper layers have nodes from
    # all clusters, providing cross-cluster bridges. Without shuffling, the
    # entry point and upper layers are biased toward whichever cluster was
    # inserted first, making greedy descent unable to reach other clusters.
    vec_entities = [e for e in entities if e["has_vector"]]
    random.shuffle(vec_entities)

    print(f"\n[3/3] Inserting vectors ({len(vec_entities):,} entities, shuffled)...")
    vec_start = time.time()
    vec_count = 0
    vec_errors = 0

    for e in vec_entities:
        vec = gen_vector(e["type_idx"], e["entity_idx"])
        try:
            r = session.post(f"{ENDPOINT}/vectors", json={
                "predicate": PRED_VEC,
                "subject": e["iri"],
                "vector": vec,
            }, timeout=30)
            r.raise_for_status()
            vec_count += 1
        except Exception as ex:
            vec_errors += 1

        if vec_count % 5000 == 0:
            elapsed = time.time() - vec_start
            rate = vec_count / elapsed if elapsed > 0 else 0
            print(f"    {vec_count:,} vectors ({rate:.0f}/sec, errors: {vec_errors})")

    vec_elapsed = time.time() - vec_start
    print(f"  VECTORS: {vec_count:,} in {vec_elapsed:.1f}s ({vec_count/vec_elapsed:.0f}/sec)")
    print(f"  TOTAL TRIPLES (graph + vector): {total_triples + vec_count:,}")

    return total_triples, vec_count, entities


def run_queries(entities):
    session = requests.Session()
    print("\n" + "=" * 60)
    print("QUERY STRESS TESTS")
    print("=" * 60)

    results = []

    def run(name, query, expect_min=None):
        print(f"\n--- {name} ---")
        q_preview = query.replace('\n', ' ')[:120]
        print(f"  {q_preview}...")
        start = time.time()
        try:
            r = session.get(f"{ENDPOINT}/sparql", params={"query": query}, timeout=300)
            elapsed = time.time() - start
            if r.status_code != 200:
                print(f"  ERROR {r.status_code}: {r.text[:200]}")
                results.append({"name": name, "status": "ERROR", "time": elapsed, "rows": 0})
                return 0
            data = r.json()
            rows = len(data["results"]["bindings"])
            status = "OK" if (expect_min is None or rows >= expect_min) else "LOW"
            print(f"  {rows:,} rows in {elapsed:.3f}s" + (f" (expected >= {expect_min})" if status == "LOW" else ""))
            for b in data["results"]["bindings"][:3]:
                vals = {k: v.get("value", "?")[:50] for k, v in b.items()}
                print(f"    {vals}")
            results.append({"name": name, "status": status, "time": elapsed, "rows": rows})
            return rows
        except Exception as e:
            elapsed = time.time() - start
            print(f"  ERROR: {e}")
            results.append({"name": name, "status": "ERROR", "time": elapsed, "rows": 0})
            return 0

    # ── 1. Basic graph queries ──

    run("Type lookup: all alpha entities (LIMIT 100)",
        f'SELECT ?e WHERE {{ ?e <{PRED_TYPE}> <{NS}class/alpha> }} LIMIT 100',
        expect_min=100)

    run("1-hop: alpha → link → ?target (LIMIT 100)",
        f'SELECT ?src ?tgt WHERE {{ ?src <{PRED_TYPE}> <{NS}class/alpha> . ?src <{PRED_LINK}> ?tgt }} LIMIT 100',
        expect_min=50)

    run("2-hop: alpha → link → ? → link → ? (LIMIT 50)",
        f'SELECT ?a ?b ?c WHERE {{ ?a <{PRED_TYPE}> <{NS}class/alpha> . ?a <{PRED_LINK}> ?b . ?b <{PRED_LINK}> ?c }} LIMIT 50',
        expect_min=10)

    run("All types (DISTINCT)",
        f'SELECT DISTINCT ?type WHERE {{ ?e <{PRED_TYPE}> ?type }}',
        expect_min=5)

    run("Large result: 1000 link triples",
        f'SELECT ?s ?o WHERE {{ ?s <{PRED_LINK}> ?o }} LIMIT 1000',
        expect_min=1000)

    # ── 2. Vector queries ──

    # Pick an alpha entity that has a vector
    alpha_vec_entity = next(e for e in entities if e["type_name"] == "alpha" and e["has_vector"])
    query_vec = gen_vector(alpha_vec_entity["type_idx"], alpha_vec_entity["entity_idx"])
    vec_str = " ".join(f"{v:.6f}" for v in query_vec)

    run("VECTOR_SIMILAR: find entities near alpha vector (threshold 0.7)",
        f'SELECT ?entity WHERE {{ VECTOR_SIMILAR(?entity <{PRED_VEC}> "{vec_str}"^^<http://sutra.dev/f32vec>, 0.7) }} LIMIT 20',
        expect_min=1)

    # Pick a beta entity for cross-cluster search
    beta_vec_entity = next(e for e in entities if e["type_name"] == "beta" and e["has_vector"])
    beta_vec = gen_vector(beta_vec_entity["type_idx"], beta_vec_entity["entity_idx"])
    beta_vec_str = " ".join(f"{v:.6f}" for v in beta_vec)

    run("VECTOR_SIMILAR: find entities near beta vector (threshold 0.7)",
        f'SELECT ?entity WHERE {{ VECTOR_SIMILAR(?entity <{PRED_VEC}> "{beta_vec_str}"^^<http://sutra.dev/f32vec>, 0.7) }} LIMIT 20',
        expect_min=1)

    # ── 3. Wormhole queries: graph ↔ vector ──

    run("WORMHOLE vector→graph: similar to alpha, get their type",
        f'SELECT ?entity ?type WHERE {{ VECTOR_SIMILAR(?entity <{PRED_VEC}> "{vec_str}"^^<http://sutra.dev/f32vec>, 0.7) . ?entity <{PRED_TYPE}> ?type }} LIMIT 20',
        expect_min=1)

    run("WORMHOLE vector→graph→graph: similar to alpha, follow link, get type",
        f'SELECT ?entity ?linked ?type WHERE {{ VECTOR_SIMILAR(?entity <{PRED_VEC}> "{vec_str}"^^<http://sutra.dev/f32vec>, 0.7) . ?entity <{PRED_LINK}> ?linked . ?linked <{PRED_TYPE}> ?type }} LIMIT 20',
        expect_min=1)

    run("WORMHOLE graph→vector: alpha entities filtered by vector similarity",
        f'SELECT ?entity WHERE {{ ?entity <{PRED_TYPE}> <{NS}class/alpha> . VECTOR_SIMILAR(?entity <{PRED_VEC}> "{vec_str}"^^<http://sutra.dev/f32vec>, 0.7) }} LIMIT 20',
        expect_min=1)

    run("WORMHOLE graph→vector→graph: typed entities, vector filter, follow link",
        f'SELECT ?entity ?linked WHERE {{ ?entity <{PRED_TYPE}> <{NS}class/alpha> . VECTOR_SIMILAR(?entity <{PRED_VEC}> "{vec_str}"^^<http://sutra.dev/f32vec>, 0.7) . ?entity <{PRED_LINK}> ?linked }} LIMIT 20',
        expect_min=1)

    # ── 4. UNION + Vector ──

    run("UNION: alpha OR beta entities",
        f'SELECT ?e WHERE {{ {{ ?e <{PRED_TYPE}> <{NS}class/alpha> }} UNION {{ ?e <{PRED_TYPE}> <{NS}class/beta> }} }} LIMIT 100',
        expect_min=50)

    run("UNION + VECTOR: (alpha UNION beta) filtered by vector similarity",
        f'SELECT ?e WHERE {{ {{ ?e <{PRED_TYPE}> <{NS}class/alpha> }} UNION {{ ?e <{PRED_TYPE}> <{NS}class/beta> }} . VECTOR_SIMILAR(?e <{PRED_VEC}> "{vec_str}"^^<http://sutra.dev/f32vec>, 0.6) }} LIMIT 20',
        expect_min=1)

    # ── 5. Cross-cluster vector search ──

    run("Cross-cluster: search with alpha vector, find what types come back",
        f'SELECT ?entity ?type WHERE {{ VECTOR_SIMILAR(?entity <{PRED_VEC}> "{vec_str}"^^<http://sutra.dev/f32vec>, 0.3) . ?entity <{PRED_TYPE}> ?type }} LIMIT 50',
        expect_min=1)

    # ── Summary ──
    print("\n" + "=" * 60)
    print("RESULTS SUMMARY")
    print("=" * 60)
    print(f"{'Query':<70} {'Status':<6} {'Time':>8} {'Rows':>6}")
    print("-" * 96)
    for r in results:
        print(f"{r['name'][:70]:<70} {r['status']:<6} {r['time']:>7.3f}s {r['rows']:>6,}")

    ok = sum(1 for r in results if r["status"] == "OK")
    low = sum(1 for r in results if r["status"] == "LOW")
    err = sum(1 for r in results if r["status"] == "ERROR")
    total_time = sum(r["time"] for r in results)
    print("-" * 96)
    print(f"Total: {ok} OK, {low} LOW, {err} ERROR — {total_time:.3f}s total")

    return results


if __name__ == "__main__":
    if not health_check():
        print("ERROR: SutraDB not running at", ENDPOINT)
        sys.exit(1)

    total_triples, vec_count, entities = generate_and_load()
    query_results = run_queries(entities)

    report = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "version": "v2",
        "data": {
            "entities": NUM_ENTITIES,
            "graph_triples": total_triples,
            "vector_triples": vec_count,
            "total_triples": total_triples + vec_count,
            "dimensions": DIMENSIONS,
            "entity_types": len(TYPES),
            "vector_fraction": VECTOR_FRACTION,
        },
        "queries": query_results,
    }
    with open("stress_test_report.json", "w") as f:
        json.dump(report, f, indent=2)
    print(f"\nReport saved to stress_test_report.json")
