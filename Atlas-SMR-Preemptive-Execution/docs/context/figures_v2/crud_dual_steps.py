"""CRUD cache and dual-state executors: architecture + caching, as an overview and a six-step story.

    python3 crud_dual_steps.py <output-dir>

The same treatment as scalable_steps.py (iteration 11), one figure per executor, in three styles:
  X1   rooms: each thread's room holds the state it owns; states as see-through sheets
  X2   lanes: threads as lanes; the state as a ledger and key strips
  X12  a mix: X2's lanes and key strips, with X1's exact read paths

Batch 10 is the running example (executor_model.py): o1 c ← 5, o2 e ← 8, o3 read e, o4 read b.
Both executors run it in order, so o3 reads o2's 8 and there is no collision to resolve.

CRUD cache, one worker thread:
  1 o1, o2 write into ω10   one container over Ω* and Δ*; writes land only in ω10
  2 o3, o4 read             o3 finds 8 in ω10, o4 finds b deleted in Ω*
  3 append Ω10              ω10 becomes Ω10 and is folded into Ω*; the replies are held
  4 confirm 7 arrives       the oldest decision, Ω7, is the head of the log; Ω8–Ω10 stay speculated
  5 commit Ω7 → Δ7          Δ7's writes go into Δ*; Ω7 leaves the log; Ω* is left as it is
  6 send the replies        batch 7's replies go out; batch 10's wait with Ω10
Dual-state, main + preemptive + confirmed workers:
  1 o1, o2 write Ω*        no container: the batch changes the full copy Ω* in place
  2 o3, o4 read Ω*         o3 finds 8, o4 finds no b
  3 log batch 10            the batch and its replies join the log
  4 confirm 7 arrives       batch 7, the oldest, is the head of the log; 8–10 stay speculated
  5 send replies, forward   the preemptive worker sends batch 7's replies and forwards batch 7
  6 run batch 7 again       the confirmed worker re-executes it on Δ* and drops its replies
Decisions are confirmed in order, so the first confirmation after batch 10 is for 7 (iteration 14).
State is drawn only while it is used: batch 10's own state appears when it exists, and persistent
state that a step does not touch is faded. The read pool is not drawn. In the CRUD cache figures the worker holds
the speculative cache (ω10 as its newest, still open layer, the log and Ω*), and the confirmed state Δ* is drawn
apart from it (iteration 16).
"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "figures"))
from executor_model import BATCH, CRUD, DUAL, KEYS, op_text  # noqa: E402

from caching import Sheet, ray  # noqa: E402
from scalable_arch import clients, room, wcard  # noqa: E402
from scalable_steps import (badge_n, chip, conf_box, confirm_labels, confirm_log, dcard, fade, kv_box,  # noqa: E402
                            progress, shown, spec_group, strip)
from svgkit import (CONF, CONF_T, CONS, CONS_T, INK, LIVE, LIVE_T, MUTED, PANEL, SPEC, SPEC_T,  # noqa: E402
                    WHITE, WORK, WORK_T, Svg, legend, text_width, thread_badge)

PFX = {"crud": "arch-crud", "dual": "arch-dual"}
NAME = {"crud": "CRUD cache", "dual": "Dual-state"}
OPS = BATCH[10]
REPLIES = list(CRUD[1].log[-1][2])                 # ['4', '7', '8', 'none'], the same for both
OMEGA10 = CRUD[1].log[-1][1]                       # {c: 5, e: 8}
S0, C0, LOG0 = CRUD[0].S, CRUD[0].C, CRUD[0].log
C1 = CRUD[1].C
S7 = CRUD[2].S                                     # after Δ7, the oldest decision (Ω* is not refolded)
P0, P1 = DUAL[0].P, DUAL[1].P
assert DUAL[2].S == S7 and DUAL[2].P == P1          # both executors confirm 7 to the same Δ*
DELTA7 = dict(LOG0[0][1])                          # {a: 5, d: 9}
REPLIES7 = [(op_text(op), r) for op, r in zip(BATCH[7], LOG0[0][2])]

# where each op's read lands, in order: o1 and o2 read the old value they overwrite
SOURCE = {"crud": ["Ω^{*}", "Δ^{*}", "ω_{10}", "Ω^{*}"], "dual": ["Ω^{*}", "Ω^{*}", "Ω^{*}", "Ω^{*}"]}
ACTIVE = {1: {0, 1}, 2: {2, 3}}

STEPS = {
    "crud": {
        0: ("overview", "Steps 1–3 speculate batch 10; steps 4–6 confirm the oldest decision, Ω7. Decisions are confirmed in order."),
        1: ("o1, o2 write into ω10",
            "One container runs the ops in order. o1 and o2 write c = 5 and e = 8 into ω10; Ω^{*} and Δ^{*} are only read."),
        2: ("o3, o4 read",
            "o3 finds e = 8 in ω10: running in order, it sees o2's write, so nothing collides. o4 finds b deleted in Ω^{*}."),
        3: ("append Ω10", "ω10 becomes Ω10 and is folded into Ω^{*}. The replies held are 4, 7, 8 and none."),
        4: ("confirm 7 arrives", "Consensus confirms the oldest pending decision, 7: it is the head of the log. Ω8, Ω9, Ω10 stay speculated."),
        5: ("commit Ω7 → Δ7", "Δ7's stored writes go into Δ^{*}: a 1 → 5, d + 9. Ω7 leaves the log; Ω^{*} is not refolded, only a backtrack does that."),
        6: ("send the replies of Δ7", "Batch 7's replies reach their clients. Batch 10's (o3 → 8) wait with Ω10 until 8 and 9 are confirmed."),
    },
    "dual": {
        0: ("overview", "Steps 1–3 speculate batch 10; steps 4–6 confirm the oldest batch, 7. Batches are confirmed in order."),
        1: ("o1, o2 write Ω^{*}",
            "There is no container: batch 10 changes the full copy Ω^{*} in place, c 4 → 5 and e 7 → 8."),
        2: ("o3, o4 read Ω^{*}", "o3 reads e = 8 straight from Ω^{*}, after o2's write; o4 finds no b."),
        3: ("log batch 10", "The batch and its replies join the log: Ω^{*} already holds its effect."),
        4: ("confirm 7 arrives", "Consensus confirms the oldest pending batch, 7: it is the head of the log. 8, 9 and 10 stay speculated."),
        5: ("send replies, forward",
            "The preemptive worker sends batch 7's replies and forwards batch 7. Batch 10's replies (o3 → 8) wait their turn."),
        6: ("run batch 7 again",
            "The confirmed worker runs batch 7 again on Δ^{*}: a 1 → 5, d + 9. The replies it computes are dropped."),
    },
}
# Persistent state each step uses; everything else that exists is faded.
USED = {
    "crud": {0: {"log", "C", "S"}, 1: {"C", "S"}, 2: {"C"}, 3: {"log", "C"}, 4: {"log"}, 5: {"log", "S"},
             6: {"log"}},
    "dual": {0: {"log", "Sp", "S"}, 1: {"Sp"}, 2: {"Sp"}, 3: {"log"}, 4: {"log"}, 5: {"log"}, 6: {"S"}},
}
LEGEND = {
    "crud": [("block", CONF, CONF_T, "confirmed"), ("block", SPEC, SPEC_T, "speculated"),
             ("capsule", LIVE, LIVE_T, "container / being built"), ("badge", WORK, None, "thread")],
    "dual": [("block", CONF, CONF_T, "confirmed"), ("block", SPEC, SPEC_T, "speculated"),
             ("badge", WORK, None, "thread")],
}


def head(s, ex, style, step, h, x_prog):
    name, cap = STEPS[ex][step]
    sub = {"x1": "X1 rooms", "x2": "X2 lanes", "x12": "X1 + X2"}[style]
    if step == 0:
        s.title(NAME[ex], f"Architecture + caching · {sub} · overview of batch 10")
    else:
        s.title(NAME[ex], f"Architecture + caching · {sub} · step {step}: {name}")
        progress(s, x_prog, 50, step, n=6, split=3)
    s.text(32, h - 50, cap, size=12.5, fill=CONF if step else MUTED, weight=600 if step else 400)
    legend(s, 32, h - 22, LEGEND[ex])


def panel(s, x, y, w, h, n, title, rows, thread="W"):
    """The thread's activity in this step: a numbered title and rows of (text, colour, weight) runs."""
    s.rect(x, y, w, h, r=12, fill=WHITE, stroke=WORK, sw=1.2, shadow=True)
    badge_n(s, x + 16, y + 18, n)
    s.text(x + 32, y + 22, title, size=12.5, weight=600)
    thread_badge(s, x + w - 10 - (4 + 3.3 * len(thread)) * 1.6, y + 18, thread, WORK)
    for r, runs in enumerate(rows):
        xx = x + 16
        for t, col, wt in runs:
            s.text(xx, y + 44 + r * 19, t, size=11.5, fill=col, weight=wt)
            xx += text_width(t, 11.5, wt) + (16 if "→" in t else 6)


