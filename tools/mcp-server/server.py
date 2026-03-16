"""SutraDB MCP Server — Model Context Protocol interface for AI agents.

Exposes SutraDB operations as MCP tools that AI agents (Claude, GPT, etc.)
can call directly. This is the agent↔database bridge.

Tools exposed:
- sparql_query: Execute SPARQL queries
- insert_triples: Insert N-Triples data
- describe_entity: Get all triples about an entity
- vector_search: Find similar entities by embedding
- database_info: Get database statistics
- health_check: Check if SutraDB is running

Usage:
    python tools/mcp-server/server.py

Requires: pip install mcp requests
"""

import io
import json
import sys
from typing import Any

import requests

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8")

SUTRA_ENDPOINT = "http://localhost:3030"


def sutra_request(method: str, path: str, **kwargs) -> dict:
    """Make a request to SutraDB."""
    url = f"{SUTRA_ENDPOINT}{path}"
    resp = requests.request(method, url, timeout=30, **kwargs)
    if resp.status_code >= 400:
        return {"error": f"HTTP {resp.status_code}: {resp.text}"}
    try:
        return resp.json()
    except Exception:
        return {"result": resp.text}


def handle_tool_call(name: str, arguments: dict) -> Any:
    """Route a tool call to the appropriate handler."""

    if name == "sparql_query":
        query = arguments.get("query", "")
        result = sutra_request("POST", "/sparql", data=query)
        # Simplify output for agent consumption
        if "results" in result and "bindings" in result["results"]:
            bindings = result["results"]["bindings"]
            if len(bindings) == 0:
                return "No results."
            # Format as a readable table
            cols = result.get("head", {}).get("vars", [])
            rows = []
            for b in bindings[:50]:  # Limit output for agent context
                row = {c: b.get(c, {}).get("value", "") for c in cols}
                rows.append(row)
            return {"columns": cols, "rows": rows, "total": len(bindings)}
        return result

    elif name == "insert_triples":
        ntriples = arguments.get("ntriples", "")
        return sutra_request(
            "POST", "/triples",
            data=ntriples.encode("utf-8"),
            headers={"Content-Type": "text/plain; charset=utf-8"},
        )

    elif name == "describe_entity":
        entity = arguments.get("entity", "")
        query = f'SELECT ?p ?o WHERE {{ <{entity}> ?p ?o }}'
        result = sutra_request("POST", "/sparql", data=query)
        if "results" in result and "bindings" in result["results"]:
            props = []
            for b in result["results"]["bindings"]:
                p = b.get("p", {}).get("value", "")
                o = b.get("o", {}).get("value", "")
                props.append({"predicate": p, "object": o})
            return {"entity": entity, "properties": props}
        return result

    elif name == "vector_search":
        query_text = arguments.get("query_text", "")
        predicate = arguments.get("predicate", "http://sutra.dev/hasEmbedding")
        limit = arguments.get("limit", 10)
        # First get embedding from Ollama
        try:
            embed_resp = requests.post(
                "http://localhost:11434/api/embeddings",
                json={"model": "mxbai-embed-large", "prompt": query_text},
                timeout=60,
            )
            if embed_resp.status_code != 200:
                return {"error": "Failed to generate embedding"}
            vector = embed_resp.json().get("embedding", [])
        except Exception as e:
            return {"error": f"Embedding error: {e}"}

        vec_str = " ".join(f"{v:.6f}" for v in vector)
        query = (
            f'SELECT ?entity WHERE {{\n'
            f'  VECTOR_SIMILAR(?entity <{predicate}> '
            f'"{vec_str}"^^<http://sutra.dev/f32vec>, 0.5)\n'
            f'}} LIMIT {limit}'
        )
        return sutra_request("POST", "/sparql", data=query)

    elif name == "database_info":
        health = sutra_request("GET", "/health")
        vectors = sutra_request("GET", "/vectors/health")
        # Get a sample of data
        sample = sutra_request(
            "POST", "/sparql",
            data="SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 5",
        )
        return {
            "healthy": health.get("result") == "ok",
            "vectors": vectors,
            "sample_triples": sample.get("results", {}).get("bindings", [])[:5],
        }

    elif name == "health_check":
        try:
            resp = requests.get(f"{SUTRA_ENDPOINT}/health", timeout=5)
            return {"healthy": resp.status_code == 200}
        except Exception:
            return {"healthy": False, "error": "SutraDB not reachable"}

    else:
        return {"error": f"Unknown tool: {name}"}


