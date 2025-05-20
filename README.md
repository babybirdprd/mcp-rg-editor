# Desktop Commander Enhanced (Rust MCP Server)

This is a powerful MCP (Model Context Protocol) server built in Rust, significantly extending the capabilities of the original `randomm/mcp-rg` and heavily inspired by the feature set of the Node.js-based [Desktop Commander MCP](https://github.com/wonderwhy-er/DesktopCommanderMCP). It provides AI models like Anthropic's Claude with a comprehensive suite of tools to interact with a local system, including:

-   **Advanced Code Search:** Efficient code search using Ripgrep (`rg`).
-   **Full Filesystem Operations:** Read, write, list, move files and directories, with support for URL reading and image handling.
-   **Robust Terminal Command Execution:** Run commands with timeouts, manage background sessions, stream output, and select shells.
-   **Process Management:** List and terminate system processes.
-   **Surgical Text Editing:** Perform targeted text replacements in files with fuzzy matching feedback and line ending preservation.
-   **Configuration Management:** View and (in-memory) modify server settings via MCP tools.

The server supports both **STDIO** and **Server-Sent Events (SSE)** transport protocols.

## Features

-   **Execute terminal commands** with output streaming (`execute_command`, `read_output`).
-   **Command timeout** and **background execution** support.
-   **Process management** (`list_processes`, `kill_process`).
-   **Session management** for long-running commands (`list_sessions`, `force_terminate`).
-   **Server configuration management** (in-memory via MCP tools):
    -   `get_config`: View current settings (FILES_ROOT, ALLOWED_DIRECTORIES, BLOCKED_COMMANDS, etc.).
    -   `set_config_value`: Modify settings like `allowedDirectories`, `blockedCommands`, `defaultShell` for the current session.
-   **Full filesystem operations:**
    -   `read_file`: Read local files (with line offset/length for text) or content from URLs. Handles text and common image types (PNG, JPEG, GIF, WebP displayed as images).
    -   `read_multiple_files`: Read multiple local files simultaneously.
    -   `write_file`: Write or append to files, respecting `fileWriteLineLimit` (requires chunking for large writes).
    -   `create_directory`: Create directories, including nested ones.
    -   `list_directory`: List files and subdirectories with `[FILE]` / `[DIR]` prefixes.
    -   `move_file`: Move or rename files/directories.
    -   `search_files`: Find files/directories by name (case-insensitive substring match) with configurable timeout.
    -   `get_file_info`: Retrieve metadata like size, timestamps, permissions (octal on Unix).
-   **Code editing capabilities (`edit_block`):**
    -   Surgical text replacements.
    -   `expected_replacements` parameter to control number of changes.
    -   Fuzzy search fallback with character-level diff feedback (`{-removed-}{+added+}`) when exact match fails.
    -   Automatic line ending detection and preservation.
    -   Logging of fuzzy search attempts for analysis.
-   **Advanced Code Search (`search_code` via Ripgrep):**
    -   Fast regex and literal string search.
    -   Path, glob, and file-type filtering.
    -   Context lines, line numbers, case sensitivity options.
    -   Configurable timeout.
-   **Security:**
    -   `FILES_ROOT`: All operations are confined within this root directory.
    -   `ALLOWED_DIRECTORIES`: Fine-grained access control for filesystem tools. Supports tilde (`~`) expansion. An empty list defaults to `FILES_ROOT`. `/` or `C:\` grants full (dangerous) access.
    -   `BLOCKED_COMMANDS`: Prevent execution of potentially harmful commands (checks first word of command).
    -   Path traversal prevention and tilde (`~`) expansion for user convenience.
-   **Dual Transport:**
    -   STDIO: For local integration (e.g., Claude Desktop, CLI tools).
    -   SSE (HTTP): For network-based access. (Requires `sse` feature).
-   **Comprehensive Audit Logging:**
    -   All tool calls are logged with timestamp, tool name, and sanitized arguments.
    -   Log rotation based on size.
-   **Fuzzy Search Logging:**
    -   Detailed logs for `edit_block` fuzzy search attempts, aiding in debugging failed edits.

## Prerequisites

-   Rust (latest stable, e.g., 1.70+).
-   Ripgrep (`rg`) installed and in your system's PATH (for `search_code` tool).
-   Docker (optional, for containerized deployment).

## Installation & Build

1.  **Clone the repository:**
    ```bash
    git clone <repository_url> # Replace with the actual URL
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

-   `FILES_ROOT`: (Required) Absolute path to the root directory the server can operate within. Supports `~` for home directory.
-   `ALLOWED_DIRECTORIES`: Comma-separated list of absolute paths or tilde-expanded paths. If empty, defaults to `FILES_ROOT`. For full filesystem access (DANGEROUS), set to `/` (Linux/macOS) or a drive letter like `C:\` (Windows).
-   `BLOCKED_COMMANDS`: Comma-separated list of command names (e.g., `rm,sudo`) to block. Default list includes many destructive commands.
-   `DEFAULT_SHELL`: Shell for `execute_command` (e.g., `bash`, `powershell`). System default if not set.
-   `LOG_LEVEL`: `trace`, `debug`, `info`, `warn`, `error`. Default: `info`.
-   `MCP_TRANSPORT`: `stdio` (default) or `sse`.
-   `MCP_SSE_HOST`: Host for SSE (e.g., `0.0.0.0`). Default: `127.0.0.1`.
-   `MCP_SSE_PORT`: Port for SSE (e.g., `3000`). Default: `3000`.
-   `FILE_READ_LINE_LIMIT`: Max lines for `read_file` (local text files). Default: `1000`.
-   `FILE_WRITE_LINE_LIMIT`: Max lines for `write_file` per call. Default: `50`.
-   `AUDIT_LOG_MAX_SIZE_MB`: Max size for audit log before rotation. Default: `10`.
-   `MCP_LOG_DIR`: Directory for audit and fuzzy search logs. Defaults to `$FILES_ROOT/.mcp-logs`. Supports `~`.

## Running the Server

### STDIO Mode (Default)

```bash
# Set environment variables (or use a .env file)
export FILES_ROOT=~/my_projects
export ALLOWED_DIRECTORIES="~/my_projects/project_a,~/my_projects/project_b"
# ... other env vars

./target/release/mcp-rg-editor
```
The server will listen for JSON-RPC messages on STDIN and send responses to STDOUT. Logs go to STDERR.

### SSE Mode

Ensure the `sse` feature was included during build.

```bash
export FILES_ROOT=~/my_projects
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

2.  **Run with `docker run` (STDIO Example):**
    ```bash
    docker run -i --rm \
      -v /path/on/host/projects:/app/files \
      -e FILES_ROOT=/app/files \
      -e ALLOWED_DIRECTORIES=/app/files \
      -e MCP_TRANSPORT=stdio \
      mcp-rg-editor
    ```

3.  **Run with `docker-compose`:**
    Modify `docker-compose.yml` for volumes and environment variables.
    ```bash
    docker-compose up -d # For SSE mode in detached
    # For STDIO: docker-compose run --rm mcp-server (ensure tty:false, stdin_open:true in yml)
    ```

## MCP Client Setup (e.g., Claude Desktop)

### STDIO Example (Claude Desktop)

Replace `/abs/path/to/mcp-rg-editor` and paths for `FILES_ROOT`, `ALLOWED_DIRECTORIES`.
```json
{
  "mcpServers": {
    "desktop_commander_rust": {
      "command": "/abs/path/to/mcp-rg-editor/target/release/mcp-rg-editor",
      "env": {
        "FILES_ROOT": "~/.my_code_root", // Example using tilde
        "ALLOWED_DIRECTORIES": "~/.my_code_root/project1,~/.my_code_root/project2",
        "BLOCKED_COMMANDS": "sudo,rm,mkfs", // Customize as needed
        "LOG_LEVEL": "info",
        "MCP_TRANSPORT": "stdio",
        "RUST_LOG": "info,mcp_rg_editor=debug", // More specific logging
        "RUST_BACKTRACE": "1"
      }
    }
  }
}
```

## Available Tools

| Category        | Tool                | Description                                                                 |
|-----------------|---------------------|-----------------------------------------------------------------------------|
| **Configuration**| `get_config`        | Get current server configuration (FILES_ROOT, allowed dirs, etc.).          |
|                 | `set_config_value`  | Set a server configuration value in-memory (e.g., `allowedDirectories`).      |
| **Filesystem**  | `read_file`         | Read local file (line offset/length for text) or URL. Handles images.       |
|                 | `read_multiple_files`| Read multiple local files. Handles images.                                 |
|                 | `write_file`        | Write/append to files, respects `fileWriteLineLimit`. Chunk large writes.   |
|                 | `create_directory`  | Create directories, including nested ones.                                  |
|                 | `list_directory`    | List directory contents with `[FILE]` / `[DIR]` prefixes.                   |
|                 | `move_file`         | Move/rename files or directories.                                           |
|                 | `search_files`      | Find files/dirs by name (case-insensitive substring) with timeout.          |
|                 | `get_file_info`     | Get file/dir metadata (size, timestamps, permissions).                      |
| **Code Search** | `search_code`       | Search code with Ripgrep (regex, globs, context, hidden, timeout).          |
| **Text Editing**| `edit_block`        | Replace text. `expected_replacements`. Fuzzy feedback if exact match fails. |
| **Terminal**    | `execute_command`   | Run terminal commands; timeout, background exec, shell choice.                |
|                 | `read_output`       | Get new output from a running command session by `session_id`.              |
|                 | `force_terminate`   | Stop a running command session by `session_id`.                             |
|                 | `list_sessions`     | List active command sessions (ID, command, PID, runtime).                   |
| **Process**     | `list_processes`    | List system processes (PID, name, CPU/mem, command, status).                |
|                 | `kill_process`      | Terminate a system process by PID.                                          |

### `edit_block` Usage

The `edit_block` tool is powerful but requires careful usage:
-   **Small, focused edits:** Prefer multiple small `edit_block` calls over one large one.
-   **Context is key:** For `old_string`, include enough surrounding context (1-3 lines typically) to make it unique if `expected_replacements` is 1.
-   **`expected_replacements`:**
    -   Default `1`: Replaces the first exact match. If more than one found, it errors.
    -   Set to `N`: Replaces exactly `N` occurrences. Errors if a different number is found.
    -   Set to `0`: Replaces *all* exact occurrences. Use with caution.
-   **Fuzzy Fallback:** If an exact match for `old_string` fails, the tool attempts a fuzzy search.
    -   If a close match (>=70% similarity by default) is found, it returns a diff: `common_prefix{-removed-}{+added+}common_suffix`. **It does not automatically apply the fuzzy match.** You must then call `edit_block` again with the *exact* text from the file (as shown in the `-removed-` part of the diff) as your new `old_string`.
    -   Details of fuzzy attempts are logged to `$FILES_ROOT/.mcp-logs/fuzzy-search.log`.

### Handling Long-Running Commands

1.  `execute_command` returns after `timeout_ms` (default 1s) with initial output.
2.  If the command didn't finish, it continues in the background. The result includes a `session_id` and `timed_out: true`.
3.  Use `read_output` with the `session_id` to get new output.
4.  Use `force_terminate` with `session_id` to stop it if needed.
5.  `list_sessions` shows all backgrounded commands.

## Security

-   **`FILES_ROOT`**: The primary jail. All local file/command operations are confined within this directory.
-   **`ALLOWED_DIRECTORIES`**: A list of comma-separated, absolute, or tilde-expanded paths. Filesystem tools can only operate within these directories (which must also be under `FILES_ROOT`, unless `FILES_ROOT` itself is very broad like `/`). An empty list defaults to only `FILES_ROOT`. Setting this to `/` (Unix) or `C:\` (Windows) grants full filesystem access *within the scope of what `FILES_ROOT` allows* and is dangerous.
-   **`BLOCKED_COMMANDS`**: Prevents execution of specified commands (e.g., `rm`, `sudo`). Matches the first word of a command.
-   Path canonicalization and validation are used to prevent traversal attacks (e.g., `../../`).
-   Tilde (`~`) expansion is supported for convenience in path parameters.
-   **Terminal commands can still access files outside `ALLOWED_DIRECTORIES` if `FILES_ROOT` is permissive.** The `ALLOWED_DIRECTORIES` setting primarily restricts the *filesystem tools* (`read_file`, `write_file`, etc.), not the general terminal. True terminal sandboxing is a more complex OS-level feature not implemented here.

## Troubleshooting

-   Set `LOG_LEVEL=debug` (or `trace`) and `RUST_LOG=mcp_rg_editor=debug` (or `trace`) for detailed server logs (sent to STDERR).
-   Ensure `rg` (ripgrep) is installed and in PATH for `search_code`.
-   Verify `FILES_ROOT` is an absolute path and exists. `ALLOWED_DIRECTORIES` paths must also be absolute or tilde-expanded and exist (or their parents must exist if they are targets for creation).
-   For Docker STDIO: ensure `-i` flag is used with `docker run`.
-   For Docker SSE: ensure ports are correctly mapped (`-p host:container`).
-   Check logs in `$FILES_ROOT/.mcp-logs/` (or `$MCP_LOG_DIR` if set) for audit and fuzzy search details.

## Development

-   Format code: `cargo fmt`
-   Lint code: `cargo clippy --features "stdio,sse"`
-   Run tests: `cargo test --features "stdio,sse"` (see `tests/README.md`)

## Contributing

Contributions are welcome! Please fork the repository, create a feature branch, and open a pull request.

## License

This project is licensed under the MIT License.