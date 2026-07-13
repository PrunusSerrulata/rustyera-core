using MinorShift.Emuera.GameProc;
using MinorShift.Emuera.Runtime;

namespace MinorShift.Emuera.GameView;

internal sealed partial class EmueraConsole
{
	internal ConsoleState HeadlessState => state;
	internal Process HeadlessProcess => process;
	internal InputRequest HeadlessInputRequest => inputReq;

	internal void HeadlessResume(string input)
	{
		RunEmueraProgram(input);
	}
}
