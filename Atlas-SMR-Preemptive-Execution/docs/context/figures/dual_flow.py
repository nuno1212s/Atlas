"""Dual-state executor: routes B (containers) and C (timeline table) with flows and changes.

    python3 dual_flow.py <output-dir>

Notation as in every figure: Ωk is decision k while speculated, Δk once confirmed. Here the
log holds whole batches and their held replies, never a write set: speculation
writes straight into a second full copy S_p, a commit re-executes the batch on S in the confirmed
worker, and a backtrack clones S into S_p and re-executes the surviving batches.
"""
import os
import sys

from executor_model import BATCH, DUAL, KEYS, TOMB, op_text
from flow_common import (D, W, PEND_FILL, badge, cell, changes, header, key_row, marks_for, parrow,
                         region, strike, tx)
from paperfig import BODY, DARK, FULL, INK, LIGHT, MID, PAPER, SMALL, TINY, Canvas

PFX = "preemptive-context-dual-flow"

STEPS = [
    ("0", "start", tx(rf"{W(7)}, {W(8)}, {W(9)} speculated on $S_p$"), "start"),
    ("1", rf"speculate {W(10)}", r"execute batch 10 on $S_p$ in place", "spec"),
    ("2", rf"commit {W(7)}", rf"{W(7)} confirmed as {D(7)}: re-executed on $S$", "commit"),
    ("3", rf"backtrack to {W(9)}", r"$S_p \leftarrow$ clone($S$) (blocking), replay $\Omega_8$", "back"),
]


def ops_text(q):
    return ",  ".join(op_text(op) for op in BATCH[q])


def notes(i):
    st, prev = DUAL[i], DUAL[i - 1] if i else None
    kind = STEPS[i][3]
    if kind == "start":
        return [rf"$S_p$ = S after {W(7)}, {W(8)}, {W(9)}",
                "S: confirmed through seq 6", "log: batches + held replies"]
    added = [q for q in st.seqs() if q not in prev.seqs()]
    removed = [q for q in prev.seqs() if q not in st.seqs()]
    gone = [f"− {W(q)} (now {D(q)})" if kind == "commit" else f"− {W(q)}" for q in removed]
    log = "log: " + ", ".join([f"+ {W(q)}" for q in added] + gone)
    out = [log, tx(changes(prev.P, st.P, "$S_p$")), tx(changes(prev.S, st.S, "S"))]
    if kind == "spec":
        out.append(rf"replies of {W(10)} held")
    elif kind == "back":
        out.append(rf"{W(8)} replayed, replies redone")
    return out


def rows_for(i):
    st, prev = DUAL[i], DUAL[i - 1] if i else None
    kind = STEPS[i][3]
    rows = {q: ("new" if prev and q not in prev.seqs() else None) for q in st.seqs()}
    if kind == "commit":
        rows[prev.seqs()[0]] = "applied"
    if kind == "back":
        for q in prev.seqs():
            if q not in st.seqs():
                rows[q] = "dropped"
            else:
                rows[q] = "replayed"
    return sorted(rows.items(), reverse=True)


