#!/bin/bash
#
# Reckoner full smoke test
#
# Tests every CLI command against a real (temporary) environment.
# Requires: git, gh, docker, pas, claude (Claude subscription).
#
# Usage:
#   ./tests/smoke_test.sh                  # quick mode (skip Claude calls)
#   SMOKE_FULL=1 ./tests/smoke_test.sh     # full mode (includes Claude + task)
#
set -uo pipefail  # no -e: we handle errors per-test

# ── Helpers ───────────────────────────────────────────────────────────

PASS=0
FAIL=0
SKIP=0

pass() { PASS=$((PASS + 1)); echo "  [PASS] $1"; }
fail() { FAIL=$((FAIL + 1)); echo "  [FAIL] $1: $2"; }
skip() { SKIP=$((SKIP + 1)); echo "  [SKIP] $1"; }

# Strip ANSI escape codes from output before grepping
strip_ansi() { sed 's/\x1b\[[0-9;]*m//g'; }

# Find the reck binary
RECK="${RECK:-$(cd "$(dirname "$0")/.." && pwd)/target/release/reck}"
if [ ! -x "$RECK" ]; then
    echo "ERROR: reck binary not found at $RECK"
    echo "Run: cargo build --release"
    exit 1
fi

# Use a temporary home to avoid clobbering real ~/.reckoner
export SMOKE_HOME=$(mktemp -d)
export HOME="$SMOKE_HOME"
cleanup() { [ -d "$SMOKE_HOME" ] && find "$SMOKE_HOME" -delete 2>/dev/null || true; }
trap cleanup EXIT

echo "Reckoner Smoke Test"
echo "  binary: $RECK"
echo "  home:   $SMOKE_HOME"
echo "  mode:   ${SMOKE_FULL:+full}${SMOKE_FULL:-quick (set SMOKE_FULL=1 for Claude tests)}"
echo ""

# ── 1. Version & Help ─────────────────────────────────────────────────

echo "=== CLI Basics ==="

if "$RECK" --version 2>&1 | grep -q "reck"; then
    pass "--version"
else
    fail "--version" "no version output"
fi

if "$RECK" --help 2>&1 | grep -q "software factory"; then
    pass "--help"
else
    fail "--help" "missing description"
fi

# Check all subcommands exist in help
for cmd in add list remove sync task status logs lint schedule infra observe doctor config init; do
    if "$RECK" --help 2>&1 | grep -qi "$cmd"; then
        pass "help lists '$cmd'"
    else
        fail "help lists '$cmd'" "subcommand not found in help output"
    fi
done

# ── 2. Init ───────────────────────────────────────────────────────────

echo ""
echo "=== Init ==="

if "$RECK" init 2>&1 | strip_ansi | grep -q "Directories ready"; then
    pass "init creates directories"
else
    fail "init" "did not report directories ready"
fi

if [ -f "$SMOKE_HOME/.reckoner/config.toml" ]; then
    pass "init creates config.toml"
else
    fail "init creates config.toml" "file not found"
fi

if [ -f "$SMOKE_HOME/.reckoner/reckoner.db" ]; then
    pass "init creates database"
else
    fail "init creates database" "file not found"
fi

if [ -d "$SMOKE_HOME/.reckoner/repos" ]; then
    pass "init creates repos dir"
else
    fail "init creates repos dir" "directory not found"
fi

if [ -d "$SMOKE_HOME/.reckoner/logs" ]; then
    pass "init creates logs dir"
else
    fail "init creates logs dir" "directory not found"
fi

# Re-running init should not fail
if "$RECK" init 2>&1 | strip_ansi | grep -q "already exists\|Directories ready"; then
    pass "init is idempotent"
else
    fail "init is idempotent" "second run failed unexpectedly"
fi

# ── 3. Config ─────────────────────────────────────────────────────────

echo ""
echo "=== Config ==="

if "$RECK" config 2>&1 | grep -q "default_model"; then
    pass "config shows pas settings"
else
    fail "config" "missing pas settings"
fi

if "$RECK" config 2>&1 | grep -q "repos_dir"; then
    pass "config shows general settings"
else
    fail "config" "missing general settings"
fi

# ── 4. Doctor ─────────────────────────────────────────────────────────

echo ""
echo "=== Doctor ==="

DOCTOR_OUT=$("$RECK" doctor 2>&1)

if echo "$DOCTOR_OUT" | grep -q "\[ok\] git"; then
    pass "doctor finds git"
else
    fail "doctor finds git" "git not detected"
fi

if echo "$DOCTOR_OUT" | grep -q "\[ok\] docker\|FAIL.*docker\|WARN.*docker"; then
    pass "doctor checks docker"
else
    fail "doctor checks docker" "docker not mentioned"
fi

# ── 5. Repo Management ───────────────────────────────────────────────

echo ""
echo "=== Repo Management ==="

# Create a local bare repo to test with (no network needed)
TEST_UPSTREAM="$SMOKE_HOME/upstream.git"
git init --bare "$TEST_UPSTREAM" >/dev/null 2>&1

# Create a temporary clone, add a file, push to upstream
TEST_CLONE="$SMOKE_HOME/clone"
git clone "$TEST_UPSTREAM" "$TEST_CLONE" >/dev/null 2>&1
cd "$TEST_CLONE"
git config user.name "Test"
git config user.email "test@test.com"
echo "# Test Repo" > README.md
echo '[package]\nname = "test"' > Cargo.toml
git add -A
git commit -m "initial" >/dev/null 2>&1
git push origin main >/dev/null 2>&1
cd - >/dev/null