# ── MCP Protocol (stdio JSON-RPC) ───────────────────────────────────────────

TOOLS = [
    {
        "name": "sparql_query",
        "description": "Execute a SPARQL query against SutraDB. Supports SELECT, ASK, CONSTRUCT, DESCRIBE, INSERT DATA, DELETE DATA. Returns results as JSON.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The SPARQL query to execute",
                }
            },
            "required": ["query"],
        },
    },
    {
        "name": "insert_triples",
        "description": "Insert RDF triples in N-Triples format into SutraDB.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "ntriples": {
                    "type": "string",
                    "description": "N-Triples data (one triple per line, each ending with ' .')",
                }
            },
            "required": ["ntriples"],
        },
    },
    {
        "name": "describe_entity",
        "description": "Get all properties and values for a given entity IRI.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "entity": {
                    "type": "string",
                    "description": "The full IRI of the entity to describe",
                }
            },
            "required": ["entity"],
        },
    },
    {
        "name": "vector_search",
        "description": "Find entities similar to a text query using vector embeddings. Generates an embedding via Ollama and searches the HNSW index.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query_text": {
                    "type": "string",
                    "description": "Text to find similar entities for",
                },
                "predicate": {
                    "type": "string",
                    "description": "Vector predicate IRI (default: http://sutra.dev/hasEmbedding)",
                },
                "limit": {
                    "type": "integer",
                    "description": "Max results to return (default: 10)",
                },
            },
            "required": ["query_text"],
        },
    },
    {
        "name": "database_info",
        "description": "Get SutraDB database statistics: health, vector index info, and sample data.",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "health_check",
        "description": "Check if SutraDB is running and healthy.",
        "inputSchema": {"type": "object", "properties": {}},
    },
]


def handle_message(msg: dict) -> dict:
    """Handle a JSON-RPC message."""
    method = msg.get("method", "")
    msg_id = msg.get("id")

    if method == "initialize":
        return {
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {
                    "name": "sutradb-mcp",
                    "version": "0.1.0",
                },
            },
        }

    if method == "notifications/initialized":
        return None  # No response needed

    if method == "tools/list":
        return {
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {"tools": TOOLS},
        }

    if method == "tools/call":
        params = msg.get("params", {})
        tool_name = params.get("name", "")
        arguments = params.get("arguments", {})
        result = handle_tool_call(tool_name, arguments)
        return {
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {
                "content": [
                    {
                        "type": "text",
                        "text": json.dumps(result, ensure_ascii=False, indent=2)
                        if isinstance(result, (dict, list))
                        else str(result),
                    }
                ]
            },
        }

    return {
        "jsonrpc": "2.0",
        "id": msg_id,
        "error": {"code": -32601, "message": f"Unknown method: {method}"},
    }


def main():
    """Run the MCP server on stdio."""
    sys.stderr.write("SutraDB MCP server starting on stdio...\n")
    sys.stderr.write(f"Connecting to SutraDB at {SUTRA_ENDPOINT}\n")

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
            response = handle_message(msg)
            if response:
                sys.stdout.write(json.dumps(response) + "\n")
                sys.stdout.flush()
        except json.JSONDecodeError:
            sys.stderr.write(f"Invalid JSON: {line}\n")
        except Exception as e:
            sys.stderr.write(f"Error: {e}\n")


if __name__ == "__main__":
    main()
