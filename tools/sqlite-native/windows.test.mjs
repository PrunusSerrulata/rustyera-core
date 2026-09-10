import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  mkdtemp,
  mkdir,
  readFile,
  realpath,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import {
  nativeArtifacts,
  WINDOWS_TARGET,
  windowsLayout,
  windowsToolchain,
  verifyWindowsInputs,
  verifyWindowsCompileFlags,
} from "./windows.mjs";

async function identity(file) {
  const bytes = await readFile(file);
  return {
    bytes: bytes.length,
    digest: createHash("sha256").update(bytes).digest("hex"),
  };
}

test("Windows toolchain rejects inherited Clang argument overrides", async () => {
  await assert.rejects(
    windowsToolchain(
      { CCC_OVERRIDE_OPTIONS: "^-DSQLITE_THREADSAFE=0" },
      identity,
    ),
    /CCC_OVERRIDE_OPTIONS/,
  );
});

async function fixture(t) {
  const parent = await realpath(os.tmpdir());
  const root = await realpath(
    await mkdtemp(path.join(parent, "rustyera-sql-windows-")),
  );
  t.after(async () => {
    const relative = path.relative(parent, root);
    assert.ok(
      relative.startsWith("rustyera-sql-windows-") &&
        !relative.includes(path.sep),
    );
    await rm(root, { recursive: true, force: true });
  });
  const config = {
    schemaVersion: 1,
    sdkVersion: "10.0.12345.0",
    vcTools: path.join(root, "vc"),
    sdkRoot: path.join(root, "sdk"),
    clangResource: path.join(root, "clang"),
  };
  const includes = [
    path.join(config.clangResource, "include"),
    path.join(config.vcTools, "include"),
    ...["ucrt", "shared", "um"].map((part) =>
      path.join(config.sdkRoot, "Include", config.sdkVersion, part),
    ),
  ];
  const libraries = [
    ...["msvcrt.lib", "vcruntime.lib", "oldnames.lib"].map((name) =>
      path.join(config.vcTools, "lib", "x64", name),
    ),
    path.join(
      config.sdkRoot,
      "Lib",
      config.sdkVersion,
      "ucrt",
      "x64",
      "ucrt.lib",
    ),
    ...["kernel32.lib", "advapi32.lib"].map((name) =>
      path.join(config.sdkRoot, "Lib", config.sdkVersion, "um", "x64", name),
    ),
  ];
  for (const directory of includes) {
    await mkdir(directory, { recursive: true });
    await writeFile(
      path.join(directory, "fixture.h"),
      "/* bounded fixture */\n",
    );
  }
  for (const file of libraries) {
    await mkdir(path.dirname(file), { recursive: true });
    await writeFile(file, "fixture import library\n");
  }
  for (const key of ["vcTools", "sdkRoot", "clangResource"])
    config[key] = await realpath(config[key]);
  const filename = path.join(root, "config.json");
  await writeFile(filename, JSON.stringify(config));
  const load = () =>
    windowsToolchain({ RUSTYERA_SQLITE_WINDOWS_TOOLCHAIN: filename }, identity);
  return {
    root,
    config,
    filename,
    includes,
    libraries,
    load,
    windows: await load(),
  };
}

test(
  "records the actual Windows SDK import-library casing",
  { skip: process.platform !== "win32" },
  async (t) => {
    const f = await fixture(t);
    const file = path.join(
      f.config.sdkRoot,
      "Lib",
      f.config.sdkVersion,
      "um",
      "x64",
      "kernel32.lib",
    );
    const renamed = path.join(path.dirname(file), "kernel32.Lib");
    await rename(file, renamed);
    const loaded = await windowsToolchain(
      { RUSTYERA_SQLITE_WINDOWS_TOOLCHAIN: f.filename },
      identity,
    );
    assert.equal(loaded.linkInputs[4].path, await realpath(renamed));
    await verifyWindowsInputs(loaded, identity);
  },
);

