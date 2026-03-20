//! Native MCP (Model Context Protocol) server for AI agents.
//!
//! Implements JSON-RPC 2.0 over stdin/stdout with database maintenance
//! and query tools. Supports two modes:
//! - **Server mode**: connects to a running SutraDB HTTP endpoint
//! - **Serverless mode**: opens a `.sdb` file directly (no server needed)
//!
//! On startup, checks for updates from GitHub releases. If an update is
//! available, it will auto-install after 2 minutes unless the agent calls
//! the `decline_update` tool or `--no-auto-update` is passed.
//!
//! Serverless tools call library functions directly — no PATH dependency
//! on the `sutra` binary.

use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::AsyncBufReadExt;
use tokio::sync::{mpsc, Mutex};

const GITHUB_REPO: &str = "EmmaLeonhart/SutraDB";
const AUTO_UPDATE_DELAY_SECS: u64 = 120;

/// Pending update state shared between the main loop and the background timer.
struct PendingUpdate {
    latest_version: String,
    declined: bool,
    applied: bool,
}

/// Run the MCP server, reading JSON-RPC from stdin, writing to stdout.
pub async fn run_mcp_server(
    url: String,
    data_dir: Option<String>,
    passcode: Option<String>,
    auto_update: bool,
) -> anyhow::Result<()> {
    let mode = if data_dir.is_some() {
        "serverless"
    } else {
        "server"
    };

    let ctx = Arc::new(McpContext {
        url,
        data_dir,
        passcode,
        mode: mode.to_string(),
    });

    let pending_update: Arc<Mutex<Option<PendingUpdate>>> = Arc::new(Mutex::new(None));

    // Channel for background tasks to send notifications to the main loop
    let (notify_tx, mut notify_rx) = mpsc::unbounded_channel::<Value>();

    // Kick off background update check
    if auto_update {
        let pending = Arc::clone(&pending_update);
        let tx = notify_tx.clone();
        tokio::spawn(async move {
            startup_update_check(pending, tx).await;
        });
    }

    let stdout = Arc::new(Mutex::new(tokio::io::stdout()));
    let stdin = tokio::io::stdin();
    let reader = tokio::io::BufReader::new(stdin);
    let mut lines = reader.lines();

    loop {
        tokio::select! {
            // Handle incoming JSON-RPC requests from stdin
            line_result = lines.next_line() => {
                let line = match line_result {
                    Ok(Some(l)) => l,
                    Ok(None) => break, // EOF
                    Err(_) => break,   // stdin closed
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
                        write_response(&stdout, &err).await;
                        continue;
                    }
                };

                let id = request.get("id").cloned().unwrap_or(Value::Null);
                let method = request["method"].as_str().unwrap_or("");

                let response = match method {
                    "initialize" => handle_initialize(&id),
                    "notifications/initialized" => continue,
                    "notifications/cancelled" => continue,
                    "ping" => json!({"jsonrpc": "2.0", "id": id, "result": {}}),
                    "tools/list" => handle_tools_list(&id),
                    "tools/call" => {
                        handle_tools_call(&id, &request["params"], &ctx, &pending_update, &notify_tx).await
                    }
                    "resources/list" => handle_resources_list(&id, &ctx),
                    "resources/read" => handle_resources_read(&id, &request["params"], &ctx).await,
                    "prompts/list" => handle_prompts_list(&id),
                    "prompts/get" => handle_prompts_get(&id, &request["params"]),
                    "logging/setLevel" => {
                        // Acknowledge — we log via tracing, level changes are best-effort
                        json!({"jsonrpc": "2.0", "id": id, "result": {}})
                    }
                    _ => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {"code": -32601, "message": format!("Method not found: {}", method)}
                    }),
                };

                write_response(&stdout, &response).await;
            }

            // Handle outgoing notifications from background tasks
            Some(notification) = notify_rx.recv() => {
                write_response(&stdout, &notification).await;
            }
        }
    }

    Ok(())
}

async fn write_response(stdout: &Arc<Mutex<tokio::io::Stdout>>, response: &Value) {
    use tokio::io::AsyncWriteExt;
    let mut out = stdout.lock().await;
    let msg = format!("{}\n", response);
    let _ = out.write_all(msg.as_bytes()).await;
    let _ = out.flush().await;
}

/// Send an MCP notification (no id, no response expected).
fn send_notification(tx: &mpsc::UnboundedSender<Value>, method: &str, params: Value) {
    let notif = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params
    });
    let _ = tx.send(notif);
}

/// Send a log notification (notifications/message in MCP spec).
fn send_log(tx: &mpsc::UnboundedSender<Value>, level: &str, logger: &str, data: &str) {
    send_notification(
        tx,
        "notifications/message",
        json!({
            "level": level,
            "logger": logger,
            "data": data
        }),
    );
}

struct McpContext {
    url: String,
    data_dir: Option<String>,
    passcode: Option<String>,
    mode: String,
}

// ─── Initialize ──────────────────────────────────────────────────────────────

fn handle_initialize(id: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {},
                "resources": {},
                "prompts": {},
                "logging": {}
            },
            "serverInfo": {
                "name": "sutra-mcp",
                "version": env!("CARGO_PKG_VERSION")
            }
        }
    })
}

// ─── Resources ───────────────────────────────────────────────────────────────

