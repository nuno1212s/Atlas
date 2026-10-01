"""CRUD cache: routes B (containers) and C (tables), with flow arrows and explicit changes.

    python3 crud_flow.py <output-dir>
"""
import os
import sys

from executor_model import CRUD, KEYS, TOMB
from flow_common import (D, PEND_FILL, badge, cell, changes, curved, header, key_row, marks_for,
                         parrow, region, strike, tx)
from paperfig import BODY, DARK, FULL, COLUMN, INK, LIGHT, MID, PAPER, SMALL, TINY, Canvas

PFX = "preemptive-context-cache-flow"

STEPS = [
    ("0", "start", tx(rf"{D(7)}, {D(8)}, {D(9)} speculated"), "start"),
    ("1", rf"speculate {D(10)}", r"$C \leftarrow C \oplus \Delta_{10}$", "spec"),
    ("2", rf"commit {D(7)}", r"$S \leftarrow S \oplus \Delta_7$, refold $C$", "commit"),
    ("3", rf"backtrack to {D(9)}", rf"drop {D(9)}, {D(10)}; refold $C$", "back"),
]


def step_notes(i):
    """The explicit list of what changed in step i, derived from the model."""
    st, prev = CRUD[i], CRUD[i - 1] if i else None
    kind = STEPS[i][3]
    if kind == "start":
        return [tx(rf"$C$ = {D(7)} ⊕ {D(8)} ⊕ {D(9)}"), "S: confirmed through seq 6", "staged: batch 12"]
    added = [q for q in st.seqs() if q not in prev.seqs()]
    removed = [q for q in prev.seqs() if q not in st.seqs()]
    log = "log: " + ", ".join([f"+ {D(q)}" for q in added] + [f"− {D(q)}" for q in removed])
    out = [log, tx(changes(prev.C, st.C, "C")), tx(changes(prev.S, st.S, "S"))]
    if kind == "spec":
        out.append(rf"replies of {D(10)} held")
    elif kind == "commit":
        out.append(rf"replies of {D(7)} → clients".replace("→", r"$\rightarrow$"))
    else:
        out.append("staged batch 12 evicted")
    return out


def log_rows(i):
    """(seq, delta, state) newest first, including entries that left the log in step i."""
    st, prev = CRUD[i], CRUD[i - 1] if i else None
    kind = STEPS[i][3]
    rows = {q: (d, "new" if prev and q not in prev.seqs() else None) for q, d, _ in st.log}
    if kind == "commit":
        rows[prev.log[0][0]] = (prev.log[0][1], "applied")
    if kind == "back":
        for q, d, _ in prev.log:
            if q not in st.seqs():
                rows[q] = (d, "dropped")
    return sorted(((q, d, s) for q, (d, s) in rows.items()), reverse=True)


