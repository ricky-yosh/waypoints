# Temporary GPUI patches

The root `[patch.crates-io]` selects these published sources:

| Crate | Upstream version | Local change |
|---|---|---|
| `gpui-kit` | 0.9.0 | Expose `hot-reload = ["gpui/hot-reload"]` |
| `gpui-pre` | 0.3.11 | Use core AccessKit 0.25.2 |
| `gpui-pre-macos` | 0.3.11 | Use AccessKit 0.25.2 and macOS adapter 0.27.2 |
| `gpui-pre-linux` | 0.3.11 | Align core AccessKit and Unix adapter 0.24.1 |
| `gpui-pre-windows` | 0.3.11 | Align core AccessKit and Windows adapter 0.35.2 |

The AccessKit versions must agree because GPUI exchanges accessibility types
between its core and platform backends. These are manifest-only patches, not a
backport of AccessKit implementation code. The app's normal macOS backend uses
the published `accesskit_macos` 0.27.2 release.

That release includes [AccessKit #835](https://github.com/AccessKit/accesskit/pull/835),
fixing the observer bookkeeping corruption that can abort the process when a
window closes on a Touch Bar Mac. See
[the original report](https://github.com/AccessKit/accesskit/issues/823).
Headless UI tests cannot reproduce AppKit's native teardown; verify by repeatedly
closing and reopening Settings on an affected Mac, with and without `./dev`.
Linux and Windows adapter versions are aligned but require testing on those hosts.
The optional `gpui-fast` backend is not selected by this app.

## Refreshing

`scripts/refresh-ui-vendor.py` downloads the pinned crates.io archives, verifies
their SHA-256 checksums, regenerates these five directories, and applies the
manifest patches. It omits archive bookkeeping, upstream lockfiles, and GPUI's
demo examples and their Cargo targets. The example image referenced by
`gpui-pre/src/platform.rs` unit tests is retained. Other files, including original
manifests and licenses when present, are unchanged.

**The script replaces those generated directories.** Review or save local edits
before running:

```sh
python3 scripts/refresh-ui-vendor.py
```

For an upgrade, update the script's versions and checksums from crates.io and
review whether each patch is still necessary. Update the app's normal and dev
dependency pins together. Build, regenerate the workspace lockfile, run tests
and Clippy, and smoke-test native windows and hot reload. Renovate ignores
vendored manifests and cannot perform this refresh for us.

Once published GPUI versions support the fixed AccessKit family, remove their
local overrides and sources. Remove the Kit override once it exposes hot reload
as well. Do not remove the AccessKit alignment from just one side of the shared
type boundary.
