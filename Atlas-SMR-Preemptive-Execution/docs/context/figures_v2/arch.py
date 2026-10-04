"""Type 1 — executor architecture: threads, states and the wires between them.

    python3 arch.py <output-dir>
"""
import os
import sys

from svgkit import (EXECUTORS, CONF, CONF_T, CONS, CONS_T, DROP, INK, LINE, LIVE, LIVE_T, MUTED, PANEL, POOL, POOL_T, READP,
                    READP_T, SPEC, SPEC_T, WHITE, WORK, WORK_T, Svg, block, capsule, card, legend, person, stack,
                    thread_badge)

PFX = "arch"


def cache_group(s, x, y, w, h, title="speculative cache", bottom=False):
    """The speculative cache (the log and Ω*), drawn apart from the confirmed state Δ*."""
    s.rect(x, y, w, h, r=10, fill=WHITE, stroke=SPEC, sw=1.3, dash="5 4", opacity=0.6)
    s.rect(x, y, w, h, r=10, fill="none", stroke=SPEC, sw=1.3, dash="5 4")
    s.text(x + 8, y + h - 8 if bottom else y + 14, title, size=10.5, weight=600, fill=SPEC)


def conf_group(s, x, y, w, h, title="confirmed state"):
    s.rect(x, y, w, h, r=12, fill=WHITE, stroke=CONF, sw=1.5)
    s.text(x + 10, y + 16, title, size=10.5, weight=600, fill=CONF)


def lane(s, x, y, w, h, name, colour=WORK, tint=WORK_T):
    s.rect(x, y, w, h, r=14, fill=tint)
    thread_badge(s, x + 10, y + 16, name, colour)


def consensus(s, x, y):
    s.rect(x, y, 108, 58, r=12, fill=CONS_T, stroke=CONS, sw=1.4)
    s.text(x + 54, y + 27, "consensus", size=13.5, weight=600, fill=CONS, anchor="middle")
    s.text(x + 54, y + 44, "orders batches", size=10.5, fill=MUTED, anchor="middle")


def clients(s, x, y):
    for i in range(3):
        person(s, x + i * 20, y, colour="#8b93a3", scale=0.9)
    s.text(x + 20, y + 26, "clients", size=11.5, fill=MUTED, anchor="middle")


def panel(s, y0, title, sub):
    s.rect(170, y0, 732, 190, r=18, fill=PANEL)
    s.text(190, y0 + 26, sub, size=13, weight=600, fill=MUTED)