# ---------------------------------------------------------------------------
# B1 — containers with flow arrows, 2 × 2
# ---------------------------------------------------------------------------
def b1_panel(cv, px, py, i):
    st, prev = CRUD[i], CRUD[i - 1] if i else None
    num, title, effect, kind = STEPS[i]
    header(cv, px, py + 5, num, title, effect)

    x0, p, w, h = px + 24, 20, 16, 10.5
    xs = [x0 + j * p + p / 2 for j in range(5)]
    col = dict(zip(KEYS, xs))
    gr = x0 + 5 * p
    slots = {10: py + 40, 9: py + 53, 8: py + 66, 7: py + 79}
    yC, yS = py + 100, py + 141

    region(cv, px, py + 23, gr - px + 6, 88, "pend", "pending · discardable")
    region(cv, px, py + 122, gr - px + 6, 27, "conf", "confirmed · authoritative")

    for q, d, state in log_rows(i):
        y = slots[q]
        ghost = state in ("applied", "dropped")
        cv.text(px + 3, y + 0.3, D(q), size=SMALL, color=MID if ghost else INK)
        for k, v in d.items():
            cell(cv, col[k], y, w, h, "D", v, "gone" if ghost else ("chg" if state == "new" else None))
        if state == "dropped":
            strike(cv, x0 + 2, y, gr - 2, y)
    cv.line([(x0, yC - 8.5), (gr, yC - 8.5)], lw=0.3, color=MID)

    chg, gone = marks_for(prev.C if prev else None, st.C)
    cv.text(px + 3, yC + 0.3, "$C$", size=SMALL)
    for k, v in st.C.items():
        cell(cv, col[k], yC, w, h, "C", v, chg.get(k))
    for k, v in gone.items():
        cell(cv, col[k], yC, w, h, "C", v, "gone")

    chg, gone = marks_for(prev.S if prev else None, st.S)
    cv.text(px + 3, yS + 0.3, "$S$", size=SMALL)
    for k, v in st.S.items():
        cell(cv, col[k], yS, w, h, "S", v, chg.get(k))
    key_row(cv, xs, yS + 12)

    # commit gate between the containers
    gx = x0 + 2.5 * p
    active = kind == "commit"
    cv.arrow((gx, py + 112), (gx, py + 121), lw=1.0 if active else 0.6, color=INK if active else LIGHT)

    # flows for this step
    if kind == "start":
        cv.bracket(gr + 1, slots[9] - 5, slots[7] + 5)
        parrow(cv, [(gr + 3.5, slots[7] + 5), (gr + 3.5, yC), (gr + 0.5, yC)], lw=0.7)
        cv.text(gr + 7, (slots[7] + yC) / 2 + 1, "fold,\noldest\nfirst", size=TINY, color=DARK, linespacing=1.1)
    elif kind == "spec":
        y = slots[10]
        cv.box(px + 158, y - 6, 44, 12, fill=PAPER, edge=INK, lw=0.6, ls=(0, (2, 1.1)), r=1.2)
        cv.text(px + 180, y + 0.3, "batch 10", size=TINY, ha="center")
        parrow(cv, [(px + 158, y), (gr + 1, y)], lw=0.9)
        cv.text((px + 158 + gr) / 2 + 2, y - 5, "execute", size=TINY, ha="center", color=DARK)
        parrow(cv, [(gr - 2, y + 5.5), (gr + 5, y + 8), (gr + 5, yC), (gr + 0.5, yC)], lw=0.9)
        cv.text(gr + 8, (y + yC) / 2 + 6, tx("⊕ merge"), size=TINY, color=DARK)
    elif kind == "commit":
        y = slots[7]
        parrow(cv, [(gr + 0.5, y), (gr + 6, y), (gr + 6, yS), (gr + 0.5, yS)], lw=1.1, head=(3.6, 1.8))
        cv.text(gr + 9, (y + yC) / 2 + 1, tx(rf"apply" + "\n" + D(7)), size=TINY, color=DARK, linespacing=1.1)
        parrow(cv, [(gr + 6, y), (px + 170, y)], lw=0.7)
        cv.text(px + 172, y + 0.3, "replies → clients", size=TINY, color=DARK)
    else:
        for q in (10, 9):
            parrow(cv, [(gr + 0.5, slots[q]), (gr + 16, slots[q])], lw=0.6, color=MID)
            cv.text(gr + 18, slots[q] + 0.3, "dropped", size=TINY, color=MID)
        y = slots[8]
        parrow(cv, [(gr + 0.5, y), (gr + 6, y), (gr + 6, yC), (gr + 0.5, yC)], lw=0.9)
        cv.text(gr + 9, (y + yC) / 2 + 2, "refold", size=TINY, color=DARK)

    # explicit list of changes (the replies arrow already says where committed replies go)
    notes = [n for n in step_notes(i) if not (kind == "commit" and n.startswith("replies"))]
    for j, line in enumerate(notes):
        cv.text(px + 164, py + 100 + j * 9, line, size=TINY, color=DARK)


