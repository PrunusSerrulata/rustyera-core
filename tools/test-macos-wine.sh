#!/usr/bin/env bash
set -euo pipefail

# Run the Windows-only reference oracle on macOS through one persistent Wine
# process. The fixed prefix keeps Wine initialization and the installed state
# stable between differential-test runs.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CLI_DIR="$REPO_ROOT/reference/emuera.em/emuera-reference-cli"
PROJECT="$CLI_DIR/Emuera.ReferenceCli.csproj"
WINE_PREFIX="$REPO_ROOT/.wine-prefix/emuera-reference-cli"
WORK_DIR="$REPO_ROOT/.wine-tmp/emuera-reference-cli"
PUBLISH_DIR="$CLI_DIR/bin/x64/Debug-NAudio/net10.0-windows/win-x64/publish"
EXECUTABLE="$PUBLISH_DIR/Emuera.ReferenceCli.exe"
OUTPUT_FILE="${1:-$WORK_DIR/wine-smoke.ndjson}"
REQUEST_FILE="$WORK_DIR/requests.ndjson"
STDERR_FILE="$WORK_DIR/wine-stderr.log"
FIXTURE_SOURCE_DIR="$CLI_DIR/tests/fixture"
SYSTEM_FIXTURE_SOURCE_DIR="$CLI_DIR/tests/fixture-system"
ONEINPUT_FIXTURE_SOURCE_DIR="$CLI_DIR/tests/fixture-oneinput"
ONEINPUT_LONG_FIXTURE_SOURCE_DIR="$CLI_DIR/tests/fixture-oneinput-long"
ORACLE_TIMEOUT_SECONDS="${EMUERA_REFERENCE_TIMEOUT_SECONDS:-30}"

for command_name in dotnet wine winepath jq perl; do
    if ! command -v "$command_name" >/dev/null 2>&1; then
        echo "required command not found: $command_name" >&2
        exit 127
    fi
done

mkdir -p "$WINE_PREFIX" "$WORK_DIR" "$(dirname "$OUTPUT_FILE")"
FIXTURE_DIR="$(mktemp -d "$WORK_DIR/fixture.XXXXXX")"
SYSTEM_FIXTURE_DIR="$(mktemp -d "$WORK_DIR/system-fixture.XXXXXX")"
ONEINPUT_FIXTURE_DIR="$(mktemp -d "$WORK_DIR/oneinput-fixture.XXXXXX")"
ONEINPUT_LONG_FIXTURE_DIR="$(mktemp -d "$WORK_DIR/oneinput-long-fixture.XXXXXX")"
trap 'rm -rf "$FIXTURE_DIR" "$SYSTEM_FIXTURE_DIR" "$ONEINPUT_FIXTURE_DIR" "$ONEINPUT_LONG_FIXTURE_DIR"' EXIT
cp -R "$FIXTURE_SOURCE_DIR/." "$FIXTURE_DIR"
cp -R "$FIXTURE_SOURCE_DIR/." "$SYSTEM_FIXTURE_DIR"
cp -R "$SYSTEM_FIXTURE_SOURCE_DIR/." "$SYSTEM_FIXTURE_DIR"
cp -R "$FIXTURE_SOURCE_DIR/." "$ONEINPUT_FIXTURE_DIR"
cp -R "$ONEINPUT_FIXTURE_SOURCE_DIR/." "$ONEINPUT_FIXTURE_DIR"
cp -R "$FIXTURE_SOURCE_DIR/." "$ONEINPUT_LONG_FIXTURE_DIR"
cp -R "$ONEINPUT_FIXTURE_SOURCE_DIR/." "$ONEINPUT_LONG_FIXTURE_DIR"
cp -R "$ONEINPUT_LONG_FIXTURE_SOURCE_DIR/." "$ONEINPUT_LONG_FIXTURE_DIR"

export WINEPREFIX="$WINE_PREFIX"
export WINEDEBUG=-all
export MVK_CONFIG_LOG_LEVEL=0

