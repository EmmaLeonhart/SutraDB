#!/bin/bash
# Install SutraDB CLI.
# Requires Rust toolchain (cargo) to be installed.

set -e

echo "Building SutraDB (release)..."
cargo build --release -p sutra-cli

INSTALL_DIR="${HOME}/.sutra/bin"
mkdir -p "$INSTALL_DIR"

echo "Installing to $INSTALL_DIR/sutra ..."
cp target/release/sutra "$INSTALL_DIR/sutra"
chmod +x "$INSTALL_DIR/sutra"

echo ""
echo "Done! Add $INSTALL_DIR to your PATH if not already there:"
echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
echo ""
echo "Usage:"
echo "  sutra serve                    Start the HTTP server"
echo "  sutra query \"SELECT ...\"       Run a SPARQL query"
echo "  sutra import data.nt           Import N-Triples file"
echo "  sutra export -o dump.nt        Export all triples"
echo "  sutra info                     Show database statistics"
echo ""