PANELS = {
    "crud": {
        3: ("append, and fold into Ω^{*}", [[("ω_{10} → Ω_{10}, the newest log entry", SPEC, 600)],
                                        [("Ω^{*} ← Ω^{*} ⊕ Ω_{10}:  c 4 → 5,  e + 8", SPEC, 400)],
                                        [("replies held:  o1 4 · o2 7 ·", MUTED, 400), ("o3 8", CONF, 600),
                                         ("· o4 none", MUTED, 400)]]),
        4: ("confirmation of 7 arrives", [[("the head of the log is Ω_{7}, the oldest: it was speculated", CONF, 600)],
                                          [("its writes and replies are already computed: nothing runs again", MUTED, 400)],
                                          [("Ω_{8}, Ω_{9}, Ω_{10} stay speculated: each is confirmed in turn", MUTED, 400)]]),
        5: ("commit Ω_{7} → Δ_{7}", [[("Δ^{*} ← Δ^{*} ⊕ Δ_{7}:  a 1 → 5,  d + 9", CONF, 600)],
                                     [("Ω_{7} leaves the log", MUTED, 400)],
                                     [("Ω^{*} is not refolded: Ω_{7}'s writes stay in it, and now match Δ^{*}", MUTED, 400)]]),
        6: ("send the replies of Δ_{7}", [[("batch 7:", MUTED, 400)] + [(f"{op} → {r}", INK, 400) for op, r in REPLIES7],
                                          [("batch 10's replies (o3 → 8) wait with Ω_{10}", CONF, 400)],
                                          [("until Δ_{8} and Δ_{9} are committed first", MUTED, 400)]]),
    },
    "dual": {
        3: ("log batch 10 and its replies", [[("batch 10 joins the log, with its replies", SPEC, 600)],
                                             [("replies held:  o1 4 · o2 7 ·", MUTED, 400), ("o3 8", CONF, 600),
                                              ("· o4 none", MUTED, 400)],
                                             [("Ω^{*} already holds batch 10's effect", MUTED, 400)]]),
        4: ("confirmation of 7 arrives", [[("the head of the log is batch 7, the oldest", CONF, 600)],
                                          [("its replies were computed on Ω^{*}", MUTED, 400)],
                                          [("batches 8, 9 and 10 stay speculated, in order", MUTED, 400)]]),
        5: ("send the replies, forward batch 7", [[("batch 7:", MUTED, 400)] + [(f"{op} → {r}", INK, 400)
                                                                             for op, r in REPLIES7],
                                                  [("batch 7 leaves the log and goes to the confirmed worker", MUTED, 400)],
                                                  [("batch 10's replies (o3 → 8) wait with Ω_{10}", CONF, 400)]]),
        6: ("run batch 7 again on Δ^{*}", [[("Δ^{*}:  a 1 → 5,  d + 9", CONF, 600)],
                                        [("the replies it computes are dropped", MUTED, 400)],
                                        [("Δ^{*} catches up with Ω^{*} to batch 7", MUTED, 400)]]),
    },
}


