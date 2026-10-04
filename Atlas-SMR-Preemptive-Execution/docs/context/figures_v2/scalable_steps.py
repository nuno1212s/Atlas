"""Scalable CRUD executor: architecture + caching, as an overview and as a five-step collision story.

    python3 scalable_steps.py <output-dir>

Three styles (iteration 07), each drawn as an overview (step 0) and as steps 1–5 of batch 10:
  X1   rooms: each thread's room holds the state it owns; C over S as see-through sheets
  X2   lanes: exec pool, worker and read pool as lanes; the worker's state as a ledger and key strips
  X12  a mix: X2's lanes and key strips, with X1's exact read paths and ownership notes

Batch 10 (executor_model.py): o1 c ← 5, o2 e ← 8, o3 read e, o4 read b. The collision is on e: o2
writes it and o3 reads it in parallel, so o3 sees the stale 7. Every step shows where that collision is:
  1 run in parallel       each op in its own container on T1–T4, writing only its ℓ
  2 collision check       the worker compares accesses: e is W by o2 and R by o3, so they collide
  3 merge non-colliders   ℓ1 and ℓ4 go into ω10 in batch order; o2 and o3 wait
  4 re-run colliders      o2 then o3 run again on the worker over C ⊕ ω10; o3 now reads 8
  5 append Ω10            ω10 becomes Ω10, C folds it in, and o3's held reply is 8
"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "figures"))
from executor_model import CRUD, KEYS, SCALABLE, TOMB, TRACE10, op_text  # noqa: E402

from caching import Sheet, ray  # noqa: E402
from scalable_arch import clients, room, wcard  # noqa: E402
from svgkit import (CONF, CONF_T, CONS, CONS_T, DROP, DROP_T, INK, LIVE, LIVE_T, MUTED, PANEL, POOL,  # noqa: E402
                    POOL_T, READP, READP_T, SPEC, SPEC_T, WHITE, WORK, WORK_T, Svg, capsule, legend,
                    text_width, thread_badge)

PFX = "arch-scal"
S0, C0, LOG0 = CRUD[0].S, CRUD[0].C, CRUD[0].log
C1 = SCALABLE[1].C
OPS = TRACE10.ops
COLL = TRACE10.colliders                     # {1, 2}: o2 and o3
OMEGA = {0: TRACE10.delta, 1: {}, 2: {}, 3: dict(TRACE10.bd_pass1), 4: TRACE10.delta, 5: TRACE10.delta}
NEW_IN_OMEGA = {3: {"c"}, 4: {"e"}}

STEPS = {
    0: ("overview", "Steps 1–5 below each show where the collision on e stands."),
    1: ("run in parallel",
        "o2 writes e = 8 into its own ℓ2 while o3 reads e: o3 cannot see ℓ2, so it finds the stale 7 in S."),
    2: ("collision check",
        "The worker compares every op's accesses: e is written by o2 and read by o3, so o2 and o3 collide."),
    3: ("merge non-colliders",
        "ℓ1 and ℓ4 go into ω10 in batch order. o2 and o3 are set aside: their ℓ and replies are dropped."),
    4: ("re-run colliders",
        "o2, then o3, run again on the worker over C ⊕ ω10. o3 now reads e = 8: the collision is resolved."),
    5: ("append Ω10",
        "ω10 becomes Ω10 and is folded into C. The reply held for o3 is 8, never the stale 7."),
}
LEGEND = [("block", CONF, CONF_T, "confirmed"), ("block", SPEC, SPEC_T, "speculated"),
          ("capsule", LIVE, LIVE_T, "container / being built"), ("badge", WORK, None, "thread"),
          ("badge", POOL, None, "parallel thread"), ("badge", DROP, None, "collision")]


def shown(v):
    return "⊥" if v == TOMB else v


# ---------------------------------------------------------------------------
# shared pieces
# ---------------------------------------------------------------------------
def strip(s, x, y, values, colour, cell=18, gap=3, keys=True, size=11, rings=(), ring_colour=DROP,
          opacity=None):
    for i, k in enumerate(KEYS):
        cx = x + i * (cell + gap)
        if keys:
            s.text(cx + cell / 2, y - 4, k, size=9.5, fill=MUTED, anchor="middle")
        if k in values:
            s.rect(cx, y, cell, cell, r=4, fill=colour, stroke=colour, opacity=opacity)
            s.text(cx + cell / 2, y + cell / 2 + size * 0.36, shown(values[k]), size=size, weight=600,
                   fill=WHITE, anchor="middle", opacity=opacity)
        else:
            s.rect(cx, y, cell, cell, r=4, fill="none", stroke=colour, sw=0.8, dash="2 2", opacity=0.6)
        if k in rings:
            s.rect(cx - 3, y - 3, cell + 6, cell + 6, r=6, fill="none", stroke=ring_colour, sw=2)
    return x + 5 * (cell + gap) - gap


def progress(s, x, y, step):
    """Five numbered dots; the current step is filled."""
    s.text(x - 12, y + 4, "step", size=11, fill=MUTED, anchor="end")
    for n in range(1, 6):
        cx = x + (n - 1) * 30
        if n == step:
            s.circle(cx, y, 12, fill=INK)
            s.text(cx, y + 4.5, str(n), size=12, weight=600, fill=WHITE, anchor="middle")
        else:
            s.circle(cx, y, 11, fill=WHITE, stroke="#c4c9d2", sw=1.4)
            s.text(cx, y + 4.5, str(n), size=11.5, fill=MUTED, anchor="middle")
        if n < 5:
            s.line(cx + 12, y, cx + 18, y, stroke="#c4c9d2", sw=1.4)


def badge_n(s, x, y, n, colour=INK):
    s.circle(x, y, 10, fill=colour)
    s.text(x, y + 4, str(n), size=11, weight=600, fill=WHITE, anchor="middle")


def collision_chip(s, x, y, text="collision on e"):
    w = text_width(text, 11, 600) + 34
    s.rect(x, y - 11, w, 22, r=11, fill=DROP_T, stroke=DROP, sw=1.4)
    s.circle(x + 12, y, 7, fill=DROP)
    s.line(x + 9, y - 3, x + 15, y + 3, stroke=WHITE, sw=1.6)
    s.line(x + 9, y + 3, x + 15, y - 3, stroke=WHITE, sw=1.6)
    s.text(x + 24, y + 4, text, size=11, weight=600, fill=DROP)
    return w


def chip(s, x, y, text, colour, tint):
    w = text_width(text, 10.5, 600) + 14
    s.rect(x, y - 9, w, 18, r=9, fill=tint, stroke=colour, sw=1.1)
    s.text(x + w / 2, y + 3.5, text, size=10.5, weight=600, fill=colour, anchor="middle")
    return w


def container(s, x, y, w, h, pos, step, cell=15, gap=3):
    """An op's container at a given step: its thread, op, private ℓ, reply, and collision state."""
    local, _, reply = TRACE10.lanes[pos]
    collider = pos in COLL
    done = step >= 3
    edge = DROP if collider and step >= 2 else LIVE
    s.rect(x, y, w, h, r=14, fill=LIVE_T, stroke=edge, sw=1.8 if edge == DROP else 1.5, dash="4 3",
           opacity=0.55 if done and step != 0 else None)
    bw = thread_badge(s, x + 10, y + 17, f"T{pos + 1}", POOL)
    s.text(x + 18 + bw, y + 22, f"o{pos + 1} · {op_text(OPS[pos])}", size=12, weight=600, fill=LIVE)
    narrow = w < 200
    cy = y if narrow else y + 17                  # narrow containers carry the chip on their top edge
    if step >= 2 and collider:
        cw = text_width("collider", 10.5, 600) + 14
        chip(s, x + w - cw - 8, cy, "collider", DROP, DROP_T)
    elif step >= 3 and not collider:
        cw = text_width("merged", 10.5, 600) + 14
        chip(s, x + w - cw - 8, cy, "merged", LIVE, WHITE)
    s.text(x + 12, y + h - 13, f"ℓ_{{{pos + 1}}}", size=12, weight=600, fill=LIVE)
    ring = {"e"} if pos == 1 and step in (0, 1, 2) else set()
    sx = x + 32
    strip(s, sx, y + h - 13 - cell * 0.75, local, LIVE, cell=cell, gap=gap, keys=False, size=10, rings=ring,
          opacity=0.35 if collider and done and step != 0 else None)
    stale = collider and OPS[pos][0] == "r"
    if collider and done and step != 0:
        txt, col, wt = ("dropped" if narrow else "reply dropped"), DROP, 400
    else:
        txt, col, wt = (f"→ {reply}" + (" stale" if stale else "")), (DROP if stale else MUTED), (600 if stale else 400)
    s.text(x + w - 10, y + h - 13, txt, size=11, fill=col, anchor="end", weight=wt)
    if stale and step in (0, 1, 2):
        tw = text_width(txt, 11, 600)
        s.rect(x + w - 14 - tw, y + h - 27, tw + 8, 20, r=6, fill="none", stroke=DROP, sw=1.6)
    if collider and done and step != 0:
        s.line(sx - 2, y + h - 13 - cell * 0.25, sx + 5 * (cell + gap) - gap + 2, y + h - 13 - cell * 0.25,
               stroke=DROP, sw=1.4, opacity=0.8)


