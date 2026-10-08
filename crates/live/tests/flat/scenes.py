import os
import sys

from playwright.sync_api import sync_playwright

GROUND = '#eceff3'
SURFACE = '#ffffff'
SLATE = '#2b3442'
RAIL = '#36404f'
MUTE = '#8d96a3'
LINE = '#d3d9e1'
FAINT = '#f2f4f7'
TEAL = '#138f8a'
CORAL = '#e2614c'
INDIGO = '#4c5fd0'
SUN = '#f0b429'
SAGE = '#5f9e62'
PLUM = '#8e57b0'
SKY = '#3d8fd8'
TAGS = [CORAL, PLUM, SUN, TEAL, SAGE, INDIGO]

POINTER = 'M0 0V52L12 41L21 60L30 56L21 37H37Z'

DEFS = f'''<defs>
<filter id="lift1" x="-50%" y="-50%" width="200%" height="250%"><feDropShadow dx="0" dy="4" stdDeviation="5" flood-color="{SLATE}" flood-opacity=".22"/></filter>
<filter id="lift2" x="-50%" y="-50%" width="200%" height="250%"><feDropShadow dx="0" dy="12" stdDeviation="16" flood-color="{SLATE}" flood-opacity=".26"/></filter>
<filter id="halo" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="8"/></filter>
<linearGradient id="sunrise" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#ffd36b"/><stop offset="1" stop-color="#f0a92e"/></linearGradient>
<linearGradient id="deep" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#5b78e6"/><stop offset="1" stop-color="#3a4fc0"/></linearGradient>
<linearGradient id="mist" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#ffffff" stop-opacity=".85"/><stop offset="1" stop-color="#ffffff" stop-opacity=".4"/></linearGradient>
<linearGradient id="blush" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#ffe1d8" stop-opacity=".95"/><stop offset="1" stop-color="#ffffff" stop-opacity=".3"/></linearGradient>
<linearGradient id="fade" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{SKY}" stop-opacity=".65"/><stop offset="1" stop-color="{SKY}" stop-opacity="0"/></linearGradient>
<g id="pointer"><path d="{POINTER}" fill="{SKY}" stroke="#fff" stroke-width="5" stroke-linejoin="round"/></g>
<g id="ghost"><path d="{POINTER}" fill="{SLATE}" opacity=".2"/></g>
<g id="funnel"><path d="M-12 -10H12L3 2V12L-3 9V2Z" fill="#fff"/></g>
</defs>'''


def f(value):
    return f'{value:.1f}'.rstrip('0').rstrip('.')


def svg(w, h, ground, body):
    return f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">{DEFS}<rect width="{w}" height="{h}" fill="{ground}"/>{body}</svg>'


def tile(x, y, w, h, tag, rot=0.0, faded=False, depth=0):
    cx, cy = x + w / 2, y + h / 2
    o = ''
    for d in range(depth, 0, -1):
        o += f'<rect x="{f(x + d * 5)}" y="{f(y - d * 6)}" width="{f(w)}" height="{f(h)}" rx="{f(h * .22)}" fill="{SURFACE}" stroke="{LINE}" stroke-width="2" transform="rotate({f(rot)} {f(cx)} {f(cy)})"/>'
    o += f'<g transform="translate({f(cx)},{f(cy)}) rotate({f(rot)}) translate({f(-w / 2)},{f(-h / 2)})" filter="url(#lift1)" opacity="{".5" if faded else "1"}">'
    o += f'<rect width="{f(w)}" height="{f(h)}" rx="{f(h * .22)}" fill="{SURFACE}"/>'
    o += f'<rect width="{f(h * .12)}" height="{f(h)}" rx="{f(h * .06)}" fill="{tag}"/>'
    o += f'<circle cx="{f(h * .55)}" cy="{f(h / 2)}" r="{f(h * .25)}" fill="{tag}" opacity=".85"/>'
    o += f'<rect x="{f(h)}" y="{f(h * .3)}" width="{f(w * .35)}" height="{f(h * .14)}" rx="{f(h * .07)}" fill="{SLATE}"/>'
    o += f'<rect x="{f(h)}" y="{f(h * .58)}" width="{f(w * .5)}" height="{f(h * .1)}" rx="{f(h * .05)}" fill="#c5ccd6"/>'
    return o + '</g>'