fn handle_resources_list(id: &Value, ctx: &McpContext) -> Value {
    let mut resources = vec![
        json!({
            "uri": "sutra://connection",
            "name": "Connection Info",
            "description": "Current SutraDB connection details: mode, endpoint, authentication status.",
            "mimeType": "application/json"
        }),
        json!({
            "uri": "sutra://version",
            "name": "Version",
            "description": "SutraDB version and build info.",
            "mimeType": "application/json"
        }),
    ];
    if ctx.mode == "serverless" {
        resources.push(json!({
            "uri": "sutra://schema",
            "name": "Schema",
            "description": "All unique predicates in the database (the schema).",
            "mimeType": "application/json"
        }));
    }
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {"resources": resources}
    })
}

async fn handle_resources_read(id: &Value, params: &Value, ctx: &McpContext) -> Value {
    let uri = params["uri"].as_str().unwrap_or("");
    let result = match uri {
        "sutra://connection" => {
            let info = json!({
                "mode": ctx.mode,
                "endpoint": if ctx.mode == "server" { &ctx.url } else { "n/a (direct .sdb)" },
                "data_dir": ctx.data_dir.as_deref().unwrap_or("n/a"),
                "authenticated": ctx.passcode.is_some()
            });
            Ok(serde_json::to_string_pretty(&info).unwrap())
        }
        "sutra://version" => {
            let info = json!({
                "version": env!("CARGO_PKG_VERSION"),
                "name": "SutraDB",
                "mcp_protocol": "2024-11-05",
                "features": ["sparql+", "hnsw", "rdf-star", "vector-literals"]
            });
            Ok(serde_json::to_string_pretty(&info).unwrap())
        }
        "sutra://schema" => read_schema_resource(ctx).await,
        _ => Err(format!("Unknown resource: {}", uri)),
    };

    match result {
        Ok(text) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "contents": [{"uri": uri, "mimeType": "application/json", "text": text}]
            }
        }),
        Err(e) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32602, "message": e}
        }),
    }
}

async fn read_schema_resource(ctx: &McpContext) -> Result<String, String> {
    if ctx.mode == "serverless" {
        let data_dir = ctx.data_dir.as_deref().unwrap_or("./sutra-data");
        let ps = sutra_core::PersistentStore::open(data_dir)
            .map_err(|e| format!("Failed to open store: {}", e))?;
        let mut dict = sutra_core::TermDictionary::new();
        ps.load_terms_into(&mut dict);
        let mut store = sutra_core::TripleStore::new();
        for triple in ps.iter() {
            let _ = store.insert(triple);
        }
        let mut predicates = std::collections::HashSet::new();
        for triple in store.iter() {
            predicates.insert(triple.predicate);
        }
        let pred_names: Vec<String> = predicates
            .iter()
            .filter_map(|&id| dict.resolve(id).map(|s| s.to_string()))
            .collect();
        Ok(serde_json::to_string_pretty(&pred_names).unwrap())
    } else {
        // In server mode, query for predicates via SPARQL
        let query = "SELECT DISTINCT ?p WHERE { ?s ?p ?o } LIMIT 1000";
        let result = http_post(ctx, "/sparql", query, "application/sparql-query").await?;
        Ok(serde_json::to_string_pretty(&result).unwrap())
    }
}

// ─── Prompts ─────────────────────────────────────────────────────────────────

fn handle_prompts_list(id: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "prompts": [
                {
                    "name": "explore_graph",
                    "description": "Generate a SPARQL query to explore the graph structure. Returns sample triples and schema overview.",
                    "arguments": []
                },
                {
                    "name": "find_similar",
                    "description": "Generate a SPARQL+ query with VECTOR_SIMILAR to find nodes similar to a given one.",
                    "arguments": [
                        {
                            "name": "node_iri",
                            "description": "The IRI of the node to find similar items for",
                            "required": true
                        },
                        {
                            "name": "predicate",
                            "description": "The vector predicate to use (e.g. :hasEmbedding)",
                            "required": true
                        }
                    ]
                },
                {
                    "name": "count_by_type",
                    "description": "Generate a SPARQL query to count triples grouped by rdf:type.",
                    "arguments": []
                }
            ]
        }
    })
}

fn handle_prompts_get(id: &Value, params: &Value) -> Value {
    let name = params["name"].as_str().unwrap_or("");
    let args = &params["arguments"];

    let messages = match name {
        "explore_graph" => vec![json!({
            "role": "user",
            "content": {
                "type": "text",
                "text": "Run these two queries to understand this database:\n\n1. Sample 20 triples:\nSELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 20\n\n2. List all predicates:\nSELECT DISTINCT ?p WHERE { ?s ?p ?o }\n\nThen summarize what you see."
            }
        })],
        "find_similar" => {
            let node = args["node_iri"]
                .as_str()
                .unwrap_or("<http://example.org/node>");
            let pred = args["predicate"].as_str().unwrap_or(":hasEmbedding");
            vec![json!({
                "role": "user",
                "content": {
                    "type": "text",
                    "text": format!(
                        "First get the vector for {node}:\nSELECT ?vec WHERE {{ <{node}> <{pred}> ?vec }}\n\nThen use that vector in:\nSELECT ?similar WHERE {{\n  VECTOR_SIMILAR(?similar <{pred}> \"<paste vector>\"^^sutra:f32vec, 0.8)\n}} LIMIT 10",
                        node = node, pred = pred
                    )
                }
            })]
        }
        "count_by_type" => vec![json!({
            "role": "user",
            "content": {
                "type": "text",
                "text": "SELECT ?type (COUNT(?s) AS ?count) WHERE {\n  ?s <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> ?type\n} GROUP BY ?type ORDER BY DESC(?count)"
            }
        })],
        _ => {
            return json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32602, "message": format!("Unknown prompt: {}", name)}
            });
        }
    };

    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "description": format!("Template: {}", name),
            "messages": messages
        }
    })
}

