"""Alternative presentations of the CRUD cache storyboard (confirmed state vs. pending cache).

    python3 storyboard_alternatives.py <output-dir>

Every alternative draws the same four states, computed by a tiny model of
`CachingPreemptiveState` rather than typed in by hand:

  0 start           S = {a:1, b:2, c:3, e:7}; log Δ7 {a:5, d:9}, Δ8 {b:⊥}, Δ9 {a:6, c:4}
  1 speculate Δ10      append Δ10 {c:5, e:8};                C ⊕= Δ10
  2 commit Δ7         S ⊕= Δ7; pop Δ7;                  refold C
  3 backtrack to Δ9   drop Δ9, Δ10, staged batch 12;   refold C
"""
import os
import sys
from dataclasses import dataclass, field, replace

from matplotlib.patches import Circle, Ellipse, Polygon, Rectangle

from paperfig import (BODY, COLUMN, DARK, FULL, INK, LIGHT, MID, PALE, PAPER, SMALL, TINY,
                      Canvas)

KEYS = "abcde"
TOMB = "⊥"
PFX = "preemptive-context-cache-alt"
PEND_FILL = "#eeeeee"


# ---------------------------------------------------------------------------
# Model (mirrors pending_state.rs: execute_speculatively, handle_update_confirmed, backtrack)
# ---------------------------------------------------------------------------
@dataclass(frozen=True)
class State:
    S: dict
    log: tuple            # ((seq, delta), ...) oldest first
    staged: tuple
    nc: int
    np: int

    @property
    def C(self):
        acc = {}
        for _, d in self.log:
            acc.update(d)
        return acc

    def delta(self, seq):
        return dict(self.log)[seq]


def speculate(st, seq, delta):
    return replace(st, log=st.log + ((seq, delta),), np=seq + 1)


def commit(st):
    seq, d = st.log[0]
    S = dict(st.S)
    for k, v in d.items():
        if v == TOMB:
            S.pop(k, None)
        else:
            S[k] = v
    return replace(st, S=S, log=st.log[1:], nc=seq + 1)


def backtrack(st, k):
    log = tuple(e for e in st.log if e[0] < k)
    return replace(st, log=log, staged=tuple(s for s in st.staged if s < k),
                   np=log[-1][0] + 1 if log else st.nc)


S0 = State(S={"a": "1", "b": "2", "c": "3", "e": "7"},
           log=((7, {"a": "5", "d": "9"}), (8, {"b": TOMB}), (9, {"a": "6", "c": "4"})),
           staged=(12,), nc=7, np=10)
S1 = speculate(S0, 10, {"c": "5", "e": "8"})
S2 = commit(S1)
S3 = backtrack(S2, 9)


@dataclass
class Step:
    num: str
    title: str
    effect: str
    st: State
    prev: State = None
    kind: str = "start"            # start | speculate | commit | backtrack
    new: set = field(default_factory=set)
    gone: dict = field(default_factory=dict)   # seq -> delta, for entries that left the log

    def S_changed(self):
        return {k for k in KEYS if self.prev and self.st.S.get(k) != self.prev.S.get(k)}

    def C_changed(self):
        return {k for k in self.st.C if self.prev and self.st.C.get(k) != self.prev.C.get(k)}

    def C_removed(self):
        return {k for k in (self.prev.C if self.prev else {}) if k not in self.st.C}


def steps():
    out = [Step("0", "start", r"$\Delta_7, \Delta_8, \Delta_9$ speculated", S0),
           Step("1", r"speculate $\Delta_{10}$", r"$C \leftarrow C \oplus \Delta_{10}$", S1, S0, "speculate"),
           Step("2", r"commit $\Delta_7$", r"$S \leftarrow S \oplus \Delta_7$, refold $C$", S2, S1, "commit"),
           Step("3", r"backtrack to $\Delta_9$", r"drop $\Delta_9, \Delta_{10}$, refold $C$", S3, S2, "backtrack")]
    for s in out[1:]:
        before, after = dict(s.prev.log), dict(s.st.log)
        s.new = set(after) - set(before)
        s.gone = {q: before[q] for q in before if q not in after}
    return out


STEPS = steps()


def v(x):
    return r"$\bot$" if x == TOMB else x


