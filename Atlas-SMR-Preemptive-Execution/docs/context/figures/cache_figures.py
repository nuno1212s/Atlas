"""CRUD cache figures (matplotlib, print size, grayscale).

    python3 cache_figures.py <output-dir>          # writes <name>.pdf/.svg/.png per figure

Running example, identical in every figure (see crud-cache-visuals.md §2):
  S  (confirmed through seq 6) = {a:1, b:2, c:3, e:7}
  Ω7 = {a:5, d:9}   Ω8 = {b:⊥}   Ω9 = {a:6, c:4}   (Ω: speculated decision, Δ: once confirmed)
  C  = Ω7 ⊕ Ω8 ⊕ Ω9 = {a:6, b:⊥, c:4, d:9}
  next_confirmed = 7, next_preemptive = 10, staged = {12}
  batch 10 = c ← 5, e ← 8, read e, read b  →  Ω10 = {c:5, e:8} (executor_model.py)
  the read-path figure alone uses its own mid-batch example: read a, read b, read e, update(c, 5)
"""
import os
import sys

from paperfig import (BODY, COLUMN, DARK, FULL, INK, LIGHT, MID, PALE, PAPER, SMALL, TINY,
                      Canvas)

KEYS = "abcde"
PFX = "preemptive-context-cache"

S0 = {"a": "1", "b": "2", "c": "3", "e": "7"}
D7 = {"a": "5", "d": "9"}
D8 = {"b": "⊥"}
D9 = {"a": "6", "c": "4"}
C0 = {"a": "6", "b": "⊥", "c": "4", "d": "9"}

# Layer styles: fill, edge, edge width, line style, text colour.
LAYER = {
    "S": (PAPER, INK, 0.7, "-", INK),
    "D": (LIGHT, MID, 0.4, "-", INK),
    "C": (DARK, DARK, 0.4, "-", PAPER),
    "d": (PAPER, INK, 0.7, (0, (2.2, 1.2)), INK),
}


def val(s):
    return r"$\bot$" if s == "⊥" else s


def cell(cv, cx, cy, w, h, layer, v, mark=None, size=SMALL):
    """mark: None | 'hit' (bold edge) | 'shadow' (hidden by a newer layer) | 'gone' (removed)."""
    fill, edge, lw, ls, tc = LAYER[layer]
    if mark == "hit":
        edge, lw = INK, 1.3
    elif mark == "shadow":
        fill = PALE if layer == "D" else PAPER
        edge, tc = MID if layer != "D" else PALE, MID
    elif mark == "gone":
        fill, edge, lw, ls, tc = PAPER, MID, 0.5, (0, (1.6, 1.2)), MID
    cv.box(cx - w / 2, cy - h / 2, w, h, fill=fill, edge=edge, lw=lw, ls=ls)
    cv.text(cx, cy + 0.3, val(v), size=size, ha="center", color=tc)


def grid_row(cv, x0, pitch, cy, w, h, layer, cells, marks=None, size=SMALL):
    marks = marks or {}
    for i, k in enumerate(KEYS):
        if k in cells:
            cell(cv, x0 + i * pitch + pitch / 2, cy, w, h, layer, cells[k], marks.get(k), size)


def guides(cv, x0, pitch, y0, y1):
    for i in range(len(KEYS)):
        x = x0 + i * pitch + pitch / 2
        cv.line([(x, y0), (x, y1)], lw=0.35, color=PALE, z=1)


def keys(cv, x0, pitch, y, size=SMALL):
    for i, k in enumerate(KEYS):
        cv.text(x0 + i * pitch + pitch / 2, y, f"${k}$", size=size, ha="center", color=MID)


def row_label(cv, x, cy, sym, code=None, size=9, beside=False):
    """Symbol, optionally with the code name beside it or stacked underneath."""
    if code and not beside:
        cv.text(x, cy - 3.2, sym, size=size)
        cv.text(x, cy + 6.2, code, size=TINY - 0.6, color=MID, mono=True)
        return
    cv.text(x, cy + 0.3, sym, size=size)
    if code:
        cv.text(x + 17, cy + 0.3, code, size=TINY, color=MID, mono=True)


