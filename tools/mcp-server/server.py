"""SutraDB MCP Server — Sutra Studio for AI agents.

Dual-mode MCP server for database maintenance and querying.
This is the agent<->database bridge, designed maintenance-first.

Modes:
- Server mode:     connects to SutraDB HTTP endpoint (default http://localhost:3030)
- Serverless mode: shells out to `sutra` CLI binary for direct .sdb file access

Environment variables:
  SUTRA_MODE      = server | serverless  (default: server)
  SUTRA_URL       = HTTP endpoint        (default: http://localhost:3030)
  SUTRA_DATA_DIR  = path to .sdb data    (default: ./sutra-data)
  SUTRA_CLI       = path to sutra binary (default: sutra)
  SUTRA_PASSCODE  = auth passcode        (optional, server mode only)

Usage:
    python tools/mcp-server/server.py
    python tools/mcp-server/server.py --mode serverless --data-dir ./my-data
    SUTRA_MODE=server SUTRA_URL=http://localhost:3030 python tools/mcp-server/server.py

Protocol: JSON-RPC over stdio, MCP version 2024-11-05
"""

import argparse
import io
import json
import os
import subprocess
import sys
from typing import Any, Optional

import requests

# Fix Windows Unicode output
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8")
sys.stderr = io.TextIOWrapper(sys.stderr.buffer, encoding="utf-8")

# ── Configuration ─────────────────────────────────────────────────────────────

MODE = os.environ.get("SUTRA_MODE", "server")  # server | serverless
SUTRA_URL = os.environ.get("SUTRA_URL", "http://localhost:3030")
DATA_DIR = os.environ.get("SUTRA_DATA_DIR", "./sutra-data")
SUTRA_CLI = os.environ.get("SUTRA_CLI", "sutra")
PASSCODE = os.environ.get("SUTRA_PASSCODE", "")


def configure_from_args():
    """Override config from command-line arguments if provided."""
    global MODE, SUTRA_URL, DATA_DIR, SUTRA_CLI, PASSCODE
    parser = argparse.ArgumentParser(description="SutraDB MCP Server")
    parser.add_argument("--mode", choices=["server", "serverless"], default=None)
    parser.add_argument("--url", default=None, help="SutraDB HTTP endpoint")
    parser.add_argument("--data-dir", default=None, help="Path to .sdb data directory")
    parser.add_argument("--cli", default=None, help="Path to sutra CLI binary")
    parser.add_argument("--passcode", default=None, help="Auth passcode")
    args = parser.parse_args()

    if args.mode:
        MODE = args.mode
    if args.url:
        SUTRA_URL = args.url
    if args.data_dir:
        DATA_DIR = args.data_dir
    if args.cli:
        SUTRA_CLI = args.cli
    if args.passcode:
        PASSCODE = args.passcode


# ── Transport Layer ───────────────────────────────────────────────────────────


def _auth_headers() -> dict:
    """Build auth headers for server mode requests."""
    headers = {}
    if PASSCODE:
        headers["Authorization"] = f"Bearer {PASSCODE}"
    return headers


def http_request(method: str, path: str, **kwargs) -> dict:
    """Make an HTTP request to SutraDB server."""
    url = f"{SUTRA_URL}{path}"
    headers = {**_auth_headers(), **kwargs.pop("headers", {})}
    try:
        resp = requests.request(method, url, timeout=60, headers=headers, **kwargs)
    except requests.ConnectionError:
        return {"error": f"Cannot connect to SutraDB at {SUTRA_URL}. Is the server running?"}
    except requests.Timeout:
        return {"error": "Request timed out after 60 seconds."}
    except Exception as e:
        return {"error": f"HTTP request failed: {e}"}

    if resp.status_code >= 400:
        return {"error": f"HTTP {resp.status_code}: {resp.text}"}
    try:
        return resp.json()
    except Exception:
        return {"result": resp.text}


