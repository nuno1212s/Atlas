"""Type 1 — executor architecture: threads, states and the wires between them.

    python3 arch.py <output-dir>
"""
import os
import sys

from svgkit import (CONF, CONF_T, CONS, CONS_T, DROP, INK, LINE, LIVE, LIVE_T, MUTED, PANEL, POOL, POOL_T, READP,
                    READP_T, SPEC, SPEC_T, WHITE, WORK, WORK_T, Svg, block, capsule, card, legend, person, stack,
                    thread_badge)

PFX = "arch"


def layered(s, x, y, w=104, h=96):
    """C over S: the small speculated cache drawn in front of the confirmed state."""
    block(s, x + 18, y + 18, w, h, CONF, CONF_T, "S", size=17)
    block(s, x, y, 46, 40, SPEC, SPEC_T, "C", size=15)
    return (x, y, w + 18, h + 18)


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
    s.text(190, y0 + 26, title, size=15, weight=600)
    s.text(190 + len(title) * 9.0 + 10, y0 + 26, sub, size=12, fill=MUTED)


def a1_blocks(out):
    s = Svg(1000, 790)
    s.title("Executor architecture", "A1 · blocks and wires: one row per executor")

    # --- CRUD cache ----------------------------------------------------------
    y = 100
    panel(s, y, "CRUD cache", "one worker thread")
    consensus(s, 30, y + 66)
    lane(s, 190, y + 40, 520, 136, "worker")
    capsule(s, 210, y + 116, 104, 36, "batch k")
    s.text(262, y + 168, "runs in a container", size=10.5, fill=MUTED, anchor="middle")
    stack(s, 348, y + 120, ["Ω_{7}", "Ω_{8}", "Ω_{9}"], SPEC, SPEC_T, w=36, gap=5)
    s.text(402, y + 110, "log", size=10.5, fill=MUTED, anchor="middle")
    layered(s, 548, y + 50, w=110, h=96)
    s.arrow([(314, y + 134), (346, y + 134)], stroke=LIVE)
    s.arrow([(458, y + 126), (540, y + 82)], stroke=SPEC, curve=-0.25)
    s.label(497, y + 90, "fold", size=10.5, bg=WORK_T)
    s.arrow([(548, y + 82), (262, y + 82), (262, y + 114)], stroke=MUTED, dash="4 3")
    s.label(420, y + 86, "reads C, then S", size=10.5, bg=WORK_T)
    s.arrow([(138, y + 98), (208, y + 134)], stroke=CONS)
    s.label(176, y + 104, "batch k", size=10.5, bg=PANEL)
    # commit: the oldest Ω goes into S; its replies go to the clients
    s.arrow([(84, y + 124), (84, y + 182), (366, y + 182), (366, y + 150)], stroke=CONS)
    s.label(130, y + 186, "confirm k", size=10.5, bg=PANEL)
    s.arrow([(366, y + 182), (640, y + 182), (640, y + 166)], stroke=CONF, sw=2)
    s.label(560, y + 186, "commit: Ω → Δ", size=10.5, fill=CONF, bg=PANEL)
    s.arrow([(640, y + 182), (900, y + 182), (918, y + 142)], stroke=INK)
    s.label(800, y + 186, "replies", size=10.5, bg=PANEL)
    s.rect(730, y + 54, 150, 40, r=20, fill=READP_T, stroke=READP, sw=1.2)
    s.text(805, y + 79, "read pool ×4", size=12, weight=600, fill=READP, anchor="middle")
    s.arrow([(730, y + 86), (678, y + 104)], stroke=READP, dash="4 3")
    clients(s, 922, y + 120)

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
    block(s, 450, y + 58, 96, 104, SPEC, SPEC_T, "S_{p}", size=17, sub="full copy")
    lane(s, 574, y + 40, 196, 136, "confirmed")
    block(s, 640, y + 58, 104, 104, CONF, CONF_T, "S", size=17, sub="full copy")
    s.arrow([(138, y + 95), (188, y + 95)], stroke=CONS)
    s.arrow([(286, y + 132), (448, y + 132)], stroke=SPEC)
    s.label(368, y + 136, "run batch k in place", size=10.5, bg=WORK_T)
    s.arrow([(334, y + 66), (334, y + 33), (720, y + 33), (720, y + 56)], stroke=CONF, sw=2)
    s.label(600, y + 37, "on confirm: run k again", size=10.5, fill=CONF, bg=PANEL)
    s.arrow([(640, y + 150), (548, y + 150)], stroke=DROP, dash="4 3")
    s.label(594, y + 166, "copy S back", size=10.5, fill=DROP, bg=PANEL)
    s.arrow([(430, y + 176), (430, y + 184), (900, y + 184), (918, y + 142)], stroke=INK)
    s.label(800, y + 188, "replies", size=10.5, bg=PANEL)
    s.rect(784, y + 54, 104, 40, r=20, fill=READP_T, stroke=READP, sw=1.2)
    s.text(836, y + 79, "read pool", size=12, weight=600, fill=READP, anchor="middle")
    s.arrow([(800, y + 94), (746, y + 116)], stroke=READP, dash="4 3")
    clients(s, 922, y + 120)

    # --- scalable --------------------------------------------------------------
    y = 520
    panel(s, y, "Scalable CRUD", "worker + a pool of 4")
    consensus(s, 30, y + 66)
    s.rect(190, y + 40, 136, 136, r=14, fill=POOL_T)
    thread_badge(s, 200, y + 56, "exec pool ×4", POOL)
    for i in range(4):
        capsule(s, 212, y + 72 + i * 25, 92, 20, f"op {i + 1}", size=10.5)
    lane(s, 340, y + 40, 370, 136, "worker")
    card(s, 356, y + 116, "ω", "#7a5cc2", LIVE_T, w=36)
    s.text(374, y + 108, "merge", size=10.5, fill=MUTED, anchor="middle")
    stack(s, 410, y + 116, ["Ω_{7}", "Ω_{8}", "Ω_{9}"], SPEC, SPEC_T, w=36, gap=5)
    layered(s, 562, y + 50, w=100, h=96)
    s.arrow([(138, y + 95), (188, y + 95)], stroke=CONS)
    s.label(162, y + 86, "fork", size=10.5, bg=PANEL)
    s.arrow([(326, y + 130), (354, y + 130)], stroke=LIVE)
    s.label(340, y + 152, "join", size=10.5, bg=PANEL)
    s.arrow([(392, y + 130), (408, y + 130)], stroke=LIVE)
    s.arrow([(520, y + 122), (556, y + 84)], stroke=SPEC, curve=-0.25)
    s.arrow([(562, y + 82), (330, y + 82)], stroke=MUTED, dash="4 3")
    s.label(452, y + 86, "every op reads C, then S", size=10.5, bg=WORK_T)
    s.arrow([(84, y + 124), (84, y + 182), (428, y + 182), (428, y + 146)], stroke=CONS)
    s.label(130, y + 186, "confirm k", size=10.5, bg=PANEL)
    s.arrow([(428, y + 182), (650, y + 182), (650, y + 166)], stroke=CONF, sw=2)
    s.label(560, y + 186, "commit: Ω → Δ", size=10.5, fill=CONF, bg=PANEL)
    s.arrow([(650, y + 182), (900, y + 182), (918, y + 142)], stroke=INK)
    s.label(800, y + 186, "replies", size=10.5, bg=PANEL)
    s.rect(730, y + 54, 150, 40, r=20, fill=READP_T, stroke=READP, sw=1.2)
    s.text(805, y + 79, "read pool ×4", size=12, weight=600, fill=READP, anchor="middle")
    s.arrow([(730, y + 86), (680, y + 104)], stroke=READP, dash="4 3")
    clients(s, 922, y + 120)

    legend(s, 32, 752, [("block", CONF, CONF_T, "confirmed"), ("block", SPEC, SPEC_T, "speculated"),
                        ("capsule", LIVE, LIVE_T, "container"), ("badge", WORK, None, "thread"),
                        ("badge", POOL, None, "parallel threads")])
    s.save(os.path.join(out, f"{PFX}-a1-blocks"))


