"""Scalable CRUD executor: the two caching layers, in route B (containers) and route C (table).

    python3 scalable_flow.py <output-dir>

Cross-batch layer: the log of speculated decisions Ω and their fold C, exactly as in the CRUD
cache (Ωk is decision k while speculated, Δk once confirmed; every entry drawn here is speculated).
Intra-batch layer (rebuilt for every batch, scalable_crud/pending_state.rs:197-307):
  ① each operation runs in parallel through its own local cache ℓ over C over S
  ② accesses are compared; a Write or Delete with anything on the same key collides
  ③ non-colliders' ℓ are merged, in batch order, into batch_delta (ω10, the batch's write set)
  ④ colliders re-run sequentially, in batch order, over current_view = C ⊕ ω10
  ⑤ ω10 becomes Ω10 (speculated): appended to the log and merged into C
Commit and backtrack are the CRUD cache's (crud_flow.py figures).
"""
import os
import sys

import matplotlib
from matplotlib.patches import Polygon

from executor_model import KEYS, SCALABLE, TOMB, TRACE10, op_text
from flow_common import (W as D, D as Dc, w as om, PEND_FILL, WORK_FILL, badge, cell, curved, key_row, marks_for,
                         parrow, region, tx, val)
from paperfig import BODY, COLUMN, DARK, FULL, INK, LIGHT, MID, PAPER, SMALL, TINY, Canvas

PFX = "preemptive-context-scalable-flow"
T = TRACE10
C0, C1, S0 = SCALABLE[0].C, SCALABLE[1].C, SCALABLE[0].S


def row5(cv, x, y, d, kind, pitch=15, w=12.5, h=9.5, marks=None, gone=None, keys=False):
    xs = [x + j * pitch + pitch / 2 for j in range(5)]
    col = dict(zip(KEYS, xs))
    for k, v in d.items():
        cell(cv, col[k], y, w, h, kind, v, (marks or {}).get(k), size=TINY - 0.3)
    for k, v in (gone or {}).items():
        cell(cv, col[k], y, w, h, kind, v, "gone", size=TINY - 0.3)
    if keys:
        key_row(cv, xs, y - 9, size=TINY - 1)
    return col


def op_short(op):
    return op_text(op).replace("read ", "rd ")


# ---------------------------------------------------------------------------
# B — the two layers as nested containers, the batch pipeline left to right
# ---------------------------------------------------------------------------
def fig_b(out):
    cv = Canvas(FULL, 250)
    ops = T.ops

    # --- cross-batch layer ---------------------------------------------------
    region(cv, 0, 0, FULL, 66, "pend", "CROSS-BATCH LAYER · the log and its fold C (pending, discardable)")
    for j, (q, d, _) in enumerate(SCALABLE[0].log):
        x = 8 + j * 40
        cv.box(x, 14, 34, 30, fill=PAPER, edge=MID, lw=0.5, r=1.4)
        cv.text(x + 17, 20, D(q), size=TINY, ha="center")
        for r_, (k, v) in enumerate(sorted(d.items())):
            cv.text(x + 17, 28 + r_ * 7, tx(f"{k} ← {v}"), size=TINY - 0.8, ha="center", color=DARK)
    cv.text(132, 30, tx("fold ⊕ →"), size=TINY, ha="center", color=DARK)
    cv.text(150, 12, "$C$ before batch 10", size=TINY - 0.4, color=DARK)
    cC = row5(cv, 150, 34, C0, "C", keys=True)
    # after
    cv.box(330, 14, 40, 30, fill=PAPER, edge=INK, lw=1.2, r=1.4)
    cv.text(350, 20, D(10), size=TINY, ha="center", weight="bold")
    for r_, (k, v) in enumerate(sorted(T.delta.items())):
        cv.text(350, 28 + r_ * 7, tx(f"{k} ← {v}"), size=TINY - 0.8, ha="center")
    cv.text(384, 30, tx("⊕ →"), size=TINY, ha="center", color=DARK)
    chg, _ = marks_for(C0, C1)
    row5(cv, 400, 34, C1, "C", marks=chg, keys=True)
    cv.text(400, 12, tx(rf"$C$ after: $C$ ⊕ {D(10)}"), size=TINY - 0.4, color=DARK)

    # --- intra-batch layer ---------------------------------------------------
    wy0, wy1 = 74, 200
    region(cv, 0, wy0, FULL, wy1 - wy0, "work", "INTRA-BATCH LAYER · batch 10 workspace (built for this batch, then discarded)")
    heads = [(6, "1", "parallel · T1–T4"), (134, "2", "collision check"), (222, "3", "merge non-colliders"),
             (300, "4", "re-run · worker"), (424, "5", tx(rf"{D(10)} = {om(10)}"))]
    cv.box(3, 99, 127, 72, fill=PAPER, edge=LIGHT, lw=0, hatch="//////", r=1.5, z=0.4)
    for x, n, t in heads:
        badge(cv, x, 92, n, r=3.8)
        cv.text(x + 10, 92, t, size=TINY + 0.2, weight="bold", bbox=dict(boxstyle="square,pad=0.15", fc=WORK_FILL, ec="none"))

    # ① lanes
    lane_y = [108, 126, 144, 162]
    for pos, (op, y) in enumerate(zip(ops, lane_y)):
        local, accesses, reply = T.lanes[pos]
        collides = pos in T.colliders
        cv.box(6, y - 7.5, 120, 15, fill=PAPER, edge=INK if not collides else INK, lw=0.5 if not collides else 0.9,
               ls="-" if not collides else (0, (2, 1.2)), r=1.4)
        cv.text(10, y, f"o{pos + 1}", size=TINY, weight="bold")
        cv.text(22, y, tx(op_short(op)), size=TINY - 0.3)
        cv.text(62, y, "ℓ:", size=TINY - 0.6, color=MID)
        if local:
            k, v = next(iter(local.items()))
            cell(cv, 76, y, 13, 9.5, "L", v, size=TINY - 0.4)
        else:
            cv.text(76, y, "–", size=TINY, ha="center", color=MID)
        k, kind = accesses[0]
        cv.text(94, y, f"{kind} {k}", size=TINY - 0.4, ha="center", color=DARK)
        stale = pos in T.colliders and ops[pos][0] == "r"
        cv.text(112, y, f"→ {tx(reply)}", size=TINY - 0.4, ha="center", color=MID if stale else INK,
                style="italic" if stale else "normal")
    cv.text(66, 178, "each unit reads ℓ, then C, then S", size=TINY - 0.6, ha="center", color=DARK)
    cv.text(66, 186, "o3 saw e = 7 from S: stale", size=TINY - 0.6, ha="center", color=DARK, style="italic")

    # ② collision matrix over the keys the batch touched
    touched = []
    for _, accesses, _ in T.lanes:
        for k, _ in accesses:
            if k not in touched:
                touched.append(k)
    mx = {pos: 160 + pos * 14 for pos in range(len(ops))}
    my = {k: 116 + r_ * 13 for r_, k in enumerate(touched)}
    for pos, x in mx.items():
        cv.text(x, 106, f"o{pos + 1}", size=TINY - 0.6, ha="center", color=DARK)
    for k, y in my.items():
        cv.text(142, y, f"${k}$", size=TINY, color=MID)
        cv.line([(150, y + 6.5), (212, y + 6.5)], lw=0.25, color=LIGHT)
    for pos, (_, accesses, _) in enumerate(T.lanes):
        for k, kind in accesses:
            cv.text(mx[pos], my[k], kind, size=TINY, ha="center",
                    weight="bold" if k in T.collided_on else "normal")
    for k, poss in T.collided_on.items():
        xs = [mx[p_] for p_ in poss]
        cv.box(min(xs) - 6, my[k] - 5.5, max(xs) - min(xs) + 12, 11, fill="none", edge=INK, lw=1.0,
               ls=(0, (2, 1.2)), r=1.5, z=6)
    cv.text(174, 160, "W × R on e", size=TINY - 0.4, ha="center", color=DARK)
    cv.text(174, 168, "colliders: " + ", ".join(f"o{p_ + 1}" for p_ in sorted(T.colliders)),
            size=TINY - 0.4, ha="center", weight="bold")

    # ③ pass 1
    non = [p_ for p_ in range(len(ops)) if p_ not in T.colliders]
    cv.text(226, 108, "in batch order: " + ", ".join(f"o{p_ + 1}" for p_ in non), size=TINY - 0.6, color=DARK)
    cv.text(224, 126, om(10), size=TINY, weight="bold")
    row5(cv, 236, 126, T.bd_pass1, "d", pitch=11, w=9.5, keys=True)
    cv.text(226, 142, "o4 read only;\nnothing to merge", size=TINY - 0.8, color=DARK, linespacing=1.1, va="top")

    # ④ pass 2
    cv.text(322, 106, tx(rf"view = $C$ ⊕ {om(10)}"), size=TINY - 0.4, color=DARK)
    row5(cv, 304, 118, T.view, "C", pitch=14, w=11.5, h=9)
    for j, (pos, local, reply) in enumerate(T.reexec):
        y = 138 + j * 17
        cv.box(304, y - 7, 106, 14, fill=PAPER, edge=INK, lw=0.6, r=1.4)
        cv.text(308, y, f"o{pos + 1}", size=TINY, weight="bold")
        cv.text(320, y, tx(op_short(ops[pos])), size=TINY - 0.3)
        if local:
            k, v = next(iter(local.items()))
            cv.text(372, y, tx(rf"{om(10)} ⊕ {k}={v}"), size=TINY - 0.4, color=DARK)
        else:
            was = T.lanes[pos][2]
            cv.text(368, y, tx(f"→ {reply}"), size=TINY, weight="bold")
            cv.text(386, y, f"(was {was})", size=TINY - 0.6, color=MID)
    cv.arrow((357, 123), (357, 130.5), lw=0.6)
    cv.text(304, 176, "one at a time, in batch order;\neach sees the ones before it",
            size=TINY - 0.8, color=DARK, linespacing=1.1, va="top")

    # ⑤ result
    row5(cv, 428, 126, T.delta, "D", pitch=14, w=11.5, keys=True)
    cv.text(428, 142, "replies, in order:", size=TINY - 0.6, color=DARK)
    cv.text(428, 150, "  ".join(f"o{i + 1}:{r}" for i, r in enumerate(T.replies)), size=TINY - 1, color=DARK)

    # flow between the steps
    for x1, x2 in ((127, 133), (215, 221), (293, 299), (413, 423)):
        cv.arrow((x1, 135), (x2, 135), lw=0.9)
    # cross-layer arrows
    parrow(cv, [(cC["a"], 39), (cC["a"], 70), (100, 70), (100, 99)], lw=0.6, ls=(0, (2, 1.2)))
    cv.text(104, 60, "read by every unit", size=TINY - 0.8, color=DARK)
    parrow(cv, [(cC["d"] + 7, 34), (318, 34), (318, 112.5)], lw=0.6)
    cv.text(262, 60, "copied into the view", size=TINY - 0.8, color=DARK)
    parrow(cv, [(470, 121), (470, 70), (350, 70), (350, 44.5)], lw=1.1, head=(3.6, 1.8))
    cv.text(412, 60, "append and merge", size=TINY - 0.8, color=DARK)

    # --- confirmed ------------------------------------------------------------
    region(cv, 0, 208, FULL, 36, "conf", "CONFIRMED · S is read on a miss, never written during speculation")
    cS = row5(cv, 150, 229, S0, "S", keys=False)
    key_row(cv, list(cS.values()), 238, size=TINY - 1)
    parrow(cv, [(140, 229), (14, 229), (14, 170)], lw=0.6, ls=(0, (2, 1.2)))
    parrow(cv, [(232, 229), (400, 229), (400, 172)], lw=0.6, ls=(0, (2, 1.2)))
    cv.text(20, 223, "miss", size=TINY - 0.8, color=DARK)
    cv.text(406, 223, "miss", size=TINY - 0.8, color=DARK)
    cv.save(os.path.join(out, f"{PFX}-b-two-layers"))


