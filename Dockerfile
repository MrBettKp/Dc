# Use the official Rust image with version 1.82+
FROM rust:1.82-slim

# Install system dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Set working directory
WORKDIR /app

# Copy Cargo files first for better caching
COPY Cargo.toml Cargo.lock ./

# Create src directory and a dummy main.rs for dependency caching
RUN mkdir src && echo "fn main() {}" > src/main.rs

# Build dependencies (this layer will be cached)
RUN cargo build --release
RUN rm src/main.rs

# Copy source code
COPY src/ src/

# Build the application
RUN cargo build --release

# Create a smaller runtime image
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Copy the binary from builder stage
COPY --from=0 /app/target/release/indexer /usr/local/bin/indexer

# Create a non-root user
RUN useradd -r -s /bin/false indexer
USER indexer

# Default command
ENTRYPOINT ["indexer"]
CMD ["--help"]