def step_badge(cv, x, y, num, r=4.2):
    cv.ax.add_patch(Circle((x + r, y), r, facecolor=INK, edgecolor=INK, lw=0, zorder=6))
    cv.text(x + r, y + 0.3, num, size=TINY, ha="center", color=PAPER, weight="bold", z=7)


def kv_cell(cv, cx, cy, w, h, text, fill=PAPER, edge=INK, lw=0.6, ls="-", tc=INK, size=SMALL):
    cv.box(cx - w / 2, cy - h / 2, w, h, fill=fill, edge=edge, lw=lw, ls=ls, r=1.2)
    if text is not None:
        cv.text(cx, cy + 0.3, v(text), size=size, ha="center", color=tc)


def slot(cv, cx, cy, w, h):
    cv.box(cx - w / 2, cy - h / 2, w, h, fill="none", edge=PALE, lw=0.5, r=1.2, z=1)


def strike(cv, x0, y0, x1, y1, lw=0.7):
    cv.line([(x0, y0), (x1, y1)], lw=lw, color=MID, z=8)


# ---------------------------------------------------------------------------
# A. Swimlanes: time runs down, the two structures live in separate lanes
# ---------------------------------------------------------------------------
def alt_swimlanes(out):
    rowh, top = 52, 32
    cv = Canvas(FULL, top + 4 * rowh + 4)
    xl0, xl1 = 84, 204          # confirmed lane
    xs0, xs1 = 212, FULL        # speculative lane
    kw = 22                     # key cell pitch

    # lanes
    cv.box(xl0, 0, xl1 - xl0, cv.h, fill=PAPER, edge=INK, lw=0.8, r=2, z=0.3)
    cv.box(xs0, 0, xs1 - xs0 - 0.5, cv.h, fill=PEND_FILL, edge=MID, lw=0.8, ls=(0, (3, 2)), r=2, z=0.3)
    cv.text(xl0 + 5, 8, "CONFIRMED", size=SMALL, weight="bold")
    cv.text(xl0 + 5, 16, "$S$: authoritative, written only by commit", size=TINY, color=DARK)
    cv.text(xs0 + 5, 8, "SPECULATIVE", size=SMALL, weight="bold")
    cv.text(xs0 + 5, 16, "log + cache $C$: discardable, never seen by clients", size=TINY, color=DARK)

    sx = lambda k: xl0 + 13 + KEYS.index(k) * kw
    slot_x = {7: xs0 + 22, 8: xs0 + 58, 9: xs0 + 94, 10: xs0 + 130}
    cx0 = xs0 + 170
    cxk = lambda k: cx0 + 8 + KEYS.index(k) * kw
    for k in KEYS:
        cv.text(sx(k), 27, f"${k}$", size=TINY, ha="center", color=MID)
        cv.text(cxk(k), 27, f"${k}$", size=TINY, ha="center", color=MID)
    for q, x in slot_x.items():
        cv.text(x, 27, f"seq {q}", size=TINY, ha="center", color=MID)

    for i, s in enumerate(STEPS):
        y0 = top + i * rowh
        yc = y0 + rowh / 2
        if i:
            cv.line([(2, y0), (FULL - 2, y0)], lw=0.35, color=LIGHT, z=0.4)
        step_badge(cv, 0, y0 + 9, s.num)
        cv.text(11, y0 + 9, s.title, size=BODY, weight="bold")
        cv.text(0, y0 + 20, s.effect, size=TINY, color=DARK)
        if s.kind == "backtrack":
            cv.text(0, y0 + 29, "staged batch 12 evicted", size=TINY, color=DARK)

        # confirmed lane: S
        chg = s.S_changed()
        for k in KEYS:
            if k in s.st.S:
                kv_cell(cv, sx(k), yc, 18, 14, s.st.S[k], lw=1.3 if k in chg else 0.6)
            else:
                slot(cv, sx(k), yc, 18, 14)

        # speculative lane: log cards, seq-aligned, head nearest the confirmed lane
        cards = dict(s.st.log)
        for q, x in slot_x.items():
            if q in cards:
                card(cv, x, yc, q, cards[q], bold=q in s.new)
            elif q in s.gone:
                card(cv, x, yc, q, s.gone[q], ghost=True, struck=s.kind == "backtrack")
        if s.kind == "commit":
            cv.arrow((slot_x[7] - 16, yc), (xl1 - 3, yc), lw=1.0, head=(3.6, 1.8))

        # C
        cv.text(cx0 - 12, yc + 0.3, "=", size=BODY, ha="center", color=DARK)
        chg, rem = s.C_changed(), s.C_removed()
        for k in KEYS:
            if k in s.st.C:
                kv_cell(cv, cxk(k), yc, 18, 14, s.st.C[k], fill=DARK, edge=INK if k in chg else DARK,
                        lw=1.3 if k in chg else 0.4, tc=PAPER)
            elif k in rem:
                kv_cell(cv, cxk(k), yc, 18, 14, s.prev.C[k], fill=PAPER, edge=MID, ls=(0, (1.5, 1.2)), tc=MID)
            else:
                slot(cv, cxk(k), yc, 18, 14)
    cv.text(cxk("c"), 8, "$C$ = fold of the log", size=SMALL, ha="center", weight="bold")
    cv.save(os.path.join(out, f"{PFX}-a-swimlanes"))


