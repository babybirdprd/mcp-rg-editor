# MCP Enhanced Server (mcp-rg-editor)

This is a powerful MCP (Model Context Protocol) server built in Rust, extending the capabilities of the original `randomm/mcp-rg`. It provides AI models like Anthropic's Claude with a comprehensive suite of tools to interact with a local system, including:

-   **Code Search:** Efficient code search using Ripgrep (`rg`).
-   **Filesystem Operations:** Read, write, list, move files and directories.
-   **Terminal Command Execution:** Run commands, manage sessions, and stream output.
-   **Process Management:** List and terminate system processes.
-   **Text Editing:** Perform targeted text replacements in files.
-   **Configuration Management:** View and (soon) modify server settings.

The server supports both **STDIO** and **Server-Sent Events (SSE)** transport protocols.

## Features

-   **Ripgrep Integration (`search_code`):**
    -   Fast regex and literal string search.
    -   Path and file-type filtering.
    -   Context lines, line numbers, case sensitivity options.
-   **Filesystem Tools:**
    -   `read_file`: Read file content with line-based offset and length.
    -   `write_file`: Write or append content, respecting line limits (requires chunking for large writes).
    -   `create_directory`: Create directories, including nested ones.
    -   `list_directory`: List files and subdirectories.
    -   `move_file`: Move or rename files/directories.
    -   `search_files`: Simple search for files/directories by name (substring match).
    -   `get_file_info`: Retrieve metadata like size, timestamps, permissions.
-   **Terminal Tools:**
    -   `execute_command`: Execute shell commands with configurable timeout and shell. Supports background execution.
    -   `read_output`: Stream output from ongoing commands.
    -   `force_terminate`: Kill a running command session.
    -   `list_sessions`: List active command sessions.
-   **Process Tools:**
    -   `list_processes`: List system processes with PID, name, CPU/memory usage.
    -   `kill_process`: Terminate a system process by PID.
-   **Editing Tools:**
    -   `edit_block`: Replace exact occurrences of a string in a file. Supports `expected_replacements` parameter.
-   **Configuration Tools:**
    -   `get_config`: View current server configuration (FILES_ROOT, ALLOWED_DIRECTORIES, etc.).
    -   `set_config_value`: (Planned) Modify server configuration dynamically. Currently, config is via environment variables.
-   **Security:**
    -   `FILES_ROOT`: All operations are confined within this root directory.
    -   `ALLOWED_DIRECTORIES`: Fine-grained access control for filesystem tools.
    -   `BLOCKED_COMMANDS`: Prevent execution of potentially harmful commands.
    -   Path traversal prevention.
-   **Dual Transport:**
    -   STDIO: For local integration (e.g., Claude Desktop, CLI tools).
    -   SSE (HTTP): For network-based access, allowing multiple clients. (Requires `sse` feature).

## Prerequisites

-   Rust (latest stable, e.g., 1.70+).
-   Ripgrep (`rg`) installed and in your system's PATH (for `search_code` tool).
-   Docker (optional, for containerized deployment).

## Installation & Build

1.  **Clone the repository:**
    ```bash
    git clone <repository_url>
    cd mcp-rg-editor
    ```

2.  **Build the server:**
    *   For both STDIO and SSE support (default):
        ```bash
        cargo build --release --features "stdio,sse"
        ```
    *   For STDIO only:
        ```bash
        cargo build --release --features "stdio" --no-default-features
        ```
    The binary will be at `target/release/mcp-rg-editor`.

## Configuration

Configuration is managed via environment variables. Create a `.env` file in the project root or set them directly. See `.env.example` for all options.

**Key Environment Variables:**

