"""SutraDB Benchmark Suite.

Runs performance tests against a live SutraDB instance and saves results.

Usage:
    python tools/benchmark.py [--endpoint http://localhost:3030]
"""

import argparse
import io
import json
import sys
import time

import requests

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8")

ENDPOINT = "http://localhost:3030"
SESSION = requests.Session()


def timed_query(query, label, runs=3):
    """Run a SPARQL query multiple times and return timing stats."""
    times = []
    result_count = 0
    for _ in range(runs):
        start = time.perf_counter()
        resp = SESSION.post(f"{ENDPOINT}/sparql", data=query, timeout=60)
        elapsed = time.perf_counter() - start
        times.append(elapsed)
        if resp.status_code == 200:
            data = resp.json()
            result_count = len(data.get("results", {}).get("bindings", []))

    avg = sum(times) / len(times)
    best = min(times)
    worst = max(times)
    print(f"  {label}: {avg*1000:.1f}ms avg ({best*1000:.1f}-{worst*1000:.1f}ms) [{result_count} rows]")
    return {
        "label": label,
        "query": query[:100],
        "avg_ms": round(avg * 1000, 1),
        "best_ms": round(best * 1000, 1),
        "worst_ms": round(worst * 1000, 1),
        "result_count": result_count,
        "runs": runs,
    }