def room(s, x, y, w, h, name, colour=WORK, tint=WORK_T):
    s.rect(x, y, w, h, r=16, fill=tint)
    thread_badge(s, x + 12, y + 18, name, colour)


def a2_rooms(out):
    """Each thread is a room; the state drawn inside a room is owned by that thread."""
    s = Svg(1000, 760)
    s.title("Executor architecture", "A2 · threads as rooms: a thread owns what is inside its room")
    s.rect(30, 96, 940, 36, r=18, fill=CONS_T, stroke=CONS, sw=1.3)
    s.text(500, 119, "consensus · orders batches, then confirms them", size=13, weight=600, fill=CONS, anchor="middle")
    s.rect(30, 690, 940, 36, r=18, fill="#eef0f3")
    for i in range(9):
        person(s, 380 + i * 30, 712, colour="#9aa1af", scale=0.65)
    cols = [(30, "CRUD cache", "1 thread"), (350, "Dual-state", "3 threads"), (670, "Scalable CRUD", "1 thread + pool")]
    for x, name, sub in cols:
        s.text(x + 150, 164, name, size=15, weight=600, anchor="middle")
        s.text(x + 150, 182, sub, size=11.5, fill=MUTED, anchor="middle")

    # CRUD cache: everything lives in the worker
    x = 30
    room(s, x + 10, 198, 280, 320, "worker")
    capsule(s, x + 34, 232, 116, 34, "batch k")
    stack(s, x + 34, 300, ["Ω_{7}", "Ω_{8}", "Ω_{9}"], SPEC, SPEC_T, w=34, gap=5)
    s.text(x + 90, 346, "log", size=10.5, fill=MUTED, anchor="middle")
    layered(s, x + 160, 380, w=96, h=90)
    s.arrow([(x + 92, 266), (x + 92, 298)], stroke=LIVE)
    room(s, x + 10, 540, 280, 100, "read pool ×4", READP, READP_T)
    for i in range(4):
        s.circle(x + 50 + i * 26, 600, 8, fill=READP, opacity=0.75)
    s.arrow([(x + 220, 560), (x + 220, 500)], stroke=READP, dash="4 3")
    s.label(x + 240, 532, "&S", size=11, fill=READP, bg=WHITE)
    s.arrow([(x + 40, 132), (x + 40, 196)], stroke=CONS)
    s.arrow([(x + 280, 330), (x + 300, 330), (x + 300, 688)], stroke=INK)
    s.label(x + 300, 664, "replies", size=10.5, bg=WHITE)

    # dual-state: three rooms, two full copies
    x = 350
    room(s, x + 10, 198, 280, 56, "main")
    s.text(x + 200, 231, "routes", size=11.5, fill=MUTED, anchor="middle")
    room(s, x + 10, 272, 280, 150, "preemptive")
    stack(s, x + 26, 318, ["Ω_{7}", "Ω_{8}", "Ω_{9}"], SPEC, SPEC_T, w=34, gap=5)
    s.text(x + 82, 364, "log of batches", size=10.5, fill=MUTED, anchor="middle")
    block(s, x + 176, 300, 96, 108, SPEC, SPEC_T, "S_{p}", size=17)
    room(s, x + 10, 440, 280, 128, "confirmed")
    block(s, x + 176, 452, 96, 108, CONF, CONF_T, "S", size=17)
    room(s, x + 10, 586, 280, 70, "read pool", READP, READP_T)
    for i in range(4):
        s.circle(x + 40 + i * 22, 632, 7, fill=READP, opacity=0.75)
    s.arrow([(x + 40, 132), (x + 40, 196)], stroke=CONS)
    s.arrow([(x + 40, 254), (x + 40, 270)], stroke=CONS)
    s.arrow([(x + 122, 350), (x + 122, 506), (x + 174, 506)], stroke=CONF, sw=2)
    s.text(x + 114, 530, "run k again", size=10.5, fill=CONF, anchor="end")
    s.arrow([(x + 210, 452), (x + 210, 410)], stroke=DROP, dash="4 3")
    s.label(x + 150, 433, "copy of S", size=10.5, fill=DROP, bg=WHITE)
    s.arrow([(x + 250, 610), (x + 250, 562)], stroke=READP, dash="4 3")
    s.label(x + 270, 600, "&S", size=11, fill=READP, bg=READP_T)
    s.arrow([(x + 290, 346), (x + 304, 346), (x + 304, 688)], stroke=INK)
    s.label(x + 304, 664, "replies", size=10.5, bg=WHITE)

    # scalable: the worker owns the state; the pool borrows it
    x = 670
    room(s, x + 10, 198, 280, 250, "worker")
    card(s, x + 28, 244, "ω", LIVE, LIVE_T, w=34)
    s.text(x + 45, 236, "merge", size=10.5, fill=MUTED, anchor="middle")
    stack(s, x + 76, 244, ["Ω_{7}", "Ω_{8}", "Ω_{9}"], SPEC, SPEC_T, w=34, gap=5)
    layered(s, x + 160, 300, w=96, h=90)
    s.arrow([(x + 62, 258), (x + 74, 258)], stroke=LIVE)
    room(s, x + 10, 466, 280, 104, "exec pool ×4", POOL, POOL_T)
    for i in range(4):
        capsule(s, x + 24 + i * 66, 506, 56, 26, f"op {i + 1}", size=10.5)
    s.text(x + 150, 556, "one container per op", size=10.5, fill=MUTED, anchor="middle")
    room(s, x + 10, 586, 280, 70, "read pool ×4", READP, READP_T)
    for i in range(4):
        s.circle(x + 50 + i * 22, 632, 7, fill=READP, opacity=0.75)
    s.arrow([(x + 40, 278), (x + 40, 504)], stroke=POOL)
    s.label(x + 40, 330, "fork", size=10.5, fill=POOL, bg=WORK_T)
    s.arrow([(x + 90, 504), (x + 90, 282)], stroke=LIVE)
    s.label(x + 90, 330, "join", size=10.5, fill=LIVE, bg=WORK_T)
    s.arrow([(x + 200, 504), (x + 200, 422)], stroke=MUTED, dash="4 3")
    s.label(x + 200, 470, "&C, &S", size=11, fill=MUTED, bg=WHITE)
    s.arrow([(x + 40, 132), (x + 40, 196)], stroke=CONS)
    s.arrow([(x + 290, 280), (x + 300, 280), (x + 300, 688)], stroke=INK)
    s.label(x + 300, 664, "replies", size=10.5, bg=WHITE)
    s.text(500, 750, "&S: read-only access to state another thread owns", size=11.5, fill=MUTED, anchor="middle")
    s.save(os.path.join(out, f"{PFX}-a2-rooms"))


