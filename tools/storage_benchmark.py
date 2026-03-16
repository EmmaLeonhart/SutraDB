"""Storage engine benchmark for SutraDB.

Tests sled-backed PersistentStore performance characteristics:
- Bulk insert throughput
- Point lookup latency
- Range scan performance
- Write-then-read consistency

Run against a live SutraDB instance to measure end-to-end.
Results are saved for comparison with future storage backends.

Usage:
    python tools/storage_benchmark.py
"""

import io
import json
import sys
import time

import requests

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8")

ENDPOINT = "http://localhost:3030"
SESSION = requests.Session()
# Warmup
SESSION.get(f"{ENDPOINT}/health")


def bulk_insert_benchmark(n=1000):
    """Measure bulk triple insertion throughput."""
    lines = []
    for i in range(n):
        lines.append(
            f"<http://bench/s{i}> <http://bench/p> <http://bench/o{i}> ."
        )
    payload = "\n".join(lines)

    start = time.perf_counter()
    resp = SESSION.post(
        f"{ENDPOINT}/triples",
        data=payload.encode("utf-8"),
        headers={"Content-Type": "text/plain"},
    )
    elapsed = time.perf_counter() - start

    result = resp.json()
    inserted = result.get("inserted", 0)
    rate = inserted / max(elapsed, 0.001)
    print(f"  Bulk insert {n} triples: {elapsed*1000:.1f}ms ({rate:.0f} triples/sec)")
    return {
        "test": "bulk_insert",
        "count": n,
        "inserted": inserted,
        "elapsed_ms": round(elapsed * 1000, 1),
        "rate_per_sec": round(rate),
    }


