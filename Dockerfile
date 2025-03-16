# Builder stage
FROM rust:1.81-slim-bullseye as builder
WORKDIR /app

# Copy only the dependency manifests first to cache dependencies
COPY Cargo.toml .

# Create a dummy main.rs to build dependencies
RUN mkdir -p src && \
    echo "fn main() {}" > src/main.rs && \
    cargo build --release && \
    rm -rf src

# Now copy the actual source code
COPY src/ src/

# Force cargo to rebuild only the application code
RUN touch src/main.rs && \
    cargo build --release

# Runtime stage - using Ubuntu for better GLIBC compatibility
FROM ubuntu:22.04
WORKDIR /app

# Install only the required runtime dependencies
RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Copy the binary from the builder stage
COPY --from=builder /app/target/release/keyvalue-cache .

# Expose port 7171
EXPOSE 7171

# Run with a non-root user for security
RUN useradd -m appuser
USER appuser

# Configure environment for t3.small (2 cores, 2GB RAM)
ENV RUST_LOG=info
ENV WORKERS=2

# Run the binary
CMD ["./keyvalue-cache"]
