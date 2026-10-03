"""Type 2 — caching structure: the confirmed state, the small pending cache and the write log.

    python3 caching.py <output-dir>

Values are the running example (executor_model.py): S = {a:1, b:2, c:3, e:7}, log Ω7 Ω8 Ω9,
C = Ω7 ⊕ Ω8 ⊕ Ω9 = {a:6, b:⊥, c:4, d:9}, and batch 10 has written c ← 5 so far (ω).
"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "figures"))
from executor_model import CRUD, KEYS, TOMB  # noqa: E402

from svgkit import (CONF, CONF_T, DROP, INK, LINE, LIVE, LIVE_T, MUTED, PANEL, POOL, SPEC, SPEC_T, WHITE,  # noqa: E402
                    Svg, block, card, legend, text_width)

PFX = "caching"
S0 = CRUD[0].S
C0 = CRUD[0].C
LOG0 = CRUD[0].log
OMEGA = {"c": "5"}                       # batch 10, after its first operation


def shown(v):
    return "⊥" if v == TOMB else v


# ---------------------------------------------------------------------------
# B1 — see-through layers
# ---------------------------------------------------------------------------
class Sheet:
    """An oblique sheet with one slot per key; slot i's centre is returned by centre(i)."""

    def __init__(self, x, y, slot=54, dx=46, dy=-30, pad=12):
        self.x, self.y, self.slot, self.dx, self.dy, self.pad = x, y, slot, dx, dy, pad
        self.w = 5 * slot + 2 * pad

    def corners(self):
        x, y, w, dx, dy = self.x, self.y, self.w, self.dx, self.dy
        return [(x, y), (x + w, y), (x + w + dx, y + dy), (x + dx, y + dy)]

    def tile(self, i, inset=0.18):
        x0 = self.x + self.pad + i * self.slot
        fx, fy = 1 - inset, inset
        a = (x0 + 4 + self.dx * fy, self.y + self.dy * fy)
        b = (x0 + self.slot - 4 + self.dx * fy, self.y + self.dy * fy)
        c = (x0 + self.slot - 4 + self.dx * fx, self.y + self.dy * fx)
        d = (x0 + 4 + self.dx * fx, self.y + self.dy * fx)
        return [a, b, c, d]

    def centre(self, i):
        pts = self.tile(i)
        return (sum(p[0] for p in pts) / 4, sum(p[1] for p in pts) / 4)

    def draw(self, s, colour, tint, values, opacity=0.55):
        s.poly(self.corners(), fill=tint, stroke=colour, sw=1.4, opacity=opacity)
        s.poly(self.corners(), fill="none", stroke=colour, sw=1.4)
        for i, k in enumerate(KEYS):
            pts = self.tile(i)
            if k in values:
                s.poly(pts, fill=colour, stroke=colour, sw=1)
                cx, cy = self.centre(i)
                s.text(cx, cy + 5, shown(values[k]), size=14, weight=600, fill=WHITE, anchor="middle")
            else:
                s.poly(pts, fill="none", stroke=colour, sw=0.8, dash="2 3", opacity=0.6)