def omega_box(s, x, y, step, w=124, h=74):
    vals = OMEGA[step]
    s.rect(x, y, w, h, r=10, fill=LIVE_T, stroke=LIVE, sw=1.6, shadow=True,
           dash="5 4" if not vals else None)
    s.text(x + 10, y + 19, "ω_{10}", size=14, weight=600, fill=LIVE)
    s.text(x + w - 8, y + 19, "empty" if not vals else "on the worker", size=10.5, fill=MUTED, anchor="end")
    strip(s, x + 10, y + 42, vals, LIVE, rings=NEW_IN_OMEGA.get(step, ()), ring_colour=INK)


def log_row(s, xs, y, step, w=72):
    if step in (0, 5):
        wcard(s, xs[0], y, 10, TRACE10.delta, w=w)
        if step == 5:
            s.rect(xs[0] - 4, y - 4, w + 8, 66, r=11, fill="none", stroke=INK, sw=1.6)
    else:
        s.rect(xs[0], y, w, 58, r=8, fill=WHITE, stroke=SPEC, sw=1.2, dash="4 4", opacity=0.6)
        s.text(xs[0] + w / 2, y + 34, "Ω_{10}?", size=12, fill=SPEC, anchor="middle", opacity=0.6)
    for x, (q, d, _) in zip(xs[1:], reversed(LOG0)):
        wcard(s, x, y, q, d, w=w)