def ops_box(s, ex, x, y, w, step, horizontal=False, cell_w=None):
    """Batch 10's ops: in a container (CRUD) or applied straight to Ω* (dual)."""
    done = step >= 3 and step != 0
    if ex == "crud":
        edge, fill, dash, title = LIVE, LIVE_T, "5 4", "container · reads ω, then Ω^{*}, then Δ^{*}"
    else:
        edge, fill, dash, title = SPEC, SPEC_T, None, "batch 10 · runs in place on Ω^{*}"
    h = 82 if horizontal else 142
    s.rect(x, y, w, h, r=14, fill=fill, stroke=edge, sw=1.6, dash=dash, opacity=0.5 if done else None)
    s.text(x + 14, y + 22, title, size=12, weight=600, fill=edge, opacity=0.5 if done else None)
    ran = {0, 1} if step == 1 else set(range(4))
    for i, op in enumerate(OPS):
        active = i in ACTIVE.get(step, ())
        col = INK if active else (MUTED if (done or i not in ran) else INK)
        rep = f"→ {REPLIES[i]}" if i in ran else "…"
        src = f"from {SOURCE[ex][i]}" if i in ran else ""
        if horizontal:
            cw = cell_w
            cx, cy = x + 12 + i * cw, y + 38
            if active:
                s.rect(cx - 4, cy - 6, cw - 6, 40, r=8, fill=WHITE, stroke=edge, sw=1.2)
            s.text(cx + 4, cy + 8, f"o{i + 1} · {op_text(op)}", size=11.5, weight=600 if active else 400, fill=col)
            s.text(cx + 4, cy + 25, f"{rep}  {src}", size=10.5, fill=CONF if (active and i == 2) else MUTED)
        else:
            ry = y + 44 + i * 24
            if active:
                s.rect(x + 8, ry - 13, w - 16, 21, r=6, fill=WHITE, stroke=edge, sw=1.2)
            s.text(x + 16, ry + 2, f"o{i + 1} · {op_text(op)}", size=12, weight=600 if active else 400, fill=col)
            s.text(x + w - 16, ry + 2, f"{rep}", size=11.5, fill=CONF if (active and i == 2) else col,
                   anchor="end", weight=600 if active else 400)
            s.text(x + w - 60, ry + 2, src, size=10.5, fill=MUTED, anchor="end")
    return h


def omega_box(s, x, y, step, w=150, h=74):
    if step not in (0, 1, 2, 3):
        return
    s.rect(x, y, w, h, r=10, fill=LIVE_T, stroke=LIVE, sw=1.6, shadow=True)
    s.text(x + 10, y + 19, "ω_{10}", size=14, weight=600, fill=LIVE)
    strip(s, x + 12, y + 42, OMEGA10, LIVE, cell=20, gap=4, rings={"c", "e"} if step == 1 else
          ({"e"} if step == 2 else ()), ring_colour=INK)


def bcard(s, x, y, q, w=64, h=58, dashed=False):
    """A dual-state log entry: the batch itself (its ops) and its replies."""
    s.rect(x, y, w, h, r=8, fill=WHITE if dashed else SPEC_T, stroke=SPEC, sw=1.4, shadow=not dashed)
    s.text(x + 8, y + 17, f"Ω_{{{q}}}", size=12.5, weight=600, fill=SPEC)
    for r, op in enumerate(BATCH[q][:2]):
        s.text(x + 8, y + 32 + r * 12, op_text(op), size=9.5, fill=INK)
    if len(BATCH[q]) > 2:
        s.text(x + w - 6, y + 17, f"+{len(BATCH[q]) - 2}", size=9.5, fill=MUTED, anchor="end")


