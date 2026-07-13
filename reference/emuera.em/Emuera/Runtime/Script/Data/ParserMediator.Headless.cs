using MinorShift.Emuera.Runtime.Utils;
using System.Collections.Generic;

namespace MinorShift.Emuera;

/// <summary>Serializable warning data exposed to the reference oracle.</summary>
internal readonly record struct HeadlessParserWarning(
	string Message,
	ScriptPosition? Position,
	int Level,
	string StackTrace);

internal partial class ParserMediator
{
	/// <summary>
	/// Atomically returns and clears parser warnings so each NDJSON response owns
	/// exactly the diagnostics produced while handling its request.
	/// </summary>
	internal static List<HeadlessParserWarning> HeadlessDrainWarnings()
	{
		lock (warningListLock)
		{
			List<HeadlessParserWarning> result = new(warningList.Count);
			foreach (ParserWarning warning in warningList)
			{
				result.Add(new HeadlessParserWarning(
					warning.WarningMes,
					warning.WarningPos,
					warning.WarningLevel,
					warning.StackTrace));
			}
			warningList.Clear();
			return result;
		}
	}
}
