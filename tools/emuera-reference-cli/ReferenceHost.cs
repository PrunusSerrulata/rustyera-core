using MinorShift.Emuera;
using MinorShift.Emuera.GameView;
using MinorShift.Emuera.Runtime.Config;
using MinorShift.Emuera.Runtime.Config.JSON;
using MinorShift.Emuera.Runtime.Script.Parser;
using MinorShift.Emuera.Runtime.Script.Statements.Expression;
using MinorShift.Emuera.Runtime.Utils;
using MinorShift.Emuera.Runtime.Utils.EvilMask;
using System.Globalization;
using System.Text.Json.Nodes;

namespace Emuera.ReferenceCli;

internal sealed class ReferenceHost : IDisposable
{
    EmueraConsole? console;
    string? gameDirectory;

    internal EmueraConsole? ConsoleOrNull => console;
    internal bool IsLoaded => console?.HeadlessProcess is not null;

    internal async Task<JsonNode> Load(JsonObject request)
    {
        Reset();
        var directory = Path.GetFullPath(OracleService.RequiredString(request, "gameDir"));
        if (!Directory.Exists(Path.Combine(directory, "csv")) || !Directory.Exists(Path.Combine(directory, "erb")))
            throw new DirectoryNotFoundException("gameDir must contain csv and erb directories");
        System.Text.Encoding.RegisterProvider(System.Text.CodePagesEncodingProvider.Instance);
        CultureInfo.CurrentCulture = CultureInfo.InvariantCulture;
        CultureInfo.DefaultThreadCurrentCulture = CultureInfo.InvariantCulture;
        MinorShift.Emuera.Program.ConfigureHeadless(directory, request["debug"]?.GetValue<bool>() ?? false);
        ConfigData.Instance.LoadConfig();
        JSONConfig.Load();
        Lang.LoadLanguageFiles();
        Lang.SetLanguage();
        // Construct the regular Emuera backend without creating a native
        // WinForms handle. Creating a hidden Form still blocks under Wine when
        // no interactive desktop is available.
        console = EmueraConsole.CreateHeadless();
        console.noOutputLog = true;
        await console.Initialize();
        gameDirectory = directory;
        return Snapshot(Array.Empty<string>());
    }

    internal JsonNode Execute(JsonObject request)
    {
        RequireLoaded();
        ConfigureLimits(request);
        console!.HeadlessProcess.HeadlessExecuteLine(OracleService.RequiredString(request, "statement"));
        return Snapshot(ReadWatches(request));
    }

    internal JsonNode Run(JsonObject request)
    {
        RequireLoaded();
        ConfigureLimits(request);
        if (request["entry"] is JsonValue entryNode)
        {
            var entry = entryNode.GetValue<string>();
            var arguments = request["arguments"]?.GetValue<string>();
            console!.HeadlessProcess.HeadlessPrepareCall(entry, arguments ?? string.Empty);
            console!.HeadlessResume(null!);
        }
        if (request["inputs"] is JsonArray inputs)
        {
            foreach (var input in inputs)
            {
                if (console!.HeadlessState != ConsoleState.WaitInput) break;
                console.HeadlessResume(input?.ToString() ?? string.Empty);
            }
        }
        return Snapshot(ReadWatches(request));
    }

    internal void RequireLoaded()
    {
        if (!IsLoaded) throw new InvalidOperationException("operation requires a loaded game; call 'load' first");
    }

    void ConfigureLimits(JsonObject request)
    {
        var instructions = request["instructionLimit"]?.GetValue<long>() ?? 1_000_000;
        var timeoutMs = request["timeoutMs"]?.GetValue<int>() ?? 10_000;
        if (instructions <= 0) throw new ArgumentOutOfRangeException("instructionLimit", "must be positive");
        if (timeoutMs <= 0) throw new ArgumentOutOfRangeException("timeoutMs", "must be positive");
        console!.HeadlessProcess.ConfigureHeadlessLimits(instructions, TimeSpan.FromMilliseconds(timeoutMs));
    }

    JsonNode Snapshot(IEnumerable<string> watches)
    {
        var output = new JsonArray();
        if (console is not null)
            foreach (var line in console.DisplayLineList) output.Add(line.ToString());
        var result = new JsonObject
        {
            ["gameDir"] = gameDirectory,
            ["state"] = console?.HeadlessState.ToString(),
            ["termination"] = Termination(),
            ["output"] = output,
            ["instructionCount"] = console?.HeadlessProcess?.HeadlessInstructionCount ?? 0,
            ["position"] = JsonProjection.Graph(console?.HeadlessProcess?.GetRunningPosition()),
        };
        if (console?.HeadlessProcess?.HeadlessRunCompleted != true && console?.HeadlessInputRequest is { } input)
            result["inputRequest"] = JsonProjection.Graph(input, 3);
        var watchValues = new JsonObject();
        foreach (var expression in watches)
        {
            try { watchValues[expression] = Evaluate(expression); }
            catch (Exception error) { watchValues[expression] = new JsonObject { ["error"] = error.Message }; }
        }
        result["watches"] = watchValues;
        return result;
    }

    JsonNode? Evaluate(string source)
    {
        var words = LexicalAnalyzer.Analyse(new CharStream(source), LexEndWith.EoL, LexAnalyzeFlag.None);
        var expression = ExpressionParser.ReduceExpressionTerm(words, TermEndWith.EoL)
            ?? throw new CodeEE("watch expression is empty");
        return expression.IsInteger
            ? JsonValue.Create(expression.GetIntValue(GlobalStatic.EMediator))
            : JsonValue.Create(expression.GetStrValue(GlobalStatic.EMediator));
    }

    string Termination()
    {
        var limit = console?.HeadlessProcess?.HeadlessLimitReason;
        if (limit is not null) return limit;
        if (console?.HeadlessProcess?.HeadlessRunCompleted == true) return "completed";
        return console?.HeadlessState switch
        {
            ConsoleState.WaitInput => "waitingInput",
            ConsoleState.Quit => "quit",
            ConsoleState.Error => "error",
            ConsoleState.Running => "running",
            _ => console?.HeadlessState.ToString() ?? "notLoaded",
        };
    }

    static IEnumerable<string> ReadWatches(JsonObject request) =>
        request["watch"] is JsonArray array ? array.Select(item => item!.GetValue<string>()) : [];

    internal void Reset()
    {
        console?.Dispose();
        console = null;
        gameDirectory = null;
        GlobalStatic.Reset();
    }

    public void Dispose() => Reset();
}
