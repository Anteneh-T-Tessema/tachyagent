#!/bin/bash
# Tachy Benchmark Suite — measures tool calling accuracy and latency
# Usage: ./bench/run_bench.sh [model]
#
# Runs 15 tasks against the specified model and reports success rate + timing.

set -e

MODEL="${1:-gemma4:26b}"
TACHY="./target/release/tachy-cli"
RESULTS_DIR="bench/results"
mkdir -p "$RESULTS_DIR"

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
RESULT_FILE="$RESULTS_DIR/bench_${MODEL//:/_}_${TIMESTAMP}.txt"

echo "Tachy Benchmark Suite"
echo "Model: $MODEL"
echo "Date: $(date)"
echo "=========================="
echo ""

PASS=0
FAIL=0
TOTAL=0
TOTAL_TIME=0

run_test() {
    local name="$1"
    local prompt="$2"
    local expect="$3"
    local timeout_secs="${4:-45}"

    TOTAL=$((TOTAL + 1))
    printf "  %-40s " "$name"

    START=$(date +%s)
    OUTPUT=$(timeout "$timeout_secs" "$TACHY" --model "$MODEL" prompt "$prompt" 2>&1) || OUTPUT="TIMEOUT"
    END=$(date +%s)
    ELAPSED=$((END - START))
    TOTAL_TIME=$((TOTAL_TIME + ELAPSED))

    if echo "$OUTPUT" | grep -qi "$expect"; then
        PASS=$((PASS + 1))
        printf "✓ PASS  (%ds)\n" "$ELAPSED"
    else
        FAIL=$((FAIL + 1))
        printf "✗ FAIL  (%ds)\n" "$ELAPSED"
        echo "    Expected: $expect" >> "$RESULT_FILE"
        echo "    Got: $(echo "$OUTPUT" | head -3)" >> "$RESULT_FILE"
    fi
}

echo "=== Knowledge (no tools) ===" | tee -a "$RESULT_FILE"
run_test "Simple math" "what is 7 * 8? answer with just the number" "56"
run_test "Capital city" "what is the capital of France? one word" "Paris"
run_test "Code concept" "what does FIFO stand for in computing? one line" "first"

echo "" | tee -a "$RESULT_FILE"
echo "=== Tool: list_directory ===" | tee -a "$RESULT_FILE"
run_test "List current dir" "list the files in the current directory" "Cargo"
run_test "List subdirectory" "list the files in the crates directory" "audit\|backend\|runtime"

echo "" | tee -a "$RESULT_FILE"
echo "=== Tool: read_file ===" | tee -a "$RESULT_FILE"
run_test "Read Cargo.toml" "read the Cargo.toml file and tell me the edition" "2021"
run_test "Read with analysis" "read crates/audit/src/lib.rs and list the public modules" "event\|logger\|policy"

echo "" | tee -a "$RESULT_FILE"
echo "=== Tool: bash ===" | tee -a "$RESULT_FILE"
run_test "Simple command" "run the command: echo tachy-bench-ok" "tachy-bench-ok"
run_test "Command with pipe" "run: ls crates | wc -l" "11\|10\|12"

echo "" | tee -a "$RESULT_FILE"
echo "=== Tool: grep_search ===" | tee -a "$RESULT_FILE"
run_test "Grep for pattern" "search for 'fn main' in the crates directory" "main"
run_test "Grep with context" "search for 'unsafe' in all .rs files" "unsafe\|no match\|not found\|No match"

echo "" | tee -a "$RESULT_FILE"
echo "=== Tool: edit_file ===" | tee -a "$RESULT_FILE"
echo "test-content-for-bench" > /tmp/tachy-bench-edit.txt
run_test "Edit file" "edit /tmp/tachy-bench-edit.txt: replace 'test-content-for-bench' with 'edited-by-tachy'" "edited\|success\|replaced" 60

echo "" | tee -a "$RESULT_FILE"
echo "=== Multi-step ===" | tee -a "$RESULT_FILE"
run_test "Read then analyze" "read Cargo.toml and count how many crates are in this workspace" "11\|12\|10" 60
run_test "Multi-tool chain" "list the crates directory, then read one of the Cargo.toml files and tell me its dependencies" "serde\|runtime\|tokio" 90

echo ""
echo "=========================="
echo "Results: $PASS/$TOTAL passed ($FAIL failed)"
echo "Total time: ${TOTAL_TIME}s"
echo "Average: $((TOTAL_TIME / TOTAL))s per task"
ACCURACY=$(echo "scale=1; $PASS * 100 / $TOTAL" | bc)
echo "Accuracy: ${ACCURACY}%"
echo ""

# Save summary
{
    echo "Model: $MODEL"
    echo "Date: $(date)"
    echo "Passed: $PASS/$TOTAL"
    echo "Failed: $FAIL"
    echo "Accuracy: ${ACCURACY}%"
    echo "Total time: ${TOTAL_TIME}s"
    echo "Average: $((TOTAL_TIME / TOTAL))s per task"
} >> "$RESULT_FILE"

echo "Full results saved to $RESULT_FILE"
