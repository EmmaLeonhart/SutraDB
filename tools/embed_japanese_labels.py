"""Generate embeddings for Japanese labels in SutraDB.

Fetches all entities with Japanese rdfs:label values, generates embeddings
via Ollama (mxbai-embed-large), and inserts them as a dedicated vector
predicate (sutra:jaEmbedding) for Japanese-specific similarity search.

Usage:
    python tools/embed_japanese_labels.py
"""

import io
import json
import sys
import time

import requests

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8")

SUTRA = "http://localhost:3030"
OLLAMA = "http://localhost:11434"
MODEL = "mxbai-embed-large"
VECTOR_PRED = "http://sutra.dev/jaEmbedding"
DIMENSIONS = 1024


def get_japanese_labels():
    """Fetch all entities with Japanese labels."""
    # Fetch all labels, filter for @ja in Python since LANGMATCHES may not work via HTTP
    resp = requests.post(
        f"{SUTRA}/sparql",
        data="SELECT ?s ?label WHERE { ?s <http://www.w3.org/2000/01/rdf-schema#label> ?label } LIMIT 5000",
    )
    results = resp.json()["results"]["bindings"]

    ja_labels = {}
    for row in results:
        entity = row["s"]["value"]
        label = row["label"]["value"]
        # Labels come back as: 延喜式神名帳"@ja (with trailing quote+lang)
        if label.endswith('"@ja'):
            clean = label[:-4]  # strip "@ja
            if clean.endswith('"'):
                clean = clean[:-1]
            ja_labels[entity] = clean
        elif "@ja" in label:
            # Fallback: strip everything after @ja
            idx = label.index("@ja")
            clean = label[:idx].rstrip('"')
            ja_labels[entity] = clean

    return ja_labels


def get_embedding(text):
    """Get embedding from Ollama."""
    try:
        resp = requests.post(
            f"{OLLAMA}/api/embeddings",
            json={"model": MODEL, "prompt": text},
            timeout=60,
        )
        if resp.status_code == 200:
            return resp.json().get("embedding")
    except Exception as e:
        print(f"  [WARN] Embedding failed: {e}")
    return None


def declare_vector_predicate():
    """Declare the Japanese embedding vector predicate."""
    resp = requests.post(
        f"{SUTRA}/vectors/declare",
        json={
            "predicate": VECTOR_PRED,
            "dimensions": DIMENSIONS,
            "m": 16,
            "ef_construction": 200,
            "metric": "cosine",
        },
    )
    return resp.status_code == 200


def insert_vector(subject, vector):
    """Insert a vector embedding."""
    requests.post(
        f"{SUTRA}/vectors",
        json={
            "predicate": VECTOR_PRED,
            "subject": subject,
            "vector": vector,
        },
    )


def main():
    print("=== Japanese Label Embeddings ===\n")

    # Check services
    try:
        r = requests.get(f"{SUTRA}/health", timeout=5)
        assert r.status_code == 200
        print("[OK] SutraDB running")
    except Exception:
        print("[ERROR] SutraDB not running at", SUTRA)
        sys.exit(1)

    # Declare vector predicate
    declare_vector_predicate()
    print(f"[OK] Vector predicate declared: {VECTOR_PRED}")

    # Fetch Japanese labels
    ja_labels = get_japanese_labels()
    print(f"[OK] Found {len(ja_labels)} entities with Japanese labels\n")

    if not ja_labels:
        print("No Japanese labels found. Nothing to do.")
        return

    # Generate and insert embeddings
    done = 0
    errors = 0
    start = time.time()

    for entity, label in ja_labels.items():
        done += 1
        elapsed = time.time() - start
        rate = done / max(elapsed, 1)
        print(f"[{done:>4}/{len(ja_labels)}] {label}  ({entity.split('/')[-1]}, {rate:.1f}/s)")

        embedding = get_embedding(label)
        if embedding and len(embedding) == DIMENSIONS:
            insert_vector(entity, embedding)
        else:
            errors += 1
            print(f"  [WARN] Bad embedding for {label}")

    elapsed = time.time() - start
    print(f"\n=== Done ===")
    print(f"Embedded: {done - errors}/{len(ja_labels)}")
    print(f"Errors:   {errors}")
    print(f"Time:     {elapsed:.1f}s")
    print(f"Rate:     {(done - errors) / max(elapsed, 1):.1f} embeddings/s")


if __name__ == "__main__":
    main()
