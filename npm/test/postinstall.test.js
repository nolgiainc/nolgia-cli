"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const { assetName } = require("../scripts/postinstall.js");

for (const [platform, arch, expected] of [
  ["darwin", "x64", "nolgia-x86_64-apple-darwin"],
  ["darwin", "arm64", "nolgia-x86_64-apple-darwin"],
  // Static musl since v0.2.30 (NOL-1070): the -gnu assets could not start
  // on Debian 12, Ubuntu 22.04 or slim container images.
  ["linux", "x64", "nolgia-x86_64-unknown-linux-musl"],
  ["linux", "arm64", "nolgia-aarch64-unknown-linux-musl"],
  ["win32", "x64", "nolgia-x86_64-pc-windows-msvc.exe"],
  ["win32", "arm64", "nolgia-aarch64-pc-windows-msvc.exe"],
  ["linux", "ia32", null],
  ["freebsd", "x64", null],
]) {
  test(`asset for ${platform}/${arch}`, () => {
    assert.equal(assetName(platform, arch), expected);
  });
}