# ---------------------------------------------------------------------------
# C — one table; the intra-batch layer as extra column groups
# ---------------------------------------------------------------------------
def fig_c(out):
    cv = Canvas(FULL, 160)
    pitch, gap = 27, 8
    cols = []                                     # (id, group)
    cols += [("S", "conf")]
    cols += [(7, "cross"), (8, "cross"), (9, "cross"), ("C", "cross")]
    cols += [("o0", "work"), ("o1", "work"), ("o2", "work"), ("o3", "work"), ("bd", "work"),
             ("r1", "work"), ("r2", "work"), ("D10", "work")]
    cols += [("C1", "after")]
    x, xs, prev_group = 22, {}, None
    for cid, grp in cols:
        if prev_group and grp != prev_group:
            x += gap
        xs[cid] = x + pitch / 2
        x += pitch
        prev_group = grp
    left = {g: min(xs[c] for c, gg in cols if gg == g) - pitch / 2 for g in ("conf", "cross", "work", "after")}
    right = {g: max(xs[c] for c, gg in cols if gg == g) + pitch / 2 for g in ("conf", "cross", "work", "after")}
    ry = {k: 66 + r_ * 12 for r_, k in enumerate(KEYS)}
    top, bot = 4, ry["e"] + 7

    cv.box(left["cross"], top, right["cross"] - left["cross"], bot - top, fill=PEND_FILL, edge="none", lw=0, r=1.5, z=0.2)
    cv.box(left["after"], top, right["after"] - left["after"], bot - top, fill=PEND_FILL, edge="none", lw=0, r=1.5, z=0.2)
    cv.box(left["work"], top, right["work"] - left["work"], bot - top, fill=WORK_FILL, edge=DARK, lw=0.5,
           ls=(0, (1.2, 1.2)), r=1.5, z=0.2)
    cv.text((left["conf"] + right["conf"]) / 2, 10, "confirmed", size=TINY - 0.4, ha="center", weight="bold")
    cv.text((left["cross"] + right["cross"]) / 2, 10, "cross-batch layer", size=TINY - 0.4, ha="center", weight="bold")
    cv.text((left["work"] + right["work"]) / 2, 10, "intra-batch layer: batch 10 workspace", size=TINY - 0.4,
            ha="center", weight="bold")
    cv.text((left["after"] + right["after"]) / 2, 10, "after", size=TINY - 0.4, ha="center", weight="bold")
    subs = [(("o0", "o3"), "1  parallel · T1–T4"), (("bd", "bd"), "3  pass 1"), (("r1", "r2"), "4  pass 2 · worker"),
            (("D10", "D10"), "5")]
    cv.box(xs["o0"] - pitch / 2 + 1, 15.5, xs["o3"] - xs["o0"] + pitch - 2, 9, fill="none", edge=MID, lw=0,
           hatch="//////", r=1, z=0.25)
    for (a, b_), t in subs:
        cv.text((xs[a] + xs[b_]) / 2, 20, t, size=TINY - 0.8, ha="center", color=DARK,
                bbox=dict(boxstyle="square,pad=0.15", fc=WORK_FILL, ec="none"))
    labels = {"S": "$S$", "C": "$C$", "bd": om(10), "D10": D(10), "C1": r"$C$", 7: D(7), 8: D(8), 9: D(9),
              "o0": "o1", "o1": "o2", "o2": "o3", "o3": "o4", "r1": "o2", "r2": "o3"}
    for cid, _ in cols:
        cv.text(xs[cid], 30, labels[cid], size=TINY, ha="center", weight="bold" if cid in ("D10",) else "normal")
    for pos in range(4):
        cv.text(xs[f"o{pos}"], 39, tx(op_short(T.ops[pos])), size=TINY - 1, ha="center", color=DARK)
        mark = "×" if pos in T.colliders else "·"
        cv.text(xs[f"o{pos}"], 48, mark, size=TINY + 0.5 if pos in T.colliders else TINY, ha="center",
                weight="bold")
    cv.text(left["work"] - 1, 48, "2  collides:", size=TINY - 1, ha="right", color=DARK)
    for j, (pos, _, _) in enumerate(T.reexec):
        cv.text(xs[f"r{j + 1}"], 39, tx(op_short(T.ops[pos])), size=TINY - 1, ha="center", color=DARK)
    cv.line([(0, top), (FULL, top)], lw=0.8)
    cv.line([(0, 54), (FULL, 54)], lw=0.45)
    cv.line([(0, bot), (FULL, bot)], lw=0.8)

    log = {q: d for q, d, _ in SCALABLE[0].log}
    chg, _ = marks_for(C0, C1)
    for k in KEYS:
        y = ry[k]
        cv.text(4, y, f"${k}$", size=SMALL, color=MID)
        if k in S0:
            cv.text(xs["S"], y, S0[k], size=TINY, ha="center")
        for q in (7, 8, 9):
            if k in log[q]:
                cv.text(xs[q], y, val(log[q][k]), size=TINY, ha="center",
                        color=MID if k in {kk for qq in (8, 9) if qq > q for kk in log[qq]} else INK)
        if k in C0:
            cell(cv, xs["C"], y, 15, 9, "C", C0[k])
        for pos in range(4):
            local, accesses, reply = T.lanes[pos]
            if k in local:
                cell(cv, xs[f"o{pos}"], y, 15, 9, "L", local[k])
            elif (k, "R") in accesses:
                stale = pos in T.colliders
                cv.text(xs[f"o{pos}"], y, "r " + tx(reply), size=TINY - 0.6, ha="center",
                        color=MID if stale else DARK, style="italic")
        if k in T.bd_pass1:
            cell(cv, xs["bd"], y, 15, 9, "d", T.bd_pass1[k])
        for j, (pos, local, reply) in enumerate(T.reexec):
            if k in local:
                cell(cv, xs[f"r{j + 1}"], y, 15, 9, "L", local[k])
            elif T.ops[pos][0] == "r" and T.ops[pos][1] == k:
                cv.text(xs[f"r{j + 1}"], y, "r " + tx(reply), size=TINY - 0.4, ha="center", weight="bold")
        if k in T.delta:
            cell(cv, xs["D10"], y, 15, 9, "D", T.delta[k], "chg")
        if k in C1:
            cell(cv, xs["C1"], y, 15, 9, "C", C1[k], chg.get(k))

    # the collision on e
    ye = ry["e"]
    cv.box(xs["o1"] - 9, ye - 6, xs["o2"] - xs["o1"] + 18, 12, fill="none", edge=INK, lw=1.0,
           ls=(0, (2, 1.2)), r=1.5, z=6)
    # flows along rows
    yc = ry["c"]
    for a, b_ in (("o0", "bd"), ("bd", "D10"), ("D10", "C1")):
        cv.arrow((xs[a] + 8.5, yc), (xs[b_] - 8.5, yc), lw=0.55, color=DARK, head=(2.6, 1.2))
    curved(cv, (xs["r1"] + 6, ye + 4.5), (xs["D10"] - 7, ye + 4), rad=0.25, lw=0.55, color=DARK, head=(2.6, 1.2))
    cv.arrow((xs["D10"] + 8.5, ye), (xs["C1"] - 8.5, ye), lw=0.55, color=DARK, head=(2.6, 1.2))
    curved(cv, (xs["r1"] + 4, ye - 5), (xs["r2"] - 6, ye - 5), rad=-0.5, lw=0.55, color=DARK, head=(2.4, 1.1))

    notes = [
        "o3 read e in parallel and saw 7 (from S); o2 writes e, so W × R makes o2 and o3 collide.",
        tx(rf"Pass 1 merges the non-colliders into {om(10)}; pass 2 re-runs o2 then o3 over $C$ ⊕ {om(10)}, and o3 now reads 8."),
        tx(rf"{om(10)} becomes {D(10)}: appended to the log and merged into C. Commit and backtrack are the CRUD cache's."),
    ]
    for j, n in enumerate(notes):
        cv.text(0, bot + 10 + j * 9, n, size=TINY, color=DARK)
    cv.save(os.path.join(out, f"{PFX}-c-table"))


