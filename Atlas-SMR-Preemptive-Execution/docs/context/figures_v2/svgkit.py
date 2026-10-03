"""A small SVG builder for the v2 figures: flat shapes, colour by meaning, few words.

Text markup: `Ω_{7}` renders 7 as a subscript, `x^{2}` as a superscript. Every figure is written
as .svg and converted with rsvg-convert to .png (2x) and .pdf (fonts embedded).
"""
import math
import re
import subprocess
from xml.sax.saxutils import escape

FONT = "Avenir Next, Helvetica Neue, Helvetica, Arial, sans-serif"

# Colour carries meaning; each role has a strong tone and a tint.
INK = "#1d2433"
MUTED = "#6b7385"
FAINT = "#aab1bf"
LINE = "#d5d9e0"
PANEL = "#f4f5f8"
WHITE = "#ffffff"

CONF, CONF_T = "#2f6fb2", "#dbe8f6"        # confirmed: S, Δ
SPEC, SPEC_T = "#d98e1f", "#fbebd0"        # speculated: C, Ω, S_p
LIVE, LIVE_T = "#7a5cc2", "#ebe4f8"        # the decision being built: ω, containers
POOL, POOL_T = "#1f9e89", "#d4efe9"        # parallel threads (exec pool)
WORK, WORK_T = "#3d4a5c", "#e3e7ee"        # the worker thread
READP, READP_T = "#a07c3b", "#f1e8d6"      # read pool
DROP, DROP_T = "#d1495b", "#f9dde1"        # discarded
CONS, CONS_T = "#4a4e69", "#e4e5ec"        # consensus / ordering


def _markup(s):
    s = escape(s)
    s = re.sub(r"_\{([^}]*)\}", r'<tspan baseline-shift="sub" font-size="72%">\1</tspan>', s)
    s = re.sub(r"\^\{([^}]*)\}", r'<tspan baseline-shift="super" font-size="72%">\1</tspan>', s)
    s = re.sub(r"\*\*([^*]*)\*\*", r'<tspan font-weight="600">\1</tspan>', s)
    return s


def text_width(s, size, weight=400):
    """Rough advance width for layout (Avenir Next averages ~0.56 em; demi ~0.6 em)."""
    plain = re.sub(r"[_^]\{([^}]*)\}", lambda m: "x" * max(1, round(len(m.group(1)) * 0.72)), s)
    plain = plain.replace("**", "")
    return len(plain) * size * (0.6 if weight >= 600 else 0.56)


