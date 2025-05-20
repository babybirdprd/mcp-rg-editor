#!/bin/bash
# Test the MCP server running in Docker with STDIO transport

TEMP_FILE=$(mktemp)
IMAGE_NAME="mcp-rg-editor:latest" # Use the image built by docker-compose or docker build

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"
PROJECT_ROOT="$(realpath "${SCRIPT_DIR}/..")"
TEST_DATA_DIR_HOST="${PROJECT_ROOT}/test_data_docker" # Use a separate dir for Docker tests
TEST_DATA_DIR_CONTAINER="/app/files" # This must match FILES_ROOT in Docker
LOG_DIR_CONTAINER="/app/files/.mcp-logs-docker" # Log dir inside container for this test

cleanup() {
  echo "Cleaning up Docker test data..."
  rm -f "$TEMP_FILE"
  rm -rf "$TEST_DATA_DIR_HOST"
}
trap cleanup EXIT

# Ensure test_data directory exists on host and is clean
rm -rf "$TEST_DATA_DIR_HOST"
mkdir -p "$TEST_DATA_DIR_HOST"
# No need to create log dir on host, it will be inside container volume

echo "Hello from Docker test_read.txt" > "$TEST_DATA_DIR_HOST/test_read.txt"
echo "Initial Docker edit content." > "$TEST_DATA_DIR_HOST/test_edit_docker.txt"


echo "Testing MCP server in Docker with STDIO transport ($IMAGE_NAME)..."
echo "Host test data dir: $TEST_DATA_DIR_HOST"
echo "Container test data dir: $TEST_DATA_DIR_CONTAINER"
echo "Container log dir: $LOG_DIR_CONTAINER"
echo "Sending initialize, tools/list, and various tool calls..."

# Prepare environment variables for Docker
DOCKER_ENV_ARGS=(
  "-e" "FILES_ROOT=${TEST_DATA_DIR_CONTAINER}"
  "-e" "ALLOWED_DIRECTORIES=${TEST_DATA_DIR_CONTAINER}" 
  "-e" "LOG_LEVEL=debug"
  "-e" "RUST_LOG=mcp_rg_editor=debug,info"
  "-e" "MCP_TRANSPORT=stdio"
  "-e" "MCP_LOG_DIR=${LOG_DIR_CONTAINER}" # Set log dir inside container
  "-e" "DEFAULT_SHELL=sh"
)

REQUEST_BATCH=$(cat << JSON_BATCH
{
  "jsonrpc": "2.0", "id": "init-docker-1", "method": "initialize", "params": {}
}
{
  "jsonrpc": "2.0", "id": "list-tools-docker-1", "method": "tools/list", "params": {}
}
{
  "jsonrpc": "2.0", "id": "get-config-docker-1", "method": "tools/call", "params": { "name": "get_config", "arguments": {} }
}
{
  "jsonrpc": "2.0", "id": "read-file-docker-1", "method": "tools/call", "params": { "name": "read_file", "arguments": { "path": "test_read.txt" } }
}
{
  "jsonrpc": "2.0", "id": "search-code-docker-1", "method": "tools/call", "params": { "name": "search_code", "arguments": { "pattern": "Hello from Docker", "path": "." } }
}
{
  "jsonrpc": "2.0", "id": "edit-block-docker-1", "method": "tools/call", "params": { "name": "edit_block", "arguments": { "file_path": "test_edit_docker.txt", "old_string": "Initial Docker edit content.", "new_string": "Edited Docker content.", "expected_replacements": 1 } }
}
{
  "jsonrpc": "2.0", "id": "list-dir-docker-1", "method": "tools/call", "params": { "name": "list_directory", "arguments": { "path": "${LOG_DIR_CONTAINER}" } }
}
JSON_BATCH
)

# Check if Docker image exists
if ! docker image inspect "$IMAGE_NAME" &> /dev/null; then
    echo "❌ ERROR: Docker image $IMAGE_NAME not found. Build it first, e.g., with 'docker build -t $IMAGE_NAME .'"
    exit 1
fi