# ---------------------------------------------------------------------------
# Threads and containers (figures D–H)
#
# Threading, from scalable_crud/mod.rs and pending_state.rs:
#   - one worker thread receives every request from the channel and handles them one at a time;
#   - step ① runs on the exec pool (rayon, PARALLEL_EXEC_THREAD_POOL_SIZE = 4): one
#     ParallelExecutionUnit per operation, joined by pool.install (the worker waits there);
#   - steps ②–⑤, the collider re-runs (a CachingState over current_view), commit and backtrack
#     all run on the worker thread;
#   - unordered reads run on a separate read pool (READ_THREAD_POOL_SIZE = 4) over S only.
# A container is the proxy the application executes against: it implements CRUDState, so
# app.speculatively_execute(&mut unit, op) never sees S or C directly. It reads the speculated
# view (its own ℓ, then C, then S) and writes only into its own ℓ, which the worker later folds
# into the decision being built (ω10, which becomes Ω10).
# ---------------------------------------------------------------------------
matplotlib.rcParams["hatch.linewidth"] = 0.35

POOL, WORKER, READ = "pool", "worker", "read"


def ttag(cv, x, y, name, kind=POOL, size=TINY - 0.8):
    """Thread tag. Black: exec pool thread (parallel). White: the worker thread. Grey: read pool."""
    fill, edge, tc = {POOL: (INK, INK, PAPER), WORKER: (PAPER, INK, INK), READ: (MID, MID, PAPER)}[kind]
    w = 4 + 3.3 * len(name)
    cv.box(x, y - 3.5, w, 7, fill=fill, edge=edge, lw=0.6, r=1.2, z=7)
    cv.text(x + w / 2, y + 0.2, name, size=size, ha="center", color=tc, weight="bold", z=8)
    return w


def tag_w(name):
    return 4 + 3.3 * len(name)


def container(cv, x, y, w, h, title, tag, kind=POOL, dashed=False):
    """A proxy container: double border, the operation as title, its thread in the corner."""
    cv.box(x, y, w, h, fill=PAPER, edge=INK, lw=1.0, r=2.2, z=3, ls=(0, (3, 1.2)) if dashed else "-")
    cv.box(x + 1.6, y + 1.6, w - 3.2, h - 3.2, fill="none", edge=INK, lw=0.3, r=1.4, z=3.1)
    cv.text(x + 5, y + 7.5, title, size=TINY, weight="bold")
    ttag(cv, x + w - tag_w(tag) - 4, y + 7.5, tag, kind)


def par_frame(cv, x, y, w, h, title):
    """A parallel section: hatched ground under a solid frame."""
    cv.box(x, y, w, h, fill=PAPER, edge=LIGHT, lw=0, hatch="//////", r=2, z=0.4)
    cv.box(x, y, w, h, fill="none", edge=INK, lw=0.8, r=2, z=0.5)
    cv.text(x + 3, y + 6, title, size=TINY - 0.2, weight="bold",
            bbox=dict(boxstyle="square,pad=0.15", fc=PAPER, ec="none"))


def seq_frame(cv, x, y, w, h, title):
    cv.box(x, y, w, h, fill=PAPER, edge=INK, lw=0.6, r=2, z=0.5)
    cv.text(x + 3, y + 6, title, size=TINY - 0.2, weight="bold",
            bbox=dict(boxstyle="square,pad=0.15", fc=PAPER, ec="none"))


def ell(p):
    return rf"$\ell_{{{p}}}$"


def source(k, *layers):
    """Which layer serves a read of k: the index of the first layer holding it (the last is S)."""
    for i, layer in enumerate(layers[:-1]):
        if k in layer:
            return i
    return len(layers) - 1


def legend_threads(cv, x, y, extra=()):
    items = [(POOL, "T1", "exec pool thread (parallel)"), (WORKER, "W", "worker thread (one request at a time)")]
    items += list(extra)
    for kind, name, text in items:
        x += ttag(cv, x, y, name, kind) + 3
        t = cv.text(x, y, text, size=TINY - 0.6, color=DARK)
        x += 4.4 + 2.55 * len(text)
    return x