test("Windows identity verifies only the bounded fixture and uses platform artifacts", async (t) => {
  const f = await fixture(t);
  const layout = await verifyWindowsInputs(f.windows, identity);
  assert.equal(layout.include.length, 5);
  assert.equal(layout.libraries.length, 6);
  assert.equal(layout.flags[0], "--no-default-config");
  assert.deepEqual(nativeArtifacts(WINDOWS_TARGET), {
    object: "sqlite3.obj",
    archive: "sqlite3.lib",
  });
  assert.deepEqual(nativeArtifacts("x86_64-unknown-linux-gnu"), {
    object: "sqlite3.o",
    archive: "libsqlite3.a",
  });
});

for (const change of ["modify", "add", "delete"]) {
  test(`header ${change} invalidates a retained identity`, async (t) => {
    const f = await fixture(t);
    const header = path.join(f.includes[2], "fixture.h");
    if (change === "modify") await writeFile(header, "changed\n");
    if (change === "add")
      await writeFile(path.join(f.includes[2], "new.h"), "new\n");
    if (change === "delete") await rm(header);
    await assert.rejects(
      verifyWindowsInputs(f.windows, identity),
      /identity changed|Empty Windows/,
    );
  });
}

test("CRT library byte changes invalidate a retained identity", async (t) => {
  const f = await fixture(t);
  await writeFile(f.libraries[0], "different import library\n");
  await assert.rejects(
    verifyWindowsInputs(f.windows, identity),
    /library identity changed/,
  );
});

test("manifest paths, include rows, link paths and CRT flags cannot be substituted", async (t) => {
  const f = await fixture(t);
  for (const mutate of [
    (w) => {
      w.headers[0].path = w.headers[1].path;
    },
    (w) => {
      w.linkInputs[0].path = w.linkInputs[1].path;
    },
    (w) => {
      w.libraryDirectories.reverse();
    },
    (w) => {
      w.flags.push("-fms-runtime-lib=static");
    },
    (w) => {
      w.headers = {};
    },
    (w) => {
      w.config = null;
    },
    (w) => {
      w.config.extra = true;
    },
  ]) {
    const changed = structuredClone(f.windows);
    mutate(changed);
    await assert.rejects(verifyWindowsInputs(changed, identity));
  }
});

test("full compiler argv forbids a second CRT, target or include override", async (t) => {
  const f = await fixture(t);
  const layout = await windowsLayout(f.config);
  const defines = ["SQLITE_THREADSAFE=1"];
  const tuning = ["-mtune=generic"];
  const valid = [
    "-O2",
    ...layout.flags,
    "-fno-strict-aliasing",
    "-DSQLITE_THREADSAFE=1",
    ...tuning,
  ];
  assert.doesNotThrow(() =>
    verifyWindowsCompileFlags(valid, layout, defines, tuning),
  );
  for (const override of [
    "-fms-runtime-lib=static",
    "--target=x86_64-w64-mingw32",
    "-isystem",
    "-I.",
  ])
    assert.throws(
      () =>
        verifyWindowsCompileFlags(
          [...valid, override],
          layout,
          defines,
          tuning,
        ),
      /compilation flags/,
    );
  assert.throws(
    () => verifyWindowsCompileFlags(valid.slice(1), layout, defines, tuning),
    /compilation flags/,
  );
});

test("loader and verifier reject malformed configurations and noncanonical roots", async (t) => {
  const f = await fixture(t);
  const invalid = [
    null,
    [],
    1,
    "config",
    {},
    { ...f.config, extra: true },
    { ...f.config, sdkVersion: "../other" },
    { ...f.config, sdkVersion: 10 },
    { ...f.config, vcTools: "relative" },
    { ...f.config, sdkRoot: null },
    { ...f.config, vcTools: f.config.vcTools + path.sep + "." },
  ];
  for (const config of invalid) {
    await writeFile(f.filename, JSON.stringify(config));
    await assert.rejects(f.load());
    await assert.rejects(
      verifyWindowsInputs({ ...f.windows, config }, identity),
    );
  }
  await assert.rejects(
    windowsToolchain(
      { RUSTYERA_SQLITE_WINDOWS_TOOLCHAIN: "relative.json" },
      identity,
    ),
  );
});
