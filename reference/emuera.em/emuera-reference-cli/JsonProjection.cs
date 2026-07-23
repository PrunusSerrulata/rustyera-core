using MinorShift.Emuera;
using MinorShift.Emuera.Runtime.Script.Data;
using MinorShift.Emuera.Runtime.Script.Parser;
using MinorShift.Emuera.Runtime.Script.Statements;
using System.Collections;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Text.Json.Nodes;

namespace Emuera.ReferenceCli;

internal static class JsonProjection
{
    internal static JsonArray Diagnostics(IEnumerable<HeadlessParserWarning> warnings)
    {
        var result = new JsonArray();
        foreach (var warning in warnings)
        {
            result.Add(new JsonObject
            {
                ["level"] = warning.Level,
                ["message"] = warning.Message,
                ["position"] = warning.Position is { } position
                    ? new JsonObject { ["file"] = position.Filename, ["line"] = position.LineNo }
                    : null,
                ["stack"] = warning.StackTrace,
            });
        }
        return result;
    }

    internal static JsonArray Words(WordCollection words)
    {
        var result = new JsonArray();
        foreach (var word in words.Collection) result.Add(Word(word));
        return result;
    }

    static JsonNode Word(Word word)
    {
        var result = new JsonObject
        {
            ["type"] = word.GetType().Name,
            ["kind"] = word.Type.ToString(),
            ["isMacro"] = word.IsMacro,
        };
        switch (word)
        {
            case IdentifierWord identifier: result["code"] = identifier.Code; break;
            case LiteralIntegerWord integer: result["value"] = integer.Int; break;
            case LiteralStringWord text: result["value"] = text.Str; break;
            case OperatorWord op: result["operator"] = op.Code.ToString(); break;
            case SymbolWord symbol: result["symbol"] = symbol.Type.ToString(); break;
            case MacroWord macro: result["index"] = macro.Number; break;
            case StrFormWord form:
                var strings = new JsonArray();
                foreach (var item in form.Strs) strings.Add(item);
                result["strings"] = strings;
                var substitutions = new JsonArray();
                foreach (var subword in form.SubWords) substitutions.Add(SubWord(subword));
                result["subwords"] = substitutions;
                break;
        }
        return result;
    }

    static JsonNode SubWord(SubWord subword)
    {
        var result = new JsonObject
        {
            ["type"] = subword.GetType().Name,
            ["isMacro"] = subword.IsMacro,
        };
        if (subword.Words is not null) result["tokens"] = Words(subword.Words);
        if (subword is TripleSymbolSubWord triple) result["symbol"] = triple.Code.ToString();
        if (subword is YenAtSubWord conditional)
        {
            result["left"] = conditional.Left is null ? null : Word(conditional.Left);
            result["right"] = conditional.Right is null ? null : Word(conditional.Right);
        }
        return result;
    }

    internal static JsonNode? LogicalLine(LogicalLine? line)
    {
        if (line is null) return null;
        var result = new JsonObject
        {
            ["type"] = line.GetType().Name,
            ["isError"] = line.IsError,
            ["error"] = string.IsNullOrEmpty(line.ErrMes) ? null : line.ErrMes,
            ["position"] = Graph(line.Position),
        };
        if (line is InstructionLine instruction)
        {
            result["functionCode"] = instruction.FunctionCode.ToString();
            result["functionName"] = instruction.Function.Name;
            result["assignmentOperator"] = instruction.AssignOperator.ToString();
            result["argument"] = Graph(instruction.Argument);
        }
        else if (line is FunctionLabelLine function)
        {
            result["label"] = function.LabelName;
            result["isMethod"] = function.IsMethod;
        }
        else if (line is GotoLabelLine label) result["label"] = label.LabelName;
        result["rawGraph"] = Graph(line);
        return result;
    }

