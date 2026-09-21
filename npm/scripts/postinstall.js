// Downloads the platform binary for this package version from GitHub releases
// into vendor/. No runtime dependencies; node >= 18 for global fetch.
"use strict";

const fs = require("node:fs");
const path = require("node:path");
const os = require("node:os");

const REPO = "nolgiainc/nolgia-cli";
const VERSION = require("../package.json").version;

function assetName(platform = process.platform, arch = process.arch) {
  if (platform === "darwin") {
    // Universal binary covers x64 and arm64.
    return "nolgia-x86_64-apple-darwin";
  }
  // Statically linked musl builds since v0.2.30 (NOL-1070): they start on
  // any Linux, including Debian 12, Ubuntu 22.04 and slim container images.
  if (platform === "linux" && arch === "x64") {
    return "nolgia-x86_64-unknown-linux-musl";
  }
  if (platform === "linux" && arch === "arm64") {
    return "nolgia-aarch64-unknown-linux-musl";
  }
  if (platform === "win32" && arch === "x64") {
    return "nolgia-x86_64-pc-windows-msvc.exe";
  }
  if (platform === "win32" && arch === "arm64") {
    return "nolgia-aarch64-pc-windows-msvc.exe";
  }
  return null;
}

function binaryPath() {
  const ext = process.platform === "win32" ? ".exe" : "";
  return path.join(__dirname, "..", "vendor", `nolgia${ext}`);
}

function writeInstallMetadata() {
  try {
    const configHome =
      process.env.XDG_CONFIG_HOME && process.env.XDG_CONFIG_HOME.length > 0
        ? process.env.XDG_CONFIG_HOME
        : path.join(os.homedir(), ".config");
    const dir = path.join(configHome, "nolgia");
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(
      path.join(dir, "install-metadata.json"),
      JSON.stringify({
        method: "npm",
        tag: `v${VERSION}`,
        installed_at: new Date().toISOString(),
      }) + "\n"
    );
  } catch {
    // Metadata is best-effort; the update hint falls back to path inference.
  }
}

async function main() {
  const asset = assetName();
  if (!asset) {
    console.error(
      `@nolgia/cli has no prebuilt binary for ${process.platform}/${process.arch}; ` +
        "install with: cargo install nolgia-cli"
    );
    process.exit(1);
  }

  const url = `https://github.com/${REPO}/releases/download/v${VERSION}/${asset}`;
  const dest = binaryPath();
  fs.mkdirSync(path.dirname(dest), { recursive: true });

  const response = await fetch(url, { redirect: "follow" });
  if (!response.ok) {
    console.error(`failed to download ${url}: HTTP ${response.status}`);
    process.exit(1);
  }
  const bytes = Buffer.from(await response.arrayBuffer());
  fs.writeFileSync(dest, bytes);
  if (process.platform !== "win32") {
    fs.chmodSync(dest, 0o755);
  }
  writeInstallMetadata();
  console.log(`nolgia ${VERSION} installed for ${process.platform}/${process.arch}`);
}

module.exports = { assetName };

if (require.main === module) {
  main().catch((err) => {
    console.error(err.message || err);
    process.exit(1);
  });
}