# reck add (local path as URL)
if "$RECK" add "$TEST_UPSTREAM" 2>&1 | grep -q "Added"; then
    pass "add registers repo"
else
    fail "add" "did not report 'Added'"
fi

# reck list
if "$RECK" list 2>&1 | grep -q "upstream"; then
    pass "list shows registered repo"
else
    fail "list" "repo not in list"
fi

# reck sync
if "$RECK" sync upstream 2>&1 | grep -q "Synced"; then
    pass "sync fetches repo"
else
    fail "sync" "did not report 'Synced'"
fi

# ── 6. Status (empty) ────────────────────────────────────────────────

echo ""
echo "=== Status ==="

if "$RECK" status 2>&1 | grep -q "No active tasks"; then
    pass "status shows no active tasks"
else
    fail "status" "unexpected output for empty status"
fi

# ── 7. Lint ───────────────────────────────────────────────────────────

echo ""
echo "=== Lint ==="

if "$RECK" lint upstream 2>&1 | strip_ansi | grep -qi "linter\|findings\|toolchain"; then
    pass "lint runs against repo"
else
    fail "lint" "unexpected output"
fi

# ── 8. Logs (no task yet) ────────────────────────────────────────────

echo ""
echo "=== Logs ==="

LOGS_OUT=$("$RECK" logs reck-nonexistent 2>&1 || true)
if echo "$LOGS_OUT" | grep -qi "no logs found\|error"; then
    pass "logs errors on missing task"
else
    fail "logs" "did not error on missing task"
fi

# ── 9. Schedule ───────────────────────────────────────────────────────

echo ""
echo "=== Schedule ==="

if "$RECK" schedule list 2>&1 | grep -q "No schedules"; then
    pass "schedule list shows empty"
else
    fail "schedule list" "unexpected output"
fi

# schedule add (will create plist)
if "$RECK" schedule add --name smoke-test --repo upstream --pipeline test.dot --cron "0 3 * * *" 2>&1 | strip_ansi | grep -qi "smoke-test\|com.reckoner\|Schedule"; then
    pass "schedule add creates schedule"
else
    fail "schedule add" "did not create schedule"
fi

# schedule list should now show it
if "$RECK" schedule list 2>&1 | grep -q "smoke-test"; then
    pass "schedule list shows new schedule"
else
    fail "schedule list" "schedule not in list"
fi

# schedule remove
if "$RECK" schedule remove smoke-test 2>&1 | grep -q "Removed"; then
    pass "schedule remove deletes schedule"
else
    fail "schedule remove" "did not report removed"
fi

# ── 10. Infra ─────────────────────────────────────────────────────────

echo ""
echo "=== Infra ==="

if "$RECK" infra status 2>&1 | grep -q "not configured\|configured\|running"; then
    pass "infra status reports state"
else
    fail "infra status" "unexpected output"
fi

# Verify compose file gets written
"$RECK" infra status >/dev/null 2>&1  # triggers ensure_compose_file
if [ -f "$SMOKE_HOME/.reckoner/infra/docker-compose.yml" ]; then
    pass "infra creates docker-compose.yml"
else
    # infra status doesn't always write compose, that's ok
    skip "infra creates docker-compose.yml (only written on infra up)"
fi

# ── 11. Task (full mode only) ────────────────────────────────────────

echo ""
echo "=== Task ==="

if [ -n "${SMOKE_FULL:-}" ]; then
    # This actually calls Claude — costs money, requires subscription
    TASK_OUT=$("$RECK" task upstream "add a comment to README.md saying hello from smoke test" --no-pr 2>&1)
    if echo "$TASK_OUT" | grep -q "Task reck-.*completed"; then
        pass "task runs end-to-end"

        # Extract task ID
        TASK_ID=$(echo "$TASK_OUT" | grep "Task reck-" | grep -o "reck-[a-f0-9]*")

        # Check status
        if "$RECK" status "$TASK_ID" 2>&1 | grep -q "done"; then
            pass "status shows completed task"
        else
            fail "status shows completed task" "task not marked done"
        fi

        # Check logs exist
        if "$RECK" logs "$TASK_ID" 2>&1 | grep -q "files"; then
            pass "logs shows task summary"
        else
            fail "logs shows task summary" "no summary output"
        fi

        # Check log filtering
        if "$RECK" logs "$TASK_ID" --app 2>&1 | grep -q "result\|error\|empty"; then
            pass "logs --app shows app output"
        else
            fail "logs --app" "unexpected output"
        fi
    else
        fail "task" "did not complete: $TASK_OUT"
    fi
else
    skip "task end-to-end (set SMOKE_FULL=1)"
    skip "status shows completed task"
    skip "logs shows task summary"
    skip "logs --app shows app output"
fi

# ── 12. Remove repo ──────────────────────────────────────────────────

echo ""
echo "=== Cleanup ==="

if "$RECK" remove upstream 2>&1 | grep -q "Removed"; then
    pass "remove unregisters repo"
else
    fail "remove" "did not report removed"
fi

if "$RECK" list 2>&1 | grep -q "No repos"; then
    pass "list empty after remove"
else
    fail "list after remove" "repo still in list"
fi

# ── Summary ───────────────────────────────────────────────────────────

echo ""
echo "========================================"
echo "  PASS: $PASS"
echo "  FAIL: $FAIL"
echo "  SKIP: $SKIP"
echo "  TOTAL: $((PASS + FAIL + SKIP))"
echo "========================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