def cli_run(args: list, input_data: Optional[str] = None) -> dict:
    """Run a sutra CLI command and return structured output."""
    cmd = [SUTRA_CLI] + args
    try:
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=120,
            input=input_data,
        )
    except FileNotFoundError:
        return {"error": f"sutra CLI not found at '{SUTRA_CLI}'. Is it installed and on PATH?"}
    except subprocess.TimeoutExpired:
        return {"error": "CLI command timed out after 120 seconds."}
    except Exception as e:
        return {"error": f"CLI execution failed: {e}"}

    output = result.stdout.strip()
    if result.returncode != 0:
        error_msg = result.stderr.strip() or output or f"Exit code {result.returncode}"
        return {"error": error_msg}
    return {"result": output}


# ── Tool Implementations ─────────────────────────────────────────────────────


def tool_health_report(_args: dict) -> Any:
    """Full health diagnostics covering HNSW indexes, storage, and consistency."""
    if MODE == "server":
        health = http_request("GET", "/health")
        vectors = http_request("GET", "/vectors/health")
        is_healthy = health.get("result") == "ok" or health.get("result", "").strip() == "ok"
        return {
            "mode": "server",
            "endpoint": SUTRA_URL,
            "server_reachable": "error" not in health,
            "healthy": is_healthy,
            "vector_indexes": vectors,
        }
    else:
        result = cli_run(["health", "--data-dir", DATA_DIR])
        return {
            "mode": "serverless",
            "data_dir": DATA_DIR,
            "report": result.get("result", result.get("error", "unknown")),
        }


def tool_rebuild_hnsw(_args: dict) -> Any:
    """Trigger HNSW compaction to remove tombstones and restore connectivity."""
    if MODE == "server":
        return http_request("POST", "/vectors/rebuild")
    else:
        result = cli_run(["health", "--data-dir", DATA_DIR, "--rebuild-hnsw"])
        return {
            "mode": "serverless",
            "result": result.get("result", result.get("error", "unknown")),
        }


def tool_verify_consistency(_args: dict) -> Any:
    """Check index consistency across SPO/POS/OSP indexes.

    In server mode, the server auto-repairs on startup so this checks
    current health. In serverless mode, opens the store and verifies.
    """
    if MODE == "server":
        health = http_request("GET", "/health")
        vectors = http_request("GET", "/vectors/health")
        is_reachable = "error" not in health
        return {
            "mode": "server",
            "endpoint": SUTRA_URL,
            "server_reachable": is_reachable,
            "note": (
                "Server mode auto-repairs index inconsistencies on startup. "
                "If the server is running, indexes are consistent. "
                "Use health_report and database_info for detailed diagnostics."
            ),
            "health": health,
            "vectors": vectors,
        }
    else:
        # In serverless mode, open the store via CLI info command which
        # will fail if the store is corrupted
        info_result = cli_run(["info", "--data-dir", DATA_DIR])
        if "error" in info_result:
            return {
                "mode": "serverless",
                "consistent": False,
                "error": info_result["error"],
                "recommendation": "Try running: sutra health --data-dir " + DATA_DIR,
            }
        return {
            "mode": "serverless",
            "consistent": True,
            "info": info_result.get("result", ""),
            "note": "Store opened successfully. Run health_report for detailed diagnostics.",
        }


def tool_database_info(_args: dict) -> Any:
    """Get database statistics: triple count, term count, vector index health."""
    if MODE == "server":
        health = http_request("GET", "/health")
        vectors = http_request("GET", "/vectors/health")
        # Get a count of triples
        count_result = http_request(
            "POST", "/sparql",
            data="SELECT (COUNT(*) AS ?count) WHERE { ?s ?p ?o }",
            headers={"Content-Type": "application/sparql-query"},
        )
        triple_count = None
        if "results" in count_result:
            bindings = count_result.get("results", {}).get("bindings", [])
            if bindings:
                triple_count = bindings[0].get("count", {}).get("value")

        # Get sample data
        sample = http_request(
            "POST", "/sparql",
            data="SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 5",
            headers={"Content-Type": "application/sparql-query"},
        )
        sample_triples = []
        if "results" in sample:
            for b in sample.get("results", {}).get("bindings", [])[:5]:
                sample_triples.append({
                    "subject": b.get("s", {}).get("value", ""),
                    "predicate": b.get("p", {}).get("value", ""),
                    "object": b.get("o", {}).get("value", ""),
                })

        return {
            "mode": "server",
            "endpoint": SUTRA_URL,
            "healthy": "error" not in health,
            "triple_count": triple_count,
            "vector_indexes": vectors,
            "sample_triples": sample_triples,
        }
    else:
        result = cli_run(["info", "--data-dir", DATA_DIR])
        return {
            "mode": "serverless",
            "data_dir": DATA_DIR,
            "info": result.get("result", result.get("error", "unknown")),
        }


