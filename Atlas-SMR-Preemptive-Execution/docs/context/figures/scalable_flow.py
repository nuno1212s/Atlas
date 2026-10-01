"""Scalable CRUD executor: the two caching layers, in route B (containers) and route C (table).

    python3 scalable_flow.py <output-dir>

Cross-batch layer: the log of Δs and their fold C, exactly as in the CRUD cache.
Intra-batch layer (rebuilt for every batch, scalable_crud/pending_state.rs:197-307):
  ① each operation runs in parallel through its own local cache ℓ over C over S
  ② accesses are compared; a Write or Delete with anything on the same key collides
  ③ non-colliders' ℓ are merged, in batch order, into batch_delta (bd)
  ④ colliders re-run sequentially, in batch order, over current_view = C ⊕ bd
  ⑤ bd becomes Δ10: appended to the log and merged into C
Commit and backtrack are the CRUD cache's (crud_flow.py figures).
"""
import os
import sys

from executor_model import KEYS, SCALABLE, TOMB, TRACE10, op_text
from flow_common import (D, PEND_FILL, WORK_FILL, badge, cell, curved, key_row, marks_for, parrow,
                         region, tx, val)
from paperfig import BODY, DARK, FULL, INK, LIGHT, MID, PAPER, SMALL, TINY, Canvas

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
    heads = [(6, "1", "parallel units"), (134, "2", "collision check"), (222, "3", "merge non-colliders"),
             (300, "4", "re-run colliders"), (424, "5", tx(rf"{D(10)} = bd"))]
    for x, n, t in heads:
        badge(cv, x, 92, n, r=3.8)
        cv.text(x + 10, 92, t, size=TINY + 0.2, weight="bold")

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
    cv.text(226, 126, "bd", size=TINY, weight="bold")
    row5(cv, 236, 126, T.bd_pass1, "d", pitch=11, w=9.5, keys=True)
    cv.text(226, 142, "o4 read only;\nnothing to merge", size=TINY - 0.8, color=DARK, linespacing=1.1, va="top")

    # ④ pass 2
    cv.text(322, 106, tx("view = C ⊕ bd"), size=TINY - 0.4, color=DARK)
    row5(cv, 304, 118, T.view, "C", pitch=14, w=11.5, h=9)
    for j, (pos, local, reply) in enumerate(T.reexec):
        y = 138 + j * 17
        cv.box(304, y - 7, 106, 14, fill=PAPER, edge=INK, lw=0.6, r=1.4)
        cv.text(308, y, f"o{pos + 1}", size=TINY, weight="bold")
        cv.text(320, y, tx(op_short(ops[pos])), size=TINY - 0.3)
        if local:
            k, v = next(iter(local.items()))
            cv.text(372, y, tx(f"bd ⊕ {k}={v}"), size=TINY - 0.4, color=DARK)
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
    subs = [(("o0", "o3"), "1  parallel units"), (("bd", "bd"), "3  pass 1"), (("r1", "r2"), "4  pass 2"),
            (("D10", "D10"), "5")]
    for (a, b_), t in subs:
        cv.text((xs[a] + xs[b_]) / 2, 20, t, size=TINY - 0.8, ha="center", color=DARK)
    labels = {"S": "$S$", "C": "$C$", "bd": "bd", "D10": D(10), "C1": r"$C$", 7: D(7), 8: D(8), 9: D(9),
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
        tx("Pass 1 merges the non-colliders into bd; pass 2 re-runs o2 then o3 over C ⊕ bd, and o3 now reads 8."),
        tx(rf"bd becomes {D(10)}: appended to the log and merged into C. Commit and backtrack are the CRUD cache's."),
    ]
    for j, n in enumerate(notes):
        cv.text(0, bot + 10 + j * 9, n, size=TINY, color=DARK)
    cv.save(os.path.join(out, f"{PFX}-c-table"))


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "."
    os.makedirs(out, exist_ok=True)
    for f in (fig_b, fig_c):
        f(out)
        print(f.__name__)