def fig_d(out):
    """D — middleware: speculated view | containers | decision state, read arrows in, write arrows out."""
    H = 286
    cv = Canvas(FULL, H)
    ops = T.ops
    VX, VW = 0, 112
    MX, MW = 120, 222
    DX = 350
    DW = FULL - DX

    region(cv, VX, 0, VW, 262, "pend", "SPECULATED VIEW · read only")
    region(cv, MX, 0, MW, 262, "work", "MIDDLEWARE · a container per operation")
    cv.box(DX, 0, DW, 262, fill=PAPER, edge=DARK, lw=0.7, ls=(0, (4, 1.5)), r=2, z=0.3)
    cv.text(DX + 3, 5.5, tx(rf"DECISION STATE · {D(10)} being built"), size=TINY, weight="bold", color=DARK)

    # --- the speculated view: C over S, by key --------------------------------
    cv.text(VX + 6, 18, tx(rf"$C$ = {D(7)} ⊕ {D(8)} ⊕ {D(9)}"), size=TINY)
    cv.text(VX + 6, 27, "over the confirmed S", size=TINY - 0.6, color=DARK)
    sx, cx = VX + 36, VX + 66
    ry = {k: 56 + i * 15 for i, k in enumerate(KEYS)}
    cv.text(sx, 43, "$S$", size=TINY, ha="center")
    cv.text(cx, 43, "$C$", size=TINY, ha="center")
    cv.text(VX + 92, 43, tx("read ←"), size=TINY - 1, ha="center", color=DARK)
    for k, y in ry.items():
        cv.text(VX + 10, y, f"${k}$", size=SMALL, color=MID)
        if k in S0:
            cell(cv, sx, y, 15, 10, "S", S0[k])
        else:
            cv.text(sx, y, "–", size=TINY, ha="center", color=MID)
        if k in C0:
            cell(cv, cx, y, 15, 10, "C", C0[k])
        else:
            cv.text(cx, y, "–", size=TINY, ha="center", color=MID)
    cv.text(VX + 6, 140, "a read stops at the first\nlayer holding the key:\nthe container's own ℓ,\nthen C, then S",
            size=TINY - 0.6, color=DARK, va="top", linespacing=1.15)
    cv.text(VX + 6, 178, "every container holds\n&C and &S: shared by all\nthreads, never written\nwhile batch 10 runs",
            size=TINY - 0.6, color=DARK, va="top", linespacing=1.15)
    # after step 5
    cv.box(VX + 6, 228, VW - 12, 26, fill=PAPER, edge=INK, lw=0.9, r=1.4)
    cv.text(VX + 10, 235, tx(rf"5  the log gains {D(10)}"), size=TINY - 0.2, weight="bold")
    cv.text(VX + 10, 246, tx(rf"$C$ ← $C$ ⊕ {D(10)}: c 4→5, e +8"), size=TINY - 0.8, color=DARK)

    # --- ① parallel containers ----------------------------------------------
    par_frame(cv, MX + 6, 13, MW - 12, 146, "1  PARALLEL · exec pool, 4 threads")
    kx, kw, kh = MX + 22, 172, 27
    cy = [26, 58, 90, 122]
    layers = (C0, S0)
    for pos, (op, y) in enumerate(zip(ops, cy)):
        local, accesses, reply = T.lanes[pos]
        stale = pos in T.colliders and op[0] == "r"
        container(cv, kx, y, kw, kh, f"o{pos + 1} · {op_short(op)}", f"T{pos + 1}", POOL)
        yy = y + 19
        cv.text(kx + 6, yy, ell(pos + 1), size=TINY)
        if local:
            k, v = next(iter(local.items()))
            cv.text(kx + 15, yy, f"{k}:", size=TINY - 0.4, color=DARK)
            cell(cv, kx + 28, yy, 11, 8.5, "L", v, size=TINY - 0.6)
        else:
            cv.text(kx + 22, yy, "empty", size=TINY - 0.8, color=MID)
        k, kind = accesses[0]
        cv.text(kx + 46, yy, f"logs {kind} {k}", size=TINY - 0.6, color=DARK)
        cv.text(kx + 84, yy, f"reply {reply}", size=TINY - 0.6, color=MID if stale else INK,
                style="italic" if stale else "normal")
        if stale:
            cv.text(kx + 118, yy, "stale", size=TINY - 0.8, color=MID, style="italic")
        # the read, from the layer that serves it into the container
        if source(k, C0, S0) == 0:
            pts = [(cx + 7.5, ry[k]), (kx, y + kh / 2)]
        else:                   # from S, straight through C's empty slot
            pts = [(sx + 7.5, ry[k]), (cx + 11, ry[k]), (kx, y + kh / 2)]
        parrow(cv, pts, lw=0.5, ls=(0, (2, 1.2)), color=DARK, head=(2.6, 1.2))
        # out to the join barrier
        cv.arrow((kx + kw, y + kh / 2), (MX + MW - 13, y + kh / 2), lw=0.6, head=(2.6, 1.2))
    jx = MX + MW - 12.5
    cv.box(jx, 28, 2.6, 118, fill=INK, edge=INK, lw=0, r=0.6, z=6)
    cv.text(jx - 2, 152, "join", size=TINY - 1, ha="right", color=DARK)
    cv.text(kx, 155, tx("each container returns (accesses, ℓ); the worker waits at the join"),
            size=TINY - 1, color=DARK)

    # --- ② ③ on the worker ---------------------------------------------------
    bx, bw = DX + 8, DW - 16
    cv.box(bx, 16, bw, 46, fill=PAPER, edge=INK, lw=0.7, r=1.6)
    cv.text(bx + 4, 23, "2  collision check", size=TINY, weight="bold")
    ttag(cv, bx + bw - tag_w("W") - 4, 23, "W", WORKER)
    cv.text(bx + 4, 33, "e:  W by o2,  R by o3   → collide", size=TINY - 0.6)
    cv.text(bx + 4, 41, "c:  W by o1      b:  R by o4", size=TINY - 0.6, color=DARK)
    cv.text(bx + 4, 52, "colliders o2, o3: their ℓ and replies are dropped", size=TINY - 0.8, weight="bold")
    parrow(cv, [(jx + 2.6, 60), (bx, 40)], lw=0.9, head=(3, 1.4))

    cv.box(bx, 72, bw, 26, fill=PAPER, edge=INK, lw=0.7, r=1.6)
    cv.text(bx + 4, 79, "3  pass 1: merge non-colliders", size=TINY, weight="bold")
    ttag(cv, bx + bw - tag_w("W") - 4, 79, "W", WORKER)
    cv.text(bx + 4, 90, tx(rf"{ell(1)}, {ell(4)} into {om(10)}, in batch order"), size=TINY - 0.6, color=DARK)
    cv.arrow((bx + bw / 2, 62), (bx + bw / 2, 72), lw=0.7, head=(2.6, 1.2))

    # ω10, the decision's write set: only the keys it holds, one per row
    wy = 108
    cv.box(bx, wy, bw, 44, fill=WORK_FILL, edge=INK, lw=1.0, ls=(0, (3, 1.3)), r=1.6)
    cv.text(bx + 4, wy + 7, tx(rf"{om(10)} · batch_delta"), size=TINY, weight="bold")
    cv.text(bx + bw - 4, wy + 7, "the decision's write set", size=TINY - 1, ha="right", color=DARK)
    wrow = {"c": wy + 20, "e": wy + 34}
    wcx = bx + 26
    for k, why in (("c", "pass 1, from o1"), ("e", "pass 2, from o2's re-run")):
        cell(cv, wcx, wrow[k], 13, 10, "d", T.delta[k])
        cv.text(wcx + 10, wrow[k], f"${k}$", size=TINY, color=MID)
        cv.text(wcx + 18, wrow[k], why, size=TINY - 0.8, color=DARK)
    cv.arrow((bx + bw / 2, 98), (bx + bw / 2, wy), lw=0.7, head=(2.6, 1.2))

    # --- ④ sequential re-runs -----------------------------------------------
    seq_frame(cv, MX + 6, 167, MW - 12, 88, "4  SEQUENTIAL · worker thread, colliders in batch order")
    view = dict(T.view)             # current_view: C ⊕ ω10, grown by each re-run in turn
    for j, (pos, local, reply) in enumerate(T.reexec):
        y = 180 + j * 36
        ym = y + kh / 2
        op = ops[pos]
        k = op[1]
        container(cv, kx, y, kw, kh, f"o{pos + 1} again · {op_short(op)}", "W", WORKER)
        yy = y + 19
        if local:
            cv.text(kx + 6, yy, tx(rf"over $C$ ⊕ {om(10)}"), size=TINY - 0.6, color=DARK)
            k2, v = next(iter(local.items()))
            cv.text(kx + 62, yy, f"writes {k2}:", size=TINY - 0.6, color=DARK)
            cell(cv, kx + 94, yy, 11, 8.5, "L", v, size=TINY - 0.6)
            cv.text(kx + 108, yy, f"reply {reply}", size=TINY - 0.6)
            # the write lands in ω10
            parrow(cv, [(kx + kw, ym), (bx - 12, ym), (bx - 12, wrow[k2] - 2), (wcx - 6.5, wrow[k2] - 2)],
                   lw=0.9, head=(3, 1.4))
        else:
            cv.text(kx + 6, yy, tx(rf"reads {k} = {reply} from {om(10)}"), size=TINY - 0.6, color=DARK)
            cv.text(kx + 84, yy, f"reply {reply}", size=TINY - 0.4, weight="bold")
            cv.text(kx + 110, yy, f"(was {T.lanes[pos][2]})", size=TINY - 0.8, color=MID)
        if k in view:           # served by ω10: written by an earlier re-run in this pass
            parrow(cv, [(wcx - 6.5, wrow[k] + 2), (bx - 4, wrow[k] + 2), (bx - 4, ym), (kx + kw, ym)],
                   lw=0.5, ls=(0, (2, 1.2)), color=DARK, head=(2.6, 1.2))
        else:                   # neither C nor ω10 holds it: S, along the gap between the bands
            gx = VX + VW + 4
            parrow(cv, [(sx + 7.5, ry[k] + 2.5), (gx, ry[k] + 2.5), (gx, ym), (kx, ym)],
                   lw=0.5, ls=(0, (2, 1.2)), color=DARK, head=(2.6, 1.2))
        view.update(local)

    # --- ⑤ -------------------------------------------------------------------
    cv.box(bx, 196, bw, 30, fill=PAPER, edge=INK, lw=1.2, r=1.6)
    cv.text(bx + 4, 203, tx(rf"5  {om(10)} becomes {D(10)}"), size=TINY, weight="bold")
    ttag(cv, bx + bw - tag_w("W") - 4, 203, "W", WORKER)
    cv.text(bx + 4, 216, "replies held, in order:  " + "  ".join(f"o{i + 1}:{r}" for i, r in enumerate(T.replies)),
            size=TINY - 0.8, color=DARK)
    cv.arrow((bx + bw / 2, wy + 44), (bx + bw / 2, 196), lw=0.7, head=(2.6, 1.2))
    parrow(cv, [(bx + bw / 2, 226), (bx + bw / 2, 258), (VX + VW / 2, 258), (VX + VW / 2, 254)], lw=1.1,
           head=(3.4, 1.7))
    cv.text(bx + bw / 2 - 4, 254, "append to the log", size=TINY - 0.8, ha="right", color=DARK)

    x = legend_threads(cv, 0, 274)
    cv.line([(x + 4, 274), (x + 18, 274)], lw=0.6, ls=(0, (2, 1.2)), color=DARK)
    cv.text(x + 21, 274, "read", size=TINY - 0.6, color=DARK)
    cv.line([(x + 40, 274), (x + 54, 274)], lw=0.7)
    cv.text(x + 57, 274, "write / hand-off", size=TINY - 0.6, color=DARK)
    cv.save(os.path.join(out, f"{PFX}-d-middleware"))


