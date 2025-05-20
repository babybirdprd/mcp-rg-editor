#!/bin/bash
# Test the MCP server locally with a proper JSON-RPC sequence

# Create a temporary file for the response
TEMP_FILE=$(mktemp)
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"
PROJECT_ROOT="$(realpath "${SCRIPT_DIR}/..")" # Assuming tests dir is one level down
TEST_DATA_DIR="${PROJECT_ROOT}/test_data"
TARGET_DIR="${PROJECT_ROOT}/target/release"
MCP_BINARY="${TARGET_DIR}/mcp-rg-editor"

# Cleanup function
cleanup() {
  rm -f "$TEMP_FILE"
  rm -rf "$TEST_DATA_DIR/new_dir"
  rm -f "$TEST_DATA_DIR/test_write.txt"
}
trap cleanup EXIT

echo "Project Root: $PROJECT_ROOT"
echo "Test Data Dir: $TEST_DATA_DIR"
echo "MCP Binary: $MCP_BINARY"

# Ensure test_data directory exists
mkdir -p "$TEST_DATA_DIR"
echo "Hello from test_read.txt" > "$TEST_DATA_DIR/test_read.txt"
echo "Initial content for edit." > "$TEST_DATA_DIR/test_edit.txt"


echo "Testing local MCP server (mcp-rg-editor)..."
echo "Sending initialize, tools/list, and various tool calls..."

# Prepare environment variables
export FILES_ROOT="$TEST_DATA_DIR"
export ALLOWED_DIRECTORIES="$TEST_DATA_DIR"
export LOG_LEVEL="debug" # For more verbose test output
export RUST_LOG="debug"
export MCP_TRANSPORT="stdio"
export BLOCKED_COMMANDS="forbidden_command"
export FILE_READ_LINE_LIMIT=10
export FILE_WRITE_LINE_LIMIT=5

# Test with proper initialization sequence and multiple tool calls
# Note: Each JSON object must be on a single line for stdio transport.
# Use jq to compact JSON for requests.
REQUEST_BATCH=$(cat << JSON_BATCH
{
  "jsonrpc": "2.0",
  "id": "init-1",
  "method": "initialize",
  "params": {}
}
{
  "jsonrpc": "2.0",
  "id": "list-tools-1",
  "method": "tools/list",
  "params": {}
}
{
  "jsonrpc": "2.0",
  "id": "get-config-1",
  "method": "tools/call",
  "params": {
    "name": "get_config",
    "arguments": {}
  }
}
{
  "jsonrpc": "2.0",
  "id": "read-file-1",
  "method": "tools/call",
  "params": {
    "name": "read_file",
    "arguments": { "path": "test_read.txt" }
  }
}
{
  "jsonrpc": "2.0",
  "id": "search-code-1",
  "method": "tools/call",
  "params": {
    "name": "search_code",
    "arguments": { "pattern": "Hello", "path": "." }
  }
}
{
  "jsonrpc": "2.0",
  "id": "exec-echo-1",
  "method": "tools/call",
  "params": {
    "name": "execute_command",
    "arguments": { "command": "echo TestEcho" }
  }
}
{
  "jsonrpc": "2.0",
  "id": "list-proc-1",
  "method": "tools/call",
  "params": {
    "name": "list_processes",
    "arguments": {}
  }
}
{
  "jsonrpc": "2.0",
  "id": "edit-block-1",
  "method": "tools/call",
  "params": {
    "name": "edit_block",
    "arguments": { "file_path": "test_edit.txt", "old_string": "Initial", "new_string": "Edited", "expected_replacements": 1 }
  }
}
{
  "jsonrpc": "2.0",
  "id": "create-dir-1",
  "method": "tools/call",
  "params": {
    "name": "create_directory",
    "arguments": { "path": "new_dir" }
  }
}
{
  "jsonrpc": "2.0",
  "id": "list-dir-1",
  "method": "tools/call",
  "params": {
    "name": "list_directory",
    "arguments": { "path": "." }
  }
}
JSON_BATCH
)

# Ensure binary exists
if [ ! -f "$MCP_BINARY" ]; then
    echo "❌ ERROR: MCP binary not found at $MCP_BINARY. Build the project first with 'cargo build --release --features \"stdio,sse\"'."
    exit 1
fi


# Send compacted JSON requests
echo "$REQUEST_BATCH" | jq -c '.' | "$MCP_BINARY" > "$TEMP_FILE"


# Display the response
echo ""
echo "--- Server Response Log (first 50 lines) ---"
head -n 50 "$TEMP_FILE"
echo "--- End of Server Response Log ---"
echo ""

# Basic checks for success
# Check for specific successful responses. This is a bit fragile but good for a start.
SUCCESS_COUNT=0
ERROR_COUNT=0

