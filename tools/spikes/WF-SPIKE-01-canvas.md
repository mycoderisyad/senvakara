# WF-SPIKE-01 — Workflow canvas under the software renderer

Date: 2026-11-27. Machine: the owner's Windows 11 PC. Slint `=1.19.0`, `SLINT_BACKEND=winit-software` (the app default), window 1280×800, release build without LTO (`opt-level 3`, `codegen-units 16`).

Question: can `slint-node-editor` 2.0.0 serve as the Workflows canvas, or does Senvakara need its own?

**Result: no-go for `slint-node-editor`; go for an own canvas** (Rust owns screen-space geometry, Slint draws).

## Crates

- `node-canvas/`: the library's own downstream fixture (`smoke/downstream` at v2.0.0) with workflow-shaped node cards and knobs (`SPIKE_NODES`, `SPIKE_ZOOM`, `SPIKE_ANIM`, `SPIKE_ANIM_COUNT`, `SPIKE_GRID`, `SPIKE_LINKS`, `SPIKE_SNAPSHOT`).
- `own-canvas/`: fallback prototype. Nodes and edges arrive as screen-space rows from Rust; zoom multiplies coordinates and sizes; one `Path` per edge sized to the edge's bounding box; a node move updates that node's row and only its own edge rows.

Both are standalone (`[workspace]` of their own, not workspace members). Build: `cargo build --release` in `node-canvas/` (11 m 39 s cold, 1.4 GB target dir, exe 13 MB); `own-canvas/` reuses it with `CARGO_TARGET_DIR=../node-canvas/target` (21 s).

## Findings

1. **Builds with Senvakara's pins.** `slint-node-editor =2.0.0` + `slint-build =1.19.0` with `experimental-module-builds`, style `fluent-dark`, same Slint features as `apps/senvakara`. No source changes needed.
2. **Zoom is broken under the software renderer.** The library zooms with `transform-scale: root.zoom` on its world container (`node-editor.slint:878-887` in the 2.0.0 crate). Slint 1.19 docs: the software renderer has "No support for rotations or scaling". Snapshots at zoom 0.4 / 1.0 / 1.5: the grid and the zoom-dependent detail change, but node size and position stay identical. Hit testing uses `world × zoom + pan`, so clicks would miss whenever zoom ≠ 1.
3. **Drag cost is dominated by links and the grid, not by node count.** The library re-evaluates every link's geometry on every move (its own comment: a `GeometryVersion` bump "re-evaluates every link in the graph"), and its dense grid is one canvas-wide `Path`.
4. **The own canvas zooms correctly** (snapshots at 0.5 and 1.5: cards, text, ports and curves all scale) and costs about half the CPU while dragging.

## Measurements

CPU is the average over 10 s after a 3 s warm-up, as % of one core (`TotalProcessorTime`); memory is private bytes. "Drag" = one node moved by 3 px every 16 ms (≈ a continuous mouse drag); "idle" = nothing changing.

| Scenario (software renderer) | slint-node-editor | own canvas |
|---|---|---|
| Idle, 50 nodes | 1.6 % (0.16 % at 200) | 0 % |
| Idle, 200 nodes | 0.16 %, 15.2 MB | 0 %, 12.7 MB |
| Drag 1 node, 2 nodes, no grid | 13.7 % | 10.9 % |
| Drag 1 node, 50 nodes + 49 links, no grid | 57.0 % | **25.9 %** |
| Drag 1 node, 50 nodes, no links, no grid | 18.3 % | — |
| Drag 1 node, 50 nodes + links, sparse grid (96 px) | 58.6 % | 33.9 % |
| Drag 1 node, 50 nodes + links, dense grid (24 px, library default) | 99 % | — |
| Drag 1 node, 200 nodes + links, no grid | — | 35.1 % |
| Every node moving each frame, 50 nodes | 99.5 % | — |
| Drag 1 node, 50 nodes, FemtoVG (OpenGL) | 48.7 %, 46.6 MB private | — |

Memory stays small either way: +5 to +8 MB private for 200 nodes over a 2-node graph.

## Consequences for the plan

- Build the canvas in-house (plan §7 fallback). Reuse ideas, not the dependency: bezier hit testing and the pin-geometry cache are worth porting (MIT/Apache-2.0 allows it with the notice kept).
- Budget "drag 50 nodes < 25 % of a core": the own canvas measures 25.9 % without a grid. Keep the grid sparse (or dots drawn only on pan/zoom), and coalesce drag updates to one per frame; re-measure in WF-IMPL with the real node card.
- Idle cost is zero in both: nothing redraws unless a row changes.

## Not verified

- Real mouse drags and the Slint MCP path (the timer stands in for pointer moves).
- Keyboard/accessibility of either canvas.
- Other DPI scales than this PC's.