def step_panel(s, x, y, w, h, step):
    """What the worker does in this step, with the collision always named."""
    if step not in (2, 3, 4, 5):
        return
    s.rect(x, y, w, h, r=12, fill=WHITE, stroke=WORK, sw=1.2, shadow=True)
    badge_n(s, x + 16, y + 18, step)
    s.text(x + 32, y + 22, {2: "collision check", 3: "merge the non-colliders, in batch order",
                            4: "re-run the colliders, in batch order, over C ⊕ ω_{10}",
                            5: "append, and fold into C"}[step], size=12.5, weight=600)
    thread_badge(s, x + w - 26, y + 18, "W", WORK)
    r1, r2, r3 = y + 42, y + 60, y + 78
    if step == 2:
        s.text(x + 16, r1, "c   W by o1", size=11.5, fill=MUTED)
        s.text(x + 120, r1, "b   R by o4", size=11.5, fill=MUTED)
        cw = collision_chip(s, x + 12, r2 - 3, "e   W by o2  ×  R by o3")
        s.text(x + 22 + cw, r2 + 1, "→ o2 and o3 collide", size=11.5, weight=600, fill=DROP)
        s.text(x + 16, r3 + 2, "their ℓ and replies will be dropped", size=11, fill=MUTED)
    elif step == 3:
        s.text(x + 16, r1, "ℓ_{1}  c ← 5   → ω_{10}", size=11.5, fill=LIVE, weight=600)
        s.text(x + 200, r1, "ℓ_{4}  nothing to merge", size=11.5, fill=MUTED)
        collision_chip(s, x + 12, r2 + 4, "o2, o3 wait for step 4")
    elif step == 4:
        cw = (w - 44) / 2
        capsule(s, x + 14, y + 36, cw, 26, "o2 again · e ← 8", size=11)
        capsule(s, x + 30 + cw, y + 36, cw, 26, "o3 again · read e", size=11)
        s.text(x + 14 + cw / 2, r3 + 2, "writes e = 8 into ω_{10}", size=11, fill=LIVE, anchor="middle")
        s.text(x + 30 + cw * 1.5, r3 + 2, "→ 8 from ω_{10}, was 7", size=11, weight=600, fill=CONF,
               anchor="middle")
        s.arrow([(x + 14 + cw, y + 49), (x + 30 + cw, y + 49)], stroke=LIVE, sw=1.4)
    else:
        s.text(x + 16, r1, "ω_{10} → Ω_{10}, the newest log entry", size=11.5, fill=SPEC, weight=600)
        s.text(x + 16, r2, "C ← C ⊕ Ω_{10}:  c 4 → 5,  e + 8", size=11.5, fill=SPEC)
        s.text(x + 16, r3 + 2, "replies held:  o1 4 · o2 7 ·", size=11.5, fill=MUTED)
        s.text(x + 16 + text_width("replies held:  o1 4 · o2 7 · ", 11.5), r3 + 2, "o3 8", size=11.5, weight=600,
               fill=CONF)
        s.text(x + 16 + text_width("replies held:  o1 4 · o2 7 · o3 8 ", 11.5), r3 + 2, "· o4 none", size=11.5,
               fill=MUTED)


