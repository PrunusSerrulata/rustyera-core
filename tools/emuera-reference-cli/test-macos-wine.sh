#!/usr/bin/env bash
set -euo pipefail

# Run the Windows-only reference oracle on macOS through one persistent Wine
# process. The fixed prefix keeps Wine initialization and the installed state
# stable between differential-test runs.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
PROJECT="$SCRIPT_DIR/Emuera.ReferenceCli.csproj"
WINE_PREFIX="$REPO_ROOT/.wine-prefix/emuera-reference-cli"
WORK_DIR="$REPO_ROOT/.wine-tmp/emuera-reference-cli"
PUBLISH_DIR="$SCRIPT_DIR/bin/x64/Debug-NAudio/net10.0-windows/win-x64/publish"
EXECUTABLE="$PUBLISH_DIR/Emuera.ReferenceCli.exe"
OUTPUT_FILE="${1:-$WORK_DIR/wine-smoke.ndjson}"
REQUEST_FILE="$WORK_DIR/requests.ndjson"
STDERR_FILE="$WORK_DIR/wine-stderr.log"

for command_name in dotnet wine jq; do
    if ! command -v "$command_name" >/dev/null 2>&1; then
        echo "required command not found: $command_name" >&2
        exit 127
    fi
done

mkdir -p "$WINE_PREFIX" "$WORK_DIR" "$(dirname "$OUTPUT_FILE")"

export WINEPREFIX="$WINE_PREFIX"
export WINEDEBUG=-all
export MVK_CONFIG_LOG_LEVEL=0

if [[ ! -f "$WINE_PREFIX/system.reg" ]]; then
    wineboot -u
fi

# A framework-dependent build cannot locate macOS's .NET installation from
# inside Wine, so publish the Windows runtime beside the executable.
dotnet publish "$PROJECT" \
    -c Debug-NAudio \
    -p:Platform=x64 \
    -r win-x64 \
    --self-contained true \
    -p:PublishSingleFile=false \
    --nologo \
    -clp:ErrorsOnly

printf '%s\n' \
    '{"id":"wine-capabilities","op":"capabilities"}' \
    '{"id":"wine-lex","op":"lex","source":"1 + 2"}' \
    '{"id":"wine-expression","op":"parseExpression","source":"1 + 2 * 3"}' \
    >"$REQUEST_FILE"

# Wine emits CRLF on stdout. Normalize it so the saved file is ordinary NDJSON
# on macOS while keeping Wine diagnostics separate from protocol output.
wine "$EXECUTABLE" <"$REQUEST_FILE" 2>"$STDERR_FILE" \
    | tr -d '\r' >"$OUTPUT_FILE"

jq -e -s '
    length == 3 and
    map(.id) == ["wine-capabilities", "wine-lex", "wine-expression"] and
    all(.[]; .ok == true)
' "$OUTPUT_FILE" >/dev/null

echo "Wine reference CLI smoke test passed."
echo "NDJSON: $OUTPUT_FILE"
echo "Wine stderr: $STDERR_FILE"
