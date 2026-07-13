using MinorShift.Emuera.GameProc.Function;
using MinorShift.Emuera.Runtime.Script;
using MinorShift.Emuera.Runtime.Script.Parser;
using MinorShift.Emuera.Runtime.Script.Statements;
using MinorShift.Emuera.Runtime.Utils;
using System;
using System.Diagnostics;

namespace MinorShift.Emuera.GameProc;

internal sealed partial class Process
{
	long headlessInstructionLimit = long.MaxValue;
	long headlessInstructionCount;
	TimeSpan headlessTimeout = TimeSpan.MaxValue;
	readonly Stopwatch headlessStopwatch = new();
	bool headlessFunctionRun;

	internal long HeadlessInstructionCount => headlessInstructionCount;
	internal string HeadlessLimitReason { get; private set; }
	internal bool HeadlessRunCompleted { get; private set; }

	internal void ConfigureHeadlessLimits(long instructionLimit, TimeSpan timeout)
	{
		headlessInstructionLimit = instructionLimit > 0 ? instructionLimit : long.MaxValue;
		headlessTimeout = timeout > TimeSpan.Zero ? timeout : TimeSpan.MaxValue;
		headlessInstructionCount = 0;
		HeadlessLimitReason = null;
		headlessStopwatch.Restart();
	}

	/// <summary>
	/// Starts a user function as an isolated VM entry point. Its argument text is
	/// still parsed by Emuera's real CALL argument builder, but a null return
	/// address intentionally makes the function the root of this run.
	/// </summary>
	internal void HeadlessPrepareCall(string functionName, string rawArguments)
	{
		string source = $"CALL {functionName}{(string.IsNullOrWhiteSpace(rawArguments) ? "" : "," + rawArguments)}";
		LogicalLine logicalLine = LogicalLineParser.ParseLine(source, console);
		if (logicalLine is not InstructionLine instruction)
			throw new CodeEE(logicalLine?.ErrMes ?? "Failed to parse function call");
		if (!ArgumentParser.SetArgumentTo(instruction) ||
			instruction.Argument is not SpCallArgment callArgument)
			throw new CodeEE(instruction.ErrMes ?? "Failed to parse function arguments");
		CalledFunction call = CalledFunction.CallFunction(this, functionName, null);
		if (call == null)
			throw new CodeEE($"Function is not defined: {functionName}");
		UserDefinedFunctionArgument arguments = call.ConvertArg(callArgument.RowArgs, out string error);
		if (arguments == null)
			throw new CodeEE(error);

		state.ClearFunctionList();
		state.IntoFunction(call, arguments, exm);
		headlessFunctionRun = true;
		HeadlessRunCompleted = false;
	}

	/// <summary>Stops DoScript before it re-enters Emuera's title/system state machine.</summary>
	bool HeadlessFinishFunctionRun()
	{
		if (!Program.HeadlessMode || !headlessFunctionRun)
			return false;
		headlessFunctionRun = false;
		HeadlessRunCompleted = true;
		return true;
	}

	/// <summary>
	/// Parses and dispatches one source statement with the same expression
	/// mediator and process state used by the loaded VM. This is the smallest
	/// useful bridge around the UI/debug-console layer.
	/// </summary>
	internal FunctionCode HeadlessExecuteLine(string source)
	{
		HeadlessCheckLimit();
		LogicalLine logicalLine = LogicalLineParser.ParseLine(source, console);
		if (logicalLine is not InstructionLine instruction)
			throw new CodeEE("Headless execution requires an instruction line");
		if (!ArgumentParser.SetArgumentTo(instruction) || instruction.IsError)
			throw new CodeEE(instruction.ErrMes ?? "Failed to parse instruction arguments");
		if (instruction.Function.IsFlowContorol())
			throw new CodeEE("Control-flow instructions require the headless function runner");

		if (instruction.Function.Instruction != null)
		{
			bool useCallForm = false;
			string functionNotFoundName = null;
			instruction.Function.Instruction.SetJumpTo(
				ref useCallForm,
				instruction,
				0,
				ref functionNotFoundName);
			if (!string.IsNullOrEmpty(functionNotFoundName))
				throw new CodeEE($"Function is not defined: {functionNotFoundName}");
			instruction.Function.Instruction.DoInstruction(exm, instruction, state);
		}
		else if (instruction.Function.IsFlowContorol())
			doFlowControlFunction(instruction);
		else
			doNormalFunction(instruction);
		return instruction.FunctionCode;
	}

	void HeadlessCheckLimit()
	{
		if (!Program.HeadlessMode)
			return;
		headlessInstructionCount++;
		if (headlessInstructionCount > headlessInstructionLimit)
		{
			HeadlessLimitReason = "instructionLimit";
			throw new CodeEE("Headless instruction limit exceeded");
		}
		if (headlessStopwatch.Elapsed > headlessTimeout)
		{
			HeadlessLimitReason = "timeout";
			throw new CodeEE("Headless execution timeout exceeded");
		}
	}
}