def seg(cv, x0, x1, y, text, kind=WORKER, h=13, cont=False, size=TINY - 1):
    """A slice of work on a lane: a container (double border) or plain work."""
    if cont:
        container_slim(cv, x0, y - h / 2, x1 - x0, h)
    else:
        fill = {WORKER: PAPER, READ: PALE_FILL, POOL: PAPER}[kind]
        cv.box(x0, y - h / 2, x1 - x0, h, fill=fill, edge=INK, lw=0.6, r=1.2, z=3)
    cv.text((x0 + x1) / 2, y + 0.2, text, size=size, ha="center", z=6, linespacing=1.05)


def container_slim(cv, x, y, w, h):
    cv.box(x, y, w, h, fill=PAPER, edge=INK, lw=1.0, r=1.8, z=3)
    cv.box(x + 1.4, y + 1.4, w - 2.8, h - 2.8, fill="none", edge=INK, lw=0.3, r=1.1, z=3.1)


PALE_FILL = "#e4e4e4"


def fig_e(out):
    """E — thread lanes over time: what runs where, and what waits for the worker."""
    cv = Canvas(FULL, 222)
    L = 64                                   # lane labels end here
    lanes = {"ch1": 14, "ch2": 28, "W": 52, "T1": 76, "T2": 92, "T3": 108, "T4": 124, "R1": 150, "R2": 166}
    cv.text(0, 21, "channel", size=TINY, weight="bold")
    cv.text(0, 29, "from consensus", size=TINY - 1.2, color=DARK)
    for name, kind, y in (("W", WORKER, lanes["W"]), ("T1", POOL, lanes["T1"]), ("T2", POOL, lanes["T2"]),
                          ("T3", POOL, lanes["T3"]), ("T4", POOL, lanes["T4"]), ("R1", READ, lanes["R1"]),
                          ("R2", READ, lanes["R2"])):
        ttag(cv, L - 16, y, name, kind)
        cv.line([(L, y), (FULL, y)], lw=0.3, color=LIGHT, z=1)
    cv.text(0, lanes["W"], "worker", size=TINY, weight="bold")
    cv.text(0, lanes["T1"] - 2, "exec pool", size=TINY, weight="bold")
    cv.text(0, lanes["T1"] + 6, "rayon, 4 threads", size=TINY - 1.2, color=DARK)
    cv.text(0, lanes["R1"] - 2, "read pool", size=TINY, weight="bold")
    cv.text(0, lanes["R1"] + 6, "R3, R4 idle", size=TINY - 1.2, color=DARK)
    # parallel sections, hatched behind the pool lanes
    for x0, x1, y0, y1 in ((96, 168, 66, 134), (380, 424, 140, 176)):
        cv.box(x0, y0, x1 - x0, y1 - y0, fill=PAPER, edge=LIGHT, lw=0, hatch="//////", r=1.5, z=0.4)

    def request(x, row, text, pick):
        y = lanes[row]
        w_ = 3.0 + 2.62 * len(text)
        cv.box(x, y - 5, w_, 10, fill=PAPER, edge=INK, lw=0.7, r=3, z=5)
        cv.text(x + w_ / 2, y + 0.2, text, size=TINY - 1, ha="center", z=6)
        if pick > x + w_ + 2:
            cv.line([(x + w_, y), (pick, y)], lw=0.6, ls=(0, (1, 1.2)), color=DARK, z=4)
            cv.text((x + w_ + pick) / 2, y - 3.2, "waits", size=TINY - 1.6, ha="center", color=DARK)
        parrow(cv, [(pick, y + (5 if pick <= x + w_ + 2 else 0)), (pick, lanes["W"] - 7)], lw=0.7, head=(2.6, 1.2))

    request(70, "ch1", "PreemptiveUpdate(10)", 76)
    request(122, "ch2", "Finalized(7)", 300)
    request(196, "ch1", "PreemptiveUpdate(11)", 452)
    request(306, "ch2", "Unordered(rd a, rd e)", 380)

    yW = lanes["W"]
    seg(cv, 74, 94, yW, "seq ok")
    cv.line([(96, yW), (166, yW)], lw=0.7, ls=(0, (1, 1.3)), color=DARK, z=2)
    cv.text(131, yW - 4.5, "waits in pool.install", size=TINY - 1.3, ha="center", color=DARK)
    # fork / join bars
    for x in (95, 167):
        cv.box(x - 1.2, yW - 7, 2.4, lanes["T4"] + 7 - (yW - 7), fill=INK, edge=INK, lw=0, r=0.5, z=6)
    cv.text(95, lanes["T4"] + 11, "fork", size=TINY - 1.2, ha="center", color=DARK)
    cv.text(167, lanes["T4"] + 11, "join", size=TINY - 1.2, ha="center", color=DARK)
    ends = (150, 162, 138, 130)               # unequal work: the join waits for the slowest
    for pos, (op, x1) in enumerate(zip(T.ops, ends)):
        seg(cv, 99, x1, lanes[f"T{pos + 1}"], f"o{pos + 1}  {op_short(op)}", POOL, cont=True)
        if pos in T.colliders and op[0] == "r":
            cv.text(x1 + 3, lanes[f"T{pos + 1}"], "reply 7, stale", size=TINY - 1.3, color=MID, style="italic")
    seg(cv, 170, 186, yW, "2")
    seg(cv, 186, 202, yW, "3")
    seg(cv, 204, 240, yW, "o2 again", cont=True)
    seg(cv, 240, 272, yW, "o3 again", cont=True)
    seg(cv, 274, 296, yW, tx(rf"5  {D(10)}"))
    cv.text(237, yW + 11, tx(rf"collisions, merge, re-runs over $C$ ⊕ {om(10)}"), size=TINY - 1.3, ha="center",
            color=DARK)
    seg(cv, 300, 376, yW, tx(rf"commit {D(7)} → {Dc(7)}: S ← S ⊕ {Dc(7)}"))
    cv.text(338, yW + 11, tx(rf"refold $C$; replies of {Dc(7)} sent"), size=TINY - 1.3, ha="center", color=DARK)
    # unordered reads: on S, no container
    cv.line([(382, yW), (422, yW)], lw=0.7, ls=(0, (1, 1.3)), color=DARK, z=2)
    for x in (381, 423):
        cv.box(x - 1.2, yW - 7, 2.4, lanes["R2"] + 7 - (yW - 7), fill=INK, edge=INK, lw=0, r=0.5, z=6)
    S2 = SCALABLE[2].S
    seg(cv, 385, 418, lanes["R1"], f"rd a → {S2['a']}", READ)
    seg(cv, 385, 412, lanes["R2"], f"rd e → {S2['e']}", READ)
    seg(cv, 426, 448, yW, "send")
    seg(cv, 452, 500, yW, "batch 11:  1 …", size=TINY - 1.1)
    cv.arrow((L, 186), (FULL - 2, 186), lw=0.6, head=(3, 1.4))
    cv.text(FULL - 2, 191, "time (not to scale)", size=TINY - 1.2, ha="right", color=DARK)

    notes = [
        "Only the hatched sections run in parallel. Each is a fork–join, and the worker waits at the join for the slowest thread.",
        tx(rf"The worker handles one request at a time: Finalized(7) and batch 11 queue in the channel while batch 10 speculates."),
        tx(rf"Unordered reads run on S with no container, so they see only confirmed decisions: rd e returns 7, not the speculated 8."),
    ]
    for j, n in enumerate(notes):
        cv.text(0, 198 + j * 8, n, size=TINY - 0.6, color=DARK)
    cv.save(os.path.join(out, f"{PFX}-e-thread-lanes"))