# ---------------------------------------------------------------------------
# Structure: the cache is the top view of the log
# ---------------------------------------------------------------------------
def fig_structure(out):
    cv = Canvas(COLUMN, 118)
    x0, p, w, h = 66, 21, 17.5, 11.5
    col = lambda k: x0 + KEYS.index(k) * p + p / 2
    yk, yc, y9, y8, y7, yf, ys = 7, 21, 53, 67, 81, 94, 106
    right = x0 + 5 * p + 6

    guides(cv, x0, p, 13, ys + 7)
    keys(cv, x0, p, yk)

    # log group
    cv.box(x0 - 3, y9 - 10, 5 * p + 6, (y7 - y9) + 20, fill="#f6f6f6", edge="none", lw=0, z=0.5)
    grid_row(cv, x0, p, y9, w, h, "D", D9)
    grid_row(cv, x0, p, y8, w, h, "D", D8)
    grid_row(cv, x0, p, y7, w, h, "D", D7, {"a": "shadow"})
    grid_row(cv, x0, p, yc, w, h, "C", C0)
    grid_row(cv, x0, p, ys, w, h, "S", S0, {"a": "shadow", "b": "shadow", "c": "shadow"})

    for k, src in (("a", y9), ("b", y8), ("c", y9), ("d", y7)):
        cv.arrow((col(k), src - h / 2 - 0.6), (col(k), yc + h / 2 + 0.8), lw=0.55)

    cv.line([(x0 - 3, yf), (x0 + 5 * p + 3, yf)], lw=1.0)

    row_label(cv, 0, yc, "$C$", "accumulated_cache")
    row_label(cv, 0, y9, r"$\Omega_9$", "tail", beside=True)
    row_label(cv, 0, y8, r"$\Omega_8$")
    row_label(cv, 0, y7, r"$\Omega_7$", "head", beside=True)
    row_label(cv, 0, ys, "$S$", "confirmed_state")

    cv.text(right, yc, "newest entry\nper key in the log", size=TINY, color=DARK, linespacing=1.15)
    cv.bracket(right - 1, y9 - 7, y7 + 7)
    cv.text(right + 5, (y9 + y7) / 2, "pending log;\nentry per batch:\nbatch, replies, Ω",
            size=TINY, color=DARK, linespacing=1.15)
    cv.text(right, yf, "commit frontier", size=TINY, color=DARK)
    cv.text(right, ys, "gray: shadowed,\nnever read", size=TINY, color=DARK, linespacing=1.15)
    cv.save(os.path.join(out, f"{PFX}-structure"))


# ---------------------------------------------------------------------------
# Read path: δ over C over S
# ---------------------------------------------------------------------------
def fig_read_path(out):
    cv = Canvas(COLUMN, 118)
    x0, p, w, h = 40, 28, 22, 12
    col = lambda k: x0 + KEYS.index(k) * p + p / 2
    yd, yc, ys = 44, 69, 94
    right = x0 + 5 * p + 6

    guides(cv, x0, p, 30, ys + 7)
    keys(cv, x0, p, ys + 15)

    grid_row(cv, x0, p, yd, w, h, "d", {"c": "5"}, {"c": "hit"})
    grid_row(cv, x0, p, yc, w, h, "C", C0, {"a": "hit", "b": "hit"})
    grid_row(cv, x0, p, ys, w, h, "S", S0, {"a": "shadow", "b": "shadow", "c": "shadow", "e": "hit"})

    ops = (("a", "read a", "→ 6", yc, 0.6), ("b", "read b", "→ none", yc, 0.6),
           ("c", "update c", "← 5", yd, 1.2), ("e", "read e", "→ 7", ys, 0.6))
    for k, op, res, stop, lw in ops:
        cv.text(col(k), 6, op, size=TINY, ha="center", mono=True)
        cv.text(col(k), 14, res, size=TINY, ha="center", mono=True, color=DARK)
        cv.arrow((col(k), 19), (col(k), stop - h / 2 - 0.8), lw=lw,
                 head=(3.6, 1.8) if lw > 1 else (3.2, 1.5))

    row_label(cv, 0, yd, r"$\omega_{10}$", "batch")
    row_label(cv, 0, yc, "$C$", "pending")
    row_label(cv, 0, ys, "$S$", "confirmed")

    cv.text(right, yd, "writes stop here", size=TINY, color=DARK)
    cv.text(right, yc, r"$\bot$ hides b = 2" + "\nheld by $S$", size=TINY, color=DARK, linespacing=1.15)
    cv.text(right, ys, "reached only\nwhen both miss", size=TINY, color=DARK, linespacing=1.15)
    cv.save(os.path.join(out, f"{PFX}-read-path"))


