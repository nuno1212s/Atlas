"""Type 3 — the lifecycle of a decision: speculated (Ω), then confirmed (Δ) or discarded.

    python3 lifecycle.py <output-dir>
"""
import os
import sys

from svgkit import (EXECUTORS, POOL_T, CONF, CONF_T, CONS, CONS_T, DROP, DROP_T, FAINT, INK, LINE, LIVE, LIVE_T, MUTED, PANEL, SPEC,
                    SPEC_T, WHITE, Svg, block, capsule, card, person, text_width)

PFX = "lifecycle"
GREY, GREY_T = "#7d8597", "#eceef2"


def node(s, cx, cy, w, h, name, sub, colour, tint, size=17):
    s.rect(cx - w / 2, cy - h / 2, w, h, r=h / 2, fill=tint, stroke=colour, sw=1.8, shadow=True)
    s.text(cx, cy + (0 if sub else 6), name, size=size, weight=600, fill=colour, anchor="middle")
    if sub:
        s.text(cx, cy + 17, sub, size=11.5, fill=MUTED, anchor="middle")


def chip(s, x, y, who, what, colour):
    w = text_width(who, 11, 600) + text_width(what, 11) + 22
    s.rect(x, y - 12, w, 20, r=10, fill=WHITE, stroke=colour, sw=1)
    s.text(x + 9, y + 2, who, size=11, weight=600, fill=colour)
    s.text(x + 13 + text_width(who, 11, 600), y + 2, what, size=11, fill=INK)
    return w


# ---------------------------------------------------------------------------
# C1 — a state machine for one decision
# ---------------------------------------------------------------------------
HOW = {
    "crud": dict(spec="in a container over Ω^{*}, Δ^{*}", commit="apply Ω_{k} to Δ^{*}, refold Ω^{*}",
                 back="drop Ω_{k} onwards, refold Ω^{*}"),
    "dual": dict(spec="in place, on the copy Ω^{*}", commit="run batch k again on Δ^{*}",
                 back="copy all of Δ^{*} into Ω^{*}, replay the rest"),
    "scal": dict(spec="a container per op, in parallel", commit="apply Ω_{k} to Δ^{*}, refold Ω^{*}",
                 back="drop Ω_{k} onwards, refold Ω^{*}"),
}


def c1_states(out, ex):
    s = Svg(1000, 540)
    s.title(EXECUTORS[ex], "Decision lifecycle · C1 · the states one decision goes through")
    how = HOW[ex]
    D, ST, O, C, X = (130, 290), (330, 160), (480, 290), (820, 290), (480, 440)
    node(s, *D, 170, 60, "batch k", "decided by consensus", CONS, CONS_T)
    node(s, *ST, 150, 52, "staged", "waits for k−1", GREY, GREY_T, size=15)
    node(s, *O, 150, 64, "Ω_{k}", "speculated", SPEC, SPEC_T, size=22)
    node(s, *C, 190, 64, "Δ_{k}", "confirmed · replies sent", CONF, CONF_T, size=22)
    node(s, *X, 150, 52, "discarded", None, DROP, DROP_T, size=15)

    s.arrow([(D[0] + 86, D[1]), (O[0] - 78, O[1])], stroke=SPEC, sw=2)
    s.label((D[0] + O[0]) / 2 + 4, D[1] - 10, "speculate", size=12, fill=SPEC, weight=600)
    s.label((D[0] + O[0]) / 2 + 4, D[1] + 22, "k is next", size=11, fill=MUTED)
    s.text((D[0] + O[0]) / 2 + 4, D[1] + 40, how["spec"], size=11.5, fill=SPEC, anchor="middle")
    s.arrow([(D[0] + 20, D[1] - 32), (ST[0] - 76, ST[1])], stroke=GREY, curve=-0.25)
    s.label(D[0] + 10, ST[1] + 6, "arrives early", size=11, fill=MUTED)
    s.arrow([(ST[0] + 50, ST[1] + 27), (O[0] - 30, O[1] - 33)], stroke=GREY)
    s.label(ST[0] + 110, ST[1] + 56, "gap closes", size=11, fill=MUTED)

    s.arrow([(O[0] + 76, O[1]), (C[0] - 96, C[1])], stroke=CONF, sw=2.4)
    s.label((O[0] + C[0]) / 2, O[1] - 10, "commit", size=12, fill=CONF, weight=600)
    s.label((O[0] + C[0]) / 2, O[1] + 22, "consensus confirms k", size=11, fill=MUTED)
    s.text((O[0] + C[0]) / 2, O[1] + 40, how["commit"], size=11.5, fill=CONF, anchor="middle")

    s.arrow([(O[0], O[1] + 33), (X[0], X[1] - 27)], stroke=DROP, sw=2)
    s.label(O[0] + 12, (O[1] + X[1]) / 2 - 4, "backtrack", size=12, fill=DROP, weight=600, anchor="start")
    s.text(O[0] + 12, (O[1] + X[1]) / 2 + 14, how["back"], size=11.5, fill=DROP)
    s.arrow([(X[0] - 76, X[1]), (D[0], X[1]), (D[0], D[1] + 31)], stroke=DROP, dash="5 4")
    s.label(D[0] + 70, X[1] - 8, "k is decided again", size=11, fill=DROP)

    s.arrow([(ST[0] + 76, ST[1]), (C[0] - 60, ST[1]), (C[0] - 40, C[1] - 33)], stroke=GREY, dash="5 4")
    s.label(600, ST[1] - 8, "confirmed while still staged: run it without speculating", size=11, fill=MUTED)
    s.save(os.path.join(out, f"{PFX}-c1-states-{ex}"))