def b1_layers(out):
    s = Svg(1000, 560)
    s.title("Caching structure", "B1 · see-through layers: a read drops until a layer holds the key")
    top, gap = 210, 112
    sheets = [("ω", "the batch being run", LIVE, LIVE_T, OMEGA),
              ("C", "pending cache", SPEC, SPEC_T, C0),
              ("S", "confirmed state", CONF, CONF_T, S0)]
    geo = []
    for j, (name, sub, col, tint, vals) in enumerate(sheets):
        sh = Sheet(150, top + j * gap)
        geo.append(sh)
    # rays first stop on the first layer that holds the key
    reads = ["a", "b", "c", "e"]
    for j in reversed(range(3)):
        name, sub, col, tint, vals = sheets[j]
        geo[j].draw(s, col, tint, vals)
        s.text(130, geo[j].y - 6, name, size=22, weight=600, fill=col, anchor="end")
        s.text(130, geo[j].y + 12, sub, size=11.5, fill=MUTED, anchor="end")
    for k in reads:
        i = KEYS.index(k)
        hit = next(j for j, (_, _, _, _, v) in enumerate(sheets) if k in v)
        x, y_hit = geo[hit].centre(i)
        x0, y0 = geo[0].centre(i)
        s.text(x0, top - 76, f"read {k}", size=12, weight=600, fill=INK, anchor="middle")
        s.line(x0, top - 68, x, y_hit - 12, stroke=INK, sw=1.6, arrow=True)
        for j in range(hit):
            cx, cy = geo[j].centre(i)
            s.circle(cx, cy, 3, fill=WHITE, stroke=INK, sw=1.2)
    # the log, folded into C; commit drains its left end into S; backtrack cuts its right end
    lx, ly = 560, 330
    s.text(lx, ly - 30, "write log", size=14, weight=600)
    s.text(lx, ly - 12, "C = Ω_{7} ⊕ Ω_{8} ⊕ Ω_{9}, the newest write per key", size=11.5, fill=SPEC)
    for n, (q, d, _) in enumerate(LOG0):
        x = lx + n * 104
        s.rect(x, ly, 92, 58, r=8, fill=SPEC_T, stroke=SPEC, sw=1.4, shadow=True)
        s.text(x + 10, ly + 20, f"Ω_{{{q}}}", size=14, weight=600, fill=SPEC)
        for r, (k, v) in enumerate(sorted(d.items())):
            s.text(x + 10, ly + 36 + r * 14, f"{k} ← {shown(v)}", size=11.5, fill=INK)
    s.arrow([(lx - 4, ly + 22), (geo[1].x + geo[1].w + 30, geo[1].y - 16)], stroke=SPEC, sw=1.8)
    s.label(lx - 30, ly + 4, "fold", size=11.5, fill=SPEC, bg=WHITE)
    s.arrow([(lx + 46, ly + 60), (lx + 46, geo[2].y - 14), (geo[2].x + geo[2].w + 28, geo[2].y - 14)],
            stroke=CONF, sw=2)
    s.text(lx + 56, ly + 98, "commit: the oldest, Ω_{7}, becomes Δ_{7}", size=12, fill=CONF)
    s.text(lx + 56, ly + 114, "and is written into S", size=12, fill=CONF)
    s.line(lx + 202, ly - 4, lx + 202, ly + 70, stroke=DROP, sw=2, dash="5 4")
    s.text(lx + 210, ly + 82, "backtrack: drop from here, refold C", size=12, fill=DROP)
    s.text(150, 530, "○  the read passed an empty slot", size=11.5, fill=MUTED)
    s.text(380, 530, "⊥  deleted: the read stops and returns none", size=11.5, fill=MUTED)
    s.save(os.path.join(out, f"{PFX}-b1-layers"))


