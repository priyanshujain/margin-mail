// Tauri's AppImage GTK plugin bundles Ubuntu's Wayland libraries but uses the host's Mesa.
// Recent Mesa needs symbols absent from that older Wayland, so WebKit aborts before rendering.
// Keep the graphics driver's matching Wayland libraries on the host. Run before bundling so
// Tauri signs the final AppImage, and use a project-local tools cache to isolate this adjustment.
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";

if (process.env.TAURI_ENV_PLATFORM === "linux") {
  const revision = "b5eb8d05b4c0ed40107fe2158c5d8527f94568ef";
  const url = `https://raw.githubusercontent.com/tauri-apps/linuxdeploy-plugin-gtk/${revision}/linuxdeploy-plugin-gtk.sh`;
  const response = await fetch(url);
  if (!response.ok) throw new Error(`GTK bundling plugin: HTTP ${response.status}`);
  const source = await response.text();
  const hash = createHash("sha256").update(source).digest("hex");
  if (hash !== "cb379f9b0733e9ad9f8bd78f8c2fa038aef2478523bb7d4c8e64ff6a1ea3501a") {
    throw new Error("GTK bundling plugin does not match the pinned source");
  }

  const metadata = JSON.parse(execFileSync("cargo", [
    "metadata", "--no-deps", "--format-version", "1",
    "--manifest-path", "src-tauri/Cargo.toml",
  ], { encoding: "utf8" }));
  const tools = join(metadata.target_directory, ".tauri");
  await mkdir(tools, { recursive: true });
  await writeFile(join(tools, "linuxdeploy-plugin-gtk.sh"), `${source}
# Use the host Wayland libraries alongside the host graphics driver.
find "$APPDIR/usr/lib" -name 'libwayland-*.so*' -delete
`, { mode: 0o755 });
  console.log("Prepared AppImage GTK plugin with host Wayland libraries");
}
