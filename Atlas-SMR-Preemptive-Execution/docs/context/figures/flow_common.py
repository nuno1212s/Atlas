"""Drawing helpers shared by the flow figures (containers, cells, arrows, change tags)."""
from matplotlib.patches import Circle, FancyArrowPatch

from executor_model import KEYS, TOMB
from paperfig import DARK, INK, LIGHT, MID, PALE, PAPER, SMALL, TINY

PEND_FILL = "#eeeeee"
WORK_FILL = "#f8f8f8"


def tx(s):
    """Route symbols missing from the text font through mathtext when a string needs math."""
    if "⊥" in s or "$" in s:
        s = s.replace("⊥", r"$\bot$").replace("→", r"$\rightarrow$").replace("←", r"$\leftarrow$")
        s = s.replace("⊕", r"$\oplus$")
    return s


def val(x):
    return r"$\bot$" if x == TOMB else x


def D(q):
    return rf"$\Delta_{{{q}}}$"


def B(q):
    return rf"$B_{{{q}}}$"


def badge(cv, x, y, num, r=4.2):
    cv.ax.add_patch(Circle((x + r, y), r, facecolor=INK, edgecolor=INK, lw=0, zorder=6))
    cv.text(x + r, y + 0.3, num, size=TINY, ha="center", color=PAPER, weight="bold", z=7)


def header(cv, x, y, num, title, effect=None):
    badge(cv, x, y, num)
    cv.text(x + 11, y, title, size=8, weight="bold")
    if effect:
        cv.text(x, y + 10, effect, size=TINY, color=DARK)


def region(cv, x, y, w, h, kind, title=None, title_size=TINY):
    fill, edge, lw, ls = {
        "pend": (PEND_FILL, MID, 0.7, (0, (3, 2))),
        "conf": (PAPER, INK, 0.9, "-"),
        "work": (WORK_FILL, DARK, 0.6, (0, (1.2, 1.2))),
    }[kind]
    cv.box(x, y, w, h, fill=fill, edge=edge, lw=lw, ls=ls, r=2, z=0.3)
    if title:
        cv.text(x + 3, y + 5.5, title, size=title_size, weight="bold", color=DARK if kind != "conf" else INK,
                bbox=dict(boxstyle="square,pad=0.1", fc=fill, ec="none"))


# cell kinds: S (confirmed), D (log entry), C (cache), P (dual-state preemptive copy),
# d (in-flight delta), L (per-operation local cache)
CELL = {
    "S": (PAPER, INK, 0.6, "-", INK),
    "D": (LIGHT, MID, 0.4, "-", INK),
    "C": (DARK, DARK, 0.4, "-", PAPER),
    "P": ("#d9d9d9", DARK, 0.5, "-", INK),
    "d": (PAPER, INK, 0.6, (0, (2, 1.1)), INK),
    "L": (PAPER, MID, 0.5, (0, (1.2, 1)), INK),
}


def cell(cv, cx, cy, w, h, kind, v, mark=None, size=TINY):
    """mark: None | 'chg' (changed in this step) | 'gone' (removed) | 'dim' (shadowed / stale)."""
    fill, edge, lw, ls, tc = CELL[kind]
    if mark == "chg":
        edge, lw = INK, 1.25
    elif mark == "gone":
        fill, edge, lw, ls, tc = PAPER, MID, 0.45, (0, (1.4, 1.1)), MID
    elif mark == "dim":
        fill, edge, tc = (PALE if kind == "D" else PAPER), (PALE if kind == "D" else MID), MID
    cv.box(cx - w / 2, cy - h / 2, w, h, fill=fill, edge=edge, lw=lw, ls=ls, r=1.1)
    cv.text(cx, cy + 0.3, val(v), size=size, ha="center", color=tc)


def key_row(cv, xs, y, size=TINY):
    for k, x in zip(KEYS, xs):
        cv.text(x, y, f"${k}$", size=size, ha="center", color=MID)


def parrow(cv, pts, lw=0.8, color=INK, head=(3.2, 1.5), ls="-", z=4):
    """Arrow along a polyline; the head sits on the last segment."""
    if len(pts) > 2:
        cv.line(pts[:-1], lw=lw, color=color, ls=ls, z=z)
    cv.arrow(pts[-2], pts[-1], lw=lw, color=color, head=head, ls=ls, z=z)


def curved(cv, a, b, rad=0.35, lw=0.7, color=INK, head=(3.0, 1.4), z=6):
    p = FancyArrowPatch(a, b, arrowstyle=f"-|>,head_length={head[0]},head_width={head[1]}",
                        mutation_scale=1, lw=lw, color=color, shrinkA=0, shrinkB=0, zorder=z,
                        connectionstyle=f"arc3,rad={rad}")
    cv.ax.add_patch(p)


def strike(cv, x0, y0, x1, y1, lw=0.6, color=MID):
    cv.line([(x0, y0), (x1, y1)], lw=lw, color=color, z=8)


def changes(before, after, label):
    """'S: a 1→5, d +9' — the per-key difference between two maps, in key order."""
    parts = []
    for k in KEYS:
        b, a = before.get(k), after.get(k)
        if b == a:
            continue
        if b is None:
            parts.append(f"{k} +{a}")
        elif a is None:
            parts.append(f"{k} −")
        else:
            parts.append(f"{k} {b}→{a}")
    return f"{label}: " + (", ".join(parts) if parts else "unchanged")


def marks_for(before, after):
    """Cell marks for `after` (changed) and the removed keys to draw as ghosts."""
    chg = {k: "chg" for k in after if before is not None and before.get(k) != after[k]}
    gone = {k: before[k] for k in (before or {}) if k not in after}
    return chg, gone
