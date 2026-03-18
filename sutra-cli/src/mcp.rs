//! Native MCP (Model Context Protocol) server for AI agents.
//!
//! Implements JSON-RPC 2.0 over stdin/stdout with database maintenance
//! and query tools. Supports two modes:
//! - **Server mode**: connects to a running SutraDB HTTP endpoint
//! - **Serverless mode**: opens a `.sdb` file directly (no server needed)

use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

/// Run the MCP server, reading JSON-RPC from stdin, writing to stdout.
pub async fn run_mcp_server(
    url: String,
    data_dir: Option<String>,
    passcode: Option<String>,
) -> anyhow::Result<()> {
    let mode = if data_dir.is_some() {
        "serverless"
    } else {
        "server"
    };

    let ctx = McpContext {
        url,
        data_dir,
        passcode,
        mode: mode.to_string(),
    };

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let err = json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": {"code": -32700, "message": format!("Parse error: {}", e)}
                });
                writeln!(out, "{}", err)?;
                out.flush()?;
                continue;
            }
        };

        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request["method"].as_str().unwrap_or("");

        let response = match method {
            "initialize" => handle_initialize(&id),
            "notifications/initialized" => continue, // no response needed
            "ping" => json!({"jsonrpc": "2.0", "id": id, "result": {}}),
            "tools/list" => handle_tools_list(&id),
            "tools/call" => handle_tools_call(&id, &request["params"], &ctx).await,
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": format!("Method not found: {}", method)}
            }),
        };

        writeln!(out, "{}", response)?;
        out.flush()?;
    }

    Ok(())
}

struct McpContext {
    url: String,
    data_dir: Option<String>,
    passcode: Option<String>,
    mode: String,
}

fn handle_initialize(id: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {
                "name": "sutra-mcp",
                "version": env!("CARGO_PKG_VERSION")
            }
        }
    })
}

fn handle_tools_list(id: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "tools": [
                {
                    "name": "health_report",
                    "description": "Get full database health diagnostics including HNSW indexes, storage stats, and consistency status.",
                    "inputSchema": {"type": "object", "properties": {}}
                },
                {
                    "name": "rebuild_hnsw",
                    "description": "Compact and rebuild all HNSW vector indexes. Removes tombstones and restores connectivity.",
                    "inputSchema": {"type": "object", "properties": {}}
                },
                {
                    "name": "verify_consistency",
                    "description": "Check that SPO/POS/OSP indexes are consistent. Reports whether repair is needed.",
                    "inputSchema": {"type": "object", "properties": {}}
                },
                {
                    "name": "database_info",
                    "description": "Get database statistics: triple count, vector index count, and sample data.",
                    "inputSchema": {"type": "object", "properties": {}}
                },
                {
                    "name": "sparql_query",
                    "description": "Execute a SPARQL query and return results.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "query": {"type": "string", "description": "The SPARQL query to execute"}
                        },
                        "required": ["query"]
                    }
                },
                {
                    "name": "insert_triples",
                    "description": "Insert RDF triples in N-Triples format.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "data": {"type": "string", "description": "N-Triples data (one triple per line)"}
                        },
                        "required": ["data"]
                    }
                },
                {
                    "name": "backup",
                    "description": "Create a backup snapshot of the database.",
                    "inputSchema": {"type": "object", "properties": {}}
                },
                {
                    "name": "vector_search",
                    "description": "Search for similar vectors using SPARQL VECTOR_SIMILAR.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "predicate": {"type": "string", "description": "Vector predicate IRI"},
                            "vector": {"type": "array", "items": {"type": "number"}, "description": "Query vector (array of floats)"},
                            "threshold": {"type": "number", "description": "Minimum similarity threshold (0.0-1.0)"},
                            "limit": {"type": "integer", "description": "Maximum results to return"}
                        },
                        "required": ["predicate", "vector"]
                    }
                }
            ]
        }
    })
}