// ─── Tools list ──────────────────────────────────────────────────────────────

fn handle_tools_list(id: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "tools": [
                {
                    "name": "health_report",
                    "description": "Get full database health diagnostics including HNSW indexes, storage stats, and consistency status. Returns structured text with [HEALTHY/WARNING/CRITICAL] status per metric.",
                    "inputSchema": {"type": "object", "properties": {}}
                },
                {
                    "name": "rebuild_hnsw",
                    "description": "Compact and rebuild all HNSW vector indexes. Removes tombstones and restores connectivity. Sends progress notifications.",
                    "inputSchema": {"type": "object", "properties": {}}
                },
                {
                    "name": "verify_consistency",
                    "description": "Check that SPO/POS/OSP indexes are consistent. Automatically repairs if inconsistency is found.",
                    "inputSchema": {"type": "object", "properties": {}}
                },
                {
                    "name": "database_info",
                    "description": "Get database statistics: triple count, term count, vector index count.",
                    "inputSchema": {"type": "object", "properties": {}}
                },
                {
                    "name": "sparql_query",
                    "description": "Execute a SPARQL query and return results. Supports SPARQL+ extensions (VECTOR_SIMILAR, VECTOR_SCORE).",
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
                    "description": "Insert RDF triples in N-Triples format. Each line: <subject> <predicate> <object> .",
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
                    "description": "Create a backup snapshot of the database. Works in both serverless and server modes.",
                    "inputSchema": {"type": "object", "properties": {}}
                },
                {
                    "name": "vector_search",
                    "description": "Search for similar vectors using SPARQL VECTOR_SIMILAR.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "predicate": {"type": "string", "description": "Vector predicate IRI (e.g. http://example.org/hasEmbedding)"},
                            "vector": {"type": "array", "items": {"type": "number"}, "description": "Query vector (array of floats)"},
                            "threshold": {"type": "number", "description": "Minimum similarity threshold (0.0-1.0, default 0.8)"},
                            "limit": {"type": "integer", "description": "Maximum results to return (default 10)"}
                        },
                        "required": ["predicate", "vector"]
                    }
                },
                {
                    "name": "download_studio",
                    "description": "Download and install Sutra Studio (the GUI dashboard). Downloads the pre-built binary from GitHub releases for the current platform. Sutra Studio provides graph visualization, HNSW health diagnostics, SPARQL query editor, and ontology browsing.",
                    "inputSchema": {"type": "object", "properties": {}}
                },
                {
                    "name": "launch_studio",
                    "description": "Launch Sutra Studio. Opens the GUI dashboard connecting to the current SutraDB instance. Downloads Studio first if not already installed.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "endpoint": {
                                "type": "string",
                                "description": "SutraDB endpoint URL for Studio to connect to (defaults to current MCP connection)"
                            }
                        }
                    }
                },
                {
                    "name": "check_update",
                    "description": "Check if a SutraDB update is available. Returns current version, latest version, and auto-update status.",
                    "inputSchema": {"type": "object", "properties": {}}
                },
                {
                    "name": "decline_update",
                    "description": "Decline the pending auto-update. By default, SutraDB auto-updates 2 minutes after startup if a new version is available. Call this to cancel.",
                    "inputSchema": {"type": "object", "properties": {}}
                }
            ]
        }
    })
}

// ─── Tools dispatch ──────────────────────────────────────────────────────────

