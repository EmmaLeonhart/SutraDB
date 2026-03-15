# Build stage
FROM rust:1.82-slim AS builder
WORKDIR /src
COPY . .
RUN cargo build --release -p sutra-cli

# Runtime stage
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /src/target/release/sutra /usr/local/bin/sutra

# Default data directory
RUN mkdir -p /data
VOLUME /data

EXPOSE 3030

ENTRYPOINT ["sutra"]
CMD ["serve", "--port", "3030", "--data-dir", "/data"]
