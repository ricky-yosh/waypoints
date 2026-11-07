# Development-tool references

This is a reference list, **not an implementation roadmap or an instruction to
install dependencies**.

## Current choices and boundaries

- Waypoints uses a headless core, a shared local JSON-RPC backend, and CLI and
  desktop clients. Both clients use the same backend. See
  [architecture](02-architecture.md#architecture-decision-local-backend-gui-and-cli).
- The desktop app uses the pinned Rune and Rune Kit revisions in
  `crates/waypoints-app/Cargo.toml`; see the
  [current stack](06-stack-and-storage.md#rune). Verify APIs against that source.
- Installed project skills cover Rust practices and review. Read the applicable
  guides and verify signatures against the pinned source.
- MCP is out of scope. Do not add an MCP adapter or make GUI automation a
  prerequisite for agent access to workflow operations.
- Runtime UI inspection and visual testing are development tools, not a second
  workflow API. A temporary screenshot is not a required image archive.
- Native Rust is the desktop direction. A React/React Flow prototype is optional;
  adding a JavaScript runtime is not an adopted plan.

## References

| Reference | Potential use |
|---|---|
| [Rune](https://github.com/anomalyco/rune) | Selected native framework; use the revision pinned in the app manifest |
| [Rune Kit](https://github.com/anomalyco/rune/tree/main/crates/rune-kit) | Selected component layer at the same pinned revision |
| [BeautifulRust](https://github.com/morganlinton/BeautifulRust) | Rust guidance and lint-policy research; do not overwrite installed skills/configuration without review |
| [cargo-audit](https://github.com/rustsec/rustsec/tree/main/cargo-audit) | Known-advisory scanning candidate |
| [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) | Dependency/license/source policy candidate |
| [cargo-geiger](https://github.com/geiger-rs/cargo-geiger) | Unsafe-code inventory candidate |

Earlier GPUI/vendor and MCP research was removed with the GPUI cutover. Do not
reintroduce a second toolkit or GUI automation dependency without a concrete
need and review.

## Dependency and skill review

Before adopting or upgrading a candidate:

1. Identify the capability needed and check the selected stack first.
2. Inspect maintainers, release/commit history, advisories, dependency sources,
   licenses, and compatibility with the pinned Rune revision.
3. Review `build.rs`, procedural macros, installation scripts, hooks and skills
   for unexpected filesystem, process, network or credential access.
4. Test in an isolated fixture. Review Git dependencies at exact commits and
   preserve reproducible resolution through `Cargo.lock`.
5. Record concrete risks, accepted exceptions and the reason for adoption.

Unsafe Rust is not itself proof of a vulnerability, and a passing scanner is not
proof of trustworthiness.

The actual headless test/build commands are in
[backend checks](11-backend-cli.md#checks) and
[Rune validation](port-info/validation.md). Native validation must additionally
exercise real windows, keyboard/focus, IME, accessibility, resizing, themes and
failure states; compilation or an ideal screenshot is not enough.
