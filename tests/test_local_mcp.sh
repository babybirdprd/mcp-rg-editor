#!/bin/bash
# Test the MCP server locally with a proper JSON-RPC sequence

# Create a temporary file for the response
TEMP_FILE=$(mktemp)
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"
PROJECT_ROOT="$(realpath "${SCRIPT_DIR}/..")" # Assuming tests dir is one level down
TEST_DATA_DIR="${PROJECT_ROOT}/test_data"
TARGET_DIR="${PROJECT_ROOT}/target/release"
MCP_BINARY="${TARGET_DIR}/mcp-rg-editor"
LOG_DIR="${TEST_DATA_DIR}/.mcp-logs" # For checking logs

# Cleanup function
cleanup() {
  echo "Cleaning up..."
  rm -f "$TEMP_FILE"
  rm -rf "$TEST_DATA_DIR/new_dir"
  rm -f "$TEST_DATA_DIR/test_write.txt"
  rm -f "$TEST_DATA_DIR/test_edit.txt"
  rm -f "$TEST_DATA_DIR/test_append.txt"
  rm -rf "$LOG_DIR" # Clean up logs
  # Restore original test_read.txt if it was modified by a bad edit_block test
  echo "Hello from test_read.txt" > "$TEST_DATA_DIR/test_read.txt"
}
trap cleanup EXIT

echo "Project Root: $PROJECT_ROOT"
echo "Test Data Dir: $TEST_DATA_DIR"
echo "MCP Binary: $MCP_BINARY"
echo "Log Dir: $LOG_DIR"

# Ensure test_data directory exists and is clean
rm -rf "$TEST_DATA_DIR"
mkdir -p "$TEST_DATA_DIR"
mkdir -p "$LOG_DIR" # Create log dir for testing log features

echo "Hello from test_read.txt" > "$TEST_DATA_DIR/test_read.txt"
echo "Initial content for edit." > "$TEST_DATA_DIR/test_edit.txt"
echo "Line1 for append" > "$TEST_DATA_DIR/test_append.txt"


echo "Testing local MCP server (mcp-rg-editor)..."
echo "Sending initialize, tools/list, and various tool calls..."

# Prepare environment variables
export FILES_ROOT="$TEST_DATA_DIR"
export ALLOWED_DIRECTORIES="$TEST_DATA_DIR" # Restrict to test_data for safety
export LOG_LEVEL="debug" 
export RUST_LOG="mcp_rg_editor=debug,info" # Specific logging for our crate
export MCP_TRANSPORT="stdio"
export BLOCKED_COMMANDS="forbidden_command,rm" # Add rm to blocked for a test
export FILE_READ_LINE_LIMIT=10
export FILE_WRITE_LINE_LIMIT=5 # Low limit for testing chunking advice
export DEFAULT_SHELL="sh" # Use sh for predictable basic commands
export MCP_LOG_DIR="$LOG_DIR" # Direct logs to our test log dir

