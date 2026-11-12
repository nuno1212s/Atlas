"""A small executable model of the three preemptive executors, used to draw their figures.

Every figure takes its numbers from here rather than from hand-typed tables, so the CRUD
cache, dual-state and scalable figures describe one workload and cannot drift apart. The
model mirrors, in miniature:

  CRUD cache   single_threaded_crud/pending_state.rs   execute_speculatively, handle_update_confirmed, backtrack
  dual-state   single_thread_double_state/preemptive_worker/preemptive_requests.rs   (+ confirmed worker)
  scalable     scalable_crud/pending_state.rs:197-307   (parallel units, collisions, two passes)

Operations are ("w", key, value) for create/update, ("d", key) for delete and ("r", key) for a
read. The benchmark application maps both Create and Update to `CRUDState::update`.
"""
from dataclasses import dataclass, field, replace

KEYS = "abcde"
TOMB = "⊥"

S_INIT = {"a": "1", "b": "2", "c": "3", "e": "7"}
BATCH = {
    7: (("w", "a", "5"), ("w", "d", "9")),
    8: (("d", "b"),),
    9: (("w", "a", "6"), ("w", "c", "4")),
    # o1..o4: o2 writes e and o3 reads it, so the scalable executor flags a collision
    10: (("w", "c", "5"), ("w", "e", "8"), ("r", "e"), ("r", "b")),
}


def op_text(op):
    if op[0] == "w":
        return f"{op[1]} ← {op[2]}"
    if op[0] == "d":
        return f"del {op[1]}"
    return f"read {op[1]}"


def shown(val):
    """How a read result reads to a client: a tombstone or a missing key is 'none'."""
    return "none" if val in (None, TOMB) else val


def run(ops, base_read):
    """Execute ops in order through a private delta over `base_read` (CachingState)."""
    delta, replies, accesses = {}, [], []

    def rd(k):
        return delta[k] if k in delta else base_read(k)

    for op in ops:
        kind, k = op[0], op[1]
        replies.append(shown(rd(k)))
        if kind == "w":
            delta[k] = op[2]
        elif kind == "d":
            delta[k] = TOMB
        accesses.append((k, {"w": "W", "d": "D", "r": "R"}[kind]))
    return delta, replies, accesses


def fold(deltas):
    acc = {}
    for d in deltas:
        acc.update(d)
    return acc


def apply(S, delta):
    out = dict(S)
    for k, val in delta.items():
        if val == TOMB:
            out.pop(k, None)
        else:
            out[k] = val
    return out


def layered(*layers):
    """Read through layers in order; a tombstone in a cache layer stops the read."""
    def rd(k):
        for layer in layers[:-1]:
            if k in layer:
                return layer[k]
        return layers[-1].get(k)
    return rd


# ---------------------------------------------------------------------------
# CRUD cache
# ---------------------------------------------------------------------------
@dataclass(frozen=True)
class Crud:
    S: dict
    log: tuple = ()             # ((seq, delta, replies), ...) oldest first
    staged: tuple = (12,)

    @property
    def C(self):
        return fold(d for _, d, _ in self.log)

    def entry(self, seq):
        return next(d for q, d, _ in self.log if q == seq)

    def seqs(self):
        return [q for q, _, _ in self.log]


def crud_speculate(st, seq):
    delta, replies, _ = run(BATCH[seq], layered(st.C, st.S))
    return replace(st, log=st.log + ((seq, delta, tuple(replies)),))


def crud_commit(st):
    _, delta, _ = st.log[0]
    return replace(st, S=apply(st.S, delta), log=st.log[1:])


def crud_backtrack(st, k):
    return replace(st, log=tuple(e for e in st.log if e[0] < k),
                   staged=tuple(s for s in st.staged if s < k))


# ---------------------------------------------------------------------------
# Dual-state: two full copies; the log holds batches (and replies), never deltas
# ---------------------------------------------------------------------------
@dataclass(frozen=True)
class Dual:
    S: dict                     # confirmed worker's confirmed_state
    P: dict                     # preemptive worker's preemptive_state (full copy)
    log: tuple = ()             # ((seq, replies), ...) oldest first

    def seqs(self):
        return [q for q, _ in self.log]


def dual_speculate(st, seq):
    delta, replies, _ = run(BATCH[seq], layered(st.P))
    return replace(st, P=apply(st.P, delta), log=st.log + ((seq, tuple(replies)),))


def dual_commit(st):
    seq, _ = st.log[0]
    delta, _, _ = run(BATCH[seq], layered(st.S))        # re-executed; replies discarded
    return replace(st, S=apply(st.S, delta), log=st.log[1:])


def dual_backtrack(st, k):
    P = dict(st.S)                                      # clone of the confirmed state
    kept = []
    for seq, _ in st.log:
        if seq < k:                                     # survivors are re-executed
            delta, replies, _ = run(BATCH[seq], layered(P))
            P = apply(P, delta)
            kept.append((seq, tuple(replies)))
    return replace(st, P=P, log=tuple(kept))