async fn handle_tools_call(
    id: &Value,
    params: &Value,
    ctx: &McpContext,
    pending_update: &Arc<Mutex<Option<PendingUpdate>>>,
    notify_tx: &mpsc::UnboundedSender<Value>,
) -> Value {
    let tool_name = params["name"].as_str().unwrap_or("");
    let args = &params["arguments"];

    let result = match tool_name {
        "health_report" => tool_health_report(ctx).await,
        "rebuild_hnsw" => tool_rebuild_hnsw(ctx, notify_tx).await,
        "verify_consistency" => tool_verify_consistency(ctx).await,
        "database_info" => tool_database_info(ctx).await,
        "sparql_query" => tool_sparql_query(ctx, args).await,
        "insert_triples" => tool_insert_triples(ctx, args).await,
        "backup" => tool_backup(ctx).await,
        "vector_search" => tool_vector_search(ctx, args).await,
        "download_studio" => tool_download_studio(notify_tx).await,
        "launch_studio" => tool_launch_studio(ctx, args, notify_tx).await,
        "check_update" => tool_check_update(pending_update).await,
        "decline_update" => tool_decline_update(pending_update).await,
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

// ─── Auto-update ─────────────────────────────────────────────────────────────

async fn startup_update_check(
    pending: Arc<Mutex<Option<PendingUpdate>>>,
    tx: mpsc::UnboundedSender<Value>,
) {
    let current = env!("CARGO_PKG_VERSION");

    let latest = match check_latest_version().await {
        Some(v) => v,
        None => return,
    };

    if latest == current {
        return;
    }

    // Store pending update state
    {
        let mut lock = pending.lock().await;
        *lock = Some(PendingUpdate {
            latest_version: latest.clone(),
            declined: false,
            applied: false,
        });
    }

    // Notify the agent about the available update
    send_log(
        &tx,
        "warning",
        "sutra-update",
        &format!(
            "Update available: v{} -> v{}. Auto-updating in {} seconds. Call 'decline_update' to cancel.",
            current, latest, AUTO_UPDATE_DELAY_SECS
        ),
    );

    tracing::info!(
        "Update available: v{} -> v{}. Auto-updating in {} seconds.",
        current,
        latest,
        AUTO_UPDATE_DELAY_SECS
    );

    // Wait the decline window
    tokio::time::sleep(std::time::Duration::from_secs(AUTO_UPDATE_DELAY_SECS)).await;

    // Check if declined
    let should_update = {
        let lock = pending.lock().await;
        match lock.as_ref() {
            Some(p) => !p.declined && !p.applied,
            None => false,
        }
    };

    if !should_update {
        send_log(
            &tx,
            "info",
            "sutra-update",
            "Auto-update declined or already applied.",
        );
        return;
    }

    // Perform the update
    send_log(
        &tx,
        "info",
        "sutra-update",
        &format!("Auto-update window expired. Downloading v{}...", latest),
    );

    match perform_update(&latest).await {
        Ok(()) => {
            let mut lock = pending.lock().await;
            if let Some(p) = lock.as_mut() {
                p.applied = true;
            }
            send_log(
                &tx,
                "info",
                "sutra-update",
                &format!(
                    "Updated to v{}. Restart the MCP server to use the new version.",
                    latest
                ),
            );
        }
        Err(e) => {
            send_log(
                &tx,
                "error",
                "sutra-update",
                &format!("Auto-update failed: {}. Continuing with v{}.", e, current),
            );
        }
    }

    // Also update Sutra Studio if it is installed
    if studio_is_installed() {
        send_log(
            &tx,
            "info",
            "sutra-update",
            "Sutra Studio is installed — checking for Studio update...",
        );
        // Remove old installation and re-download to get the matching version
        match download_studio_from_github(&tx).await {
            Ok(msg) => {
                send_log(
                    &tx,
                    "info",
                    "sutra-update",
                    &format!("Studio updated: {}", msg),
                );
            }
            Err(e) => {
                send_log(
                    &tx,
                    "warning",
                    "sutra-update",
                    &format!("Studio update failed (non-critical): {}", e),
                );
            }
        }
    }
}

async fn check_latest_version() -> Option<String> {
    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        GITHUB_REPO
    );
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("User-Agent", format!("sutra/{}", env!("CARGO_PKG_VERSION")))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: Value = resp.json().await.ok()?;
    let tag = json["tag_name"].as_str()?;
    Some(tag.strip_prefix('v').unwrap_or(tag).to_string())
}

async fn perform_update(latest: &str) -> Result<(), String> {
    let current = env!("CARGO_PKG_VERSION");
    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        GITHUB_REPO
    );
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("User-Agent", format!("sutra/{}", current))
        .send()
        .await
        .map_err(|e| format!("HTTP error: {}", e))?;
    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("JSON error: {}", e))?;

    let assets = json["assets"].as_array().cloned().unwrap_or_default();
    let asset_url = get_asset_url(&assets).ok_or_else(|| {
        format!(
            "No pre-built binary for this platform. Update manually: \
             cargo install --git https://github.com/{} sutra-cli",
            GITHUB_REPO
        )
    })?;

    let archive_resp = client
        .get(&asset_url)
        .header("User-Agent", format!("sutra/{}", current))
        .send()
        .await
        .map_err(|e| format!("Download error: {}", e))?;
    if !archive_resp.status().is_success() {
        return Err(format!("Download failed (HTTP {})", archive_resp.status()));
    }

    let bytes = archive_resp
        .bytes()
        .await
        .map_err(|e| format!("Read error: {}", e))?;

    let current_exe = std::env::current_exe().map_err(|e| format!("Cannot find exe: {}", e))?;
    let backup_path = current_exe.with_extension("old");

    let binary_name = if cfg!(target_os = "windows") {
        "sutra.exe"
    } else {
        "sutra"
    };

    let extracted = if asset_url.ends_with(".zip") {
        extract_from_zip(&bytes, binary_name)?
    } else {
        extract_from_tar_gz(&bytes, binary_name)?
    };

    if backup_path.exists() {
        std::fs::remove_file(&backup_path).ok();
    }
    std::fs::rename(&current_exe, &backup_path)
        .map_err(|e| format!("Backup current binary: {}", e))?;
    std::fs::write(&current_exe, &extracted).map_err(|e| format!("Write new binary: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&current_exe, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("Set permissions: {}", e))?;
    }

    tracing::info!(
        "Binary updated: v{} -> v{}. Old binary at {}",
        current,
        latest,
        backup_path.display()
    );
    Ok(())
}

fn get_asset_url(assets: &[Value]) -> Option<String> {
    let target = if cfg!(target_os = "windows") {
        "windows-x64"
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "macos-arm64"
        } else {
            "macos-x64"
        }
    } else {
        "linux-x64"
    };
    for asset in assets {
        if let Some(name) = asset["name"].as_str() {
            if name.contains(target) {
                return asset["browser_download_url"]
                    .as_str()
                    .map(|s| s.to_string());
            }
        }
    }
    None
}

fn extract_from_zip(data: &[u8], file_name: &str) -> Result<Vec<u8>, String> {
    let cursor = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| format!("Zip open error: {}", e))?;
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("Zip read error: {}", e))?;
        if file.name().ends_with(file_name) {
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut file, &mut buf)
                .map_err(|e| format!("Zip extract error: {}", e))?;
            return Ok(buf);
        }
    }
    Err(format!("Binary '{}' not found in archive", file_name))
}

fn extract_from_tar_gz(data: &[u8], file_name: &str) -> Result<Vec<u8>, String> {
    let cursor = std::io::Cursor::new(data);
    let gz = flate2::read::GzDecoder::new(cursor);
    let mut archive = tar::Archive::new(gz);
    let entries = archive
        .entries()
        .map_err(|e| format!("Tar open error: {}", e))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("Tar read error: {}", e))?;
        let path = entry
            .path()
            .map_err(|e| format!("Tar path error: {}", e))?
            .to_path_buf();
        if path.file_name().and_then(|n| n.to_str()) == Some(file_name) {
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut buf)
                .map_err(|e| format!("Tar extract error: {}", e))?;
            return Ok(buf);
        }
    }
    Err(format!("Binary '{}' not found in archive", file_name))
}

