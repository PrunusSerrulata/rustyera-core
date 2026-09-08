import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { constants, createReadStream } from "node:fs";
import {
  access,
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  realpath,
  rename,
  rm,
  rmdir,
  statfs,
  writeFile,
} from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.dirname(fileURLToPath(import.meta.url));
const source = path.join(root, "source");
export const SQLITE_SOURCE_ID =
  "2026-07-24 19:02:57 bf7c7f30031888f4e796e429ab3978879485813aaca6f641c7b33e4e09459bcc";
// Match the shipped official WASM's SQL defaults/extensions. Native connections need
// mutex support; temp storage is deliberately memory-only rather than WASM's TEMP_STORE=2.
export const SQLITE_DEFINES = Object.freeze([
  "SQLITE_THREADSAFE=1",
  "SQLITE_TEMP_STORE=3",
  "SQLITE_DQS=0",
  "SQLITE_DEFAULT_CACHE_SIZE=-16384",
  "SQLITE_DEFAULT_RECURSIVE_TRIGGERS=1",
  "SQLITE_DEFAULT_AUTOVACUUM=1",
  "SQLITE_DEFAULT_SECTOR_SIZE=4096",
  "SQLITE_MAX_MMAP_SIZE=0",
  "SQLITE_MAX_WORKER_THREADS=0",
  "SQLITE_ENABLE_API_ARMOR",
  "SQLITE_ENABLE_COLUMN_METADATA",
  "SQLITE_ENABLE_MATH_FUNCTIONS",
  "SQLITE_ENABLE_FTS5",
  "SQLITE_ENABLE_RTREE",
  "SQLITE_ENABLE_SESSION",
  "SQLITE_ENABLE_PREUPDATE_HOOK",
  "SQLITE_ENABLE_PERCENTILE",
  "SQLITE_ENABLE_OFFSET_SQL_FUNC",
  "SQLITE_ENABLE_DBSTAT_VTAB",
  "SQLITE_ENABLE_DBPAGE_VTAB",
  "SQLITE_ENABLE_BYTECODE_VTAB",
  "SQLITE_ENABLE_STMTVTAB",
  "SQLITE_ENABLE_UNKNOWN_SQL_FUNCTION",
  "SQLITE_USE_URI=1",
  "SQLITE_OMIT_LOAD_EXTENSION",
  "SQLITE_OMIT_SHARED_CACHE",
  "SQLITE_OMIT_DEPRECATED",
  "SQLITE_OMIT_UTF16",
]);

async function identity(filename, algorithm = "sha256") {
  const before = await lstat(filename);
  if (!before.isFile()) throw new Error(`Not a regular file: ${filename}`);
  const hash = createHash(algorithm);
  let bytes = 0;
  for await (const chunk of createReadStream(filename)) {
    hash.update(chunk);
    bytes += chunk.length;
  }
  const after = await lstat(filename);
  if (
    before.size !== bytes ||
    after.size !== bytes ||
    before.mtimeMs !== after.mtimeMs
  )
    throw new Error(`File changed while hashing: ${filename}`);
  return { bytes, digest: hash.digest("hex") };
}

async function executable(name, environment) {
  if (!name || name.includes("\0")) throw new Error("Invalid tool executable");
  const candidates = name.includes(path.sep)
    ? [path.resolve(name)]
    : (environment.PATH ?? "")
        .split(path.delimiter)
        .filter(Boolean)
        .map((entry) => path.resolve(entry, name));
  for (const candidate of candidates) {
    try {
      await access(candidate, constants.X_OK);
      // Preserve argv[0]: rustup and compiler driver symlinks select their tool by name.
      return candidate;
    } catch (error) {
      if (!["ENOENT", "EACCES", "ENOTDIR"].includes(error.code)) throw error;
    }
  }
  throw new Error(`Required tool unavailable (no install fallback): ${name}`);
}