def log_row(s, ex, xs, y, step, w=72):
    """The write log: Ω7–Ω9 (+ Ω10 once appended); during confirmation, ghosts of what is already in Δ*."""
    card = (lambda x, q, d: wcard(s, x, y, q, d, w=w)) if ex == "crud" else (lambda x, q, d: bcard(s, x, y, q, w=w))
    olds = [(q, d) for q, d, _ in reversed(LOG0)]
    if step <= 3:
        if step in (0, 3):
            card(xs[0], 10, OMEGA10)
            if step == 3:
                s.rect(xs[0] - 4, y - 4, w + 8, 66, r=11, fill="none", stroke=INK, sw=1.6)
        for x, (q, d) in zip(xs[1:], olds):
            card(x, q, d)
        return
    def cardq(x, q):
        card(x, q, OMEGA10 if q == 10 else dict(LOG0[q - 7][1]))

    def d7(x):
        if ex == "crud":
            dcard(s, x, y, 7, DELTA7, w=w)
        else:                                # confirmed, on its way to the confirmed worker
            s.rect(x, y, w, 58, r=8, fill=CONF_T, stroke=CONF, sw=1.6, shadow=True)
            s.text(x + 8, y + 17, "Δ_{7}", size=12.5, weight=600, fill=CONF)
            s.text(x + 8, y + 34, "batch 7", size=9.5, fill=INK)
            s.text(x + 8, y + 46, "+ replies", size=9.5, fill=MUTED)
    confirm_log(s, xs, y, step, 4, cardq, d7, w=w)


def log_labels(s, xs, y, step, w=72):
    if step <= 3:
        x_new = xs[0] if step in (0, 3) else xs[1]
        s.text(x_new, y, "newest", size=10.5, fill=MUTED)
        s.text(xs[3] + w, y, "oldest", size=10.5, fill=MUTED, anchor="end")
        s.arrow([(x_new + 50, y - 4), (xs[3] + w - 52, y - 4)], stroke="#b9bfca", sw=1)
    else:
        confirm_labels(s, xs, y, step, 6, w=w)


def reply_chips(s, x, y):
    """Batch 7's replies, the ones that leave once 7 is confirmed."""
    for op, r in REPLIES7:
        x += chip(s, x, y, f"{op} → {r}", CONF, CONF_T) + 8