// ─── Studio tools ─────────────────────────────────────────────────────────────

/// Get the directory where Sutra Studio is (or should be) installed.
/// Located alongside the sutra binary: `<binary-dir>/sutra-studio/`.
fn studio_install_dir() -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("Cannot find exe: {}", e))?;
    let dir = exe
        .parent()
        .ok_or_else(|| "Cannot determine binary directory".to_string())?;
    Ok(dir.join("sutra-studio"))
}

/// Get the platform-specific Studio executable path.
fn studio_executable() -> Result<std::path::PathBuf, String> {
    let dir = studio_install_dir()?;
    if cfg!(target_os = "windows") {
        Ok(dir.join("sutra_studio.exe"))
    } else if cfg!(target_os = "macos") {
        Ok(dir
            .join("sutra_studio.app")
            .join("Contents")
            .join("MacOS")
            .join("sutra_studio"))
    } else {
        Ok(dir.join("sutra_studio"))
    }
}

/// Check if Sutra Studio is installed.
fn studio_is_installed() -> bool {
    studio_executable().map(|p| p.exists()).unwrap_or(false)
}

/// Get the asset name pattern for Sutra Studio on this platform.
fn studio_asset_target() -> &'static str {
    if cfg!(target_os = "windows") {
        "sutra-studio-windows"
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "sutra-studio-macos-arm64"
        } else {
            "sutra-studio-macos-x64"
        }
    } else {
        "sutra-studio-linux"
    }
}

/// Download and install Sutra Studio from GitHub releases.
async fn download_studio_from_github(
    notify_tx: &mpsc::UnboundedSender<Value>,
) -> Result<String, String> {
    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        GITHUB_REPO
    );
    let client = http_client();
    let resp = client
        .get(&url)
        .header("User-Agent", format!("sutra/{}", env!("CARGO_PKG_VERSION")))
        .send()
        .await
        .map_err(|e| format!("HTTP error: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("GitHub API returned HTTP {}", resp.status()));
    }
    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("JSON error: {}", e))?;

    let assets = json["assets"].as_array().cloned().unwrap_or_default();
    let target = studio_asset_target();
    let asset_url = assets
        .iter()
        .find(|a| {
            a["name"]
                .as_str()
                .map(|n| n.contains(target))
                .unwrap_or(false)
        })
        .and_then(|a| a["browser_download_url"].as_str().map(|s| s.to_string()))
        .ok_or_else(|| {
            format!(
                "No Sutra Studio build for this platform ({}) in the latest release. \
                 Build from source: cd sutra-studio && flutter build",
                target
            )
        })?;

    let version = json["tag_name"].as_str().unwrap_or("unknown");

    send_log(
        notify_tx,
        "info",
        "sutra-studio",
        &format!("Downloading Sutra Studio {} for {}...", version, target),
    );

    let archive_resp = client
        .get(&asset_url)
        .header("User-Agent", format!("sutra/{}", env!("CARGO_PKG_VERSION")))
        .send()
        .await
        .map_err(|e| format!("Download error: {}", e))?;
    if !archive_resp.status().is_success() {
        return Err(format!("Download failed (HTTP {})", archive_resp.status()));
    }

    let bytes = archive_resp
        .bytes()
        .await
        .map_err(|e| format!("Read error: {}", e))?;

    let install_dir = studio_install_dir()?;

    // Remove old installation if it exists
    if install_dir.exists() {
        std::fs::remove_dir_all(&install_dir)
            .map_err(|e| format!("Failed to remove old Studio installation: {}", e))?;
    }
    std::fs::create_dir_all(&install_dir)
        .map_err(|e| format!("Failed to create Studio directory: {}", e))?;

    // Extract archive
    if asset_url.ends_with(".zip") {
        extract_zip_to_dir(&bytes, &install_dir)?;
    } else {
        extract_tar_gz_to_dir(&bytes, &install_dir)?;
    }

    // Set executable permission on Unix
    #[cfg(unix)]
    {
        if let Ok(exe) = studio_executable() {
            if exe.exists() {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755));
            }
        }
    }

    send_log(
        notify_tx,
        "info",
        "sutra-studio",
        &format!(
            "Sutra Studio {} installed to {}",
            version,
            install_dir.display()
        ),
    );

    Ok(format!(
        "Sutra Studio {} installed successfully at {}",
        version,
        install_dir.display()
    ))
}

/// Extract a zip archive into a directory, preserving structure.
fn extract_zip_to_dir(data: &[u8], dest: &std::path::Path) -> Result<(), String> {
    let cursor = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| format!("Zip open error: {}", e))?;
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("Zip read error: {}", e))?;
        let out_path = dest.join(
            file.enclosed_name()
                .ok_or_else(|| "Invalid zip entry name".to_string())?,
        );
        if file.is_dir() {
            std::fs::create_dir_all(&out_path).map_err(|e| format!("Create dir error: {}", e))?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("Create dir error: {}", e))?;
            }
            let mut outfile = std::fs::File::create(&out_path)
                .map_err(|e| format!("Create file error: {}", e))?;
            std::io::copy(&mut file, &mut outfile).map_err(|e| format!("Extract error: {}", e))?;
        }
    }
    Ok(())
}