def read_path(cv, x, y, hit, value, labels=("ℓ", "C", "S"), widths=(11, 11, 13)):
    """A read through the layers, left to right: misses drawn faint, the hit holds the value."""
    for i, (lab, cw) in enumerate(zip(labels, widths)):
        if i < hit:
            cv.box(x, y - 4, cw, 8, fill=PAPER, edge=MID, lw=0.4, ls=(0, (1.2, 1)), r=1, z=4)
            cv.text(x + cw / 2, y + 0.2, lab, size=TINY - 1.4, ha="center", color=MID)
            cv.arrow((x + cw, y), (x + cw + 4, y), lw=0.4, color=MID, head=(1.6, 0.8))
        elif i == hit:
            cv.box(x, y - 4, cw, 8, fill=PAPER, edge=INK, lw=0.8, r=1, z=4)
            cv.text(x + cw / 2, y + 0.2, f"{lab} {value}", size=TINY - 1.2, ha="center", weight="bold")
        x += cw + 4


def fig_g(out):
    """G — the journey of two requests (o2 and o3) from the decided batch to their clients."""
    cv = Canvas(COLUMN, 318)
    A, B, TW = 52, 148, 90                   # track left edges and width
    ca, cb = A + TW / 2, B + TW / 2
    cv.text(ca, 8, f"o2 · {op_short(T.ops[1])}", size=TINY + 0.4, ha="center", weight="bold")
    cv.text(cb, 8, f"o3 · {op_short(T.ops[2])}", size=TINY + 0.4, ha="center", weight="bold")
    cv.text(ca, 16, "a client's write", size=TINY - 1.2, ha="center", color=DARK)
    cv.text(cb, 16, "another client's read", size=TINY - 1.2, ha="center", color=DARK)
    stages = [(30, "batch 10", ""), (44, "1  parallel", "exec pool"), (104, "2  collide", "worker"),
              (128, "3  pass 1", "worker"), (148, "4  re-run", "worker, in order"),
              (222, "5  decision", "worker"), (250, "6  commit", "worker"), (278, "7  reply", "")]
    for y, t, sub in stages:
        cv.text(0, y + 4, t, size=TINY - 0.4, weight="bold")
        if sub:
            cv.text(0, y + 11, sub, size=TINY - 1.4, color=DARK)
        cv.line([(0, y - 2), (COLUMN, y - 2)], lw=0.25, color=LIGHT, z=0.2)

    cv.text((A + B + TW) / 2, 34, "o1 · o2 · o3 · o4, positions fixed by consensus", size=TINY - 1, ha="center",
            color=DARK)
    for x in (ca, cb):
        cv.arrow((x, 38), (x, 46), lw=0.6, head=(2.4, 1.1))

    # 1 parallel: one container each, on different threads
    cv.box(A - 3, 44, B + TW + 3 - (A - 3), 54, fill=PAPER, edge=LIGHT, lw=0, hatch="//////", r=1.5, z=0.4)
    for x, pos, tname in ((A, 1, "T2"), (B, 2, "T3")):
        local, accesses, reply = T.lanes[pos]
        container(cv, x, 46, TW, 50, "ParallelExecutionUnit", tname, POOL)
        cv.text(x + 5, 64, "reads e", size=TINY - 1.2, color=DARK)
        read_path(cv, x + 26, 64, 2, S0["e"])
        if local:
            k, v = next(iter(local.items()))
            cv.text(x + 5, 76, tx(rf"writes {ell(pos + 1)}: {k} = {v}"), size=TINY - 1)
        else:
            cv.text(x + 5, 76, tx(rf"{ell(pos + 1)} stays empty"), size=TINY - 1, color=DARK)
        stale = T.ops[pos][0] == "r"
        cv.text(x + 5, 88, f"reply {reply}" + ("  (stale)" if stale else "  (old e)"), size=TINY - 1,
                color=MID if stale else INK, style="italic" if stale else "normal")

    # 2 collide
    cv.arrow((ca + 18, 109), (cb - 18, 109), lw=0.9, both=True, head=(2.6, 1.2))
    cv.text((ca + cb) / 2, 104.5, "W × R on e", size=TINY - 1, ha="center", weight="bold")
    for x in (ca, cb):
        cv.line([(x, 96), (x, 109), (x + (18 if x == ca else -18), 109)], lw=0.6)
        cv.text(x, 117, tx(r"collider: $\ell$ and reply dropped"), size=TINY - 1.4, ha="center", color=DARK)

    # 3 pass 1: what the batch already holds before the re-runs
    cv.box(A, 126, B + TW - A, 13, fill=WORK_FILL, edge=INK, lw=0.6, ls=(0, (2, 1.1)), r=1.2)
    cv.text((A + B + TW) / 2, 132.7, tx(rf"{om(10)} = {{c: 5}}   from o1; o4 wrote nothing"), size=TINY - 1,
            ha="center")

    # 4 re-runs on the worker, o2 first
    (p2, l2, r2), (p3, l3, r3) = T.reexec
    container(cv, A, 146, TW, 40, "CachingState", "W", WORKER)
    cv.text(A + 5, 163, "reads e", size=TINY - 1.2, color=DARK)
    read_path(cv, A + 26, 163, 2, S0["e"], labels=("ℓ", "C⊕ω", "S"), widths=(9, 20, 13))
    cv.text(A + 5, 176, tx(rf"writes e = {l2['e']} into {om(10)}"), size=TINY - 1)
    cv.text(A + TW - 5, 176, f"reply {r2}", size=TINY - 1, ha="right", weight="bold")
    container(cv, B, 176, TW, 40, "CachingState", "W", WORKER)
    cv.text(B + 5, 193, "reads e", size=TINY - 1.2, color=DARK)
    read_path(cv, B + 26, 193, 1, l2["e"], labels=("ℓ", "C⊕ω", "S"), widths=(9, 24, 13))
    cv.text(B + 5, 206, tx(rf"from {om(10)}, after o2"), size=TINY - 1, color=DARK)
    cv.text(B + TW - 5, 206, f"reply {r3}", size=TINY - 1, ha="right", weight="bold")
    parrow(cv, [(A + TW, 176), (B + 20, 176)], lw=0.8, head=(2.6, 1.2))
    cv.text(A + TW + 2, 172, tx(rf"e = 8 in {om(10)}"), size=TINY - 1.4, color=DARK)
    for x, y0, y1 in ((ca, 139, 146), (cb, 139, 176)):
        cv.arrow((x, y0), (x, y1), lw=0.6, head=(2.4, 1.1))

    # 5 the decision, 6 commit, 7 replies
    cv.box(A, 220, B + TW - A, 22, fill=PAPER, edge=INK, lw=1.0, r=1.4)
    cv.text((A + B + TW) / 2, 226, tx(rf"{om(10)} becomes {D(10)} = {{c: 5, e: 8}}, appended to the log"),
            size=TINY - 1, ha="center", weight="bold")
    cv.text((A + B + TW) / 2, 235.5, f"replies held: o2 → {r2}, o3 → {r3}", size=TINY - 1, ha="center",
            color=DARK)
    cv.arrow((ca, 186), (ca, 220), lw=0.6, head=(2.4, 1.1))
    cv.arrow((cb, 216), (cb, 220), lw=0.6, head=(2.4, 1.1))
    cv.box(A, 248, B + TW - A, 22, fill=PAPER, edge=INK, lw=1.0, r=1.4)
    cv.text((A + B + TW) / 2, 254, tx(rf"Finalized(10), after {Dc(7)}, {Dc(8)}, {Dc(9)}: {D(10)} → {Dc(10)}"),
            size=TINY - 1, ha="center", weight="bold")
    cv.text((A + B + TW) / 2, 263.5, tx(rf"S ← S ⊕ {Dc(10)}; refold $C$"), size=TINY - 1, ha="center",
            color=DARK)
    cv.arrow(((A + B + TW) / 2, 242), ((A + B + TW) / 2, 248), lw=0.6, head=(2.4, 1.1))
    for x, r in ((A, r2), (B, r3)):
        cv.arrow((x + TW / 2, 270), (x + TW / 2, 277), lw=0.6, head=(2.4, 1.1))
        cv.box(x + 14, 277, TW - 28, 13, fill=PAPER, edge=INK, lw=0.6, r=4)
        cv.text(x + TW / 2, 283.7, f"client gets {r}" + (" (old e)" if x == A else ""), size=TINY - 1,
                ha="center", weight="bold")
    cv.text(0, 300, "o3's client never sees the stale 7: replies leave", size=TINY - 1, color=DARK)
    cv.text(0, 308, tx(rf"only once {Dc(10)} is confirmed, from the re-run."), size=TINY - 1, color=DARK)
    cv.save(os.path.join(out, f"{PFX}-g-request-journey"))