def a1_blocks(out, ex):
    y0 = {"crud": 100, "dual": 310, "scal": 520}[ex]
    s = Svg(1000, 330, oy=y0 - 100)
    s.title(EXECUTORS[ex], "Executor architecture · A1 · blocks and wires")

    if ex == "crud":
        # --- CRUD cache ----------------------------------------------------------
        y = 100
        panel(s, y, "CRUD cache", "one worker thread")
        consensus(s, 30, y + 66)
        lane(s, 190, y + 40, 456, 136, "worker")
        capsule(s, 206, y + 116, 104, 36, "batch k")
        s.text(258, y + 168, "runs in a container", size=10.5, fill=MUTED, anchor="middle")
        cache_group(s, 330, y + 62, 304, 106)
        # the batch writes Ω* directly; the container records its writes, which join the log as Ω_k
        block(s, 342, y + 86, 96, 64, SPEC, SPEC_T, "Ω^{*}", size=16, shadow=False)
        stack(s, 478, y + 96, ["Ω_{9}", "Ω_{8}", "Ω_{7}"], SPEC, SPEC_T, w=36, gap=5)   # newest next to Ω*
        s.text(537, y + 140, "log", size=10.5, fill=MUTED, anchor="middle")
        s.arrow([(310, y + 124), (340, y + 124)], stroke=LIVE)
        s.arrow([(310, y + 144), (326, y + 144), (326, y + 160), (496, y + 160), (496, y + 126)], stroke=SPEC)
        s.label(420, y + 160, "recorded writes", size=9.5, fill=SPEC, bg=WHITE)
        s.arrow([(476, y + 104), (440, y + 104)], stroke=SPEC, dash="4 3")
        s.label(458, y + 94, "rebuild", size=9.5, fill=SPEC, bg=WHITE)
        conf_group(s, 662, y + 48, 120, 120)
        block(s, 678, y + 74, 88, 82, CONF, CONF_T, "Δ^{*}", size=17, shadow=False)
        s.text(252, y + 82, "on Ω^{*} (a miss reads Δ^{*})", size=10.5, fill=MUTED, anchor="middle")
        s.arrow([(138, y + 98), (204, y + 134)], stroke=CONS)
        s.label(170, y + 104, "batch k", size=10.5, bg=PANEL)
        # commit: the oldest Ω (Ω7) goes into Δ*; its replies go to the clients
        s.arrow([(84, y + 124), (84, y + 182), (578, y + 182), (578, y + 126)], stroke=CONS)
        s.label(130, y + 186, "confirm 7", size=10.5, bg=PANEL)
        s.arrow([(578, y + 182), (722, y + 182), (722, y + 158)], stroke=CONF, sw=2)
        s.label(652, y + 186, "commit: Ω → Δ", size=10.5, fill=CONF, bg=PANEL)
        s.arrow([(722, y + 182), (900, y + 182), (918, y + 142)], stroke=INK)
        s.label(830, y + 186, "replies", size=10.5, bg=PANEL)
        clients(s, 922, y + 120)

    if ex == "dual":
        # --- dual-state -----------------------------------------------------------
        y = 310
        panel(s, y, "Dual-state", "three threads, two full copies")
        consensus(s, 30, y + 66)
        lane(s, 190, y + 40, 96, 136, "main")
        s.text(238, y + 104, "routes", size=11.5, fill=MUTED, anchor="middle")
        s.text(238, y + 120, "requests", size=11.5, fill=MUTED, anchor="middle")
        lane(s, 300, y + 40, 260, 136, "preemptive")
        stack(s, 316, y + 66, ["Ω_{7}", "Ω_{8}", "Ω_{9}"], SPEC, SPEC_T, w=36, gap=5)
        s.text(370, y + 108, "log of batches", size=10.5, fill=MUTED, anchor="middle")
        block(s, 450, y + 58, 96, 104, SPEC, SPEC_T, "Ω^{*}", size=17, sub="full copy")
        lane(s, 574, y + 40, 196, 136, "confirmed")
        block(s, 640, y + 58, 104, 104, CONF, CONF_T, "Δ^{*}", size=17, sub="full copy")
        s.arrow([(138, y + 95), (188, y + 95)], stroke=CONS)
        s.arrow([(286, y + 132), (448, y + 132)], stroke=SPEC)
        s.label(368, y + 136, "run batch k in place", size=10.5, bg=WORK_T)
        s.arrow([(416, y + 66), (416, y + 33), (720, y + 33), (720, y + 56)], stroke=CONF, sw=2)
        s.label(600, y + 37, "on confirm: run k again", size=10.5, fill=CONF, bg=PANEL)
        s.arrow([(640, y + 150), (548, y + 150)], stroke=DROP, dash="4 3")
        s.label(594, y + 166, "copy Δ^{*} back", size=10.5, fill=DROP, bg=PANEL)
        s.arrow([(430, y + 176), (430, y + 184), (900, y + 184), (918, y + 142)], stroke=INK)
        s.label(800, y + 188, "replies", size=10.5, bg=PANEL)
        clients(s, 922, y + 120)

    if ex == "scal":
        # --- scalable --------------------------------------------------------------
        y = 520
        panel(s, y, "Scalable CRUD", "worker + a pool of 4")
        consensus(s, 30, y + 66)
        s.rect(190, y + 40, 136, 136, r=14, fill=POOL_T)
        thread_badge(s, 200, y + 56, "exec pool ×4", POOL)
        for i in range(4):
            capsule(s, 212, y + 72 + i * 25, 92, 20, f"op {i + 1}", size=10.5)
        lane(s, 340, y + 40, 306, 136, "worker")
        cache_group(s, 350, y + 62, 286, 106)
        # the join merges and re-runs straight on Ω*; its recorded writes join the log as Ω_k
        block(s, 362, y + 86, 92, 64, SPEC, SPEC_T, "Ω^{*}", size=16, shadow=False)
        stack(s, 492, y + 96, ["Ω_{9}", "Ω_{8}", "Ω_{7}"], SPEC, SPEC_T, w=36, gap=5)   # newest next to Ω*
        s.text(551, y + 140, "log", size=10.5, fill=MUTED, anchor="middle")
        s.arrow([(490, y + 104), (456, y + 104)], stroke=SPEC, dash="4 3")
        s.label(473, y + 94, "rebuild", size=9.5, fill=SPEC, bg=WHITE)
        conf_group(s, 662, y + 48, 120, 120)
        block(s, 678, y + 74, 88, 82, CONF, CONF_T, "Δ^{*}", size=17, shadow=False)
        s.arrow([(138, y + 95), (188, y + 95)], stroke=CONS)
        s.label(162, y + 86, "fork", size=10.5, bg=PANEL)
        s.arrow([(326, y + 118), (360, y + 118)], stroke=LIVE)
        s.label(338, y + 106, "join", size=10.5, bg=PANEL)
        s.arrow([(326, y + 144), (344, y + 144), (344, y + 160), (510, y + 160), (510, y + 126)], stroke=SPEC)
        s.label(430, y + 160, "recorded writes", size=9.5, fill=SPEC, bg=WHITE)
        s.text(560, y + 30, "every op reads Ω^{*}, then Δ^{*}", size=10.5, fill=MUTED)
        s.arrow([(84, y + 124), (84, y + 182), (592, y + 182), (592, y + 126)], stroke=CONS)
        s.label(130, y + 186, "confirm 7", size=10.5, bg=PANEL)
        s.arrow([(592, y + 182), (722, y + 182), (722, y + 158)], stroke=CONF, sw=2)
        s.label(658, y + 186, "commit: Ω → Δ", size=10.5, fill=CONF, bg=PANEL)
        s.arrow([(722, y + 182), (900, y + 182), (918, y + 142)], stroke=INK)
        s.label(830, y + 186, "replies", size=10.5, bg=PANEL)
        clients(s, 922, y + 120)

    items = [("block", CONF, CONF_T, "confirmed"), ("block", SPEC, SPEC_T, "speculated")]
    if ex != "dual":
        items.append(("capsule", LIVE, LIVE_T, "container"))
    items.append(("badge", WORK, None, "thread"))
    if ex == "scal":
        items.append(("badge", POOL, None, "parallel threads"))
    legend(s, 32, y0 + 216, items)
    s.save(os.path.join(out, f"{PFX}-a1-blocks-{ex}"))


