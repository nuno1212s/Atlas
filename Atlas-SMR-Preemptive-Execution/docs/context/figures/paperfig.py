"""Print-size drawing canvas for paper figures (matplotlib, vector output).

Coordinates are in typographic points (1/72 in), origin top-left, y growing downward, so a
figure is laid out exactly as it will print. Grayscale only; fonts are embedded as TrueType.
"""
import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402
from matplotlib.patches import FancyArrowPatch, FancyBboxPatch  # noqa: E402

# Two-column venue: ACM/IEEE column ≈ 241–252 pt, text block ≈ 505–516 pt.
COLUMN = 241
FULL = 505

SANS = ["Helvetica", "Arial", "DejaVu Sans"]
MONO = ["DejaVu Sans Mono"]

matplotlib.rcParams.update({
    "font.family": "sans-serif",
    "font.sans-serif": SANS,
    "font.size": 7.5,
    "mathtext.fontset": "custom",
    "mathtext.rm": "Arial",
    "mathtext.it": "Arial:italic",
    "mathtext.bf": "Arial:bold",
    "mathtext.fallback": "stixsans",
    "pdf.fonttype": 42,
    "ps.fonttype": 42,
    "svg.fonttype": "none",
})

# Grayscale ramp (luminance only, so it survives any printer).
INK = "#000000"
DARK = "#3d3d3d"
MID = "#8c8c8c"
LIGHT = "#cfcfcf"
PALE = "#ececec"
PAPER = "#ffffff"

BODY = 7.5
SMALL = 6.5
TINY = 5.8


class Canvas:
    def __init__(self, width, height):
        self.w, self.h = width, height
        self.fig = plt.figure(figsize=(width / 72, height / 72), dpi=72)
        ax = self.fig.add_axes([0, 0, 1, 1])
        ax.set_xlim(0, width)
        ax.set_ylim(height, 0)
        ax.axis("off")
        self.ax = ax

    # -- primitives --------------------------------------------------------
    def box(self, x, y, w, h, fill=PAPER, edge=INK, lw=0.6, ls="-", r=1.6, hatch=None, z=2):
        p = FancyBboxPatch((x, y), w, h, boxstyle=f"round,pad=0,rounding_size={r}",
                           facecolor=fill, edgecolor=edge, linewidth=lw, linestyle=ls,
                           hatch=hatch, zorder=z)
        self.ax.add_patch(p)
        return p

    def text(self, x, y, s, size=BODY, ha="left", va="center", color=INK, mono=False,
             weight="normal", style="normal", z=5, **kw):
        return self.ax.text(x, y, s, fontsize=size, ha=ha, va=va, color=color,
                            family=MONO if mono else SANS, fontweight=weight,
                            fontstyle=style, zorder=z, **kw)

    def arrow(self, a, b, lw=0.6, color=INK, head=(3.2, 1.5), ls="-", z=4, conn=None,
              both=False):
        style = f"{'<' if both else ''}-|>,head_length={head[0]},head_width={head[1]}"
        p = FancyArrowPatch(a, b, arrowstyle=style, mutation_scale=1, linewidth=lw,
                            color=color, linestyle=ls, shrinkA=0, shrinkB=0, zorder=z,
                            connectionstyle=conn or "arc3,rad=0")
        self.ax.add_patch(p)
        return p

    def line(self, pts, lw=0.6, color=INK, ls="-", z=3):
        xs, ys = zip(*pts)
        self.ax.plot(xs, ys, lw=lw, color=color, ls=ls, zorder=z, solid_capstyle="butt")

    def bracket(self, x, y0, y1, tick=2.5, lw=0.6, color=INK):
        """Right-facing square bracket at x spanning y0..y1."""
        self.line([(x, y0), (x + tick, y0), (x + tick, y1), (x, y1)], lw=lw, color=color)

    def save(self, stem, formats=("pdf", "svg", "png")):
        for fmt in formats:
            self.fig.savefig(f"{stem}.{fmt}", dpi=400 if fmt == "png" else 72,
                             facecolor=PAPER)
        plt.close(self.fig)
