#!/usr/bin/env bash
set -euo pipefail

# Build and run ALL meldr integration tests in Docker.
# Runs both integration.rs and docker_integration.rs.
#
# Usage:
#   ./run-docker-tests.sh              # run all integration tests
#   ./run-docker-tests.sh <filter>     # run tests matching <filter>
#   ./run-docker-tests.sh --build-only # just build the image

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
IMAGE_NAME="meldr-integ-tests"

cd "$SCRIPT_DIR"

echo "==> Building test image..."
docker build -f Dockerfile.test -t "$IMAGE_NAME" .

if [[ "${1:-}" == "--build-only" ]]; then
    echo "==> Image built successfully."
    exit 0
fi

TEST_FILTER="${1:-}"

echo "==> Running all integration tests..."
if [[ -n "$TEST_FILTER" ]]; then
    docker run --rm \
        -e MELDR_TEST_REPOS=/test-repos \
        "$IMAGE_NAME" \
        cargo test --features docker-tests --test docker_integration --test integration -- --test-threads=8 "$TEST_FILTER"
else
    docker run --rm \
        -e MELDR_TEST_REPOS=/test-repos \
        "$IMAGE_NAME" \
        cargo test --features docker-tests --test docker_integration --test integration -- --test-threads=8
fi

echo "==> All tests passed."
