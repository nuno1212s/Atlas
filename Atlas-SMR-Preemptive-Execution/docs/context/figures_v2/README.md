# Preemptive-execution figures, v2

A fresh start on every diagram, from iteration 04. The figures are hand-built SVG with a new
visual language: colour carries the meaning, the type is Avenir Next, there are few words, and
shapes are flat and rounded.

| Colour | Meaning |
|---|---|
| blue | confirmed: S, Δ |
| amber | speculated: C, Ω, S_p |
| violet | the decision being built (ω) and containers, drawn as dashed capsules |
| slate badge | a thread (worker, main, preemptive, confirmed) |
| teal | the exec pool, which runs in parallel |
| sand | the read pool |
| red | discarded (backtrack) |

There are four figure types with three options each. Since iteration 05, every option is drawn once
per executor: `<type>-<option>-{crud,dual,scal}`, for the CRUD cache, dual-state and scalable CRUD
executors, so there are 36 figures in all.

| Script | Type | Options |
|---|---|---|
| `arch.py` | Executor architecture | A1 blocks and wires · A2 threads as rooms · A3 pictogram matrix |
| `caching.py` | Caching structure | B1 see-through layers · B2 ledger tape · B3 footprint to scale |
| `lifecycle.py` | Decision lifecycle | C1 state machine · C2 conveyor belt · C3 branch graph |
| `requests.py` | Request flow and threads | D1 subway map · D2 sequence diagrams · D3 containers between planes |

`scalable_arch.py` (iteration 06) holds three figures for the scalable executor that merge
architecture and caching structure into one: X1 rooms with layers, X2 lanes with a ledger and
X3 layers with threads. They are written as `arch-scal-x{1,2,3}-*`.

`scalable_steps.py` (iteration 07) builds on X1 and X2 and adds their mix, X12: X2's lanes and key
strips, with X1's exact read paths. Each style is drawn as an overview (`step0`) and as steps 1–5
of batch 10, and every step shows where the collision on e stands: run in parallel, collision
check, merge non-colliders, re-run colliders, append Ω10. The files are
`arch-scal-{x1,x2,x12}-step{0..5}`.

From iteration 09, a step figure shows batch 10's own state only once it exists: ω10 from step 3,
Ω10 at step 5. The persistent state (the log, C and S) is faded in any step that does not use it.
The read pool is no longer drawn (iteration 08). If ImageMagick (`magick`) is installed, the
script also writes `arch-scal-{x1,x2,x12}-steps.gif`, which cycles through steps 1–5 at 3 s each.

From iteration 10 the story continues past speculation. Steps 6–8 confirm decision 10:
confirm 10 arrives (Ω10 is the head of the log), commit Ω10 → Δ10 into S (C is refolded), and
send the replies (o3's client gets 8). The step tracker groups the steps as speculate 1–5 and
confirm 6–8, and the GIFs cycle through all eight.

`crud_dual_steps.py` (iteration 11) gives the CRUD cache and dual-state executors the same
treatment, one figure per executor, in the X1, X2 and X12 styles. Each has an overview and six
steps of batch 10, speculate 1–3 then confirm 4–6:
- CRUD cache: o1 and o2 write into ω10, o3 and o4 read, append Ω10; then confirm, commit
  Ω10 → Δ10, send the replies.
- Dual-state: o1 and o2 write S_p in place, o3 and o4 read S_p, log batch 10; then confirm, send
  the replies and forward the batch, and the confirmed worker re-runs it on S.

The files are `arch-{crud,dual}-{x1,x2,x12}-step{0..6}`, plus `-steps.gif`.

Across a step sequence every element keeps its place (iteration 12). The log is drawn newest on
the left (Ω10) and oldest on the right (Ω7), and each decision keeps that slot through
confirmation. Decisions already confirmed turn into "in S" ghosts where they stood, and the
confirm and commit arrows go to Ω10 in its own slot.

`svgkit.py` holds the palette and shared glyphs (block, card, capsule, thread badge, arrows). It also
holds `Svg(..., ox, oy)`, a canvas whose viewBox origin can be moved, so that a layout drawn once can
be cropped to one executor without moving any coordinates. The caching sheets take their values
from `../figures/executor_model.py`.

## Rendering

You need Python 3 (standard library only) and `rsvg-convert` (`brew install librsvg`).

```bash
for f in arch caching lifecycle requests scalable_arch scalable_steps crud_dual_steps; do python3 $f.py <output-dir>; done
```

Each figure is written as `.svg`, as `.png` at 2×, and as `.pdf` with its fonts embedded.