async function run(command, args, environment, timeout = 30_000) {
  return await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      env: environment,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "",
      stderr = "",
      failure;
    let killTimer;
    const stop = (error) => {
      if (failure) return;
      failure = error;
      child.kill("SIGTERM");
      killTimer = setTimeout(() => child.kill("SIGKILL"), 1000);
    };
    const signals = ["SIGINT", "SIGTERM", "SIGHUP"];
    const onSignal = () => stop(new Error("SQLite build interrupted"));
    for (const signal of signals) process.on(signal, onSignal);
    const timer = setTimeout(
      () => stop(new Error(`Tool timed out: ${command}`)),
      timeout,
    );
    for (const [stream, name] of [
      [child.stdout, "stdout"],
      [child.stderr, "stderr"],
    ]) {
      stream.setEncoding("utf8");
      stream.on("data", (chunk) => {
        if (failure) return;
        if (name === "stdout") stdout += chunk;
        else stderr += chunk;
        if (stdout.length + stderr.length > 1024 * 1024)
          stop(new Error("Tool output exceeds 1 MiB"));
      });
    }
    child.once("error", (error) => {
      failure = error;
    });
    child.once("close", (code, signal) => {
      clearTimeout(timer);
      clearTimeout(killTimer);
      for (const item of signals) process.removeListener(item, onSignal);
      if (failure) reject(failure);
      else if (code !== 0)
        reject(new Error(`${command} exited ${code ?? signal}: ${stderr}`));
      else resolve(stdout.trim());
    });
  });
}

function jsonFlags(value) {
  if (!value) return [];
  const flags = JSON.parse(value);
  if (
    !Array.isArray(flags) ||
    flags.some((flag) => typeof flag !== "string" || flag.includes("\0"))
  )
    throw new Error(
      "RUSTYERA_SQLITE_CFLAGS_JSON must be a JSON array of arguments",
    );
  // Contract-defining macros and source/output selection are never overridden by ambient flags.
  if (
    flags.some(
      (flag) =>
        !/^-m(?:arch|cpu|tune)=[-a-zA-Z0-9_+.]+$/.test(flag) ||
        flag.endsWith("=native"),
    )
  )
    throw new Error(
      "Only explicit -march/-mcpu/-mtune tuning flags are supported",
    );
  return flags;
}