def fig_b1(out):
    pw, ph = 248, 162
    cv = Canvas(FULL, 2 * ph + 4)
    for i in range(4):
        b1_panel(cv, (i % 2) * (pw + 9), 2 + (i // 2) * ph, i)
    cv.line([(FULL / 2, 4), (FULL / 2, 2 * ph)], lw=0.35, color=LIGHT)
    cv.line([(0, ph), (FULL, ph)], lw=0.35, color=LIGHT)
    cv.save(os.path.join(out, f"{PFX}-b1-containers-arrows"))


# ---------------------------------------------------------------------------
# B2 — one row per operation: before, the operation and its changes, after
# ---------------------------------------------------------------------------
def mini(cv, x, y, i, role):
    """A compact pending/confirmed pair. role = 'before' (mark what the step consumes) or 'after'."""
    st, prev = CRUD[i], CRUD[i - 1] if i else None
    kind = STEPS[i + 1][3] if role == "before" else STEPS[i][3]
    x0, p, w, h = x + 16, 15, 12.5, 9.5
    xs = [x0 + j * p + p / 2 for j in range(5)]
    col = dict(zip(KEYS, xs))
    gr = x0 + 5 * p
    slots = {10: y + 16, 9: y + 27, 8: y + 38, 7: y + 49}
    yC, yS = y + 65, y + 98
    region(cv, x, y, gr - x + 5, 75, "pend", "pending", title_size=TINY - 0.4)
    region(cv, x, y + 81, gr - x + 5, 24, "conf", "confirmed", title_size=TINY - 0.4)
    rows = log_rows(i) if role == "after" else [(q, d, None) for q, d, _ in reversed(st.log)]
    for q, d, state in rows:
        yy = slots[q]
        ghost = state in ("applied", "dropped")
        consumed = role == "before" and ((kind == "commit" and q == st.log[0][0]) or
                                         (kind == "back" and q >= 9))
        cv.text(x + 2, yy + 0.3, D(q), size=TINY, color=MID if ghost else INK)
        for k, v in d.items():
            mark = "gone" if ghost else ("chg" if state == "new" or consumed else None)
            cell(cv, col[k], yy, w, h, "D", v, mark, size=TINY - 0.4)
        if state == "dropped":
            strike(cv, x0 + 2, yy, gr - 2, yy)
    cv.line([(x0, yC - 7.5), (gr, yC - 7.5)], lw=0.3, color=MID)
    cv.text(x + 2, yC + 0.3, "$C$", size=TINY)
    chg, gone = marks_for(prev.C if (prev and role == "after") else None, st.C)
    for k, v in st.C.items():
        cell(cv, col[k], yC, w, h, "C", v, chg.get(k), size=TINY - 0.4)
    for k, v in gone.items():
        cell(cv, col[k], yC, w, h, "C", v, "gone", size=TINY - 0.4)
    cv.text(x + 2, yS + 0.3, "$S$", size=TINY)
    chg, _ = marks_for(prev.S if (prev and role == "after") else None, st.S)
    for k, v in st.S.items():
        cell(cv, col[k], yS, w, h, "S", v, chg.get(k), size=TINY - 0.4)
    key_row(cv, xs, yS + 10, size=TINY - 0.6)


def fig_b2(out):
    rowh = 122
    cv = Canvas(FULL, 12 + 3 * rowh)
    cv.text(47, 5, "before", size=SMALL, ha="center", color=MID)
    cv.text(FULL - 47, 5, "after", size=SMALL, ha="center", color=MID)
    for r, i in enumerate((1, 2, 3)):
        y = 14 + r * rowh
        mini(cv, 0, y, i - 1, "before")
        mini(cv, FULL - 97, y, i, "after")
        num, title, effect, _ = STEPS[i]
        mid = FULL / 2
        badge(cv, mid - 56, y + 28, num)
        cv.text(mid - 45, y + 28, title, size=BODY + 0.5, weight="bold")
        cv.arrow((104, y + 40), (FULL - 104, y + 40), lw=1.4, head=(5, 2.4))
        cv.text(mid, y + 48, effect, size=SMALL, ha="center", color=DARK)
        for j, line in enumerate(step_notes(i)):
            cv.text(132, y + 60 + j * 9.5, "•  " + line if not line.startswith("•") else line, size=TINY + 0.3, color=INK)
        if r:
            cv.line([(0, y - 4), (FULL, y - 4)], lw=0.35, color=LIGHT)
    cv.save(os.path.join(out, f"{PFX}-b2-before-after"))


# ---------------------------------------------------------------------------
# B3 — one mechanism map, numbered flows
# ---------------------------------------------------------------------------
def fig_b3(out):
    st = CRUD[0]
    d10 = CRUD[1].entry(10)
    cv = Canvas(COLUMN, 172)
    x0, p, w, h = 26, 19, 15, 10
    xs = [x0 + j * p + p / 2 for j in range(5)]
    col = dict(zip(KEYS, xs))
    gr = x0 + 5 * p
    yd, yC, y9, y8, y7, yS = 45, 64, 84, 97, 110, 150

    cv.box(x0, 2, 5 * p, 17, fill=PAPER, edge=INK, lw=0.6, ls=(0, (2, 1.1)), r=1.4)
    cv.text(x0 + 2.5 * p, 7, "batch 10", size=TINY, ha="center", weight="bold")
    cv.text(x0 + 2.5 * p, 14, "c ← 5,  e ← 8,  read e,  read b", size=TINY - 0.5, ha="center", color=DARK)

    region(cv, 0, 24, gr + 6, 96, "pend", "pending · discardable")
    region(cv, 0, 134, gr + 6, 24, "conf", "confirmed · authoritative")

    cv.text(3, yd + 0.3, r"$\delta_{10}$", size=SMALL)
    for k, v in d10.items():
        cell(cv, col[k], yd, w, h, "d", v)
    cv.text(3, yC + 0.3, "$C$", size=SMALL)
    for k, v in st.C.items():
        cell(cv, col[k], yC, w, h, "C", v)
    cv.line([(x0, 73), (gr, 73)], lw=0.3, color=MID)
    for q, y in ((9, y9), (8, y8), (7, y7)):
        cv.text(3, y + 0.3, D(q), size=SMALL)
        for k, v in st.entry(q).items():
            cell(cv, col[k], y, w, h, "D", v)
    cv.text(3, yS + 0.3, "$S$", size=SMALL)
    for k, v in st.S.items():
        cell(cv, col[k], yS, w, h, "S", v)
    key_row(cv, xs, yS + 11)

    # 1 execute
    cv.arrow((col["c"], 19), (col["c"], yd - h / 2 - 0.5), lw=0.9)
    # 2 reads: δ, then C, then S (right edge; the log is skipped)
    parrow(cv, [(gr + 0.5, yd), (gr + 6, yd), (gr + 6, yC), (gr + 0.5, yC)], lw=0.6, ls=(0, (2, 1.2)))
    parrow(cv, [(gr + 6, yC), (gr + 6, yS), (gr + 0.5, yS)], lw=0.6, ls=(0, (2, 1.2)))
    # 3 end of batch: δ10 merged into C (and appended as Δ10)
    for k in d10:
        cv.arrow((col[k], yd + h / 2 + 0.5), (col[k], yC - h / 2 - 0.5), lw=0.9)
    # 4 commit the head
    for k in st.entry(7):
        cv.arrow((col[k], y7 + h / 2 + 0.5), (col[k], 134), lw=1.1, head=(3.6, 1.8))
    # 5 backtrack cut
    cv.line([(x0 - 4, (y9 + y8) / 2), (gr + 4, (y9 + y8) / 2)], lw=0.9, ls=(0, (2.5, 1.8)))

    marks = [(col["c"] + 7, 31, "1"), (gr + 10, 100, "2"), (col["d"], (yd + yC) / 2, "3"),
             (col["c"], 125, "4"), (col["e"], (y9 + y8) / 2, "5")]
    for x, y, n in marks:
        badge(cv, x - 4, y, n, r=3.6)

    nl = "\n"
    notes = [
        ("1", 30, "execute batch 10;" + nl + "every write goes" + nl + "into δ10 only"),
        ("2", 52, "a read tries δ10," + nl + "then C, then S; the" + nl + "log is never read"),
        ("3", 76, tx("batch ends: δ10 is" + nl + rf"appended as {D(10)}," + nl + rf"and C ⊕= {D(10)}")),
        ("5", 101, tx(rf"backtrack to {D(9)}:" + nl + rf"drop {D(9)} and newer," + nl + "then refold C")),
        ("4", 127, tx(rf"commit {D(7)}, the head:" + nl + rf"S ⊕= {D(7)}, pop it," + nl + "refold C, send replies")),
    ]
    for n, y, text in notes:
        badge(cv, 138, y, n, r=3.6)
        cv.text(147, y - 3.5, text, size=TINY - 0.3, color=DARK, linespacing=1.15, va="top")
    cv.save(os.path.join(out, f"{PFX}-b3-mechanism"))


# ---------------------------------------------------------------------------
# C1 — version tables with arrows along each row, 2 × 2
# ---------------------------------------------------------------------------
def c1_panel(cv, px, py, i):
    st, prev = CRUD[i], CRUD[i - 1] if i else None
    num, title, effect, kind = STEPS[i]
    header(cv, px, py + 5, num, title, effect)
    xS = px + 25
    xq = {7: px + 54, 8: px + 74, 9: px + 94, 10: px + 114}
    xC = px + 138
    top, yh = py + 24, py + 43
    ry = {k: py + 50 + r * 11 for r, k in enumerate(KEYS)}
    bot = ry["e"] + 6
    cv.box(px + 40, top, xC + 13 - (px + 40), bot - top, fill=PEND_FILL, edge="none", lw=0, r=1.2, z=0.2)
    for x in (px + 38.8, px + 40.4):
        cv.line([(x, top), (x, bot)], lw=0.5)
    cv.line([(px, top), (xC + 13, top)], lw=0.8)
    cv.line([(px, yh), (xC + 13, yh)], lw=0.45)
    cv.line([(px, bot), (xC + 13, bot)], lw=0.8)
    cv.text(px, top + 5, "confirmed", size=TINY - 0.6, weight="bold")
    cv.text((xq[7] + xq[10]) / 2, top + 5, "pending log", size=TINY - 0.6, ha="center", weight="bold")
    cv.text(xC, top + 5, "cache", size=TINY - 0.6, ha="center", weight="bold")
    cv.text(xS, top + 13.5, "$S$", size=TINY, ha="center")
    cv.text(xC, top + 13.5, "$C$", size=TINY, ha="center")

    rows = {q: (d, s) for q, d, s in log_rows(i)}
    for q, x in xq.items():
        d, s = rows.get(q, (None, None))
        cv.text(x, top + 13.5, D(q), size=TINY, ha="center",
                color=MID if s in ("applied", "dropped") or d is None else INK,
                weight="bold" if s == "new" else "normal")
    for k in KEYS:
        cv.text(px + 3, ry[k], f"${k}$", size=TINY, color=MID)

    # S column, with transitions written in place
    for k in KEYS:
        new, old = st.S.get(k), prev.S.get(k) if prev else st.S.get(k)
        if new == old:
            if new is not None:
                cv.text(xS, ry[k], new, size=TINY, ha="center")
        else:
            cv.text(xS - 6, ry[k], old or "–", size=TINY, ha="center", color=MID)
            if old:
                strike(cv, xS - 8.5, ry[k] + 2, xS - 3.5, ry[k] - 2, lw=0.5)
            cv.text(xS + 6, ry[k], new, size=TINY, ha="center", weight="bold")
    # log cells
    for q, (d, s) in rows.items():
        ghost = s in ("applied", "dropped")
        for k, v in d.items():
            cv.text(xq[q], ry[k], tx(v), size=TINY, ha="center", color=MID if ghost else INK,
                    weight="bold" if s == "new" else "normal")
        if s == "dropped":
            cv.line([(xq[q], top + 9), (xq[q], bot - 1)], lw=0.6, color=MID, z=6)
    # C column
    chg, gone = marks_for(prev.C if prev else None, st.C)
    for k, v in st.C.items():
        cell(cv, xC, ry[k], 13, 8.8, "C", v, chg.get(k))
    for k, v in gone.items():
        cell(cv, xC, ry[k], 13, 8.8, "C", v, "gone")
        strike(cv, xC - 4, ry[k] + 3, xC + 4, ry[k] - 3, lw=0.5)

    # arrows along rows
    newest = {}
    for q, d, _ in st.log:
        for k in d:
            newest[k] = q
    if kind in ("start", "spec", "back"):
        for k, q in newest.items():
            if kind != "start" and k not in chg and not (kind == "back" and k in st.C):
                continue
            cv.arrow((xq[q] + 5, ry[k]), (xC - 7.5, ry[k]), lw=0.5, color=DARK, head=(2.6, 1.2))
    if kind == "commit":
        for k in prev.log[0][1]:
            curved(cv, (xq[7] - 5, ry[k] - 1), (xS + 11, ry[k] - 1), rad=0.55, lw=0.8)
        cv.text(xq[7], bot + 5, tx(rf"{D(7)} → S"), size=TINY - 0.4, ha="center", color=DARK)

    for j, line in enumerate(step_notes(i)):
        cv.text(xC + 20, py + 52 + j * 9, line, size=TINY, color=DARK)


def fig_c1(out):
    pw, ph = 248, 116
    cv = Canvas(FULL, 2 * ph + 2)
    for i in range(4):
        c1_panel(cv, (i % 2) * (pw + 9), 1 + (i // 2) * ph, i)
    cv.line([(FULL / 2, 4), (FULL / 2, 2 * ph)], lw=0.35, color=LIGHT)
    cv.line([(0, ph - 2), (FULL, ph - 2)], lw=0.35, color=LIGHT)
    cv.save(os.path.join(out, f"{PFX}-c1-table-arrows"))


# ---------------------------------------------------------------------------
# C2 — one timeline table: S and C side by side after every step
# ---------------------------------------------------------------------------
def fig_c2(out):
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
        st, prev = CRUD[g], CRUD[g - 1] if g else None
        x = gx(g)
        xS, xC = x + sw / 2, x + sw + 4 + sw / 2
        cv.box(x + sw + 2, top, sw + 4, bot - top, fill=PEND_FILL, edge="none", lw=0, r=1, z=0.2)
        badge(cv, x, 7, num)
        cv.text(x + 11, 7, title if g == 0 else "", size=BODY, weight="bold")
        cv.text(x, 17, "state after step " + num, size=TINY - 0.4, color=MID)
        cv.text(xS, 31, "$S$ confirmed", size=TINY, ha="center")
        cv.text(xC, 31, "$C$ cache", size=TINY, ha="center")
        for part, (xx, cur, before, kindc) in {"S": (xS, st.S, prev.S if prev else None, "S"),
                                                 "C": (xC, st.C, prev.C if prev else None, "C")}.items():
            chg, gone = marks_for(before, cur)
            for k, v in cur.items():
                cell(cv, xx, ry[k], sw - 8, 9.5, kindc, v, chg.get(k))
                if k in chg and before and before.get(k) is not None:
                    cv.text(xx - sw / 2 + 1, ry[k] - 5.5, before[k] if before[k] != TOMB else "⊥",
                            size=TINY - 0.6, color=MID)
            for k, v in gone.items():
                cell(cv, xx, ry[k], sw - 8, 9.5, kindc, v, "gone")
                strike(cv, xx - 5, ry[k] + 3.5, xx + 5, ry[k] - 3.5, lw=0.5)
        cv.text(x, bot + 8, "log: " + " ".join(D(q) for q in st.seqs()), size=TINY, color=DARK)
        replies = {"start": "held", "spec": "held"}.get(kind)
        held = ", ".join(D(q) for q in st.seqs())
        sent = rf"sent {D(7)}; " if kind == "commit" else ""
        cv.text(x, bot + 18, f"replies: {sent}held {held}", size=TINY, color=DARK)
        if g:
            # the transition from the previous group
            x1, x2 = gx(g - 1) + 2 * sw + 8, x - 3
            cv.text((x1 + x2) / 2, 7, title, size=TINY + 0.5, ha="center", weight="bold")
            cv.arrow((x1, 14), (x2, 14), lw=1.0, head=(3.6, 1.8))
            cv.text((x1 + x2) / 2, 20, effect, size=TINY - 0.8, ha="center", color=DARK)
    cv.text(0, 141, "Bold: changed in this step (small gray: the value before). Dashed and struck: "
            "removed. Shaded columns are speculative and discardable.", size=TINY, color=MID)
    cv.save(os.path.join(out, f"{PFX}-c2-timeline"))


FIGS = [fig_b1, fig_b2, fig_b3, fig_c1, fig_c2]

if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "."
    os.makedirs(out, exist_ok=True)
    for f in FIGS:
        f(out)
        print(f.__name__)
