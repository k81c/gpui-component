# Architecture

These documents describe the current architecture implemented by `gpui-base`
and the crates built directly on it. They are maintained as durable references
rather than project-progress logs.

- [Architecture](ARCHITECTURE.md) explains the crate boundaries, ownership model,
  component taxonomy, state flow, overlay system, and native/WASM integration.
- [Styling and Motion](STYLING-AND-MOTION.md) explains semantic tokens, typed
  state styles, application-owned presentation, and animation primitives.
- [GPUI Shell](gpui-shell.md) explains the scriptable application runtime built
  on `gpui-base`: the engine seam, the render protocol, call scopes, the object
  model, capabilities and the sandbox, and the measured performance model.
- [Upstream sync 保守ガイド](upstream-sync/README.md) records the fork's fixed
  baseline, implementation differences, complete file inventory, porting order,
  and validation evidence for the next upstream sync. This maintenance guide is
  intentionally written in Japanese.

For component-level APIs and runnable examples, see the
[gpui-base documentation](../website/base/index.md).
