
####################################
# STAGE 1: Build the binary
####################################
FROM rust:1.86-slim AS builder

# Install dependencies for building
RUN apt-get update && \
    apt-get install -y pkg-config libssl-dev build-essential && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app

COPY Cargo.toml Cargo.lock ./
# Create a dummy src/main.rs to allow fetching dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs
# Build dependencies first to leverage Docker layer caching
RUN cargo build --release --features "stdio,sse" 
RUN rm -f target/release/deps/mcp_rg_enhanced* # Remove dummy artifacts

# Copy full source code
COPY src ./src

# Build the actual project with all features
RUN cargo build --release --features "stdio,sse"

####################################
# STAGE 2: Create the runtime image
####################################
FROM debian:bookworm-slim

# Install ripgrep and minimal runtime dependencies
RUN apt-get update && \
    apt-get install -y ripgrep ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Create a non-root user
RUN groupadd -r mcpuser && useradd -r -g mcpuser mcpuser

# Copy the binary from builder stage
COPY --from=builder /usr/src/app/target/release/mcp-rg-editor /usr/local/bin/

WORKDIR /app

# Default environment variables (can be overridden at runtime)
ENV FILES_ROOT=/app/files
ENV ALLOWED_DIRECTORIES=/app/files 
ENV BLOCKED_COMMANDS="sudo,su,rm,mkfs,fdisk,dd,reboot,shutdown,poweroff,halt"
ENV LOG_LEVEL=info
ENV MCP_TRANSPORT=stdio
ENV MCP_SSE_HOST=0.0.0.0
ENV MCP_SSE_PORT=3000
ENV FILE_READ_LINE_LIMIT=1000
ENV FILE_WRITE_LINE_LIMIT=50

# Create and own the files directory for volume mounting
RUN mkdir -p /app/files && \
    chown -R mcpuser:mcpuser /app

# Expose port for SSE transport
EXPOSE 3000

USER mcpuser

ENTRYPOINT ["mcp-rg-editor"]