class Svg:
    def __init__(self, w, h, bg=WHITE):
        self.w, self.h = w, h
        self.items = []
        self.markers = {}
        if bg:
            self.rect(0, 0, w, h, r=0, fill=bg)

    # -- shapes ----------------------------------------------------------------
    def _style(self, fill="none", stroke="none", sw=1.0, dash=None, opacity=None, extra=""):
        st = f'fill="{fill}" stroke="{stroke}" stroke-width="{sw}"'
        if dash:
            st += f' stroke-dasharray="{dash}"'
        if opacity is not None:
            st += f' opacity="{opacity}"'
        return st + extra

    def rect(self, x, y, w, h, r=10, fill="none", stroke="none", sw=1.0, dash=None, opacity=None, shadow=False):
        f = ' filter="url(#soft)"' if shadow else ""
        self.items.append(f'<rect x="{x:.2f}" y="{y:.2f}" width="{w:.2f}" height="{h:.2f}" rx="{r}" ry="{r}" '
                          f'{self._style(fill, stroke, sw, dash, opacity)}{f}/>')

    def circle(self, cx, cy, r, fill="none", stroke="none", sw=1.0, dash=None, opacity=None):
        self.items.append(f'<circle cx="{cx:.2f}" cy="{cy:.2f}" r="{r}" {self._style(fill, stroke, sw, dash, opacity)}/>')

    def poly(self, pts, fill="none", stroke="none", sw=1.0, dash=None, opacity=None, close=True):
        d = " ".join(f"{x:.2f},{y:.2f}" for x, y in pts)
        tag = "polygon" if close else "polyline"
        self.items.append(f'<{tag} points="{d}" {self._style(fill, stroke, sw, dash, opacity)} '
                          f'stroke-linejoin="round" stroke-linecap="round"/>')

    def path(self, d, fill="none", stroke=INK, sw=1.5, dash=None, opacity=None, arrow=None, start_arrow=None):
        extra = ' stroke-linecap="round" stroke-linejoin="round"'
        if arrow:
            extra += f' marker-end="url(#{self._marker(arrow)})"'
        if start_arrow:
            extra += f' marker-start="url(#{self._marker(start_arrow, start=True)})"'
        self.items.append(f'<path d="{d}" {self._style(fill, stroke, sw, dash, opacity, extra)}/>')

    def line(self, x1, y1, x2, y2, stroke=INK, sw=1.5, dash=None, arrow=None, opacity=None):
        self.path(f"M{x1:.2f},{y1:.2f} L{x2:.2f},{y2:.2f}", stroke=stroke, sw=sw, dash=dash,
                  arrow=(stroke if arrow is True else arrow), opacity=opacity)

    def arrow(self, pts, stroke=INK, sw=1.6, dash=None, curve=0.0, both=False, opacity=None, round_=10):
        """An arrow along a polyline with rounded corners, or a single curved hop when curve != 0."""
        if len(pts) == 2 and curve:
            (x1, y1), (x2, y2) = pts
            mx, my = (x1 + x2) / 2, (y1 + y2) / 2
            nx, ny = -(y2 - y1), (x2 - x1)
            cx, cy = mx + nx * curve, my + ny * curve
            d = f"M{x1:.2f},{y1:.2f} Q{cx:.2f},{cy:.2f} {x2:.2f},{y2:.2f}"
        else:
            d = f"M{pts[0][0]:.2f},{pts[0][1]:.2f}"
            for i in range(1, len(pts) - 1):
                (px, py), (x, y), (nx_, ny_) = pts[i - 1], pts[i], pts[i + 1]
                l1 = math.hypot(x - px, y - py)
                l2 = math.hypot(nx_ - x, ny_ - y)
                r = min(round_, l1 / 2, l2 / 2)
                ax, ay = x - (x - px) / l1 * r, y - (y - py) / l1 * r
                bx, by = x + (nx_ - x) / l2 * r, y + (ny_ - y) / l2 * r
                d += f" L{ax:.2f},{ay:.2f} Q{x:.2f},{y:.2f} {bx:.2f},{by:.2f}"
            d += f" L{pts[-1][0]:.2f},{pts[-1][1]:.2f}"
        self.path(d, stroke=stroke, sw=sw, dash=dash, arrow=stroke, start_arrow=stroke if both else None,
                  opacity=opacity)

    def _marker(self, colour, start=False):
        key = (colour, start)
        if key not in self.markers:
            mid = f"m{len(self.markers)}"
            d = "M10,0 L0,5 L10,10 z" if start else "M0,0 L10,5 L0,10 z"
            self.markers[key] = (mid, f'<marker id="{mid}" viewBox="0 0 10 10" refX="{0 if start else 10}" refY="5" '
                                      f'markerWidth="6.5" markerHeight="6.5" markerUnits="userSpaceOnUse" '
                                      f'orient="auto"><path d="{d}" fill="{colour}"/></marker>')
        return self.markers[key][0]

    # -- text ------------------------------------------------------------------
    def text(self, x, y, s, size=13, weight=400, fill=INK, anchor="start", italic=False, spacing=None,
             opacity=None):
        st = f'font-family="{FONT}" font-size="{size}" font-weight="{weight}" fill="{fill}" text-anchor="{anchor}"'
        if italic:
            st += ' font-style="italic"'
        if spacing:
            st += f' letter-spacing="{spacing}"'
        if opacity is not None:
            st += f' opacity="{opacity}"'
        lines = s.split("\n")
        for i, ln in enumerate(lines):
            self.items.append(f'<text x="{x:.2f}" y="{y + i * size * 1.25:.2f}" {st}>{_markup(ln)}</text>')

    def label(self, x, y, s, size=11, fill=MUTED, anchor="middle", weight=400, bg=WHITE, pad=3):
        """Text on a background patch, for labels that sit on top of lines."""
        w = text_width(s, size, weight)
        x0 = x - (w / 2 if anchor == "middle" else (w if anchor == "end" else 0))
        if bg:
            self.rect(x0 - pad, y - size * 0.9, w + 2 * pad, size * 1.25, r=3, fill=bg)
        self.text(x, y, s, size=size, fill=fill, anchor=anchor, weight=weight)

    def title(self, title, sub=None, x=32, y=44):
        self.text(x, y, title, size=22, weight=600)
        if sub:
            self.text(x, y + 24, sub, size=13.5, fill=MUTED)

    # -- output ----------------------------------------------------------------
    def svg(self):
        defs = ['<filter id="soft" x="-20%" y="-20%" width="140%" height="160%">'
                '<feDropShadow dx="0" dy="1.5" stdDeviation="2" flood-color="#1d2433" flood-opacity="0.12"/></filter>']
        defs += [m for _, m in self.markers.values()]
        return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{self.w}" height="{self.h}" '
                f'viewBox="0 0 {self.w} {self.h}"><defs>{"".join(defs)}</defs>\n' + "\n".join(self.items) + "\n</svg>\n")

    def save(self, stem):
        with open(stem + ".svg", "w") as f:
            f.write(self.svg())
        subprocess.run(["rsvg-convert", "-z", "2", stem + ".svg", "-o", stem + ".png"], check=True)
        subprocess.run(["rsvg-convert", "-f", "pdf", stem + ".svg", "-o", stem + ".pdf"], check=True)