# ---------------------------------------------------------------------------
# B — containers with flow arrows, 2 × 2
# ---------------------------------------------------------------------------
def b_panel(cv, px, py, i):
    st, prev = DUAL[i], DUAL[i - 1] if i else None
    num, title, effect, kind = STEPS[i]
    header(cv, px, py + 5, num, title, effect)
    x0, p, w, h = px + 24, 20, 16, 10.5
    xs = [x0 + j * p + p / 2 for j in range(5)]
    col = dict(zip(KEYS, xs))
    gr = x0 + 5 * p
    slots = {10: py + 40, 9: py + 51, 8: py + 62, 7: py + 73}
    yP, yS = py + 97, py + 141
    region(cv, px, py + 23, gr - px + 6, 88, "pend", "preemptive worker · speculative")
    region(cv, px, py + 122, gr - px + 6, 27, "conf", "confirmed worker · authoritative")

    for q, state in rows_for(i):
        y = slots[q]
        ghost = state in ("applied", "dropped")
        cv.text(px + 3, y + 0.3, D(q) if state == "applied" else W(q), size=SMALL, color=MID if ghost else INK)
        cv.text(x0 + 2, y + 0.3, ops_text(q), size=TINY - 0.7, color=MID if ghost else INK,
                weight="bold" if state in ("new", "replayed") else "normal")
        if state == "dropped":
            strike(cv, x0 + 1, y, gr - 1, y)
    cv.line([(x0, yP - 9), (gr, yP - 9)], lw=0.3, color=MID)
    cv.text(px + 3, yP + 0.3, "$S_p$", size=SMALL)
    chg, gone = marks_for(prev.P if prev else None, st.P)
    for k, v in st.P.items():
        cell(cv, col[k], yP, w, h, "P", v, chg.get(k))
    for k, v in gone.items():
        cell(cv, col[k], yP, w, h, "P", v, "gone")
    cv.text(px + 3, yS + 0.3, "$S$", size=SMALL)
    chg, _ = marks_for(prev.S if prev else None, st.S)
    for k, v in st.S.items():
        cell(cv, col[k], yS, w, h, "S", v, chg.get(k))
    key_row(cv, xs, yS + 12)

    if kind == "start":
        cv.bracket(gr + 1, slots[9] - 5, slots[7] + 5)
        parrow(cv, [(gr + 3.5, slots[7] + 5), (gr + 3.5, yP), (gr + 0.5, yP)], lw=0.7)
        cv.text(gr + 7, (slots[7] + yP) / 2 + 2, "executed\nin place", size=TINY, color=DARK, linespacing=1.1)
    elif kind == "spec":
        y = slots[10]
        cv.box(px + 158, y - 6, 44, 12, fill=PAPER, edge=INK, lw=0.6, ls=(0, (2, 1.1)), r=1.2)
        cv.text(px + 180, y + 0.3, "batch 10", size=TINY, ha="center")
        parrow(cv, [(px + 158, y), (gr + 1, y)], lw=0.9)
        parrow(cv, [(gr - 2, y + 5), (gr + 5, y + 7), (gr + 5, yP), (gr + 0.5, yP)], lw=0.9)
        cv.text(gr + 8, (y + yP) / 2 + 6, "execute\n" + r"on $S_p$", size=TINY, color=DARK, linespacing=1.1)
    elif kind == "commit":
        y = slots[7]
        parrow(cv, [(gr + 0.5, y), (gr + 6, y), (gr + 6, yS), (gr + 0.5, yS)], lw=1.1, head=(3.6, 1.8))
        cv.text(gr + 9, (y + yP) / 2 + 3, tx("forward " + D(7) + "\n" + r"re-execute on $S$"),
                size=TINY, color=DARK, linespacing=1.1)
        parrow(cv, [(gr + 6, y), (px + 176, y)], lw=0.7)
        cv.text(px + 178, y + 0.3, "replies → clients", size=TINY, color=DARK)
    else:
        for q in (10, 9):
            parrow(cv, [(gr + 0.5, slots[q]), (gr + 16, slots[q])], lw=0.6, color=MID)
            cv.text(gr + 18, slots[q] + 0.3, "dropped", size=TINY, color=MID)
        # clone S into S_p (blocking request to the confirmed worker), then replay the survivor
        parrow(cv, [(gr + 0.5, yS), (gr + 12, yS), (gr + 12, yP + 2), (gr + 0.5, yP + 2)], lw=1.2, head=(3.8, 1.9))
        cv.text(gr + 15, (yP + yS) / 2 + 2, "clone", size=TINY, color=DARK)
        y = slots[8]
        parrow(cv, [(gr + 0.5, y), (gr + 5, y), (gr + 5, yP - 2), (gr + 0.5, yP - 2)], lw=0.9)
        cv.text(gr + 8, (y + yP) / 2 - 2, tx("replay " + W(8)), size=TINY, color=DARK)

    gx = x0 + 2.5 * p
    active = kind in ("commit", "back")
    cv.arrow((gx, py + 112), (gx, py + 121), lw=0.9 if kind == "commit" else 0.6,
             color=INK if kind == "commit" else LIGHT)
    for j, line in enumerate(notes(i)):
        cv.text(px + 166, py + 100 + j * 9, line, size=TINY - 0.2, color=DARK)


