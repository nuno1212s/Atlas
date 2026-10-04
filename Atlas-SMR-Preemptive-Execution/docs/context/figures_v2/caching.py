"""Type 2 — caching structure: what each executor keeps, and where a read finds its value.

    python3 caching.py <output-dir>

One figure per executor and option. Values are the running example (executor_model.py):
Δ* = {a:1, b:2, c:3, e:7}, decisions 7, 8, 9 pending.
- CRUD cache: Ω* = Ω7 ⊕ Ω8 ⊕ Ω9 = {a:6, b:⊥, c:4, d:9}; batch 10 has written c ← 5 so far (ω).
- Dual-state: Ω* = Δ* with batches 7, 8, 9 run on it = {a:6, c:4, d:9, e:7}.
- Scalable: the same Ω*; batch 10's four ops each run in their own ℓ (o1 c ← 5, o2 e ← 8,
  o3 read e, o4 read b).
"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "figures"))
from executor_model import BATCH, CRUD, DUAL, KEYS, TOMB, TRACE10, op_text  # noqa: E402

from svgkit import (CONF, CONF_T, DROP, EXECUTORS, FAINT, INK, LINE, LIVE, LIVE_T, MUTED, PANEL,  # noqa: E402
                    SPEC, SPEC_T, WHITE, Svg, block, card, legend)

PFX = "caching"
S0 = CRUD[0].S
C0 = CRUD[0].C
LOG0 = CRUD[0].log
P0 = DUAL[0].P
OMEGA = {"c": "5"}                       # batch 10, after its first operation
FAINT_LINE = "#b9bfca"


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
        g = max(1.5, self.slot * 0.07)
        fx, fy = 1 - inset, inset
        a = (x0 + g + self.dx * fy, self.y + self.dy * fy)
        b = (x0 + self.slot - g + self.dx * fy, self.y + self.dy * fy)
        c = (x0 + self.slot - g + self.dx * fx, self.y + self.dy * fx)
        d = (x0 + g + self.dx * fx, self.y + self.dy * fx)
        return [a, b, c, d]

    def centre(self, i):
        pts = self.tile(i)
        return (sum(p[0] for p in pts) / 4, sum(p[1] for p in pts) / 4)

    def draw(self, s, colour, tint, values, opacity=0.55, size=14):
        s.poly(self.corners(), fill=tint, stroke=colour, sw=1.4, opacity=opacity)
        s.poly(self.corners(), fill="none", stroke=colour, sw=1.4)
        for i, k in enumerate(KEYS):
            pts = self.tile(i)
            if k in values:
                s.poly(pts, fill=colour, stroke=colour, sw=1)
                cx, cy = self.centre(i)
                s.text(cx, cy + size * 0.36, shown(values[k]), size=size, weight=600, fill=WHITE, anchor="middle")
            else:
                s.poly(pts, fill="none", stroke=colour, sw=0.8, dash="2 3", opacity=0.6)


def sheet_label(s, sh, name, sub, colour):
    s.text(sh.x - 20, sh.y - 6, name, size=22, weight=600, fill=colour, anchor="end")
    s.text(sh.x - 20, sh.y + 12, sub, size=11.5, fill=MUTED, anchor="end")


def ray(s, pts, passed=()):
    """A read: a line through the given points, with a hollow mark at each layer it passed."""
    s.arrow(pts, stroke=INK, sw=1.6, round_=0)
    for cx, cy in passed:
        s.circle(cx, cy, 3, fill=WHITE, stroke=INK, sw=1.2)


def log_cards(s, lx, ly, entries, label="Ω", w=92):
    for n, (q, lines) in enumerate(entries):
        x = lx + n * (w + 12)
        s.rect(x, ly, w, 58, r=8, fill=SPEC_T, stroke=SPEC, sw=1.4, shadow=True)
        s.text(x + 10, ly + 20, f"{label}_{{{q}}}" if label else f"batch {q}", size=14, weight=600, fill=SPEC)
        for r, t in enumerate(lines):
            s.text(x + 10, ly + 36 + r * 14, t, size=11.5, fill=INK)
    return [lx + n * (w + 12) for n in range(len(entries))]


def crud_log():
    return [(q, [f"{k} ← {shown(v)}" for k, v in sorted(d.items())]) for q, d, _ in LOG0]


def b1_layers(out, ex):
    s = Svg(1000, {"crud": 580, "dual": 480, "scal": 540}[ex])
    s.title(EXECUTORS[ex], "Caching structure · B1 · see-through layers: a read drops until a layer holds the key")
    if ex == "crud":
        top, gap = 210, 112
        sheets = [("ω", "the batch being run", LIVE, LIVE_T, OMEGA), ("Ω^{*}", "pending cache", SPEC, SPEC_T, C0),
                  ("Δ^{*}", "confirmed state", CONF, CONF_T, S0)]
        geo = [Sheet(170, top + j * gap) for j in range(3)]
        for j in reversed(range(3)):
            name, sub, col, tint, vals = sheets[j]
            geo[j].draw(s, col, tint, vals)
            sheet_label(s, geo[j], name, sub, col)
        for k in ("a", "b", "c", "e"):
            i = KEYS.index(k)
            hit = next(j for j, (_, _, _, _, v) in enumerate(sheets) if k in v)
            x, y_hit = geo[hit].centre(i)
            s.text(x, top - 76, f"read {k}", size=12, weight=600, fill=INK, anchor="middle")
            ray(s, [(x, top - 68), (x, y_hit - 12)], [geo[j].centre(i) for j in range(hit)])
        lx, ly = 580, 330
        s.text(lx, ly - 30, "write log", size=14, weight=600)
        s.text(lx, ly - 12, "Ω^{*} = Ω_{7} ⊕ Ω_{8} ⊕ Ω_{9}, the newest write per key", size=11.5, fill=SPEC)
        xs = log_cards(s, lx, ly, crud_log())
        s.arrow([(lx - 4, ly + 22), (geo[1].x + geo[1].w + 30, geo[1].y - 16)], stroke=SPEC, sw=1.8)
        s.label(lx - 30, ly + 4, "fold", size=11.5, fill=SPEC, bg=WHITE)
        s.arrow([(lx + 46, ly + 60), (lx + 46, geo[2].y - 14), (geo[2].x + geo[2].w + 28, geo[2].y - 14)],
                stroke=CONF, sw=2)
        s.text(lx + 56, ly + 98, "commit: the oldest, Ω_{7}, becomes Δ_{7}", size=12, fill=CONF)
        s.text(lx + 56, ly + 114, "and is written into Δ^{*}", size=12, fill=CONF)
        s.line(xs[2] - 6, ly - 4, xs[2] - 6, ly + 70, stroke=DROP, sw=2, dash="5 4")
        s.text(xs[2] + 2, ly + 82, "backtrack: drop from here, refold Ω^{*}", size=12, fill=DROP)
        s.text(170, 550, "○  the read passed an empty slot", size=11.5, fill=MUTED)
        s.text(400, 550, "⊥  deleted: the read stops and returns none", size=11.5, fill=MUTED)

    elif ex == "dual":
        top = 230
        sp, sc = Sheet(170, top), Sheet(170, top + 200)
        sc.draw(s, CONF, CONF_T, S0)
        sheet_label(s, sc, "Δ^{*}", "confirmed worker", CONF)
        sp.draw(s, SPEC, SPEC_T, P0, opacity=0.95)
        sheet_label(s, sp, "Ω^{*}", "preemptive worker", SPEC)
        for k in ("a", "b", "c", "e"):
            i = KEYS.index(k)
            x, y_hit = sp.centre(i)
            s.text(x, top - 96, f"read {k}", size=12, weight=600, fill=INK, anchor="middle")
            ray(s, [(x, top - 88), (x, y_hit - 12)])
        bx, by = sp.centre(KEYS.index("b"))
        s.text(bx + 12, by - 18, "none", size=11.5, fill=SPEC, weight=600)
        s.text(230, top + 52, "Ω^{*} is a full copy: every read stops here", size=12, fill=MUTED)
        lx, ly = 580, 214
        s.text(lx, ly - 30, "log of batches", size=14, weight=600)
        s.text(lx, ly - 12, "kept with their replies, oldest first", size=11.5, fill=MUTED)
        entries = [(q, [op_text(op) for op in BATCH[q]]) for q in (7, 8, 9)]
        xs = log_cards(s, lx, ly, entries, label=None)
        s.arrow([(lx - 4, ly + 30), (sp.x + sp.w + 34, sp.y - 16)], stroke=SPEC, sw=1.8)
        s.label(lx - 60, ly + 66, "run in order", size=11.5, fill=SPEC, bg=WHITE)
        s.arrow([(lx + 46, ly + 60), (lx + 46, sc.y - 14), (sc.x + sc.w + 28, sc.y - 14)], stroke=CONF, sw=2)
        s.text(lx + 56, ly + 98, "commit: batch 7 runs again on Δ^{*}", size=12, fill=CONF)
        s.text(lx + 56, ly + 114, "(its replies were already sent)", size=12, fill=CONF)
        s.line(xs[2] - 6, ly - 4, xs[2] - 6, ly + 70, stroke=DROP, sw=2, dash="5 4")
        s.text(xs[2] + 2, ly + 82, "backtrack: drop from here", size=12, fill=DROP)
        s.arrow([(sc.x + 30, sc.y - 34), (sc.x + 30, sp.y + 4)], stroke=DROP, sw=2, dash="5 4")
        s.text(sc.x + 40, (sc.y + sp.y) / 2 + 10, "backtrack: copy all of Δ^{*} into Ω^{*}, then replay batch 8",
               size=12, fill=DROP)

    else:
        top = 196
        cs, ss = Sheet(170, top + 112), Sheet(170, top + 224)
        ss.draw(s, CONF, CONF_T, S0)
        sheet_label(s, ss, "Δ^{*}", "confirmed state", CONF)
        cs.draw(s, SPEC, SPEC_T, C0)
        sheet_label(s, cs, "Ω^{*}", "pending cache", SPEC)
        minis = []
        for pos in range(4):
            local, _, _ = TRACE10.lanes[pos]
            m = Sheet(150 + pos * 92, top, slot=14, dx=18, dy=-14, pad=4)
            m.draw(s, LIVE, LIVE_T, local, size=9.5)
            s.text(m.x + m.w / 2 + 6, top + 18, f"ℓ_{{{pos + 1}}} · o{pos + 1}", size=11.5, weight=600,
                   fill=LIVE, anchor="middle")
            minis.append(m)
        s.text(150, top - 34, "ℓ  one private cache per op, in parallel", size=12, weight=600, fill=LIVE)
        # o3's read of e: its own ℓ3, then the shared C, then S (where it finds the stale 7)
        i = KEYS.index("e")
        x0, y0 = minis[2].centre(i)
        cx, cy = cs.centre(i)
        hx, hy = ss.centre(i)
        ray(s, [(x0, y0), (cx, cy), (hx, hy - 12)], [(x0, y0), (cx, cy)])
        s.text(hx + 14, hy - 30, "o3: read e → 7", size=12, weight=600, fill=INK)
        s.text(170, top + 290, "Each op reads its own ℓ, then the shared Ω^{*} and Δ^{*}; writes stay in its ℓ.",
               size=12, fill=MUTED)
        s.text(170, top + 308, "o3 cannot see o2's write of e (8): the worker re-runs both afterwards.",
               size=12, fill=MUTED)
        lx, ly = 600, 330
        s.text(lx, ly - 30, "write log", size=14, weight=600)
        s.text(lx, ly - 12, "Ω^{*} = Ω_{7} ⊕ Ω_{8} ⊕ Ω_{9}", size=11.5, fill=SPEC)
        log_cards(s, lx, ly, crud_log(), w=84)
        s.rect(lx + 288, ly, 84, 58, r=8, fill=WHITE, stroke=SPEC, sw=1.4, dash="5 4")
        s.text(lx + 330, ly + 34, "Ω_{10}", size=14, weight=600, fill=SPEC, anchor="middle")
        s.arrow([(minis[3].x + minis[3].w + 30, top - 10), (lx + 330, top - 10), (lx + 330, ly - 2)], stroke=LIVE)
        s.text(lx + 150, top - 18, "merge the ℓs, re-run colliders", size=11.5, fill=LIVE, anchor="middle")
        s.arrow([(lx - 4, ly + 22), (cs.x + cs.w + 30, cs.y - 16)], stroke=SPEC, sw=1.8)
        s.label(lx - 30, ly + 4, "fold", size=11.5, fill=SPEC, bg=WHITE)
        s.text(lx, ly + 98, "commit and backtrack: as in the CRUD cache", size=12, fill=MUTED)
    s.save(os.path.join(out, f"{PFX}-b1-layers-{ex}"))


# ---------------------------------------------------------------------------
# B2 — a ledger tape
# ---------------------------------------------------------------------------
def tape(s, y, entries, label, w=104, x0=222, gap=12, newest=True):
    s.rect(x0 - 8, y + 34, 3 * (w + gap) + w + 18, 84, r=12, fill=PANEL)
    xs = []
    for n, (q, lines) in enumerate(entries):
        x = x0 + n * (w + gap)
        xs.append(x)
        s.rect(x, y + 42, w, 68, r=9, fill=SPEC_T, stroke=SPEC, sw=1.5, shadow=True)
        s.text(x + 12, y + 64, f"Ω_{{{q}}}" if label else f"batch {q}", size=15, weight=600, fill=SPEC)
        for r, t in enumerate(lines):
            s.text(x + 12, y + 82 + r * 15, t, size=12, fill=INK)
    xn = x0 + 3 * (w + gap)
    s.rect(xn, y + 42, w, 68, r=9, fill=WHITE, stroke=SPEC, sw=1.4, dash="5 4")
    s.text(xn + w / 2, y + 81, "next Ω" if label else "next batch", size=13, fill=SPEC, anchor="middle")
    s.text(x0 + 4, y + 22, "oldest", size=11, fill=MUTED)
    if newest:
        s.text(xn + w, y + 22, "newest", size=11, fill=MUTED, anchor="end")
    s.line(x0 + 50, y + 18, xn + 50, y + 18, stroke=FAINT_LINE, sw=1, arrow=FAINT_LINE)
    return xs, xn


def key_strip(s, x, y, values, colour, tint, title, sub):
    s.rect(x, y + 18, 200, 116, r=12, fill=tint, stroke=colour, sw=1.6, shadow=True)
    s.text(x + 14, y + 42, title, size=14, weight=600, fill=colour)
    for i, k in enumerate(KEYS):
        kx = x + 16 + i * 36
        s.text(kx + 14, y + 66, k, size=11, fill=MUTED, anchor="middle")
        if k in values:
            s.rect(kx, y + 74, 28, 26, r=5, fill=colour, stroke=colour)
            s.text(kx + 14, y + 92, shown(values[k]), size=13, weight=600, fill=WHITE, anchor="middle")
        else:
            s.rect(kx, y + 74, 28, 26, r=5, fill="none", stroke=colour, sw=0.9, dash="2 3")
    s.text(x + 14, y + 122, sub, size=11, fill=MUTED)


def b2_ledger(out, ex):
    y = 160 if ex == "scal" else 120
    s = Svg(1000, y + 240)
    s.title(EXECUTORS[ex], "Caching structure · B2 · a ledger: confirmed on the left, pending decisions on a tape")
    block(s, 40, y, 150, 150, CONF, CONF_T, "Δ^{*}", size=24, sub="confirmed")
    if ex == "dual":
        entries = [(q, [op_text(op) for op in BATCH[q]]) for q in (7, 8, 9)]
        xs, xn = tape(s, y, entries, label=False)
    else:
        xs, xn = tape(s, y, crud_log(), label=True, newest=ex != "scal")
    cx0 = 760
    if ex == "dual":
        s.arrow([(xn + 108, y + 76), (cx0 - 6, y + 76)], stroke=SPEC, sw=1.8)
        s.label((xn + 108 + cx0) / 2, y + 66, "run", size=11.5, fill=SPEC, bg=WHITE)
        key_strip(s, cx0, y, P0, SPEC, SPEC_T, "Ω^{*}  full copy", "Δ^{*} with every batch run on it")
        s.arrow([(xs[0] + 20, y + 112), (xs[0] + 20, y + 132), (194, y + 132)], stroke=CONF, sw=2)
        s.text(xs[0] - 4, y + 166, "commit: batch 7 runs again on Δ^{*}", size=12, fill=CONF)
        s.line(xs[2] - 6, y + 26, xs[2] - 6, y + 126, stroke=DROP, sw=2, dash="5 4")
        s.text(xs[2] - 2, y + 144, "backtrack: cut here, copy Δ^{*} into Ω^{*},", size=12, fill=DROP)
        s.text(xs[2] - 2, y + 160, "replay what is left", size=12, fill=DROP)
    else:
        s.arrow([(xn + 108, y + 76), (cx0 - 6, y + 76)], stroke=SPEC, sw=1.8)
        s.label((xn + 108 + cx0) / 2, y + 66, "fold", size=11.5, fill=SPEC, bg=WHITE)
        key_strip(s, cx0, y, C0, SPEC, SPEC_T, "Ω^{*}  pending cache", "the newest write per key")
        s.arrow([(xs[0] + 20, y + 112), (xs[0] + 20, y + 132), (194, y + 132)], stroke=CONF, sw=2)
        s.text(xs[0] - 4, y + 166, "commit: Ω_{7} becomes Δ_{7}, written into Δ^{*}", size=12, fill=CONF)
        s.line(xs[2] - 6, y + 26, xs[2] - 6, y + 126, stroke=DROP, sw=2, dash="5 4")
        s.text(xs[2] - 2, y + 144, "backtrack: cut here, refold Ω^{*}", size=12, fill=DROP)
    if ex == "scal":
        # the next entry is assembled from one tiny cache per op
        for pos in range(4):
            local = TRACE10.lanes[pos][0]
            x = xn + 2 + pos * 26
            s.rect(x, y - 30, 22, 20, r=6, fill=LIVE_T, stroke=LIVE, sw=1.2, dash="3 2")
            s.text(x + 11, y - 16, "".join(f"{v}" for v in local.values()) or "–", size=10, weight=600,
                   fill=LIVE, anchor="middle")
            s.line(x + 11, y - 10, xn + 52, y + 40, stroke=LIVE, sw=1)
        s.text(xn + 52, y - 40, "ℓ_{1} … ℓ_{4}, merged", size=11, fill=LIVE, anchor="middle")
    items = [("block", CONF, CONF_T, "confirmed"), ("block", SPEC, SPEC_T, "pending (speculated)")]
    if ex == "scal":
        items.append(("block", LIVE, LIVE_T, "per-op cache"))
    legend(s, 40, y + 220, items)
    s.save(os.path.join(out, f"{PFX}-b2-ledger-{ex}"))


# ---------------------------------------------------------------------------
# B3 — footprint, drawn to scale
# ---------------------------------------------------------------------------
def b3_footprint(out, ex):
    s = Svg(640, 520)
    s.title(EXECUTORS[ex], "Caching structure · B3 · what it keeps in memory, to scale")
    unit, base, x = 190, 400, 60

    def sq(x, side, colour, tint, name, size=18, dash=None, opacity=None):
        s.rect(x, base - side, side, side, r=10, fill=tint, stroke=colour, sw=1.6, dash=dash, opacity=opacity)
        if name:
            s.text(x + side / 2, base - side / 2 + 6, name, size=size, weight=600, fill=colour, anchor="middle")

    s.line(x, base + 1, 600, base + 1, stroke=LINE, sw=1)
    if ex == "dual":
        sq(x, unit, CONF, CONF_T, "Δ^{*}", size=22)
        sq(x + unit + 14, unit, SPEC, SPEC_T, "Ω^{*}", size=22)
        s.text(x, 150, "2 × the state", size=15, weight=600)
        s.text(x, base + 40, "Ω^{*} is a second full copy, kept in sync by re-running every batch;", size=12.5,
               fill=MUTED)
        s.text(x, base + 58, "a backtrack copies all of Δ^{*} into it", size=12.5, fill=MUTED)
    else:
        sq(x, unit, CONF, CONF_T, "Δ^{*}", size=22)
        sq(x + unit + 14, unit, "#c4c9d2", WHITE, None, dash="5 5")
        s.text(x + unit + 14 + unit / 2, base - unit / 2 - 4, "no second copy", size=12, fill=MUTED,
               anchor="middle")
        sq(x + unit + 30, 52, SPEC, SPEC_T, "Ω^{*}", size=15)
        for i in range(3):
            s.rect(x + unit + 92 + i * 15, base - 28, 12, 28, r=3, fill=SPEC_T, stroke=SPEC, sw=1.1)
        s.text(x + unit + 114, base - 36, "log", size=11, fill=MUTED, anchor="middle")
        s.text(x, 150, "the state + the pending writes", size=15, weight=600)
        if ex == "scal":
            for i in range(4):
                s.rect(x + unit + 30 + i * 16, base - 92, 12, 12, r=3, fill=LIVE_T, stroke=LIVE, sw=1.1, dash="2 2")
            s.text(x + unit + 30, base - 100, "ℓ per op, only while a batch runs", size=11, fill=LIVE)
        s.text(x, base + 40, "Ω^{*} and the log grow with what is pending, not with the state;", size=12.5,
               fill=MUTED)
        s.text(x, base + 58, "a backtrack only rebuilds Ω^{*} from the log", size=12.5, fill=MUTED)
    items = [("block", CONF, CONF_T, "confirmed"), ("block", SPEC, SPEC_T, "speculated")]
    if ex == "scal":
        items.append(("block", LIVE, LIVE_T, "per-op cache"))
    legend(s, 60, 500, items)
    s.save(os.path.join(out, f"{PFX}-b3-footprint-{ex}"))


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "."
    os.makedirs(out, exist_ok=True)
    for f in (b1_layers, b2_ledger, b3_footprint):
        for ex in EXECUTORS:
            f(out, ex)
        print(f.__name__)
