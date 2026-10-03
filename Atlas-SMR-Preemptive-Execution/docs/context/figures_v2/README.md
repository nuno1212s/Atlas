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

There are four figure types with three options each:

| Script | Type | Options |
|---|---|---|
| `arch.py` | Executor architecture | A1 blocks and wires · A2 threads as rooms · A3 pictogram matrix |
| `caching.py` | Caching structure | B1 see-through layers · B2 ledger tape · B3 footprint to scale |
| `lifecycle.py` | Decision lifecycle | C1 state machine · C2 conveyor belt · C3 branch graph |
| `requests.py` | Request flow and threads | D1 subway map · D2 sequence diagrams · D3 containers between planes |

`svgkit.py` holds the palette and shared glyphs (block, card, capsule, thread badge, arrows). The
caching sheets take their values from `../figures/executor_model.py`.

## Rendering

You need Python 3 (standard library only) and `rsvg-convert` (`brew install librsvg`).

```bash
for f in arch caching lifecycle requests; do python3 $f.py <output-dir>; done
```

Each figure is written as `.svg`, as `.png` at 2×, and as `.pdf` with its fonts embedded.