/// Extract a tar.gz archive into a directory, preserving structure.
fn extract_tar_gz_to_dir(data: &[u8], dest: &std::path::Path) -> Result<(), String> {
    let cursor = std::io::Cursor::new(data);
    let gz = flate2::read::GzDecoder::new(cursor);
    let mut archive = tar::Archive::new(gz);
    archive
        .unpack(dest)
        .map_err(|e| format!("Tar extract error: {}", e))?;
    Ok(())
}

async fn tool_download_studio(notify_tx: &mpsc::UnboundedSender<Value>) -> Result<String, String> {
    if studio_is_installed() {
        send_log(
            notify_tx,
            "info",
            "sutra-studio",
            "Sutra Studio already installed. Re-downloading to get latest version...",
        );
    }
    download_studio_from_github(notify_tx).await
}

async fn tool_launch_studio(
    ctx: &McpContext,
    args: &Value,
    notify_tx: &mpsc::UnboundedSender<Value>,
) -> Result<String, String> {
    // Download first if not installed
    if !studio_is_installed() {
        send_log(
            notify_tx,
            "info",
            "sutra-studio",
            "Sutra Studio not found. Downloading...",
        );
        download_studio_from_github(notify_tx).await?;
    }

    let exe = studio_executable()?;
    if !exe.exists() {
        return Err(format!(
            "Studio executable not found at {}. The release archive may have a different structure. \
             Check the sutra-studio directory and launch manually.",
            exe.display()
        ));
    }

    // Determine the endpoint for Studio to connect to
    let endpoint = if let Some(ep) = args.get("endpoint").and_then(|v| v.as_str()) {
        ep.to_string()
    } else if ctx.mode == "server" {
        ctx.url.clone()
    } else {
        // Serverless mode — we need an HTTP server for Studio to connect to.
        // Start a background `sutra serve` pointing at the same .sdb file.
        let data_dir = ctx.data_dir.as_deref().unwrap_or("./sutra-data");
        let port = find_available_port().unwrap_or(3030);

        send_log(
            notify_tx,
            "info",
            "sutra-studio",
            &format!(
                "Serverless mode — starting temporary server on port {} for Studio (data: {})...",
                port, data_dir
            ),
        );

        let sutra_exe = std::env::current_exe()
            .map_err(|e| format!("Cannot find sutra binary: {}", e))?;
        let mut serve_args = vec![
            "serve".to_string(),
            "--port".to_string(),
            port.to_string(),
            "--data-dir".to_string(),
            data_dir.to_string(),
        ];
        if let Some(ref pass) = ctx.passcode {
            serve_args.push("--passcode".to_string());
            serve_args.push(pass.clone());
        }

        match std::process::Command::new(&sutra_exe)
            .args(&serve_args)
            .spawn()
        {
            Ok(_) => {
                // Give the server a moment to start
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                send_log(
                    notify_tx,
                    "info",
                    "sutra-studio",
                    &format!("Background server started on port {}", port),
                );
            }
            Err(e) => {
                return Err(format!(
                    "Failed to start background server for Studio: {}. \
                     Start a server manually with `sutra serve --data-dir {}` \
                     and try again.",
                    e, data_dir
                ));
            }
        }

        format!("http://localhost:{}", port)
    };

    send_log(
        notify_tx,
        "info",
        "sutra-studio",
        &format!("Launching Sutra Studio (connecting to {})...", endpoint),
    );

    // Launch Studio as a detached process
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("cmd")
        .args(["/c", "start", "", &exe.to_string_lossy()])
        .env("SUTRA_ENDPOINT", &endpoint)
        .spawn();

    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open")
        .arg(exe.parent().unwrap().parent().unwrap().parent().unwrap()) // .app bundle
        .env("SUTRA_ENDPOINT", &endpoint)
        .spawn();

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let result = std::process::Command::new(&exe)
        .env("SUTRA_ENDPOINT", &endpoint)
        .spawn();

    match result {
        Ok(_) => Ok(format!(
            "Sutra Studio launched, connecting to {}. \
             The GUI provides graph visualization, HNSW health diagnostics, \
             SPARQL query editor, and ontology browsing.",
            endpoint
        )),
        Err(e) => Err(format!("Failed to launch Sutra Studio: {}", e)),
    }
}

/// Find an available TCP port by binding to port 0.
fn find_available_port() -> Option<u16> {
    std::net::TcpListener::bind("127.0.0.1:0")
        .ok()
        .and_then(|l| l.local_addr().ok())
        .map(|a| a.port())
}

// ─── Update tools ────────────────────────────────────────────────────────────

async fn tool_check_update(pending: &Arc<Mutex<Option<PendingUpdate>>>) -> Result<String, String> {
    let current = env!("CARGO_PKG_VERSION");
    let lock = pending.lock().await;
    match lock.as_ref() {
        Some(p) => {
            let status = if p.applied {
                "applied (restart to use new version)"
            } else if p.declined {
                "declined for this session"
            } else {
                "pending (will auto-apply after 2-minute window)"
            };
            Ok(format!(
                "Current version: v{}\nLatest version: v{}\nAuto-update status: {}",
                current, p.latest_version, status
            ))
        }
        None => Ok(format!(
            "Current version: v{}\nNo update pending. Either already up to date or auto-update is disabled.",
            current
        )),
    }
}

async fn tool_decline_update(
    pending: &Arc<Mutex<Option<PendingUpdate>>>,
) -> Result<String, String> {
    let mut lock = pending.lock().await;
    match lock.as_mut() {
        Some(p) if !p.applied => {
            p.declined = true;
            Ok(format!(
                "Auto-update to v{} declined for this session. \
                 Run `sutra update` manually to install later.",
                p.latest_version
            ))
        }
        Some(p) if p.applied => Ok(format!(
            "Update to v{} has already been applied. Restart to use the new version.",
            p.latest_version
        )),
        _ => Ok("No pending update to decline.".to_string()),
    }
}

