# Development-tool references

This is a reference list, **not an implementation roadmap or an instruction to
install dependencies**. It replaces earlier tooling proposals that mixed
product architecture, optional GUI debugging, duplicate security checklists,
and superseded MCP integration plans.

## Current choices and boundaries

- Waypoints uses a headless core, a shared local JSON-RPC backend, and CLI and
  desktop clients. Both clients use the same backend. See
  [architecture](02-architecture.md#architecture-decision-local-backend-gui-and-cli).
- The app manifest selects `gpui-kit` with inspector support; see the
  [current stack](06-stack-and-storage.md#gpui) for the pin. The app provides the
  [desktop workflow](05-ui-and-diffs.md). A comparison between `gpui-component`
  and `gpui-kit` is not a pending prerequisite; use GPUI Kit's reexports and
  component layer.
- Installed project skills cover Rust practices/review and GPUI Kit coding/design.
  Read the applicable guides and verify API signatures against the pinned source.
- MCP is out of scope. Do not install `gpui-mcp`, build an MCP adapter, or make
  GUI automation a prerequisite for agent access to workflow operations.
- Runtime UI inspection and visual testing are development tools, not a second
  workflow API. A temporary screenshot is not a required image archive.
- Native Rust is the desktop direction. A React/React Flow prototype is optional;
  adding a JavaScript runtime through GPUIX is not an adopted plan.

## References to evaluate when a concrete need arises

The links below preserve earlier research leads. Listing a project does not
verify its current maintenance, compatibility, security, or advertised features.

| Reference | Potential use |
|---|---|
| [GPUI source](https://github.com/zed-industries/zed) | Framework implementation and patterns; do not depend on Zed's internal editor crates without a specific review |
| [GPUI Kit](https://github.com/longbridge/gpui-kit) and [documentation](https://gpui-kit.com) | Selected application entry point; exact APIs must match the pinned version |
| [GPUI Component](https://github.com/longbridge/gpui-component) | Component-layer source/research, not a second application dependency to add alongside GPUI Kit |
| [awesome-gpui](https://github.com/zed-industries/awesome-gpui) | Discovery of ecosystem candidates, not an automatically trusted dependency list |
| [Native GPUI inspector source](https://github.com/zed-industries/zed/blob/main/crates/gpui/src/inspector.rs) | Runtime GUI inspection; this source does not by itself establish a complete DevTools or external automation API |
| [gpui-devtools](https://github.com/joris-gallot/gpui-devtools) | Optional human-facing runtime inspection; verify compatibility before adoption |
| [Guise](https://github.com/wess/guise) | Alternative-toolkit reference; do not introduce another toolkit solely for debugging without a concrete need |
| [GPUI Kit Canvas](https://github.com/JetSquirrel/gpui-kit-canvas) | Optional design/prototyping research, not a running-application debugger |
| [GPUI Storybook](https://github.com/stayhydated/gpui-storybook) | Optional isolated component development; no MCP integration is planned |
| [GPUIX](https://github.com/remorses/gpuix) | Earlier React/Solid integration research; not selected for the native runtime |
| [Script Kit](https://github.com/johnlindquist/script-kit-next) | Example GPUI application, not an authoritative substitute for project guides |
| [BeautifulRust](https://github.com/morganlinton/BeautifulRust) | Rust guidance and lint-policy research; do not overwrite installed skills/configuration without review |
| [AprilNEA GPUI skills](https://github.com/AprilNEA/gpui-skills) | Alternative guidance to evaluate only if the installed guides leave a concrete gap |

Earlier [gpui-mcp](https://github.com/themixednuts/gpui-mcp) research is superseded
by the no-MCP decision. It is retained here only as a historical reference, not
an installation, evaluation priority, or expected deliverable.

## Dependency and skill review

Before adopting or upgrading a candidate:

1. Identify the capability needed and check the selected stack first.
2. Inspect maintainers, release/commit history, advisories, dependency sources,
   licenses, and compatibility with the pinned GPUI Kit version.
3. Review `build.rs`, procedural macros, installation scripts, hooks and skills
   for unexpected filesystem, process, network or credential access.
4. Test in an isolated fixture. Review Git dependencies at exact commits and
   preserve reproducible resolution through `Cargo.lock`.
5. Record concrete risks, accepted exceptions and the reason for adoption.

Useful tools include [cargo-audit](https://github.com/rustsec/rustsec/tree/main/cargo-audit)
for known advisories, [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) for
dependency/license/source policies, and
[cargo-geiger](https://github.com/geiger-rs/cargo-geiger) for unsafe-code inventory.
These are candidates for security workflows, not a claim that they are installed,
configured in CI, or were run during this cleanup. Unsafe Rust is not itself proof
of a vulnerability, and a passing scanner is not proof of trustworthiness.

The project's actual headless test/build commands are in
[backend checks](11-backend-cli.md#checks). GUI validation must additionally
exercise real windows, keyboard/focus behavior, resizing, themes and failure
states using the installed GPUI Kit guides. Do not substitute compilation or an
ideal screenshot for the task working.
