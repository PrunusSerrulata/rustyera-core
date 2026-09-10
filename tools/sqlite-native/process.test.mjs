import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";
import test from "node:test";
import path from "node:path";
import { executable, run } from "./build.mjs";

test(
  "Windows executable discovery accepts npm's case-insensitive Path",
  {
    skip: process.platform !== "win32",
  },
  async () => {
    const directory = path.dirname(process.execPath);
    const name = path.basename(process.execPath, ".exe");
    for (const key of ["Path", "pAtH", "PATH"]) {
      assert.equal(
        await executable(name, { [key]: directory }),
        process.execPath,
      );
    }
  },
);

function childFixture(start) {
  const child = new EventEmitter();
  Object.assign(child, {
    pid: 1234,
    exitCode: null,
    signalCode: null,
    stdout: new PassThrough(),
    stderr: new PassThrough(),
  });
  child.kill = () => {
    child.exitCode = 1;
    return true;
  };
  return {
    child,
    spawnChild: (_command, _args, options) => {
      assert.equal(options.windowsHide, true);
      queueMicrotask(() => start?.(child));
      return child;
    },
  };
}

test("tool completion preserves output and removes signal listeners", async () => {
  const listeners = process.listenerCount("SIGINT");
  const fixture = childFixture((child) => {
    child.stdout.write("identity\n");
    child.exitCode = 0;
    child.emit("close", 0);
  });
  assert.equal(await run("clang", [], {}, 100, fixture), "identity");
  assert.equal(process.listenerCount("SIGINT"), listeners);
});

test("spawn failure settles even without a close event", async () => {
  const fixture = childFixture((child) => {
    child.pid = undefined;
    child.emit("error", new Error("spawn failed"));
  });
  await assert.rejects(
    run("missing", [], {}, 100, {
      ...fixture,
      platform: "win32",
      terminateTree: async () => {},
      cleanupDeadlineMs: 20,
    }),
    (error) => {
      assert.match(error.message, /descendant exit is unconfirmed/);
      assert.match(error.errors[0].message, /spawn failed/);
      return true;
    },
  );
  assert.equal(fixture.child.stdout.destroyed, true);
});

for (const mode of ["tree failure", "tree timeout", "inherited pipes"]) {
  test(`cancellation is bounded with ${mode}`, { timeout: 1000 }, async () => {
    const listeners = process.listenerCount("SIGTERM");
    const fixture = childFixture();
    let requested = false;
    await assert.rejects(
      run("clang", [], {}, 5, {
        ...fixture,
        platform: "win32",
        cleanupDeadlineMs: 25,
        terminateTree: async (child) => {
          requested = true;
          assert.equal(child.pid, 1234);
          if (mode === "tree failure") throw new Error("taskkill failed");
          if (mode === "tree timeout") return new Promise(() => {});
          child.exitCode = 1; // A descendant still owns the output pipe.
        },
      }),
      /descendant exit is unconfirmed/,
    );
    assert.equal(requested, true);
    assert.equal(fixture.child.stdout.destroyed, true);
    assert.equal(fixture.child.stderr.destroyed, true);
    assert.equal(process.listenerCount("SIGTERM"), listeners);
  });
}

test("normal close after successful tree cleanup preserves the timeout cause", async () => {
  const fixture = childFixture();
  await assert.rejects(
    run("clang", [], {}, 5, {
      ...fixture,
      platform: "win32",
      cleanupDeadlineMs: 100,
      terminateTree: async (child) => {
        child.exitCode = 1;
        child.emit("close", 1);
      },
    }),
    /Tool timed out/,
  );
});

test("Unix cancellation retains TERM before forced pipe cleanup", async () => {
  const fixture = childFixture();
  const signals = [];
  fixture.child.kill = (signal) => {
    signals.push(signal);
    return true;
  };
  await assert.rejects(
    run("cc", [], {}, 5, {
      ...fixture,
      platform: "linux",
      cleanupDeadlineMs: 25,
    }),
    /cleanup deadline/,
  );
  assert.deepEqual(signals, ["SIGTERM", "SIGKILL"]);
});