def card(cv, cx, cy, seq, delta, bold=False, ghost=False, struck=False, w=30, line=7.2):
    n = len(delta)
    h = 11 + n * line
    x, y = cx - w / 2, cy - h / 2
    cv.box(x, y, w, h, fill=PAPER if ghost else LIGHT, edge=MID if ghost else INK,
           lw=1.3 if bold else 0.5, ls=(0, (1.5, 1.2)) if ghost else "-", r=1.4)
    tc = MID if ghost else INK
    cv.text(cx, y + 5.2, rf"$\Delta_{{{seq}}}$", size=TINY, ha="center", color=tc)
    cv.line([(x + 2, y + 9.5), (x + w - 2, y + 9.5)], lw=0.3, color=MID if not ghost else PALE)
    for j, (k, val) in enumerate(sorted(delta.items())):
        cv.text(cx, y + 14 + j * line, f"{k} $\\leftarrow$ " + (r"$\bot$" if val == TOMB else val),
                size=TINY, ha="center", color=tc)
    if struck:
        strike(cv, x + 1, y + h - 1, x + w - 1, y + 1)
    return h


# ---------------------------------------------------------------------------
# B. Containers: the current storyboard, with the two structures boxed apart
# ---------------------------------------------------------------------------
def alt_containers(out):
    p, w, h = 16.5, 14, 10.5
    lw_, pw = 16, 16 + 5 * 16.5 + 26
    gap = (FULL - 4 * pw) / 3
    cv = Canvas(FULL, 176)
    for i, s in enumerate(STEPS):
        px = i * (pw + gap)
        x0 = px + lw_
        col = lambda k: x0 + KEYS.index(k) * p + p / 2
        step_badge(cv, px, 6, s.num)
        cv.text(px + 11, 6, s.title, size=BODY, weight="bold")
        cv.text(px, 16, s.effect, size=TINY, color=DARK)

        # pending container
        py0, py1 = 24, 110
        cv.box(px, py0, pw - 2, py1 - py0, fill=PEND_FILL, edge=MID, lw=0.7, ls=(0, (3, 2)), r=2, z=0.3)
        cv.text(px + 3, py0 + 6, "pending · discardable", size=TINY, color=DARK, weight="bold")
        slots = {10: py0 + 20, 9: py0 + 33, 8: py0 + 46, 7: py0 + 59}
        logd = dict(s.st.log)
        for q, y in slots.items():
            d = logd.get(q) or s.gone.get(q)
            if d is None:
                continue
            ghost = q in s.gone
            cv.text(px + 2, y + 0.3, rf"$\Delta_{{{q}}}$", size=SMALL, color=MID if ghost else INK)
            for k, val in d.items():
                if ghost:
                    kv_cell(cv, col(k), y, w, h, val, fill=PAPER, edge=MID, ls=(0, (1.5, 1.2)), tc=MID, size=TINY)
                else:
                    kv_cell(cv, col(k), y, w, h, val, fill=LIGHT, edge=INK if q in s.new else MID,
                            lw=1.3 if q in s.new else 0.4, size=TINY)
            if ghost and s.kind == "backtrack":
                cv.line([(x0, y), (x0 + 5 * p, y)], lw=0.6, color=MID, z=8)
            note = {"commit": "applied ↓", "backtrack": "dropped"}.get(s.kind) if ghost else ("new" if q in s.new else None)
            if note:
                cv.text(x0 + 5 * p + 1, y, note, size=TINY - 0.4, color=MID)
        yc = py0 + 76
        cv.line([(x0, yc - 8.5), (x0 + 5 * p, yc - 8.5)], lw=0.3, color=MID)
        cv.text(px + 2, yc + 0.3, "$C$", size=SMALL)
        chg, rem = s.C_changed(), s.C_removed()
        for k in KEYS:
            if k in s.st.C:
                kv_cell(cv, col(k), yc, w, h, s.st.C[k], fill=DARK, edge=INK if k in chg else DARK,
                        lw=1.3 if k in chg else 0.4, tc=PAPER, size=TINY)
            elif k in rem:
                kv_cell(cv, col(k), yc, w, h, s.prev.C[k], fill=PAPER, edge=MID, ls=(0, (1.5, 1.2)), tc=MID, size=TINY)

        # the boundary
        cv.arrow((x0 + 2.5 * p, py1 + 1), (x0 + 2.5 * p, py1 + 11), lw=0.8,
                 color=INK if s.kind == "commit" else LIGHT)
        head = 7 if s.kind == "commit" else s.st.log[0][0]
        cv.text(x0 + 2.5 * p + 4, py1 + 6, rf"commit $\Delta_{{{head}}}$" + ("" if s.kind == "commit" else " (next)"),
                size=TINY, color=DARK if s.kind == "commit" else MID)

        # confirmed container
        cy0 = py1 + 13
        cv.box(px, cy0, pw - 2, 30, fill=PAPER, edge=INK, lw=0.9, r=2, z=0.3)
        cv.text(px + 3, cy0 + 6, "confirmed · authoritative", size=TINY, weight="bold")
        ys = cy0 + 18
        cv.text(px + 2, ys + 0.3, "$S$", size=SMALL)
        chg = s.S_changed()
        for k in KEYS:
            if k in s.st.S:
                kv_cell(cv, col(k), ys, w, h, s.st.S[k], lw=1.3 if k in chg else 0.6, size=TINY)
        for k in KEYS:
            cv.text(col(k), cy0 + 36, f"${k}$", size=TINY, ha="center", color=MID)
    cv.save(os.path.join(out, f"{PFX}-b-containers"))


