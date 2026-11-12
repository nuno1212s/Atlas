"""Scalable CRUD executor: architecture and caching structure in one figure.

    python3 scalable_arch.py <output-dir>

Three ways of joining the two (iteration 06):
  X1  threads as rooms, with the caching layers drawn inside the room that owns them
  X2  thread lanes, with the worker's state drawn as the ledger tape feeding Ω* over Δ*
  X3  see-through layers, with the threads that own and read them drawn around them

Batch 10 is the running example (executor_model.py): o1 c ← 5, o2 e ← 8, o3 read e, o4 read b.
Each op runs in its own container on the exec pool, with a private cache ℓ over the shared Ω* and Δ*;
o3 reads the stale e = 7 from Δ*, so the worker re-runs o2 and o3 straight on Ω*; their recorded writes become Ω10.
"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "figures"))
from executor_model import CRUD, KEYS, TOMB, TRACE10, op_text  # noqa: E402

from caching import Sheet, ray, sheet_label  # noqa: E402
from svgkit import (CONF, CONF_T, CONS, CONS_T, DROP, INK, LIVE, LIVE_T, MUTED, PANEL, POOL, POOL_T,  # noqa: E402
                    READP, READP_T, SPEC, SPEC_T, WHITE, WORK, WORK_T, Svg, legend, person, thread_badge)

PFX = "arch-scal"
S0, C0, LOG0 = CRUD[0].S, CRUD[0].C, CRUD[0].log
OPS = TRACE10.ops
OMEGA10 = TRACE10.delta                    # {c: 5, e: 8}: batch 10's recorded writes, logged as Ω10


def shown(v):
    return "⊥" if v == TOMB else v


def strip(s, x, y, values, colour, tint, cell=18, gap=3, keys=True, size=11):
    """Five key cells a–e; filled where the map holds the key."""
    for i, k in enumerate(KEYS):
        cx = x + i * (cell + gap)
        if keys:
            s.text(cx + cell / 2, y - 4, k, size=9.5, fill=MUTED, anchor="middle")
        if k in values:
            s.rect(cx, y, cell, cell, r=4, fill=colour, stroke=colour)
            s.text(cx + cell / 2, y + cell / 2 + size * 0.36, shown(values[k]), size=size, weight=600, fill=WHITE,
                   anchor="middle")
        else:
            s.rect(cx, y, cell, cell, r=4, fill="none", stroke=colour, sw=0.8, dash="2 2", opacity=0.7)
    return x + 5 * (cell + gap) - gap


def room(s, x, y, w, h, name, colour, tint, sub=None):
    s.rect(x, y, w, h, r=16, fill=tint)
    bw = thread_badge(s, x + 12, y + 18, name, colour)
    if sub:
        s.text(x + 20 + bw, y + 22, sub, size=11.5, fill=MUTED)


def wcard(s, x, y, q, d, w=72, h=58, dashed=False, size=11):
    s.rect(x, y, w, h, r=8, fill=WHITE if dashed else SPEC_T, stroke=SPEC, sw=1.4, dash="5 4" if dashed else None,
           shadow=not dashed)
    s.text(x + 9, y + 18, f"Ω_{{{q}}}", size=13, weight=600, fill=SPEC)
    for r, (k, v) in enumerate(sorted(d.items())):
        s.text(x + 9, y + 34 + r * 13, f"{k} ← {shown(v)}", size=size, fill=INK)


def op_container(s, x, y, w, h, pos, cell=15, gap=3):
    """One op's container: its thread, the op, its private cache ℓ and its reply."""
    local, _, reply = TRACE10.lanes[pos]
    s.rect(x, y, w, h, r=14, fill=LIVE_T, stroke=LIVE, sw=1.5, dash="4 3")
    bw = thread_badge(s, x + 10, y + 17, f"T{pos + 1}", POOL)
    s.text(x + 18 + bw, y + 22, f"o{pos + 1} · {op_text(OPS[pos])}", size=12, weight=600, fill=LIVE)
    s.text(x + 12, y + h - 13, f"ℓ_{{{pos + 1}}}", size=12, weight=600, fill=LIVE)
    strip(s, x + 32, y + h - 13 - cell * 0.75, local, LIVE, LIVE_T, cell=cell, gap=gap, keys=False, size=10)
    stale = pos in TRACE10.colliders and OPS[pos][0] == "r"
    txt = f"→ {reply}" + (" stale" if stale else "")
    s.text(x + w - 10, y + h - 13, txt, size=11, fill=DROP if stale else MUTED, anchor="end",
           weight=600 if stale else 400)