# ---------------------------------------------------------------------------
# Scalable CRUD: same cross-batch layer as the CRUD cache, plus an intra-batch layer
# ---------------------------------------------------------------------------
@dataclass
class ScalableTrace:
    seq: int
    ops: tuple
    lanes: list = field(default_factory=list)      # (local_cache, accesses, reply) per op
    colliders: set = field(default_factory=set)
    collided_on: dict = field(default_factory=dict)  # key -> positions involved
    bd_pass1: dict = field(default_factory=dict)    # batch_delta after merging non-colliders
    view: dict = field(default_factory=dict)        # current_view = C ⊕ bd_pass1
    reexec: list = field(default_factory=list)      # (pos, delta, reply) per collider, in order
    delta: dict = field(default_factory=dict)       # final batch_delta = Δseq
    replies: list = field(default_factory=list)


def scalable_speculate(st, seq):
    ops = BATCH[seq]
    tr = ScalableTrace(seq, ops)
    C, S = st.C, st.S
    # step 1: every op in parallel, each through its own local cache over C over S
    for op in ops:
        local, replies, accesses = run((op,), layered(C, S))
        tr.lanes.append((local, accesses, replies[0]))
    # step 2: collisions — Write or Delete with anything on the same key
    seen = {}
    for pos, (_, accesses, _) in enumerate(tr.lanes):
        for k, kind in accesses:
            for other_pos, other_kind in seen.get(k, []):
                if "W" in (kind, other_kind) or "D" in (kind, other_kind):
                    tr.colliders |= {pos, other_pos}
                    tr.collided_on.setdefault(k, set()).update({pos, other_pos})
            seen.setdefault(k, []).append((pos, kind))
    # step 3: merge non-colliders in batch order
    for pos, (local, _, _) in enumerate(tr.lanes):
        if pos not in tr.colliders:
            tr.bd_pass1.update(local)
    bd = dict(tr.bd_pass1)
    view = {**C, **bd}
    tr.view = dict(view)
    # step 4: re-execute colliders sequentially over current_view
    for pos in sorted(tr.colliders):
        local, replies, _ = run((ops[pos],), layered(view, S))
        bd.update(local)
        view.update(local)
        tr.reexec.append((pos, local, replies[0]))
    tr.delta = bd
    reply_of = {pos: r for pos, (_, _, r) in enumerate(tr.lanes) if pos not in tr.colliders}
    reply_of.update({pos: r for pos, _, r in tr.reexec})
    tr.replies = [reply_of[i] for i in range(len(ops))]
    return replace(st, log=st.log + ((seq, bd, tuple(tr.replies)),)), tr


# ---------------------------------------------------------------------------
# The four steps every storyboard draws
# ---------------------------------------------------------------------------
def _crud_states():
    st = Crud(S=dict(S_INIT))
    for q in (7, 8, 9):
        st = crud_speculate(st, q)
    s1 = crud_speculate(st, 10)
    s2 = crud_commit(s1)
    return [st, s1, s2, crud_backtrack(s2, 9)]


def _dual_states():
    st = Dual(S=dict(S_INIT), P=dict(S_INIT))
    for q in (7, 8, 9):
        st = dual_speculate(st, q)
    s1 = dual_speculate(st, 10)
    s2 = dual_commit(s1)
    return [st, s1, s2, dual_backtrack(s2, 9)]


def _scalable_states():
    st = Crud(S=dict(S_INIT))
    for q in (7, 8, 9):
        st, _ = scalable_speculate(st, q)
    s1, trace = scalable_speculate(st, 10)
    s2 = crud_commit(s1)
    return [st, s1, s2, crud_backtrack(s2, 9)], trace


CRUD = _crud_states()
DUAL = _dual_states()
SCALABLE, TRACE10 = _scalable_states()


def _check():
    c0, c1, c2, c3 = CRUD
    assert c0.C == {"a": "6", "b": TOMB, "c": "4", "d": "9"}
    assert c1.entry(10) == {"c": "5", "e": "8"} and c1.log[-1][2] == ("4", "7", "8", "none")
    assert c2.S == {"a": "5", "b": "2", "c": "3", "d": "9", "e": "7"}
    assert c2.C == {"a": "6", "b": TOMB, "c": "5", "e": "8"} and c3.C == {"b": TOMB}
    d0, d1, d2, d3 = DUAL
    assert d0.P == {"a": "6", "c": "4", "d": "9", "e": "7"} and d1.P["e"] == "8"
    assert d2.S == c2.S and d3.P == {"a": "5", "c": "3", "d": "9", "e": "7"}
    assert TRACE10.colliders == {1, 2} and TRACE10.lanes[2][2] == "7"      # stale parallel read
    assert TRACE10.replies == ["4", "7", "8", "none"]                      # corrected to 8
    assert TRACE10.delta == c1.entry(10) and [s.C for s in SCALABLE] == [s.C for s in CRUD]


_check()