if [[ ! -f "$WINE_PREFIX/system.reg" ]]; then
    wineboot -u
fi

FIXTURE_WINDOWS_PATH="$(winepath -w "$FIXTURE_DIR")"
SYSTEM_FIXTURE_WINDOWS_PATH="$(winepath -w "$SYSTEM_FIXTURE_DIR")"
ONEINPUT_FIXTURE_WINDOWS_PATH="$(winepath -w "$ONEINPUT_FIXTURE_DIR")"
ONEINPUT_LONG_FIXTURE_WINDOWS_PATH="$(winepath -w "$ONEINPUT_LONG_FIXTURE_DIR")"

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
    '{"id":"wine-tooltip-delay","op":"execute","statement":"TOOLTIP_SETDELAY 0"}' \
    '{"id":"wine-config-drawing","op":"eval","source":"GETCONFIGS(\"描画インターフェース\")"}' \
    '{"id":"wine-config-font-size","op":"eval","source":"GETCONFIG(\"フォントサイズ\")"}' \
    '{"id":"wine-config-fore-color","op":"eval","source":"GETCONFIG(\"文字色\")"}' \
    '{"id":"wine-config-stain-list","op":"eval","source":"GETCONFIGS(\"汚れの初期値\")"}' \
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
    '{"id":"wine-compat","op":"run","entry":"ORACLE_COMPAT","watch":["FLAG:0"]}' \
    '{"id":"wine-compat-rest","op":"run","entry":"ORACLE_COMPAT_REST","watch":["RESULT:1","RESULT:2","RESULT:3","RESULT:4","RESULT:5","RESULTS:10","FLAG:1","FLAG:2"]}' \
    '{"id":"wine-native-tail","op":"run","entry":"ORACLE_NATIVE","watch":["RESULT:0","RESULT:1","RESULT:2","RESULT:3","RESULT:4","RESULT:5","RESULT:6","RESULT:7","RESULT:8","RESULT:9","RESULT:10","RESULT:11","RESULT:12","RESULT:13","RESULTS:0","RESULTS:1","RESULTS:2","RESULTS:3","RESULTS:4","RESULTS:5","RESULTS:6","RESULTS:7"]}' \
    '{"id":"wine-dynamic-variables","op":"run","entry":"ORACLE_DYNAMIC_VARIABLES","watch":["RESULT:1","RESULT:2","RESULT:50","RESULT:51","RESULT:52","RESULT:53","RESULT:54","RESULTS:50","RESULTS:51","RESULTS:52","FLAG:4","SAVESTR:0"]}' \
    '{"id":"wine-reflection","op":"run","entry":"ORACLE_REFLECTION","watch":["RESULT:12","RESULT:13","RESULTS:8","RESULTS:9"]}' \
    '{"id":"wine-map","op":"run","entry":"ORACLE_MAP","watch":["RESULT","RESULTS"]}' \
    '{"id":"wine-presentation","op":"run","entry":"ORACLE_PRESENTATION"}' \
    '{"id":"wine-print-family","op":"run","entry":"ORACLE_PRINT_FAMILY"}' \
    '{"id":"wine-linecount","op":"run","entry":"ORACLE_LINECOUNT","watch":["RESULT:50","RESULT:51","RESULT:52"]}' \
    '{"id":"wine-html-pop","op":"run","entry":"ORACLE_HTML_POP","watch":["RESULTS:30"]}' \
    '{"id":"wine-presentation-23","op":"run","entry":"ORACLE_PRESENTATION_23","watch":["RESULTS:31","RESULTS:32","RESULTS:33","RESULTS:34"]}' \
    '{"id":"wine-structured","op":"run","entry":"ORACLE_STRUCTURED","watch":["RESULT:0","RESULT:1","RESULT:2","RESULT:3","RESULT:4","RESULT:5","RESULTS:0","RESULTS:1","RESULTS:2"]}' \
    '{"id":"wine-compat-12","op":"run","entry":"ORACLE_COMPAT_12","watch":["RESULT:20","RESULT:21","RESULT:22","RESULT:23","RESULT:24","RESULTS:20","RESULTS:21","RESULTS:22"]}' \
    '{"id":"wine-presentation-3","op":"run","entry":"ORACLE_PRESENTATION_3","watch":["RESULT:40","RESULT:41","RESULT:42","RESULT:43","RESULT:44","RESULT:45","RESULT:46","RESULT:47","RESULT:48","RESULT:49","RESULT:50","RESULTS:40"]}' \
    '{"id":"wine-input","op":"run","entry":"ORACLE_INPUT","inputs":["42"],"watch":["RESULT"]}' \
    '{"id":"wine-restart","op":"run","entry":"ORACLE_RESTART_FLOW","uiInputs":[{"text":"C","changedByMouse":true},{"text":"0","changedByMouse":true},{"text":"6","changedByMouse":true},{"text":"0","changedByMouse":true}]}' \
    '{"id":"wine-pending-auto-button","op":"run","entry":"ORACLE_PENDING_AUTO_BUTTON","uiInputs":[{"text":"58","changedByMouse":true}]}' \
    >>"$REQUEST_FILE"
