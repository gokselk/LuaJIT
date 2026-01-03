#!/bin/bash
cd /home/user/LuaJIT/luajit-rs

pass=0
fail=0
skip=0

for f in test-suite/test/misc/*.lua; do
    name=$(basename "$f")

    # Skip tests that need unimplemented features
    if grep -q "require.*ctest\|require.*jit\|require.*bit\|coroutine\." "$f" 2>/dev/null; then
        skip=$((skip + 1))
        continue
    fi

    output=$(timeout 5 ./target/release/luajit-rs "$f" 2>&1)
    code=$?

    if [ $code -eq 0 ]; then
        echo "PASS: $name"
        pass=$((pass + 1))
    elif [ $code -eq 124 ]; then
        echo "TIMEOUT: $name"
        fail=$((fail + 1))
    else
        err=$(echo "$output" | head -1)
        echo "FAIL: $name - $err"
        fail=$((fail + 1))
    fi
done

echo ""
echo "Results: $pass passed, $fail failed, $skip skipped"