# ---------------------------------------------------------------------------
# B2 — a ledger tape
# ---------------------------------------------------------------------------
def b2_ledger(out):
    s = Svg(1000, 600)
    s.title("Caching structure", "B2 · a ledger: confirmed on the left, pending decisions on a tape")
    # main CRUD row
    y = 150
    block(s, 40, y, 150, 150, CONF, CONF_T, "S", size=24, sub="confirmed")
    tape_x = 222
    s.rect(tape_x - 8, y + 34, 470, 84, r=12, fill=PANEL)
    xs = []
    for n, (q, d, _) in enumerate(LOG0):
        x = tape_x + n * 116
        xs.append(x)
        s.rect(x, y + 42, 104, 68, r=9, fill=SPEC_T, stroke=SPEC, sw=1.5, shadow=True)
        s.text(x + 12, y + 64, f"Ω_{{{q}}}", size=15, weight=600, fill=SPEC)
        for r, (k, v) in enumerate(sorted(d.items())):
            s.text(x + 12, y + 82 + r * 15, f"{k} ← {shown(v)}", size=12, fill=INK)
    # newest joins on the right
    xn = tape_x + 3 * 116
    s.rect(xn, y + 42, 104, 68, r=9, fill=WHITE, stroke=SPEC, sw=1.4, dash="5 4")
    s.text(xn + 52, y + 81, "next Ω", size=13, fill=SPEC, anchor="middle")
    s.text(tape_x + 4, y + 22, "oldest", size=11, fill=MUTED)
    s.text(xn + 104, y + 22, "newest", size=11, fill=MUTED, anchor="end")
    s.line(tape_x + 50, y + 18, xn + 50, y + 18, stroke=FAINT_LINE, sw=1, arrow=FAINT_LINE)
    # commit off the left end, cut on the right
    s.arrow([(xs[0] + 20, y + 112), (xs[0] + 20, y + 132), (194, y + 132)], stroke=CONF, sw=2)
    s.text(xs[0] - 4, y + 166, "commit: Ω_{7} becomes Δ_{7}, written into S", size=12, fill=CONF)
    s.line(xs[2] - 6, y + 26, xs[2] - 6, y + 126, stroke=DROP, sw=2, dash="5 4")
    s.text(xs[2] - 2, y + 144, "backtrack: cut here", size=12, fill=DROP)
    cx0 = 760
    s.arrow([(xn + 108, y + 76), (cx0 - 6, y + 76)], stroke=SPEC, sw=1.8)
    s.label((xn + 108 + cx0) / 2, y + 66, "fold", size=11.5, fill=SPEC, bg=WHITE)
    s.rect(cx0, y + 18, 200, 116, r=12, fill=SPEC_T, stroke=SPEC, sw=1.6, shadow=True)
    s.text(cx0 + 14, y + 42, "C  pending cache", size=14, weight=600, fill=SPEC)
    for i, k in enumerate(KEYS):
        x = cx0 + 16 + i * 36
        s.text(x + 14, y + 66, k, size=11, fill=MUTED, anchor="middle")
        if k in C0:
            s.rect(x, y + 74, 28, 26, r=5, fill=SPEC, stroke=SPEC)
            s.text(x + 14, y + 92, shown(C0[k]), size=13, weight=600, fill=WHITE, anchor="middle")
        else:
            s.rect(x, y + 74, 28, 26, r=5, fill="none", stroke=SPEC, sw=0.9, dash="2 3")
    s.text(cx0 + 14, y + 122, "the newest write per key", size=11, fill=MUTED)

    # the other two executors, as variants of the same tape
    y = 400
    s.line(30, y - 22, 970, y - 22, stroke=LINE, sw=1)
    s.text(40, y + 6, "Dual-state", size=15, weight=600)
    block(s, 40, y + 22, 90, 70, CONF, CONF_T, "S", size=18)
    for n, q in enumerate((7, 8, 9)):
        card(s, 160 + n * 60, y + 40, f"Ω_{{{q}}}", SPEC, SPEC_T, w=52, h=34, size=13)
    s.arrow([(346, y + 57), (420, y + 57)], stroke=SPEC, sw=1.8)
    s.label(383, y + 47, "run", size=11.5, fill=SPEC, bg=WHITE)
    block(s, 426, y + 22, 90, 70, SPEC, SPEC_T, "S_{p}", size=18)
    s.text(160, y + 112, "the tape holds whole batches; speculation runs them on a full copy S_{p}", size=12, fill=MUTED)

    s.text(560, y + 6, "Scalable CRUD", size=15, weight=600)
    block(s, 560, y + 22, 90, 70, CONF, CONF_T, "S", size=18)
    for n, q in enumerate((7, 8, 9)):
        card(s, 680 + n * 60, y + 40, f"Ω_{{{q}}}", SPEC, SPEC_T, w=52, h=34, size=13)
    for i in range(4):
        s.rect(866 + i * 22, y + 26, 18, 16, r=5, fill=LIVE_T, stroke=LIVE, sw=1, dash="2 2")
        s.line(875 + i * 22, y + 42, 900, y + 58, stroke=LIVE, sw=1)
    s.rect(882, y + 58, 36, 30, r=6, fill=LIVE_T, stroke=LIVE, sw=1.4)
    s.text(900, y + 78, "ω", size=14, weight=600, fill=LIVE, anchor="middle")
    s.arrow([(880, y + 73), (862, y + 73)], stroke=LIVE)
    s.text(680, y + 112, "same tape; each entry merged from per-op caches", size=12, fill=MUTED)
    legend(s, 40, 570, [("block", CONF, CONF_T, "confirmed"), ("block", SPEC, SPEC_T, "pending (speculated)"),
                        ("block", LIVE, LIVE_T, "being built")])
    s.save(os.path.join(out, f"{PFX}-b2-ledger"))