# Send compacted JSON requests
echo "$REQUEST_BATCH" | jq -c '.' | docker run \
  -i --rm \
  -v "${TEST_DATA_DIR_HOST}:${TEST_DATA_DIR_CONTAINER}" \
  "${DOCKER_ENV_ARGS[@]}" \
  "$IMAGE_NAME" > "$TEMP_FILE" 2>&1


echo ""
echo "--- Server Response Log (Docker - first 50 lines from $TEMP_FILE) ---"
head -n 50 "$TEMP_FILE"
echo "--- End of Server Response Log (Docker) ---"
echo ""

SUCCESS_COUNT=0
ERROR_COUNT=0
TOTAL_CHECKS=0

check_result_docker() {
  local id="$1"
  local condition_grep="$2"
  local description="$3"
  local file_to_check_content_host_path="$4" # Optional: HOST path to file
  local expected_file_content="$5"         # Optional: expected content

  TOTAL_CHECKS=$((TOTAL_CHECKS + 1))
  if grep -q "\"id\":\"$id\"" "$TEMP_FILE" && grep -Eq "$condition_grep" "$TEMP_FILE"; then
    if [ -n "$file_to_check_content_host_path" ]; then
      if grep -qF "$expected_file_content" "$file_to_check_content_host_path"; then
        echo "✅ Docker: $description"
        SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
      else
        echo "❌ Docker: $description (File content mismatch in $file_to_check_content_host_path)"
        echo "   Expected to find: $expected_file_content"
        echo "   Actual content:"
        cat "$file_to_check_content_host_path"
        ERROR_COUNT=$((ERROR_COUNT + 1))
      fi
    else
      echo "✅ Docker: $description"
      SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
    fi
  else
    echo "❌ Docker: $description (Response missing or condition not met for ID $id with grep '$condition_grep')"
    ERROR_COUNT=$((ERROR_COUNT + 1))
  fi
}


# Check initialize
check_result_docker "init-docker-1" '"name":"mcp-rg-editor-desktop-commander-enhanced"' "Initialize successful."

# Check list_tools
check_result_docker "list-tools-docker-1" '"name":"search_code"' "List tools successful."

# Check get_config (Path inside container is /app/files)
ESCAPED_CONTAINER_DIR=$(echo "$TEST_DATA_DIR_CONTAINER" | sed 's/\//\\\//g')
check_result_docker "get-config-docker-1" "\"files_root\":\"$ESCAPED_CONTAINER_DIR\"" "Get config successful."

# Check read_file
check_result_docker "read-file-docker-1" "Hello from Docker test_read.txt" "Read file successful."

# Check search_code
check_result_docker "search-code-docker-1" "test_read.txt:1:Hello from Docker test_read.txt" "Search code successful."

# Check edit_block
check_result_docker "edit-block-docker-1" '"replacements_made":1' "Edit block successful." "$TEST_DATA_DIR_HOST/test_edit_docker.txt" "Edited Docker content."

# Check if logs were created inside the container (by listing the log directory via MCP)
# This implicitly tests that the MCP_LOG_DIR env var worked.
check_result_docker "list-dir-docker-1" '\[FILE\] tool_calls.log' "Log directory listing shows audit log."


echo ""
echo "--- Docker Test Summary ---"
echo "Total checks performed: $TOTAL_CHECKS"
echo "Successful checks: $SUCCESS_COUNT"
echo "Failed checks: $ERROR_COUNT"
echo "-------------------------"

if [ "$ERROR_COUNT" -eq 0 ] && [ "$SUCCESS_COUNT" -eq "$TOTAL_CHECKS" ]; then
  echo "✅✅✅ OVERALL SUCCESS: Docker STDIO transport test passed!"
  echo "The MCP server correctly processed JSON-RPC requests via STDIN"
  echo "and responded via STDOUT while running in a Docker container."
  exit 0
else
  echo "❌❌❌ OVERALL ERROR: Docker STDIO transport test failed! ($SUCCESS_COUNT successes, $ERROR_COUNT errors out of $TOTAL_CHECKS checks)"
  echo "The server did not correctly process all inputs or return expected outputs."
  exit 1
fi