# ---------------------------------------------------------------------------
# C. Version table: one row per key, read left to right as that key's history
# ---------------------------------------------------------------------------
def alt_table(out):
    cols = [("S", "confirmed")] + [(q, "log") for q in (7, 8, 9, 10)] + [("C", "cache")]
    kcol, cw = 9, 14
    tw = kcol + cw * len(cols) + 10
    gap = (FULL - 4 * tw) / 3
    cv = Canvas(FULL, 112)
    for i, s in enumerate(STEPS):
        tx = i * (tw + gap)
        step_badge(cv, tx, 6, s.num)
        cv.text(tx + 11, 6, s.title, size=BODY, weight="bold")
        cv.text(tx, 16, s.effect, size=TINY, color=DARK)

        xs, x = [], tx + kcol
        for j, (c, grp) in enumerate(cols):
            if j in (1, 5):
                x += 3
            xs.append(x + cw / 2)
            x += cw
        ytop, yh, rh = 25, 44, 10.5
        # group headers
        cv.text(tx, ytop + 4, "confirmed", size=TINY - 0.6, weight="bold")
        cv.text((xs[1] + xs[4]) / 2, ytop + 4, "pending log", size=TINY - 0.6, ha="center", weight="bold")
        cv.text(xs[5], ytop + 4, "cache", size=TINY - 0.6, ha="center", weight="bold")
        # background for the speculative side
        cv.box(xs[1] - cw / 2 - 1.5, ytop - 1, xs[5] - xs[1] + cw + 3, (yh + 5 * rh + 1) - (ytop - 1),
               fill=PEND_FILL, edge="none", lw=0, r=1.5, z=0.2)
        cv.line([(tx, ytop - 1), (xs[5] + cw / 2 + 1.5, ytop - 1)], lw=0.8)
        cv.line([(tx, yh), (xs[5] + cw / 2 + 1.5, yh)], lw=0.5)
        # double rule between confirmed and pending
        for dx in (-0.8, 0.8):
            xb = xs[1] - cw / 2 - 1.5 + dx
            cv.line([(xb, ytop - 1), (xb, yh + 5 * rh + 1)], lw=0.5, z=3)
        logd = dict(s.st.log)
        for j, (c, grp) in enumerate(cols):
            lab = {"S": "$S$", "C": "$C$"}.get(c) or rf"$\Delta_{{{c}}}$"
            gone = c in s.gone
            cv.text(xs[j], ytop + 13, lab, size=TINY, ha="center",
                    color=MID if gone else INK, weight="bold" if c in s.new else "normal")
        for r_, k in enumerate(KEYS):
            y = yh + 5 + r_ * rh
            cv.text(tx + 2, y, f"${k}$", size=TINY, color=MID)
            cells = [s.st.S.get(k)] + [(logd.get(q) or s.gone.get(q) or {}).get(k) for q in (7, 8, 9, 10)] + [s.st.C.get(k)]
            for j, val in enumerate(cells):
                c = cols[j][0]
                if val is None:
                    if c == "C" and k in s.C_removed():
                        cv.text(xs[j], y + 0.3, v(s.prev.C[k]), size=TINY, ha="center", color=LIGHT)
                        strike(cv, xs[j] - 3, y + 2.5, xs[j] + 3, y - 2.5, lw=0.5)
                    continue
                gone = c in s.gone
                bold = (c == "S" and k in s.S_changed()) or (c == "C" and k in s.C_changed()) or c in s.new
                if c == "C":
                    cv.box(xs[j] - 5.5, y - 4.2, 11, 8.4, fill=DARK, edge=INK if bold else DARK,
                           lw=1.2 if bold else 0.3, r=1)
                    cv.text(xs[j], y + 0.3, v(val), size=TINY, ha="center", color=PAPER)
                else:
                    cv.text(xs[j], y + 0.3, v(val), size=TINY, ha="center",
                            color=MID if gone else INK, weight="bold" if bold else "normal")
        yb = yh + 5 * rh + 1
        cv.line([(tx, yb), (xs[5] + cw / 2 + 1.5, yb)], lw=0.8)
        for q in s.gone:
            j = [c for c, _ in cols].index(q)
            if s.kind == "backtrack":
                cv.line([(xs[j], ytop + 9), (xs[j], yb - 1)], lw=0.6, color=MID, z=6)
            else:
                cv.text(xs[j], yb + 5, r"$\rightarrow S$", size=TINY - 0.6, ha="center", color=DARK)
    cv.text(0, 107, "Read a row left to right: a key's confirmed value, then its pending writes in "
            "sequence order; C holds the last one.", size=TINY, color=MID)
    cv.save(os.path.join(out, f"{PFX}-c-version-table"))