// ─── HTTP helpers (server mode) ──────────────────────────────────────────────

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

// ─── Serverless helpers (direct library calls, no PATH dependency) ───────────

/// Open PersistentStore and hydrate in-memory TripleStore + TermDictionary.
fn open_serverless(
    data_dir: &str,
) -> Result<
    (
        sutra_core::PersistentStore,
        sutra_core::TripleStore,
        sutra_core::TermDictionary,
    ),
    String,
> {
    let ps = sutra_core::PersistentStore::open(data_dir)
        .map_err(|e| format!("Failed to open store: {}", e))?;
    let mut dict = sutra_core::TermDictionary::new();
    let mut store = sutra_core::TripleStore::new();
    ps.load_terms_into(&mut dict);
    for triple in ps.iter() {
        let _ = store.insert(triple);
    }
    Ok((ps, store, dict))
}

/// Resolve a TermId to its string representation.
fn resolve_id(id: sutra_core::TermId, dict: &sutra_core::TermDictionary) -> String {
    if let Some(n) = sutra_core::decode_inline_integer(id) {
        return n.to_string();
    }
    if let Some(b) = sutra_core::decode_inline_boolean(id) {
        return b.to_string();
    }
    dict.resolve(id)
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("_:id{}", id))
}

// ─── Tool implementations ────────────────────────────────────────────────────

async fn tool_health_report(ctx: &McpContext) -> Result<String, String> {
    if ctx.mode == "serverless" {
        let data_dir = ctx.data_dir.as_deref().unwrap_or("./sutra-data");
        let (_ps, store, dict) = open_serverless(data_dir)?;
        let vectors = sutra_hnsw::VectorRegistry::new();
        let report = sutra_sparql::generate_health_report(&store, &dict, &vectors, None);
        return Ok(report.to_ai_text());
    }
    let health = http_get(ctx, "/health").await?;
    let vectors = http_get(ctx, "/vectors/health").await?;
    Ok(json!({"health": health, "vectors": vectors}).to_string())
}

async fn tool_rebuild_hnsw(
    ctx: &McpContext,
    notify_tx: &mpsc::UnboundedSender<Value>,
) -> Result<String, String> {
    if ctx.mode == "serverless" {
        let data_dir = ctx.data_dir.as_deref().unwrap_or("./sutra-data");
        let (_ps, store, dict) = open_serverless(data_dir)?;

        // Rebuild HNSW from stored vector triples
        send_log(
            notify_tx,
            "info",
            "sutra-hnsw",
            "Rebuilding HNSW indexes from stored vectors...",
        );

        let mut vectors = sutra_hnsw::VectorRegistry::new();
        let mut vec_count = 0usize;
        let f32vec_suffix = "^^<http://sutra.dev/f32vec>";
        for triple in store.iter() {
            if let Some(obj_str) = dict.resolve(triple.object) {
                if obj_str.contains(f32vec_suffix) {
                    if let Some(start) = obj_str.find('"') {
                        let end = obj_str[start + 1..].find('"').map(|p| p + start + 1);
                        if let Some(end) = end {
                            let vec_str = &obj_str[start + 1..end];
                            let floats: Vec<f32> = vec_str
                                .split_whitespace()
                                .filter_map(|s| s.parse::<f32>().ok())
                                .collect();
                            if !floats.is_empty() {
                                let dims = floats.len();
                                if !vectors.has_index(triple.predicate) {
                                    let config = sutra_hnsw::VectorPredicateConfig {
                                        predicate_id: triple.predicate,
                                        dimensions: dims,
                                        m: 16,
                                        ef_construction: 200,
                                        metric: sutra_hnsw::DistanceMetric::Cosine,
                                    };
                                    let _ = vectors.declare(config);
                                }
                                let _ = vectors.insert(triple.predicate, floats, triple.object);
                                vec_count += 1;
                            }
                        }
                    }
                }
            }
        }

        // Now compact each index
        let mut total_removed = 0usize;
        for pred_id in vectors.predicates() {
            if let Some(index) = vectors.get_mut(pred_id) {
                let pred_name = dict.resolve(pred_id).unwrap_or("unknown");
                let before = index.len();
                let removed = index.compact();
                total_removed += removed;
                send_log(
                    notify_tx,
                    "info",
                    "sutra-hnsw",
                    &format!(
                        "Rebuilt '{}': {} tombstones removed ({} -> {} nodes)",
                        pred_name,
                        removed,
                        before,
                        index.len()
                    ),
                );
            }
        }

        return Ok(format!(
            "Rebuilt HNSW from {} vectors. Removed {} tombstones across all indexes.",
            vec_count, total_removed
        ));
    }

    send_log(
        notify_tx,
        "info",
        "sutra-hnsw",
        "Requesting HNSW rebuild from server...",
    );
    let result = http_post(ctx, "/vectors/rebuild", "", "application/json").await?;
    send_log(notify_tx, "info", "sutra-hnsw", "HNSW rebuild complete.");
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
    let health = http_get(ctx, "/health").await?;
    Ok(format!(
        "Server is running (consistency verified at startup): {}",
        health
    ))
}

