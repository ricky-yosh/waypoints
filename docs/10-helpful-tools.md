# GPUI Ecosystem Research

## Current architecture decision — supersedes MCP recommendations below

Waypoints uses a **headless core + shared local JSON-RPC backend + desktop GUI and CLI clients**. The CLI provides workflow operations to local agents with terminal access. **MCP is out of scope: do not install `gpui-mcp` or build an MCP adapter as part of this plan.**

The MCP integration proposals and priorities retained below are earlier research, not the current implementation roadmap. In particular, the proposed MCP debugging architecture, Phase 1 integration, MCP planning deliverables, and initial-stack MCP selection are superseded by this decision.

The native GPUI inspector remains relevant for GUI development. Runtime UI inspection and visual verification are separate from the semantic workflow API; agents can operate Waypoints through its CLI without automating GUI clicks. Screenshots may be temporary; no image archive is required. Evaluate any additional debugging tool for compatibility and actual need before adoption.

See [Architecture: local backend, GUI, and CLI](02-architecture.md#architecture-decision-local-backend-gui-and-cli) and the [current build order](07-spikes-and-build-order.md#build-order).

Use the awesome-gpui repository as a primary reference for discovering GPUI libraries, tools, example applications, and architectural patterns.

Before implementing significant functionality from scratch, consult this directory to identify existing solutions.

Periodically revisit it when evaluating new features, UI components, debugging tools, or development workflows.

Treat listed projects as candidates rather than automatically trusted dependencies. Evaluate their security, maintenance activity, compatibility, and performance before adoption.

# GPUI Development Stack and AI-Assisted Debugging

## 1. Project Objective

Build a development environment for native Rust desktop applications using **GPUI**, Zed's GPU-accelerated UI framework.

The development environment should prioritize:

1. Native Rust implementation and performance.
2. Reusable UI components and consistent styling.
3. Idiomatic Rust and GPUI-specific coding practices.
4. AI-assisted UI development and debugging.
5. Runtime inspection of rendered elements.
6. Automated UI interaction and visual verification.
7. Dependency security and maintainability.

The primary objective is to allow AI coding agents such as Claude Code or Codex to **inspect, interact with, debug, and iteratively improve running GPUI applications**, rather than relying exclusively on source-code analysis.

---

# 2. GPUI Libraries and Frameworks

## GPUI — Core Framework

**Repository:** https://github.com/zed-industries/zed

GPUI is the underlying Rust framework used to build native, GPU-accelerated desktop interfaces.

It provides:
- Declarative UI construction.
- Entity-based state management.
- GPU-accelerated rendering.
- Event handling and subscriptions.
- Layout and styling.
- Asynchronous tasks.
- Native window management.

**Decision:** Use GPUI as the primary UI framework.

All additional libraries must be compatible with the project's pinned GPUI revision.

## gpui-component — UI Component Library

**Repository:** https://github.com/longbridge/gpui-component

Provides reusable GPUI components and styling conventions.

Potential benefits:
- Reduces the need to implement common controls.
- Provides established component patterns.
- Supports consistent application styling.
- Includes GPUI-specific development guidance.

**Planning task:** Evaluate whether to adopt gpui-component as the primary component library.

Review compatibility, dependencies, maintenance, and component coverage.

## gpui-kit — Alternative Component Library

**Repository:** https://github.com/longbridge/gpui-kit

Another component library providing reusable GPUI components and development conventions.

Relevant because of its relationship to GPUI Kit Canvas.

**Planning task:** Compare gpui-kit against gpui-component.

Avoid adopting both without a clear requirement.

Evaluate:
- Component availability.
- Styling and theming.
- API stability.
- GPUI version compatibility.
- Documentation.
- Maintenance activity.

## GPUIX — React/Solid Integration

**Repository:** https://github.com/remorses/gpuix

Allows developers to construct GPUI applications using React or SolidJS.

Unlike a React-to-Rust compiler, GPUIX retains JavaScript execution at runtime.

Potential benefits:
- JSX-based UI development.
- Familiar React/Solid programming models.
- Rapid UI iteration.
- Development and testing utilities.

Potential drawbacks:
- Additional JavaScript runtime.
- JavaScript–Rust integration overhead.
- More complex debugging architecture.
- Additional dependencies.
- Potentially larger binaries and higher memory consumption.

**Decision:** Do not adopt GPUIX initially.

Prefer native Rust GPUI unless there is a compelling productivity requirement.

---

# 3. Visual UI Development

## GPUI Kit Canvas

**Repository:** https://github.com/JetSquirrel/gpui-kit-canvas

A visual design environment for GPUI Kit applications.

Capabilities include:
- Drag-and-drop UI composition.
- Component previews.
- Multi-screen design.
- Theme and styling configuration.
- AI-oriented implementation instructions.

Potential workflow:

```text
Visual UI Design
       |
       v
GPUI Kit Canvas
       |
       v
AI Coding Agent
       |
       v
Rust GPUI Implementation
```

**Important:** GPUI Kit Canvas is a design tool, not a debugger for running GPUI applications.

Its usefulness depends partly on whether gpui-kit is adopted.

**Planning task:** Determine whether GPUI Kit Canvas can export structured design information that agents can use for implementation and visual verification.

---

# 4. UI Inspection and Debugging

This is a high-priority area.

## Native GPUI Inspector

**Source:**
https://github.com/zed-industries/zed/blob/main/crates/gpui/src/inspector.rs

GPUI contains native inspection infrastructure.

Relevant capabilities include:
- Element identification.
- Source-code locations.
- Element selection.
- Interactive element picking.
- Inspector state.
- Custom inspector renderers.

This infrastructure could serve as the foundation for an AI-accessible debugging interface.

However, `inspector.rs` is not itself a complete DevTools implementation.

In particular, it does not independently provide a complete serialized UI tree, computed style inspection, or an MCP server.

**Planning task:**

Investigate how GPUI's inspector integrates with its rendering and layout systems.

Determine whether its internal data can be exposed through a development-only API.

Prioritize:
1. Element hierarchy.
2. Element identifiers.
3. Rust source locations.
4. Element bounding boxes.
5. Computed layout information.
6. Styling information.
7. Element visibility and clipping.

## gpui-devtools

**Repository:** https://github.com/joris-gallot/gpui-devtools

A third-party visual inspection tool for GPUI.

Potential capabilities:
- Interactive element inspection.
- Element hierarchy and identification.
- Layout dimensions.
- Styling information.
- Source-code navigation.

**Limitation:** No verified native MCP interface for AI agents.

**Planning task:**

Evaluate whether gpui-devtools can provide the inspection functionality needed without implementing a new inspector.

Investigate whether its inspection data can be reused programmatically.

Check compatibility with the current GPUI revision.

## Guise DevTools

**Repository:** https://github.com/wess/guise

An alternative GPUI-related UI toolkit with inspection and debugging functionality.

Potentially useful for:
- Component hierarchy inspection.
- Box-model visualization.
- Styling inspection.
- Layout diagnostics.

**Planning task:** Evaluate as an alternative or reference implementation.

Do not introduce Guise solely for debugging unless integration is practical.

---

# 5. AI Agent UI Interaction

## gpui-mcp

**Repository:** https://github.com/themixednuts/gpui-mcp

An MCP integration intended to let AI agents interact with GPUI applications.

This is one of the most relevant projects for the proposed workflow.

Capabilities to evaluate:
- Semantic UI inspection.
- Element identification.
- Clicking and keyboard interaction.
- Screenshot capture.
- Application state observation.
- Automated UI testing.

Unlike GPUI's native inspector, this project focuses on exposing functionality to external agents.

**Important limitation:**

An accessibility or semantic UI tree does not necessarily expose GPUI's complete internal layout and styling information.

Application-side instrumentation may also be necessary.

**Planning task:**

Investigate gpui-mcp as the primary AI interaction interface.

Determine:
1. Supported GPUI versions.
2. Integration requirements.
3. Available MCP tools.
4. Screenshot capabilities.
5. Element identification strategy.
6. Accessibility support.
7. Security implications.
8. Whether native inspector data can be integrated.

## GPUI Storybook

**Repository:** https://github.com/stayhydated/gpui-storybook

A development environment for previewing and testing GPUI components independently.

Potential capabilities:
- Isolated component rendering.
- Interactive component previews.
- Component inspection.
- Screenshot capture.
- MCP-based automation.

**Planning task:**

Evaluate whether Storybook can serve as a testing environment for reusable components.

Determine whether its MCP interface exposes inspector information or only interaction and screenshots.

It may be useful for component development even if it is not adopted for full-application debugging.

---

# 6. Proposed AI Debugging Architecture

The recommended architecture combines source-code analysis, native runtime inspection, and automated UI interaction.

```text
              Claude Code / Codex
                      |
          +-----------+-----------+
          |           |           |
          v           v           v
     Rust Source   MCP Tools   Screenshots
          |           |           |
          |           v           |
          |      gpui-mcp         |
          |           |           |
          |           v           |
          |     GPUI Inspector    |
          |           |           |
          +-----------+-----------+
                      |
                      v
              Running GPUI App
                      |
                      v
              Diagnose Problem
                      |
                      v
               Modify Rust
                      |
                      v
              Rebuild and Test
                      |
                      +----> Repeat
```

### Desired Agent Capabilities

The eventual system should expose operations equivalent to:

| Tool | Description |
|---|---|
| `get_ui_tree` | Return the current element hierarchy |
| `get_element_info` | Return element ID, type, and source location |
| `get_element_bounds` | Return rendered position and dimensions |
| `get_element_styles` | Return layout and styling information |
| `highlight_element` | Visually identify an element |
| `click_element` | Click an element |
| `type_text` | Enter text into a focused input |
| `press_key` | Simulate keyboard input |
| `take_screenshot` | Capture rendered UI |
| `get_focus_state` | Inspect keyboard focus |
| `get_render_stats` | Retrieve rendering diagnostics, if available |

These are proposed interface requirements, not a claim that existing libraries implement all of them.

### Preferred Implementation Strategy

**Phase 1: Existing tools**

Integrate gpui-mcp and evaluate its interaction and screenshot capabilities.

Use the native GPUI inspector or gpui-devtools for human debugging.

**Phase 2: Enhanced inspection**

Investigate exposing native GPUI inspector data through MCP.

Prioritize element bounds, source locations, and hierarchy information.

**Phase 3: Automated debugging**

Enable agents to:
1. Launch the application.
2. Inspect the current UI.
3. Interact with controls.
4. Capture screenshots.
5. Identify rendering or behavioral problems.
6. Locate the responsible Rust code.
7. Apply changes.
8. Rebuild and verify.

**Phase 4: Regression testing**

Add repeatable interaction tests, layout assertions, and screenshot comparisons.

---

# 7. AI Coding Skills and Development Practices

## BeautifulRust

**Repository:** https://github.com/morganlinton/BeautifulRust

Provides AI agent guidance for general Rust development.

Relevant areas:
- Ownership and borrowing.
- Error handling.
- API design.
- Idiomatic Rust.
- Code review.
- Maintainability.
- Clippy conventions.

**Limitation:** Does not provide dedicated GPUI framework guidance.

**Recommendation:** Evaluate as a general Rust development and review skill.

## gpui-component Agent Skills

**Repository:** https://github.com/longbridge/gpui-component

Contains GPUI-specific development guidance.

Relevant concepts:
- `Entity<T>` and `WeakEntity<T>`.
- `App`, `Window`, and `Context<T>`.
- `Render` and `IntoElement`.
- Event handling.
- Subscriptions.
- Reactive updates.
- Focus management.
- Asynchronous operations.
- Component construction.
- Testing.

**Recommendation:** Prioritize these skills for native GPUI development.

Verify that the documented APIs match the project's pinned GPUI version.

## gpui-kit Agent Skills

**Repository:** https://github.com/longbridge/gpui-kit

Provides GPUI Kit development conventions and design guidance.

**Recommendation:** Consider if gpui-kit becomes the selected component library.

## AprilNEA GPUI Skills

**Repository:** https://github.com/AprilNEA/gpui-skills

An alternative collection of GPUI-oriented agent instructions.

**Planning task:** Compare against Longbridge's skills for coverage, correctness, and maintenance.

## Script Kit GPUI Implementation

**Repository:** https://github.com/johnlindquist/script-kit-next

A GPUI-based desktop application that may provide useful architectural and implementation examples.

**Recommendation:** Treat as a reference implementation rather than an automatically trusted source of best practices.

---

# 8. Dependency Security and Integrity

Before adopting third-party libraries, perform a security and maintenance review.

### Automated Rust Checks

Use:

```bash
cargo audit
cargo deny check
cargo geiger
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

These checks serve different purposes:

- `cargo audit`: Known dependency vulnerabilities.
- `cargo deny`: Dependency policies, licenses, and advisories.
- `cargo geiger`: Unsafe Rust usage.
- `cargo clippy`: Code-quality and correctness lints.
- `cargo test`: Functional regression testing.

Passing these checks does not establish that a dependency is free of malicious behavior.

### Manual Review

Inspect:
- `Cargo.toml` and `Cargo.lock`.
- Git dependencies.
- `build.rs` files.
- Procedural macros.
- Installation scripts.
- Network access.
- Filesystem access.
- Process execution.
- GitHub Actions.
- Maintainer activity.
- Release history.

Pin third-party Git dependencies to reviewed commits.

### MCP Security

MCP servers deserve additional attention because they can expose application information and perform actions.

Prefer:
- Local-only communication.
- Development-only integration.
- No production credentials.
- Minimal filesystem permissions.
- Explicitly defined interaction capabilities.
- Isolated testing environments.

Do not expose the debugging interface in production builds without a separate security review.

### AI Skill Security

Agent skills should also be reviewed before installation.

Inspect:
- `SKILL.md` instructions.
- Referenced scripts.
- Installation commands.
- Hooks.
- External downloads.
- Unexpected requests for credentials or elevated permissions.

Treat third-party agent instructions as untrusted until reviewed.

---

# 9. Recommended Initial Stack

| Category | Initial selection | Status |
|---|---|---|
| UI framework | GPUI | Recommended |
| Component library | gpui-component or gpui-kit | Evaluate |
| Visual design | GPUI Kit Canvas | Optional |
| UI inspection | Native GPUI inspector | Investigate |
| Human-facing DevTools | gpui-devtools | Evaluate |
| Agent UI interaction | gpui-mcp | Priority evaluation |
| Component testing | GPUI Storybook | Optional |
| Rust best practices | BeautifulRust | Evaluate |
| GPUI best practices | Longbridge GPUI skills | Recommended |
| Static analysis | Clippy | Recommended |
| Dependency auditing | cargo-audit / cargo-deny | Recommended |
| Unsafe-code review | cargo-geiger | Optional |

GPUIX is not recommended for the initial implementation because the project prioritizes native Rust execution.

---

# 10. Planning Priorities

## Priority 1 — Establish the GPUI Stack

- Determine the exact GPUI version or Git revision.
- Evaluate gpui-component versus gpui-kit.
- Select one primary component library.
- Establish a minimal working application.
- Verify build and platform compatibility.

## Priority 2 — Configure AI Development Guidance

- Evaluate BeautifulRust.
- Review Longbridge's GPUI agent skills.
- Verify skill compatibility with the selected GPUI revision.
- Establish project-level coding conventions.
- Configure formatting, linting, and testing.

## Priority 3 — Enable AI UI Interaction

- Evaluate gpui-mcp.
- Integrate it into a minimal GPUI application.
- Verify UI-tree inspection.
- Verify clicking and keyboard interaction.
- Verify screenshot capture.
- Document limitations.

## Priority 4 — Investigate Native Inspector Integration

- Examine GPUI's `inspector.rs`.
- Trace element registration and inspection state.
- Investigate access to layout information.
- Compare against gpui-devtools.
- Determine whether an MCP adapter is practical.

## Priority 5 — Implement an Automated Debugging Loop

Create a proof of concept where an agent can:

1. Launch a GPUI application.
2. Inspect the UI.
3. Identify an element.
4. Interact with it.
5. Capture a screenshot.
6. Find its Rust source.
7. Modify the implementation.
8. Rebuild and verify the result.

## Priority 6 — Evaluate Optional Tooling

After the core workflow works, investigate:

- GPUI Storybook.
- GPUI Kit Canvas.
- Guise DevTools.
- Screenshot regression testing.
- Performance diagnostics.

---

# 11. Expected Deliverables

The planning agent should produce:

1. **Compatibility assessment:** Determine which libraries work with the selected GPUI revision.
2. **Dependency recommendation:** Select a minimal, maintainable development stack.
3. **Security assessment:** Identify concrete risks in third-party dependencies and installation procedures.
4. **MCP integration plan:** Explain how agents will inspect and interact with the running application.
5. **Inspector integration plan:** Determine how to expose layout and source-location information.
6. **Development workflow:** Define how agents implement, test, debug, and verify UI changes.
7. **Implementation roadmap:** Break the work into incremental milestones with clear acceptance criteria.

## Final Architectural Principle

**Do not make source-code inspection the only debugging mechanism.**

The intended workflow should combine:

**Rust source code + runtime UI inspection + screenshots + interaction automation + automated verification.**

Prefer existing GPUI infrastructure over custom implementations, but verify that each library actually provides the required capabilities before adopting it.

The goal is a reliable, agent-accessible GPUI development environment with minimal runtime overhead and minimal unnecessary dependencies.

# 12. Dependency Security and Integrity Verification

All third-party GPUI libraries, Rust crates, MCP servers, and AI agent skills should undergo security and integrity checks before integration.

## Automated Security Tools

Integrate the following tools into the development workflow:

| Tool | Purpose |
|---|---|
| [cargo-audit](https://github.com/rustsec/rustsec/tree/main/cargo-audit) | Identify dependencies with known security vulnerabilities using the RustSec advisory database. |
| [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) | Enforce dependency security, licensing, source, and version policies. |
| [cargo-geiger](https://github.com/geiger-rs/cargo-geiger) | Identify unsafe Rust usage throughout the dependency tree. |
| cargo clippy | Detect common Rust programming mistakes and code-quality issues. |

### Initial Setup

```bash
cargo install cargo-audit cargo-deny cargo-geiger

# Initialize dependency policies
cargo deny init

# Run security checks
cargo audit
cargo deny check
cargo geiger
```

Configure `cargo-audit` and `cargo-deny` to run automatically in CI.

Use `cargo-geiger` during dependency evaluations and security reviews. Unsafe Rust is not inherently malicious, but it may warrant closer inspection.

## Third-Party Library Verification

Before adopting libraries discovered through [awesome-gpui](https://github.com/zed-industries/awesome-gpui) or other sources:

1. **Review maintainers:** Examine contributor history, project ownership, and maintenance activity.
2. **Inspect dependencies:** Identify unexpected dependencies and unnecessary functionality.
3. **Inspect build scripts:** Review `build.rs`, procedural macros, and installation scripts because they may execute code during compilation.
4. **Review sensitive operations:** Investigate unexpected network connections, filesystem access, process execution, and environment-variable access.
5. **Check repository integrity:** Examine commit history, releases, security advisories, and GitHub Actions.
6. **Verify compatibility:** Ensure the library works with the project's pinned GPUI revision.
7. **Pin dependencies:** Use specific reviewed Git commits and commit `Cargo.lock` for reproducible dependency resolution.
8. **Review updates:** Inspect dependency changes before upgrading.

Do not treat GitHub stars, inclusion in awesome-gpui, or automated security scan results as proof of security.

## AI Agent Skills and MCP Security

Third-party AI agent skills and MCP servers require additional scrutiny.

**Agent skills:**
- Review `SKILL.md` files before installation.
- Inspect referenced scripts, hooks, and installation commands.
- Identify instructions requesting unnecessary permissions or access to sensitive files.
- Treat external skill instructions as untrusted until reviewed.

**MCP servers:**
- Prefer local-only connections.
- Avoid exposing development servers publicly.
- Limit filesystem and command-execution permissions.
- Prevent unnecessary access to credentials and environment variables.
- Disable debugging interfaces in production builds.
- Test unfamiliar servers in isolated environments.

## Secure Development Workflow

Establish a repeatable dependency review process:

```text
Discover Library
       |
       v
Review Repository
       |
       v
Inspect Dependencies & Build Scripts
       |
       v
Run Security Scanners
       |
       v
Verify GPUI Compatibility
       |
       v
Test in Isolated Environment
       |
       v
Pin Reviewed Version
       |
       v
Integrate into Project
       |
       v
Monitor Vulnerabilities & Updates
```

## Planning Agent Requirements

The planning agent should:

1. Establish automated dependency security checks.
2. Create a reusable third-party library evaluation procedure.
3. Review proposed GPUI libraries before installation.
4. Verify the security of MCP servers and agent skills.
5. Minimize unnecessary dependencies and permissions.
6. Document identified risks and accepted exceptions.
7. Include dependency vulnerability monitoring in CI.

**Security principle:** Prefer established, maintainable libraries, but verify their behavior rather than relying on reputation alone. Automated scanners are useful safeguards, not guarantees of security.