# ---------------------------------------------------------------------------
# Storyboard: speculate, commit, backtrack
# ---------------------------------------------------------------------------
def panel(cv, px, py, num, title, effect, c, log, s, ptr, geo):
    lw_, p, w, h, notew = geo
    x0 = px + lw_
    col_end = x0 + 5 * p
    from matplotlib.patches import Circle
    cv.ax.add_patch(Circle((px + 4, py + 5), 4, facecolor=INK, edgecolor=INK, lw=0, zorder=4))
    cv.text(px + 4, py + 5.3, num, size=TINY, ha="center", color=PAPER, weight="bold")
    cv.text(px + 11, py + 5, title, size=BODY, weight="bold")
    cv.text(px, py + 15, effect, size=SMALL, color=DARK)
    yc = py + 31
    slots = [py + 48, py + 61, py + 74, py + 87]
    yf, ys = py + 96.5, py + 106
    guides(cv, x0, p, yc - 7, ys + 6)

    c_cells, c_marks = c
    grid_row(cv, x0, p, yc, w, h, "C", c_cells, c_marks, size=TINY)
    row_label(cv, px, yc, "$C$", size=8)

    for (sym, cells, marks, state), y in zip(log, slots[4 - len(log):]):
        if state in ("dropped", "applied"):
            grid_row(cv, x0, p, y, w, h, "D", cells, {k: "gone" for k in cells}, size=TINY)
        if state == "dropped":
            cv.line([(x0 + 1, y), (col_end - 1, y)], lw=0.6, color=MID, z=6)
        if state not in ("dropped", "applied"):
            grid_row(cv, x0, p, y, w, h, "D", cells, marks, size=TINY)
        row_label(cv, px, y, sym, size=8)
        if state:
            cv.text(col_end + 2, y, state, size=TINY, color=MID)

    cv.line([(x0 - 2, yf), (col_end + 2, yf)], lw=0.9)
    s_cells, s_marks = s
    grid_row(cv, x0, p, ys, w, h, "S", s_cells, s_marks, size=TINY)
    row_label(cv, px, ys, "$S$", size=8)
    keys(cv, x0, p, ys + 11, size=TINY)
    cv.text(px, ys + 17, ptr, size=TINY, color=DARK, linespacing=1.2, va="top")


STEPS = [
    ("0", "start", r"$\Omega_7, \Omega_8, \Omega_9$ speculated",
     (C0, {}),
     [(r"$\Omega_9$", D9, {}, None), (r"$\Omega_8$", D8, {}, None), (r"$\Omega_7$", D7, {}, None)],
     (S0, {}),
     "$n_c$ = 7,  $n_p$ = 10\nstaged: batch 12"),
    ("1", r"speculate $\Omega_{10}$", r"$C \leftarrow C \oplus \Omega_{10}$, append",
     ({**C0, "c": "5", "e": "8"}, {"c": "hit", "e": "hit"}),
     [(r"$\Omega_{10}$", {"c": "5", "e": "8"}, {"c": "hit", "e": "hit"}, "new"), (r"$\Omega_9$", D9, {}, None),
      (r"$\Omega_8$", D8, {}, None), (r"$\Omega_7$", D7, {}, None)],
     (S0, {}),
     r"$n_c$ = 7,  $n_p$ = 11" + "\n" + r"replies of $\Omega_{10}$ held"),
    ("2", r"commit $\Omega_7$", r"$S \leftarrow S \oplus \Delta_7$, refold $C$",
     ({"a": "6", "b": "⊥", "c": "5", "d": "9", "e": "8"}, {"d": "gone"}),
     [(r"$\Omega_{10}$", {"c": "5", "e": "8"}, {}, None), (r"$\Omega_9$", D9, {}, None),
      (r"$\Omega_8$", D8, {}, None), (r"$\Delta_7$", D7, {}, "applied")],
     ({**S0, "a": "5", "d": "9"}, {"a": "hit", "d": "hit"}),
     r"$n_c$ = 8,  $n_p$ = 11" + "\n" + r"replies of $\Delta_7$ sent"),
    ("3", r"backtrack to $\Omega_9$", r"drop $\Omega_9, \Omega_{10}$, refold $C$",
     ({"a": "6", "b": "⊥", "c": "5", "e": "8"}, {"a": "gone", "c": "gone", "e": "gone"}),
     [(r"$\Omega_{10}$", {"c": "5", "e": "8"}, {}, "dropped"), (r"$\Omega_9$", D9, {}, "dropped"),
      (r"$\Omega_8$", D8, {}, "kept")],
     ({**S0, "a": "5", "d": "9"}, {}),
     "$n_c$ = 8,  $n_p$ = 9\nstaged batch 12 evicted"),
]