async fn handle_tools_call(id: &Value, params: &Value, ctx: &McpContext) -> Value {
    let tool_name = params["name"].as_str().unwrap_or("");
    let args = &params["arguments"];

    let result = match tool_name {
        "health_report" => tool_health_report(ctx).await,
        "rebuild_hnsw" => tool_rebuild_hnsw(ctx).await,
        "verify_consistency" => tool_verify_consistency(ctx).await,
        "database_info" => tool_database_info(ctx).await,
        "sparql_query" => tool_sparql_query(ctx, args).await,
        "insert_triples" => tool_insert_triples(ctx, args).await,
        "backup" => tool_backup(ctx).await,
        "vector_search" => tool_vector_search(ctx, args).await,
        _ => Err(format!("Unknown tool: {}", tool_name)),
    };

    match result {
        Ok(content) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{"type": "text", "text": content}]
            }
        }),
        Err(e) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{"type": "text", "text": format!("Error: {}", e)}],
                "isError": true
            }
        }),
    }
}

// ─── HTTP helpers ────────────────────────────────────────────────────────────

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_default()
}

async fn http_get(ctx: &McpContext, path: &str) -> Result<Value, String> {
    let client = http_client();
    let url = format!("{}{}", ctx.url, path);
    let mut req = client.get(&url);
    if let Some(ref pass) = ctx.passcode {
        req = req.header("Authorization", format!("Bearer {}", pass));
    }
    let resp = req.send().await.map_err(|e| format!("HTTP error: {}", e))?;
    let text = resp
        .text()
        .await
        .map_err(|e| format!("Read error: {}", e))?;
    serde_json::from_str(&text).map_err(|_| text)
}

async fn http_post(
    ctx: &McpContext,
    path: &str,
    body: &str,
    content_type: &str,
) -> Result<Value, String> {
    let client = http_client();
    let url = format!("{}{}", ctx.url, path);
    let mut req = client
        .post(&url)
        .header("Content-Type", content_type)
        .body(body.to_string());
    if let Some(ref pass) = ctx.passcode {
        req = req.header("Authorization", format!("Bearer {}", pass));
    }
    let resp = req.send().await.map_err(|e| format!("HTTP error: {}", e))?;
    let text = resp
        .text()
        .await
        .map_err(|e| format!("Read error: {}", e))?;
    serde_json::from_str(&text).map_err(|_| text)
}