async fn tool_database_info(ctx: &McpContext) -> Result<String, String> {
    if ctx.mode == "serverless" {
        let data_dir = ctx.data_dir.as_deref().unwrap_or("./sutra-data");
        let ps = sutra_core::PersistentStore::open(data_dir)
            .map_err(|e| format!("Failed to open store: {}", e))?;
        let triple_count = ps.len();
        let mut dict = sutra_core::TermDictionary::new();
        let term_count = ps.load_terms_into(&mut dict);
        return Ok(format!(
            "SutraDB — {}\n  Triples: {}\n  Terms:   {}",
            data_dir, triple_count, term_count
        ));
    }
    let count_query = "SELECT (COUNT(*) AS ?count) WHERE { ?s ?p ?o }";
    let count_result = http_post(ctx, "/sparql", count_query, "application/sparql-query").await?;
    let vectors = http_get(ctx, "/vectors/health").await?;
    Ok(json!({"triples": count_result, "vectors": vectors}).to_string())
}

async fn tool_sparql_query(ctx: &McpContext, args: &Value) -> Result<String, String> {
    let query = args["query"].as_str().ok_or("Missing 'query' argument")?;
    if ctx.mode == "serverless" {
        let data_dir = ctx.data_dir.as_deref().unwrap_or("./sutra-data");
        let (_ps, store, dict) = open_serverless(data_dir)?;
        let vectors = sutra_hnsw::VectorRegistry::new();

        let mut parsed =
            sutra_sparql::parse(query).map_err(|e| format!("SPARQL parse error: {}", e))?;
        sutra_sparql::optimize(&mut parsed);
        let result = sutra_sparql::execute_with_vectors(&parsed, &store, &dict, &vectors)
            .map_err(|e| format!("SPARQL execution error: {}", e))?;

        // Format as table
        let mut output = String::new();
        if result.columns.len() == 1 && result.columns[0] == "result" {
            // ASK query
            if let Some(row) = result.rows.first() {
                if let Some(&id) = row.get("result") {
                    if sutra_core::decode_inline_boolean(id) == Some(true) {
                        output.push_str("true");
                    } else {
                        output.push_str("false");
                    }
                }
            }
        } else {
            // SELECT query
            output.push_str(&result.columns.join("\t"));
            output.push('\n');
            output.push_str(&"-".repeat(result.columns.len() * 20));
            output.push('\n');
            for row in &result.rows {
                let vals: Vec<String> = result
                    .columns
                    .iter()
                    .map(|col| {
                        row.get(col)
                            .map(|&id| resolve_id(id, &dict))
                            .unwrap_or_default()
                    })
                    .collect();
                output.push_str(&vals.join("\t"));
                output.push('\n');
            }
            output.push_str(&format!("\n{} rows", result.rows.len()));
        }
        return Ok(output);
    }
    let result = http_post(ctx, "/sparql", query, "application/sparql-query").await?;
    Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
}

async fn tool_insert_triples(ctx: &McpContext, args: &Value) -> Result<String, String> {
    let data = args["data"].as_str().ok_or("Missing 'data' argument")?;
    if ctx.mode == "serverless" {
        let data_dir = ctx.data_dir.as_deref().unwrap_or("./sutra-data");
        let ps = sutra_core::PersistentStore::open(data_dir)
            .map_err(|e| format!("Failed to open store: {}", e))?;

        let mut inserted = 0usize;
        let mut errors = 0usize;
        for line in data.lines() {
            let parsed = match sutra_core::parse_ntriples_line(line) {
                Some(t) => t,
                None => continue,
            };
            let (subj_str, pred_str, obj_str) = parsed;
            let s_id = ps
                .intern(&subj_str)
                .map_err(|e| format!("Intern error: {}", e))?;
            let p_id = ps
                .intern(&pred_str)
                .map_err(|e| format!("Intern error: {}", e))?;
            let o_id = ps
                .intern(&obj_str)
                .map_err(|e| format!("Intern error: {}", e))?;
            match ps.insert(sutra_core::Triple::new(s_id, p_id, o_id)) {
                Ok(()) => inserted += 1,
                Err(_) => errors += 1,
            }
        }
        ps.flush().map_err(|e| format!("Flush error: {}", e))?;
        return Ok(format!("Inserted {} triples ({} errors)", inserted, errors));
    }
    let result = http_post(ctx, "/triples", data, "application/n-triples").await?;
    Ok(result.to_string())
}

async fn tool_backup(ctx: &McpContext) -> Result<String, String> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if ctx.mode == "serverless" {
        let data_dir = ctx.data_dir.as_deref().unwrap_or("./sutra-data");
        let backup_dir = format!("{}/backups/backup_{}", data_dir, timestamp);
        std::fs::create_dir_all(&backup_dir).map_err(|e| format!("Create dir: {}", e))?;
        copy_dir_for_backup(
            std::path::Path::new(data_dir),
            std::path::Path::new(&backup_dir),
        )
        .map_err(|e| format!("Backup failed: {}", e))?;
        return Ok(format!("Backup created at {}", backup_dir));
    }

    // Server mode: snapshot the data directory that the server is using.
    // We infer the data dir from the /health endpoint or default.
    // Since we can't know the server's data dir, we create a backup via
    // exporting all triples to a local N-Triples file.
    let export_result = http_get(ctx, "/graph").await?;
    let backup_file = format!("sutra_backup_{}.ttl", timestamp);
    let content = if let Some(s) = export_result.as_str() {
        s.to_string()
    } else {
        serde_json::to_string_pretty(&export_result).unwrap_or_default()
    };
    std::fs::write(&backup_file, &content).map_err(|e| format!("Write backup: {}", e))?;
    Ok(format!(
        "Backup exported to {} ({} bytes). For full disk-level backup, \
         use --backup-interval on sutra serve or copy the data directory.",
        backup_file,
        content.len()
    ))
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
