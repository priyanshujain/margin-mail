import { execFileSync } from "node:child_process";
import { access, copyFile, mkdir, readFile, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join, resolve } from "node:path";

const signing = process.env.MARGIN_SIGNING_DIR || join(homedir(), ".margin-signing");
const appProfile = join(signing, "studio.margin.mail.provisionprofile");
const extensionProfile = join(signing, "studio.margin.mail.autofill.provisionprofile");
const present = await Promise.all([appProfile, extensionProfile].map(async (path) => {
  try { await access(path); return true; } catch (error) {
    if (error.code === "ENOENT") return false;
    throw error;
  }
}));
if (!present.every(Boolean)) {
  console.error("Email OTP AutoFill is unavailable: both Margin Mail provisioning profiles are required.");
  process.exit(0);
}
const identity = process.env.APPLE_SIGNING_IDENTITY;
if (!identity || identity === "-") throw new Error("Email OTP AutoFill requires a Developer ID signing identity.");
const plist = (path) => JSON.parse(execFileSync("plutil", ["-convert", "json", "-o", "-", path], { encoding: "utf8" }));
const profile = (path, identifier) => {
  const xml = execFileSync("security", ["cms", "-D", "-i", path]);
  const entitlements = JSON.parse(execFileSync("plutil", ["-extract", "Entitlements", "json", "-o", "-", "-"], { input: xml, encoding: "utf8" }));
  const expiry = execFileSync("plutil", ["-extract", "ExpirationDate", "raw", "-o", "-", "-"], { input: xml, encoding: "utf8" }).trim();
  if (entitlements["com.apple.application-identifier"] !== `TQV87WLXK3.${identifier}` ||
      entitlements["com.apple.developer.team-identifier"] !== "TQV87WLXK3" ||
      entitlements["com.apple.developer.authentication-services.autofill-credential-provider"] !== true ||
      !entitlements["com.apple.security.application-groups"]?.some((group) =>
        group === "TQV87WLXK3.*" || group === "TQV87WLXK3.studio.margin.mail") ||
      !(new Date(expiry).getTime() > Date.now())) {
    throw new Error(`Invalid or expired AutoFill profile for ${identifier}.`);
  }
  return entitlements;
};
const appEntitlements = profile(appProfile, "studio.margin.mail");
const extensionEntitlements = profile(extensionProfile, "studio.margin.mail.autofill");
const metadata = JSON.parse(execFileSync("cargo", ["metadata", "--no-deps", "--format-version", "1", "--manifest-path", "src-tauri/Cargo.toml"], { encoding: "utf8" }));
const output = join(metadata.target_directory, ".tauri", "autofill");
const contents = join(output, "MarginMailAutoFill.appex", "Contents");
await mkdir(join(contents, "MacOS"), { recursive: true });
const config = JSON.parse(await readFile("src-tauri/tauri.conf.json", "utf8"));
const info = plist("native/autofill/Info.plist");
info.CFBundleShortVersionString = config.version;
info.CFBundleVersion = config.bundle.macOS.bundleVersion || config.version;
const writePlist = async (path, value) => {
  await writeFile(path, JSON.stringify(value));
  execFileSync("plutil", ["-convert", "xml1", path]);
};
await writePlist(join(contents, "Info.plist"), info);
await copyFile(extensionProfile, join(contents, "embedded.provisionprofile"));
for (const [name, authorized] of [["host", appEntitlements], ["extension", extensionEntitlements]]) {
  const value = plist(name === "host" ? "native/autofill/host.entitlements" : "native/autofill/autofill.entitlements");
  value["com.apple.application-identifier"] = authorized["com.apple.application-identifier"];
  value["com.apple.developer.team-identifier"] = authorized["com.apple.developer.team-identifier"];
  await writePlist(join(output, `${name}.entitlements`), value);
}
const arch = process.arch === "arm64" ? "arm64" : "x86_64";
execFileSync("xcrun", ["swiftc", "-O", "-parse-as-library", "-application-extension", "-module-name", "MarginMailAutoFill",
  "-target", `${arch}-apple-macosx15.0`, "-Xlinker", "-e", "-Xlinker", "_NSExtensionMain",
  "native/autofill/CredentialProvider.swift", "-o", join(contents, "MacOS", "MarginMailAutoFill")], { stdio: ["ignore", "ignore", "inherit"] });
const extension = join(output, "MarginMailAutoFill.appex");
execFileSync("codesign", ["--force", "--sign", identity, "--options", "runtime", "--timestamp", "--entitlements", join(output, "extension.entitlements"), extension], { stdio: ["ignore", "ignore", "inherit"] });
execFileSync("codesign", ["--verify", "--strict", extension], { stdio: ["ignore", "ignore", "inherit"] });
const overlay = join(output, "tauri.conf.json");
await writeFile(overlay, JSON.stringify({ bundle: { macOS: {
  entitlements: join(output, "host.entitlements"),
  files: { "PlugIns/MarginMailAutoFill.appex": extension, "embedded.provisionprofile": resolve(appProfile) },
} } }));
console.log(overlay);