async function inputsFor({ target, output, environment }) {
  const rustc = await executable(environment.RUSTC || "rustc", environment);
  const rustVersion = await run(rustc, ["-vV"], environment);
  const host = /^host: (.+)$/m.exec(rustVersion)?.[1];
  target ||= environment.CARGO_BUILD_TARGET || host;
  if (!host || target !== host)
    throw new Error(
      `Cross compilation is not supported: host=${host}, target=${target}`,
    );
  const platformTarget =
    process.platform === "darwin"
      ? { arm64: "aarch64-apple-darwin", x64: "x86_64-apple-darwin" }[
          process.arch
        ]
      : process.platform === "linux"
        ? {
            arm64: "aarch64-unknown-linux-gnu",
            x64: "x86_64-unknown-linux-gnu",
          }[process.arch]
        : undefined;
  if (target !== platformTarget)
    throw new Error(
      `Unsupported native SQLite platform/ABI: ${process.platform}/${process.arch}/${target}`,
    );
  output = path.resolve(
    output ||
      environment.RUSTYERA_SQLITE_NATIVE_OUTPUT ||
      path.join(root, "../../../target/sqlite-native", target),
  );
  const relative = path.relative(root, output);
  if (
    !relative ||
    (!relative.startsWith(`..${path.sep}`) &&
      relative !== ".." &&
      !path.isAbsolute(relative))
  )
    throw new Error(
      "Build output must be outside the SQLite source/tool directory",
    );
  for (const name of [
    "CFLAGS",
    "CPPFLAGS",
    "LDFLAGS",
    "CPATH",
    "C_INCLUDE_PATH",
    "LIBRARY_PATH",
  ])
    if (environment[name])
      throw new Error(
        `Unsupported ambient ${name}; use the explicit SQLite build configuration`,
      );
  const compiler = await executable(
    environment.RUSTYERA_SQLITE_CC ||
      environment.CC ||
      (process.platform === "darwin" ? "clang" : "cc"),
    environment,
  );
  const archiver = await executable(
    environment.RUSTYERA_SQLITE_AR || environment.AR || "ar",
    environment,
  );
  const compilerVersion = await run(compiler, ["--version"], environment);
  const compilerTarget = await run(compiler, ["-dumpmachine"], environment);
  const expectedArch =
    process.arch === "arm64" ? /^(arm64|aarch64)-/ : /^x86_64-/;
  if (
    !expectedArch.test(compilerTarget) ||
    !(process.platform === "darwin" ? /apple-darwin/ : /linux-gnu/).test(
      compilerTarget,
    )
  )
    throw new Error(
      `Compiler target does not match ${target}: ${compilerTarget}`,
    );
  let sdk;
  if (process.platform === "darwin") {
    const xcrun = await executable("xcrun", environment);
    const sdkPath =
      environment.SDKROOT ||
      (await run(xcrun, ["--sdk", "macosx", "--show-sdk-path"], environment));
    sdk = {
      path: await realpath(sdkPath),
      settings: await identity(path.join(sdkPath, "SDKSettings.json")),
    };
  }
  const deployment =
    process.platform === "darwin"
      ? environment.MACOSX_DEPLOYMENT_TARGET || "11.0"
      : null;
  if (deployment && !/^\d+\.\d+(?:\.\d+)?$/.test(deployment))
    throw new Error("Invalid macOS deployment target");
  const manifestPath = path.join(source, "manifest.json");
  const sourceManifest = JSON.parse(await readFile(manifestPath, "utf8"));
  if (
    sourceManifest.schemaVersion !== 1 ||
    sourceManifest.sqliteVersion !== "3.53.4" ||
    sourceManifest.sqliteVersionNumber !== 3053004 ||
    sourceManifest.sourceId !== SQLITE_SOURCE_ID
  )
    throw new Error("Unexpected SQLite source manifest/version");
  const files = {};
  for (const name of ["sqlite3.c", "sqlite3.h"]) {
    files[name] = await identity(path.join(source, name), "sha3-256");
    if (files[name].digest !== sourceManifest.files[name])
      throw new Error(`SQLite SHA3-256 mismatch: ${name}`);
  }
  const header = await readFile(path.join(source, "sqlite3.h"), "utf8");
  if (
    !header.includes(
      `#define SQLITE_VERSION        "${sourceManifest.sqliteVersion}"`,
    ) ||
    !header.includes(
      `#define SQLITE_SOURCE_ID      "${sourceManifest.sourceId}"`,
    )
  )
    throw new Error("SQLite header version/source ID differs from manifest");
  const flags = [
    "-O2",
    "-fPIC",
    "-fno-strict-aliasing",
    ...SQLITE_DEFINES.map((value) => `-D${value}`),
    ...(sdk
      ? [
          "-isysroot",
          sdk.path,
          `-mmacosx-version-min=${deployment}`,
          "-arch",
          process.arch === "arm64" ? "arm64" : "x86_64",
        ]
      : []),
    ...jsonFlags(environment.RUSTYERA_SQLITE_CFLAGS_JSON),
  ];
  const inputs = {
    schemaVersion: 1,
    target,
    output,
    sourceManifest,
    files,
    builder: await identity(fileURLToPath(import.meta.url)),
    sourceManifestIdentity: await identity(manifestPath),
    compiler: {
      path: compiler,
      realPath: await realpath(compiler),
      identity: await identity(await realpath(compiler)),
      version: compilerVersion,
      target: compilerTarget,
    },
    archiver: {
      path: archiver,
      realPath: await realpath(archiver),
      identity: await identity(await realpath(archiver)),
      flags: ["rcs"],
      environment: { ZERO_AR_DATE: "1" },
    },
    rustc: {
      path: rustc,
      realPath: await realpath(rustc),
      version: rustVersion,
    },
    sdk: sdk ?? null,
    deployment,
    flags,
    systemLibraries: process.platform === "linux" ? ["m", "pthread"] : [],
    environment: Object.fromEntries(
      [
        "PATH",
        "SDKROOT",
        "DEVELOPER_DIR",
        "MACOSX_DEPLOYMENT_TARGET",
        "CC",
        "AR",
        "RUSTYERA_SQLITE_CC",
        "RUSTYERA_SQLITE_AR",
        "RUSTYERA_SQLITE_CFLAGS_JSON",
      ].map((name) => [name, environment[name] ?? null]),
    ),
  };
  return { inputs, output, compiler, archiver, flags };
}

function linkEnvironment(output) {
  return {
    SQLITE3_LIB_DIR: output,
    SQLITE3_INCLUDE_DIR: output,
    SQLITE3_STATIC: "1",
    SQLITE3_NO_PKG_CONFIG: "1",
  };
}

