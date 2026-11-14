Keep things simple and stupid:

1. Does this need to exist? -> no: skip it (YAGNI)
2. Stdlib does it? -> use it
3. Native platform feature? -> use it
4. Installed dependency? -> use it
5. One line? -> one line
6. Only then: the minimum that works

Avoid writing tests unless requested.

Use `git mv` to move things that should be tracked.

Run `make check` whenever a big change is done.

Never attempt to build inside the sandbox.

Avoid OOP, particularly inherentance unless absolutely necessary.

DO not use WebKit unless is is literally for rendering a webpage. Avoid using JS.

Avoid trivial functions and abstractions.

Always run make check outside of the sandbox to avoid blocking.

You are not allowed to use [allow(deprecated)]

Focus on your task.
You are not allowed to revert changes made by others unless explicitly requested.

Use fraction instead of f64 when dealing with time.

Never revert changes made by other agents.

A clear crash is always preferable to an unclean state

Put math logic into the math module. Use num_trait when fit.

You are not allowed to call cargo directly. Use Makefile.

CUDA Oxide supports many Rust features like enum and match. Use those when needed.

Avoid magic numbers.

Use builder pattern where it fits. Avoid crazy long function arguments.

Only bump project version when there is a need for it.

Never implement placeholder of any kind. It either works or it just fails entirely.

Avoid editing .shrimp project files directly. Use the Shrimply binary editing API.
Start with `shrimply cli search "describe the task"`, inspect command help or
`shrimply schema`, and use `--dry-run` to preview edits.
Inform the user when the binary API is missing features that are needed.

Before adding dependency or going with popular options, ask user their opinion on the dependency. Always prefer skia for 2D rendering.


Avoid extra labels and meaningless texts.


When porting to a new UI framework. Use the GTK UI as the referrence.

Avoid platform specific code unless absolutely necessary.

Try to put as much logic into the shared Rust backend as possible.

Before you stop you must spawn a review agent to ensure that you are maximizing code resuablity.

Keep code hygiene in mind, remove ALL duplicated logic BEFORE you add any new lines of code.