# ---------------------------------------------------------------------------
# D. Version chains: one annotated picture, operations drawn as cuts
# ---------------------------------------------------------------------------
def alt_chains(out):
    cv = Canvas(COLUMN, 160)
    st = S1                                 # the richest state: 7..10 pending
    colx = {"S": 38, 7: 88, 8: 116, 9: 144, 10: 172, 12: 222}
    y0, rh = 36, 16
    ky = lambda k: y0 + KEYS.index(k) * rh
    w, h = 17, 11

    cv.box(colx[7] - 14, 13, colx[10] - colx[7] + 28, 5 * rh + 26, fill=PEND_FILL, edge=MID,
           lw=0.6, ls=(0, (3, 2)), r=2, z=0.2)
    cv.text(colx["S"], 6, "confirmed", size=SMALL, ha="center", weight="bold")
    cv.text((colx[7] + colx[10]) / 2, 6, "pending log (discardable)", size=SMALL, ha="center", weight="bold")
    cv.text(colx[12], 6, "staged", size=SMALL, ha="center", color=MID)
    cv.text(colx["S"], 20, "$S$", size=TINY, ha="center", color=DARK)
    for q in (7, 8, 9, 10):
        cv.text(colx[q], 20, rf"$\Delta_{{{q}}}$", size=TINY, ha="center", color=DARK)
    cv.text(colx[12], 20, "batch 12", size=TINY, ha="center", color=MID)

    logd = dict(st.log)
    C = st.C
    for k in KEYS:
        y = ky(k)
        cv.text(4, y + 0.3, f"${k}$", size=SMALL, color=MID)
        chain = []
        if k in st.S:
            chain.append(("S", st.S[k]))
        chain += [(q, logd[q][k]) for q in (7, 8, 9, 10) if k in logd[q]]
        for idx, (c, val) in enumerate(chain):
            x = colx[c]
            head = c != "S" and idx == len(chain) - 1
            if c == "S":
                kv_cell(cv, x, y, w, h, val, lw=0.7, tc=MID if len(chain) > 1 else INK,
                        edge=MID if len(chain) > 1 else INK)
            elif head:
                kv_cell(cv, x, y, w, h, val, fill=DARK, edge=DARK, tc=PAPER)
            else:
                kv_cell(cv, x, y, w, h, val, fill=LIGHT, edge=MID, lw=0.4, tc=MID)
            if idx:
                px = colx[chain[idx - 1][0]]
                cv.arrow((px + w / 2 + 0.5, y), (x - w / 2 - 0.5, y), lw=0.5, head=(2.6, 1.2), color=MID)
    cv.box(colx[12] - 11, ky("a") - 6, 22, 4 * rh + 12, fill="none", edge=MID, lw=0.5, ls=(0, (1.5, 1.2)))
    cv.text(colx[12], ky("c") + 0.3, "batch\nonly,\nnot yet\nexecuted", size=TINY - 0.8, ha="center",
            color=MID, linespacing=1.1)

    yb = y0 + 5 * rh - 4
    xf = (colx["S"] + colx[7]) / 2 + 2
    cv.line([(xf, 14), (xf, yb + 8)], lw=1.3)
    xc = (colx[8] + colx[9]) / 2
    cv.line([(xc, 14), (xc, yb + 8)], lw=0.9, ls=(0, (2.5, 1.8)))
    # key
    yk = yb + 18
    cv.line([(0, yk), (10, yk)], lw=1.3)
    cv.text(14, yk, r"commit frontier: commit $\Delta_7$ moves it one column right, folding $\Delta_7$ into $S$",
            size=TINY, color=DARK)
    cv.line([(0, yk + 10), (10, yk + 10)], lw=0.9, ls=(0, (2.5, 1.8)))
    cv.text(14, yk + 10, r"backtrack to $\Delta_9$: drop every version right of the cut", size=TINY, color=DARK)
    cv.box(2, yk + 16, 8, 7, fill=DARK, edge=DARK, lw=0.3, r=1)
    cv.text(14, yk + 20, "newest pending version of a key: exactly what $C$ holds", size=TINY, color=DARK)
    cv.save(os.path.join(out, f"{PFX}-d-version-chains"))


