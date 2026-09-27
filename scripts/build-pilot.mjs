// Build an isolated local macOS pilot with the operator binaries it needs.
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
if (process.platform !== "darwin") throw new Error("This pilot build is verified on macOS only.");
const run = (command, args, extra = {}) => execFileSync(command, args, { cwd: root, stdio: "inherit", ...extra });
const capture = (command, args) => execFileSync(command, args, { cwd: root, encoding: "utf8" }).trim();
const manifest = join(root, "src-tauri/Cargo.toml");
const bins = ["contractorcrm-mcp"];
const host = capture("rustc", ["-vV"]).split("\n").find(line => line.startsWith("host: ")).slice(6);
const targetDir = JSON.parse(capture("cargo", ["metadata", "--manifest-path", manifest, "--no-deps", "--format-version", "1"])).target_directory;
mkdirSync(join(root, "src-tauri/binaries"), { recursive: true });
for (const bin of bins) {
  run("cargo", ["build", "--release", "--manifest-path", manifest, "--bin", bin], {
    env: { ...process.env, TAURI_CONFIG: JSON.stringify({ bundle: { externalBin: [] } }) },
  });
  copyFileSync(join(targetDir, "release", bin), join(root, "src-tauri/binaries", bin + "-" + host));
}
run("npm", ["run", "tauri", "build", "--", "--config", "src-tauri/tauri.pilot.conf.json", "--bundles", "app"]);
const config = JSON.parse(readFileSync(join(root, "src-tauri/tauri.pilot.conf.json"), "utf8"));
const bundledApp = join(targetDir, "release/bundle/macos", config.productName + ".app");
// File-provider metadata in a synced workspace invalidates code signing.
// Copy only bundle contents into a local staging directory before signing.
const stage = mkdtempSync(join(tmpdir(), "contractorcrm-pilot-"));
const app = join(stage, config.productName + ".app");
run("ditto", ["--norsrc", "--noextattr", bundledApp, app]);
const info = JSON.parse(capture("plutil", ["-convert", "json", "-o", "-", join(app, "Contents/Info.plist")]));
if (info.CFBundleIdentifier !== config.identifier) throw new Error("Pilot bundle identifier mismatch");
if (info.CFBundleExecutable !== "contractorcrm") throw new Error("Pilot desktop executable mismatch");
for (const bin of bins) readFileSync(join(app, "Contents/MacOS", bin));
// Local source builds need a consistent ad-hoc signature after bundling helpers.
run("codesign", ["--force", "--deep", "--sign", "-", app]);
run("codesign", ["--verify", "--deep", "--strict", app]);
const sourceSha = capture("git", ["rev-parse", "HEAD"]);
const dirty = capture("git", ["status", "--porcelain", "--untracked-files=normal"]).length > 0;
const out = join(root, "pilot-artifacts");
mkdirSync(out, { recursive: true });
const zip = join(out, config.productName.replaceAll(" ", "-") + "-" + sourceSha.slice(0, 12) + (dirty ? "-dirty" : "") + ".zip");
run("ditto", ["-c", "-k", "--sequesterRsrc", "--keepParent", app, zip]);
const evidence = {
  sourceSha, dirty, version: info.CFBundleShortVersionString,
  identifier: info.CFBundleIdentifier, executable: info.CFBundleExecutable,
  builtAt: new Date().toISOString(), macOS: capture("sw_vers", ["-productVersion"]),
  architecture: host, app, archive: zip,
  sha256: createHash("sha256").update(readFileSync(zip)).digest("hex"),
  distribution: "Ad-hoc signed local developer pilot. No Developer ID or notarization claim.",
};
writeFileSync(zip + ".json", JSON.stringify(evidence, null, 2) + "\n");
console.log(JSON.stringify(evidence, null, 2));
