using MinorShift.Emuera.GameProc;
using MinorShift.Emuera.Runtime;
using MinorShift.Emuera.Runtime.Config;
using MinorShift.Emuera.Runtime.Script.Statements;
using MinorShift.Emuera.UI.Game;
using System;

namespace MinorShift.Emuera.GameView;

internal sealed partial class EmueraConsole
{
	string headlessWindowTitle = string.Empty;

	/// <summary>
	/// Creates the normal script/runtime console without constructing WinForms
	/// controls. This entry point is intentionally gated so the game startup
	/// path continues to use MainWindow and the public constructor unchanged.
	/// </summary>
	internal static EmueraConsole CreateHeadless()
	{
		if (!Program.HeadlessMode)
			throw new InvalidOperationException("The headless console is only available in headless mode");
		return new EmueraConsole();
	}

	private EmueraConsole()
	{
		window = null;
		CBProc = new ClipboardProcessor(null);
		state = ConsoleState.Initializing;
		if (Config.FPS > 0)
			msPerFrame = 1000 / (uint)Config.FPS;
		displayLineList = [];
		printBuffer = new PrintStringBuffer(this);

		genericTimer = new();
		genericTimer.Elapsed += tickTimer;
		genericTimer.Interval = 10;
		genericTimer.Enabled = false;
		CBG_Clear();
		// redrawTimer is a WinForms timer and is deliberately not created.
		redrawTimer = null;
	}

	internal ConsoleState HeadlessState => state;
	internal Process HeadlessProcess => process;
	internal InputRequest HeadlessInputRequest => inputReq;

	internal void ApplyTextBoxChanges()
	{
		if (!Program.HeadlessMode)
			window.ApplyTextBoxChanges();
	}

	internal void HeadlessResume(string input)
	{
		RunEmueraProgram(input);
	}
}