# Test with proper initialization sequence and multiple tool calls
REQUEST_BATCH=$(cat << JSON_BATCH
{
  "jsonrpc": "2.0", "id": "init-1", "method": "initialize", "params": {}
}
{
  "jsonrpc": "2.0", "id": "list-tools-1", "method": "tools/list", "params": {}
}
{
  "jsonrpc": "2.0", "id": "get-config-1", "method": "tools/call", "params": { "name": "get_config", "arguments": {} }
}
{
  "jsonrpc": "2.0", "id": "read-file-1", "method": "tools/call", "params": { "name": "read_file", "arguments": { "path": "test_read.txt" } }
}
{
  "jsonrpc": "2.0", "id": "search-code-1", "method": "tools/call", "params": { "name": "search_code", "arguments": { "pattern": "Hello", "path": "." } }
}
{
  "jsonrpc": "2.0", "id": "exec-echo-1", "method": "tools/call", "params": { "name": "execute_command", "arguments": { "command": "echo TestEcho" } }
}
{
  "jsonrpc": "2.0", "id": "list-proc-1", "method": "tools/call", "params": { "name": "list_processes", "arguments": {} }
}
{
  "jsonrpc": "2.0", "id": "edit-block-exact-1", "method": "tools/call", "params": { "name": "edit_block", "arguments": { "file_path": "test_edit.txt", "old_string": "Initial content for edit.", "new_string": "Edited exact content.", "expected_replacements": 1 } }
}
{
  "jsonrpc": "2.0", "id": "edit-block-fuzzy-fail-1", "method": "tools/call", "params": { "name": "edit_block", "arguments": { "file_path": "test_edit.txt", "old_string": "Edited exact content that is slightly different", "new_string": "This should not apply", "expected_replacements": 1 } }
}
{
  "jsonrpc": "2.0", "id": "create-dir-1", "method": "tools/call", "params": { "name": "create_directory", "arguments": { "path": "new_dir" } }
}
{
  "jsonrpc": "2.0", "id": "list-dir-1", "method": "tools/call", "params": { "name": "list_directory", "arguments": { "path": "." } }
}
{
  "jsonrpc": "2.0", "id": "write-file-rewrite-1", "method": "tools/call", "params": { "name": "write_file", "arguments": { "path": "test_write.txt", "content": "Line1\nLine2\nLine3", "mode": "rewrite" } }
}
{
  "jsonrpc": "2.0", "id": "write-file-append-1", "method": "tools/call", "params": { "name": "write_file", "arguments": { "path": "test_append.txt", "content": "\nLine2 for append\nLine3 for append", "mode": "append" } }
}
{
  "jsonrpc": "2.0", "id": "write-file-too-many-lines-1", "method": "tools/call", "params": { "name": "write_file", "arguments": { "path": "test_write_long.txt", "content": "1\n2\n3\n4\n5\n6" } }
}
{
  "jsonrpc": "2.0", "id": "search-files-1", "method": "tools/call", "params": { "name": "search_files", "arguments": { "path": ".", "pattern": "test_read" } }
}
{
  "jsonrpc": "2.0", "id": "get-file-info-1", "method": "tools/call", "params": { "name": "get_file_info", "arguments": { "path": "test_read.txt" } }
}
{
  "jsonrpc": "2.0", "id": "exec-blocked-1", "method": "tools/call", "params": { "name": "execute_command", "arguments": { "command": "rm -rf /" } }
}
{
  "jsonrpc": "2.0", "id": "read-url-1", "method": "tools/call", "params": { "name": "read_file", "arguments": { "path": "https://raw.githubusercontent.com/modelcontextprotocol/mcp-specs/main/README.md", "is_url": true } }
}
JSON_BATCH
)

# Ensure binary exists
if [ ! -f "$MCP_BINARY" ]; then
    echo "❌ ERROR: MCP binary not found at $MCP_BINARY. Build the project first with 'cargo build --release --features \"stdio,sse\"'."
    exit 1
fi

# Send compacted JSON requests
echo "$REQUEST_BATCH" | jq -c '.' | "$MCP_BINARY" > "$TEMP_FILE" 2>&1 # Capture stderr too for debugging

# Display the response
echo ""
echo "--- Server Response Log (first 100 lines from $TEMP_FILE) ---"
head -n 100 "$TEMP_FILE"
echo "--- End of Server Response Log ---"
echo ""

SUCCESS_COUNT=0
ERROR_COUNT=0
TOTAL_CHECKS=0

check_result() {
  local id="$1"
  local condition_grep="$2"
  local description="$3"
  local file_to_check_content="$4" # Optional: file path to check content
  local expected_file_content="$5" # Optional: expected content in the file

  TOTAL_CHECKS=$((TOTAL_CHECKS + 1))
  if grep -q "\"id\":\"$id\"" "$TEMP_FILE" && grep -Eq "$condition_grep" "$TEMP_FILE"; then
    if [ -n "$file_to_check_content" ]; then
      if grep -qF "$expected_file_content" "$file_to_check_content"; then
        echo "✅ $description"
        SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
      else
        echo "❌ $description (File content mismatch in $file_to_check_content)"
        echo "   Expected to find: $expected_file_content"
        echo "   Actual content:"
        cat "$file_to_check_content"
        ERROR_COUNT=$((ERROR_COUNT + 1))
      fi
    else
      echo "✅ $description"
      SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
    fi
  else
    echo "❌ $description (Response missing or condition not met for ID $id with grep '$condition_grep')"
    ERROR_COUNT=$((ERROR_COUNT + 1))
  fi
}


# Check initialize
check_result "init-1" '"name":"mcp-rg-editor-desktop-commander-enhanced"' "Initialize successful."

# Check list_tools
check_result "list-tools-1" '"name":"search_code".*"name":"read_file"' "List tools successful."

# Check get_config (escape path for grep)
ESCAPED_TEST_DATA_DIR=$(echo "$TEST_DATA_DIR" | sed 's/\//\\\//g')
check_result "get-config-1" "\"files_root\":\"$ESCAPED_TEST_DATA_DIR\"" "Get config successful."

# Check read_file
check_result "read-file-1" "Hello from test_read.txt" "Read file successful."

# Check search_code (ripgrep)
check_result "search-code-1" "test_read.txt:1:Hello from test_read.txt" "Search code successful."

# Check execute_command
check_result "exec-echo-1" "TestEcho" "Execute command successful."

# Check list_processes
check_result "list-proc-1" '"pid":' "List processes successful."