def fig_h(out):
    """H — layer cake: one column per container, read paths down through private and shared layers."""
    cv = Canvas(FULL, 196)
    ops = T.ops
    cols = [(f"T{p + 1}", POOL, p, False) for p in range(4)] + [("W", WORKER, p, True) for p, _, _ in T.reexec]
    x0, pitch, gap = 96, 64, 14
    xs = [x0 + i * pitch + (gap if i >= 4 else 0) + pitch / 2 for i in range(len(cols))]
    rows = {"op": 22, "L": 42, "w": 68, "C": 92, "S": 116, "out": 140}
    left, right = x0 - 4, xs[-1] + pitch / 2 + 2

    # phase headers
    cv.box(xs[0] - pitch / 2, 2, 4 * pitch, 11, fill=PAPER, edge=LIGHT, lw=0, hatch="//////", r=1, z=0.4)
    cv.text((xs[0] + xs[3]) / 2, 7.5, "1  PARALLEL · exec pool", size=TINY - 0.4, ha="center", weight="bold",
            bbox=dict(boxstyle="square,pad=0.15", fc=PAPER, ec="none"))
    cv.text((xs[4] + xs[5]) / 2, 7.5, "4  SEQUENTIAL · worker", size=TINY - 0.4, ha="center", weight="bold")
    sep = (xs[3] + xs[4]) / 2
    cv.line([(sep, 2), (sep, 150)], lw=0.6, ls=(0, (2, 1.5)), color=DARK, z=1)

    # row labels
    labels = [("L", "container's own ℓ", "private, writable"), ("w", tx(rf"{om(10)} · batch_delta"), "worker only"),
              ("C", tx(rf"$C$ = {D(7)} ⊕ {D(8)} ⊕ {D(9)}"), "shared, read only"),
              ("S", "S · confirmed", "shared, read only"), ("out", "what happens next", "")]
    for key, t, sub in labels:
        cv.text(0, rows[key] - (3 if sub else 0), t, size=TINY - 0.4, weight="bold")
        if sub:
            cv.text(0, rows[key] + 5, sub, size=TINY - 1.2, color=DARK)
    # shared layers: one band across every column
    cv.box(left, rows["C"] - 9, right - left, 18, fill=PEND_FILL, edge=MID, lw=0.6, ls=(0, (3, 2)), r=1.5, z=0.5)
    cv.box(left, rows["S"] - 9, right - left, 18, fill=PAPER, edge=INK, lw=0.8, r=1.5, z=0.5)
    # ω10: visible to the worker only
    cv.box(xs[4] - pitch / 2 + 2, rows["w"] - 9, 2 * pitch - 4, 18, fill=WORK_FILL, edge=INK, lw=0.8,
           ls=(0, (3, 1.3)), r=1.5, z=0.5)
    cv.text((xs[1] + xs[2]) / 2, rows["w"], tx(rf"not built yet: pool threads never see {om(10)}"),
            size=TINY - 1, ha="center", color=MID, style="italic", z=6,
            bbox=dict(boxstyle="square,pad=0.25", fc=PAPER, ec="none"))

    view = dict(T.bd_pass1)
    outcomes = {0: tx(rf"{ell(1)} into {om(10)}, pass 1"), 1: "collider: dropped", 2: "collider: dropped",
                3: "read only: nothing"}
    for i, ((tname, kind, pos, rerun), x) in enumerate(zip(cols, xs)):
        op = ops[pos]
        k = op[1]
        tw_ = tag_w(tname)
        ttag(cv, x - (tw_ + 30) / 2, rows["op"], tname, kind)
        cv.text(x - (tw_ + 30) / 2 + tw_ + 3, rows["op"], f"o{pos + 1} {op_short(op)}", size=TINY - 0.6,
                weight="bold")
        if rerun:
            local = next(l_ for p_, l_, _ in T.reexec if p_ == pos)
            reply = next(r_ for p_, _, r_ in T.reexec if p_ == pos)
        else:
            local, _, reply = T.lanes[pos]
        container_slim(cv, x - 24, rows["L"] - 8, 48, 16)
        if local:
            k2, v = next(iter(local.items()))
            cv.text(x - 8, rows["L"] + 0.2, f"{k2}:", size=TINY - 0.6, ha="right", color=DARK, z=6)
            cell(cv, x + 2, rows["L"], 11, 9, "L", v, size=TINY - 0.6)
        else:
            cv.text(x, rows["L"] + 0.2, "empty", size=TINY - 1, ha="center", color=MID, z=6)
        # where the read of k stops
        layers = (["w"] if rerun else []) + ["C", "S"]
        maps = {"w": view, "C": C0, "S": S0}
        hit = next((ly for ly in layers[:-1] if k in maps[ly]), "S")
        val_ = maps[hit].get(k)
        for ly in layers:
            if ly == hit:
                break
            cv.text(x + 4, rows[ly], f"{k} miss", size=TINY - 1.4, color=MID, z=6)
        kind_ = {"w": "d", "C": "C", "S": "S"}[hit]
        cell(cv, x, rows[hit], 13, 10, kind_, val_)
        cv.text(x + 9, rows[hit] + 0.2, f"${k}$", size=TINY - 1, color=DARK, z=6)
        parrow(cv, [(x, rows["L"] + 8), (x, rows[hit] - 5)], lw=0.55, ls=(0, (2, 1.2)), color=DARK, head=(2.4, 1.1))
        # outcome
        if rerun:
            out_ = tx(rf"e = 8 into {om(10)}") if local else f"reply {reply}, kept"
            view.update(local)
        else:
            out_ = outcomes[pos]
        stale = not rerun and pos in T.colliders
        cv.text(x, rows["out"], out_, size=TINY - 1.2, ha="center", color=MID if stale else INK,
                style="italic" if stale else "normal")
        cv.text(x, rows["out"] + 7, f"reply {reply}" + (" (stale)" if stale and op[0] == "r" else ""),
                size=TINY - 1.4, ha="center", color=MID if stale else DARK)
    notes = [
        tx("Private and mutable: each container's ℓ, owned by one thread. Shared and immutable for the whole batch: C and S,"),
        tx(rf"held as &C and &S, so the parallel phase needs no locks. {om(10)} exists only on the worker: o3's re-run reads e = 8"),
        tx(rf"from it, written by o2's re-run one step earlier. The re-runs read $C$ ⊕ {om(10)} as one map (current_view, a clone)."),
    ]
    for j, n in enumerate(notes):
        cv.text(0, 164 + j * 8.5, n, size=TINY - 0.6, color=DARK)
    cv.save(os.path.join(out, f"{PFX}-h-layer-cake"))