def clients(s, x, y, n=3):
    for i in range(n):
        person(s, x + i * 20, y, colour="#8b93a3", scale=0.9)
    s.text(x + (n - 1) * 10, y + 26, "clients", size=11.5, fill=MUTED, anchor="middle")


# ---------------------------------------------------------------------------
# X1 — rooms, with the caching layers inside the room that owns them
# ---------------------------------------------------------------------------
def x3_layers_threads(out):
    from scalable_steps import conf_box, spec_group     # imported here: scalable_steps imports this module
    s = Svg(1000, 716)
    s.title("Scalable CRUD", "Architecture + caching · X3 · the layers, and the threads that own and read them")
    top = 236
    # exec pool band behind the per-op caches
    s.rect(120, top - 92, 474, 132, r=16, fill=POOL_T)
    thread_badge(s, 132, top - 74, "exec pool · 4 threads in parallel", POOL)
    # the worker band holds the speculative cache: the log and Ω*
    s.rect(120, 290, 850, 206, r=16, fill=WORK_T)
    thread_badge(s, 132, 308, "worker · holds the speculative cache", WORK)
    spec_group(s, 130, 322, 830, 166, tx=140, ty=482)
    # Ω*, which batch 10's merge and re-runs write directly (there is no separate ω)
    cs = Sheet(190, 440)
    cs.draw(s, SPEC, SPEC_T, C0)
    sheet_label(s, cs, "Ω^{*}", "", SPEC)
    # the confirmed state, apart from the cache
    conf_box(s, 120, 508, 850, 100)
    ss = Sheet(190, 596)
    ss.draw(s, CONF, CONF_T, S0)
    sheet_label(s, ss, "Δ^{*}", "", CONF)

    minis = []
    for pos in range(4):
        local = TRACE10.lanes[pos][0]
        m = Sheet(140 + pos * 112, top, slot=15, dx=18, dy=-14, pad=4)
        s.rect(m.x - 8, top - 50, m.w + 24, 82, r=12, fill=LIVE_T, stroke=LIVE, sw=1.3, dash="4 3")
        thread_badge(s, m.x - 2, top - 36, f"T{pos + 1}", POOL)
        s.text(m.x + 28, top - 32, f"o{pos + 1}", size=11.5, weight=600, fill=LIVE)
        m.draw(s, LIVE, LIVE_T, local, size=9.5)
        s.text(m.x + m.w / 2 + 6, top + 22, f"ℓ_{{{pos + 1}}}", size=11.5, weight=600, fill=LIVE, anchor="middle")
        minis.append(m)

    # o3's read of e: its own ℓ3, then Ω*, then Δ*, where it finds the stale 7
    i = KEYS.index("e")
    x0, y0 = minis[2].centre(i)
    cx, cy = cs.centre(i)
    hx, hy = ss.centre(i)
    ray(s, [(x0, y0), (cx, cy), (hx, hy - 12)], [(x0, y0), (cx, cy)])
    s.text(hx + 14, hy - 30, "o3: e → 7, stale", size=11.5, weight=600, fill=DROP)

    # the log, inside the speculative cache
    xs = [620, 704, 788, 872]
    ly = 418
    s.text(616, 350, "join: merge ℓ_{1}, ℓ_{4} and re-run o2, o3,", size=10.5, fill=MUTED)
    s.text(616, 364, "straight on Ω^{*}; their writes are recorded", size=10.5, fill=MUTED)
    wcard(s, xs[0], ly, 10, OMEGA10, dashed=True)
    for x, (q, d, _) in zip(xs[1:], reversed(LOG0)):
        wcard(s, x, ly, q, d)
    s.text(xs[2], 412, "write log", size=12, weight=600)
    jx = minis[3].x + minis[3].w + 26
    s.arrow([(jx, top - 30), (604, top - 30), (604, 380), (520, 380), (520, 406)], stroke=LIVE, sw=1.8)
    s.label(jx + 20, top - 40, "join", size=10.5, fill=LIVE, bg=WHITE)
    s.arrow([(604, 392), (xs[0] + 40, 392), (xs[0] + 40, 416)], stroke=SPEC, sw=1.6)
    s.label(640, 392, "Ω_{10}", size=10.5, fill=SPEC, bg=WHITE)
    s.arrow([(xs[0] - 2, 428), (506, 428)], stroke=SPEC, sw=1.4, dash="5 4")
    s.label(560, 428, "rebuild (backtrack)", size=10.5, fill=SPEC, bg=WHITE)
    s.arrow([(xs[3] + 36, ly + 60), (xs[3] + 36, ss.y - 15), (ss.x + ss.w + 30, ss.y - 15)], stroke=CONF, sw=2)
    s.text(xs[3] + 28, ss.y - 22, "commit Ω_{7} → Δ_{7}", size=11, fill=CONF, anchor="end")

    # consensus and clients
    s.rect(820, 100, 150, 40, r=12, fill=CONS_T, stroke=CONS, sw=1.3)
    s.text(895, 125, "consensus", size=13, weight=600, fill=CONS, anchor="middle")
    s.arrow([(900, 140), (900, 160), (xs[3] + 36, 160), (xs[3] + 36, ly - 2)], stroke=CONS)
    s.text(908, 160, "confirm 7", size=10.5, fill=CONS)
    s.arrow([(820, 112), (360, 112), (360, top - 94)], stroke=CONS)
    s.label(600, 106, "batch k: one op per thread", size=10.5, fill=CONS, bg=WHITE)
    clients(s, 880, 640)
    s.arrow([(xs[3] + 36, ss.y - 15), (xs[3] + 36, 630), (868, 630)], stroke=INK)
    s.text(xs[3] + 44, 622, "replies", size=11, fill=INK)
    legend(s, 32, 700, [("block", CONF, CONF_T, "confirmed"), ("block", SPEC, SPEC_T, "speculated"),
                        ("capsule", LIVE, LIVE_T, "container / being built"), ("badge", WORK, None, "thread"),
                        ("badge", POOL, None, "parallel thread")])
    s.save(os.path.join(out, f"{PFX}-x3-layers-threads"))


def _from_overview(out, style, name):
    """X1 and X2 are drawn by scalable_steps as its overview (step 0); keep the sheet names."""
    import shutil
    import tempfile
    import scalable_steps
    with tempfile.TemporaryDirectory() as tmp:
        (scalable_steps.x1 if style == "x1" else scalable_steps.x2)(tmp, 0)
        for ext in ("svg", "png", "pdf"):
            shutil.move(os.path.join(tmp, f"arch-scal-{style}-step0.{ext}"), os.path.join(out, f"{PFX}-{name}.{ext}"))


def x1_rooms_layers(out):
    _from_overview(out, "x1", "x1-rooms-layers")


def x2_lanes_ledger(out):
    _from_overview(out, "x2", "x2-lanes-ledger")


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "."
    os.makedirs(out, exist_ok=True)
    for f in (x1_rooms_layers, x2_lanes_ledger, x3_layers_threads):
        f(out)
        print(f.__name__)
