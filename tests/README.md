# Test suite for mcp-rg-editor (Desktop Commander Enhanced)

This directory contains integration and utility tests for the project.

## Running Tests

### Prerequisites
1.  Build the server: `cargo build --release --features "stdio,sse"`
2.  Ensure `rg` (ripgrep) is installed and in your PATH.
3.  Ensure Docker is running if you want to run Docker-based tests.

### Local STDIO Tests
The `test_local_mcp.sh` script tests the server binary directly using STDIO.
It sets up a `test_data` directory, sends a batch of MCP requests, and checks responses.

```bash
./tests/test_local_mcp.sh
```

### Docker STDIO Tests
The `test_mcp_docker.sh` script tests the server running inside a Docker container, also using STDIO.
It mounts a `test_data_docker` directory into the container.

```bash
# First, build the Docker image if you haven't:
# docker build -t mcp-rg-editor:latest .
./tests/test_mcp_docker.sh
```

### Adding New Tests
-   Modify the `REQUEST_BATCH` in the shell scripts to include calls to new tools or test different parameters.
-   Add corresponding checks for the responses.
-   For more complex scenarios or unit testing of Rust modules, consider adding Rust integration tests (e.g., files in a `tests` directory at the crate root, or module tests within `src`).

## Test Coverage
The current shell scripts provide high-level integration testing for:
-   Server initialization and tool listing.
-   Core functionality of most tools:
    -   `get_config`
    -   `read_file`
    -   `search_code`
    -   `execute_command`
    -   `list_processes`
    -   `edit_block` (basic case)
    -   `create_directory`
    -   `list_directory`
-   Basic path handling within `FILES_ROOT`.
-   STDIO transport for both local binary and Docker container.

Further tests could cover:
-   SSE transport.
-   More edge cases for `edit_block` (fuzzy matching, multiple replacements).
-   Detailed `ALLOWED_DIRECTORIES` and `BLOCKED_COMMANDS` scenarios.
-   Tilde (`~`) expansion in various path inputs.
-   URL reading in `read_file`.
-   Timeout behaviors for `search_files`, `search_code`, `execute_command`.
-   Log rotation for audit and fuzzy search logs.