FAINT_LINE = "#b9bfca"


# ---------------------------------------------------------------------------
# B3 — footprint, drawn to scale
# ---------------------------------------------------------------------------
def b3_footprint(out):
    s = Svg(1000, 560)
    s.title("Caching structure", "B3 · what each executor keeps in memory, areas to scale")
    unit = 168                                  # side of the square for one full state
    base = 410

    def sq(x, side, colour, tint, name, size=18, dash=None):
        s.rect(x, base - side, side, side, r=10, fill=tint, stroke=colour, sw=1.6, dash=dash)
        if name:
            s.text(x + side / 2, base - side / 2 + 6, name, size=size, weight=600, fill=colour, anchor="middle")

    cols = [(60, "Dual-state", "2 × the state"), (380, "CRUD cache", "the state + pending writes"),
            (700, "Scalable CRUD", "the same, + per-op caches during a batch")]
    for x, name, sub in cols:
        s.text(x, 128, name, size=16, weight=600)
        s.text(x, 148, sub, size=12, fill=MUTED)
        s.line(x, base + 1, x + 270, base + 1, stroke=LINE, sw=1)
    x = 60
    sq(x, unit * 0.86, CONF, CONF_T, "S")
    sq(x + unit * 0.86 + 10, unit * 0.86, SPEC, SPEC_T, "S_{p}")
    for x in (380, 700):
        sq(x, unit, CONF, CONF_T, "S", size=22)
        sq(x + unit + 10, 46, SPEC, SPEC_T, "C", size=14)
        for i in range(3):
            s.rect(x + unit + 66 + i * 14, base - 26, 11, 26, r=3, fill=SPEC_T, stroke=SPEC, sw=1.1)
        s.text(x + unit + 87, base - 34, "log", size=11, fill=MUTED, anchor="middle")
    for i in range(4):
        s.rect(700 + unit + 12 + i * 15, base - 92, 11, 11, r=3, fill=LIVE_T, stroke=LIVE, sw=1.1, dash="2 2")
    s.text(700 + unit + 12, base - 100, "ℓ per op", size=11, fill=LIVE)
    s.text(60, base + 40, "a backtrack copies all of S into S_{p}", size=12.5, fill=MUTED)
    s.text(380, base + 40, "a backtrack only rebuilds C from the log", size=12.5, fill=MUTED)
    s.text(700, base + 40, "ℓ exist only while a batch runs", size=12.5, fill=MUTED)
    legend(s, 60, 520, [("block", CONF, CONF_T, "confirmed"), ("block", SPEC, SPEC_T, "speculated"),
                        ("block", LIVE, LIVE_T, "per-op cache")])
    s.save(os.path.join(out, f"{PFX}-b3-footprint"))


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "."
    os.makedirs(out, exist_ok=True)
    for f in (b1_layers, b2_ledger, b3_footprint):
        f(out)
        print(f.__name__)
