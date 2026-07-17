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
FIXTURE_SOURCE_DIR="$SCRIPT_DIR/tests/fixture"
ORACLE_TIMEOUT_SECONDS="${EMUERA_REFERENCE_TIMEOUT_SECONDS:-30}"

for command_name in dotnet wine winepath jq perl; do
    if ! command -v "$command_name" >/dev/null 2>&1; then
        echo "required command not found: $command_name" >&2
        exit 127
    fi
done

mkdir -p "$WINE_PREFIX" "$WORK_DIR" "$(dirname "$OUTPUT_FILE")"
FIXTURE_DIR="$(mktemp -d "$WORK_DIR/fixture.XXXXXX")"
trap 'rm -rf "$FIXTURE_DIR"' EXIT
cp -R "$FIXTURE_SOURCE_DIR/." "$FIXTURE_DIR"

export WINEPREFIX="$WINE_PREFIX"
export WINEDEBUG=-all
export MVK_CONFIG_LOG_LEVEL=0

if [[ ! -f "$WINE_PREFIX/system.reg" ]]; then
    wineboot -u
fi

FIXTURE_WINDOWS_PATH="$(winepath -w "$FIXTURE_DIR")"

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
jq -nc --arg gameDir "$FIXTURE_WINDOWS_PATH" \
    '{id:"wine-load",op:"load",gameDir:$gameDir}' >>"$REQUEST_FILE"
printf '%s\n' \
    '{"id":"wine-toneinput","op":"execute","statement":"TONEINPUTS 1000, \"DEFAULT\", 1, \"timeout\", 0, 0"}' \
    '{"id":"wine-getmillisecond","op":"eval","source":"GETMILLISECOND()"}' \
    '{"id":"wine-getsecond","op":"eval","source":"GETSECOND()"}' \
    '{"id":"wine-project","op":"analyzeProject"}' \
    '{"id":"wine-csv-varsize","op":"eval","source":"VARSIZE(\"ABL\")"}' \
    '{"id":"wine-csv-name","op":"eval","source":"GETNUM(ABL, \"later\")"}' \
    '{"id":"wine-csv-price","op":"eval","source":"ITEMPRICE:5"}' \
    '{"id":"wine-csv-str","op":"eval","source":"STR:0"}' \
    '{"id":"wine-csv-character","op":"eval","source":"CSVABL(10, 2)"}' \
    '{"id":"wine-csv-gamebase","op":"eval","source":"GAMEBASE_GAMECODE"}' \
    '{"id":"wine-analyze","op":"analyzeLine","source":"RESULT = 9"}' \
    '{"id":"wine-execute","op":"execute","statement":"RESULT = 9","watch":["RESULT"]}' \
    '{"id":"wine-putform","op":"execute","statement":"PUTFORM suffix","watch":["SAVEDATA_TEXT"]}' \
    '{"id":"wine-savenos","op":"eval","source":"SAVENOS()"}' \
    '{"id":"wine-run","op":"run","entry":"ORACLE_TEST","watch":["RESULT"]}' \
    '{"id":"wine-native-tail","op":"run","entry":"ORACLE_NATIVE","watch":["RESULT:0","RESULT:1","RESULT:2","RESULT:3","RESULT:4","RESULT:5","RESULT:6","RESULT:7","RESULT:8","RESULTS:0","RESULTS:1","RESULTS:2","RESULTS:3","RESULTS:4","RESULTS:5","RESULTS:6"]}' \
    '{"id":"wine-map","op":"run","entry":"ORACLE_MAP","watch":["RESULT","RESULTS"]}' \
    '{"id":"wine-presentation","op":"run","entry":"ORACLE_PRESENTATION"}' \
    '{"id":"wine-structured","op":"run","entry":"ORACLE_STRUCTURED","watch":["RESULT:0","RESULT:1","RESULT:2","RESULT:3","RESULT:4","RESULT:5","RESULTS:0","RESULTS:1","RESULTS:2"]}' \
    '{"id":"wine-input","op":"run","entry":"ORACLE_INPUT","inputs":["42"],"watch":["RESULT"]}' \
    '{"id":"wine-reset","op":"reset"}' \
    >>"$REQUEST_FILE"

# Wine emits CRLF on stdout. Normalize it so the saved file is ordinary NDJSON
# on macOS while keeping Wine diagnostics separate from protocol output. Keep a
# watchdog around the process so a native UI regression fails instead of
# blocking differential tests indefinitely.
perl -e 'alarm shift; exec @ARGV' "$ORACLE_TIMEOUT_SECONDS" \
    wine "$EXECUTABLE" <"$REQUEST_FILE" 2>"$STDERR_FILE" \
    | tr -d '\r' >"$OUTPUT_FILE"

