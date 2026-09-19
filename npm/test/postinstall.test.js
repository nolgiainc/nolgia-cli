"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const { assetName } = require("../scripts/postinstall.js");

for (const [platform, arch, expected] of [
  ["darwin", "x64", "nolgia-x86_64-apple-darwin"],
  ["darwin", "arm64", "nolgia-x86_64-apple-darwin"],
  ["linux", "x64", "nolgia-x86_64-unknown-linux-gnu"],
  ["linux", "arm64", "nolgia-aarch64-unknown-linux-gnu"],
  ["win32", "x64", "nolgia-x86_64-pc-windows-msvc.exe"],
  ["win32", "arm64", "nolgia-aarch64-pc-windows-msvc.exe"],
  ["linux", "ia32", null],
  ["freebsd", "x64", null],
]) {
  test(`asset for ${platform}/${arch}`, () => {
    assert.equal(assetName(platform, arch), expected);
  });
}
