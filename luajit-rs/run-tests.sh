#!/bin/bash
# LuaJIT-RS Test Runner
# Usage: ./run-tests.sh [pattern]

set -e

LUAJIT="./target/release/luajit-rs"
TEST_DIR="test-suite/test"
PATTERN="${1:-}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

passed=0
failed=0
skipped=0

# Build first
echo "Building luajit-rs..."
cargo build --release 2>/dev/null

echo ""
echo "LuaJIT-RS Test Suite"
echo "===================="
echo ""

# Find and run tests
run_test() {
    local file="$1"
    local name="${file#$TEST_DIR/}"

    # Skip FFI tests (need FFI implementation)
    if [[ "$file" == *"/ffi/"* ]]; then
        ((skipped++))
        return
    fi

    # Skip tests that require ctest library
    if grep -q "require.*ctest" "$file" 2>/dev/null; then
        ((skipped++))
        return
    fi

    # Skip tests that require jit library features we don't have
    if grep -q "require.*jit" "$file" 2>/dev/null; then
        ((skipped++))
        return
    fi

    # Skip tests that require bit library (if not implemented)
    if grep -q "require.*bit" "$file" 2>/dev/null; then
        ((skipped++))
        return
    fi

    # Skip coroutine tests (if not implemented)
    if grep -q "coroutine\." "$file" 2>/dev/null; then
        ((skipped++))
        return
    fi

    printf "  %-50s " "$name"

    if timeout 10 $LUAJIT "$file" >/dev/null 2>&1; then
        echo -e "${GREEN}PASS${NC}"
        ((passed++))
    else
        echo -e "${RED}FAIL${NC}"
        ((failed++))
        # Show error on verbose
        if [[ -n "${VERBOSE:-}" ]]; then
            $LUAJIT "$file" 2>&1 | head -5 | sed 's/^/    /'
        fi
    fi
}

# Run misc tests (most likely to work)
echo "Running misc tests..."
for file in "$TEST_DIR"/misc/*.lua; do
    if [[ -n "$PATTERN" && ! "$file" =~ $PATTERN ]]; then
        continue
    fi
    run_test "$file"
done

echo ""
echo "===================="
echo "Results:"
echo -e "  ${GREEN}$passed passed${NC}"
if [[ $failed -gt 0 ]]; then
    echo -e "  ${RED}$failed failed${NC}"
fi
if [[ $skipped -gt 0 ]]; then
    echo -e "  ${YELLOW}$skipped skipped${NC}"
fi
echo ""

if [[ $failed -gt 0 ]]; then
    exit 1
fi