def room(s, x, y, w, h, name, colour=WORK, tint=WORK_T):
    s.rect(x, y, w, h, r=16, fill=tint)
    thread_badge(s, x + 12, y + 18, name, colour)


def a2_rooms(out, ex):
    """Each thread is a room; the state drawn inside a room is owned by that thread."""
    x0 = {"crud": 30, "dual": 350, "scal": 670}[ex]
    s = Svg(340, 700, ox=x0 - 10)
    s.title(EXECUTORS[ex], "Architecture · A2 · threads as rooms")
    s.rect(x0 + 10, 96, 280, 36, r=18, fill=CONS_T, stroke=CONS, sw=1.3)
    s.text(x0 + 150, 119, "consensus", size=13, weight=600, fill=CONS, anchor="middle")
    s.rect(x0 + 10, 600, 280, 36, r=18, fill="#eef0f3")
    for i in range(5):
        person(s, x0 + 90 + i * 30, 622, colour="#9aa1af", scale=0.65)
    sub = {"crud": "1 thread", "dual": "3 threads", "scal": "worker + exec pool"}[ex]
    s.text(x0 + 150, 172, sub, size=12, fill=MUTED, anchor="middle")

    if ex == "crud":
        # CRUD cache: the worker holds the speculative cache; Δ* is drawn apart
        x = 30
        room(s, x + 10, 198, 280, 250, "worker")
        capsule(s, x + 34, 232, 116, 34, "batch k")
        cache_group(s, x + 22, 284, 256, 150, bottom=True)
        block(s, x + 34, 306, 96, 80, SPEC, SPEC_T, "Ω^{*}", size=16, shadow=False)
        stack(s, x + 148, 318, ["Ω_{9}", "Ω_{8}", "Ω_{7}"], SPEC, SPEC_T, w=34, gap=5)   # newest next to Ω*
        s.text(x + 205, 306, "log", size=10.5, fill=MUTED, anchor="middle")
        s.arrow([(x + 82, 266), (x + 82, 304)], stroke=LIVE)                 # the batch writes Ω* directly
        s.arrow([(x + 150, 249), (x + 167, 249), (x + 167, 316)], stroke=SPEC)  # and its writes are logged
        s.label(x + 206, 249, "recorded writes", size=9.5, fill=SPEC, bg=WORK_T)
        s.arrow([(x + 166, 348), (x + 166, 372), (x + 132, 372)], stroke=SPEC, dash="4 3")
        s.label(x + 190, 386, "rebuild", size=9.5, fill=SPEC, bg=WHITE)
        conf_group(s, x + 10, 462, 280, 120)
        block(s, x + 100, 488, 100, 80, CONF, CONF_T, "Δ^{*}", size=17, shadow=False)
        s.arrow([(x + 243, 348), (x + 243, 454), (x + 180, 454), (x + 180, 486)], stroke=CONF, sw=2)
        s.arrow([(x + 40, 132), (x + 40, 196)], stroke=CONS)
        s.arrow([(x + 280, 330), (x + 300, 330), (x + 300, 598)], stroke=INK)
        s.label(x + 300, 574, "replies", size=10.5, bg=WHITE)

    if ex == "dual":
        # dual-state: three rooms, two full copies
        x = 350
        room(s, x + 10, 198, 280, 56, "main")
        s.text(x + 200, 231, "routes", size=11.5, fill=MUTED, anchor="middle")
        room(s, x + 10, 272, 280, 150, "preemptive")
        stack(s, x + 26, 318, ["Ω_{7}", "Ω_{8}", "Ω_{9}"], SPEC, SPEC_T, w=34, gap=5)
        s.text(x + 82, 364, "log of batches", size=10.5, fill=MUTED, anchor="middle")
        block(s, x + 176, 300, 96, 108, SPEC, SPEC_T, "Ω^{*}", size=17)
        room(s, x + 10, 440, 280, 128, "confirmed")
        block(s, x + 176, 452, 96, 108, CONF, CONF_T, "Δ^{*}", size=17)
        s.arrow([(x + 40, 132), (x + 40, 196)], stroke=CONS)
        s.arrow([(x + 40, 254), (x + 40, 270)], stroke=CONS)
        s.arrow([(x + 122, 350), (x + 122, 506), (x + 174, 506)], stroke=CONF, sw=2)
        s.text(x + 114, 530, "run k again", size=10.5, fill=CONF, anchor="end")
        s.arrow([(x + 210, 452), (x + 210, 410)], stroke=DROP, dash="4 3")
        s.label(x + 150, 433, "copy of Δ^{*}", size=10.5, fill=DROP, bg=WHITE)
        s.arrow([(x + 290, 346), (x + 304, 346), (x + 304, 598)], stroke=INK)
        s.label(x + 304, 574, "replies", size=10.5, bg=WHITE)

    if ex == "scal":
        # scalable: the worker holds the speculative cache; Δ* is drawn apart; the pool borrows both
        x = 670
        room(s, x + 10, 198, 280, 206, "worker")
        cache_group(s, x + 20, 228, 260, 164)
        block(s, x + 60, 256, 84, 96, SPEC, SPEC_T, "Ω^{*}", size=16, shadow=False)
        stack(s, x + 162, 270, ["Ω_{9}", "Ω_{8}", "Ω_{7}"], SPEC, SPEC_T, w=32, gap=4)   # newest next to Ω*
        s.text(x + 216, 318, "log", size=10.5, fill=MUTED, anchor="middle")
        s.arrow([(x + 178, 300), (x + 178, 330), (x + 146, 330)], stroke=SPEC, dash="4 3")
        s.label(x + 200, 344, "rebuild", size=9.5, fill=SPEC, bg=WHITE)
        s.arrow([(x + 250, 300), (x + 250, 420)], stroke=CONF, sw=2)       # commit Ω7 → Δ*
        conf_group(s, x + 124, 412, 166, 68)
        block(s, x + 222, 422, 60, 50, CONF, CONF_T, "Δ^{*}", size=15, shadow=False)
        room(s, x + 10, 490, 280, 100, "exec pool ×4", POOL, POOL_T)
        for i in range(4):
            capsule(s, x + 24 + i * 66, 528, 56, 26, f"op {i + 1}", size=10.5)
        s.text(x + 150, 576, "one container per op", size=10.5, fill=MUTED, anchor="middle")
        s.arrow([(x + 30, 404), (x + 30, 526)], stroke=POOL)
        s.label(x + 30, 440, "fork", size=10.5, fill=POOL, bg=WHITE)
        s.arrow([(x + 48, 526), (x + 48, 300), (x + 58, 300)], stroke=LIVE)    # the join writes Ω* directly
        s.arrow([(x + 48, 300), (x + 48, 249), (x + 178, 249), (x + 178, 268)], stroke=SPEC)  # and is logged
        s.label(x + 140, 249, "recorded", size=9.5, fill=SPEC, bg=WHITE)
        s.label(x + 48, 466, "join", size=10.5, fill=LIVE, bg=WHITE)
        s.arrow([(x + 252, 526), (x + 252, 482)], stroke=MUTED, dash="4 3")
        s.arrow([(x + 102, 526), (x + 102, 354)], stroke=MUTED, dash="4 3")
        s.label(x + 102, 466, "&Ω^{*}", size=11, fill=MUTED, bg=WHITE)
        s.label(x + 252, 504, "&Δ^{*}", size=11, fill=MUTED, bg=WHITE)
        s.arrow([(x + 40, 132), (x + 40, 196)], stroke=CONS)
        s.arrow([(x + 290, 280), (x + 300, 280), (x + 300, 598)], stroke=INK)
        s.label(x + 300, 574, "replies", size=10.5, bg=WHITE)
    if ex == "scal":
        s.text(x0 + 150, 666, "&Ω^{*}, &Δ^{*} = read-only access to the", size=11.5, fill=MUTED, anchor="middle")
        s.text(x0 + 150, 681, "worker's state", size=11.5, fill=MUTED, anchor="middle")
    s.save(os.path.join(out, f"{PFX}-a2-rooms-{ex}"))


