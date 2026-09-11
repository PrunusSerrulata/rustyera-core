using System.Reflection;
using System.Runtime.InteropServices;
using System.Runtime.Loader;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;

// Do not substitute the machine's implicit CodePages provider.
if (args.Length != 4)
    throw new ArgumentException("Arguments: provider.dll expected-provider-sha256 source-directory fresh-output-directory");
string providerPath = Path.GetFullPath(args[0]);
string expectedHash = args[1].ToLowerInvariant();
string sourcePath = Path.GetFullPath(args[2]);
string outputPath = Path.GetFullPath(args[3]);
string Hash(string path) => Convert.ToHexString(SHA256.HashData(File.ReadAllBytes(path))).ToLowerInvariant();
if (expectedHash.Length != 64 || Hash(providerPath) != expectedHash)
    throw new InvalidOperationException("Provider SHA256 differs from the recorded original-publish DLL");
if (Directory.Exists(outputPath) || File.Exists(outputPath))
    throw new InvalidOperationException("Output must not exist; preserve prior capture evidence");
var sourceHashes = new SortedDictionary<string, string>();
foreach (string name in new[] { "Program.cs", "LegacyEncodingMap.csproj" })
    sourceHashes[name] = Hash(Path.Combine(sourcePath, name));

// A distinct context prevents implicit resolution to another shared-framework copy.
// EncodingProvider/Encoding themselves remain in the shared System.Private.CoreLib.
var context = new AssemblyLoadContext("pinned-original-codepages", isCollectible: false);
Assembly assembly = context.LoadFromAssemblyPath(providerPath);
if (Path.GetFullPath(assembly.Location) != providerPath || Hash(assembly.Location) != expectedHash)
    throw new InvalidOperationException("Actual provider assembly identity differs");
Type providerType = assembly.GetType("System.Text.CodePagesEncodingProvider", throwOnError: true)!;
var provider = (EncodingProvider)providerType.GetProperty("Instance", BindingFlags.Public | BindingFlags.Static)!.GetValue(null)!;
Encoding.RegisterProvider(provider);
int[] codePages = [932, 949, 936, 950];
var lengths = new Dictionary<int, int[]>();
var roundtrips = new Dictionary<int, bool[]>();
var outputs = new SortedDictionary<string, object>();
var violations = new List<object>();
var fallbackIdentity = new SortedDictionary<int, object>();
Directory.CreateDirectory(outputPath);
foreach (int codePage in codePages)
{
    // Direct provider call guarantees this instance; no global GetEncoding selection.
    Encoding encoding = provider.GetEncoding(codePage)
        ?? throw new InvalidOperationException($"Pinned provider has no code page {codePage}");
    var widths = new int[65536];
    var exact = new bool[65536];
    lengths[codePage] = widths;
    roundtrips[codePage] = exact;
    fallbackIdentity[codePage] = new {
        encodingType = encoding.GetType().AssemblyQualifiedName,
        encoderFallback = encoding.EncoderFallback.GetType().AssemblyQualifiedName,
        decoderFallback = encoding.DecoderFallback.GetType().AssemblyQualifiedName,
    };
    string name = $"cp{codePage}.tuples.bin";
    string path = Path.Combine(outputPath, name);
    using (var writer = new BinaryWriter(File.Create(path)))
    {
        for (int unit = 0; unit <= ushort.MaxValue; unit++)
        {
            // Exactly the upstream c.ToString(), including isolated surrogate units.
            string value = new((char)unit, 1);
            byte[] bytes = encoding.GetBytes(value);
            bool matches = string.Equals(encoding.GetString(bytes), value, StringComparison.Ordinal);
            widths[unit] = bytes.Length;
            exact[unit] = matches;
            writer.Write((ushort)unit);
            writer.Write((uint)bytes.Length);
            writer.Write((byte)(matches ? 1 : 0));
            if (bytes.Length is not (1 or 2))
                violations.Add(new { codePage, unit, emittedLength = bytes.Length, roundtrips = matches });
        }
    }
    outputs[name] = new { sha256 = Hash(path), bytes = new FileInfo(path).Length };
}

// Raw tuples are persisted before the width-domain assertion; failed captures
// retain those tuples plus a failure manifest and never emit final-width bitsets.
if (violations.Count == 0)
{
    foreach (int codePage in codePages)
    {
        var bits = new byte[8192];
        for (int unit = 0; unit <= ushort.MaxValue; unit++)
        {
            int width = roundtrips[codePage][unit] ? lengths[codePage][unit]
                : roundtrips[932][unit] ? lengths[932][unit] : lengths[codePage][unit];
            if (width is not (1 or 2))
                throw new InvalidOperationException("Final width escaped validated domain");
            if (width == 2)
                bits[unit >> 3] |= (byte)(1 << (unit & 7));
        }
        string name = $"cp{codePage}.final-width.bits";
        string path = Path.Combine(outputPath, name);
        File.WriteAllBytes(path, bits);
        outputs[name] = new { sha256 = Hash(path), bytes = bits.Length };
    }
}
var manifest = new {
    schema = "upstream-7b69-utf16-codepage-tuples-v1",
    status = violations.Count == 0 ? "generated_not_behaviorally_validated" : "invalid_emitted_width_domain",
    upstreamSemanticSha = "7b69ebd27378c03c32b6477b74901bfc3d33223c",
    provider = new {
        path = providerPath, sha256 = expectedHash, fullName = assembly.FullName,
        assemblyVersion = assembly.GetName().Version?.ToString(),
        informationalVersion = assembly.GetCustomAttribute<AssemblyInformationalVersionAttribute>()?.InformationalVersion,
        mvid = assembly.ManifestModule.ModuleVersionId,
    },
    generatorSources = sourceHashes,
    generatorAssembly = new { path = Assembly.GetExecutingAssembly().Location, sha256 = Hash(Assembly.GetExecutingAssembly().Location) },
    runtime = new { framework = RuntimeInformation.FrameworkDescription, os = RuntimeInformation.OSDescription, architecture = RuntimeInformation.ProcessArchitecture.ToString() },
    codePages, fallbackIdentity,
    rawFormat = "Per code page: 65536 ascending records, each u16 UTF16 unit LE, u32 emitted byte length LE, u8 roundtrip (0/1); 7 bytes per record, no header",
    bitsetFormat = "8192 bytes per code page; bit (unit & 7) of byte (unit >> 3), LSB first; 0=width1, 1=width2; selected exact else CP932 exact else selected emitted length",
    violations, outputs,
};
string manifestPath = Path.Combine(outputPath, "manifest.json");
File.WriteAllText(manifestPath, JsonSerializer.Serialize(manifest, new JsonSerializerOptions { WriteIndented = true }) + "\n", new UTF8Encoding(false));
File.WriteAllText(Path.Combine(outputPath, "manifest.sha256"), Hash(manifestPath) + "  manifest.json\n", new UTF8Encoding(false));
if (violations.Count != 0)
    throw new InvalidOperationException("Width-domain assertion failed; inspect preserved raw tuples and manifest");
Console.WriteLine(manifestPath);