def timed_request(method, path, label, data=None, headers=None, runs=3):
    """Time an HTTP request."""
    times = []
    for _ in range(runs):
        start = time.perf_counter()
        if method == "GET":
            resp = SESSION.get(f"{ENDPOINT}{path}", timeout=60)
        else:
            resp = SESSION.post(f"{ENDPOINT}{path}", data=data, headers=headers, timeout=60)
        elapsed = time.perf_counter() - start
        times.append(elapsed)

    avg = sum(times) / len(times)
    best = min(times)
    print(f"  {label}: {avg*1000:.1f}ms avg ({best*1000:.1f}ms best)")
    return {
        "label": label,
        "avg_ms": round(avg * 1000, 1),
        "best_ms": round(best * 1000, 1),
        "runs": runs,
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--endpoint", default="http://localhost:3030")
    args = parser.parse_args()
    global ENDPOINT
    ENDPOINT = args.endpoint

    print(f"=== SutraDB Benchmark Suite ===")
    print(f"Endpoint: {ENDPOINT}")
    print()

    # Warmup: establish connection and prime caches
    SESSION.get(f"{ENDPOINT}/health")
    SESSION.post(f"{ENDPOINT}/sparql", data="SELECT * WHERE { ?s ?p ?o } LIMIT 1")

    results = {"endpoint": ENDPOINT, "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S"), "benchmarks": []}

    # 1. Health check latency
    print("1. Health Check")
    results["benchmarks"].append(timed_request("GET", "/health", "GET /health", runs=10))

    # 2. Simple SELECT queries
    print("\n2. Simple Queries")
    results["benchmarks"].append(timed_query(
        "SELECT * WHERE { ?s ?p ?o } LIMIT 10",
        "SELECT * LIMIT 10",
    ))
    results["benchmarks"].append(timed_query(
        "SELECT * WHERE { ?s ?p ?o } LIMIT 100",
        "SELECT * LIMIT 100",
    ))
    results["benchmarks"].append(timed_query(
        "SELECT * WHERE { ?s ?p ?o } LIMIT 1000",
        "SELECT * LIMIT 1000",
    ))

    # 3. Filtered queries
    print("\n3. Filtered Queries")
    results["benchmarks"].append(timed_query(
        'SELECT ?s ?label WHERE { ?s <http://www.w3.org/2000/01/rdf-schema#label> ?label } LIMIT 50',
        "Labels LIMIT 50",
    ))
    results["benchmarks"].append(timed_query(
        'SELECT ?s WHERE { ?s <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> ?type } LIMIT 100',
        "Type lookup LIMIT 100",
    ))

    # 4. Join queries
    print("\n4. Join Queries")
    results["benchmarks"].append(timed_query(
        'SELECT ?s ?label ?type WHERE { ?s <http://www.w3.org/2000/01/rdf-schema#label> ?label . ?s <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> ?type } LIMIT 50',
        "2-pattern join LIMIT 50",
    ))
    results["benchmarks"].append(timed_query(
        'SELECT ?a ?b WHERE { ?a <http://www.wikidata.org/prop/direct/P31> ?type . ?b <http://www.wikidata.org/prop/direct/P31> ?type } LIMIT 50',
        "Self-join on type LIMIT 50",
    ))

    # 5. Aggregates
    print("\n5. Aggregates")
    results["benchmarks"].append(timed_query(
        'SELECT ?p (COUNT(*) AS ?count) WHERE { ?s ?p ?o } GROUP BY ?p LIMIT 20',
        "COUNT GROUP BY predicate",
    ))

    # 6. ASK query
    print("\n6. ASK Query")
    results["benchmarks"].append(timed_query(
        'ASK WHERE { ?s <http://www.w3.org/2000/01/rdf-schema#label> ?o }',
        "ASK labels exist",
    ))

    # 7. FILTER queries
    print("\n7. FILTER Queries")
    results["benchmarks"].append(timed_query(
        'SELECT ?s ?label WHERE { ?s <http://www.w3.org/2000/01/rdf-schema#label> ?label . FILTER(CONTAINS(?label, "神社")) } LIMIT 20',
        "FILTER CONTAINS shrine",
    ))

    # 8. OPTIONAL
    print("\n8. OPTIONAL")
    results["benchmarks"].append(timed_query(
        'SELECT ?s ?label ?desc WHERE { ?s <http://www.w3.org/2000/01/rdf-schema#label> ?label . OPTIONAL { ?s <http://schema.org/description> ?desc } } LIMIT 50',
        "OPTIONAL description LIMIT 50",
    ))

    # 9. Export formats
    print("\n9. Export Formats")
    results["benchmarks"].append(timed_request("GET", "/graph", "GET /graph (Turtle export)"))
    results["benchmarks"].append(timed_request("GET", "/graph?format=nt", "GET /graph (N-Triples)"))
    results["benchmarks"].append(timed_request(
        "POST", "/sparql.csv",
        "POST /sparql.csv",
        data="SELECT * WHERE { ?s ?p ?o } LIMIT 100",
    ))

    # 10. Vector health
    print("\n10. Vector Health")
    results["benchmarks"].append(timed_request("GET", "/vectors/health", "GET /vectors/health"))

    # 11. Service description
    print("\n11. Service Description")
    results["benchmarks"].append(timed_request("GET", "/service-description", "GET /service-description"))

    # 12. Insert + Delete roundtrip
    print("\n12. Insert/Delete Roundtrip")
    start = time.perf_counter()
    SESSION.post(f"{ENDPOINT}/sparql",
        data='INSERT DATA { <http://bench/test1> <http://bench/p> <http://bench/o> }')
    insert_time = time.perf_counter() - start
    start = time.perf_counter()
    SESSION.post(f"{ENDPOINT}/sparql",
        data='DELETE DATA { <http://bench/test1> <http://bench/p> <http://bench/o> }')
    delete_time = time.perf_counter() - start
    print(f"  INSERT DATA: {insert_time*1000:.1f}ms")
    print(f"  DELETE DATA: {delete_time*1000:.1f}ms")
    results["benchmarks"].append({"label": "INSERT DATA", "avg_ms": round(insert_time * 1000, 1)})
    results["benchmarks"].append({"label": "DELETE DATA", "avg_ms": round(delete_time * 1000, 1)})

    # Summary
    print(f"\n=== Summary ===")
    print(f"Total benchmarks: {len(results['benchmarks'])}")
    fastest = min(results["benchmarks"], key=lambda x: x.get("avg_ms", 999))
    slowest = max(results["benchmarks"], key=lambda x: x.get("avg_ms", 0))
    print(f"Fastest: {fastest['label']} ({fastest.get('avg_ms', '?')}ms)")
    print(f"Slowest: {slowest['label']} ({slowest.get('avg_ms', '?')}ms)")

    # Save results
    with open("benchmark_results.json", "w") as f:
        json.dump(results, f, indent=2)
    print(f"\nResults saved to benchmark_results.json")


if __name__ == "__main__":
    main()