# -- shared glyphs -----------------------------------------------------------------
def block(s, x, y, w, h, colour, tint, name, size=15, sub=None, shadow=True):
    """A state: a solid, rounded block with its name."""
    s.rect(x, y, w, h, r=10, fill=tint, stroke=colour, sw=1.6, shadow=shadow)
    s.text(x + w / 2, y + h / 2 + size * 0.35 - (7 if sub else 0), name, size=size, weight=600, fill=colour,
           anchor="middle")
    if sub:
        s.text(x + w / 2, y + h / 2 + 13, sub, size=11, fill=MUTED, anchor="middle")


def card(s, x, y, name, colour, tint, w=40, h=28, size=13, shadow=True, dash=None, opacity=None):
    s.rect(x, y, w, h, r=6, fill=tint, stroke=colour, sw=1.4, shadow=shadow, dash=dash, opacity=opacity)
    s.text(x + w / 2, y + h / 2 + size * 0.36, name, size=size, weight=600, fill=colour, anchor="middle",
           opacity=opacity)


def stack(s, x, y, names, colour, tint, w=40, h=28, gap=6, size=13):
    """A row of cards, left (oldest) to right (newest)."""
    for i, n in enumerate(names):
        card(s, x + i * (w + gap), y, n, colour, tint, w, h, size)
    return x + len(names) * (w + gap) - gap


def capsule(s, x, y, w, h, text=None, colour=LIVE, tint=LIVE_T, size=12, dash="4 3"):
    """A container: the proxy an operation runs inside (dashed violet capsule)."""
    s.rect(x, y, w, h, r=h / 2, fill=tint, stroke=colour, sw=1.5, dash=dash)
    if text:
        s.text(x + w / 2, y + h / 2 + size * 0.36, text, size=size, weight=600, fill=colour, anchor="middle")


def thread_badge(s, x, y, name, colour, size=10.5):
    w = text_width(name, size, 600) + 12
    s.rect(x, y - 9, w, 18, r=9, fill=colour)
    s.text(x + w / 2, y + 3.8, name, size=size, weight=600, fill=WHITE, anchor="middle")
    return w


def person(s, cx, cy, colour=MUTED, scale=1.0):
    s.circle(cx, cy - 7 * scale, 5 * scale, fill=colour)
    s.path(f"M{cx - 9 * scale:.1f},{cy + 9 * scale:.1f} Q{cx:.1f},{cy - 6 * scale:.1f} {cx + 9 * scale:.1f},{cy + 9 * scale:.1f} Z",
           fill=colour, stroke="none", sw=0)


def legend(s, x, y, items, size=11.5, gap=22):
    """items: (kind, colour, tint, text); kind in block | capsule | badge | line | dash."""
    for kind, colour, tint, t in items:
        if kind == "block":
            s.rect(x, y - 8, 16, 12, r=3, fill=tint, stroke=colour, sw=1.3)
            x += 22
        elif kind == "capsule":
            s.rect(x, y - 8, 22, 12, r=6, fill=tint, stroke=colour, sw=1.3, dash="3 2")
            x += 28
        elif kind == "badge":
            s.rect(x, y - 8, 16, 12, r=6, fill=colour)
            x += 22
        elif kind in ("line", "dash"):
            s.line(x, y - 2, x + 22, y - 2, stroke=colour, sw=1.6, dash="4 3" if kind == "dash" else None, arrow=True)
            x += 30
        s.text(x, y + 2, t, size=size, fill=MUTED)
        x += text_width(t, size) + gap
    return x
