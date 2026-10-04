"""Type 4 — how requests flow, which threads they run on, and the containers they run inside.

    python3 requests.py <output-dir>

A container is the proxy an operation executes against (it implements CRUDState): it reads the
speculated view (Ω* over Δ*) and writes only into the decision being built.
"""
import os
import sys

from svgkit import (EXECUTORS, CONF, CONF_T, CONS, CONS_T, DROP, FAINT, INK, LINE, LIVE, LIVE_T, MUTED, PANEL, POOL, POOL_T,
                    READP, SPEC, SPEC_T, WHITE, WORK, WORK_T, Svg, block, capsule, card, person, text_width,
                    thread_badge)

PFX = "requests"


# ---------------------------------------------------------------------------
# D1 — a subway map
# ---------------------------------------------------------------------------
def station(s, x, y, label, colour=WORK, below=True, size=11.5, fill=WHITE):
    s.circle(x, y, 8, fill=fill, stroke=colour, sw=3)
    if label:
        for i, ln in enumerate(label.split("\n")):
            s.text(x, y + (26 + i * 14 if below else -18 - i * 14), ln, size=size, fill=INK, anchor="middle")


def cstation(s, x, y, w, label, sub=None, colour=LIVE):
    """A container station: a capsule sitting on the line."""
    capsule(s, x - w / 2, y - 14, w, 28, label, size=11.5)
    if sub:
        s.text(x, y + 32, sub, size=11, fill=MUTED, anchor="middle")


def consensus_dot(s, x, y, label):
    s.circle(x, y, 10, fill=CONS)
    s.text(x, y - 18, label, size=11.5, weight=600, fill=CONS, anchor="middle")


def d1_subway(out, ex):
    oy, h = {"crud": (0, 300), "dual": (190, 330), "scal": (400, 300)}[ex]
    s = Svg(1000, h, oy=oy)
    s.title(EXECUTORS[ex], "Request flow · D1 · a subway map: each thread is a line, each step a station")
    X = dict(start=86, append=580, confirm=700, commit=790, reply=890)

    def tail(y, colour=WORK, commit_label="commit\nΩ_{k} → Δ_{k}"):
        s.line(X["append"], y, X["confirm"] - 14, y, stroke=colour, sw=6, dash="2 10")
        s.text((X["append"] + X["confirm"]) / 2, y - 14, "waits", size=11, fill=MUTED, anchor="middle")
        s.line(X["confirm"], y, X["reply"], y, stroke=colour, sw=6)
        consensus_dot(s, X["confirm"], y - 44, "confirm k")
        s.arrow([(X["confirm"], y - 34), (X["confirm"], y - 12)], stroke=CONS)
        station(s, X["confirm"], y, None, colour)
        station(s, X["commit"], y, commit_label, CONF)
        for i in range(2):
            person(s, X["reply"] + 14 + i * 18, y + 2, scale=0.8)
        s.text(X["reply"] + 23, y + 30, "replies", size=11.5, fill=INK, anchor="middle")

    if ex == "crud":
        # --- CRUD cache --------------------------------------------------------
        y = 180
        thread_badge(s, 32, y - 38, "worker", WORK)
        s.line(X["start"], y, X["append"], y, stroke=WORK, sw=6)
        consensus_dot(s, X["start"], y, "batch k")
        cstation(s, 300, y, 220, "run batch k, op by op", "a container over Ω^{*}, Δ^{*}")
        station(s, X["append"], y, "append Ω_{k}", SPEC)
        tail(y)

    if ex == "dual":
        # --- dual-state -----------------------------------------------------------
        y = 390
        yp, yc = y - 10, y + 76
        s.line(X["start"], y - 44, 170, y - 44, stroke=WORK, sw=6)
        thread_badge(s, 150, y - 68, "main", WORK)
        consensus_dot(s, X["start"], y - 44, "batch k")
        s.path(f"M170,{y - 44} Q200,{y - 44} 200,{y - 24} L200,{yp - 16} Q200,{yp} 220,{yp}", stroke=WORK, sw=6)
        thread_badge(s, 214, yp - 22, "preemptive", WORK)
        s.line(220, yp, X["append"], yp, stroke=WORK, sw=6)
        s.rect(300 - 92, yp - 14, 184, 28, r=6, fill=SPEC_T, stroke=SPEC, sw=1.6)
        s.text(300, yp + 5, "run batch k on Ω^{*}", size=11.5, weight=600, fill=SPEC, anchor="middle")
        s.text(300, yp + 32, "no container: in place", size=11, fill=MUTED, anchor="middle")
        station(s, X["append"], yp, "log batch k\n+ replies", SPEC)
        tail(yp, commit_label="send\nreplies")
        # the confirmed worker re-runs k
        s.line(X["confirm"] + 30, yc, X["reply"] - 10, yc, stroke=CONF, sw=6)
        s.path(f"M{X['confirm']},{yp + 8} L{X['confirm']},{yc - 16} Q{X['confirm']},{yc} {X['confirm'] + 16},{yc} "
               f"L{X['confirm'] + 30},{yc}", stroke=CONF, sw=6)
        thread_badge(s, X["confirm"] + 34, yc - 24, "confirmed", WORK)
        station(s, X["commit"] + 20, yc, "run k again on Δ^{*}", CONF)

    if ex == "scal":
        # --- scalable --------------------------------------------------------------
        y = 600
        thread_badge(s, 32, y - 54, "worker", WORK)
        thread_badge(s, 180, y - 70, "exec pool", POOL)
        s.line(X["start"], y, 160, y, stroke=WORK, sw=6)
        consensus_dot(s, X["start"], y, "batch k")
        tracks = [y - 39, y - 13, y + 13, y + 39]
        for i, ty in enumerate(tracks):
            s.path(f"M160,{y} C185,{y} 180,{ty} 205,{ty} L305,{ty} C330,{ty} 325,{y} 350,{y}", stroke=POOL, sw=4)
            capsule(s, 222, ty - 10, 66, 20, f"op {i + 1}", size=10.5)
        s.line(160, y, 350, y, stroke=WORK, sw=2, dash="2 6")
        s.text(255, y + 66, "fork: a container per op, in parallel", size=11, fill=MUTED, anchor="middle")
        s.line(350, y, X["append"], y, stroke=WORK, sw=6)
        station(s, 384, y, "collide?", WORK)
        cstation(s, 478, y, 100, "re-run o2, o3", "on the worker")
        station(s, X["append"], y, "append Ω_{k}", SPEC)
        tail(y)
    s.save(os.path.join(out, f"{PFX}-d1-subway-{ex}"))