jq -nc --arg gameDir "$ONEINPUT_FIXTURE_WINDOWS_PATH" \
    '{id:"wine-oneinput-load",op:"load",gameDir:$gameDir}' >>"$REQUEST_FILE"
printf '%s\n' \
    '{"id":"wine-oneinput-text","op":"run","entry":"ORACLE_ONEINPUT","uiInputs":[{"text":"12"},{"text":"βx"},{"text":"34"},{"text":"yz"}],"watch":["RESULT:40","RESULT:41","RESULTS:40","RESULTS:41"]}' \
    '{"id":"wine-oneinput-mouse-default","op":"run","entry":"ORACLE_ONEINPUT_MOUSE","uiInputs":[{"text":"42","changedByMouse":true},{"text":"LONG","changedByMouse":true}],"watch":["RESULT:42","RESULTS:42"]}' \
    >>"$REQUEST_FILE"
jq -nc --arg gameDir "$ONEINPUT_LONG_FIXTURE_WINDOWS_PATH" \
    '{id:"wine-oneinput-long-load",op:"load",gameDir:$gameDir}' >>"$REQUEST_FILE"
printf '%s\n' \
    '{"id":"wine-oneinput-mouse-long","op":"run","entry":"ORACLE_ONEINPUT_MOUSE","uiInputs":[{"text":"42","changedByMouse":true},{"text":"LONG","changedByMouse":true}],"watch":["RESULT:42","RESULTS:42"]}' \
    >>"$REQUEST_FILE"
jq -nc --arg gameDir "$SYSTEM_FIXTURE_WINDOWS_PATH" \
    '{id:"wine-system-load",op:"load",gameDir:$gameDir}' >>"$REQUEST_FILE"
