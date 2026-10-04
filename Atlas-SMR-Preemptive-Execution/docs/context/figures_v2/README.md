# Preemptive-execution figures, v2

A fresh start on every diagram, from iteration 04. The figures are hand-built SVG with a new
visual language: colour carries the meaning, the type is Avenir Next, there are few words, and
shapes are flat and rounded.

| Colour | Meaning |
|---|---|
| blue | confirmed: Δ* (the confirmed state), Δk |
| amber | speculated: Ω* (the speculative state), Ωk |
| violet | the decision being built (ω) and containers, drawn as dashed capsules |
| slate badge | a thread (worker, main, preemptive, confirmed) |
| teal | the exec pool, which runs in parallel |
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
Ω10 at step 5. The persistent state (the log, Ω\* and Δ\*) is faded in any step that does not use it.
The read pool is not drawn in any figure (iterations 08 and 14). If ImageMagick (`magick`) is installed, the
script also writes `arch-scal-{x1,x2,x12}-steps.gif`, which cycles through steps 1–5 at 3 s each.

From iteration 10 the story continues past speculation. Since iteration 14 the confirmation steps
confirm the **oldest** pending decision, Ω7, because decisions are confirmed in order and 10 cannot be
confirmed before 7, 8 and 9:
- *confirm 7 arrives:* Ω7 is the head of the log, while Ω8–Ω10 stay speculated;
- *commit Ω7 → Δ7* into Δ\*: Ω\* is refolded from Ω8, Ω9 and Ω10;
- *send the replies of Δ7:* batch 10's replies wait with Ω10.
 The step tracker groups the steps as speculate 1–5 and
confirm 6–8, and the GIFs cycle through all eight.

`crud_dual_steps.py` (iteration 11) gives the CRUD cache and dual-state executors the same
treatment, one figure per executor, in the X1, X2 and X12 styles. Each has an overview and six
steps of batch 10, speculate 1–3 then confirm 4–6:
- CRUD cache: o1 and o2 write into ω10, o3 and o4 read, append Ω10; then confirm 7, commit
  Ω7 → Δ7, send the replies of Δ7.
- Dual-state: o1 and o2 write Ω\* in place, o3 and o4 read Ω\*, log batch 10; then confirm 7, send
  batch 7's replies and forward it, and the confirmed worker re-runs batch 7 on Δ\*.

The files are `arch-{crud,dual}-{x1,x2,x12}-step{0..6}`, plus `-steps.gif`.

Across a step sequence every element keeps its place (iteration 12). The log is drawn newest on
the left (Ω10) and oldest on the right (Ω7), and each decision keeps that slot through
confirmation. Decisions already confirmed turn into "in S" ghosts where they stood, and the
confirm and commit arrows go to Ω10 in its own slot.

**State names (from iteration 13).**
- **Δ\*** is the confirmed state: every confirmed decision Δk, applied.
- **Ω\*** is the speculative state the executor reads through while speculating:
  - in the CRUD cache and scalable executors, the cache folded from the pending decisions,
    Ω\* = Ω7 ⊕ Ω8 ⊕ Ω9;
  - in the dual-state executor, the full speculative copy.

These replace C and S, which read alike, and the dual-state's S_p. The scripts' internal
variables (S0, C0, …) keep their old names.

**Speculative cache and confirmed state are separate entities (from iteration 16).** In the CRUD
cache and scalable figures, the worker holds the **speculative cache**: a dashed amber group with ω
(the open batch's writes, its newest layer), the write log and Ω\*. The **confirmed state** Δ\* sits
in its own box, outside that group, and is reached only by reads that miss the cache and by commits.

**ω is never a box of its own (from iteration 17).** ω is drawn as the top layer of Ω\*:
- in sheet figures (X1, X3, the CRUD cache X1), a violet sheet stacked over the Ω\* sheet;
- in box figures (X2, X12), the top row of one cache box (`cache_box` in `scalable_steps.py`), whose
  cells line up with Δ\*'s so that a read drops straight down from ω to Ω\* to Δ\*;
- in block figures (A1, A2, A3, D3), a violet band on top of the Ω\* block (`cache_block` in `arch.py`).

In X2/X12 the log runs oldest to newest towards the cache, so Ω10 is appended next to it. The
parallel reads of the scalable executor go round ω, because ω10 does not exist yet while the exec
pool runs.

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