def fig_f(out):
    """F — request routing: every request type the worker accepts, the path it takes, and on which threads."""
    cv = Canvas(FULL, 226)

    renderer = cv.fig.canvas.get_renderer()

    def act(x, y, w_, text, tag=None, kind=WORKER, cont=False, h=14, size=TINY - 1.2):
        """A step box sized to its text (w_ is a minimum); the canvas is 72 dpi, so pixels are points."""
        tx_ = x + (4 if cont else 3)
        if tag:
            tx_ += ttag(cv, tx_, y, tag, kind, size=TINY - 1.4) + 2
        t = cv.text(tx_, y + 0.2, text, size=size, z=6)
        w_ = max(w_, tx_ - x + t.get_window_extent(renderer).width + (5 if cont else 4))
        if cont:
            container_slim(cv, x, y - h / 2, w_, h)
        else:
            cv.box(x, y - h / 2, w_, h, fill=PALE_FILL if kind == READ else PAPER, edge=INK, lw=0.6, r=1.2, z=3)
        return x + w_

    def chip(x, y, text):
        w_ = 3 + 2.55 * len(text)
        cv.box(x, y - 5.5, w_, 11, fill=PAPER, edge=INK, lw=0.8, r=4, z=3)
        cv.text(x + w_ / 2, y + 0.2, text, size=TINY - 1, ha="center", weight="bold", z=6)
        return x + w_

    def diamond(x, y, text, r=7):
        cv.ax.add_patch(Polygon([(x - r, y), (x, y - r), (x + r, y), (x, y + r)], closed=True, facecolor=PAPER,
                                edgecolor=INK, lw=0.6, zorder=3))
        cv.text(x, y + 0.2, text, size=TINY - 1.6, ha="center", z=6)

    def link(a, b, label=None, dy=-3):
        parrow(cv, [a, b] if a[1] == b[1] else [a, (a[0], b[1]), b], lw=0.55, head=(2.4, 1.1))
        if label:
            cv.text(a[0] + 2, b[1] + dy, label, size=TINY - 1.6, color=DARK)

    # source, channel, dispatcher
    cv.box(0, 98, 44, 20, fill=PAPER, edge=INK, lw=0.6, r=1.2)
    cv.text(22, 104, "ordering", size=TINY - 1, ha="center")
    cv.text(22, 112, "protocol", size=TINY - 1, ha="center")
    for i in range(5):
        cv.box(52 + i * 7, 102, 6, 12, fill=PAPER if i else PALE_FILL, edge=INK, lw=0.5, r=0.6)
    cv.text(69, 96, "channel", size=TINY - 1.2, ha="center", color=DARK)
    cv.text(69, 121, "one at a time", size=TINY - 1.4, ha="center", color=DARK)
    cv.arrow((44, 108), (52, 108), lw=0.6, head=(2.4, 1.1))
    cv.arrow((87, 108), (98, 108), lw=0.6, head=(2.4, 1.1))
    cv.box(98, 10, 12, 196, fill=PAPER, edge=INK, lw=0.8, r=1.2)
    ttag(cv, 99.2, 18, "W", WORKER, size=TINY - 1.4)
    cv.text(104, 110, "match request", size=TINY - 1.2, ha="center", rotation=90, z=6)

    # PreemptiveUpdate(k)
    yU, yT, yB = 50, 24, 78
    e = chip(124, yU, "PreemptiveUpdate(k)")
    diamond(e + 14, yU, "k ?")
    dx = e + 14
    cv.arrow((110, yU), (124, yU), lw=0.55, head=(2.4, 1.1))
    cv.arrow((e, yU), (dx - 7, yU), lw=0.55, head=(2.4, 1.1))
    x = dx + 22
    link((dx + 7, yU), (x, yU), "= next", dy=-3)
    x1 = act(x, yU, 0, "1 a container per op", "T1–4", POOL, cont=True)
    cv.box(x - 3, yU - 10, x1 - x + 6, 20, fill=PAPER, edge=LIGHT, lw=0, hatch="//////", r=1.5, z=0.4)
    x = x1
    for t, c in (("2 collide", False), ("3 merge", False), ("4 re-run", True), (tx(rf"5 append {D('k')}"), False)):
        cv.arrow((x, yU), (x + 5, yU), lw=0.55, head=(2.0, 1.0))
        x = act(x + 5, yU, 0, t, "W", WORKER, cont=c)
    xd = x
    parrow(cv, [(xd - 14, yU - 7), (xd - 14, yU - 13), (dx + 44, yU - 13), (dx + 44, yU - 7)], lw=0.5, color=DARK,
           head=(2.2, 1.0))
    cv.text((xd + dx) / 2 + 10, yU - 13, "drain: if k+1 is staged, speculate it the same way", size=TINY - 1.6,
            ha="center", color=DARK, bbox=dict(boxstyle="square,pad=0.15", fc=PAPER, ec="none"), z=7)
    link((dx, yU - 7), (dx + 22, yT), "> next", dy=-3.5)
    act(dx + 22, yT, 0, "stage k in the reorder buffer, like batch 12", "W")
    link((dx, yU + 7), (dx + 22, yB), "< next", dy=8)
    act(dx + 22, yB, 0, tx(rf"backtrack: drop {D('k')} onwards, evict staged ≥ k, refold $C$; then retry k"), "W")
    cv.arrow((dx + 40, yB - 7), (dx + 40, yU + 7), lw=0.55, head=(2.2, 1.0))

    # Finalized(k)
    yF, yF1, yF2 = 116, 104, 128
    cv.arrow((110, yF), (124, yF), lw=0.55, head=(2.4, 1.1))
    e = chip(124, yF, "Finalized(k)")
    dx = e + 14
    diamond(dx, yF, "head?", r=8)
    cv.arrow((e, yF), (dx - 7, yF), lw=0.55, head=(2.4, 1.1))
    link((dx, yF - 7), (dx + 22, yF1), tx(rf"is {D('k')}"), dy=-3.5)
    act(dx + 22, yF1, 0, tx(rf"commit {D('k')} → {Dc('k')}: S ← S ⊕ {Dc('k')}, refold $C$, send the replies of {Dc('k')}"),
        "W")
    link((dx, yF + 7), (dx + 22, yF2), "k staged", dy=8)
    act(dx + 22, yF2, 0, "fallback: speculated batches go back to staged; run k on S directly; send replies", "W")

    # UpdateBatch(k)
    yUB = 150
    cv.arrow((110, yUB), (124, yUB), lw=0.55, head=(2.4, 1.1))
    e = chip(124, yUB, "UpdateBatch(k)")
    cv.arrow((e, yUB), (e + 10, yUB), lw=0.55, head=(2.4, 1.1))
    act(e + 10, yUB, 0, "never speculated: run k on S directly; send replies", "W")

    # ExecuteUnordered
    yR = 172
    cv.arrow((110, yR), (124, yR), lw=0.55, head=(2.4, 1.1))
    e = chip(124, yR, "ExecuteUnordered")
    cv.arrow((e, yR), (e + 8, yR), lw=0.55, head=(2.4, 1.1))
    cv.box(e + 8, yR - 9, 2.2, 18, fill=INK, edge=INK, lw=0, z=6)
    xr = act(e + 14, yR, 0, "read S directly, no container", "R1–4", READ)
    cv.box(e + 11, yR - 10, xr - e - 8, 20, fill=PAPER, edge=LIGHT, lw=0, hatch="//////", r=1.5, z=0.4)
    cv.box(xr + 2, yR - 9, 2.2, 18, fill=INK, edge=INK, lw=0, z=6)
    cv.arrow((xr + 4.2, yR), (xr + 10, yR), lw=0.55, head=(2.0, 1.0))
    act(xr + 10, yR, 36, "send", "W")

    # CatchUp / PollStateChannel
    yC = 194
    cv.arrow((110, yC), (124, yC), lw=0.55, head=(2.4, 1.1))
    e = chip(124, yC, "CatchUp, PollStateChannel")
    cv.arrow((e, yC), (e + 10, yC), lw=0.55, head=(2.4, 1.1))
    act(e + 10, yC, 0, tx("clear the log, C and staged; apply or install S"), "W")

    x = legend_threads(cv, 0, 216, extra=[(READ, "R1", "read pool thread (parallel)")])
    cv.text(x + 4, 216, "hatched: runs in parallel", size=TINY - 0.6, color=DARK)
    cv.save(os.path.join(out, f"{PFX}-f-request-routing"))


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "."
    os.makedirs(out, exist_ok=True)
    for f in (fig_b, fig_c, fig_d, fig_e, fig_f, fig_g, fig_h):
        f(out)
        print(f.__name__)