def mini_layered(s, x, y, k=1.0):
    """Ω* (the speculative cache) and Δ* (the confirmed state), side by side: separate entities."""
    block(s, x - 8 * k, y + 22 * k, 30 * k, 26 * k, SPEC, SPEC_T, "Ω^{*}", size=11 * k, shadow=False)
    block(s, x + 30 * k, y + 10 * k, 52 * k, 50 * k, CONF, CONF_T, "Δ^{*}", size=13 * k, shadow=False)


def gear(s, cx, cy, r, colour):
    """A circular 're-run' mark."""
    s.path(f"M{cx + r:.1f},{cy:.1f} A{r},{r} 0 1 1 {cx:.1f},{cy - r:.1f}", stroke=colour, sw=2.2, arrow=colour)


def a3_matrix(out, ex):
    """A pictogram profile: one row per aspect, a picture and a short caption in each."""
    s = Svg(560, 750)
    s.title(EXECUTORS[ex], "Executor architecture · A3 · at a glance")
    X = 330                                     # centre of the picture column
    rows = [("threads", 112), ("memory", 212), ("speculate", 336), ("commit", 482), ("backtrack", 604)]
    for name, y in rows:
        s.line(30, y, 530, y, stroke=LINE, sw=1)
        s.text(40, y + 54, name, size=15, weight=600, fill=INK)
    s.line(30, 728, 530, 728, stroke=LINE, sw=1)

    def cap(y, t):
        s.text(X, y, t, size=12, fill=MUTED, anchor="middle")

    # threads
    y = 160
    badges = {"crud": [("worker", WORK)],
              "dual": [("main", WORK), ("preemptive", WORK), ("confirmed", WORK)],
              "scal": [("worker", WORK), ("exec ×4", POOL)]}[ex]
    from svgkit import text_width
    total = sum(text_width(n, 10.5, 600) + 12 for n, _ in badges) + 6 * (len(badges) - 1)
    x = X - total / 2
    for n, col in badges:
        x += thread_badge(s, x, y, n, col) + 6
    cap(y + 32, {"crud": "one thread does all the work", "dual": "two workers, each with its own state",
                 "scal": "one worker, plus a pool for the ops"}[ex])

    # memory, drawn to a rough scale: S is the big square
    y = 228
    if ex == "dual":
        block(s, X - 70, y, 64, 64, CONF, CONF_T, "Δ^{*}", size=14, shadow=False)
        block(s, X + 6, y, 64, 64, SPEC, SPEC_T, "Ω^{*}", size=14, shadow=False)
        cap(y + 86, "two full copies")
    else:
        block(s, X - 50, y, 64, 64, CONF, CONF_T, "Δ^{*}", size=14, shadow=False)
        block(s, X + 22, y + 34, 24, 24, SPEC, SPEC_T, "Ω^{*}", size=11, shadow=False)
        for i in range(3):
            s.rect(X + 52 + i * 9, y + 44, 7, 14, r=2, fill=SPEC_T, stroke=SPEC, sw=1)
        if ex == "scal":
            for i in range(4):
                s.circle(X + 24 + i * 9, y + 20, 3.2, fill=LIVE)
        cap(y + 86, "Δ^{*} + a small cache + log" + (", + a tiny cache per op" if ex == "scal" else ""))

    # speculate
    y = 352
    if ex == "crud":
        capsule(s, X - 46, y, 92, 26, "batch", size=11)
        s.arrow([(X, y + 26), (X, y + 40)], stroke=MUTED, dash="3 3")
        mini_layered(s, X - 34, y + 42, 0.8)
        cap(y + 118, "in a container that reads Ω^{*}, then Δ^{*}")
    elif ex == "dual":
        card(s, X - 80, y + 28, "k", SPEC, SPEC_T, w=30, h=26, shadow=False)
        s.arrow([(X - 46, y + 41), (X - 10, y + 41)], stroke=SPEC)
        block(s, X - 6, y + 8, 58, 64, SPEC, SPEC_T, "Ω^{*}", size=13, shadow=False)
        cap(y + 118, "directly on the copy, in place")
    else:
        for i in range(4):
            capsule(s, X - 92 + i * 34, y + 4, 30, 20, None)
            s.arrow([(X - 77 + i * 34, y + 24), (X - 6, y + 52)], stroke=LIVE, sw=1.1)
        s.rect(X - 92, y - 2, 134, 32, r=8, fill="none", stroke=POOL, sw=1.2, dash="2 3")
        block(s, X - 30, y + 52, 48, 44, SPEC, SPEC_T, "Ω^{*}", size=12, shadow=False)
        s.text(X + 50, y + 18, "parallel", size=11, fill=POOL)
        cap(y + 118, "a container per op, in parallel, then merged into Ω^{*}")

    # commit
    y = 506
    if ex == "dual":
        card(s, X - 76, y + 18, "k", SPEC, SPEC_T, w=30, h=26, shadow=False)
        gear(s, X - 24, y + 31, 11, CONF)
        block(s, X + 4, y, 60, 60, CONF, CONF_T, "Δ^{*}", size=14, shadow=False)
        cap(y + 86, "run the batch again on Δ^{*}")
    else:
        card(s, X - 66, y + 18, "Ω", SPEC, SPEC_T, w=30, h=26, shadow=False)
        s.arrow([(X - 32, y + 31), (X - 4, y + 31)], stroke=CONF, sw=2)
        block(s, X, y, 60, 60, CONF, CONF_T, "Δ^{*}", size=14, shadow=False)
        cap(y + 86, "apply the stored writes")

    # backtrack
    y = 630
    if ex == "dual":
        block(s, X - 76, y, 50, 46, CONF, CONF_T, "Δ^{*}", size=13, shadow=False)
        s.arrow([(X - 22, y + 23), (X + 18, y + 23)], stroke=DROP, dash="4 3")
        block(s, X + 22, y, 50, 46, SPEC, SPEC_T, "Ω^{*}", size=13, shadow=False)
        cap(y + 66, "copy Δ^{*}, then replay the rest")
    else:
        x0 = X - 66
        for i, op in enumerate((1, 1, 0.35, 0.35)):
            card(s, x0 + i * 34, y + 8, "Ω", SPEC, SPEC_T, w=30, h=26, shadow=False, opacity=op)
        s.line(x0 + 66, y + 4, x0 + 134, y + 38, stroke=DROP, sw=2)
        cap(y + 66, "drop the tail, rebuild Ω^{*}")
    s.save(os.path.join(out, f"{PFX}-a3-matrix-{ex}"))


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "."
    os.makedirs(out, exist_ok=True)
    for f in (a1_blocks, a2_rooms, a3_matrix):
        for ex in EXECUTORS:
            f(out, ex)
        print(f.__name__)
