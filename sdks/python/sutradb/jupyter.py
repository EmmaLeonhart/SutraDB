"""Jupyter integration for SutraDB.

Provides %%sparql cell magic for executing SPARQL queries inline in
Jupyter notebooks with tabular result display.

Usage in a Jupyter notebook:

    # First, load the extension
    %load_ext sutradb.jupyter

    # Then use the magic
    %%sparql
    SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 10

    # Or with a custom endpoint
    %%sparql http://localhost:8080
    SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 10
"""

from __future__ import annotations

from IPython.core.magic import register_cell_magic, needs_local_scope
import requests


_default_endpoint = "http://localhost:3030"


def _shorten(iri: str) -> str:
    """Shorten an IRI for display."""
    prefixes = {
        "http://www.w3.org/1999/02/22-rdf-syntax-ns#": "rdf:",
        "http://www.w3.org/2000/01/rdf-schema#": "rdfs:",
        "http://www.w3.org/2002/07/owl#": "owl:",
        "http://www.w3.org/2001/XMLSchema#": "xsd:",
        "http://www.wikidata.org/entity/": "wd:",
        "http://www.wikidata.org/prop/direct/": "wdt:",
        "http://sutra.dev/": "sutra:",
        "http://schema.org/": "schema:",
    }
    for full, short in prefixes.items():
        if iri.startswith(full):
            return short + iri[len(full):]
    return iri


@register_cell_magic
def sparql(line, cell):
    """Execute a SPARQL query against SutraDB.

    Usage:
        %%sparql [endpoint]
        SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 10
    """
    endpoint = line.strip() if line.strip() else _default_endpoint

    try:
        resp = requests.post(
            f"{endpoint}/sparql",
            data=cell,
            headers={"Accept": "application/sparql-results+json"},
            timeout=30,
        )
        if resp.status_code != 200:
            print(f"Error: HTTP {resp.status_code}")
            print(resp.text)
            return

        data = resp.json()
        columns = data.get("head", {}).get("vars", [])
        bindings = data.get("results", {}).get("bindings", [])

        if not bindings:
            print("No results.")
            return

        # Try to use pandas for nice display
        try:
            import pandas as pd

            rows = []
            for b in bindings:
                row = {}
                for col in columns:
                    val = b.get(col, {}).get("value", "")
                    row[col] = _shorten(val)
                rows.append(row)
            df = pd.DataFrame(rows, columns=columns)
            from IPython.display import display

            display(df)
        except ImportError:
            # Fallback: plain text table
            # Header
            widths = {c: max(len(c), 10) for c in columns}
            for b in bindings[:20]:
                for c in columns:
                    v = _shorten(b.get(c, {}).get("value", ""))
                    widths[c] = max(widths[c], min(len(v), 50))

            header = " | ".join(c.ljust(widths[c]) for c in columns)
            separator = "-+-".join("-" * widths[c] for c in columns)
            print(header)
            print(separator)
            for b in bindings:
                row = " | ".join(
                    _shorten(b.get(c, {}).get("value", ""))[:widths[c]].ljust(
                        widths[c]
                    )
                    for c in columns
                )
                print(row)
            print(f"\n{len(bindings)} rows")

    except requests.ConnectionError:
        print(f"Error: Could not connect to SutraDB at {endpoint}")
    except Exception as e:
        print(f"Error: {e}")


def load_ipython_extension(ipython):
    """Called when %load_ext sutradb.jupyter is executed."""
    # The @register_cell_magic decorator handles registration
    pass