def fig_storyboard(out):
    # 1 × 4, full text width
    geo = (15, 16.5, 14, 10.5, 24)
    pw = geo[0] + 5 * geo[1] + geo[4]
    gap = (FULL - 4 * pw) / 3
    cv = Canvas(FULL, 142)
    for i, st in enumerate(STEPS):
        panel(cv, i * (pw + gap), 2, *st, geo)
        if i:
            x = i * (pw + gap) - gap / 2
            cv.line([(x, 4), (x, 138)], lw=0.35, color=LIGHT)
    cv.save(os.path.join(out, f"{PFX}-storyboard"))

    # 2 × 2, one column
    geo = (15, 14.4, 12.4, 10, 26)
    pw = geo[0] + 5 * geo[1] + geo[4]
    gap = COLUMN - 2 * pw
    cv = Canvas(COLUMN, 284)
    for i, st in enumerate(STEPS):
        panel(cv, (i % 2) * (pw + gap), 2 + (i // 2) * 143, *st, geo)
    cv.line([(0, 140), (COLUMN, 140)], lw=0.35, color=LIGHT)
    cv.save(os.path.join(out, f"{PFX}-storyboard-2x2"))


# ---------------------------------------------------------------------------
# Version-control analogy
# ---------------------------------------------------------------------------
def fig_vcs(out):
    cv = Canvas(FULL, 100)
    r, gap, x0 = 7.4, 21, 70
    notes_x = x0 + 5 * gap + r + 12

    def chain(y, spec):
        prev = None
        for i, (lab, st) in enumerate(spec):
            cx = x0 + i * gap
            if prev is not None:
                ls = (0, (1.6, 1.4)) if st in ("work", "gone") else "-"
                cv.line([(prev + r, y), (cx - r, y)], lw=0.8, color=MID if st == "gone" else INK, ls=ls)
            fill = {"main": INK, "pend": PAPER, "work": PAPER, "gone": PAPER}[st]
            edge = MID if st == "gone" else INK
            ls = {"main": "-", "pend": "-", "work": (0, (1.8, 1.1)), "gone": (0, (0.8, 1.0))}[st]
            circ = __import__("matplotlib.patches", fromlist=["Circle"]).Circle(
                (cx, y), r, facecolor=fill, edgecolor=edge, lw=0.8, ls=ls, zorder=3)
            cv.ax.add_patch(circ)
            letter = r"\Delta" if st == "main" else r"\Omega"
            label = lab if lab == "…" else "$" + letter + "_{" + lab + "}$"
            cv.text(cx, y + 0.2, label, size=TINY - 0.4, ha="center",
                    color=PAPER if st == "main" else (MID if st == "gone" else INK))
            prev = cx

    rows = [
        ("state", [("5", "main"), ("6", "main"), ("7", "pend"), ("8", "pend"), ("9", "pend"), ("10", "work")],
         r"main = $S$ ($\Delta$s);  unmerged commits = $\Omega_7, \Omega_8, \Omega_9$;  HEAD view = $C$ over $S$;  "
         r"working tree = $\omega_{10}$"),
        (r"commit $\Omega_7$", [("5", "main"), ("6", "main"), ("7", "main"), ("8", "pend"), ("9", "pend")],
         r"$\Omega_7$ becomes $\Delta_7$: fast-forward main by one ($S \leftarrow S \oplus \Delta_7$), then refold $C$"),
        (r"backtrack to $\Omega_9$", [("5", "main"), ("6", "main"), ("7", "pend"), ("8", "pend"), ("9", "gone"), ("9", "work")],
         r"reset --hard to $\Omega_8$, then commit the re-proposed $\Omega_9$;  $\Omega_7, \Omega_8$ kept, never re-executed"),
        ("state transfer", [("5", "main"), ("6", "main"), ("…", "main"), ("40", "main"), ("8", "gone"), ("9", "gone")],
         r"replace main with a snapshot through $\Delta_{40}$;  the branch and the reorder buffer are discarded"),
    ]
    y = 12
    for name, spec, note in rows:
        cv.text(0, y, name, size=BODY, weight="bold")
        chain(y, spec)
        cv.text(notes_x, y, note, size=SMALL, color=DARK)
        y += 20
    cv.text(0, y - 2, "filled: confirmed (Δ)     outlined: speculated (Ω: write set, replies, batch)     "
            "dashed: executing or re-proposed     dotted: discarded", size=TINY, color=MID)
    cv.save(os.path.join(out, f"{PFX}-vcs-analogy"))


# ---------------------------------------------------------------------------
# Footprint: what each design keeps (schematic, not to scale)
# ---------------------------------------------------------------------------
def fig_footprint(out):
    cv = Canvas(COLUMN, 98)
    x0, bh, sw, common = 50, 15, 64, 26

    def seg(x, y, w, label, fill, hatch=None, tc=INK, edge=INK, ls="-"):
        cv.box(x, y - bh / 2, w, bh, fill=fill, edge=edge, lw=0.6, r=0.8, hatch=hatch, ls=ls)
        if label:
            cv.text(x + w / 2, y + 0.3, label, size=TINY, ha="center", color=tc, linespacing=1.0)
        return x + w + 1.5

    y1, y2 = 12, 52
    cv.text(0, y1, "dual-state", size=BODY, weight="bold")
    x = seg(x0, y1, sw, "$S$ confirmed", PAPER)
    x = seg(x, y1, sw, "", PALE, hatch="//////", edge=MID)
    cv.text(x - 1.5 - sw / 2, y1 + 0.3, r"full copy of $S$", size=TINY, ha="center",
            bbox=dict(boxstyle="round,pad=0.25,rounding_size=0.8", fc=PAPER, ec="none"))
    seg(x, y1, common, "batches\n+ replies", PALE, edge=MID, ls=(0, (1.6, 1.2)), tc=DARK)
    cv.text(0, y1 + 14, "commit: re-execute the batch on $S$\nbacktrack: clone $S$, replay survivors",
            size=TINY, color=DARK, va="top", linespacing=1.2)

    cv.text(0, y2, "CRUD cache", size=BODY, weight="bold")
    x = seg(x0, y2, sw, "$S$ confirmed", PAPER)
    x = seg(x, y2, 13, "$C$", DARK, tc=PAPER, edge=DARK)
    for i in range(3):
        x = seg(x, y2, 8, "", LIGHT, edge=MID) - 0.5
    seg(x + 1, y2, common, "batches\n+ replies", PALE, edge=MID, ls=(0, (1.6, 1.2)), tc=DARK)
    cv.text(x0 + sw + 1.5 + 13 + 12, y2 - 12, r"$\Omega$ log", size=TINY, color=DARK)
    cv.text(0, y2 + 14, r"commit: $S \leftarrow S \oplus \Delta_{\mathrm{head}}$, refold $C$" + "\nbacktrack: truncate the log, refold $C$",
            size=TINY, color=DARK, va="top", linespacing=1.2)

    cv.text(0, 91, "Schematic, not to scale. Pending batches and held replies are kept by both.",
            size=TINY, color=MID)
    cv.save(os.path.join(out, f"{PFX}-footprint"))


FIGURES = [fig_structure, fig_read_path, fig_storyboard, fig_vcs, fig_footprint]

if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "."
    os.makedirs(out, exist_ok=True)
    for f in FIGURES:
        f(out)
        print(f.__name__)