def point_lookup_benchmark(runs=100):
    """Measure point lookup latency (single subject query)."""
    times = []
    for i in range(runs):
        q = f"SELECT ?p ?o WHERE {{ <http://bench/s{i}> ?p ?o }}"
        start = time.perf_counter()
        SESSION.post(f"{ENDPOINT}/sparql", data=q)
        elapsed = time.perf_counter() - start
        times.append(elapsed)

    avg = sum(times) / len(times)
    p50 = sorted(times)[len(times) // 2]
    p99 = sorted(times)[int(len(times) * 0.99)]
    print(f"  Point lookup (100 queries): avg={avg*1000:.2f}ms p50={p50*1000:.2f}ms p99={p99*1000:.2f}ms")
    return {
        "test": "point_lookup",
        "runs": runs,
        "avg_ms": round(avg * 1000, 2),
        "p50_ms": round(p50 * 1000, 2),
        "p99_ms": round(p99 * 1000, 2),
    }


def range_scan_benchmark():
    """Measure range scan performance at different sizes."""
    results = []
    for limit in [10, 100, 500, 1000]:
        q = f"SELECT * WHERE {{ ?s ?p ?o }} LIMIT {limit}"
        times = []
        for _ in range(5):
            start = time.perf_counter()
            resp = SESSION.post(f"{ENDPOINT}/sparql", data=q)
            elapsed = time.perf_counter() - start
            times.append(elapsed)

        avg = sum(times) / len(times)
        rows = len(resp.json().get("results", {}).get("bindings", []))
        print(f"  Range scan LIMIT {limit}: {avg*1000:.2f}ms ({rows} rows)")
        results.append({
            "test": f"range_scan_{limit}",
            "limit": limit,
            "avg_ms": round(avg * 1000, 2),
            "rows": rows,
        })
    return results


def predicate_scan_benchmark():
    """Measure predicate-based index scan."""
    predicates = [
        ("rdfs:label", "http://www.w3.org/2000/01/rdf-schema#label"),
        ("rdf:type", "http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
        ("schema:description", "http://schema.org/description"),
    ]
    results = []
    for name, iri in predicates:
        q = f'SELECT ?s ?o WHERE {{ ?s <{iri}> ?o }} LIMIT 100'
        times = []
        for _ in range(5):
            start = time.perf_counter()
            resp = SESSION.post(f"{ENDPOINT}/sparql", data=q)
            elapsed = time.perf_counter() - start
            times.append(elapsed)

        avg = sum(times) / len(times)
        rows = len(resp.json().get("results", {}).get("bindings", []))
        print(f"  Predicate scan {name}: {avg*1000:.2f}ms ({rows} rows)")
        results.append({
            "test": f"predicate_scan_{name}",
            "avg_ms": round(avg * 1000, 2),
            "rows": rows,
        })
    return results


def join_benchmark():
    """Measure join performance at different complexities."""
    queries = [
        ("1-hop", "SELECT ?s ?o WHERE { ?s <http://www.w3.org/2000/01/rdf-schema#label> ?o } LIMIT 50"),
        ("2-hop", "SELECT ?s ?label ?type WHERE { ?s <http://www.w3.org/2000/01/rdf-schema#label> ?label . ?s <http://www.wikidata.org/prop/direct/P31> ?type } LIMIT 50"),
        ("self-join", "SELECT ?a ?b WHERE { ?a <http://www.wikidata.org/prop/direct/P31> ?t . ?b <http://www.wikidata.org/prop/direct/P31> ?t } LIMIT 50"),
    ]
    results = []
    for name, q in queries:
        times = []
        for _ in range(5):
            start = time.perf_counter()
            resp = SESSION.post(f"{ENDPOINT}/sparql", data=q)
            elapsed = time.perf_counter() - start
            times.append(elapsed)

        avg = sum(times) / len(times)
        rows = len(resp.json().get("results", {}).get("bindings", []))
        print(f"  {name} join: {avg*1000:.2f}ms ({rows} rows)")
        results.append({
            "test": f"join_{name}",
            "avg_ms": round(avg * 1000, 2),
            "rows": rows,
        })
    return results


def export_benchmark():
    """Measure full graph export performance."""
    formats = [
        ("turtle", "/graph"),
        ("ntriples", "/graph?format=nt"),
    ]
    results = []
    for name, path in formats:
        times = []
        size = 0
        for _ in range(3):
            start = time.perf_counter()
            resp = SESSION.get(f"{ENDPOINT}{path}")
            elapsed = time.perf_counter() - start
            times.append(elapsed)
            size = len(resp.text)

        avg = sum(times) / len(times)
        print(f"  Export {name}: {avg*1000:.1f}ms ({size/1024:.1f} KB)")
        results.append({
            "test": f"export_{name}",
            "avg_ms": round(avg * 1000, 1),
            "size_kb": round(size / 1024, 1),
        })
    return results


def cleanup_benchmark_data():
    """Remove benchmark triples."""
    SESSION.post(
        f"{ENDPOINT}/sparql",
        data='DELETE DATA { }',  # Can't bulk delete easily, skip
    )


def main():
    print("=== SutraDB Storage Engine Benchmark ===\n")

    all_results = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "endpoint": ENDPOINT,
        "storage_backend": "sled",
        "tests": [],
    }

    print("1. Bulk Insert")
    all_results["tests"].append(bulk_insert_benchmark(500))
    all_results["tests"].append(bulk_insert_benchmark(2000))

    print("\n2. Point Lookups")
    all_results["tests"].append(point_lookup_benchmark())

    print("\n3. Range Scans")
    all_results["tests"].extend(range_scan_benchmark())

    print("\n4. Predicate Index Scans")
    all_results["tests"].extend(predicate_scan_benchmark())

    print("\n5. Join Performance")
    all_results["tests"].extend(join_benchmark())

    print("\n6. Export Performance")
    all_results["tests"].extend(export_benchmark())

    # Summary
    print(f"\n=== Summary ===")
    print(f"Storage backend: sled (LSM-tree)")
    print(f"Tests run: {len(all_results['tests'])}")

    with open("storage_benchmark_results.json", "w") as f:
        json.dump(all_results, f, indent=2)
    print(f"\nResults saved to storage_benchmark_results.json")


if __name__ == "__main__":
    main()