// Build.rs uses this read-only path: no compiler probes, compilation or cache repair.
// Hash the bytes again so replacing an archive in the same directory changes rustc's inputs.
export async function verifyLinkInputs(output, target) {
  output = await realpath(output);
  try {
    await lstat(`${output}.build-lock`);
    throw new Error(`SQLite link input is locked: ${output}.build-lock`);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  const manifestBytes = await readFile(path.join(output, "manifest.json"));
  const manifest = JSON.parse(manifestBytes);
  const inputs = manifest.inputs;
  if (
    manifest.schemaVersion !== 1 ||
    inputs?.target !== target ||
    inputs.sourceManifest?.sqliteVersion !== "3.53.4" ||
    inputs.sourceManifest?.sqliteVersionNumber !== 3053004 ||
    inputs.sourceManifest?.sourceId !== SQLITE_SOURCE_ID ||
    JSON.stringify(inputs.systemLibraries) !==
      JSON.stringify(target.endsWith("-linux-gnu") ? ["m", "pthread"] : []) ||
    JSON.stringify(inputs.flags?.filter((flag) => flag.startsWith("-D"))) !==
      JSON.stringify(SQLITE_DEFINES.map((value) => `-D${value}`))
  )
    throw new Error(
      "Native SQLite link manifest does not match the fixed engine contract",
    );
  const hash = createHash("sha256").update(manifestBytes);
  for (const name of ["libsqlite3.a", "sqlite3.h"]) {
    const actual = await identity(path.join(output, name));
    if (
      !actual.bytes ||
      JSON.stringify(actual) !== JSON.stringify(manifest.artifacts?.[name])
    )
      throw new Error(`Native SQLite link artifact mismatch: ${name}`);
    hash.update(actual.digest);
  }
  const header = await readFile(path.join(output, "sqlite3.h"), "utf8");
  if (
    !header.includes(`#define SQLITE_VERSION        "3.53.4"`) ||
    !header.includes(`#define SQLITE_SOURCE_ID      "${SQLITE_SOURCE_ID}"`)
  )
    throw new Error("Native SQLite link header identity mismatch");
  return hash.digest("hex");
}

async function cached(configuration) {
  const { output, inputs } = configuration;
  try {
    await lstat(`${output}.build-lock`);
    throw new Error(`SQLite build output is locked: ${output}.build-lock`);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  try {
    const directory = await lstat(output);
    if (!directory.isDirectory() || directory.isSymbolicLink())
      throw new Error(`SQLite output is not a real directory: ${output}`);
    const manifest = JSON.parse(
      await readFile(path.join(output, "manifest.json"), "utf8"),
    );
    if (JSON.stringify(manifest.inputs) !== JSON.stringify(inputs))
      return undefined;
    for (const name of ["libsqlite3.a", "sqlite3.h"])
      if (
        JSON.stringify(await identity(path.join(output, name))) !==
        JSON.stringify(manifest.artifacts[name])
      )
        return undefined;
    if (
      JSON.stringify(await identity(path.join(source, "sqlite3.h"))) !==
      JSON.stringify(manifest.artifacts["sqlite3.h"])
    )
      return undefined;
    const env = JSON.parse(
      await readFile(path.join(output, "env.json"), "utf8"),
    );
    if (JSON.stringify(env) !== JSON.stringify(linkEnvironment(output)))
      return undefined;
    if (
      manifest.schemaVersion !== 1 ||
      manifest.artifacts["libsqlite3.a"].bytes === 0
    )
      return undefined;
    return {
      ...manifest,
      environment: env,
      manifestPath: path.join(output, "manifest.json"),
    };
  } catch (error) {
    if (
      error.code === "ENOENT" ||
      error instanceof SyntaxError ||
      error instanceof TypeError
    )
      return undefined;
    throw error;
  }
}

export async function prepareNativeSqlite({
  target,
  output,
  environment = process.env,
  cacheOnly = false,
} = {}) {
  const configuration = await inputsFor({ target, output, environment });
  const hit = await cached(configuration);
  if (hit) return hit;
  if (cacheOnly)
    throw new Error(
      `SQLite static cache missing or changed: ${configuration.output}; run the explicit SQLite prebuild first (no compilation started)`,
    );
  const destination = configuration.output;
  await mkdir(path.dirname(destination), { recursive: true });
  const disk = await statfs(path.dirname(destination));
  if (disk.bavail * disk.bsize < 10 * 1024 ** 3)
    throw new Error(
      "SQLite build refused: less than 10 GiB disk space available",
    );
  const lock = `${destination}.build-lock`;
  await mkdir(lock); // Never race another writer or silently remove a stale owner's lock.
  let staging;
  try {
    try {
      const directory = await lstat(destination);
      if (!directory.isDirectory() || directory.isSymbolicLink())
        throw new Error("SQLite output must be a real directory");
      const entries = await readdir(destination);
      if (entries.length) {
        let owned;
        try {
          owned = JSON.parse(
            await readFile(path.join(destination, "manifest.json"), "utf8"),
          );
        } catch {
          throw new Error(
            "Refusing to overwrite a nonempty unowned SQLite output directory",
          );
        }
        if (
          owned.schemaVersion !== 1 ||
          owned.inputs?.output !== destination ||
          owned.inputs?.sourceManifest?.sqliteVersion !== "3.53.4"
        )
          throw new Error(
            "Refusing to overwrite an unowned SQLite output directory",
          );
      }
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
    staging = await mkdtemp(
      path.join(path.dirname(destination), ".sqlite-native-"),
    );
    await copyFile(
      path.join(source, "sqlite3.c"),
      path.join(staging, "sqlite3.c"),
    );
    await copyFile(
      path.join(source, "sqlite3.h"),
      path.join(staging, "sqlite3.h"),
    );
    for (const name of ["sqlite3.c", "sqlite3.h"])
      if (
        JSON.stringify(await identity(path.join(staging, name), "sha3-256")) !==
        JSON.stringify(configuration.inputs.files[name])
      )
        throw new Error(`Source changed while copying: ${name}`);
    await run(
      configuration.compiler,
      [
        ...configuration.flags,
        "-c",
        path.join(staging, "sqlite3.c"),
        "-o",
        path.join(staging, "sqlite3.o"),
      ],
      environment,
      300_000,
    );
    await run(
      configuration.archiver,
      [
        "rcs",
        path.join(staging, "libsqlite3.a"),
        path.join(staging, "sqlite3.o"),
      ],
      { ...environment, ZERO_AR_DATE: "1" },
    );
    const manifest = {
      schemaVersion: 1,
      inputs: configuration.inputs,
      artifacts: {
        "libsqlite3.a": await identity(path.join(staging, "libsqlite3.a")),
        "sqlite3.h": await identity(path.join(staging, "sqlite3.h")),
      },
    };
    const current = await inputsFor({
      target: configuration.inputs.target,
      output: destination,
      environment,
    });
    if (JSON.stringify(current.inputs) !== JSON.stringify(configuration.inputs))
      throw new Error("SQLite build inputs changed during compilation");
    await mkdir(destination, { recursive: true });
    for (const name of ["libsqlite3.a", "sqlite3.h"])
      await rename(path.join(staging, name), path.join(destination, name));
    await writeFile(
      path.join(staging, "env.json"),
      JSON.stringify(linkEnvironment(destination)) + "\n",
    );
    await rename(
      path.join(staging, "env.json"),
      path.join(destination, "env.json"),
    );
    await writeFile(
      path.join(staging, "manifest.json"),
      JSON.stringify(manifest) + "\n",
    );
    await rename(
      path.join(staging, "manifest.json"),
      path.join(destination, "manifest.json"),
    );
    return {
      ...manifest,
      environment: linkEnvironment(destination),
      manifestPath: path.join(destination, "manifest.json"),
    };
  } finally {
    if (staging) await rm(staging, { recursive: true, force: true });
    await rmdir(lock);
  }
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  try {
    if (process.argv[2] === "--verify-link-inputs") {
      if (process.argv.length !== 5)
        throw new Error("Expected link directory and target");
      process.stdout.write(
        (await verifyLinkInputs(process.argv[3], process.argv[4])) + "\n",
      );
    } else {
      const options = {};
      const args = process.argv.slice(2);
      for (let i = 0; i < args.length; i++) {
        if (args[i] === "--cache-only") options.cacheOnly = true;
        else if (
          ["--target", "--output"].includes(args[i]) &&
          args[i + 1] &&
          !args[i + 1].startsWith("--")
        )
          options[args[i++].slice(2)] = args[i];
        else
          throw new Error(
            "Usage: node build.mjs --target <Rust triple> --output <directory> [--cache-only]",
          );
      }
      if (!options.target || !options.output)
        throw new Error("Both --target and --output are required");
      process.stdout.write(
        JSON.stringify(await prepareNativeSqlite(options)) + "\n",
      );
    }
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
