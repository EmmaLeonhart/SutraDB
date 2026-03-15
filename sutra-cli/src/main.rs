//! SutraDB CLI: server, query, import.

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
    },
    /// Execute a SPARQL query from the command line.
    Query {
        /// The SPARQL query string.
        query: String,

        /// Data directory.
        #[arg(short, long, default_value = "./sutra-data")]
        data_dir: String,
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
        } => {
            let state = if memory_only {
                tracing::info!("Running in-memory only (no persistence)");
                Arc::new(sutra_proto::AppState {
                    store: RwLock::new(sutra_core::TripleStore::new()),
                    dict: RwLock::new(sutra_core::TermDictionary::new()),
                    vectors: RwLock::new(sutra_hnsw::VectorRegistry::new()),
                    persistent: None,
                })
            } else {
                tracing::info!("Opening persistent store at {}", data_dir);
                let ps = sutra_core::PersistentStore::open(&data_dir)?;

                // Hydrate in-memory stores from persistent storage
                let mut dict = sutra_core::TermDictionary::new();
                let mut store = sutra_core::TripleStore::new();

                // Load all terms into the in-memory dictionary
                let term_count = ps.load_terms_into(&mut dict);
                tracing::info!("Loaded {} terms from disk", term_count);

                // Load all triples into the in-memory store
                let mut triple_count = 0usize;
                for triple in ps.iter() {
                    let _ = store.insert(triple);
                    triple_count += 1;
                }
                tracing::info!("Loaded {} triples from disk", triple_count);

                Arc::new(sutra_proto::AppState {
                    store: RwLock::new(store),
                    dict: RwLock::new(dict),
                    vectors: RwLock::new(sutra_hnsw::VectorRegistry::new()),
                    persistent: Some(ps),
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

            // Hydrate in-memory stores
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

            println!("Columns: {:?}", result.columns);
            println!("Rows: {}", result.rows.len());
            for row in &result.rows {
                println!("  {:?}", row);
            }
        }
    }

    Ok(())
}