# ---------------------------------------------------------------------------
# D2 — minimal sequence diagrams, one per executor
# ---------------------------------------------------------------------------
def head(s, x, y, name, colour, tint):
    w = text_width(name, 12, 600) + 22
    s.rect(x - w / 2, y - 14, w, 28, r=14, fill=tint, stroke=colour, sw=1.4)
    s.text(x, y + 4.5, name, size=12, weight=600, fill=colour, anchor="middle")


def lifeline(s, x, y0, y1):
    s.line(x, y0, x, y1, stroke=LINE, sw=1.5, dash="4 4")


def msg(s, x1, x2, y, label, colour=INK, dash=None, above=True):
    s.arrow([(x1, y), (x2, y)], stroke=colour, dash=dash)
    s.text((x1 + x2) / 2, y - 7 if above else y + 16, label, size=11, fill=colour, anchor="middle")


def act(s, x, y0, y1, colour, tint, label=None, dash=None, w=16, side="right"):
    s.rect(x - w / 2, y0, w, y1 - y0, r=4, fill=tint, stroke=colour, sw=1.3, dash=dash)
    if label:
        lines = label.split("\n")
        for i, ln in enumerate(lines):
            if side == "right":
                s.text(x + w / 2 + 6, (y0 + y1) / 2 + 4 - (len(lines) - 1) * 7 + i * 14, ln, size=11, fill=colour)
            else:
                s.text(x - w / 2 - 6, (y0 + y1) / 2 + 4 - (len(lines) - 1) * 7 + i * 14, ln, size=11, fill=colour,
                       anchor="end")