def head(s, style, step, h):
    name, cap = STEPS[step]
    sub = {"x1": "X1 rooms", "x2": "X2 lanes", "x12": "X1 + X2"}[style]
    if step == 0:
        s.title("Scalable CRUD", f"Architecture + caching · {sub} · overview of batch 10")
    else:
        s.title("Scalable CRUD", f"Architecture + caching · {sub} · step {step}: {name}")
        progress(s, 840, 44, step)
    s.text(32, h - 50, cap, size=12.5, fill=DROP if step in (1, 2, 3) else (CONF if step in (4, 5) else MUTED),
           weight=600 if step else 400)
    legend(s, 32, h - 22, LEGEND)


# ---------------------------------------------------------------------------
# X1 — rooms
# ---------------------------------------------------------------------------
def x1(out, step):
    H = 750
    s = Svg(1000, H)
    head(s, "x1", step, H)
    s.rect(30, 96, 940, 34, r=17, fill=CONS_T, stroke=CONS, sw=1.3)
    s.text(500, 118, "consensus", size=13, weight=600, fill=CONS, anchor="middle")

    room(s, 30, 158, 380, 362, "exec pool", POOL, POOL_T, "4 threads, in parallel")
    for pos in range(4):
        container(s, 46, 196 + pos * 80, 348, 64, pos, step)
    if step in (0, 1, 2):
        collision_chip(s, 236, 348)
    if step in (0, 1):
        s.arrow([(150, 130), (150, 156)], stroke=CONS)
        s.label(214, 146, "batch k: one op per thread", size=11, fill=CONS, bg=WHITE)

    room(s, 430, 158, 540, 446, "worker", WORK, WORK_T, "owns ω, the log, C and S")
    omega_box(s, 446, 196, step)
    xs = [596, 680, 764, 848]
    log_row(s, xs, 196, step)
    s.text(xs[0], 272, "newest", size=10.5, fill=MUTED)
    s.text(xs[3] + 72, 272, "oldest", size=10.5, fill=MUTED, anchor="end")
    s.arrow([(xs[0] + 50, 268), (xs[3] + 20, 268)], stroke="#b9bfca", sw=1)
    s.text(xs[1] + 4, 290, "write log", size=12, weight=600)

    cs, ss = Sheet(500, 436, slot=50, dx=40, dy=-26), Sheet(500, 560, slot=50, dx=40, dy=-26)
    ss.draw(s, CONF, CONF_T, S0, size=13)
    cs.draw(s, SPEC, SPEC_T, C1 if step == 5 else C0, size=13)
    s.text(474, cs.y - 2, "C", size=20, weight=600, fill=SPEC, anchor="end")
    s.text(474, ss.y - 2, "S", size=20, weight=600, fill=CONF, anchor="end")
    if step == 5:
        for k in ("c", "e"):
            pts = cs.tile(KEYS.index(k))
            s.poly(pts, fill="none", stroke=INK, sw=2)

    # the worker's activity in this step
    step_panel(s, 446, 300, 484, 96, step)
    if step == 2:
        s.arrow([(410, 340), (444, 340)], stroke=LIVE, sw=1.8)
        s.label(427, 330, "join", size=10.5, fill=LIVE, bg=WHITE)
    if step in (0, 3):
        s.arrow([(410, 230), (444, 230)], stroke=LIVE, sw=1.8)
        s.label(427, 220, "join", size=10.5, fill=LIVE, bg=WHITE)
    if step == 4:
        s.arrow([(508, 298), (508, 272)], stroke=LIVE, sw=1.8)
    if step in (0, 5):
        s.arrow([(570, 225), (594, 225)], stroke=SPEC, sw=1.6)
        s.arrow([(xs[2] + 36, 256), (xs[2] + 36, 300), (cs.x + cs.w + 26, cs.y - 30)] if step == 0 else
                [(xs[0] + 36, 256), (xs[0] + 36, 296), (938, 296), (938, 404), (cs.x + cs.w + 26, cs.y - 30)],
                stroke=SPEC, sw=1.6)
    if step == 0:
        s.label(xs[2] + 26, 314, "fold: C = Ω_{7} ⊕ … ⊕ Ω_{9}", size=10.5, fill=SPEC, bg=WORK_T, anchor="end")
        # the steps, numbered where they happen
        badge_n(s, 396, 176, 1)
        badge_n(s, 456, 286, 2)
        badge_n(s, 480, 286, 3)
        badge_n(s, 504, 286, 4)
        s.text(520, 290, "check, merge, re-run", size=10.5, fill=MUTED)
        badge_n(s, 582, 210, 5)

    # reads cross from the pool into the worker's C and S
    if step in (0, 1):
        i = KEYS.index("e")
        ex_, ey = cs.centre(i)
        sx_, sy = ss.centre(i)
        ray(s, [(394, 372), (ex_, 372), (ex_, sy - 11)], [(ex_, ey)])
        s.text(sx_ + 14, sy - 24, "o3 finds e = 7, stale", size=11, weight=600, fill=DROP)
        i = KEYS.index("b")
        bx, by = cs.centre(i)
        ray(s, [(394, 468), (420, 468), (420, 386), (bx, 386), (bx, by - 11)])
        s.text(446, 362, "&C, &S: read-only, shared by all four threads", size=10.5, fill=MUTED)

    # commit and replies belong to the overview only
    if step == 0:
        s.arrow([(xs[3] + 36, 130), (xs[3] + 36, 194)], stroke=CONS)
        s.label(xs[3] + 36, 146, "confirm k", size=11, fill=CONS, bg=WHITE)
        s.arrow([(xs[3] + 72, 230), (950, 230), (950, ss.y - 12), (ss.x + ss.w + 24, ss.y - 12)], stroke=CONF,
                sw=2)
        s.text(944, 470, "commit", size=11.5, fill=CONF, weight=600, anchor="end")
        s.text(944, 485, "Ω_{7} → Δ_{7}", size=11.5, fill=CONF, anchor="end")
        s.arrow([(950, ss.y - 12), (950, 622)], stroke=INK)
        s.text(944, 612, "replies of Δ_{7}", size=11, fill=INK, anchor="end")

    room(s, 30, 540, 380, 64, "read pool", READP, READP_T, "4 threads, unordered reads")
    for k in range(4):
        s.circle(330 + k * 18, 586, 6, fill=READP, opacity=0.75)
    s.arrow([(410, 572), (ss.x + 4, 572), (ss.x + 14, ss.y - 4)], stroke=READP, dash="4 3")
    s.rect(30, 624, 940, 40, r=20, fill="#eef0f3")
    clients(s, 480, 646)
    s.save(os.path.join(out, f"{PFX}-x1-step{step}"))