def pill(x, y, w, h):
    o = f'<g filter="url(#lift1)"><rect x="{f(x)}" y="{f(y)}" width="{f(w)}" height="{f(h)}" rx="{f(h / 2)}" fill="url(#deep)"/>'
    o += f'<rect x="{f(x + 9)}" y="{f(y + 6)}" width="{f(w - 18)}" height="{f(h * .3)}" rx="{f(h * .15)}" fill="#fff" opacity=".2"/></g>'
    k = h / 110
    o += f'<path d="{POINTER}" fill="#fff" transform="translate({f(x + w / 2 - h * .3)},{f(y + h * .2)}) scale({k:.2f})"/>'
    return o


def chip(x, y, w, h):
    o = f'<g filter="url(#lift1)"><rect x="{f(x)}" y="{f(y)}" width="{f(w)}" height="{f(h)}" rx="{f(h * .3)}" fill="url(#sunrise)" stroke="#c4840c" stroke-width="3"/></g>'
    o += f'<circle cx="{f(x + h * .6)}" cy="{f(y + h / 2)}" r="{f(h * .28)}" fill="#ffe8a8" stroke="#c4840c" stroke-width="2"/>'
    return o


def tick(x, y, s=1.0, trail=56):
    return (f'<rect x="{f(x - 2.5 * s)}" y="{f(y)}" width="{f(5 * s)}" height="{f(trail * s)}" rx="2.5" fill="url(#fade)"/>'
            f'<path d="M{f(x - 10 * s)} {f(y + 1 * s)}l{f(7 * s)} {f(8 * s)}l{f(14 * s)} -{f(17 * s)}" stroke="{SKY}" stroke-width="{f(6 * s)}" fill="none" stroke-linecap="round" stroke-linejoin="round"/>')