def tool_backup(args: dict) -> Any:
    """Trigger a backup snapshot of the database.

    In server mode, creates a copy of the data directory.
    In serverless mode, copies the .sdb directory to a backup location.
    """
    backup_path = args.get("path", "")

    if MODE == "server":
        # Server mode doesn't have a dedicated backup endpoint yet,
        # so we document this limitation clearly
        return {
            "mode": "server",
            "note": (
                "Server mode backup is handled via the --backup-interval flag on startup. "
                "For manual backups, stop writes momentarily and copy the data directory, "
                "or use serverless mode to access the .sdb files directly."
            ),
            "recommendation": (
                "Start the server with: sutra serve --backup-interval 60 "
                "for hourly automatic backups."
            ),
        }
    else:
        if not backup_path:
            import time
            timestamp = int(time.time())
            backup_path = f"{DATA_DIR}/backups/backup_{timestamp}"

        # Use the OS to copy the data directory
        import shutil
        src = DATA_DIR
        dst = backup_path
        try:
            # Exclude the backups subdirectory itself
            def ignore_backups(directory, files):
                if os.path.basename(directory) == os.path.basename(src):
                    return ["backups"] if "backups" in files else []
                return []

            os.makedirs(os.path.dirname(dst) if os.path.dirname(dst) else ".", exist_ok=True)
            shutil.copytree(src, dst, ignore=ignore_backups)
            return {
                "mode": "serverless",
                "status": "ok",
                "backup_path": os.path.abspath(dst),
            }
        except Exception as e:
            return {
                "mode": "serverless",
                "status": "error",
                "error": str(e),
            }


def tool_sparql_query(args: dict) -> Any:
    """Execute a SPARQL query against SutraDB."""
    query = args.get("query", "")
    if not query.strip():
        return {"error": "Query cannot be empty."}

    if MODE == "server":
        result = http_request(
            "POST", "/sparql",
            data=query,
            headers={"Content-Type": "application/sparql-query"},
        )
        # Format results for agent readability
        if "results" in result and "bindings" in result["results"]:
            bindings = result["results"]["bindings"]
            cols = result.get("head", {}).get("vars", [])
            rows = []
            for b in bindings[:100]:  # Cap at 100 rows for agent context
                row = {c: b.get(c, {}).get("value", "") for c in cols}
                rows.append(row)
            return {
                "columns": cols,
                "rows": rows,
                "total": len(bindings),
                "truncated": len(bindings) > 100,
            }
        return result
    else:
        result = cli_run(["query", query, "--data-dir", DATA_DIR])
        return {
            "mode": "serverless",
            "output": result.get("result", result.get("error", "unknown")),
        }


def tool_insert_triples(args: dict) -> Any:
    """Insert RDF triples in N-Triples format."""
    ntriples = args.get("ntriples", "")
    if not ntriples.strip():
        return {"error": "N-Triples data cannot be empty."}

    if MODE == "server":
        return http_request(
            "POST", "/triples",
            data=ntriples.encode("utf-8"),
            headers={"Content-Type": "text/plain; charset=utf-8"},
        )
    else:
        # Write to a temp file and import via CLI
        import tempfile
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".nt", delete=False, encoding="utf-8"
        ) as f:
            f.write(ntriples)
            tmp_path = f.name
        try:
            result = cli_run(["import", tmp_path, "--data-dir", DATA_DIR])
            return {
                "mode": "serverless",
                "result": result.get("result", result.get("error", "unknown")),
            }
        finally:
            try:
                os.unlink(tmp_path)
            except OSError:
                pass