# ---------------------------------------------------------------------------
# CRUD cache — X1 rooms
# ---------------------------------------------------------------------------
def crud_x1(out, step):
    """The worker holds the speculative cache (ω10 over Ω*, and the log); Δ* is drawn apart from it."""
    ex, H = "crud", 860
    s = Svg(1000, H)
    head(s, ex, "x1", step, H, 812)
    s.rect(30, 96, 940, 34, r=17, fill=CONS_T, stroke=CONS, sw=1.3)
    s.text(500, 118, "consensus", size=13, weight=600, fill=CONS, anchor="middle")
    room(s, 30, 150, 940, 466, "worker", WORK, WORK_T, "the only thread · holds the speculative cache")
    ops_box(s, ex, 46, 188, 360, step)

    # the speculative cache: batch 10's open write set ω10 as its newest layer, then Ω*, plus the log
    spec_group(s, 46, 404, 908, 200)
    ws, cs = Sheet(150, 488, slot=46, dx=38, dy=-24), Sheet(150, 572, slot=46, dx=38, dy=-24)
    if step in (0, 1, 2, 3):
        ws.draw(s, LIVE, LIVE_T, OMEGA10, size=12)
        s.text(132, ws.y - 6, "ω_{10}", size=18, weight=600, fill=LIVE, anchor="end")
        s.text(132, ws.y + 10, "batch 10, open", size=10, fill=MUTED, anchor="end")
        for k in ({1: ("c", "e"), 2: ("e",)}.get(step, ())):
            s.poly(ws.tile(KEYS.index(k)), fill="none", stroke=INK, sw=2)
    cs.draw(s, SPEC, SPEC_T, C1 if step >= 3 else C0, size=12)
    s.text(132, cs.y - 6, "Ω^{*}", size=18, weight=600, fill=SPEC, anchor="end")
    s.text(132, cs.y + 10, "Ω_{7} ⊕ … ⊕ Ω_{10}" if step >= 3 else "Ω_{7} ⊕ Ω_{8} ⊕ Ω_{9}", size=10,
           fill=MUTED, anchor="end")
    if step == 3:
        for k in ("c", "e"):
            s.poly(cs.tile(KEYS.index(k)), fill="none", stroke=INK, sw=2)
    if "C" not in USED[ex][step]:
        fade(s, 60, cs.y - 30, 420, 40)
    xs = [604, 688, 772, 856]
    log_row(s, ex, xs, 432, step)
    log_labels(s, xs, 508, step)
    s.text(xs[1] + 4, 528, "write log", size=12, weight=600)
    if "log" not in USED[ex][step]:
        fade(s, (xs[0] if step >= 4 else xs[1]) - 6, 424, 254 + (84 if step >= 4 else 0), 112)

    # the confirmed state, apart from the cache
    conf_box(s, 30, 628, 940, 96)
    ss = Sheet(150, 708, slot=46, dx=38, dy=-24)
    ss.draw(s, CONF, CONF_T, S7 if step >= 5 else S0, size=12)
    s.text(132, ss.y - 6, "Δ^{*}", size=18, weight=600, fill=CONF, anchor="end")
    if step == 5:
        for k in ("a", "d"):
            s.poly(ss.tile(KEYS.index(k)), fill="none", stroke=INK, sw=2)
    if "S" not in USED[ex][step]:
        fade(s, 60, ss.y - 30, 420, 40, WHITE)

    # step 1: the writes land in ω10; step 2: the reads go ω10, then Ω*, then Δ*
    if step == 1:
        for k in ("c", "e"):
            kx, ky = ws.centre(KEYS.index(k))
            s.arrow([(kx, 332), (kx, ky - 10)], stroke=LIVE, sw=1.8)
        s.text(ws.centre(4)[0] + 10, 372, "o1, o2 write into ω_{10}", size=10.5, weight=600, fill=LIVE)
    if step == 2:
        kx, ky = ws.centre(KEYS.index("e"))
        ray(s, [(kx, 332), (kx, ky - 10)])
        s.text(kx + 10, 372, "o3: e = 8 from ω_{10}", size=10.5, weight=600, fill=CONF)
        bx, by = cs.centre(KEYS.index("b"))
        ray(s, [(bx, 332), (bx, by - 10)], [ws.centre(KEYS.index("b"))])
        s.text(bx + 8, 356, "o4: b is ⊥ in Ω^{*}", size=10.5, fill=MUTED)
    if step in (0, 3):
        s.arrow([(426, 468), (602, 468)], stroke=SPEC, sw=1.6)
        s.label(514, 458, "append", size=10.5, fill=SPEC, bg=WHITE)
        fx = xs[2] + 36 if step == 0 else xs[0] + 50
        s.arrow([(fx, 492), (fx, 556), (432, 556)], stroke=SPEC, sw=1.6)
        s.label(560, 552, "fold", size=10.5, fill=SPEC, bg=WHITE)

    if step in PANELS[ex]:
        t, rows = PANELS[ex][step]
        panel(s, 422, 188, 528, 96, step, t, rows)
    if step == 4:
        s.arrow([(962, 130), (962, 461), (930, 461)], stroke=CONS, sw=1.8)
        s.label(962, 146, "confirm 7", size=11, fill=CONS, bg=WHITE, weight=600, anchor="end")
    if step in (0, 5):
        s.arrow([(928, 476), (946, 476), (946, ss.y - 12), (ss.x + ss.w + 26, ss.y - 12)], stroke=CONF,
                sw=2.2 if step == 5 else 2)
        if step == 5:
            s.text(938, 620, "Δ_{7} into Δ^{*}", size=11.5, fill=CONF, weight=600, anchor="end")
    if step == 6:
        s.arrow([(950, 236), (962, 236), (962, 734)], stroke=INK, sw=1.8)
    if step == 0:
        s.arrow([(xs[3] + 36, 130), (xs[3] + 36, 430)], stroke=CONS)
        s.label(xs[3] + 36, 146, "confirm 7", size=11, fill=CONS, bg=WHITE)
        s.arrow([(946, ss.y - 12), (946, 734)], stroke=INK)
        for n, (bx_, by_) in enumerate([(424, 214), (424, 262), (514, 488), (xs[3] + 56, 162), (930, 600),
                                        (930, 728)], start=1):
            badge_n(s, bx_, by_, n)
    if step in (0, 1):
        s.arrow([(300, 130), (300, 186)], stroke=CONS)
        s.label(350, 146, "batch 10", size=11, fill=CONS, bg=WHITE)

    s.rect(30, 736, 940, 40, r=20, fill="#eef0f3")
    clients(s, 420, 758)
    if step == 6:
        reply_chips(s, 480, 758)
    s.save(os.path.join(out, f"{PFX[ex]}-x1-step{step}"))


