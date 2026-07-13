namespace MinorShift.Emuera;

/// <summary>Access points used only by the deterministic reference oracle.</summary>
static partial class Program
{
	internal static bool HeadlessMode { get; private set; }

	internal static void ConfigureHeadless(string baseDirectory, bool debugMode = false)
	{
		HeadlessMode = true;
		DebugMode = debugMode;
		AnalysisMode = false;
		AnalysisFiles = [];
		SetDirPaths(baseDirectory);
	}
}