def d2_sequence(out, ex):
    x0 = {"crud": 30, "dual": 350, "scal": 670}[ex]
    s = Svg(360, 590, ox=x0 - 30)
    s.title(EXECUTORS[ex], "Request flow · D2 · sequence diagram")
    top, bot = 140, 548
    s.rect(x0, 112, 300, 456, r=14, fill=PANEL)

    if ex == "crud":
        # CRUD cache
        # the speculative cache Ω* and the confirmed state Δ* are separate lifelines
        a, b, c, d = 70, 160, 228, 296
        for x, n, col, t in ((a, "consensus", CONS, CONS_T), (b, "worker", WORK, WORK_T), (c, "Ω^{*}", SPEC, SPEC_T),
                             (d, "Δ^{*}", CONF, CONF_T)):
            head(s, x, 166, n, col, t)
            lifeline(s, x, 180, bot)
        msg(s, a, b - 8, 220, "batch k", CONS)
        act(s, b, 230, 330, LIVE, LIVE_T, dash="4 3", w=20)
        s.text(b - 14, 284, "container", size=11, fill=LIVE, anchor="end")
        msg(s, c, b + 10, 256, "reads", MUTED, dash="4 3")
        msg(s, d, b + 10, 286, "on a miss", MUTED, dash="4 3")
        msg(s, b + 10, c, 316, "append Ω_{k}", SPEC)
        msg(s, a, b - 8, 400, "confirm 7", CONS)
        act(s, b, 410, 450, CONF, CONF_T)
        msg(s, b + 8, d, 432, "Δ^{*} ← Δ^{*} ⊕ Δ_{7}", CONF)
        s.text(c, 460, "Ω^{*} unchanged", size=10, fill=MUTED, anchor="middle")
        s.arrow([(b, 470), (b, 500), (a - 20, 500)], stroke=INK)
        s.text(a + 30, 494, "replies", size=11, fill=INK, anchor="middle")

    if ex == "dual":
        # dual-state
        a, b, c = 390, 490, 590
        for x, n, col, t in ((a, "consensus", CONS, CONS_T), (b, "preemptive", WORK, WORK_T),
                             (c, "confirmed", WORK, WORK_T)):
            head(s, x, 166, n, col, t)
            lifeline(s, x, 180, bot)
        msg(s, a, b - 8, 220, "batch k", CONS)
        act(s, b, 230, 330, SPEC, SPEC_T, "run on Ω^{*}\nin place")
        msg(s, a, b - 8, 400, "confirm k", CONS)
        act(s, b, 410, 440, SPEC, SPEC_T)
        s.arrow([(b, 450), (b, 500), (a - 20, 500)], stroke=INK)
        s.text(a + 30, 494, "replies", size=11, fill=INK, anchor="middle")
        msg(s, b + 8, c - 8, 432, "batch k", CONF)
        act(s, c, 442, 540, CONF, CONF_T, "run again\non Δ^{*}", side="left")
        s.text(c - 14, 580, "(replies dropped)", size=11, fill=MUTED, anchor="end")

    if ex == "scal":
        # scalable
        a, b, c = 710, 810, 920
        for x, n, col, t in ((a, "consensus", CONS, CONS_T), (b, "worker", WORK, WORK_T), (c, "pool ×4", POOL, POOL_T)):
            head(s, x, 166, n, col, t)
            lifeline(s, x, 180, bot)
        msg(s, a, b - 8, 220, "batch k", CONS)
        act(s, b, 228, 322, WORK, WORK_T, w=12)
        msg(s, b + 6, c - 26, 244, "fork", POOL)
        for i in range(4):
            s.rect(c - 22 + i * 12, 254, 9, 50, r=4, fill=LIVE_T, stroke=LIVE, sw=1.1, dash="3 2")
        msg(s, c - 26, b + 6, 318, "join", LIVE)
        act(s, b, 326, 356, LIVE, LIVE_T, dash="4 3", w=20)
        s.text(b - 16, 345, "re-run", size=11, fill=LIVE, anchor="end")
        s.text(b + 14, 376, "append Ω_{k}", size=11, fill=SPEC)
        msg(s, a, b - 8, 400, "confirm k", CONS)
        act(s, b, 410, 450, CONF, CONF_T, "Δ^{*} ← Δ^{*} ⊕ Δ_{k}")
        s.arrow([(b, 470), (b, 500), (a - 20, 500)], stroke=INK)
        s.text(a + 30, 494, "replies", size=11, fill=INK, anchor="middle")
    s.save(os.path.join(out, f"{PFX}-d2-sequence-{ex}"))


# ---------------------------------------------------------------------------
# D3 — containers between the speculated view and the decision
# ---------------------------------------------------------------------------
def view_plane(s, x, y, w, dual=False):
    if dual:
        s.rect(x, y, w, 56, r=12, fill=SPEC_T, stroke=SPEC, sw=1.6, shadow=True)
        s.text(x + w / 2, y + 34, "Ω^{*}  · a full copy", size=14, weight=600, fill=SPEC, anchor="middle")
        return
    # two separate entities: the speculative cache, then (read on a miss) the confirmed state
    s.rect(x, y, w, 26, r=10, fill=SPEC_T, stroke=SPEC, sw=1.6, shadow=True)
    s.text(x + w / 2, y + 18, "Ω^{*} · speculative cache", size=13, weight=600, fill=SPEC, anchor="middle")
    s.rect(x, y + 34, w, 26, r=10, fill=CONF_T, stroke=CONF, sw=1.6, shadow=True)
    s.text(x + w / 2, y + 52, "Δ^{*} · confirmed state", size=13, weight=600, fill=CONF, anchor="middle")


