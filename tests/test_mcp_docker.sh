#!/bin/bash
# Test the MCP server running in Docker with STDIO transport

TEMP_FILE=$(mktemp)
IMAGE_NAME="mcp-rg-editor:latest" # Use the image built by docker-compose or docker build

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"
PROJECT_ROOT="$(realpath "${SCRIPT_DIR}/..")"
TEST_DATA_DIR_HOST="${PROJECT_ROOT}/test_data_docker" # Use a separate dir for Docker tests
TEST_DATA_DIR_CONTAINER="/app/files" # This must match FILES_ROOT in Docker

cleanup() {
  rm -f "$TEMP_FILE"
  rm -rf "$TEST_DATA_DIR_HOST"
}
trap cleanup EXIT

# Ensure test_data directory exists on host
mkdir -p "$TEST_DATA_DIR_HOST"
echo "Hello from Docker test_read.txt" > "$TEST_DATA_DIR_HOST/test_read.txt"

echo "Testing MCP server in Docker with STDIO transport ($IMAGE_NAME)..."
echo "Host test data dir: $TEST_DATA_DIR_HOST"
echo "Container test data dir: $TEST_DATA_DIR_CONTAINER"
echo "Sending initialize, tools/list, and a search request..."

# Prepare environment variables for Docker
# These are passed via -e to docker run
DOCKER_ENV_ARGS=(
  "-e" "FILES_ROOT=${TEST_DATA_DIR_CONTAINER}"
  "-e" "ALLOWED_DIRECTORIES=${TEST_DATA_DIR_CONTAINER}" # Keep it simple for Docker test
  "-e" "LOG_LEVEL=debug"
  "-e" "RUST_LOG=debug"
  "-e" "MCP_TRANSPORT=stdio"
)

REQUEST_BATCH=$(cat << JSON_BATCH
{
  "jsonrpc": "2.0",
  "id": "init-docker-1",
  "method": "initialize",
  "params": {}
}
{
  "jsonrpc": "2.0",
  "id": "list-tools-docker-1",
  "method": "tools/list",
  "params": {}
}
{
  "jsonrpc": "2.0",
  "id": "get-config-docker-1",
  "method": "tools/call",
  "params": {
    "name": "get_config",
    "arguments": {}
  }
}
{
  "jsonrpc": "2.0",
  "id": "read-file-docker-1",
  "method": "tools/call",
  "params": {
    "name": "read_file",
    "arguments": { "path": "test_read.txt" }
  }
}
{
  "jsonrpc": "2.0",
  "id": "search-code-docker-1",
  "method": "tools/call",
  "params": {
    "name": "search_code",
    "arguments": { "pattern": "Hello from Docker", "path": "." }
  }
}
JSON_BATCH
)

# Check if Docker image exists
if ! docker image inspect "$IMAGE_NAME" &> /dev/null; then
    echo "❌ ERROR: Docker image $IMAGE_NAME not found. Build it first, e.g., with 'docker build -t $IMAGE_NAME .'"
    exit 1
fi

# Send compacted JSON requests
# The -i flag is critical for STDIO transport with Docker.
echo "$REQUEST_BATCH" | jq -c '.' | docker run \
  -i --rm \
  -v "${TEST_DATA_DIR_HOST}:${TEST_DATA_DIR_CONTAINER}" \
  "${DOCKER_ENV_ARGS[@]}" \
  "$IMAGE_NAME" > "$TEMP_FILE"


echo ""
echo "--- Server Response Log (Docker - first 30 lines) ---"
head -n 30 "$TEMP_FILE"
echo "--- End of Server Response Log (Docker) ---"
echo ""

SUCCESS_COUNT=0
ERROR_COUNT=0

# Check initialize
if grep -q '"id":"init-docker-1"' "$TEMP_FILE" && grep -q '"name":"mcp-rg-editor"' "$TEMP_FILE"; then
  echo "✅ Docker: Initialize successful."
  SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
else
  echo "❌ Docker: Initialize failed."
  ERROR_COUNT=$((ERROR_COUNT + 1))
fi

# Check list_tools
if grep -q '"id":"list-tools-docker-1"' "$TEMP_FILE" && grep -q '"name":"search_code"' "$TEMP_FILE"; then
  echo "✅ Docker: List tools successful."
  SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
else
  echo "❌ Docker: List tools failed."
  ERROR_COUNT=$((ERROR_COUNT + 1))
fi

# Check get_config
# Note: Path inside container is /app/files
if grep -q '"id":"get-config-docker-1"' "$TEMP_FILE" && grep -q "\"files_root\":\"${TEST_DATA_DIR_CONTAINER//\//\\/}\"" "$TEMP_FILE"; then
  echo "✅ Docker: Get config successful."
  SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
else
  echo "❌ Docker: Get config failed or incorrect FILES_ROOT."
  ERROR_COUNT=$((ERROR_COUNT + 1))
fi

# Check read_file
if grep -q '"id":"read-file-docker-1"' "$TEMP_FILE" && grep -q "Hello from Docker test_read.txt" "$TEMP_FILE"; then
  echo "✅ Docker: Read file successful."
  SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
else
  echo "❌ Docker: Read file failed."
  ERROR_COUNT=$((ERROR_COUNT + 1))
fi

# Check search_code
if grep -q '"id":"search-code-docker-1"' "$TEMP_FILE" && grep -q "test_read.txt:1:Hello from Docker" "$TEMP_FILE"; then
  echo "✅ Docker: Search code successful."
  SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
else
  echo "❌ Docker: Search code failed."
  ERROR_COUNT=$((ERROR_COUNT + 1))
fi


echo ""
if [ "$ERROR_COUNT" -eq 0 ] && [ "$SUCCESS_COUNT" -ge 4 ]; then # Expect at least 4 successful high-level checks for this simpler docker test
  echo "✅✅✅ OVERALL SUCCESS: Docker STDIO transport test passed!"
  echo "The MCP server correctly processed JSON-RPC requests via STDIN"
  echo "and responded via STDOUT while running in a Docker container."
  exit 0
else
  echo "❌❌❌ OVERALL ERROR: Docker STDIO transport test failed! ($SUCCESS_COUNT successes, $ERROR_COUNT errors)"
  echo "The server did not correctly process all inputs or return expected outputs."
  exit 1
fi