# ---------------------------------------------------------------------------
# CRUD cache — X2 lanes and the X12 mix
# ---------------------------------------------------------------------------
def crud_x2(out, step, mix=False):
    ex, H = "crud", 690
    style = "x12" if mix else "x2"
    s = Svg(1000, H)
    head(s, ex, style, step, H, 812)
    s.rect(30, 230, 108, 58, r=12, fill=CONS_T, stroke=CONS, sw=1.4)
    s.text(84, 257, "consensus", size=13.5, weight=600, fill=CONS, anchor="middle")
    s.text(84, 274, "orders batches", size=10.5, fill=MUTED, anchor="middle")
    s.rect(156, 96, 744, 528, r=18, fill=PANEL)
    s.rect(172, 112, 712, 396, r=14, fill=WORK_T)
    thread_badge(s, 184, 130, "worker", WORK)
    s.text(250, 134, "the only thread" + (" · holds the speculative cache: ω, the log and Ω^{*}" if mix else ""),
           size=11, fill=MUTED)

    ops_box(s, ex, 184, 150, 688, step, horizontal=True, cell_w=170)
    spec_group(s, 180, 244, 696, 148, tx=192, ty=386)
    omega_box(s, 192, 262, step)
    xs = [356, 436, 516, 596]
    log_row(s, ex, xs, 262, step)
    log_labels(s, xs, 344, step)
    if "log" not in USED[ex][step]:
        fade(s, (xs[0] if step >= 4 else xs[1]) - 4, 256, 252 + (80 if step >= 4 else 0), 96)
    c_sub = {3: "Ω^{*} ⊕ Ω_{10}", 4: "Ω_{7} ⊕ … ⊕ Ω_{10}", 5: "Ω_{7} ⊕ … ⊕ Ω_{10}",
             6: "Ω_{7} ⊕ … ⊕ Ω_{10}"}.get(step, "Ω_{7} ⊕ Ω_{8} ⊕ Ω_{9}")
    kv_box(s, 702, 258, 168, 76, "Ω^{*}", C1 if step >= 3 else C0, SPEC, SPEC_T, c_sub,
           rings={"c", "e"} if step == 3 else ())
    if "C" not in USED[ex][step]:
        fade(s, 696, 252, 180, 88)

    conf_box(s, 172, 520, 712, 92)
    kv_box(s, 702, 528, 168, 76, "Δ^{*}", S7 if step >= 5 else S0, CONF, CONF_T, "confirmed",
           rings={"a", "d"} if step == 5 else ())
    if "S" not in USED[ex][step]:
        fade(s, 696, 522, 180, 88, WHITE)

    if step in PANELS[ex]:
        t, rows = PANELS[ex][step]
        panel(s, 184, 404, 460, 96, step, t, rows)
    if step == 1:
        s.arrow([(260, 232), (260, 260)], stroke=LIVE, sw=1.8)
    if step in (0, 3):
        s.arrow([(342, 291), (354, 291)], stroke=SPEC, sw=1.6)
        s.arrow([(668, 291), (700, 291)], stroke=SPEC, sw=1.6)
    if step in (1, 2):
        if mix:
            if step == 1:
                cx = 714 + 2 * 30 + 12
                ray(s, [(270, 232), (270, 238), (cx, 238), (cx, 296)])
                ex_ = 714 + 4 * 30 + 12
                ray(s, [(440, 232), (440, 241), (ex_, 241), (ex_, 566)], [(ex_, 310)])
            else:
                ox = 204 + 4 * 24 + 10
                ray(s, [(610, 232), (610, 240), (ox, 240), (ox, 302)])
                bx = 714 + 30 + 12
                ray(s, [(780, 232), (780, 246), (bx, 246), (bx, 296)])
        else:
            s.arrow([(786, 232), (786, 256)], stroke=MUTED, dash="4 3")
            s.label(780, 246, "each op reads ω, then Ω^{*}, then Δ^{*}", size=10.5, fill=MUTED, bg=WORK_T, anchor="end")

    if step == 4:
        s.arrow([(138, 274), (148, 274), (148, 252), (xs[3] + 36, 252), (xs[3] + 36, 260)], stroke=CONS, sw=1.8)
        s.label(300, 252, "confirm 7", size=10.5, fill=CONS, bg=WHITE, weight=600)
    if step in (0, 5):
        s.arrow([(658, 322), (658, 548), (700, 548)], stroke=CONF, sw=2.2 if step == 5 else 2)
        s.text(664, 470, "Δ_{7} into Δ^{*}" if step == 5 else "on confirm 7: commit Ω_{7} → Δ_{7}", size=10.5,
               fill=CONF, weight=600 if step == 5 else 400)
    if step == 6:
        s.arrow([(644, 452), (930, 452), (930, 352)], stroke=INK, sw=1.8)
    if step == 0:
        s.arrow([(870, 566), (930, 566), (930, 352)], stroke=INK)
        for n, (bx_, by_) in enumerate([(184, 150), (524, 150), (348, 278), (xs[3] + 60, 250), (648, 466),
                                        (944, 470)], start=1):
            badge_n(s, bx_, by_, n)
    if step in (0, 1):
        s.arrow([(138, 244), (148, 244), (148, 190), (182, 190)], stroke=CONS)
    clients(s, 916, 326)
    if step == 6:
        for i, (op, r) in enumerate(REPLIES7):
            chip(s, 890, 466 + i * 22, f"{op} → {r}", CONF, CONF_T)
    s.save(os.path.join(out, f"{PFX[ex]}-{style}-step{step}"))