jq -e -s '
    length == 25 and
    map(.id) == [
        "wine-capabilities", "wine-lex", "wine-expression", "wine-load", "wine-toneinput",
        "wine-getmillisecond", "wine-getsecond", "wine-project",
        "wine-csv-varsize", "wine-csv-name", "wine-csv-price", "wine-csv-str",
        "wine-csv-character", "wine-csv-gamebase", "wine-analyze", "wine-execute",
        "wine-putform", "wine-savenos",
        "wine-run", "wine-native-tail", "wine-map", "wine-presentation", "wine-structured", "wine-input", "wine-reset"
    ] and
    all(.[]; .ok == true) and
    (map(select(.id == "wine-load"))[0].result.termination == "waitingInput") and
    (map(select(.id == "wine-project"))[0].result.functions | map(.name) | sort == ["ORACLE_INPUT", "ORACLE_MAP", "ORACLE_NATIVE", "ORACLE_PRESENTATION", "ORACLE_STRUCTURED", "ORACLE_TEST", "SYSTEM_TITLE"]) and
    (map(select(.id == "wine-project"))[0].result.functions | map(select(.name == "SYSTEM_TITLE"))[0].lines | map(.functionCode) | contains(["IF", "CALL", "CALL", "ENDIF", "INPUT", "RETURN"])) and
    (map(select(.id == "wine-csv-varsize"))[0].result.value == 120) and
    (map(select(.id == "wine-csv-name"))[0].result.value == 2) and
    (map(select(.id == "wine-csv-price"))[0].result.value == 120) and
    (map(select(.id == "wine-csv-str"))[0].result.value == "initial text") and
    (map(select(.id == "wine-csv-character"))[0].result.value == 5) and
    (map(select(.id == "wine-csv-gamebase"))[0].result.value == 42) and
    (map(select(.id == "wine-analyze"))[0].result.argument != null) and
    (map(select(.id == "wine-execute"))[0].result.watches.RESULT == 9) and
    (map(select(.id == "wine-putform"))[0].result.watches.SAVEDATA_TEXT == "suffix") and
    (map(select(.id == "wine-savenos"))[0].result.value == 20) and
    (map(select(.id == "wine-run"))[0].result.termination == "completed") and
    (map(select(.id == "wine-run"))[0].result.output | join("\n") | contains("ORACLE_OK")) and
    (map(select(.id == "wine-native-tail"))[0].result.watches == {"RESULT:0":0,"RESULT:1":4,"RESULT:2":1,"RESULT:3":1,"RESULT:4":2,"RESULT:5":2,"RESULT:6":1,"RESULT:7":946,"RESULT:8":946,"RESULTS:0":"a\\+b","RESULTS:1":"β","RESULTS:2":"ABC","RESULTS:3":"abc","RESULTS:4":"b/c","RESULTS:5":"a,b,c,","RESULTS:6":"β"}) and
    (map(select(.id == "wine-map"))[0].result.termination == "completed") and
    (map(select(.id == "wine-map"))[0].result.output | join("\n") | contains("MAP=2,1,1,1|3|b,a")) and
    (map(select(.id == "wine-presentation"))[0].result.termination == "completed") and
    (map(select(.id == "wine-presentation"))[0].result.output | join("\n") | contains("VISIBLE")) and
    (map(select(.id == "wine-structured"))[0].result.termination == "completed") and
    (map(select(.id == "wine-structured"))[0].result.watches."RESULTS:0" | contains("<xs:schema id=\"NewDataSet\"")) and
    (map(select(.id == "wine-structured"))[0].result.watches."RESULTS:1" | contains("A&amp;B")) and
    (map(select(.id == "wine-structured"))[0].result.watches."RESULTS:2" == "<root><item id=\"a\" kind=\"first\">one</item><item id=\"b\">changed</item></root>") and
    (map(select(.id == "wine-structured"))[0].result.watches."RESULT:4" == 1) and
    (map(select(.id == "wine-structured"))[0].result.watches."RESULT:5" == 1) and
    (map(select(.id == "wine-input"))[0].result.termination == "completed") and
    (map(select(.id == "wine-input"))[0].result.watches.RESULT == 42) and
    (map(select(.id == "wine-toneinput"))[0].result.termination == "waitingInput") and
    (map(select(.id == "wine-toneinput"))[0].result.inputRequest.InputType == "StrValue") and
    (map(select(.id == "wine-toneinput"))[0].result.inputRequest.OneInput == true) and
    (map(select(.id == "wine-toneinput"))[0].result.inputRequest.Timelimit == "1000") and
    (((map(select(.id == "wine-getmillisecond"))[0].result.value / 1000 | floor) as $milliseconds |
        map(select(.id == "wine-getsecond"))[0].result.value as $seconds |
        (($milliseconds - $seconds) >= -1 and ($milliseconds - $seconds) <= 1))) and
    (map(select(.id == "wine-reset"))[0].result.reset == true)
' "$OUTPUT_FILE" >/dev/null

echo "Wine reference CLI and CSV oracle smoke test passed."
echo "NDJSON: $OUTPUT_FILE"
echo "Wine stderr: $STDERR_FILE"
