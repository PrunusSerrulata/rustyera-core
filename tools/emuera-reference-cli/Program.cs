using System.Text.Json;
using System.Text.Json.Nodes;

namespace Emuera.ReferenceCli;

internal static class Program
{
    [STAThread]
    private static int Main()
    {
        Console.InputEncoding = System.Text.Encoding.UTF8;
        Console.OutputEncoding = System.Text.Encoding.UTF8;
        var service = new OracleService();
        string? line;
        while ((line = Console.ReadLine()) is not null)
        {
            JsonObject response;
            try
            {
                var request = JsonNode.Parse(line) as JsonObject
                    ?? throw new JsonException("request must be a JSON object");
                response = service.Handle(request).GetAwaiter().GetResult();
            }
            catch (Exception exception)
            {
                response = Response.Error(null, exception, new JsonArray());
            }
            Console.WriteLine(response.ToJsonString(JsonOptions.Compact));
            Console.Out.Flush();
        }
        service.Dispose();
        return 0;
    }
}

internal static class JsonOptions
{
    internal static readonly JsonSerializerOptions Compact = new()
    {
        WriteIndented = false,
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
    };
}

internal static class Response
{
    internal const int SchemaVersion = 1;
    internal const string ReferenceCommit = "26a35dc9334bb67590b96f7b8efbefbf199e391e";

    internal static JsonObject Success(JsonNode? id, JsonNode? result, JsonArray diagnostics) => new()
    {
        ["id"] = id?.DeepClone(), ["ok"] = true, ["schemaVersion"] = SchemaVersion,
        ["referenceCommit"] = ReferenceCommit, ["diagnostics"] = diagnostics, ["result"] = result,
    };

    internal static JsonObject Error(JsonNode? id, Exception exception, JsonArray diagnostics) => new()
    {
        ["id"] = id?.DeepClone(), ["ok"] = false, ["schemaVersion"] = SchemaVersion,
        ["referenceCommit"] = ReferenceCommit, ["diagnostics"] = diagnostics,
        ["error"] = new JsonObject
        {
            ["type"] = exception.GetType().FullName,
            ["message"] = exception.Message,
        },
    };
}
