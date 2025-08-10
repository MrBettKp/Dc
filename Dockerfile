# Use the official Rust nightly image
FROM rustlang/rust:nightly-slim

# Install system dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Set working directory
WORKDIR /app

# Copy Cargo.toml first for better caching
COPY Cargo.toml ./

# Create src directory and a dummy main.rs for dependency caching
RUN mkdir src && echo "fn main() {}" > src/main.rs

# Build dependencies (this layer will be cached)
RUN cargo +nightly build --release
RUN rm src/main.rs

# Copy source code
COPY src/ src/

# Build the application
RUN cargo +nightly build --release

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
CMD ["indexer", "--wallet", "7cMEhpt9y3inBNVv8fNnuaEbx7hKHZnLvR1KWKKxuDDU"]