def mini_layered(s, x, y, k=1.0):
    block(s, x + 10 * k, y + 10 * k, 58 * k, 52 * k, CONF, CONF_T, "S", size=13 * k, shadow=False)
    block(s, x, y, 26 * k, 24 * k, SPEC, SPEC_T, "C", size=11 * k, shadow=False)


def gear(s, cx, cy, r, colour):
    """A circular 're-run' mark."""
    s.path(f"M{cx + r:.1f},{cy:.1f} A{r},{r} 0 1 1 {cx:.1f},{cy - r:.1f}", stroke=colour, sw=2.2, arrow=colour)


def a3_matrix(out):
    """A pictogram matrix: rows are aspects, columns are executors."""
    s = Svg(1000, 750)
    s.title("Executor architecture", "A3 · side by side: what each executor holds and does")
    cx = {"crud": 380, "dual": 610, "scal": 840}
    for k, name in (("crud", "CRUD cache"), ("dual", "Dual-state"), ("scal", "Scalable CRUD")):
        s.text(cx[k], 126, name, size=15, weight=600, anchor="middle")
    rows = [("threads", 146), ("memory", 246), ("speculate", 360), ("commit", 496), ("backtrack", 614)]
    for name, y in rows:
        s.line(30, y, 970, y, stroke=LINE, sw=1)
        s.text(40, y + 54, name, size=15, weight=600, fill=INK)
    s.line(30, 728, 970, 728, stroke=LINE, sw=1)

    def cap(x, y, t):
        s.text(x, y, t, size=11.5, fill=MUTED, anchor="middle")

    # threads
    y = 186
    x = cx["crud"] - 66
    x += thread_badge(s, x, y, "worker", WORK) + 6
    thread_badge(s, x, y, "read ×4", READP)
    x = cx["dual"] - 100
    for n in ("main", "preemptive", "confirmed"):
        x += thread_badge(s, x, y, n, WORK) + 5
    thread_badge(s, cx["dual"] - 26, y + 26, "read ×4", READP)
    x = cx["scal"] - 92
    x += thread_badge(s, x, y, "worker", WORK) + 6
    x += thread_badge(s, x, y, "exec ×4", POOL) + 6
    thread_badge(s, x, y, "read ×4", READP)

    # memory, drawn to a rough scale: S is the big square
    y = 262
    block(s, cx["crud"] - 50, y, 64, 64, CONF, CONF_T, "S", size=14, shadow=False)
    block(s, cx["crud"] + 22, y + 34, 24, 24, SPEC, SPEC_T, "C", size=11, shadow=False)
    for i in range(3):
        s.rect(cx["crud"] + 52 + i * 9, y + 44, 7, 14, r=2, fill=SPEC_T, stroke=SPEC, sw=1)
    cap(cx["crud"], y + 82, "S + a small cache + log")
    block(s, cx["dual"] - 70, y, 64, 64, CONF, CONF_T, "S", size=14, shadow=False)
    block(s, cx["dual"] + 6, y, 64, 64, SPEC, SPEC_T, "S_{p}", size=14, shadow=False)
    cap(cx["dual"], y + 82, "two full copies")
    block(s, cx["scal"] - 50, y, 64, 64, CONF, CONF_T, "S", size=14, shadow=False)
    block(s, cx["scal"] + 22, y + 34, 24, 24, SPEC, SPEC_T, "C", size=11, shadow=False)
    for i in range(3):
        s.rect(cx["scal"] + 52 + i * 9, y + 44, 7, 14, r=2, fill=SPEC_T, stroke=SPEC, sw=1)
    for i in range(4):
        s.circle(cx["scal"] + 24 + i * 9, y + 20, 3.2, fill=LIVE)
    cap(cx["scal"], y + 82, "same, + one tiny cache per op")

    # speculate
    y = 376
    capsule(s, cx["crud"] - 46, y, 92, 26, "batch", size=11)
    s.arrow([(cx["crud"], y + 26), (cx["crud"], y + 40)], stroke=MUTED, dash="3 3")
    mini_layered(s, cx["crud"] - 34, y + 42, 0.8)
    cap(cx["crud"], y + 118, "in a container over C, S")
    card(s, cx["dual"] - 80, y + 28, "k", SPEC, SPEC_T, w=30, h=26, shadow=False)
    s.arrow([(cx["dual"] - 46, y + 41), (cx["dual"] - 10, y + 41)], stroke=SPEC)
    block(s, cx["dual"] - 6, y + 8, 58, 64, SPEC, SPEC_T, "S_{p}", size=13, shadow=False)
    cap(cx["dual"], y + 118, "directly on the copy")
    for i in range(4):
        capsule(s, cx["scal"] - 92 + i * 34, y + 4, 30, 20, None)
        s.arrow([(cx["scal"] - 77 + i * 34, y + 24), (cx["scal"] - 6, y + 52)], stroke=LIVE, sw=1.1)
    s.rect(cx["scal"] - 92, y - 2, 134, 32, r=8, fill="none", stroke=POOL, sw=1.2, dash="2 3")
    card(s, cx["scal"] - 22, y + 50, "ω", LIVE, LIVE_T, w=32, h=26, shadow=False)
    s.text(cx["scal"] + 50, y + 18, "parallel", size=11, fill=POOL)
    cap(cx["scal"], y + 118, "ops in parallel, then merged")

    # commit
    y = 520
    for k in ("crud", "scal"):
        card(s, cx[k] - 66, y + 18, "Ω", SPEC, SPEC_T, w=30, h=26, shadow=False)
        s.arrow([(cx[k] - 32, y + 31), (cx[k] - 4, y + 31)], stroke=CONF, sw=2)
        block(s, cx[k], y, 60, 60, CONF, CONF_T, "S", size=14, shadow=False)
        cap(cx[k], y + 86, "apply the stored writes")
    card(s, cx["dual"] - 76, y + 18, "k", SPEC, SPEC_T, w=30, h=26, shadow=False)
    gear(s, cx["dual"] - 24, y + 31, 11, CONF)
    block(s, cx["dual"] + 4, y, 60, 60, CONF, CONF_T, "S", size=14, shadow=False)
    cap(cx["dual"], y + 86, "run the batch again on S")

    # backtrack
    y = 640
    for k in ("crud", "scal"):
        x0 = cx[k] - 66
        for i, op in enumerate((1, 1, 0.35, 0.35)):
            card(s, x0 + i * 34, y + 8, "Ω", SPEC, SPEC_T, w=30, h=26, shadow=False, opacity=op)
        s.line(x0 + 66, y + 4, x0 + 134, y + 38, stroke=DROP, sw=2)
        cap(cx[k], y + 62, "drop the tail, refold C")
    block(s, cx["dual"] - 76, y, 50, 46, CONF, CONF_T, "S", size=13, shadow=False)
    s.arrow([(cx["dual"] - 22, y + 23), (cx["dual"] + 18, y + 23)], stroke=DROP, dash="4 3")
    block(s, cx["dual"] + 22, y, 50, 46, SPEC, SPEC_T, "S_{p}", size=13, shadow=False)
    cap(cx["dual"], y + 62, "copy S, then replay")
    s.save(os.path.join(out, f"{PFX}-a3-matrix"))


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "."
    os.makedirs(out, exist_ok=True)
    for f in (a1_blocks, a2_rooms, a3_matrix):
        f(out)
        print(f.__name__)
