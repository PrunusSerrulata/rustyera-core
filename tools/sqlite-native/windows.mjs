import { createHash } from "node:crypto";
import { lstat, readdir, readFile, realpath } from "node:fs/promises";
import path from "node:path";

export const WINDOWS_TARGET = "x86_64-pc-windows-msvc";
export function nativeArtifacts(target) {
  return target === WINDOWS_TARGET
    ? { object: "sqlite3.obj", archive: "sqlite3.lib" }
    : { object: "sqlite3.o", archive: "libsqlite3.a" };
}

// Inventory only selected compilation roots, never entire VS or SDK installations.
// All transitive headers count, including names added or removed since the prebuild.
async function treeIdentity(root, identity) {
  const rows = [];
  let bytes = 0;
  async function visit(directory, depth) {
    if (depth > 24) throw new Error("Windows include nesting exceeds limit");
    const entries = (await readdir(directory, { withFileTypes: true })).sort(
      (a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0),
    );
    for (const entry of entries) {
      const file = path.join(directory, entry.name);
      const info = await lstat(file);
      if (info.isSymbolicLink())
        throw new Error(`Windows input symlink: ${file}`);
      if (info.isDirectory()) await visit(file, depth + 1);
      else {
        if (!info.isFile())
          throw new Error(`Non-regular Windows input: ${file}`);
        bytes += info.size;
        if (rows.length >= 32768 || bytes > 1024 * 1024 * 1024)
          throw new Error("Windows include inventory exceeds input budget");
        rows.push([
          path.relative(root, file).split(path.sep).join("/"),
          await identity(file),
        ]);
      }
    }
  }
  await visit(root, 0);
  if (!rows.length) throw new Error(`Empty Windows include directory: ${root}`);
  return {
    files: rows.length,
    bytes,
    digest: createHash("sha256").update(JSON.stringify(rows)).digest("hex"),
  };
}

const equal = (a, b) => JSON.stringify(a) === JSON.stringify(b);

function record(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

// Both configuration loading and manifest verification enforce the same canonical roots.
export async function windowsLayout(config) {
  if (
    !record(config) ||
    config.schemaVersion !== 1 ||
    typeof config.sdkVersion !== "string" ||
    !/^\d+\.\d+\.\d+\.\d+$/.test(config.sdkVersion) ||
    Object.keys(config).sort().join(",") !==
      "clangResource,schemaVersion,sdkRoot,sdkVersion,vcTools"
  )
    throw new Error("Invalid Windows SQLite toolchain configuration");
  for (const key of ["vcTools", "sdkRoot", "clangResource"]) {
    const root = config[key];
    if (
      typeof root !== "string" ||
      !path.isAbsolute(root) ||
      (await realpath(root)) !== root ||
      !(await lstat(root)).isDirectory() ||
      (await lstat(root)).isSymbolicLink()
    )
      throw new Error("Invalid Windows canonical root: " + key);
  }
  const include = [
    path.join(config.clangResource, "include"),
    path.join(config.vcTools, "include"),
    ...["ucrt", "shared", "um"].map((part) =>
      path.join(config.sdkRoot, "Include", config.sdkVersion, part),
    ),
  ];
  const libraryDirectories = [
    path.join(config.vcTools, "lib", "x64"),
    ...["ucrt", "um"].map((part) =>
      path.join(config.sdkRoot, "Lib", config.sdkVersion, part, "x64"),
    ),
  ];
  const libraries = [
    ...["msvcrt.lib", "vcruntime.lib", "oldnames.lib"].map((name) =>
      path.join(libraryDirectories[0], name),
    ),
    path.join(libraryDirectories[1], "ucrt.lib"),
    ...["kernel32.lib", "advapi32.lib"].map((name) =>
      path.join(libraryDirectories[2], name),
    ),
  ];
  // SDK releases vary filename casing (for example kernel32.Lib). Record the
  // actual spelling while still rejecting junctions and redirected child paths.
  for (const paths of [include, libraryDirectories, libraries]) {
    for (let index = 0; index < paths.length; index++) {
      const file = paths[index];
      const canonical = await realpath(file);
      if (
        canonical.toLowerCase() !== file.toLowerCase() ||
        (await lstat(file)).isSymbolicLink()
      )
        throw new Error("Noncanonical Windows toolchain input: " + file);
      paths[index] = canonical;
    }
  }
  return {
    include,
    libraries,
    libraryDirectories,
    flags: [
      "--no-default-config",
      "--target=" + WINDOWS_TARGET,
      "-fms-runtime-lib=dll",
      "-nostdinc",
      "-resource-dir",
      config.clangResource,
      ...include.flatMap((directory) => ["-isystem", directory]),
    ],
  };
}

export async function windowsToolchain(environment, identity) {
  if (environment.CCC_OVERRIDE_OPTIONS)
    throw new Error("CCC_OVERRIDE_OPTIONS must be empty for Windows SQLite");
  const filename = environment.RUSTYERA_SQLITE_WINDOWS_TOOLCHAIN;
  if (typeof filename !== "string" || !path.isAbsolute(filename))
    throw new Error(
      "Windows SQLite requires an absolute RUSTYERA_SQLITE_WINDOWS_TOOLCHAIN JSON path",
    );
  const config = JSON.parse(await readFile(filename, "utf8"));
  const layout = await windowsLayout(config);
  const headers = [];
  for (const directory of layout.include)
    headers.push({
      path: directory,
      identity: await treeIdentity(directory, identity),
    });
  const linkInputs = [];
  for (const file of layout.libraries)
    linkInputs.push({ path: file, identity: await identity(file) });
  return {
    config,
    configurationIdentity: await identity(filename),
    headers,
    linkInputs,
    libraryDirectories: layout.libraryDirectories,
    flags: layout.flags,
  };
}

export async function verifyWindowsInputs(windows, identity) {
  if (
    !record(windows) ||
    !Array.isArray(windows.headers) ||
    windows.headers.length !== 5 ||
    !Array.isArray(windows.linkInputs) ||
    windows.linkInputs.length !== 6
  )
    throw new Error("Missing Windows toolchain identity");
  const layout = await windowsLayout(windows.config);
  if (
    !equal(windows.libraryDirectories, layout.libraryDirectories) ||
    !equal(windows.flags, layout.flags) ||
    !equal(
      windows.headers.map((row) => row?.path),
      layout.include,
    ) ||
    !equal(
      windows.linkInputs.map((row) => row?.path),
      layout.libraries,
    )
  )
    throw new Error(
      "Windows manifest paths or CRT flags differ from the selected toolchain",
    );
  for (const row of windows.headers)
    if (!equal(await treeIdentity(row.path, identity), row.identity))
      throw new Error("Windows SDK/header identity changed: " + row.path);
  for (const row of windows.linkInputs)
    if (!equal(await identity(row.path), row.identity))
      throw new Error("Windows CRT/library identity changed: " + row.path);
  return layout;
}

// Full ordered equality also excludes a second target/CRT/include option later in argv.
export function verifyWindowsCompileFlags(actual, layout, defines, tuning) {
  const expected = [
    "-O2",
    ...layout.flags,
    "-fno-strict-aliasing",
    ...defines.map((value) => "-D" + value),
    ...tuning,
  ];
  if (!equal(actual, expected))
    throw new Error(
      "Windows SQLite compilation flags differ from the selected toolchain",
    );
}
