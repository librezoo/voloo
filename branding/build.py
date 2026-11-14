"""Build font-independent voloo wordmarks from the LibreZoo lettering family.

The existing lowercase l and o outlines are reused unchanged. The new v follows
the family's 100-unit x-height, 25-unit stems and small rounded terminals.
Run: python3 branding/build.py
"""
from pathlib import Path
import math
import re
import xml.etree.ElementTree as ET

HERE = Path(__file__).resolve().parent
SOURCE = HERE / 'source'
original = ET.parse(SOURCE / 'librezoo-wordmark-mono.svg').getroot()
contours = re.findall(r'M[^M]*', list(original)[0].attrib['d'])
L = contours[0]
O = contours[8] + contours[9]

def rounded_polygon(points, cuts):
    corners = []
    for i, p in enumerate(points):
        before, after = points[i - 1], points[(i + 1) % len(points)]
        def toward(q, d):
            length = math.hypot(q[0] - p[0], q[1] - p[1])
            d = min(d, length * .35)
            return (p[0] + (q[0] - p[0]) * d / length, p[1] + (q[1] - p[1]) * d / length)
        corners.append((toward(before, cuts[i]), p, toward(after, cuts[i])))
    def pair(p): return f'{p[0]:.4f} {p[1]:.4f}'
    result = 'M' + pair(corners[0][0])
    for i, (start, vertex, end) in enumerate(corners):
        if i: result += ' L' + pair(start)
        result += ' Q' + pair(vertex) + ' ' + pair(end)
    return result + ' Z'

V = rounded_polygon([(0,-100),(27,-100),(55,-24),(83,-100),(110,-100),(68,0),(42,0)], [2.758,2.758,5,2.758,2.758,7.5,7.5])
word_paths = (f'<path d="{V}"/>'
    + f'<g transform="translate({109-491.1366:.4f} 0)"><path d="{O}"/></g>'
    + f'<g transform="translate({231-6.75:.4f} 0)"><path d="{L}"/></g>'
    + f'<g transform="translate({279-491.1366:.4f} 0)"><path d="{O}"/></g>'
    + f'<g transform="translate({395.576-491.1366:.4f} 0)"><path d="{O}"/></g>')

def svg(view, body, label):
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{view}" role="img" aria-label="{label}"><title>{label}</title>{body}</svg>\n'

def word(ink):
    return f'<g fill="{ink}">{word_paths}</g>'

def inset_word(x, y, width, height, ink):
    return f'<svg x="{x}" y="{y}" width="{width}" height="{height}" viewBox="-2 -160 509 165">{word(ink)}</svg>'

def parent(file, x, y, width):
    source = (SOURCE / file).read_text()
    box = re.search(r'viewBox="([^"]+)"', source)[1]
    inner = re.sub(r'^<svg[^>]*>|</svg>\s*$', '', source.strip())
    return f'<svg x="{x}" y="{y}" width="{width}" height="{width * 207.0637 / 986.9386:.4f}" viewBox="{box}">{inner}</svg>'

for dark in [False, True]:
    suffix = '-on-dark' if dark else ''
    ink = '#F5F1E9' if dark else '#2D1A2B'
    (HERE / f'voloo-wordmark{suffix}.svg').write_text(svg('-2 -160 509 165', word(ink), 'voloo'))
    for small in [False, True]:
        name = f'librezoo-voloo{"-small" if small else ""}{suffix}.svg'
        parent_file = 'librezoo-horizontal-on-dark.svg' if dark else 'librezoo-horizontal-small.svg' if small else 'librezoo-horizontal.svg'
        content = parent(parent_file, 22, 22, 260)
        content += f'<path d="M310 22V78" fill="none" stroke="{ink}" stroke-width="1.5" opacity=".35"/>'
        content += inset_word(338, 14, 220, 72, ink)
        (HERE / name).write_text(svg('0 0 600 100', content, 'librezoo · voloo'))
    parent_file = 'librezoo-horizontal-on-dark.svg' if dark else 'librezoo-horizontal-small.svg'
    content = parent(parent_file, 10, 10, 120) + inset_word(18, 48, 104, 34, ink)
    (HERE / f'librezoo-voloo-stacked{suffix}.svg').write_text(svg('0 0 140 86', content, 'librezoo · voloo'))

print('Built voloo wordmark and complete LibreZoo / voloo family lockups.')
