//! SutraDB CLI: server, query, import.

use std::sync::{Arc, Mutex};

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

        /// Data directory for persistent storage.
        #[arg(short, long, default_value = "./sutra-data")]
        data_dir: String,
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
        Commands::Serve { port, data_dir } => {
            let _ = data_dir;
            let state = Arc::new(sutra_proto::AppState {
                store: Mutex::new(sutra_core::TripleStore::new()),
                dict: Mutex::new(sutra_core::TermDictionary::new()),
                vectors: Mutex::new(sutra_hnsw::VectorRegistry::new()),
            });

            let app = sutra_proto::router(state);
            let addr = format!("0.0.0.0:{}", port);
            tracing::info!("SutraDB listening on {}", addr);

            let listener = tokio::net::TcpListener::bind(&addr).await?;
            axum::serve(listener, app).await?;
        }
        Commands::Query { query, data_dir } => {
            let _ = data_dir;
            let dict = sutra_core::TermDictionary::new();
            let store = sutra_core::TripleStore::new();
            let mut vectors = sutra_hnsw::VectorRegistry::new();

            let mut parsed = sutra_sparql::parse(&query)?;
            sutra_sparql::optimize(&mut parsed);
            let result =
                sutra_sparql::execute_with_vectors(&parsed, &store, &dict, &mut vectors)?;

            println!("Columns: {:?}", result.columns);
            println!("Rows: {}", result.rows.len());
            for row in &result.rows {
                println!("  {:?}", row);
            }
        }
    }

    Ok(())
}
