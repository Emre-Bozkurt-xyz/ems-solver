// Keeps the app version identical in tauri.conf.json, Cargo.toml, Cargo.lock and package.json.
//
//   node client/scripts/version.mjs set 0.2.0     update every file
//   node client/scripts/version.mjs check 0.2.0   fail unless every file says 0.2.0
//   node client/scripts/version.mjs show          print the versions found
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const at = (rel) => fileURLToPath(new URL(`../${rel}`, import.meta.url));

const files = {
  "tauri.conf.json": {
    path: at("src-tauri/tauri.conf.json"),
    get: (text) => JSON.parse(text).version,
    set: (text, v) => text.replace(/("version":\s*")[^"]+(")/, `$1${v}$2`),
  },
  "package.json": {
    path: at("package.json"),
    get: (text) => JSON.parse(text).version,
    set: (text, v) => text.replace(/("version":\s*")[^"]+(")/, `$1${v}$2`),
  },
  "Cargo.toml": {
    path: at("src-tauri/Cargo.toml"),
    get: (text) => text.match(/^\[package\][^[]*?^version = "([^"]+)"/m)?.[1],
    set: (text, v) => text.replace(/^(\[package\][^[]*?^version = ")[^"]+(")/m, `$1${v}$2`),
  },
  "Cargo.lock": {
    path: at("src-tauri/Cargo.lock"),
    get: (text) => text.match(/name = "ems-solver"\nversion = "([^"]+)"/)?.[1],
    set: (text, v) => text.replace(/(name = "ems-solver"\nversion = ")[^"]+(")/, `$1${v}$2`),
  },
};

const [command, version] = process.argv.slice(2);
const read = (f) => readFileSync(f.path, "utf8").replace(/\r\n/g, "\n");

if (command === "show") {
  for (const [name, f] of Object.entries(files)) console.log(`${name.padEnd(16)} ${f.get(read(f))}`);
} else if (command === "set" && /^\d+\.\d+\.\d+$/.test(version ?? "")) {
  for (const [name, f] of Object.entries(files)) {
    writeFileSync(f.path, f.set(read(f), version));
    console.log(`${name.padEnd(16)} ${f.get(read(f))}`);
  }
} else if (command === "check" && version) {
  const wrong = Object.entries(files).filter(([, f]) => f.get(read(f)) !== version);
  for (const [name, f] of wrong) console.error(`${name} says ${f.get(read(f))}, expected ${version}`);
  if (wrong.length) {
    console.error(`Run: node client/scripts/version.mjs set ${version}`);
    process.exit(1);
  }
  console.log(`All files are at ${version}`);
} else {
  console.error("usage: version.mjs show | set X.Y.Z | check X.Y.Z");
  process.exit(2);
}