printf '%s\n' \
    '{"id":"wine-stopcalltrain","op":"run","watch":["RESULT:30","RESULT:31"]}' \
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
    length == 49 and
    map(.id) == [
        "wine-capabilities", "wine-lex", "wine-expression", "wine-load", "wine-toneinput",
        "wine-tooltip-delay",
        "wine-config-drawing", "wine-config-font-size", "wine-config-fore-color", "wine-config-stain-list",
        "wine-getmillisecond", "wine-getsecond", "wine-project",
        "wine-csv-varsize", "wine-csv-name", "wine-csv-price", "wine-csv-str",
        "wine-csv-character", "wine-csv-gamebase", "wine-analyze", "wine-execute",
        "wine-putform", "wine-savenos",
        "wine-run", "wine-compat", "wine-compat-rest", "wine-native-tail", "wine-dynamic-variables", "wine-reflection", "wine-map", "wine-presentation", "wine-print-family", "wine-linecount", "wine-html-pop", "wine-presentation-23", "wine-structured", "wine-compat-12", "wine-presentation-3", "wine-input", "wine-restart", "wine-pending-auto-button", "wine-oneinput-load", "wine-oneinput-text", "wine-oneinput-mouse-default", "wine-oneinput-long-load", "wine-oneinput-mouse-long", "wine-system-load", "wine-stopcalltrain", "wine-reset"
    ] and
    all(.[]; .ok == true) and
    (map(select(.id == "wine-load"))[0].result.termination == "waitingInput") and
    (map(select(.id == "wine-tooltip-delay"))[0].result.termination == "waitingInput") and
    (map(select(.id == "wine-load"))[0].result.output | contains(["TITLE_CHARANUM=0"])) and
    (map(select(.id == "wine-project"))[0].result.functions | map(.name) | sort == ["EVENTFIRST", "ORACLE_COMPAT", "ORACLE_COMPAT_12", "ORACLE_COMPAT_REST", "ORACLE_DYNAMIC_1", "ORACLE_DYNAMIC_VARIABLES", "ORACLE_HTML_POP", "ORACLE_INPUT", "ORACLE_LINECOUNT", "ORACLE_LIST_TARGET", "ORACLE_MAP", "ORACLE_NATIVE", "ORACLE_PENDING_AUTO_BUTTON", "ORACLE_PRESENTATION", "ORACLE_PRESENTATION_23", "ORACLE_PRESENTATION_3", "ORACLE_PRINT_FAMILY", "ORACLE_REFLECTION", "ORACLE_RESTART_ABILITY", "ORACLE_RESTART_FLOW", "ORACLE_RESTART_MOVE", "ORACLE_STRUCTURED", "ORACLE_TEST", "SYSTEM_TITLE"]) and
    (map(select(.id == "wine-project"))[0].result.functions | map(select(.name == "SYSTEM_TITLE"))[0].lines | map(.functionCode) | contains(["PRINTFORM", "IF", "CALL", "CALL", "ENDIF", "INPUT", "RETURN"])) and
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
    (map(select(.id == "wine-config-drawing"))[0].result.value == "TEXTRENDERER") and
    (map(select(.id == "wine-config-font-size"))[0].result.value == 18) and
    (map(select(.id == "wine-config-fore-color"))[0].result.value == 12632256) and
    (map(select(.id == "wine-config-stain-list"))[0].result.value == "System.Collections.Generic.List`1[System.Int64]") and
    (map(select(.id == "wine-run"))[0].result.termination == "completed") and
    (map(select(.id == "wine-run"))[0].result.output | join("\n") | contains("ORACLE_OK")) and
    (map(select(.id == "wine-html-pop"))[0].result.watches."RESULTS:30" == "A&lt;&amp;<button value=\u002742\u0027>choose</button>") and
    (map(select(.id == "wine-presentation-3"))[0].result.watches."RESULT:40" == 0) and
    (map(select(.id == "wine-presentation-3"))[0].result.watches."RESULT:41" == 1) and
    (map(select(.id == "wine-presentation-3"))[0].result.watches."RESULT:42" == 1) and
    (map(select(.id == "wine-presentation-3"))[0].result.watches."RESULT:43" == 4294901760) and
    (map(select(.id == "wine-presentation-3"))[0].result.watches."RESULT:44" == 1) and
    (map(select(.id == "wine-presentation-3"))[0].result.watches."RESULT:45" == 4278255360) and
    (map(select(.id == "wine-presentation-3"))[0].result.watches."RESULT:46" == 2) and
    (map(select(.id == "wine-presentation-3"))[0].result.watches."RESULT:47" == 1) and
    (map(select(.id == "wine-presentation-3"))[0].result.watches."RESULT:48" == 1) and
    (map(select(.id == "wine-presentation-3"))[0].result.watches."RESULT:49" == 1) and
    (map(select(.id == "wine-presentation-3"))[0].result.watches."RESULT:50" == 1) and
    (map(select(.id == "wine-presentation-3"))[0].result.watches."RESULTS:40" == "a b") and
    (map(select(.id == "wine-presentation-23"))[0].result.termination == "completed") and
    (map(select(.id == "wine-presentation-23"))[0].result.watches."RESULTS:31" == "<button value=\u002716\u0027>[0x10] hex </button><button value=\u0027100\u0027>[1e2] exponent</button>") and
    (map(select(.id == "wine-presentation-23"))[0].result.watches."RESULTS:32" == "<img src=\u0027missing\u0027 srcb=\u0027hover\u0027 srcm=\u0027mask\u0027 height=\u002710px\u0027 width=\u00273\u0027 ypos=\u00277px\u0027>") and
    (map(select(.id == "wine-presentation-23"))[0].result.watches."RESULTS:33" == "<shape type=\u0027rect\u0027 param=\u00271px, 2, 3px, 4\u0027>") and
    (map(select(.id == "wine-presentation-23"))[0].result.watches."RESULTS:34" == "<shape type=\u0027space\u0027 param=\u00275px\u0027>") and
    (map(select(.id == "wine-compat"))[0].result.termination == "completed") and
    (map(select(.id == "wine-compat"))[0].result.watches."FLAG:0" == 4) and
    (map(select(.id == "wine-compat-rest"))[0].result.termination == "completed") and
    (map(select(.id == "wine-compat-rest"))[0].result.watches."RESULT:1" == 0) and
    (map(select(.id == "wine-compat-rest"))[0].result.watches."RESULT:2" == 3) and
    (map(select(.id == "wine-compat-rest"))[0].result.watches."RESULT:3" == 4) and
    (map(select(.id == "wine-compat-rest"))[0].result.watches."RESULT:4" == 1) and
    (map(select(.id == "wine-compat-rest"))[0].result.watches."RESULT:5" == 2) and
    (map(select(.id == "wine-compat-rest"))[0].result.watches."RESULTS:10" == "STORED") and
    (map(select(.id == "wine-compat-rest"))[0].result.watches."FLAG:1" == 7) and
    (map(select(.id == "wine-compat-rest"))[0].result.watches."FLAG:2" == 8) and
    (map(select(.id == "wine-native-tail"))[0].result.watches == {"RESULT:0":0,"RESULT:1":4,"RESULT:2":1,"RESULT:3":1,"RESULT:4":2,"RESULT:5":2,"RESULT:6":1,"RESULT:7":946,"RESULT:8":946,"RESULT:9":12,"RESULT:10":1,"RESULT:11":66051,"RESULT:12":2,"RESULT:13":0,"RESULTS:0":"a\\+b","RESULTS:1":"β","RESULTS:2":"ABC","RESULTS:3":"abc","RESULTS:4":"b/c","RESULTS:5":"a,b,c,","RESULTS:6":"β","RESULTS:7":"ff"}) and
    (map(select(.id == "wine-dynamic-variables"))[0].result.watches == {"RESULT:1":65,"RESULT:2":30028,"RESULT:50":1,"RESULT:51":9,"RESULT:52":16752762,"RESULT:53":-1,"RESULT:54":2,"RESULTS:50":"local","RESULTS:51":"saved","RESULTS:52":"ab","FLAG:4":9,"SAVESTR:0":"saved"}) and
    (map(select(.id == "wine-reflection"))[0].result.watches == {"RESULT:12":1,"RESULT:13":1,"RESULTS:8":"ORACLE_REFLECTION","RESULTS:9":"SAVEDATA_TEXT"}) and
    (map(select(.id == "wine-map"))[0].result.termination == "completed") and
    (map(select(.id == "wine-map"))[0].result.output | join("\n") | contains("MAP=2,1,1,1|3|b,a")) and
    (map(select(.id == "wine-presentation"))[0].result.termination == "completed") and
    (map(select(.id == "wine-presentation"))[0].result.output | join("\n") | contains("VISIBLE")) and
    (map(select(.id == "wine-print-family"))[0].result.termination == "completed") and
    (map(select(.id == "wine-print-family"))[0].result.output | join("\n") | contains("|  7|7  |界  |Target|Call|Call|Target|Call| X")) and
    (map(select(.id == "wine-print-family"))[0].result.output | join("\n") | contains("ヒラガナ")) and
    (map(select(.id == "wine-print-family"))[0].result.output | join("\n") | contains("Right[1]")) and
    (map(select(.id == "wine-print-family"))[0].result.output | join("\n") | contains("Left[2]")) and
    (map(select(.id == "wine-print-family"))[0].result.output | join("\n") | contains("F3[3]")) and
    (map(select(.id == "wine-print-family"))[0].result.output | join("\n") | contains("L[4]")) and
    (map(select(.id == "wine-linecount"))[0].result.termination == "completed") and
    (map(select(.id == "wine-linecount"))[0].result.watches == {"RESULT:50":2,"RESULT:51":1,"RESULT:52":3}) and
    (map(select(.id == "wine-structured"))[0].result.termination == "completed") and
    (map(select(.id == "wine-structured"))[0].result.watches."RESULTS:0" | contains("<xs:schema id=\"NewDataSet\"")) and
    (map(select(.id == "wine-structured"))[0].result.watches."RESULTS:1" | contains("A&amp;B")) and
    (map(select(.id == "wine-structured"))[0].result.watches."RESULTS:2" == "<root><item id=\"a\" kind=\"first\">one</item><item id=\"b\">changed</item></root>") and
    (map(select(.id == "wine-structured"))[0].result.watches."RESULT:4" == 1) and
    (map(select(.id == "wine-structured"))[0].result.watches."RESULT:5" == 1) and
    (map(select(.id == "wine-compat-12"))[0].result.termination == "completed") and
    (map(select(.id == "wine-compat-12"))[0].result.watches == {"RESULT:20":4,"RESULT:21":0,"RESULT:22":0,"RESULT:23":66051,"RESULT:24":3,"RESULTS:20":"&lt;&amp;&gt;&apos;&quot;","RESULTS:21":"A&Bあ","RESULTS:22":"LEFT"}) and
    (map(select(.id == "wine-input"))[0].result.termination == "completed") and
    (map(select(.id == "wine-input"))[0].result.watches.RESULT == 42) and
    (map(select(.id == "wine-restart"))[0].result.termination == "completed") and
    (map(select(.id == "wine-restart"))[0].result.output | join("\n") | contains("move display=1")) and
    (map(select(.id == "wine-restart"))[0].result.output | join("\n") | contains("ability page=1")) and
    (map(select(.id == "wine-restart"))[0].result.output | join("\n") | contains("invalid move") | not) and
    (map(select(.id == "wine-restart"))[0].result.output | join("\n") | contains("invalid ability") | not) and
    (map(select(.id == "wine-pending-auto-button"))[0].result.termination == "completed") and
    (map(select(.id == "wine-pending-auto-button"))[0].result.output | join("\n") | contains("pending auto=58")) and
    (map(select(.id == "wine-oneinput-text"))[0].result.termination == "completed") and
    (map(select(.id == "wine-oneinput-text"))[0].result.watches == {"RESULT:40":1,"RESULT:41":3,"RESULTS:40":"β","RESULTS:41":"y"}) and
    (map(select(.id == "wine-oneinput-mouse-default"))[0].result.watches == {"RESULT:42":4,"RESULTS:42":"L"}) and
    (map(select(.id == "wine-oneinput-mouse-long"))[0].result.watches == {"RESULT:42":42,"RESULTS:42":"LONG"}) and
    (map(select(.id == "wine-system-load"))[0].result.termination == "error") and
    (map(select(.id == "wine-stopcalltrain"))[0].result.termination == "error") and
    (map(select(.id == "wine-stopcalltrain"))[0].result.watches == {"RESULT:30":0,"RESULT:31":1}) and
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