# ---------------------------------------------------------------------------
# Dual-state — X1 rooms
# ---------------------------------------------------------------------------
def dual_x1(out, step):
    ex, H = "dual", 760
    s = Svg(1000, H)
    head(s, ex, "x1", step, H, 812)
    s.rect(30, 96, 940, 34, r=17, fill=CONS_T, stroke=CONS, sw=1.3)
    s.text(500, 118, "consensus", size=13, weight=600, fill=CONS, anchor="middle")
    room(s, 30, 156, 600, 58, "main", WORK, WORK_T, "routes each request to a worker")
    room(s, 30, 230, 600, 400, "preemptive worker", WORK, WORK_T, "owns Ω^{*} and the log")
    room(s, 650, 156, 320, 474, "confirmed worker", WORK, WORK_T, "owns Δ^{*}")

    ops_box(s, ex, 46, 268, 310, step)
    xs = [372, 436, 500, 564]
    log_row(s, ex, xs, 268, step, w=58)
    log_labels(s, xs, 344, step, w=58)
    s.text(xs[1], 362, "log of batches", size=12, weight=600)
    if "log" not in USED[ex][step]:
        fade(s, (xs[0] if step >= 4 else xs[1]) - 6, 260, 196 + (64 if step >= 4 else 0), 110)

    sp = Sheet(110, 600, slot=46, dx=38, dy=-24)
    sp.draw(s, SPEC, SPEC_T, P1, size=12, opacity=0.95)      # in place from step 1 on
    s.text(96, sp.y - 2, "Ω^{*}", size=20, weight=600, fill=SPEC, anchor="end")
    if step == 1:
        for k in ("c", "e"):
            s.poly(sp.tile(KEYS.index(k)), fill="none", stroke=INK, sw=2)
    if "Sp" not in USED[ex][step]:
        fade(s, 46, sp.y - 30, 400, 38)
    ss = Sheet(690, 600, slot=40, dx=36, dy=-24)
    ss.draw(s, CONF, CONF_T, S7 if step == 6 else S0, size=12)
    s.text(676, ss.y - 2, "Δ^{*}", size=20, weight=600, fill=CONF, anchor="end")
    if step == 6:
        for k in ("a", "d"):
            s.poly(ss.tile(KEYS.index(k)), fill="none", stroke=INK, sw=2)
    if "S" not in USED[ex][step]:
        fade(s, 662, ss.y - 30, 300, 38)

    # steps 1–2: the batch reads and writes S_p itself
    if step in (1, 2):
        rows = (0, 1) if step == 1 else (2, 3)
        for j, i in enumerate(rows):
            k = OPS[i][1]
            cx, cy = sp.centre(KEYS.index(k))
            ry = 268 + 44 + i * 24 - 4
            pts = [(356, ry), (368 + j * 8, ry), (368 + j * 8, 528 + j * 10), (cx, 528 + j * 10), (cx, cy - 10)]
            if OPS[i][0] == "w":
                s.arrow(pts, stroke=SPEC, sw=1.8, round_=0)
            else:
                ray(s, pts)
        s.text(390, 520, "writes change Ω^{*} in place" if step == 1 else "o3 reads e = 8 from Ω^{*}",
               size=10.5, weight=600, fill=SPEC if step == 1 else CONF)

    if step in PANELS[ex] and step != 6:
        t, rows = PANELS[ex][step]
        panel(s, 46, 430, 568, 96, step, t, rows)
    if step == 6:
        t, rows = PANELS[ex][step]
        panel(s, 662, 300, 296, 96, step, t, rows)
    if step in (0, 1):
        s.arrow([(330, 130), (330, 156)], stroke=CONS)
        s.label(384, 143, "batch 10", size=11, fill=CONS, bg=WHITE)
        s.arrow([(330, 214), (330, 266)], stroke=WORK)
    if step == 4:
        s.arrow([(xs[3] + 29, 130), (xs[3] + 29, 156)], stroke=CONS, sw=1.8)
        s.label(xs[3] - 30, 143, "confirm 7", size=11, fill=CONS, bg=WHITE, weight=600)
        s.arrow([(xs[3] + 29, 214), (xs[3] + 29, 262)], stroke=WORK, sw=1.8)
    if step == 5:
        s.arrow([(614, 478), (660, 478)], stroke=CONF, sw=2)
        s.text(664, 468, "batch 7", size=10.5, fill=CONF, weight=600)
        s.arrow([(330, 526), (330, 646)], stroke=INK, sw=1.8)
    if step == 0:
        s.arrow([(614, 478), (660, 478)], stroke=CONF, sw=2)
        s.arrow([(330, 526), (330, 646)], stroke=INK)
        for n, (bx, by) in enumerate([(342, 300), (342, 348), (xs[0] - 10, 262), (xs[3] + 60, 248),
                                      (316, 560), (640, 360)], start=1):
            badge_n(s, bx, by, n)

    s.rect(30, 648, 940, 40, r=20, fill="#eef0f3")
    clients(s, 420, 670)
    if step == 5:
        reply_chips(s, 480, 670)
    s.save(os.path.join(out, f"{PFX[ex]}-x1-step{step}"))


