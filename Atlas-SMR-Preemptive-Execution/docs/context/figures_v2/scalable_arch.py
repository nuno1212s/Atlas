"""Scalable CRUD executor: architecture and caching structure in one figure.

    python3 scalable_arch.py <output-dir>

Three ways of joining the two (iteration 06):
  X1  threads as rooms, with the caching layers drawn inside the room that owns them
  X2  thread lanes, with the worker's state drawn as the ledger tape feeding C over S
  X3  see-through layers, with the threads that own and read them drawn around them

Batch 10 is the running example (executor_model.py): o1 c ← 5, o2 e ← 8, o3 read e, o4 read b.
Each op runs in its own container on the exec pool, with a private cache ℓ over the shared C and S;
o3 reads the stale e = 7 from S, so the worker re-runs o2 and o3 and builds ω10, which becomes Ω10.
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
OMEGA10 = TRACE10.delta                    # {c: 5, e: 8}


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


def omega_box(s, x, y, w=124, h=74):
    s.rect(x, y, w, h, r=10, fill=LIVE_T, stroke=LIVE, sw=1.6, shadow=True)
    s.text(x + 10, y + 19, "ω_{10}", size=14, weight=600, fill=LIVE)
    s.text(x + w - 8, y + 19, "built here", size=10.5, fill=MUTED, anchor="end")
    strip(s, x + 10, y + 42, OMEGA10, LIVE, LIVE_T, cell=18, gap=3)


def clients(s, x, y, n=3):
    for i in range(n):
        person(s, x + i * 20, y, colour="#8b93a3", scale=0.9)
    s.text(x + (n - 1) * 10, y + 26, "clients", size=11.5, fill=MUTED, anchor="middle")


# ---------------------------------------------------------------------------
# X1 — rooms, with the caching layers inside the room that owns them
# ---------------------------------------------------------------------------
def x1_rooms_layers(out):
    s = Svg(1000, 720)
    s.title("Scalable CRUD", "Architecture + caching · X1 · each thread's room holds the state it owns")
    s.rect(30, 96, 940, 34, r=17, fill=CONS_T, stroke=CONS, sw=1.3)
    s.text(500, 118, "consensus", size=13, weight=600, fill=CONS, anchor="middle")

    # exec pool: one container per op, each with its private ℓ
    room(s, 30, 158, 380, 362, "exec pool", POOL, POOL_T, "4 threads, in parallel")
    for pos in range(4):
        op_container(s, 46, 196 + pos * 80, 348, 64, pos)
    s.arrow([(150, 130), (150, 156)], stroke=CONS)
    s.label(214, 146, "batch k: one op per thread", size=11, fill=CONS, bg=WHITE)

    # worker: ω, the log and C over S
    room(s, 430, 158, 540, 446, "worker", WORK, WORK_T, "owns the log, C and S")
    omega_box(s, 446, 196)
    s.text(508, 290, "merge ℓ_{1}, ℓ_{4}", size=10.5, fill=MUTED, anchor="middle")
    s.text(508, 304, "re-run o2, o3", size=10.5, fill=MUTED, anchor="middle")
    s.arrow([(410, 230), (444, 230)], stroke=LIVE, sw=1.8)
    s.label(427, 220, "join", size=10.5, fill=LIVE, bg=WHITE)
    xs = [596, 680, 764, 848]
    s.arrow([(570, 225), (594, 225)], stroke=SPEC, sw=1.6)
    wcard(s, xs[0], 196, 10, OMEGA10, dashed=True)
    for x, (q, d, _) in zip(xs[1:], reversed(LOG0)):
        wcard(s, x, 196, q, d)
    s.text(xs[0], 272, "newest", size=10.5, fill=MUTED)
    s.text(xs[3] + 72, 272, "oldest", size=10.5, fill=MUTED, anchor="end")
    s.arrow([(xs[0] + 50, 268), (xs[3] + 20, 268)], stroke="#b9bfca", sw=1)
    s.text(xs[1] + 4, 290, "write log", size=12, weight=600)

    cs, ss = Sheet(500, 436, slot=50, dx=40, dy=-26), Sheet(500, 560, slot=50, dx=40, dy=-26)
    ss.draw(s, CONF, CONF_T, S0, size=13)
    cs.draw(s, SPEC, SPEC_T, C0, size=13)
    s.text(474, cs.y - 2, "C", size=20, weight=600, fill=SPEC, anchor="end")
    s.text(474, ss.y - 2, "S", size=20, weight=600, fill=CONF, anchor="end")
    s.arrow([(xs[2] + 36, 256), (xs[2] + 36, 300), (cs.x + cs.w + 26, cs.y - 30)], stroke=SPEC, sw=1.6)
    s.label(xs[2] + 26, 314, "fold: C = Ω_{7} ⊕ Ω_{8} ⊕ Ω_{9}", size=10.5, fill=SPEC, bg=WORK_T, anchor="end")

    # commit: confirm k reaches the oldest, Ω7, which becomes Δ7 and is written into S
    s.arrow([(xs[3] + 36, 130), (xs[3] + 36, 194)], stroke=CONS)
    s.label(xs[3] + 36, 146, "confirm k", size=11, fill=CONS, bg=WHITE)
    s.arrow([(xs[3] + 72, 230), (950, 230), (950, ss.y - 12), (ss.x + ss.w + 24, ss.y - 12)], stroke=CONF, sw=2)
    s.text(944, 380, "commit", size=11.5, fill=CONF, weight=600, anchor="end")
    s.text(944, 395, "Ω_{7} → Δ_{7}", size=11.5, fill=CONF, anchor="end")

    # cross-thread reads: o3 and o4 read the worker's C and S
    i = KEYS.index("e")
    ex_, ey = cs.centre(i)
    sx_, sy = ss.centre(i)
    ray(s, [(394, 372), (ex_, 372), (ex_, sy - 11)], [(ex_, ey)])
    s.text(sx_ + 14, sy - 24, "o3 finds e = 7", size=11, weight=600, fill=DROP)
    i = KEYS.index("b")
    bx, by = cs.centre(i)
    ray(s, [(394, 468), (420, 468), (420, 386), (bx, 386), (bx, by - 11)])
    s.text(446, 362, "&C, &S: read-only, shared by all four threads", size=10.5, fill=MUTED)

    # read pool and clients
    room(s, 30, 540, 380, 64, "read pool", READP, READP_T, "4 threads, unordered reads")
    for k in range(4):
        s.circle(330 + k * 18, 586, 6, fill=READP, opacity=0.75)
    s.arrow([(410, 572), (ss.x + 4, 572), (ss.x + 14, ss.y - 4)], stroke=READP, dash="4 3")
    s.label(452, 586, "&S", size=11, fill=READP, bg=WORK_T)
    s.rect(30, 624, 940, 40, r=20, fill="#eef0f3")
    clients(s, 480, 646)
    s.arrow([(950, ss.y - 12), (950, 622)], stroke=INK)
    s.text(944, 612, "replies of Δ_{7}", size=11, fill=INK, anchor="end")
    legend(s, 32, 700, [("block", CONF, CONF_T, "confirmed"), ("block", SPEC, SPEC_T, "speculated"),
                        ("capsule", LIVE, LIVE_T, "container / being built"), ("badge", WORK, None, "thread"),
                        ("badge", POOL, None, "parallel thread")])
    s.save(os.path.join(out, f"{PFX}-x1-rooms-layers"))


# ---------------------------------------------------------------------------
# X2 — lanes, with the worker's state as a ledger tape feeding C over S
# ---------------------------------------------------------------------------
def kv_box(s, x, y, w, h, name, values, colour, tint, sub=None):
    s.rect(x, y, w, h, r=12, fill=tint, stroke=colour, sw=1.6, shadow=True)
    s.text(x + 12, y + 22, name, size=16, weight=600, fill=colour)
    if sub:
        s.text(x + w - 10, y + 22, sub, size=10.5, fill=MUTED, anchor="end")
    strip(s, x + 12, y + 40, values, colour, tint, cell=24, gap=6, size=12)


def x2_lanes_ledger(out):
    s = Svg(1000, 620)
    s.title("Scalable CRUD", "Architecture + caching · X2 · thread lanes, with the state as a ledger")
    s.rect(30, 230, 108, 58, r=12, fill=CONS_T, stroke=CONS, sw=1.4)
    s.text(84, 257, "consensus", size=13.5, weight=600, fill=CONS, anchor="middle")
    s.text(84, 274, "orders batches", size=10.5, fill=MUTED, anchor="middle")
    s.rect(156, 96, 744, 480, r=18, fill=PANEL)

    # exec pool lane
    s.rect(172, 112, 712, 126, r=14, fill=POOL_T)
    thread_badge(s, 184, 130, "exec pool · 4 threads in parallel", POOL)
    for pos in range(4):
        op_container(s, 184 + pos * 174, 150, 164, 76, pos, cell=12, gap=2)
    s.arrow([(138, 244), (152, 244), (152, 188), (182, 188)], stroke=CONS)
    s.label(150, 210, "batch k", size=10.5, fill=CONS, bg=PANEL, anchor="end")

    # worker lane: ω → tape (newest left, oldest right) → C over S
    s.rect(172, 254, 712, 230, r=14, fill=WORK_T)
    thread_badge(s, 184, 272, "worker", WORK)
    omega_box(s, 184, 300)
    s.arrow([(246, 238), (246, 298)], stroke=LIVE, sw=1.8)
    s.label(246, 262, "join", size=10.5, fill=LIVE, bg=WORK_T)
    s.text(246, 396, "merge ℓ_{1}, ℓ_{4},", size=10.5, fill=MUTED, anchor="middle")
    s.text(246, 410, "re-run o2, o3", size=10.5, fill=MUTED, anchor="middle")
    xs = [338, 420, 502, 584]
    s.arrow([(308, 330), (336, 330)], stroke=SPEC, sw=1.6)
    wcard(s, xs[0], 300, 10, OMEGA10, dashed=True)
    for x, (q, d, _) in zip(xs[1:], reversed(LOG0)):
        wcard(s, x, 300, q, d)
    s.text(xs[0], 378, "newest", size=10.5, fill=MUTED)
    s.arrow([(xs[0] + 50, 374), (xs[3] + 6, 374)], stroke="#b9bfca", sw=1)
    s.text(xs[3] + 4, 392, "oldest", size=10.5, fill=MUTED, anchor="end")
    kv_box(s, 702, 290, 168, 76, "C", C0, SPEC, SPEC_T, "Ω_{7} ⊕ Ω_{8} ⊕ Ω_{9}")
    kv_box(s, 702, 380, 168, 76, "S", S0, CONF, CONF_T, "confirmed")
    s.arrow([(658, 314), (700, 314)], stroke=SPEC, sw=1.6)
    s.label(679, 304, "fold", size=10.5, fill=SPEC, bg=WORK_T)
    s.arrow([(xs[3] + 46, 360), (xs[3] + 46, 440), (700, 440)], stroke=CONF, sw=2)
    s.text(668, 474, "commit Ω_{7} → Δ_{7}", size=10.5, fill=CONF, anchor="middle")
    s.arrow([(138, 274), (152, 274), (152, 470), (xs[3] + 22, 470), (xs[3] + 22, 362)], stroke=CONS)
    s.label(240, 474, "confirm k", size=10.5, fill=CONS, bg=WORK_T)

    # every op reads the worker's C, then S
    s.arrow([(786, 238), (786, 288)], stroke=MUTED, dash="4 3")
    s.label(786, 262, "each op reads ℓ, then C, then S", size=10.5, fill=MUTED, bg=WORK_T, anchor="end")

    # read pool lane
    s.rect(172, 500, 712, 60, r=14, fill=READP_T)
    thread_badge(s, 184, 518, "read pool · 4 threads", READP)
    for k in range(4):
        s.circle(212 + k * 24, 544, 7, fill=READP, opacity=0.75)
    s.text(360, 534, "unordered reads, on S only", size=11, fill=MUTED)
    s.arrow([(786, 500), (786, 458)], stroke=READP, dash="4 3")
    s.label(806, 486, "&S", size=11, fill=READP, bg=READP_T)

    # replies
    s.arrow([(870, 418), (930, 418), (930, 398)], stroke=INK)
    clients(s, 916, 370)
    s.text(924, 436, "replies", size=11, fill=INK, anchor="middle")
    legend(s, 32, 600, [("block", CONF, CONF_T, "confirmed"), ("block", SPEC, SPEC_T, "speculated"),
                        ("capsule", LIVE, LIVE_T, "container / being built"), ("badge", WORK, None, "thread"),
                        ("badge", POOL, None, "parallel thread")])
    s.save(os.path.join(out, f"{PFX}-x2-lanes-ledger"))


# ---------------------------------------------------------------------------
# X3 — see-through layers, with the threads that own and read them around them
# ---------------------------------------------------------------------------
def x3_layers_threads(out):
    s = Svg(1000, 660)
    s.title("Scalable CRUD", "Architecture + caching · X3 · the layers, and the threads that own and read them")
    top = 236
    # exec pool band behind the per-op caches
    s.rect(120, top - 92, 474, 132, r=16, fill=POOL_T)
    thread_badge(s, 132, top - 74, "exec pool · 4 threads in parallel", POOL)
    # worker band behind C, S and the log
    s.rect(120, top + 58, 850, 268, r=16, fill=WORK_T)
    thread_badge(s, 132, top + 76, "worker · owns the log, C and S", WORK)

    cs, ss = Sheet(190, top + 160), Sheet(190, top + 272)
    ss.draw(s, CONF, CONF_T, S0)
    sheet_label(s, ss, "S", "confirmed", CONF)
    cs.draw(s, SPEC, SPEC_T, C0)
    sheet_label(s, cs, "C", "pending", SPEC)
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

    # o3's read of e: its own ℓ3, then the shared C, then S, where it finds the stale 7
    i = KEYS.index("e")
    x0, y0 = minis[2].centre(i)
    cx, cy = cs.centre(i)
    hx, hy = ss.centre(i)
    ray(s, [(x0, y0), (cx, cy), (hx, hy - 12)], [(x0, y0), (cx, cy)])
    s.text(hx + 14, hy - 30, "o3: e → 7, stale", size=11.5, weight=600, fill=DROP)

    # ω and the log on the worker
    lx, ly = 620, top + 112
    s.text(lx - 10, ly + 34, "write log", size=12.5, weight=600, anchor="end")
    xs = [lx + n * 84 for n in range(4)]
    wcard(s, xs[0], ly, 10, OMEGA10, dashed=True)
    for x, (q, d, _) in zip(xs[1:], reversed(LOG0)):
        wcard(s, x, ly, q, d)
    s.rect(xs[0] - 4, top - 66, 124, 74, r=10, fill=LIVE_T, stroke=LIVE, sw=1.6, shadow=True)
    s.text(xs[0] + 6, top - 47, "ω_{10}", size=14, weight=600, fill=LIVE)
    s.text(xs[0] + 112, top - 47, "on the worker", size=10.5, fill=MUTED, anchor="end")
    strip(s, xs[0] + 6, top - 24, OMEGA10, LIVE, LIVE_T, cell=18, gap=3)
    s.arrow([(minis[3].x + minis[3].w + 26, top - 30), (xs[0] - 6, top - 30)], stroke=LIVE, sw=1.8)
    s.label((minis[3].x + minis[3].w + 26 + xs[0]) / 2, top - 40, "join", size=10.5, fill=LIVE, bg=WHITE)
    s.text(xs[0] + 130, top - 36, "merge ℓ_{1}, ℓ_{4}", size=10.5, fill=MUTED)
    s.text(xs[0] + 130, top - 22, "re-run o2, o3", size=10.5, fill=MUTED)
    s.arrow([(xs[0] + 36, top + 10), (xs[0] + 36, ly - 2)], stroke=SPEC, sw=1.6)
    s.arrow([(xs[2] + 20, ly + 60), (xs[2] + 20, ly + 80), (cs.x + cs.w + 24, ly + 80),
             (cs.x + cs.w + 12, cs.y - 8)], stroke=SPEC, sw=1.6)
    s.label(cs.x + cs.w + 120, ly + 84, "fold: C = Ω_{7} ⊕ Ω_{8} ⊕ Ω_{9}", size=10.5, fill=SPEC, bg=WORK_T)
    s.arrow([(xs[3] + 36, ly + 60), (xs[3] + 36, ss.y - 14), (ss.x + ss.w + 30, ss.y - 14)], stroke=CONF, sw=2)
    s.text(xs[3] + 30, ss.y - 22, "commit Ω_{7} → Δ_{7}", size=11, fill=CONF, anchor="end")

    # consensus and clients, read pool
    s.rect(820, 100, 150, 40, r=12, fill=CONS_T, stroke=CONS, sw=1.3)
    s.text(895, 125, "consensus", size=13, weight=600, fill=CONS, anchor="middle")
    s.arrow([(900, 140), (900, 160), (xs[3] + 36, 160), (xs[3] + 36, ly - 2)], stroke=CONS)
    s.text(908, 160, "confirm k", size=10.5, fill=CONS)
    s.arrow([(820, 112), (360, 112), (360, top - 94)], stroke=CONS)
    s.label(600, 106, "batch k: one op per thread", size=10.5, fill=CONS, bg=WHITE)
    s.rect(120, 568, 300, 40, r=20, fill=READP_T, stroke=READP, sw=1.2)
    s.text(270, 593, "read pool ×4 · unordered reads", size=12, weight=600, fill=READP, anchor="middle")
    s.arrow([(270, 568), (270, ss.y + 2)], stroke=READP, dash="4 3")
    clients(s, 880, 580)
    s.arrow([(xs[3] + 36, ss.y - 14), (xs[3] + 36, 570), (868, 570)], stroke=INK)
    s.text(xs[3] + 44, 560, "replies", size=11, fill=INK)
    legend(s, 32, 644, [("block", CONF, CONF_T, "confirmed"), ("block", SPEC, SPEC_T, "speculated"),
                        ("capsule", LIVE, LIVE_T, "container / being built"), ("badge", WORK, None, "thread"),
                        ("badge", POOL, None, "parallel thread")])
    s.save(os.path.join(out, f"{PFX}-x3-layers-threads"))


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "."
    os.makedirs(out, exist_ok=True)
    for f in (x1_rooms_layers, x2_lanes_ledger, x3_layers_threads):
        f(out)
        print(f.__name__)