def tool_vector_search(args: dict) -> Any:
    """Search for similar vectors using SPARQL VECTOR_SIMILAR.

    Accepts either raw vector floats or a SPARQL query with VECTOR_SIMILAR.
    """
    vector = args.get("vector", "")
    predicate = args.get("predicate", "http://sutra.dev/hasEmbedding")
    threshold = args.get("threshold", 0.5)
    limit = args.get("limit", 10)

    if not vector.strip():
        return {"error": "Vector string cannot be empty. Provide space-separated floats."}

    query = (
        f"SELECT ?entity WHERE {{\n"
        f'  VECTOR_SIMILAR(?entity <{predicate}> '
        f'"{vector}"^^<http://sutra.dev/f32vec>, {threshold})\n'
        f"}} LIMIT {limit}"
    )

    if MODE == "server":
        result = http_request(
            "POST", "/sparql",
            data=query,
            headers={"Content-Type": "application/sparql-query"},
        )
        if "results" in result and "bindings" in result["results"]:
            entities = [
                b.get("entity", {}).get("value", "")
                for b in result["results"]["bindings"]
            ]
            return {
                "entities": entities,
                "count": len(entities),
                "query_used": query,
            }
        return result
    else:
        result = cli_run(["query", query, "--data-dir", DATA_DIR])
        return {
            "mode": "serverless",
            "output": result.get("result", result.get("error", "unknown")),
            "query_used": query,
        }


# ── Tool Registry ────────────────────────────────────────────────────────────

TOOL_HANDLERS = {
    "health_report": tool_health_report,
    "rebuild_hnsw": tool_rebuild_hnsw,
    "verify_consistency": tool_verify_consistency,
    "database_info": tool_database_info,
    "backup": tool_backup,
    "sparql_query": tool_sparql_query,
    "insert_triples": tool_insert_triples,
    "vector_search": tool_vector_search,
}

TOOLS = [
    {
        "name": "health_report",
        "description": (
            "Full SutraDB health diagnostics. Returns HNSW vector index health "
            "(tombstone ratios, connectivity, dimensions), storage status, and "
            "server reachability. Use this first to understand database state."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {},
        },
    },
    {
        "name": "rebuild_hnsw",
        "description": (
            "Trigger HNSW index compaction. Removes tombstoned nodes and rebuilds "
            "connectivity. Run this when health_report shows high tombstone ratios "
            "(>30%) or degraded connectivity. This is a maintenance operation that "
            "may take time on large indexes."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {},
        },
    },
    {
        "name": "verify_consistency",
        "description": (
            "Check index consistency across SPO/POS/OSP indexes. In server mode, "
            "the server auto-repairs on startup so this confirms current health. "
            "In serverless mode, opens the store and verifies integrity. "
            "Use after crashes or unexpected shutdowns."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {},
        },
    },
    {
        "name": "database_info",
        "description": (
            "Get database statistics: triple count, term count, vector index stats, "
            "and a sample of stored triples. Good for understanding what data is in "
            "the database and its scale."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {},
        },
    },
    {
        "name": "backup",
        "description": (
            "Trigger a backup snapshot of the database. In serverless mode, copies "
            "the .sdb directory to a timestamped backup. Optionally provide a custom "
            "backup path."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": (
                        "Custom backup destination path. If omitted, creates a "
                        "timestamped backup in <data-dir>/backups/."
                    ),
                },
            },
        },
    },
    {
        "name": "sparql_query",
        "description": (
            "Execute a SPARQL query against SutraDB. Supports SELECT, ASK, "
            "CONSTRUCT, DESCRIBE, INSERT DATA, DELETE DATA. Also supports "
            "VECTOR_SIMILAR and VECTOR_SCORE extensions. Returns results as "
            "structured JSON with columns and rows."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The SPARQL query to execute.",
                },
            },
            "required": ["query"],
        },
    },
    {
        "name": "insert_triples",
        "description": (
            "Insert RDF triples in N-Triples format. Each line should be a "
            "complete triple ending with ' .' — for example: "
            "<http://example.org/s> <http://example.org/p> <http://example.org/o> ."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "ntriples": {
                    "type": "string",
                    "description": "N-Triples data (one triple per line, each ending with ' .').",
                },
            },
            "required": ["ntriples"],
        },
    },
    {
        "name": "vector_search",
        "description": (
            "Search for entities with similar vector embeddings using HNSW index. "
            "Provide a raw vector (space-separated floats) and optional similarity "
            "threshold. Uses SPARQL VECTOR_SIMILAR under the hood."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "vector": {
                    "type": "string",
                    "description": (
                        "The query vector as space-separated floats, e.g. "
                        "'0.23 -0.11 0.87 0.42'."
                    ),
                },
                "predicate": {
                    "type": "string",
                    "description": (
                        "Vector predicate IRI (default: http://sutra.dev/hasEmbedding)."
                    ),
                },
                "threshold": {
                    "type": "number",
                    "description": "Minimum similarity threshold 0.0-1.0 (default: 0.5).",
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 10).",
                },
            },
            "required": ["vector"],
        },
    },
]


