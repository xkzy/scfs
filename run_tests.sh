#!/usr/bin/env bash
# Wrapper to run cargo test under a timeout so hangs cause an automatic failure.

set -euo pipefail

: ${TEST_TIMEOUT:=300} # default to 5 minutes

if command -v timeout >/dev/null; then
  echo "Running cargo test under timeout ${TEST_TIMEOUT}s"
  exec timeout --preserve-status "${TEST_TIMEOUT}s" cargo test "$@"
else
  echo "warning: 'timeout' not found; running 'cargo test' without timeout"
  exec cargo test "$@"
fi
