//! SutraDB CLI: server, query, import, export.

use std::io::{BufRead, Write};
use std::sync::{Arc, RwLock};

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "sutra",
    about = "SutraDB — RDF-star triplestore with HNSW vector indexing"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the SPARQL HTTP server.
    Serve {
        /// Port to listen on.
        #[arg(short, long, default_value = "3030")]
        port: u16,

        /// Data directory for persistent storage (.sdb).
        #[arg(short, long, default_value = "./sutra-data")]
        data_dir: String,

        /// Run in-memory only (no persistence).
        #[arg(long)]
        memory_only: bool,

        /// Simple passcode authentication. When set, all requests
        /// (except /health) require Authorization: Bearer <passcode>.
        #[arg(long)]
        passcode: Option<String>,
    },
    /// Execute a SPARQL query from the command line.
    Query {
        /// The SPARQL query string.
        query: String,

        /// Data directory.
        #[arg(short, long, default_value = "./sutra-data")]
        data_dir: String,
    },
    /// Import N-Triples data from a file into the database.
    Import {
        /// Path to the N-Triples file (use - for stdin).
        file: String,

        /// Data directory.
        #[arg(short, long, default_value = "./sutra-data")]
        data_dir: String,
    },
    /// Export all triples as N-Triples.
    Export {
        /// Data directory.
        #[arg(short, long, default_value = "./sutra-data")]
        data_dir: String,

        /// Output file (default: stdout).
        #[arg(short, long)]
        output: Option<String>,

        /// Export format: nt (N-Triples) or ttl (Turtle).
        #[arg(short, long, default_value = "nt")]
        format: String,
    },
    /// Show database statistics.
    Info {
        /// Data directory.
        #[arg(short, long, default_value = "./sutra-data")]
        data_dir: String,
    },
    /// Agent-first installer: outputs structured config for AI agents.
    /// Generates a markdown notes file documenting the database setup.
    #[command(name = "install-agent")]
    InstallAgent {
        /// Database name (used for directory and notes file).
        #[arg(default_value = "sutra-db")]
        name: String,

        /// Port for the server.
        #[arg(long, default_value = "3030")]
        port: u16,

        /// Enable passcode authentication.
        #[arg(long)]
        passcode: Option<String>,

        /// Vector dimensions (for default embedding predicate).
        #[arg(long, default_value = "1024")]
        dimensions: usize,

        /// Distance metric: cosine, euclidean, dot.
        #[arg(long, default_value = "cosine")]
        metric: String,

        /// Skip server startup.
        #[arg(long)]
        no_serve: bool,

        /// Launch Sutra Studio after setup.
        #[arg(long)]
        launch_studio: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.command {
        Commands::Serve {
            port,
            data_dir,
            memory_only,
            passcode,
        } => {
            let state = if memory_only {
                tracing::info!("Running in-memory only (no persistence)");
                Arc::new(sutra_proto::AppState {
                    store: RwLock::new(sutra_core::TripleStore::new()),
                    dict: RwLock::new(sutra_core::TermDictionary::new()),
                    vectors: RwLock::new(sutra_hnsw::VectorRegistry::new()),
                    persistent: None,
                    passcode: passcode.clone(),
                    rate_limit_per_min: 0,
                    rate_counter: std::sync::atomic::AtomicU64::new(0),
                })
            } else {
                tracing::info!("Opening persistent store at {}", data_dir);
                let ps = sutra_core::PersistentStore::open(&data_dir)?;

                let mut dict = sutra_core::TermDictionary::new();
                let mut store = sutra_core::TripleStore::new();

                let term_count = ps.load_terms_into(&mut dict);
                tracing::info!("Loaded {} terms from disk", term_count);

                let mut triple_count = 0usize;
                for triple in ps.iter() {
                    let _ = store.insert(triple);
                    triple_count += 1;
                }
                tracing::info!("Loaded {} triples from disk", triple_count);

                // Rebuild HNSW indexes from stored vector triples
                let mut vectors = sutra_hnsw::VectorRegistry::new();
                let mut vec_count = 0usize;
                let f32vec_suffix = "^^<http://sutra.dev/f32vec>";
                for triple in store.iter() {
                    if let Some(obj_str) = dict.resolve(triple.object) {
                        if obj_str.contains(f32vec_suffix) {
                            // Parse the vector literal
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
                                        // Ensure predicate is declared
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
                                        let _ =
                                            vectors.insert(triple.predicate, floats, triple.object);
                                        vec_count += 1;
                                    }
                                }
                            }
                        }
                    }
                }
                if vec_count > 0 {
                    tracing::info!("Rebuilt {} vectors in HNSW indexes from disk", vec_count);
                }

                Arc::new(sutra_proto::AppState {
                    store: RwLock::new(store),
                    dict: RwLock::new(dict),
                    vectors: RwLock::new(vectors),
                    persistent: Some(ps),
                    passcode: passcode.clone(),
                    rate_limit_per_min: 0,
                    rate_counter: std::sync::atomic::AtomicU64::new(0),
                })
            };

            let app = sutra_proto::router(state);
            let addr = format!("0.0.0.0:{}", port);
            tracing::info!("SutraDB listening on {}", addr);

            let listener = tokio::net::TcpListener::bind(&addr).await?;
            axum::serve(listener, app).await?;
        }

        Commands::Query { query, data_dir } => {
            let ps = sutra_core::PersistentStore::open(&data_dir)?;

            let mut dict = sutra_core::TermDictionary::new();
            let mut store = sutra_core::TripleStore::new();

            ps.load_terms_into(&mut dict);
            for triple in ps.iter() {
                let _ = store.insert(triple);
            }

            let vectors = sutra_hnsw::VectorRegistry::new();

            let mut parsed = sutra_sparql::parse(&query)?;
            sutra_sparql::optimize(&mut parsed);
            let result = sutra_sparql::execute_with_vectors(&parsed, &store, &dict, &vectors)?;

            // Print results as a simple table
            if result.columns.len() == 1 && result.columns[0] == "result" {
                // ASK query
                if let Some(row) = result.rows.first() {
                    if let Some(&id) = row.get("result") {
                        if sutra_core::decode_inline_boolean(id) == Some(true) {
                            println!("true");
                        } else {
                            println!("false");
                        }
                    }
                }
            } else {
                // SELECT query
                println!("{}", result.columns.join("\t"));
                println!("{}", "-".repeat(result.columns.len() * 20));
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
                    println!("{}", vals.join("\t"));
                }
                println!("\n{} rows", result.rows.len());
            }
        }

        Commands::Import { file, data_dir } => {
            let ps = sutra_core::PersistentStore::open(&data_dir)?;

            let reader: Box<dyn BufRead> = if file == "-" {
                Box::new(std::io::stdin().lock())
            } else {
                let f = std::fs::File::open(&file)?;
                Box::new(std::io::BufReader::new(f))
            };

            let mut inserted = 0usize;
            let mut errors = 0usize;
            let mut line_no = 0usize;

            for line in reader.lines() {
                let line = line?;
                line_no += 1;

                let parsed = match sutra_core::parse_ntriples_line(&line) {
                    Some(t) => t,
                    None => continue,
                };

                let (subj_str, pred_str, obj_str) = parsed;
                let s_id = ps.intern(&subj_str)?;
                let p_id = ps.intern(&pred_str)?;
                let o_id = ps.intern(&obj_str)?;

                match ps.insert(sutra_core::Triple::new(s_id, p_id, o_id)) {
                    Ok(()) => inserted += 1,
                    Err(_) => errors += 1,
                }

                #[allow(clippy::manual_is_multiple_of)]
                if inserted > 0 && inserted % 10000 == 0 {
                    eprintln!("  {} triples imported (line {})", inserted, line_no);
                }
            }

            ps.flush()?;
            println!(
                "Imported {} triples ({} errors) from {}",
                inserted, errors, file
            );
        }

        Commands::Export {
            data_dir,
            output,
            format,
        } => {
            let ps = sutra_core::PersistentStore::open(&data_dir)?;

            let mut writer: Box<dyn Write> = if let Some(path) = &output {
                Box::new(std::fs::File::create(path)?)
            } else {
                Box::new(std::io::stdout().lock())
            };

            let mut count = 0usize;
            for triple in ps.iter() {
                let s = ps
                    .resolve(triple.subject)?
                    .unwrap_or_else(|| format!("_:id{}", triple.subject));
                let p = ps
                    .resolve(triple.predicate)?
                    .unwrap_or_else(|| format!("_:id{}", triple.predicate));
                let o = ps
                    .resolve(triple.object)?
                    .unwrap_or_else(|| resolve_object_persistent(triple.object, &ps));

                if format == "ttl" || format == "turtle" {
                    // Simplified Turtle (no prefix compression for CLI)
                    if s.starts_with("_:") {
                        write!(writer, "{}", s)?;
                    } else {
                        write!(writer, "<{}>", s)?;
                    }
                    write!(writer, " <{}> ", p)?;
                    if o.starts_with('"') || o.starts_with("_:") {
                        writeln!(writer, "{} .", o)?;
                    } else {
                        writeln!(writer, "<{}> .", o)?;
                    }
                } else {
                    // N-Triples
                    if s.starts_with("_:") {
                        write!(writer, "{}", s)?;
                    } else {
                        write!(writer, "<{}>", s)?;
                    }
                    write!(writer, " <{}> ", p)?;
                    if o.starts_with('"') || o.starts_with("_:") {
                        writeln!(writer, "{} .", o)?;
                    } else {
                        writeln!(writer, "<{}> .", o)?;
                    }
                }
                count += 1;
            }

            if output.is_some() {
                eprintln!("Exported {} triples", count);
            }
        }

        Commands::Info { data_dir } => {
            let ps = sutra_core::PersistentStore::open(&data_dir)?;
            let triple_count = ps.len();

            // Count terms
            let mut dict = sutra_core::TermDictionary::new();
            let term_count = ps.load_terms_into(&mut dict);

            println!("SutraDB — {}", data_dir);
            println!("  Triples: {}", triple_count);
            println!("  Terms:   {}", term_count);
        }

        Commands::InstallAgent {
            name,
            port,
            passcode,
            dimensions,
            metric,
            no_serve,
            launch_studio,
        } => {
            let data_dir = format!("./{}", name);
            let notes_file = format!("{}_sutra_notes.md", name);

            println!("# SutraDB Agent Installer");
            println!();
            println!("Setting up database: {}", name);
            println!("  Data directory: {}", data_dir);
            println!("  Port: {}", port);
            println!(
                "  Authentication: {}",
                if passcode.is_some() {
                    "enabled"
                } else {
                    "none"
                }
            );
            println!("  Default vector dimensions: {}", dimensions);
            println!("  Distance metric: {}", metric);
            println!();

            // Create the persistent store
            let ps = sutra_core::PersistentStore::open(&data_dir)?;
            ps.flush()?;
            println!("[OK] Database created at {}", data_dir);

            // Generate notes file
            let auth_note = match &passcode {
                Some(p) => format!(
                    "Authentication is enabled. Use header: `Authorization: Bearer {}`",
                    p
                ),
                None => "No authentication configured. All requests are accepted.".to_string(),
            };
            let serve_flag = match &passcode {
                Some(p) => format!(" --passcode {}", p),
                None => String::new(),
            };
            let auth_header = match &passcode {
                Some(p) => format!(" \\\n  -H \"Authorization: Bearer {}\"", p),
                None => " \\".to_string(),
            };
            let auth_header_query = match &passcode {
                Some(p) => format!(" \\\n  -H \"Authorization: Bearer {}\"", p),
                None => String::new(),
            };

            let notes = format!(
                r#"# SutraDB Setup Notes — {}

## Configuration
- **Database:** {}
- **Data directory:** `{}`
- **Port:** {}
- **Authentication:** {}
- **Default vector dimensions:** {}
- **Distance metric:** {}

## Quick Start
```bash
# Start the server
sutra serve --port {} --data-dir {}{}

# Check health
curl http://localhost:{}/health

# Insert triples
curl -X POST http://localhost:{}/triples \
  -H "Content-Type: text/plain"{}  -d '<http://example.org/s> <http://example.org/p> <http://example.org/o> .'

# Query
curl -X POST http://localhost:{}/sparql{} \
  -d 'SELECT * WHERE {{ ?s ?p ?o }} LIMIT 10'
```

## Endpoints
| Endpoint | Method | Description |
|----------|--------|-------------|
| `/sparql` | GET/POST | SPARQL queries |
| `/triples` | POST | Insert N-Triples |
| `/vectors/declare` | POST | Declare vector predicate |
| `/vectors` | POST | Insert vector |
| `/graph` | GET | Export as Turtle |
| `/health` | GET | Health check |
| `/vectors/health` | GET | HNSW diagnostics |

## Generated by
SutraDB Agent Installer v0.1.0
"#,
                name,
                name,
                data_dir,
                port,
                auth_note,
                dimensions,
                metric,
                port,
                data_dir,
                serve_flag,
                port,
                port,
                auth_header,
                port,
                auth_header_query,
            );

            std::fs::write(&notes_file, &notes)?;
            println!("[OK] Notes written to {}", notes_file);

            if launch_studio {
                println!("Launching Sutra Studio...");
                #[cfg(target_os = "windows")]
                {
                    let _ = std::process::Command::new("cmd")
                        .args(["/c", "start", "", "flutter", "run", "-d", "windows"])
                        .current_dir("sutra-studio")
                        .spawn();
                }
                #[cfg(not(target_os = "windows"))]
                {
                    let _ = std::process::Command::new("flutter")
                        .args(["run", "-d", "linux"])
                        .current_dir("sutra-studio")
                        .spawn();
                }
            }

            if !no_serve {
                println!();
                println!("Starting SutraDB server...");
                println!("  sutra serve --port {} --data-dir {}", port, data_dir);
                println!();

                // Actually start the server
                let ps2 = sutra_core::PersistentStore::open(&data_dir)?;
                let mut dict = sutra_core::TermDictionary::new();
                let mut store = sutra_core::TripleStore::new();
                ps2.load_terms_into(&mut dict);
                for triple in ps2.iter() {
                    let _ = store.insert(triple);
                }

                let state = Arc::new(sutra_proto::AppState {
                    store: RwLock::new(store),
                    dict: RwLock::new(dict),
                    vectors: RwLock::new(sutra_hnsw::VectorRegistry::new()),
                    persistent: Some(ps2),
                    passcode,
                    rate_limit_per_min: 0,
                    rate_counter: std::sync::atomic::AtomicU64::new(0),
                });

                let app = sutra_proto::router(state);
                let addr = format!("0.0.0.0:{}", port);
                tracing::info!("SutraDB listening on {}", addr);
                let listener = tokio::net::TcpListener::bind(&addr).await?;
                axum::serve(listener, app).await?;
            }
        }
    }

    Ok(())
}

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

fn resolve_object_persistent(id: sutra_core::TermId, ps: &sutra_core::PersistentStore) -> String {
    if let Some(n) = sutra_core::decode_inline_integer(id) {
        return format!("\"{}\"^^<http://www.w3.org/2001/XMLSchema#integer>", n);
    }
    if let Some(b) = sutra_core::decode_inline_boolean(id) {
        return format!("\"{}\"^^<http://www.w3.org/2001/XMLSchema#boolean>", b);
    }
    ps.resolve(id)
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("_:id{}", id))
}