# ---------------------------------------------------------------------------
# C2 — a conveyor belt feeding S
# ---------------------------------------------------------------------------
def belt(s, y, x0=330, x1=820):
    s.rect(x0, y + 30, x1 - x0, 12, r=6, fill="#e6e8ed")
    for x in range(x0 + 16, x1 - 8, 34):
        s.circle(x, y + 36, 3.5, fill=WHITE, stroke=FAINT, sw=1)
    s.arrow([(x1 - 30, y + 58), (x0 + 30, y + 58)], stroke=FAINT, sw=1.2)


def omega(s, x, y, q, colour=SPEC, tint=SPEC_T, letter="Ω", dash=None, opacity=None):
    card(s, x, y - 22, f"{letter}_{{{q}}}", colour, tint, w=62, h=50, size=17, dash=dash, opacity=opacity)


def c2_belt(out, ex):
    s = Svg(1000, 700)
    sub = {"crud": "decisions ride towards Δ^{*} and are written in when confirmed",
           "dual": "batches ride towards Δ^{*}; Ω^{*} already holds their effect",
           "scal": "decisions ride towards Δ^{*}; each is built from parallel ops"}[ex]
    s.title(EXECUTORS[ex], f"Decision lifecycle · C2 · a conveyor: {sub}")
    frames = [(130, "1", "speculate Ω_{10}"), (310, "2", "commit Ω_{7}"), (490, "3", "backtrack to Ω_{9}")]
    for y, n, t in frames:
        s.circle(46, y + 14, 14, fill=INK)
        s.text(46, y + 19, n, size=13, weight=600, fill=WHITE, anchor="middle")
        s.text(70, y + 19, t, size=15, weight=600)
        block(s, 190, y - 4, 112, 92, CONF, CONF_T, "Δ^{*}", size=22)
        belt(s, y + 10)
        if ex == "dual":
            block(s, 850, y - 4, 110, 92, SPEC, SPEC_T, "Ω^{*}", size=22, sub="full copy")
        if y < 490:
            s.line(30, y + 152, 970, y + 152, stroke=LINE, sw=1)
    xs = [340 + i * 74 for i in range(6)]

    # 1 speculate: Ω10 joins the end of the line
    y = 130
    for i, q in enumerate((7, 8, 9)):
        omega(s, xs[i], y + 18, q)
    omega(s, xs[3], y + 18, 10)
    if ex == "crud":
        capsule(s, 850, y - 2, 110, 34, "batch 10")
        s.arrow([(850, y + 15), (xs[3] + 64, y + 2)], stroke=LIVE, curve=0.15)
        s.text(905, y + 52, "runs, its writes", size=11.5, fill=MUTED, anchor="middle")
        s.text(905, y + 67, "become Ω_{10}", size=11.5, fill=MUTED, anchor="middle")
    elif ex == "dual":
        s.arrow([(xs[3] + 64, y + 10), (846, y + 30)], stroke=SPEC, sw=2, curve=-0.15)
        s.text(xs[3] + 120, y - 16, "batch 10 runs in place on Ω^{*};", size=11.5, fill=MUTED)
        s.text(xs[3] + 120, y - 2, "the belt keeps it with its replies", size=11.5, fill=MUTED)
    else:
        s.rect(844, y - 14, 122, 66, r=10, fill=POOL_T)
        for i in range(4):
            capsule(s, 850 + (i % 2) * 58, y - 8 + (i // 2) * 28, 52, 22, f"op {i + 1}", size=10.5)
        s.arrow([(844, y + 18), (xs[3] + 64, y + 4)], stroke=LIVE, curve=0.1)
        s.text(905, y + 70, "four containers in", size=11.5, fill=MUTED, anchor="middle")
        s.text(905, y + 85, "parallel, merged: Ω_{10}", size=11.5, fill=MUTED, anchor="middle")

    # 2 commit
    y = 310
    if ex == "dual":
        card(s, 250, y + 50, "k = 7", CONF, WHITE, w=56, h=36, size=13)
        s.arrow([(xs[0] - 6, y + 22), (306, y + 52)], stroke=CONF, sw=2.2, curve=0.2)
        s.text(xs[0] + 30, y + 92, "batch 7 runs again on Δ^{*}", size=11.5, fill=CONF, anchor="middle")
    else:
        card(s, 250, y + 50, "Δ_{7}", CONF, WHITE, w=56, h=36, size=15)
        s.arrow([(xs[0] - 6, y + 22), (306, y + 52)], stroke=CONF, sw=2.2, curve=0.2)
        s.text(xs[0] + 30, y + 92, "Ω_{7} → Δ_{7}, written into Δ^{*}", size=11.5, fill=CONF, anchor="middle")
    for i, q in enumerate((8, 9, 10)):
        omega(s, xs[i + 1], y + 18, q)
    omega(s, xs[0], y + 18, 7, opacity=0.25, dash="4 3")
    for i in range(3):
        person(s, 96 + i * 20, y + 92, scale=0.8)
    s.arrow([(186, y + 88), (150, y + 92)], stroke=INK)
    s.text(116, y + 62, "replies of Δ_{7}", size=11.5, fill=MUTED, anchor="middle")

    # 3 backtrack: batch 9 was decided differently, so Ω9 and Ω10 fall off
    y = 490
    omega(s, xs[1], y + 18, 8)
    for i, q in ((2, 9), (3, 10)):
        omega(s, xs[i] + 14, y + 96, q, colour=DROP, tint=DROP_T, opacity=0.8)
        s.arrow([(xs[i] + 31, y + 46), (xs[i] + 45, y + 70)], stroke=DROP)
    s.rect(xs[2] - 6, y - 10, 140, 62, r=10, fill="none", stroke=DROP, sw=1.4, dash="5 4")
    if ex == "dual":
        s.arrow([(246, y + 88), (246, y + 172), (905, y + 172), (905, y + 88)], stroke=DROP, sw=2, dash="5 4")
        s.label(330, y + 176, "copy all of Δ^{*} into Ω^{*}, then replay Ω_{8}", size=11.5, fill=DROP, bg=WHITE,
                anchor="start")
        s.text(xs[2] + 162, y + 132, "dropped; batch 9 runs again", size=11.5, fill=DROP)
    else:
        capsule(s, 850, y - 2, 110, 34, "batch 9′", colour=LIVE, dash="4 3")
        s.text(905, y + 52, "decided again,", size=11.5, fill=MUTED, anchor="middle")
        s.text(905, y + 67, "runs from here", size=11.5, fill=MUTED, anchor="middle")
        s.arrow([(850, y + 15), (xs[2] + 4, y + 2)], stroke=LIVE, dash="4 3", curve=0.12)
        s.text(xs[2] + 162, y + 132, "dropped; Ω^{*} is rebuilt from Ω_{8}", size=11.5, fill=DROP)
    s.save(os.path.join(out, f"{PFX}-c2-belt-{ex}"))


# ---------------------------------------------------------------------------
# C3 — a branch graph
# ---------------------------------------------------------------------------
def dot(s, x, y, q, kind, ring=False, cross=False):
    colour, tint, letter = {"D": (CONF, CONF, "Δ"), "O": (SPEC, SPEC_T, "Ω"), "X": (DROP, DROP_T, "Ω"),
                            "N": (LIVE, LIVE_T, "Ω")}[kind]
    if ring:
        s.circle(x, y, 23, fill="none", stroke=colour, sw=1.5, dash="3 3")
    s.circle(x, y, 17, fill=tint, stroke=colour, sw=2, dash="4 3" if kind == "N" else None,
             opacity=0.55 if kind == "X" else None)
    s.text(x, y + 5.5, str(q), size=14, weight=600, fill=WHITE if kind == "D" else colour, anchor="middle")
    s.text(x, y + 40, f"{letter}_{{{q}}}", size=13, fill=colour, anchor="middle", opacity=0.6 if kind == "X" else None)
    if cross:
        s.line(x - 12, y - 12, x + 12, y + 12, stroke=DROP, sw=2.2)
        s.line(x - 12, y + 12, x + 12, y - 12, stroke=DROP, sw=2.2)


def flag(s, x, y, name, colour, below=False):
    """A position marker above (or below) a node: where Δ* or Ω* currently is."""
    if below:
        s.path(f"M{x - 6},{y + 28} L{x + 6},{y + 28} L{x},{y + 21} Z", fill=colour, stroke="none")
        s.text(x, y + 62, name, size=13, weight=600, fill=colour, anchor="middle")
    else:
        s.text(x, y - 34, name, size=13, weight=600, fill=colour, anchor="middle")
        s.path(f"M{x - 6},{y - 28} L{x + 6},{y - 28} L{x},{y - 21} Z", fill=colour, stroke="none")


def c3_branch(out, ex):
    s = Svg(1000, 640)
    s.title(EXECUTORS[ex], "Decision lifecycle · C3 · a branch graph: confirmed history in blue, speculation ahead")
    step = 92
    x0 = 300
    rows = [(160, "1", "speculate Ω_{10}"), (330, "2", "commit Ω_{7}"), (500, "3", "backtrack to Ω_{9}")]
    for y, n, t in rows:
        s.circle(46, y, 14, fill=INK)
        s.text(46, y + 5, n, size=13, weight=600, fill=WHITE, anchor="middle")
        s.text(70, y + 5, t, size=15, weight=600)

    def lineup(y, conf, spec, extra=None):
        xs = {q: x0 + (q - 5) * step for q in range(5, 12)}
        last_conf = max(conf)
        s.line(x0 - 60, y, xs[last_conf], y, stroke=CONF, sw=5)
        if spec:
            s.line(xs[last_conf], y, xs[max(spec)], y, stroke=SPEC, sw=3, dash="7 5")
        for q in conf:
            dot(s, xs[q], y, q, "D")
        flag(s, xs[last_conf], y, "Δ^{*}", CONF)
        return xs

    y = 160
    xs = lineup(y, (5, 6), (7, 8, 9, 10))
    for q in (7, 8, 9):
        dot(s, xs[q], y, q, "O")
    dot(s, xs[10], y, 10, "O", ring=True)
    s.text(xs[10] + 34, y + 5, "new", size=12, fill=SPEC)
    if ex == "dual":
        flag(s, xs[10], y, "Ω^{*}", SPEC)
    if ex == "scal":
        for i in range(4):
            px = xs[10] - 36 + i * 24
            s.circle(px, y - 52, 7, fill=LIVE_T, stroke=LIVE, sw=1.2, dash="2 2")
            s.line(px, y - 45, xs[10], y - 24, stroke=LIVE, sw=1)
        s.text(xs[10] + 50, y - 48, "4 ops in parallel", size=11.5, fill=LIVE)

    y = 330
    xs = lineup(y, (5, 6, 7), (8, 9, 10))
    for q in (8, 9, 10):
        dot(s, xs[q], y, q, "O")
    s.circle(xs[7], y, 23, fill="none", stroke=CONF, sw=1.5, dash="3 3")
    if ex == "dual":
        flag(s, xs[10], y, "Ω^{*}", SPEC)
        s.text(xs[7], y + 62, "batch 7 runs again on Δ^{*}: Δ^{*} moves up", size=12, fill=CONF, anchor="middle")
    else:
        s.text(xs[7], y + 62, "Ω_{7} → Δ_{7}: Δ^{*} moves up", size=12, fill=CONF, anchor="middle")

    y = 500
    xs = lineup(y, (5, 6, 7), (8,))
    dot(s, xs[8], y, 8, "O")
    s.line(xs[8] + 17, y, xs[10], y, stroke=DROP, sw=2, dash="3 5", opacity=0.6)
    dot(s, xs[9], y, 9, "X", cross=True)
    dot(s, xs[10], y, 10, "X", cross=True)
    s.path(f"M{xs[8] + 12},{y + 12} C{xs[8] + 40},{y + 40} {xs[8] + 50},{y + 64} {xs[8] + 75},{y + 64}", stroke=LIVE, sw=3,
           dash="7 5")
    dot(s, xs[8] + 92, y + 64, 9, "N")
    s.text(xs[8] + 124, y + 69, "batch 9, decided again", size=12, fill=LIVE)
    s.text(xs[10] + 34, y + 5, "dropped", size=12, fill=DROP)
    if ex == "dual":
        s.path(f"M{xs[10]},{y - 24} C{xs[10]},{y - 70} {xs[7]},{y - 70} {xs[7] + 10},{y - 46}", stroke=DROP,
               sw=2, dash="5 4", arrow=DROP)
        s.label((xs[7] + xs[10]) / 2, y - 66, "Ω^{*} ← a copy of Δ^{*}, then replay Ω_{8}", size=11.5, fill=DROP, bg=WHITE)
        flag(s, xs[8], y, "Ω^{*}", SPEC)
    s.save(os.path.join(out, f"{PFX}-c3-branch-{ex}"))


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "."
    os.makedirs(out, exist_ok=True)
    for f in (c1_states, c2_belt, c3_branch):
        for ex in EXECUTORS:
            f(out, ex)
        print(f.__name__)