def d3_planes(out, ex):
    s = Svg(470, 660)
    s.title(EXECUTORS[ex], "Request flow · D3 · containers between what is read\nand what is written")
    rows = [(150, "speculated view" if ex == "dual" else "what is read", "read and written" if ex == "dual" else "Ω^{*}, then Δ^{*}"),
            (300, "containers", "the middleware"), (470, "decision", "being built")]
    for y, t, sub in rows:
        s.text(32, y + 24, t, size=14, weight=600)
        s.text(32, y + 42, sub, size=11.5, fill=MUTED)
    W = 240

    if ex == "crud":
        # CRUD cache: one container, the whole batch, on the worker
        x = 160
        view_plane(s, x, 150, W)
        capsule(s, x + 30, 316, W - 60, 40, "batch k, op by op")
        s.arrow([(x + 90, 214), (x + 90, 314)], stroke=MUTED, dash="4 3")
        thread_badge(s, x + 30, 306, "worker", WORK)
        s.text(x + 84, 270, "reads", size=11, fill=MUTED, anchor="end")
        s.arrow([(x + 150, 358), (x + 150, 466)], stroke=LIVE, sw=2)
        s.text(x + 158, 420, "writes", size=11, fill=LIVE)
        card(s, x + 128, 470, "ω", LIVE, LIVE_T, w=44, h=34, size=15)
        s.arrow([(x + 174, 487), (x + 196, 487)], stroke=SPEC)
        card(s, x + 198, 470, "Ω_{k}", SPEC, SPEC_T, w=44, h=34, size=14)

    if ex == "dual":
        # dual-state: no container; the batch writes straight into the full copy
        x = 160
        view_plane(s, x, 150, W, dual=True)
        s.rect(x + 50, 316, W - 100, 40, r=8, fill=WHITE, stroke=FAINT, sw=1.4, dash="5 4")
        s.text(x + W / 2, 341, "no container", size=12, fill=MUTED, anchor="middle")
        s.arrow([(x + 120, 300), (x + 120, 210)], stroke=SPEC, sw=2, both=True)
        s.text(x + 128, 262, "reads and writes Ω^{*} directly", size=11, fill=SPEC)
        s.rect(x + 40, 470, W - 80, 34, r=8, fill=SPEC_T, stroke=SPEC, sw=1.4)
        s.text(x + W / 2, 492, "log: batch k + replies", size=12, weight=600, fill=SPEC, anchor="middle")
        s.text(x + W / 2, 540, "undo = copy all of Δ^{*}", size=11.5, fill=DROP, anchor="middle")

    if ex == "scal":
        # scalable: a container per op on the pool, then re-runs on the worker
        x = 160
        view_plane(s, x, 150, W)
        s.rect(x - 4, 290, W + 8, 82, r=12, fill=POOL_T)
        for i in range(4):
            cx = x + 8 + i * 58
            capsule(s, cx, 318, 50, 30, f"op {i + 1}", size=11)
            s.arrow([(cx + 18, 214), (cx + 18, 316)], stroke=MUTED, dash="4 3", sw=1.2)
            s.arrow([(cx + 32, 350), (x + 120, 416)], stroke=LIVE, sw=1.2)
        thread_badge(s, x + 4, 302, "exec pool ×4", POOL)
        capsule(s, x + 60, 404, W - 120, 30, "re-run o2, o3", size=11)
        thread_badge(s, x + 60 + W - 120 - 18, 402, "W", WORK)
        s.arrow([(x + 120, 436), (x + 120, 468)], stroke=LIVE, sw=2)
        card(s, x + 98, 470, "ω", LIVE, LIVE_T, w=44, h=34, size=15)
        s.arrow([(x + 144, 487), (x + 166, 487)], stroke=SPEC)
        card(s, x + 168, 470, "Ω_{k}", SPEC, SPEC_T, w=44, h=34, size=14)
        s.text(x + W / 2, 540, "colliders run again, in order", size=11.5, fill=MUTED, anchor="middle")
    if ex == "crud":
        s.text(160 + W / 2, 540, "ω becomes the log entry Ω_{k}", size=11.5, fill=MUTED, anchor="middle")
    s.line(30, 588, 440, 588, stroke=LINE, sw=1)
    if ex == "dual":
        s.text(32, 614, "Without a container the batch changes Ω^{*} itself,", size=12, fill=MUTED)
        s.text(32, 632, "so undoing it means copying Δ^{*} again.", size=12, fill=MUTED)
    else:
        s.text(32, 614, "A container implements the state interface: the", size=12, fill=MUTED)
        s.text(32, 632, "application never touches Δ^{*} or Ω^{*} directly.", size=12, fill=MUTED)
    s.save(os.path.join(out, f"{PFX}-d3-planes-{ex}"))


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "."
    os.makedirs(out, exist_ok=True)
    for f in (d1_subway, d2_sequence, d3_planes):
        for ex in EXECUTORS:
            f(out, ex)
        print(f.__name__)