# Check edit_block exact
check_result "edit-block-exact-1" '"replacements_made":1' "Edit block (exact) successful." "$TEST_DATA_DIR/test_edit.txt" "Edited exact content."

# Check edit_block fuzzy fail feedback
check_result "edit-block-fuzzy-fail-1" "Found a similar text with.*similarity" "Edit block (fuzzy feedback) correct."

# Check create_directory
check_result "create-dir-1" '"success":true' "Create directory successful."
if [ ! -d "$TEST_DATA_DIR/new_dir" ]; then
  echo "❌ Create directory check FAILED: Directory $TEST_DATA_DIR/new_dir not found on disk."
  ERROR_COUNT=$((ERROR_COUNT + 1)) # Increment error if dir not found, even if MCP said success
fi

# Check list_directory
check_result "list-dir-1" '\[DIR\] new_dir.*\[FILE\] test_read.txt' "List directory successful."

# Check write_file rewrite
check_result "write-file-rewrite-1" "Successfully wrote 3 lines" "Write file (rewrite) successful." "$TEST_DATA_DIR/test_write.txt" "Line1\nLine2\nLine3"

# Check write_file append
check_result "write-file-append-1" "Successfully appended 3 lines" "Write file (append) successful." "$TEST_DATA_DIR/test_append.txt" "Line1 for append\nLine2 for append\nLine3 for append"

# Check write_file too many lines (should error due to FILE_WRITE_LINE_LIMIT=5)
check_result "write-file-too-many-lines-1" "Content exceeds line limit of 5. Received 6 lines" "Write file (too many lines) correctly errored."

# Check search_files
check_result "search-files-1" '"matches":\["test_read.txt"\]' "Search files successful."

# Check get_file_info
check_result "get-file-info-1" '"is_file":true.*"size":' "Get file info successful."

# Check execute_command blocked
check_result "exec-blocked-1" "CommandBlocked.*rm -rf /" "Execute command (blocked) correctly handled."

# Check read_url
check_result "read-url-1" "Model Context Protocol" "Read URL successful."


# Check Audit Log
if [ -f "$LOG_DIR/tool_calls.log" ]; then
  if grep -q "get_config" "$LOG_DIR/tool_calls.log" && grep -q "read_file" "$LOG_DIR/tool_calls.log"; then
    echo "✅ Audit log created and contains entries."
    SUCCESS_COUNT=$((SUCCESS_COUNT + 1)); TOTAL_CHECKS=$((TOTAL_CHECKS + 1))
  else
    echo "❌ Audit log exists but missing expected entries."
    ERROR_COUNT=$((ERROR_COUNT + 1)); TOTAL_CHECKS=$((TOTAL_CHECKS + 1))
  fi
else
  echo "❌ Audit log file not found at $LOG_DIR/tool_calls.log."
  ERROR_COUNT=$((ERROR_COUNT + 1)); TOTAL_CHECKS=$((TOTAL_CHECKS + 1))
fi

# Check Fuzzy Log (at least one entry should be there from fuzzy-fail test)
if [ -f "$LOG_DIR/fuzzy-search.log" ]; then
  if [ $(wc -l <"$LOG_DIR/fuzzy-search.log") -ge 2 ]; then # Header + 1 entry
    echo "✅ Fuzzy search log created and contains entries."
    SUCCESS_COUNT=$((SUCCESS_COUNT + 1)); TOTAL_CHECKS=$((TOTAL_CHECKS + 1))
  else
    echo "❌ Fuzzy search log exists but missing expected entries."
    ERROR_COUNT=$((ERROR_COUNT + 1)); TOTAL_CHECKS=$((TOTAL_CHECKS + 1))
  fi
else
  echo "❌ Fuzzy search log file not found at $LOG_DIR/fuzzy-search.log."
  ERROR_COUNT=$((ERROR_COUNT + 1)); TOTAL_CHECKS=$((TOTAL_CHECKS + 1))
fi


echo ""
echo "--- Test Summary ---"
echo "Total checks performed: $TOTAL_CHECKS"
echo "Successful checks: $SUCCESS_COUNT"
echo "Failed checks: $ERROR_COUNT"
echo "--------------------"

if [ "$ERROR_COUNT" -eq 0 ] && [ "$SUCCESS_COUNT" -eq "$TOTAL_CHECKS" ]; then
  echo "✅✅✅ OVERALL SUCCESS: Local MCP server test passed!"
  echo "The MCP server correctly processed JSON-RPC requests via STDIN and responded via STDOUT."
  exit 0
else
  echo "❌❌❌ OVERALL ERROR: Local MCP server test failed! ($SUCCESS_COUNT successes, $ERROR_COUNT errors out of $TOTAL_CHECKS checks)"
  echo "The server did not correctly process all inputs or return expected outputs."
  exit 1
fi