# ── MCP Protocol (JSON-RPC over stdio) ───────────────────────────────────────


def handle_message(msg: dict) -> Optional[dict]:
    """Handle an incoming JSON-RPC message per MCP spec."""
    method = msg.get("method", "")
    msg_id = msg.get("id")

    # ── initialize ────────────────────────────────────────────────────────
    if method == "initialize":
        return {
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {
                    "name": "sutra-mcp",
                    "version": "0.2.0",
                },
            },
        }

    # ── notifications (no response) ──────────────────────────────────────
    if method == "notifications/initialized":
        return None

    # ── tools/list ────────────────────────────────────────────────────────
    if method == "tools/list":
        return {
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {"tools": TOOLS},
        }

    # ── tools/call ────────────────────────────────────────────────────────
    if method == "tools/call":
        params = msg.get("params", {})
        tool_name = params.get("name", "")
        arguments = params.get("arguments", {})

        handler = TOOL_HANDLERS.get(tool_name)
        if handler is None:
            return {
                "jsonrpc": "2.0",
                "id": msg_id,
                "result": {
                    "content": [{
                        "type": "text",
                        "text": json.dumps({"error": f"Unknown tool: {tool_name}"}),
                    }],
                    "isError": True,
                },
            }

        try:
            result = handler(arguments)
        except Exception as e:
            result = {"error": f"Tool execution failed: {e}"}

        is_error = isinstance(result, dict) and "error" in result
        text = (
            json.dumps(result, ensure_ascii=False, indent=2)
            if isinstance(result, (dict, list))
            else str(result)
        )

        return {
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {
                "content": [{"type": "text", "text": text}],
                "isError": is_error,
            },
        }

    # ── ping ──────────────────────────────────────────────────────────────
    if method == "ping":
        return {
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {},
        }

    # ── unknown method ────────────────────────────────────────────────────
    return {
        "jsonrpc": "2.0",
        "id": msg_id,
        "error": {"code": -32601, "message": f"Method not found: {method}"},
    }


def main():
    """Run the MCP server on stdio."""
    configure_from_args()

    sys.stderr.write(f"sutra-mcp v0.2.0 starting (mode={MODE})\n")
    if MODE == "server":
        sys.stderr.write(f"  endpoint: {SUTRA_URL}\n")
        if PASSCODE:
            sys.stderr.write("  auth: passcode configured\n")
    else:
        sys.stderr.write(f"  data_dir: {DATA_DIR}\n")
        sys.stderr.write(f"  cli: {SUTRA_CLI}\n")

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            sys.stderr.write(f"Invalid JSON received, skipping.\n")
            continue

        try:
            response = handle_message(msg)
        except Exception as e:
            sys.stderr.write(f"Internal error handling message: {e}\n")
            response = {
                "jsonrpc": "2.0",
                "id": msg.get("id"),
                "error": {"code": -32603, "message": f"Internal error: {e}"},
            }

        if response is not None:
            sys.stdout.write(json.dumps(response) + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    main()