def fig_b(out):
    pw, ph = 248, 162
    cv = Canvas(FULL, 2 * ph + 4)
    for i in range(4):
        b_panel(cv, (i % 2) * (pw + 9), 2 + (i // 2) * ph, i)
    cv.line([(FULL / 2, 4), (FULL / 2, 2 * ph)], lw=0.35, color=LIGHT)
    cv.line([(0, ph), (FULL, ph)], lw=0.35, color=LIGHT)
    cv.save(os.path.join(out, f"{PFX}-b-containers-arrows"))


# ---------------------------------------------------------------------------
# C — timeline table: S and S_p after every step
# ---------------------------------------------------------------------------
def fig_c(out):
    cv = Canvas(FULL, 146)
    gx = lambda g: 18 + g * 124
    sw = 34
    ry = {k: 48 + r * 12.5 for r, k in enumerate(KEYS)}
    top, bot = 24, ry["e"] + 7
    for k in KEYS:
        cv.text(2, ry[k], f"${k}$", size=SMALL, color=MID)
    cv.line([(0, top), (FULL, top)], lw=0.8)
    cv.line([(0, 38), (FULL, 38)], lw=0.45)
    cv.line([(0, bot), (FULL, bot)], lw=0.8)
    for g, (num, title, effect, kind) in enumerate(STEPS):
        st, prev = DUAL[g], DUAL[g - 1] if g else None
        x = gx(g)
        xS, xP = x + sw / 2, x + sw + 4 + sw / 2
        cv.box(x + sw + 2, top, sw + 4, bot - top, fill=PEND_FILL, edge="none", lw=0, r=1, z=0.2)
        badge(cv, x, 7, num)
        cv.text(x + 11, 7, title if g == 0 else "", size=BODY, weight="bold")
        cv.text(x, 17, "state after step " + num, size=TINY - 0.4, color=MID)
        cv.text(xS, 31, "$S$ confirmed", size=TINY, ha="center")
        cv.text(xP, 31, "$S_p$ full copy", size=TINY, ha="center")
        for xx, cur, before, kindc in ((xS, st.S, prev.S if prev else None, "S"),
                                       (xP, st.P, prev.P if prev else None, "P")):
            chg, gone = marks_for(before, cur)
            for k, v in cur.items():
                cell(cv, xx, ry[k], sw - 8, 9.5, kindc, v, chg.get(k))
                if k in chg and before and before.get(k) is not None:
                    cv.text(xx - sw / 2 + 1, ry[k] - 5.5, before[k], size=TINY - 0.6, color=MID)
            for k, v in gone.items():
                cell(cv, xx, ry[k], sw - 8, 9.5, kindc, v, "gone")
                strike(cv, xx - 5, ry[k] + 3.5, xx + 5, ry[k] - 3.5, lw=0.5)
        cv.text(x, bot + 8, "log: " + " ".join(W(q) for q in st.seqs()), size=TINY, color=DARK)
        sent = rf"sent {D(7)}; " if kind == "commit" else ""
        cv.text(x, bot + 18, f"replies: {sent}held " + ", ".join(W(q) for q in st.seqs()), size=TINY, color=DARK)
        if g:
            x1, x2 = gx(g - 1) + 2 * sw + 8, x - 3
            cv.text((x1 + x2) / 2, 7, title, size=TINY + 0.5, ha="center", weight="bold")
            cv.arrow((x1, 14), (x2, 14), lw=1.0, head=(3.6, 1.8))
            short = {"spec": r"run on $S_p$", "commit": rf"{D(7)}: re-run on $S$", "back": r"clone, replay $\Omega_8$"}[kind]
            cv.text((x1 + x2) / 2, 20, short, size=TINY - 0.4, ha="center", color=DARK)
    cv.text(0, 141, "Both columns are full states ($S_p$ = preemptive_state). Bold: changed in this step (small gray: before). "
            "Shaded: the speculative copy, discarded and re-cloned on backtrack.", size=TINY, color=MID)
    cv.save(os.path.join(out, f"{PFX}-c-timeline"))


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "."
    os.makedirs(out, exist_ok=True)
    for f in (fig_b, fig_c):
        f(out)
        print(f.__name__)