def meter(cx, cy, r, sw, length, colour):
    return (f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="none" stroke="#e1e5eb" stroke-width="{sw}"/>'
            f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="none" stroke="{colour}" stroke-width="{sw}" stroke-dasharray="{length} {f(r * 7)}" stroke-linecap="round" transform="rotate(-90 {cx} {cy})"/>')


def key(x, y, w=52, h=52, dark=False):
    face, edge = ('#3d4859', '#556178') if dark else ('#ffffff', '#cbd2dc')
    return (f'<rect x="{x}" y="{y + 5}" width="{w}" height="{h}" rx="11" fill="{"#1f2632" if dark else "#c9d0da"}"/>'
            f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="11" fill="{face}" stroke="{edge}" stroke-width="2"/>')


def page(x, y, w, colour, fold):
    return (f'<path d="M{f(x)} {f(y)}h{f(w * .64)}l{f(w * .36)} {f(w * .36)}v{f(w * .92)}h-{f(w)}Z" fill="{colour}"/>'
            f'<path d="M{f(x + w * .64)} {f(y)}v{f(w * .36)}h{f(w * .36)}Z" fill="{fold}"/>')


def board():
    o = []
    A = o.append
    A(f'<rect width="280" height="1080" fill="{SLATE}"/>')
    A(f'<circle cx="56" cy="62" r="26" fill="{SKY}"/><path d="M44 63l9 10l16 -20" stroke="#fff" stroke-width="7" fill="none" stroke-linecap="round" stroke-linejoin="round"/>')
    for i, tag in enumerate([CORAL, PLUM, SUN, TEAL]):
        y = 140 + i * 96
        A(f'<rect x="20" y="{y}" width="240" height="80" rx="18" fill="{RAIL}"/><rect x="20" y="{y}" width="8" height="80" rx="4" fill="{tag}"/>')
        A(key(42, y + 14, 46, 46, dark=True))
        A(f'<circle cx="118" cy="{y + 40}" r="10" fill="{tag}"/><rect x="142" y="{y + 26}" width="{90 - i * 12}" height="12" rx="6" fill="#c5ccd6"/><rect x="142" y="{y + 48}" width="{70 - i * 8}" height="8" rx="4" fill="#6f7a89"/>')
    A(f'<rect x="30" y="960" width="220" height="14" rx="7" fill="#46516a"/><rect x="30" y="960" width="132" height="14" rx="7" fill="{SAGE}"/>')
    A(f'<rect x="280" width="1160" height="84" fill="{SURFACE}"/><rect x="280" y="82" width="1160" height="2" fill="{LINE}"/>')
    A(f'<rect x="306" y="18" width="420" height="48" rx="24" fill="{FAINT}"/><circle cx="336" cy="40" r="10" fill="none" stroke="{MUTE}" stroke-width="4"/><path d="M343 48l9 9" stroke="{MUTE}" stroke-width="4" stroke-linecap="round"/>')
    for i, colour in enumerate([INDIGO, INDIGO, PLUM]):
        A(f'<rect x="{800 + i * 140}" y="16" width="120" height="52" rx="15" fill="{colour}"/>')
    lanes = [300 + i * 280 for i in range(4)]
    for i, x in enumerate(lanes):
        A(f'<rect x="{x}" y="110" width="260" height="940" rx="24" fill="url(#{"blush" if i in (1, 2) else "mist"})" stroke="{LINE}" stroke-width="2"/>')
    y = 140
    while y < 700:
        A(pill(lanes[0] + 20, y, 220, 56))
        y += 80
    tilts = [-2.4, 1.6, -0.8, 2.2, -1.4, 0.6]
    for i in range(5):
        A(tile(lanes[1] + 18, 140 + i * 82, 224, 60, TAGS[i % len(TAGS)], tilts[i], depth=2))
    A(f'<rect x="{lanes[1] + 12}" y="{140 + 4 * 82 - 6}" width="236" height="72" rx="16" fill="none" stroke="{SKY}" stroke-width="4"/>')
    for i in range(3):
        A(tile(lanes[2] + 18, 140 + i * 82, 224, 60, TAGS[(i + 2) % len(TAGS)], tilts[i + 2], depth=1))
    for i in range(2):
        A(tile(lanes[2] + 30 + i * 12, 420 + i * 60, 200, 56, TAGS[i + 3], tilts[i + 1] * 2, faded=True))
    for i in range(3):
        A(chip(lanes[3] + 30, 140 + i * 72, 200, 52))
    A(f'<g filter="url(#lift2)"><rect x="{lanes[3] + 20}" y="380" width="220" height="250" rx="20" fill="{SURFACE}" stroke="{TEAL}" stroke-width="6"/></g>')
    A(page(lanes[3] + 100, 410, 60, TEAL, '#0c6d69'))
    A(f'<rect x="{lanes[3] + 40}" y="590" width="180" height="10" rx="5" fill="#e1e5eb"/><rect x="{lanes[3] + 40}" y="590" width="70" height="10" rx="5" fill="{TEAL}"/>')
    spots = []
    for row in range(5):
        for col in range(row + 1):
            spots.append((lanes[1] + 130 - row * 22 + col * 44 + (row * 7 % 5) - 2, 780 + row * 46))
    for i in range(14):
        sx, sy = spots[(i * 5) % len(spots)]
        A(tick(sx + ((i * 13) % 30) - 15, 560 + (i * 37) % 200, .8, 56))
    for sx, sy in spots:
        A(f'<use href="#ghost" transform="translate({f(sx + 5)},{f(sy + 9)}) scale(0.6)"/>')
    for sx, sy in spots:
        A(f'<use href="#pointer" transform="translate({f(sx)},{f(sy)}) scale(0.6)"/>')
    A(f'<rect x="{lanes[1] + 6}" y="1018" width="248" height="10" rx="5" fill="{SKY}"/>')
    A(f'<rect x="1440" width="480" height="1080" fill="{SURFACE}"/><rect x="1440" width="2" height="1080" fill="{LINE}"/>')
    A(f'<g filter="url(#lift1)"><rect x="1464" y="40" width="432" height="160" rx="22" fill="{SLATE}"/></g><rect x="1488" y="164" width="384" height="14" rx="7" fill="{CORAL}"/>')
    A(f'<rect x="1488" y="64" width="220" height="22" rx="11" fill="#c5ccd6"/><rect x="1488" y="104" width="300" height="12" rx="6" fill="#6f7a89"/>')
    A(f'<rect x="1464" y="236" width="432" height="112" rx="18" fill="#f5f7fa" stroke="{SKY}" stroke-width="3"/>')
    A(f'<rect x="1464" y="370" width="432" height="96" rx="18" fill="#f5f7fa" stroke="#c3cad4" stroke-width="3" stroke-dasharray="14 10"/>')
    A(meter(1540, 560, 44, 13, 230, SUN) + meter(1680, 560, 44, 13, 120, SKY))
    A(f'<circle cx="1820" cy="560" r="62" fill="{SKY}" opacity=".25" filter="url(#halo)"/><circle cx="1820" cy="560" r="44" fill="{SKY}" filter="url(#lift1)"/>')
    for i in range(4):
        A(key(1480 + i * 66, 680) + f'<circle cx="{1780 + (i % 2) * 54}" cy="{706 + (i // 2) * 64}" r="22" fill="{[SAGE, SKY, CORAL, SLATE][i]}"/>')
    A(f'<rect x="1440" y="900" width="480" height="180" fill="{SLATE}" opacity=".55"/><rect x="1500" y="930" width="360" height="56" rx="28" fill="#fff"/>')
    return ''.join(o)


def pattern_geometry(kind, s, w):
    h = w * s / 2
    n = int(200 / s) + 2
    d = ''
    for k in range(-n, n + 1):
        c = k * s
        if kind in ('stripes', 'hatch'):
            d += f'M{c - h} -200H{c + h}V200H{c - h}Z'
        if kind == 'hatch':
            d += f'M-200 {c - h}H200V{c + h}H-200Z'
        if kind == 'dots':
            for j in range(-n, n + 1):
                y = j * s
                d += f'M{c + h} {y}A{h} {h} 0 1 0 {c - h} {y}A{h} {h} 0 1 0 {c + h} {y}Z'
        if kind == 'chevrons':
            for j in range(-n, n + 1):
                y = j * s
                d += f'M{c - h} {y}L{c + h} {y}L{c - s / 2 + h} {y + s / 2}L{c + h} {y + s}L{c - h} {y + s}L{c - s / 2 - h} {y + s / 2}Z'
    return d


PATTERNS = [
    ('stripes', 'rect', (70, 80), 12, 0.35, 45, SKY, 0.8, SURFACE),
    ('dots', 'rect', (200, 80), 14, 0.45, 0, CORAL, 1, SURFACE),
    ('hatch', 'rect', (330, 80), 16, 0.2, 30, SLATE, 0.5, SURFACE),
    ('chevrons', 'rect', (460, 80), 18, 0.3, 0, TEAL, 1, SURFACE),
    ('stripes', 'circle', (580, 80), 10, 0.5, -30, SURFACE, 0.6, SUN),
]


def patterns():
    defs = ''
    body = ''
    for i, (kind, form, (cx, cy), s, w, deg, colour, alpha, base) in enumerate(PATTERNS):
        geo = '<rect x="-60" y="-60" width="120" height="120" rx="20"' if form == 'rect' else '<circle cx="0" cy="0" r="50"'
        defs += f'<clipPath id="c{i}">{geo}/></clipPath>'
        body += (f'<g transform="translate({cx},{cy})">{geo} fill="{base}"/>'
                 f'<g clip-path="url(#c{i})"><path transform="rotate({deg})" d="{pattern_geometry(kind, s, w)}" fill="{colour}" fill-opacity="{alpha}"/></g></g>')
    w, h = 640, 160
    return f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}"><defs>{defs}</defs><rect width="{w}" height="{h}" fill="{GROUND}"/>{body}</svg>'


def scenes():
    s = {}
    s['card'] = (320, 230, GROUND, tile(20, 40, 180, 60, CORAL, 0, depth=2) + tile(110, 125, 180, 60, PLUM, -2.6, depth=1))
    s['faded'] = (320, 200, GROUND, tile(20, 30, 180, 60, TEAL, -2, faded=True) + tile(60, 70, 180, 60, CORAL, 3, faded=True) + tile(40, 120, 180, 60, SUN, 0))
    s['sheet'] = (440, 720, SLATE,
                  f'<rect width="440" height="720" fill="{GROUND}"/><rect width="440" height="720" fill="{SLATE}" opacity=".55"/>'
                  f'<g filter="url(#lift2)"><rect x="50" y="50" width="340" height="600" rx="30" fill="#fff" stroke="{SKY}" stroke-width="6"/></g>'
                  f'<rect x="80" y="80" width="110" height="110" rx="28" fill="{SUN}"/><path d="M116 116l38 38M154 116l-38 38" stroke="#fff" stroke-width="12" stroke-linecap="round"/>')
    s['panel'] = (420, 300, SURFACE,
                  f'<rect x="24" y="20" width="372" height="120" rx="18" fill="#f5f7fa" stroke="{SKY}" stroke-width="3"/>'
                  f'<rect x="24" y="154" width="372" height="84" rx="18" fill="#f5f7fa" stroke="{LINE}" stroke-width="2"/>'
                  f'<rect x="34" y="258" width="262" height="14" rx="7" fill="#46516a"/><rect x="34" y="258" width="104" height="14" rx="7" fill="{SAGE}"/>')
    s['pill'] = (300, 320, GROUND,
                 f'<rect x="10" y="10" width="200" height="300" rx="24" fill="url(#blush)" stroke="{LINE}" stroke-width="2"/><rect x="226" y="10" width="64" height="300" rx="24" fill="url(#mist)" stroke="{LINE}" stroke-width="2"/>'
                 + chip(36, 40, 150, 52) + pill(26, 120, 170, 56) + f'<rect x="147" y="220" width="6" height="60" rx="3" fill="url(#fade)"/>')
    s['dashed'] = (440, 200, SURFACE,
                   f'<rect x="20" y="20" width="300" height="150" rx="30" fill="#f2f4f7" stroke="#c3cad4" stroke-width="4" stroke-dasharray="14 10"/>'
                   f'<path d="M170 66v58M141 95h58" stroke="#a7b0bd" stroke-width="10" stroke-linecap="round"/>'
                   f'<rect x="340" y="0" width="100" height="200" fill="{SLATE}"/>'
                   f'<rect x="352" y="14" width="76" height="170" rx="34" fill="#fff" opacity=".16" stroke="#fff" stroke-opacity=".5" stroke-width="3" stroke-dasharray="12 10"/>')
    s['arc'] = (360, 200, SURFACE,
                meter(80, 90, 44, 13, 230, SUN)
                + f'<circle cx="250" cy="90" r="72" fill="#fff" filter="url(#lift1)"/>' + meter(250, 90, 60, 12, 250, SKY))
    s['glow'] = (300, 300, SURFACE,
                 f'<circle cx="150" cy="150" r="92" fill="{INDIGO}" opacity=".25" filter="url(#halo)"/><circle cx="150" cy="150" r="74" fill="{INDIGO}" filter="url(#lift1)"/>')
    s['icons'] = (520, 260, GROUND,
                  '<use href="#ghost" transform="translate(25,31) scale(1)"/><use href="#pointer" transform="translate(20,20) scale(1)"/>'
                  '<use href="#ghost" transform="translate(95,31) scale(0.6)"/><use href="#pointer" transform="translate(90,20) scale(0.6)"/>'
                  + tick(160, 40, .8, 60)
                  + f'<circle cx="230" cy="50" r="28" fill="{SKY}"/><path d="M216 51l9 10l17 -20" stroke="#fff" stroke-width="7" fill="none" stroke-linecap="round" stroke-linejoin="round"/>'
                  + f'<rect x="280" y="20" width="60" height="60" rx="16" fill="{PLUM}"/><path d="M294 50h32M310 34v32" stroke="#fff" stroke-width="7" stroke-linecap="round"/>'
                  + f'<rect x="350" y="20" width="60" height="60" rx="16" fill="{SUN}"/><path d="M366 36l28 28M394 36l-28 28" stroke="#fff" stroke-width="7" stroke-linecap="round"/>'
                  + f'<rect x="420" y="20" width="60" height="60" rx="16" fill="{RAIL}"/><use href="#funnel" transform="translate(450,50) scale(1.3)"/>'
                  + f'<path d="M22 150h50M22 170h50M22 190h50" stroke="{SLATE}" stroke-width="8" stroke-linecap="round"/>'
                  + f'<circle cx="120" cy="162" r="9" fill="none" stroke="{MUTE}" stroke-width="4"/><path d="M126 169l8 8" stroke="{MUTE}" stroke-width="4" stroke-linecap="round"/>'
                  + f'<path d="M285 200h-50m0 0l18 -16m-18 16l18 16M325 200h50m0 0l-18 -16m18 16l-18 16" stroke="{SLATE}" stroke-width="8" fill="none" stroke-linecap="round" stroke-linejoin="round" opacity=".4"/>'
                  + page(150, 132, 56, SAGE, '#3f7a42')
                  + f'<rect x="410" y="110" width="100" height="100" rx="24" fill="{CORAL}"/><use href="#funnel" transform="translate(460,160) scale(1.6)"/>')
    s['pointer'] = (200, 200, GROUND, '<use href="#ghost" transform="translate(66,62) scale(1.6)"/><use href="#pointer" transform="translate(60,50) scale(1.6)"/>')
    s['scrim'] = (400, 240, GROUND,
                  f'<rect x="20" y="20" width="160" height="200" rx="28" fill="#fff"/><circle cx="270" cy="90" r="58" fill="{SKY}"/><rect x="200" y="160" width="180" height="50" rx="25" fill="{CORAL}"/>'
                  f'<rect x="0" width="400" height="240" fill="{SLATE}" opacity=".6"/>'
                  f'<rect x="100" y="100" width="200" height="40" rx="20" fill="#fff"/>'
                  f'<rect x="200" width="200" height="120" fill="#fff" opacity=".7"/>')
    s['board'] = (1920, 1080, GROUND, board())
    return s


def render(page_html, w, h, out, scale, browser):
    html = f'<html><head><style>html,body{{margin:0;background:#000}} svg{{display:block}}</style></head><body>{page_html}</body></html>'
    with sync_playwright() as p:
        b = p.chromium.launch(executable_path=browser) if browser else p.chromium.launch()
        pg = b.new_page(viewport={'width': w, 'height': h}, device_scale_factor=scale)
        pg.set_content(html)
        pg.wait_for_timeout(200)
        pg.screenshot(path=out)
        b.close()


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.dirname(os.path.abspath(__file__))
    browser = os.environ.get('FLAT_CHROMIUM')
    all_scenes = {name: (w, h, svg(w, h, ground, body)) for name, (w, h, ground, body) in scenes().items()}
    all_scenes['patterns'] = (640, 160, patterns())
    for name, (w, h, source) in all_scenes.items():
        with open(os.path.join(out, f'{name}.svg'), 'w') as fh:
            fh.write(source)
        for k in (1, 2):
            render(source, w, h, os.path.join(out, f'{name}@{k}x.png'), k, browser)
        print('wrote', name)


if __name__ == '__main__':
    main()
