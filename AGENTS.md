# Agent Instructions

## Philosophy (KISS & YAGNI)

1. Does this need to exist? -> no: skip it (YAGNI)
2. Stdlib does it? -> use it
3. Native platform feature? -> use it
4. Installed dependency? -> use it
5. One line? -> one line
6. Only then: the minimum that works

## Code Design & Architecture

- Avoid OOP, particularly inheritance, unless absolutely necessary.
- Avoid trivial functions, abstractions, magic numbers, and extra labels or meaningless text.
- Remove ALL duplicated logic BEFORE adding new lines of code.
- Use the builder pattern where it fits; avoid long function argument lists.
- Never implement placeholders of any kind—features must either work or fail entirely.
- A clear crash is always preferable to an unclean state.
- You are not allowed to use `#[allow(deprecated)]`.
- Put math logic into the `math` module and use `num_traits` when it fits.
- Use fractions instead of `f64` when dealing with time.

## UI & Platform Guidelines

- Try to put as much logic into the shared Rust backend as possible.
- Avoid platform-specific code unless absolutely necessary.
- When porting to a new UI framework, use the GTK UI as the reference.
- Always prefer Skia for 2D rendering.
- Do not use WebKit unless it is literally for rendering a webpage. Avoid using JS.

## Dependencies

- Reusing dependencies already declared or locked in this repository is allowed without asking, including adding an existing workspace crate or third-party crate as a direct dependency.
- Ask the user before introducing a dependency that is new to the repository.

## Build, Verification & Tooling

- You are not allowed to call `cargo` directly; use the `Makefile`.
- Never attempt to build inside the sandbox.
- Run `make check` outside of the sandbox whenever a big change is done (to avoid blocking).
- Avoid writing tests unless requested.
- Use `git mv` to move tracked files.
- Only bump the project version when needed.
- Avoid editing `.shrimp` project files directly; use the Shrimply binary editing API:
  - Start with `shrimply cli search "describe the task"`, inspect command help or `shrimply schema`, and use `--dry-run` to preview edits.
  - Inform the user when the binary API is missing needed features.

## Agent Workflow & Coordination

- Focus on your task. Never revert changes made by other agents or users unless explicitly requested.
- Use the ignored root `LOCAL.md` for agent coordination:
  - Read it at task start and before editing shared files (create it locally if absent; never commit it).
  - Append dated task ownership, progress, requests, and replies.
  - Preserve other agents' entries and check pending requests before finishing.
- Before you stop, you must spawn a review agent to ensure the code has no duplicated paths or logic: zoom out to the big picture, identify new ways to unify things, and see what existing code can now be removed as a result of the unified structure.