# Check initialize
if grep -q '"id":"init-1"' "$TEMP_FILE" && grep -q '"name":"mcp-rg-editor"' "$TEMP_FILE"; then
  echo "✅ Initialize successful."
  SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
else
  echo "❌ Initialize failed or missing."
  ERROR_COUNT=$((ERROR_COUNT + 1))
fi

# Check list_tools
if grep -q '"id":"list-tools-1"' "$TEMP_FILE" && grep -q '"name":"search_code"' "$TEMP_FILE" && grep -q '"name":"read_file"' "$TEMP_FILE"; then
  echo "✅ List tools successful."
  SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
else
  echo "❌ List tools failed or incomplete."
  ERROR_COUNT=$((ERROR_COUNT + 1))
fi

# Check get_config
if grep -q '"id":"get-config-1"' "$TEMP_FILE" && grep -q "\"files_root\":\"${TEST_DATA_DIR//\//\\/}\"" "$TEMP_FILE"; then # Handle windows paths in json
  echo "✅ Get config successful."
  SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
else
  echo "❌ Get config failed or incorrect FILES_ROOT."
  ERROR_COUNT=$((ERROR_COUNT + 1))
fi

# Check read_file
if grep -q '"id":"read-file-1"' "$TEMP_FILE" && grep -q "Hello from test_read.txt" "$TEMP_FILE"; then
  echo "✅ Read file successful."
  SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
else
  echo "❌ Read file failed or content mismatch."
  ERROR_COUNT=$((ERROR_COUNT + 1))
fi

# Check search_code (ripgrep)
if grep -q '"id":"search-code-1"' "$TEMP_FILE" && grep -q "test_read.txt:1:Hello" "$TEMP_FILE"; then
  echo "✅ Search code successful."
  SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
else
  echo "❌ Search code failed or no matches."
  ERROR_COUNT=$((ERROR_COUNT + 1))
fi

# Check execute_command
if grep -q '"id":"exec-echo-1"' "$TEMP_FILE" && grep -q "TestEcho" "$TEMP_FILE"; then
  echo "✅ Execute command successful."
  SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
else
  echo "❌ Execute command failed."
  ERROR_COUNT=$((ERROR_COUNT + 1))
fi

# Check list_processes
if grep -q '"id":"list-proc-1"' "$TEMP_FILE" && grep -q '"pid":' "$TEMP_FILE"; then # Check if some process info is returned
  echo "✅ List processes successful."
  SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
else
  echo "❌ List processes failed."
  ERROR_COUNT=$((ERROR_COUNT + 1))
fi

# Check edit_block
if grep -q '"id":"edit-block-1"' "$TEMP_FILE" && grep -q '"replacements_made":1' "$TEMP_FILE"; then
  if grep -q "Edited content for edit." "$TEST_DATA_DIR/test_edit.txt"; then
    echo "✅ Edit block successful and file updated."
    SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
  else
    echo "❌ Edit block reported success, but file content is wrong."
    ERROR_COUNT=$((ERROR_COUNT + 1))
  fi
else
  echo "❌ Edit block failed."
  ERROR_COUNT=$((ERROR_COUNT + 1))
fi

# Check create_directory
if grep -q '"id":"create-dir-1"' "$TEMP_FILE" && grep -q '"success":true' "$TEMP_FILE"; then
  if [ -d "$TEST_DATA_DIR/new_dir" ]; then
    echo "✅ Create directory successful."
    SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
  else
    echo "❌ Create directory reported success, but directory not found."
    ERROR_COUNT=$((ERROR_COUNT + 1))
  fi
else
  echo "❌ Create directory failed."
  ERROR_COUNT=$((ERROR_COUNT + 1))
fi

# Check list_directory
if grep -q '"id":"list-dir-1"' "$TEMP_FILE" && grep -q "new_dir" "$TEMP_FILE" && grep -q "test_read.txt" "$TEMP_FILE"; then
  echo "✅ List directory successful."
  SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
else
  echo "❌ List directory failed or output incorrect."
  ERROR_COUNT=$((ERROR_COUNT + 1))
fi


echo ""
if [ "$ERROR_COUNT" -eq 0 ] && [ "$SUCCESS_COUNT" -ge 8 ]; then # Expect at least 8 successful high-level checks
  echo "✅✅✅ OVERALL SUCCESS: Local MCP server test passed!"
  echo "The MCP server correctly processed JSON-RPC requests via STDIN and responded via STDOUT."
  exit 0
else
  echo "❌❌❌ OVERALL ERROR: Local MCP server test failed! ($SUCCESS_COUNT successes, $ERROR_COUNT errors)"
  echo "The server did not correctly process all inputs or return expected outputs."
  exit 1
fi