async fn cli_run(ctx: &McpContext, args: &[&str]) -> Result<String, String> {
    let data_dir = ctx.data_dir.as_deref().unwrap_or("./sutra-data");
    let mut cmd = tokio::process::Command::new("sutra");
    for arg in args {
        cmd.arg(arg);
    }
    cmd.arg("--data-dir").arg(data_dir);
    let output = cmd
        .output()
        .await
        .map_err(|e| format!("Failed to run sutra CLI: {}", e))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

// ─── Tool implementations ────────────────────────────────────────────────────

async fn tool_health_report(ctx: &McpContext) -> Result<String, String> {
    if ctx.mode == "serverless" {
        return cli_run(ctx, &["health"]).await;
    }
    let health = http_get(ctx, "/health").await?;
    let vectors = http_get(ctx, "/vectors/health").await?;
    Ok(json!({"health": health, "vectors": vectors}).to_string())
}

async fn tool_rebuild_hnsw(ctx: &McpContext) -> Result<String, String> {
    if ctx.mode == "serverless" {
        return cli_run(ctx, &["health", "--rebuild-hnsw"]).await;
    }
    let result = http_post(ctx, "/vectors/rebuild", "", "application/json").await?;
    Ok(result.to_string())
}

async fn tool_verify_consistency(ctx: &McpContext) -> Result<String, String> {
    if ctx.mode == "serverless" {
        let data_dir = ctx.data_dir.as_deref().unwrap_or("./sutra-data");
        let ps = sutra_core::PersistentStore::open(data_dir)
            .map_err(|e| format!("Failed to open store: {}", e))?;
        let consistent = ps.verify_consistency();
        if consistent {
            return Ok("Indexes are consistent. No repair needed.".to_string());
        }
        let count = ps.repair().map_err(|e| format!("Repair failed: {}", e))?;
        ps.flush().map_err(|e| format!("Flush failed: {}", e))?;
        return Ok(format!(
            "Inconsistency detected and repaired. Rebuilt {} triples in secondary indexes.",
            count
        ));
    }
    // Server mode: consistency is verified on startup, report current state
    let health = http_get(ctx, "/health").await?;
    Ok(format!(
        "Server is running (consistency verified at startup): {}",
        health
    ))
}

async fn tool_database_info(ctx: &McpContext) -> Result<String, String> {
    if ctx.mode == "serverless" {
        return cli_run(ctx, &["info"]).await;
    }
    let count_query = "SELECT (COUNT(*) AS ?count) WHERE { ?s ?p ?o }";
    let count_result = http_post(ctx, "/sparql", count_query, "application/sparql-query").await?;
    let vectors = http_get(ctx, "/vectors/health").await?;
    Ok(json!({"triples": count_result, "vectors": vectors}).to_string())
}

async fn tool_sparql_query(ctx: &McpContext, args: &Value) -> Result<String, String> {
    let query = args["query"].as_str().ok_or("Missing 'query' argument")?;
    if ctx.mode == "serverless" {
        return cli_run(ctx, &["query", query]).await;
    }
    let result = http_post(ctx, "/sparql", query, "application/sparql-query").await?;
    Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
}

async fn tool_insert_triples(ctx: &McpContext, args: &Value) -> Result<String, String> {
    let data = args["data"].as_str().ok_or("Missing 'data' argument")?;
    if ctx.mode == "serverless" {
        // Write to temp file, import via CLI
        let tmp = std::env::temp_dir().join("sutra_mcp_import.nt");
        std::fs::write(&tmp, data).map_err(|e| format!("Write temp file: {}", e))?;
        let result = cli_run(
            ctx,
            &["import", tmp.to_str().unwrap_or("sutra_mcp_import.nt")],
        )
        .await;
        let _ = std::fs::remove_file(&tmp);
        return result;
    }
    let result = http_post(ctx, "/triples", data, "application/n-triples").await?;
    Ok(result.to_string())
}

async fn tool_backup(ctx: &McpContext) -> Result<String, String> {
    if ctx.mode == "serverless" {
        let data_dir = ctx.data_dir.as_deref().unwrap_or("./sutra-data");
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let backup_dir = format!("{}/backups/backup_{}", data_dir, timestamp);
        std::fs::create_dir_all(&backup_dir).map_err(|e| format!("Create dir: {}", e))?;
        copy_dir_for_backup(
            std::path::Path::new(data_dir),
            std::path::Path::new(&backup_dir),
        )
        .map_err(|e| format!("Backup failed: {}", e))?;
        return Ok(format!("Backup created at {}", backup_dir));
    }
    Err("Backup in server mode: use --backup-interval on sutra serve, or stop the server and copy the data directory.".to_string())
}

async fn tool_vector_search(ctx: &McpContext, args: &Value) -> Result<String, String> {
    let predicate = args["predicate"]
        .as_str()
        .ok_or("Missing 'predicate' argument")?;
    let vector = args["vector"]
        .as_array()
        .ok_or("Missing 'vector' argument")?;
    let threshold = args["threshold"].as_f64().unwrap_or(0.8);
    let limit = args["limit"].as_u64().unwrap_or(10);

    let vec_str: Vec<String> = vector
        .iter()
        .filter_map(|v| v.as_f64().map(|f| format!("{:.6}", f)))
        .collect();

    let query = format!(
        "SELECT ?s WHERE {{ ?s <{}> ?vec . VECTOR_SIMILAR(?s <{}> \"{}\"^^<http://sutra.dev/f32vec>, {}) }} LIMIT {}",
        predicate, predicate, vec_str.join(" "), threshold, limit
    );

    tool_sparql_query(ctx, &json!({"query": query})).await
}

/// Copy directory for backup, skipping the backups subdirectory.
fn copy_dir_for_backup(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            if entry.file_name() == "backups" {
                continue;
            }
            copy_dir_for_backup(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}
