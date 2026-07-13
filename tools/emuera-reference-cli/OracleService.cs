using MinorShift.Emuera;
using MinorShift.Emuera.GameProc.Function;
using MinorShift.Emuera.Runtime.Script.Parser;
using MinorShift.Emuera.Runtime.Script.Statements;
using MinorShift.Emuera.Runtime.Script.Statements.Expression;
using MinorShift.Emuera.Runtime.Utils;
using System.Text.Json.Nodes;

namespace Emuera.ReferenceCli;

internal sealed class OracleService : IDisposable
{
    readonly ReferenceHost host = new();

    internal async Task<JsonObject> Handle(JsonObject request)
    {
        var id = request["id"];
        // Prevent a previous failed request from leaking warnings into this one.
        ParserMediator.HeadlessDrainWarnings();
        try
        {
            var op = request["op"]?.GetValue<string>()
                ?? throw new ArgumentException("missing string property 'op'");
            JsonNode? result = op switch
            {
                "capabilities" => Capabilities(),
                "reset" => Reset(),
                "lex" => Lex(request),
                "parseExpression" => ParseExpression(request, false),
                "parseLine" => ParseLine(request, request["reduceArguments"]?.GetValue<bool>() == true),
                "analyzeLine" => AnalyzeLine(request),
                "load" => await host.Load(request),
                "eval" => ParseExpression(request, true),
                "execute" => host.Execute(request),
                "run" => host.Run(request),
                _ => throw new ArgumentException($"unknown operation '{op}'"),
            };
            return Response.Success(id, result, JsonProjection.Diagnostics(ParserMediator.HeadlessDrainWarnings()));
        }
        catch (Exception exception)
        {
            return Response.Error(id, exception, JsonProjection.Diagnostics(ParserMediator.HeadlessDrainWarnings()));
        }
    }

    JsonNode Capabilities() => new JsonObject
    {
        ["protocol"] = "ndjson",
        ["platform"] = "windows",
        ["operations"] = new JsonArray("capabilities", "reset", "lex", "parseExpression", "parseLine", "analyzeLine", "load", "eval", "execute", "run"),
        ["emueraVersion"] = AssemblyData.EmueraVersionText,
    };

    JsonNode Reset()
    {
        host.Reset();
        return new JsonObject { ["reset"] = true };
    }

    JsonNode Lex(JsonObject request)
    {
        var source = RequiredString(request, "source");
        var end = Enum.Parse<LexEndWith>(request["endWith"]?.GetValue<string>() ?? "EoL", true);
        var flags = ParseLexFlags(request["flags"] as JsonArray);
        var stream = new CharStream(source);
        var words = LexicalAnalyzer.Analyse(stream, end, flags);
        return new JsonObject
        {
            ["tokens"] = JsonProjection.Words(words),
            ["consumedUtf16"] = stream.CurrentPosition,
            ["consumedUtf8"] = System.Text.Encoding.UTF8.GetByteCount(source.AsSpan(0, Math.Min(stream.CurrentPosition, source.Length))),
        };
    }

    JsonNode ParseExpression(JsonObject request, bool evaluate)
    {
        var source = RequiredString(request, "source");
        var words = LexicalAnalyzer.Analyse(new CharStream(source), LexEndWith.EoL, LexAnalyzeFlag.None);
        var expression = ExpressionParser.ReduceExpressionTerm(words, TermEndWith.EoL)
            ?? throw new CodeEE("expression is empty");
        var result = new JsonObject
        {
            ["expression"] = JsonProjection.Graph(expression),
            ["operandType"] = expression.GetOperandType().FullName,
        };
        if (evaluate)
        {
            host.RequireLoaded();
            result["value"] = expression.IsInteger
                ? JsonValue.Create(expression.GetIntValue(GlobalStatic.EMediator))
                : JsonValue.Create(expression.GetStrValue(GlobalStatic.EMediator));
        }
        return result;
    }

    JsonNode? AnalyzeLine(JsonObject request)
    {
        host.RequireLoaded();
        return ParseLine(request, true);
    }

    JsonNode? ParseLine(JsonObject request, bool reduceArguments)
    {
        var source = RequiredString(request, "source");
        var line = LogicalLineParser.ParseLine(source, host.ConsoleOrNull);
        if (line is InstructionLine instruction && reduceArguments)
            ArgumentParser.SetArgumentTo(instruction);
        return JsonProjection.LogicalLine(line);
    }

    static LexAnalyzeFlag ParseLexFlags(JsonArray? flags)
    {
        var value = LexAnalyzeFlag.None;
        if (flags is null) return value;
        foreach (var item in flags)
            value |= Enum.Parse<LexAnalyzeFlag>(item!.GetValue<string>(), true);
        return value;
    }

    internal static string RequiredString(JsonObject request, string name) =>
        request[name]?.GetValue<string>() ?? throw new ArgumentException($"missing string property '{name}'");

    public void Dispose() => host.Dispose();
}