# ---------------------------------------------------------------------------
# Dual-state — X2 lanes and the X12 mix
# ---------------------------------------------------------------------------
def dual_x2(out, step, mix=False):
    ex, H = "dual", 680
    style = "x12" if mix else "x2"
    s = Svg(1000, H)
    head(s, ex, style, step, H, 812)
    s.rect(30, 290, 108, 58, r=12, fill=CONS_T, stroke=CONS, sw=1.4)
    s.text(84, 317, "consensus", size=13.5, weight=600, fill=CONS, anchor="middle")
    s.text(84, 334, "orders batches", size=10.5, fill=MUTED, anchor="middle")
    s.rect(156, 96, 744, 510, r=18, fill=PANEL)

    s.rect(172, 112, 712, 44, r=12, fill=WORK_T)
    thread_badge(s, 184, 134, "main", WORK)
    s.text(232, 138, "routes each request to a worker", size=11, fill=MUTED)

    s.rect(172, 168, 712, 300, r=14, fill=WORK_T)
    thread_badge(s, 184, 186, "preemptive worker", WORK)
    s.text(470, 190, "owns Ω^{*} and the log" if mix else "", size=11, fill=MUTED)
    ops_box(s, ex, 184, 204, 250, step)
    xs = [452, 516, 580, 644]
    log_row(s, ex, xs, 204, step, w=58)
    log_labels(s, xs, 280, step, w=58)
    if "log" not in USED[ex][step]:
        fade(s, (xs[0] if step >= 4 else xs[1]) - 4, 198, 196 + (64 if step >= 4 else 0), 92)
    kv_box(s, 702, 300, 168, 76, "Ω^{*}", P1, SPEC, SPEC_T, "full copy", rings={"c", "e"} if step == 1 else ())
    if "Sp" not in USED[ex][step]:
        fade(s, 696, 294, 180, 88)

    s.rect(172, 480, 712, 110, r=14, fill=WORK_T)
    thread_badge(s, 184, 498, "confirmed worker", WORK)
    s.text(320, 502, "owns Δ^{*}" if mix else "", size=11, fill=MUTED)
    kv_box(s, 702, 500, 168, 76, "Δ^{*}", S7 if step == 6 else S0, CONF, CONF_T,
           "confirmed", rings={"a", "d"} if step == 6 else ())
    if "S" not in USED[ex][step]:
        fade(s, 696, 494, 180, 88)

    if step in PANELS[ex] and step != 6:
        t, rows = PANELS[ex][step]
        panel(s, 184, 362, 500, 96, step, t, rows)
    if step == 6:
        t, rows = PANELS[ex][step]
        panel(s, 184, 506, 500, 76, step, t, rows[:2])

    if step in (1, 2):
        if mix:
            rows = (0, 1) if step == 1 else (2, 3)
            for j, i in enumerate(rows):
                k = OPS[i][1]
                cx = 714 + KEYS.index(k) * 30 + 12
                ry = 204 + 44 + i * 24 - 4
                pts = [(434, ry), (442 + j * 6, ry), (442 + j * 6, 286 + j * 6), (cx, 286 + j * 6), (cx, 338)]
                if OPS[i][0] == "w":
                    s.arrow(pts, stroke=SPEC, sw=1.8, round_=0)
                else:
                    ray(s, pts)
        else:
            s.arrow([(434, 300), (700, 330)], stroke=SPEC, sw=1.8)
            s.label(560, 300, "in place" if step == 1 else "reads Ω^{*}", size=10.5, fill=SPEC, bg=WORK_T)
    if step in (0, 1):
        s.arrow([(410, 156), (410, 202)], stroke=WORK)
        s.label(410, 176, "batch 10", size=10.5, fill=CONS, bg=PANEL)
    if step == 4:
        s.arrow([(xs[3] + 29, 156), (xs[3] + 29, 200)], stroke=WORK, sw=1.8)
        s.label(xs[3] + 29, 176, "confirm 7", size=10.5, fill=CONS, bg=PANEL, weight=600)
    if step == 5:
        s.arrow([(560, 458), (560, 494)], stroke=CONF, sw=2)
        s.label(612, 476, "batch 7", size=10.5, fill=CONF, bg=PANEL, weight=600)
        s.arrow([(684, 420), (930, 420), (930, 352)], stroke=INK, sw=1.8)
        for i, (op, r) in enumerate(REPLIES7):
            chip(s, 890, 436 + i * 22, f"{op} → {r}", CONF, CONF_T)
    if step == 0:
        s.arrow([(560, 458), (560, 494)], stroke=CONF, sw=2)
        for n, (bx, by) in enumerate([(420, 214), (420, 262), (xs[0] - 8, 198), (xs[3] + 29, 176),
                                      (xs[3] + 64, 400), (690, 520)], start=1):
            badge_n(s, bx, by, n)
    s.arrow([(138, 304), (150, 304), (150, 134), (170, 134)], stroke=CONS)
    clients(s, 916, 326)
    s.save(os.path.join(out, f"{PFX[ex]}-{style}-step{step}"))


def gif(out, ex, style, n=6, seconds=3.0, width=1400):
    import shutil
    import subprocess
    if not shutil.which("magick"):
        print("magick not found: skipping the GIF")
        return
    frames = [os.path.join(out, f"{PFX[ex]}-{style}-step{k}.png") for k in range(1, n + 1)]
    subprocess.run(["magick", "-delay", str(int(seconds * 100)), "-loop", "0", *frames, "-resize", f"{width}x",
                    "+dither", "-colors", "256", "-layers", "Optimize",
                    os.path.join(out, f"{PFX[ex]}-{style}-steps.gif")], check=True)


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "."
    os.makedirs(out, exist_ok=True)
    for step in range(7):
        crud_x1(out, step)
        crud_x2(out, step)
        crud_x2(out, step, mix=True)
        dual_x1(out, step)
        dual_x2(out, step)
        dual_x2(out, step, mix=True)
    for ex in ("crud", "dual"):
        for style in ("x1", "x2", "x12"):
            gif(out, ex, style)
    print("crud, dual × x1, x2, x12: steps 0-6, and a GIF of steps 1-6 for each")