# ---------------------------------------------------------------------------
# E. Pipeline: batches flow through the log into the confirmed store
# ---------------------------------------------------------------------------
def alt_pipeline(out):
    cv = Canvas(FULL, 124)
    ymid = 72
    st = S0

    # incoming
    cv.text(0, ymid - 20, "consensus", size=SMALL, weight="bold")
    cv.text(0, ymid - 11, "proposal accepted", size=TINY, color=DARK)
    cv.arrow((0, ymid - 3), (46, ymid - 3), lw=0.8)
    cv.text(0, ymid + 10, "reorder buffer", size=TINY, color=MID)
    kv_cell(cv, 22, ymid + 21, 26, 11, None, fill=PAPER, edge=MID, ls=(0, (1.5, 1.2)))
    cv.text(22, ymid + 21.3, "batch 12", size=TINY - 0.8, ha="center", color=MID)

    # executing batch
    bx = 72
    cv.box(bx - 22, ymid - 22, 44, 38, fill=PAPER, edge=INK, lw=0.6, ls=(0, (2.2, 1.2)), r=1.8)
    cv.text(bx, ymid - 15, r"batch 10 · $\delta_{10}$", size=TINY, ha="center")
    cv.text(bx, ymid - 5, "writes → δ", size=TINY, ha="center", color=DARK)
    cv.text(bx, ymid + 5, "reads: δ, C, S", size=TINY, ha="center", color=DARK)
    cv.arrow((bx + 22, ymid - 3), (126, ymid - 3), lw=0.8)
    cv.text(110, ymid - 8, r"append $\Delta_{10}$", size=TINY, ha="center", color=DARK)

    # speculative region
    rx0, rx1 = 128, 330
    cv.box(rx0, 8, rx1 - rx0, 104, fill=PEND_FILL, edge=MID, lw=0.8, ls=(0, (3, 2)), r=2.5, z=0.3)
    cv.text(rx0 + 5, 16, "SPECULATIVE · discardable", size=SMALL, weight="bold")
    # log cards: newest left, head right (nearest the store)
    order = [q for q, _ in reversed(st.log)]
    cx = {q: rx0 + 40 + i * 44 for i, q in enumerate(order)}
    for q in order:
        card(cv, cx[q], ymid, q, st.delta(q), w=32)
    cv.text(cx[order[0]], ymid + 24, "tail", size=TINY, ha="center", color=MID)
    cv.text(cx[order[-1]], ymid + 24, "head", size=TINY, ha="center", color=MID)
    cv.text((cx[order[0]] + cx[order[-1]]) / 2, ymid + 34, r"pending log: batch, replies and $\Delta$ per entry",
            size=TINY, ha="center", color=DARK)
    # C over the log
    cy = 36
    cxk = lambda k: rx0 + 118 + KEYS.index(k) * 17
    cv.text(rx0 + 8, cy, "$C$ = fold of the log", size=TINY, color=DARK)
    for k in KEYS:
        if k in st.C:
            kv_cell(cv, cxk(k), cy, 14, 11, st.C[k], fill=DARK, edge=DARK, tc=PAPER, size=TINY)
        cv.text(cxk(k), cy - 9.5, f"${k}$", size=TINY - 0.6, ha="center", color=MID)
    for q in order:
        cv.line([(cx[q], ymid - 17), (cx[q], cy + 12)], lw=0.4, color=MID, ls=(0, (1.2, 1.2)))
    # backtrack cut between 8 and 9
    xc = (cx[9] + cx[8]) / 2
    cv.line([(xc, ymid - 18), (xc, ymid + 19)], lw=0.9, ls=(0, (2.5, 1.8)))
    cv.text(xc, ymid - 24, r"backtrack to $\Delta_9$ cuts here", size=TINY, ha="center", color=DARK,
            bbox=dict(boxstyle="round,pad=0.2,rounding_size=0.8", fc=PEND_FILL, ec="none"))

    # commit
    cv.arrow((rx1 + 2, ymid), (rx1 + 48, ymid), lw=1.2, head=(4, 2))
    cv.text(rx1 + 25, ymid - 8, r"commit $\Delta_7$", size=SMALL, ha="center", weight="bold")
    cv.text(rx1 + 25, ymid + 6, r"apply $\Delta_7$," + "\n" + r"send its replies", size=TINY, ha="center", color=DARK,
            linespacing=1.1, va="top")

    # confirmed store (a cylinder)
    sx0, sw = 384, 118
    top = 30
    cv.ax.add_patch(Rectangle((sx0, top), sw, 74, facecolor=PAPER, edgecolor="none", zorder=2))
    cv.line([(sx0, top), (sx0, top + 74)], lw=0.9)
    cv.line([(sx0 + sw, top), (sx0 + sw, top + 74)], lw=0.9)
    cv.ax.add_patch(Ellipse((sx0 + sw / 2, top + 74), sw, 12, facecolor=PAPER, edgecolor=INK, lw=0.9, zorder=1.9))
    cv.ax.add_patch(Ellipse((sx0 + sw / 2, top), sw, 12, facecolor=PAPER, edgecolor=INK, lw=0.9, zorder=3))
    cv.text(sx0 + sw / 2, 16, "CONFIRMED · authoritative", size=SMALL, ha="center", weight="bold")
    skx = lambda k: sx0 + 15 + KEYS.index(k) * 22
    for k in KEYS:
        cv.text(skx(k), ymid - 12, f"${k}$", size=TINY, ha="center", color=MID)
        if k in st.S:
            kv_cell(cv, skx(k), ymid, 18, 13, st.S[k], lw=0.7)
    cv.text(sx0 + sw / 2, ymid + 17, "$S$ (confirmed_state)", size=TINY, ha="center", color=DARK)
    cv.save(os.path.join(out, f"{PFX}-e-pipeline"))