-   `FILES_ROOT`: (Required) Absolute path to the root directory the server can operate within.
-   `ALLOWED_DIRECTORIES`: Comma-separated list of absolute paths the server can access. Defaults to `FILES_ROOT`. For full filesystem access (DANGEROUS), set to `/` (Linux/macOS) or `C:\` (Windows - though be mindful of drive letters).
-   `BLOCKED_COMMANDS`: Comma-separated list of command names (e.g., `rm,sudo`) to block.
-   `DEFAULT_SHELL`: Shell for `execute_command` (e.g., `bash`, `powershell`). System default if not set.
-   `LOG_LEVEL`: `trace`, `debug`, `info`, `warn`, `error`. Default: `info`.
-   `MCP_TRANSPORT`: `stdio` (default) or `sse`.
-   `MCP_SSE_HOST`: Host for SSE (e.g., `0.0.0.0`). Default: `127.0.0.1`.
-   `MCP_SSE_PORT`: Port for SSE (e.g., `3000`). Default: `3000`.
-   `FILE_READ_LINE_LIMIT`: Max lines for `read_file`. Default: `1000`.
-   `FILE_WRITE_LINE_LIMIT`: Max lines for `write_file` per call. Default: `50`.

## Running the Server

### STDIO Mode (Default)

```bash
# Set environment variables (or use a .env file)
export FILES_ROOT=/path/to/your/projects
export ALLOWED_DIRECTORIES=/path/to/your/projects/project_a,/path/to/your/projects/project_b
# ... other env vars

./target/release/mcp-rg-editor
```
The server will listen for JSON-RPC messages on STDIN and send responses to STDOUT. Logs go to STDERR.

### SSE Mode

Ensure the `sse` feature was included during build.

```bash
# Set environment variables
export FILES_ROOT=/path/to/your/projects
export MCP_TRANSPORT=sse
export MCP_SSE_HOST=0.0.0.0 # Listen on all interfaces
export MCP_SSE_PORT=8080
# ... other env vars

./target/release/mcp-rg-editor
```
The server will start an HTTP server listening on `MCP_SSE_HOST:MCP_SSE_PORT`. MCP communication happens over an SSE endpoint (typically `/mcp`).

## Docker Deployment

A `Dockerfile` and `docker-compose.yml` are provided.

1.  **Build the Docker image:**
    ```bash
    docker build -t mcp-rg-editor .
    ```

2.  **Run with `docker run`:**

    *   **STDIO Mode:**
        ```bash
        docker run -i --rm \
          -v /path/on/host/projects:/app/files \
          -e FILES_ROOT=/app/files \
          -e ALLOWED_DIRECTORIES=/app/files \
          -e MCP_TRANSPORT=stdio \
          mcp-rg-editor
        ```
        The `-i` flag is crucial for STDIO.

    *   **SSE Mode:**
        ```bash
        docker run -d --rm \
          -p 8080:3000 \
          -v /path/on/host/projects:/app/files \
          -e FILES_ROOT=/app/files \
          -e ALLOWED_DIRECTORIES=/app/files \
          -e MCP_TRANSPORT=sse \
          -e MCP_SSE_HOST=0.0.0.0 \
          -e MCP_SSE_PORT=3000 \
          --name mcp-server \
          mcp-rg-editor
        ```
        This maps container port 3000 to host port 8080.

3.  **Run with `docker-compose`:**
    Modify `docker-compose.yml` to set your desired volume mounts and environment variables.
    ```bash
    docker-compose up -d # For detached mode
    # For STDIO mode, you might need: docker-compose run --rm mcp-server (ensure tty:false and stdin_open:true)
    ```

## MCP Client Setup (e.g., Claude Desktop)

Refer to the `claude_desktop_config.json` examples below.

### STDIO Example (Claude Desktop)

Replace `/abs/path/to/mcp-rg-editor` and paths for `FILES_ROOT`, `ALLOWED_DIRECTORIES`.
```json
{
  "mcpServers": {
    "desktop_enhanced": {
      "command": "/abs/path/to/mcp-rg-editor/target/release/mcp-rg-editor",
      "env": {
        "FILES_ROOT": "/home/user/my_code",
        "ALLOWED_DIRECTORIES": "/home/user/my_code/project1,/home/user/my_code/project2",
        "BLOCKED_COMMANDS": "sudo,rm",
        "LOG_LEVEL": "info",
        "MCP_TRANSPORT": "stdio",
        "RUST_LOG": "info", // For tracing subscriber
        "RUST_BACKTRACE": "1"
      }
    }
  }
}
```

### SSE Example (Conceptual - Claude Desktop doesn't directly support HTTP MCPs yet)

If a client supports HTTP/SSE MCPs, you would point it to `http://<MCP_SSE_HOST>:<MCP_SSE_PORT>/mcp`.
For Claude Desktop to use an SSE server, you'd typically need a local proxy that converts Claude Desktop's STDIO expectation into an HTTP request to your SSE server. This is an advanced setup.