    internal static JsonNode Project(LabelDictionary labels)
    {
        var functions = new JsonArray();
        var ordered = labels.GetAllLabels(true)
            .OrderBy(label => label.FileIndex)
            .ThenBy(label => label.Position?.LineNo ?? -1)
            .ThenBy(label => label.Index);
        foreach (var function in ordered)
        {
            var logicalLines = new List<LogicalLine>();
            var current = function.NextLine;
            while (current is not null and not NullLine and not FunctionLabelLine)
            {
                logicalLines.Add(current);
                current = current.NextLine;
            }
            var lineIndices = logicalLines
                .Select((line, index) => (line, index))
                .ToDictionary(pair => pair.line, pair => pair.index);
            var lines = new JsonArray();
            for (var index = 0; index < logicalLines.Count; index++)
            {
                var line = logicalLines[index];
                var projected = new JsonObject
                {
                    ["index"] = index,
                    ["type"] = line.GetType().Name,
                    ["isError"] = line.IsError,
                    ["error"] = string.IsNullOrEmpty(line.ErrMes) ? null : line.ErrMes,
                    ["position"] = line.Position is { } position
                        ? new JsonObject { ["file"] = position.Filename, ["line"] = position.LineNo }
                        : null,
                };
                if (line is InstructionLine instruction)
                {
                    projected["functionCode"] = instruction.FunctionCode.ToString();
                    projected["functionName"] = instruction.Function.Name;
                    projected["assignmentOperator"] = instruction.AssignOperator.ToString();
                    projected["argumentType"] = instruction.Argument?.GetType().Name;
                    projected["argument"] = Graph(instruction.Argument, 4);
                    projected["jumpTo"] = instruction.JumpTo is not null
                        && lineIndices.TryGetValue(instruction.JumpTo, out var jumpTo) ? jumpTo : null;
                    projected["jumpToEndCatch"] = instruction.JumpToEndCatch is not null
                        && lineIndices.TryGetValue(instruction.JumpToEndCatch, out var jumpToEndCatch)
                        ? jumpToEndCatch : null;
                }
                else if (line is GotoLabelLine label) projected["label"] = label.LabelName;
                lines.Add(projected);
            }
            functions.Add(new JsonObject
            {
                ["name"] = function.LabelName,
                ["isError"] = function.IsError,
                ["isEvent"] = function.IsEvent,
                ["isSystem"] = function.IsSystem,
                ["isMethod"] = function.IsMethod,
                ["returnType"] = function.MethodType.FullName,
                ["parameters"] = Graph(function.Arg, 4),
                ["defaults"] = Graph(function.Def, 4),
                ["lines"] = lines,
            });
        }
        return new JsonObject { ["functions"] = functions };
    }

    internal static JsonNode? Graph(object? value, int maxDepth = 8) =>
        new StableGraphWriter(maxDepth).Write(value);

    sealed class StableGraphWriter(int maxDepth)
    {
        readonly Dictionary<object, int> seen = new(ReferenceComparer.Instance);
        int nextId = 1;

        internal JsonNode? Write(object? value, int depth = 0)
        {
            if (value is null) return null;
            var type = value.GetType();
            if (value is string text) return JsonValue.Create(text);
            if (value is char character) return JsonValue.Create(character.ToString());
            if (value is bool boolean) return JsonValue.Create(boolean);
            if (value is byte or sbyte or short or ushort or int or uint or long or ulong)
                return JsonValue.Create(Convert.ToString(value, System.Globalization.CultureInfo.InvariantCulture));
            if (value is float or double or decimal)
                return JsonValue.Create(Convert.ToString(value, System.Globalization.CultureInfo.InvariantCulture));
            if (value is Enum) return JsonValue.Create(value.ToString());
            if (value is Type runtimeType) return JsonValue.Create(runtimeType.FullName);
            if (depth >= maxDepth) return new JsonObject { ["$truncated"] = type.FullName };
            if (!type.IsValueType)
            {
                if (seen.TryGetValue(value, out var existing)) return new JsonObject { ["$ref"] = existing };
                seen[value] = nextId++;
            }
            if (value is IDictionary dictionary)
            {
                var map = new JsonObject();
                var entries = dictionary.Cast<DictionaryEntry>()
                    .OrderBy(entry => entry.Key?.ToString(), StringComparer.Ordinal)
                    .Take(256);
                foreach (DictionaryEntry entry in entries)
                    map[entry.Key?.ToString() ?? "null"] = Write(entry.Value, depth + 1);
                if (dictionary.Count > 256) map["$truncatedItems"] = dictionary.Count - 256;
                return map;
            }
            if (value is IEnumerable enumerable)
            {
                var array = new JsonArray();
                var count = 0;
                foreach (var item in enumerable)
                {
                    if (count++ == 256)
                    {
                        array.Add(new JsonObject { ["$truncatedItems"] = true });
                        break;
                    }
                    array.Add(Write(item, depth + 1));
                }
                return array;
            }
            var result = new JsonObject { ["$type"] = type.FullName };
            if (!type.IsValueType) result["$id"] = seen[value];
            foreach (var field in Fields(type))
            {
                try { result[field.Name] = Write(field.GetValue(value), depth + 1); }
                catch (Exception error) { result[field.Name] = new JsonObject { ["$error"] = error.Message }; }
            }
            return result;
        }

        static IEnumerable<FieldInfo> Fields(Type type) => type
            .GetFields(BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic)
            .Where(field => !field.IsStatic && !field.Name.Contains("window", StringComparison.OrdinalIgnoreCase)
                && !field.Name.Contains("console", StringComparison.OrdinalIgnoreCase))
            .OrderBy(field => field.Name, StringComparer.Ordinal);
    }

    sealed class ReferenceComparer : IEqualityComparer<object>
    {
        internal static readonly ReferenceComparer Instance = new();
        public new bool Equals(object? x, object? y) => ReferenceEquals(x, y);
        public int GetHashCode(object obj) => RuntimeHelpers.GetHashCode(obj);
    }
}