# ---------------------------------------------------------------------------
# X2 and the X1 + X2 mix — lanes
# ---------------------------------------------------------------------------
def kv_box(s, x, y, w, h, name, values, colour, tint, sub=None, rings=()):
    s.rect(x, y, w, h, r=12, fill=tint, stroke=colour, sw=1.6, shadow=True)
    s.text(x + 12, y + 22, name, size=16, weight=600, fill=colour)
    if sub:
        s.text(x + 34, y + 21, sub, size=10.5, fill=MUTED)
    strip(s, x + 12, y + 40, values, colour, cell=24, gap=6, size=12, rings=rings, ring_colour=INK)


def x2(out, step, mix=False):
    H = 680
    s = Svg(1000, H)
    style = "x12" if mix else "x2"
    head(s, style, step, H)
    s.rect(30, 230, 108, 58, r=12, fill=CONS_T, stroke=CONS, sw=1.4)
    s.text(84, 257, "consensus", size=13.5, weight=600, fill=CONS, anchor="middle")
    s.text(84, 274, "orders batches", size=10.5, fill=MUTED, anchor="middle")
    s.rect(156, 96, 744, 494, r=18, fill=PANEL)

    s.rect(172, 112, 712, 126, r=14, fill=POOL_T)
    pool_label = "exec pool · 4 threads in parallel" + (" · each writes only its own ℓ" if mix else "")
    thread_badge(s, 184, 130, pool_label, POOL)
    for pos in range(4):
        container(s, 184 + pos * 174, 150, 164, 76, pos, step, cell=12, gap=2)
    if step in (0, 1, 2):
        collision_chip(s, 534, 130)
    if step in (0, 1):
        s.arrow([(138, 244), (152, 244), (152, 188), (182, 188)], stroke=CONS)
        s.label(150, 210, "batch k", size=10.5, fill=CONS, bg=PANEL, anchor="end")

    s.rect(172, 254, 712, 244, r=14, fill=WORK_T)
    thread_badge(s, 184, 272, "worker", WORK)
    if mix:
        s.text(360, 278, "owns ω, the log, C and S", size=11, fill=MUTED)
    omega_box(s, 184, 296, step)
    xs = [338, 420, 502, 584]
    log_row(s, xs, 296, step)
    s.text(xs[0], 374, "newest", size=10.5, fill=MUTED)
    s.arrow([(xs[0] + 50, 370), (xs[3] + 6, 370)], stroke="#b9bfca", sw=1)
    s.text(xs[3] + 4, 388, "oldest", size=10.5, fill=MUTED, anchor="end")
    kv_box(s, 702, 286, 168, 76, "C", C1 if step == 5 else C0, SPEC, SPEC_T,
           "C ⊕ Ω_{10}" if step == 5 else "Ω_{7} ⊕ Ω_{8} ⊕ Ω_{9}", rings={"c", "e"} if step == 5 else ())
    kv_box(s, 702, 376, 168, 76, "S", S0, CONF, CONF_T, "confirmed")
    if step in (0, 5):
        s.arrow([(308, 325), (336, 325)], stroke=SPEC, sw=1.6)
        s.arrow([(658, 310), (700, 310)], stroke=SPEC, sw=1.6)
        s.label(679, 300, "fold", size=10.5, fill=SPEC, bg=WORK_T)

    step_panel(s, 184, 398, 500, 92, step)
    if step == 2:
        s.arrow([(322, 238), (322, 396)], stroke=LIVE, sw=1.8)
        s.label(322, 262, "join", size=10.5, fill=LIVE, bg=WORK_T)
    if step in (0, 3):
        s.arrow([(246, 238), (246, 294)], stroke=LIVE, sw=1.8)
        s.label(246, 262, "join", size=10.5, fill=LIVE, bg=WORK_T)
    if step == 4:
        s.arrow([(246, 396), (246, 372)], stroke=LIVE, sw=1.8)
    if step == 0:
        badge_n(s, 172 + 700, 130, 1)
        for n, bx in ((2, 200), (3, 224), (4, 248)):
            badge_n(s, bx, 400, n)
        s.text(264, 404, "check, merge, re-run: on the worker", size=10.5, fill=MUTED)
        badge_n(s, 322, 310, 5)

    # reads: every op reads ℓ, then C, then S
    if step in (0, 1):
        if mix:
            i = KEYS.index("e")
            cx = 714 + i * 30 + 12
            ray(s, [(640, 226), (640, 262), (cx, 262), (cx, 418)], [(cx, 338)])
            s.text(870, 472, "o3 finds e = 7, stale", size=11, weight=600, fill=DROP, anchor="end")
            s.text(652, 254, "o3 reads e: ℓ_{3} → C → S", size=10.5, fill=MUTED)
        else:
            s.arrow([(786, 238), (786, 284)], stroke=MUTED, dash="4 3")
            s.label(786, 262, "each op reads ℓ, then C, then S", size=10.5, fill=MUTED, bg=WORK_T, anchor="end")

    if step == 0:
        s.arrow([(xs[3] + 60, 356), (xs[3] + 60, 384), (700, 384)], stroke=CONF, sw=2)
        s.text(694, 412, "on confirm k: commit Ω_{7} → Δ_{7}", size=10.5, fill=CONF, anchor="end")
        s.arrow([(870, 414), (930, 414), (930, 394)], stroke=INK)
        s.text(924, 432, "replies", size=11, fill=INK, anchor="middle")
    clients(s, 916, 366)

    s.rect(172, 512, 712, 62, r=14, fill=READP_T)
    thread_badge(s, 184, 530, "read pool · 4 threads", READP)
    for k in range(4):
        s.circle(212 + k * 24, 556, 7, fill=READP, opacity=0.75)
    s.text(360, 548, "unordered reads, on S only", size=11, fill=MUTED)
    s.arrow([(724, 512), (724, 454)], stroke=READP, dash="4 3")
    s.save(os.path.join(out, f"{PFX}-{style}-step{step}"))


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "."
    os.makedirs(out, exist_ok=True)
    for step in range(6):
        x1(out, step)
        x2(out, step)
        x2(out, step, mix=True)
    print("x1, x2, x12: steps 0-5")