### Docker with STDIO (Claude Desktop)

```json
{
  "mcpServers": {
    "desktop_enhanced_docker": {
      "command": "docker",
      "args": [
        "run", "-i", "--rm",
        "-v", "/path/on/host/projects:/app/files",
        "-e", "FILES_ROOT=/app/files",
        "-e", "ALLOWED_DIRECTORIES=/app/files", // Adjust as needed inside container
        "-e", "LOG_LEVEL=debug",
        "-e", "RUST_LOG=debug",
        "-e", "MCP_TRANSPORT=stdio",
        "mcp-rg-editor:latest" // Your built image name
      ]
    }
  }
}
```

## Available Tools

*(A brief summary of tools will be listed here, similar to Desktop Commander's README)*

| Category        | Tool                | Description                                                                 |
|-----------------|---------------------|-----------------------------------------------------------------------------|
| **Configuration**| `get_config`        | Get current server configuration.                                           |
|                 | `set_config_value`  | (Planned) Set a server configuration value.                                 |
| **Filesystem**  | `read_file`         | Read file content with line offset/length.                                  |
|                 | `write_file`        | Write/append to files, respects line limits.                                |
|                 | `create_directory`  | Create directories.                                                         |
|                 | `list_directory`    | List directory contents.                                                    |
|                 | `move_file`         | Move/rename files or directories.                                           |
|                 | `search_files`      | Find files by name (substring match).                                       |
|                 | `get_file_info`     | Get file/directory metadata.                                                |
| **Code Search** | `search_code`       | Search code with Ripgrep (regex, file types, context).                      |
| **Text Editing**| `edit_block`        | Replace exact string occurrences in a file.                                 |
| **Terminal**    | `execute_command`   | Run terminal commands, with timeout and background execution.                 |
|                 | `read_output`       | Get new output from a running command.                                      |
|                 | `force_terminate`   | Stop a running command session.                                             |
|                 | `list_sessions`     | List active command sessions.                                               |
| **Process**     | `list_processes`    | List system processes.                                                      |
|                 | `kill_process`      | Terminate a process by PID.                                                 |

## Security

-   **`FILES_ROOT`**: Enforces a top-level boundary.
-   **`ALLOWED_DIRECTORIES`**: Provides more granular control. Paths outside these (but within `FILES_ROOT`) are inaccessible to filesystem tools.
-   **`BLOCKED_COMMANDS`**: Prevents execution of specified commands. Uses regex for matching the command itself (first word).
-   Path canonicalization and validation are used to prevent traversal attacks.
-   Run the server with the least privileges necessary.
-   When using Docker, mount only necessary host directories.

## Troubleshooting

-   Check `LOG_LEVEL` and `RUST_LOG` for detailed server logs (sent to STDERR).
-   Ensure `rg` is installed for `search_code`.
-   Verify `FILES_ROOT` and `ALLOWED_DIRECTORIES` are absolute paths and exist.
-   For Docker STDIO: ensure `-i` flag is used with `docker run`.
-   For Docker SSE: ensure ports are correctly mapped (`-p host:container`).

## Development

-   Format code: `cargo fmt`
-   Lint code: `cargo clippy --features "stdio,sse"`
-   Run tests: `cargo test --features "stdio,sse"` (Test suite needs to be expanded)

## Contributing

Contributions are welcome! Please fork the repository, create a feature branch, and open a pull request.

## License

This project is licensed under the MIT License. See `LICENSE` file (if one was provided in `randomm/mcp-rg`, otherwise assume MIT as per its README).