# ---------------------------------------------------------------------------
# F. Transparent sheets: pending deltas lie on top of the confirmed base
# ---------------------------------------------------------------------------
def alt_sheets(out):
    pw, gap = 118, 11.3
    cv = Canvas(FULL, 158)
    kx, sk, depth = 15.5, 0.55, 12      # key pitch, skew, sheet depth
    for i, s in enumerate(STEPS):
        px = i * (pw + gap)
        step_badge(cv, px, 6, s.num)
        cv.text(px + 11, 6, s.title, size=BODY, weight="bold")
        cv.text(px, 16, s.effect, size=TINY, color=DARK)
        base_y = 128
        ox = px + 8

        def sheet(y, fill, edge, lw, ls, alpha=1.0, z=1):
            w = 5 * kx + 4
            pts = [(ox, y), (ox + w, y), (ox + w + depth * sk * 2, y - depth), (ox + depth * sk * 2, y - depth)]
            cv.ax.add_patch(Polygon(pts, closed=True, facecolor=fill, edgecolor=edge, lw=lw,
                                    ls=ls, alpha=alpha, zorder=z))

        def cell_at(y, k):
            return ox + 2 + KEYS.index(k) * kx + kx / 2 + depth * sk, y - depth / 2

        # base sheet S
        sheet(base_y, PAPER, INK, 1.0, "-", z=1)
        chg = s.S_changed()
        for k in KEYS:
            if k in s.st.S:
                x, y = cell_at(base_y, k)
                cv.text(x, y + 0.3, v(s.st.S[k]), size=SMALL, ha="center",
                        weight="bold" if k in chg else "normal", z=2)
            x, y = cell_at(base_y, k)
            cv.text(x - depth * sk, base_y + 6, f"${k}$", size=TINY - 0.6, ha="center", color=MID)
        cv.text(ox + 5 * kx + 14, base_y - 6, "$S$", size=SMALL)
        cv.line([(px, base_y - depth - 5), (px + pw - 10, base_y - depth - 5)], lw=0.9)

        # pending sheets, oldest lowest
        entries = [(q, d, False) for q, d in s.st.log] + [(q, d, True) for q, d in s.gone.items()]
        entries.sort()
        y = base_y - depth - 14
        for q, d, gone in entries:
            if gone and s.kind == "commit":
                continue
            sheet(y, PAPER if gone else LIGHT, MID, 0.5 if gone else 0.6,
                  (0, (1.5, 1.2)) if gone else "-", alpha=0.85, z=1 + (140 - y) / 100)
            for k, val in d.items():
                x, yy = cell_at(y, k)
                cv.text(x, yy + 0.3, v(val), size=SMALL, ha="center", color=MID if gone else INK,
                        weight="bold" if q in s.new else "normal", z=3 + (140 - y) / 100)
            lab = rf"$\Delta_{{{q}}}$" + ("  new" if q in s.new else "")
            cv.text(ox + 5 * kx + 14, y - 6, lab, size=TINY, color=MID if gone else INK)
            y -= 13
        # the view from above
        cy = 34
        cv.text(px, cy, "$C$", size=SMALL)
        for k in KEYS:
            x = ox + 2 + KEYS.index(k) * kx + kx / 2 + depth * sk
            if k in s.st.C:
                kv_cell(cv, x, cy, 12.5, 10, s.st.C[k], fill=DARK, edge=INK if k in s.C_changed() else DARK,
                        lw=1.2 if k in s.C_changed() else 0.3, tc=PAPER, size=TINY)
    cv.text(0, 149, "Thick rule: commit frontier. Solid sheet: confirmed $S$. Gray sheets: pending "
            r"$\Delta$s, oldest lowest; dashed: dropped." + "\n"
            "Looking down a column gives what a read returns; $C$ is that view over the pending sheets alone.",
            linespacing=1.3, size=TINY, color=MID)
    cv.save(os.path.join(out, f"{PFX}-f-sheets"))


ALTS = [alt_swimlanes, alt_containers, alt_table, alt_chains, alt_pipeline, alt_sheets]

if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "."
    os.makedirs(out, exist_ok=True)
    assert S1.C == {"a": "6", "b": TOMB, "c": "5", "d": "9", "e": "8"}
    assert S2.S == {"a": "5", "b": "2", "c": "3", "d": "9", "e": "7"} and S2.C == {"a": "6", "b": TOMB, "c": "5", "e": "8"}
    assert S3.C == {"b": TOMB} and S3.np == 9 and S3.staged == ()
    for f in ALTS:
        f(out)
        print(f.__name__)
