"""Build the illustrated terminal figures, responsive quick guide and two-page A4 PDF.

uv run --no-project --with reportlab --with svglib --with qrcode python artwork/build_quick_start.py
Font: set VOLOO_DOC_FONT to an embeddable Chinese TrueType font when needed.
"""
from pathlib import Path
from math import sin, cos, pi
from html import escape
import json
import os
import shutil
import qrcode
import qrcode.image.svg

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / 'site/assets/guide'
ASSETS.mkdir(parents=True, exist_ok=True)
WEBSITE = 'https://voloo.librezoo.com/'
qr = qrcode.QRCode(error_correction=qrcode.constants.ERROR_CORRECT_M, border=4)
qr.add_data(WEBSITE)
qr.make(fit=True)
qr.make_image(image_factory=qrcode.image.svg.SvgPathImage).save(ROOT/'poster/user-guide-qr-main.svg')
INK, PAPER, PURPLE = '#2d1a2b', '#fffdf8', '#5e43b7'
STEPS = [
    ('剥线 11 mm', '未处理导线用剥线钳剥线 11 mm。\n预制端头先匹配，线芯齐整不散。'),
    ('按住橙色件', '按住对应的橙色按压件，方向朝电路板，夹口随之打开。'),
    ('插入绿色圆孔', '持续按住，把导线插入相邻的绿色圆孔；铜芯完全进入夹持区。'),
    ('松开，轻拉确认', '松开橙色件后轻拉导线，确认夹紧；孔外不露铜。'),
]

# Figure: the actual connector is the explanation, not a generic mechanism.
# Visible features: three housing faces, two pole seams, two three-band circular
# entries, two release buttons with top/recess/leg/lip faces, two rectangular slots,
# locating features, lower pin stubs, and one insulated lead. Each stays attached
# to the same pole and surface in every illustrated pose. Colours have meaning.
def at(x, y, z):
    return (190 + (x-y)*8.5, 205 + (x+y)*3.2-z*8.8)

def points(vs):
    return ' '.join(f'{x:.2f},{y:.2f}' for x,y in vs)

def poly(vs, fill, stroke='#344d43', width=.7):
    return f'<polygon points="{points(vs)}" fill="{fill}" stroke="{stroke}" stroke-width="{width}" stroke-linejoin="round"/>'

def box(x,y,z,w,d,h,colors):
    p=lambda a,b,c:at(a,b,c)
    return ''.join([
        poly([p(x+w,y,z),p(x+w,y+d,z),p(x+w,y+d,z+h),p(x+w,y,z+h)],colors[2]),
        poly([p(x,y+d,z),p(x+w,y+d,z),p(x+w,y+d,z+h),p(x,y+d,z+h)],colors[1]),
        poly([p(x,y,z+h),p(x+w,y,z+h),p(x+w,y+d,z+h),p(x,y+d,z+h)],colors[0]),
    ])

def disc(x,y,z,r,fill):
    return poly([at(x+r*cos(i*2*pi/48),y+r*sin(i*2*pi/48),z) for i in range(48)],fill,'none')

def arrow(x,y,dx,dy,color=PURPLE):
    ex,ey=x+dx,y+dy
    length=(dx*dx+dy*dy)**.5;ux,uy=dx/length,dy/length;bx,by=ex-ux*12,ey-uy*12
    return f'<path d="M{x:.1f} {y:.1f}L{ex:.1f} {ey:.1f}" stroke="{color}" stroke-width="4" fill="none" stroke-linecap="round"/>'+poly([(ex,ey),(bx-uy*6,by+ux*6),(bx+uy*6,by-ux*6)],color,'none')

def illustration(phase):
    view='20 100 310 100' if phase==0 else '50 65 255 220'
    out=['<svg xmlns="http://www.w3.org/2000/svg" viewBox="{view}" role="img" aria-label="按压式弹簧端子操作示意"><defs><marker id="arrow" markerWidth="8" markerHeight="8" refX="6" refY="3" orient="auto" markerUnits="strokeWidth"><path d="M0 0L0 6L6 3Z" fill="'+PURPLE+'"/></marker><linearGradient id="wire" x1="0" x2="1"><stop stop-color="#313035"/><stop offset=".45" stop-color="#747177"/><stop offset="1" stop-color="#353239"/></linearGradient></defs>']
    if phase==0:
        out += ['<ellipse cx="176" cy="184" rx="130" ry="12" fill="#e8e1e5"/>',
                '<path d="M38 145H194" stroke="#333037" stroke-width="30" stroke-linecap="round"/>',
                '<path d="M41 137H190" stroke="#6d6870" stroke-width="5" stroke-linecap="round"/>',
                '<path d="M194 145H306" stroke="#a76936" stroke-width="17"/>',
                '<path d="M195 140H305M195 145H305M195 150H305" stroke="#e3b274" stroke-width="2"/>',
                '<ellipse cx="306" cy="145" rx="3" ry="8.5" fill="#bc854d"/>']
    else:
        pressed=phase in (1,2,4)
        out.append('<ellipse cx="172" cy="242" rx="115" ry="33" fill="#e8e1e5"/>')
        # Small portions of the real solder pins identify a fixed terminal, not a plug.
        for x in (2.54,7.62):
            for y in (2.0,8.5):
                out.append(box(x-.35,y-.5,-1.8,.7,1,2.0,('#d9dde0','#929b9e','#b2b8bb')))
        out.append(box(0,0,0,10.16,10.2,12.8,('#76b898','#478166','#5d9b7b')))
        for x in (2.54,7.62):
            out.append(box(x-1.8,5.3,12.8,3.6,.8,.03,('#36684f','#36684f','#36684f')))
            out += [disc(x,8.0,12.84,1.62,'#9acdb0'),disc(x,8.0,12.86,1.35,'#447b5d'),disc(x,8.0,12.88,1.02,'#172f24')]
        # Pole separation seam and the moulded alignment feature.
        out.append(poly([at(5.08,0,12.85),at(5.08,4.6,12.85),at(5.13,4.6,12.85),at(5.13,0,12.85)],'#548e70','none'))
        for pole,x in enumerate((2.54,7.62)):
            z=11.8 if pressed and pole==0 else 12.9
            out.append(box(x-2.1,10.3,1.0,4.2,2.2,z-1,('#ef954b','#c96628','#dd7b34')))
            out.append(box(x-2.1,10.3,z,4.2,3.4,.7,('#ffad65','#d77731','#ee9147')))
            out.append(box(x-1.55,10.9,z+.71,3.1,1.8,.02,('#d47b37','#d47b37','#d47b37')))
            out.append(box(x-1.2,11.2,z+.74,2.4,1.15,.02,('#e9954c','#e9954c','#e9954c')))
        # One pole is used to illustrate the operation; no fixed polarity is implied.
        hx,hy=at(2.54,8.0,12.9)
        if phase in (2,3,4):
            out.append(f'<path d="M{hx:.2f} {hy-45:.2f}L{hx:.2f} {hy:.2f}" stroke="#252329" stroke-width="15" stroke-linecap="round"/>')
            out.append(f'<path d="M{hx-2.5:.2f} {hy-45:.2f}L{hx-2.5:.2f} {hy-2:.2f}" stroke="#77717b" stroke-width="3" stroke-linecap="round"/>')
        bx,by=at(2.54,12.0,13.6 if not pressed else 12.5)
        if phase in (1,4):out.append(arrow(bx,by-62,0,56))
        if phase==2:out.append(arrow(hx+24,hy-48,-18,33))
        if phase==3:out.append(arrow(hx+28,hy-22,0,-30))
        if phase==4:out.append(arrow(hx+28,hy-22,0,-30))
    return (''.join(out)+'</svg>').replace('{view}',view)

for phase in range(5):
    (ASSETS/f'terminal-step-{phase}.svg').write_text(illustration(phase))
shutil.copy2(ROOT/'poster/user-guide-qr-main.svg', ASSETS/'user-guide-qr.svg')

cards=''.join(f'<article class="step"><img src="assets/guide/terminal-step-{i}.svg" alt="{escape(t)}操作示意"><div><h3><span>{i+1}</span>{t}</h3><p>{b}</p></div></article>' for i,(t,b) in enumerate(STEPS))
css='''*{box-sizing:border-box}html{scroll-behavior:smooth}body{margin:0;background:#f5f1e9;color:#2d1a2b;font:17px/1.8 -apple-system,"PingFang SC",sans-serif}main{max-width:1080px;margin:auto;padding:28px 24px 64px}header{display:flex;justify-content:space-between;align-items:center;gap:20px;border-bottom:1px solid #d8cdd5;padding-bottom:18px}header img{width:244px;height:50px}header a{font-size:14px}h1{font-size:40px;line-height:1.3;letter-spacing:-.03em;margin:36px 0 12px}h2{font-size:27px;margin:38px 0 20px}h3{margin:0;font-size:20px;line-height:1.5}p{margin:8px 0}a{color:#5e43b7;text-underline-offset:4px}.intro{color:#65545f;font-size:19px}.specs{display:flex;flex-wrap:wrap;gap:12px;margin:22px 0}.specs b{padding:10px 18px;border:1px solid #d8cdd5;border-radius:12px;background:#fffdf8;font-size:20px}.warn{background:#fff0be;border-left:5px solid #fec600;padding:18px 22px;border-radius:0 14px 14px 0}.warn b{font-size:19px}.hero{display:grid;grid-template-columns:1fr 1fr;gap:28px;align-items:center;background:#fffdf8;border-radius:22px;padding:24px}.hero>img{width:100%;height:auto}.connections{display:grid;gap:15px}.connections article{border-left:4px solid #5e43b7;padding-left:16px}.connections h3{font-size:22px}.connections p{font-size:16px;color:#65545f}.key{display:flex;gap:10px;flex-wrap:wrap;margin:12px 0}.pos,.neg{color:#fffdf8;padding:2px 10px;border-radius:7px;font-weight:750}.pos{background:#b92c32}.neg{background:#252026}.steps{display:grid;grid-template-columns:1fr 1fr;gap:18px}.step{background:#fffdf8;padding:20px;border:1px solid #ddd2da;border-radius:18px}.step img{width:100%;height:190px;object-fit:contain}.step h3 span{display:inline-grid;place-items:center;background:#fec600;border-radius:7px;width:28px;height:28px;font-size:17px;margin-right:9px}.step p{font-size:16px;color:#65545f}.remove{display:grid;grid-template-columns:180px 1fr;align-items:center;gap:22px;background:#fffdf8;border-radius:18px;padding:18px;margin-top:20px}.remove img{width:180px}.code{background:#241622;color:#fffdf8;border-radius:14px;padding:18px 20px;margin:12px 0;display:flex;justify-content:space-between;gap:15px;align-items:center}.code code{font:15px/1.7 ui-monospace,monospace;overflow-wrap:anywhere}.code button{background:transparent;color:inherit;border:1px solid #8b7b88;border-radius:9px;padding:9px 13px;cursor:pointer;flex:none}ol{padding-left:24px}li+li{margin-top:12px}.faq{display:grid;gap:14px}.faq article{padding:16px 20px;background:#fffdf8;border:1px solid #ddd2da;border-radius:14px}.faq h3{font-size:18px}footer{margin-top:38px;padding-top:20px;border-top:1px solid #d8cdd5;font-size:14px;color:#65545f}.links{display:flex;gap:18px;flex-wrap:wrap}small{font-size:13px;color:#65545f}.download{display:inline-block;background:#fec600;padding:10px 18px;border-radius:12px;font-weight:700;text-decoration:none;color:#2d1a2b;margin-top:14px}@media(max-width:650px){main{padding:18px 18px 44px}header{flex-wrap:wrap;gap:12px}h1{font-size:31px}.hero{grid-template-columns:1fr;padding:18px}.hero>img{max-width:420px;margin:auto}.steps{grid-template-columns:1fr}.step{display:grid;grid-template-columns:116px 1fr;gap:10px;padding:15px}.step img{height:135px}.step h3{font-size:18px}.step p{font-size:15px}.remove{grid-template-columns:105px 1fr;gap:12px}.remove img{width:105px}.code code{font-size:12px}.code{padding:14px}.specs b{font-size:17px}.intro{font-size:17px}}'''
html=f'''<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><link rel="icon" href="data:,"><title>voloo 快速开始</title><style>{css}</style></head><body><main><header><img src="assets/brand/librezoo-voloo-compact.svg" alt="librezoo · voloo"><a href="index.html">返回产品介绍</a></header><h1>接好三处，再打开仪表盘。</h1><p class="intro">voloo 快速开始 · USB-C 连接版</p><div class="specs"><b>3.3–28 V DC</b><b>最高建议 3 A</b></div><div class="warn"><b>上电前先确认</b><p>正负极接反会损坏硬件；超过 3 A 可能损坏硬件，请设置外部电源限流。设备与 USB 操作电脑分别使用独立电源，同一多口适配器的不同输出也不行。</p></div><a class="download" href="assets/guide/voloo-quick-start-zh.pdf" download>下载两页 A4 快速说明书</a><p class="links"><a href="#connect">认清连接</a><a href="#terminal">端子接线</a><a href="#software">首次控制</a></p><h2 id="connect">01 认清三处连接</h2><div class="hero"><img src="assets/hero.png" alt="voloo 红正黑负接线示意，AI 辅助产品效果图"><div class="connections"><article><h3>独立电源 A → IN / SOURCE</h3><p>电压、限流须匹配设备；voloo 不调压。未连接 voloo 时先量清电源极性，再关闭输出接线。</p></article><article><h3>OUT / DUT → 设备</h3><p>设备只从 OUT 取电，断开自带 USB、电池或调试器等其他供电。</p></article><article><h3>电脑（电源 B）→ USB-C</h3><p>使用支持数据传输的 USB 线；电脑由另一独立电源 B 供电。</p></article><div class="key"><span class="pos">红线接＋</span><span class="neg">黑线接−</span></div><p>颜色只作辅助，接线以实物 IN / OUT 和＋／−标记及电源实际极性为准。</p></div></div><h2 id="terminal">02 导线怎么装进端子</h2><div class="warn"><b>只在断电后接线、换线或取线。</b><p>如设备仍有残压，先保持 USB 连接，执行泄放并确认 VOUT ≤ 1 V；再关闭并断开外部电源、拔下 USB-C，确认输入与设备端均无电后操作导线。</p></div><div class="steps">{cards}</div><p><b>线进绿色圆孔；橙色件只用来开夹，不是插线孔。</b>端子固定在 voloo 上，无需拧螺丝或拆下整个端子。</p><div class="remove"><img src="assets/guide/terminal-step-4.svg" alt="断电后按住橙色件，沿进线孔方向取线"><div><h3>需要取线？</h3><p>先按上面的方式断电，按住对应橙色件，再沿进线孔方向取出。不要直接硬拽，也不要撬动按压件。</p></div></div><small>动作图为端子局部示意，不表示固定的正负极位置。剥线 11 mm 来自 KEFA KF142V-5.08 图纸；元件规格不改变 voloo 整机的电压、电流限制。</small><h2 id="software">03 安装软件，完成第一次控制</h2><p>电脑先从 <a href="https://nodejs.org/">nodejs.org</a> 安装 Node.js（推荐当前 LTS，最低 18），再在终端执行：</p><div class="code"><code>npm install -g @librezoo/voloo</code><button data-copy="npm install -g @librezoo/voloo">复制</button></div><div class="code"><code>voloo ui</code><button data-copy="voloo ui">复制</button></div><ol><li><b>核对设备。</b>浏览器会打开本机仪表盘，首次只连接目标这台 voloo，确认它出现在仪表盘且输出为 OFF。可用 <code>voloo ls</code> 在终端查询。</li><li><b>长按开关。</b>核对接线、标称电压、限流和输出 OFF 状态，再打开独立直流电源。确认设备允许断电后，长按开关控制供电；短按不切换。</li><li><b>需要清掉残压时用「泄放」。</b>确认 VOUT ≤ 1 V 后才算完成；结果未知时先重新检查状态，不盲目重复指令。</li></ol><p>没有硬件可先运行 <code>voloo demo</code>。接入 AI Agent、脚本或 HTTP API 的详细方法见完整说明。</p><h2>遇到问题，先看这里</h2><div class="faq"><article><h3>没有出现设备</h3><p>换一根支持数据传输的 USB 线，检查连接，再运行 <code>voloo doctor</code>。Linux 可能需要按提示设置串口权限。</p></article><article><h3>关断后仍有电压</h3><p>OFF 不等于 0 V。用泄放确认输出 ≤ 1 V，并排查设备是否还有其他供电。</p></article><article><h3>过压后输出仍关闭</h3><p>过压锁定解除不会自动重新供电。回到 3.3–28 V 并检查接线、限流后，重新发出开启指令；不需要额外过压复位命令。</p></article></div><footer><div class="links"><a href="guide.html">完整中文说明</a><a href="https://github.com/librezoo/voloo">上位机开源仓库</a><a href="https://www.lcsc.com/datasheet/C475134.pdf">端子厂家图纸</a></div><p>产品外观为 AI 辅助效果渲染；操作图为结构示意。实际接线以实物标识为准。</p></footer></main><script>document.addEventListener('click',async e=>{{const b=e.target.closest('[data-copy]');if(!b)return;try{{await navigator.clipboard.writeText(b.dataset.copy);b.textContent='已复制';}}catch{{b.textContent='请手动复制';}}setTimeout(()=>b.textContent='复制',1800);}});</script></body></html>'''
(ROOT/'site/quick-start.html').write_text(html)

# A focused figure sheet is also used in the complete Markdown manual.
sheet=f'''<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><title>voloo 端子操作图</title><style>{css}body{{width:1200px;background:#fffdf8}}main{{max-width:none;padding:36px}}.steps{{grid-template-columns:repeat(4,1fr);gap:14px}}.step{{padding:14px}}.step img{{height:175px}}h1{{font-size:34px;margin:0 0 12px}}.step h3{{font-size:20px}}.step p{{font-size:17px;line-height:1.65}}.note{{font-size:20px;margin-top:22px}}</style></head><body><main id="terminal-figure"><h1>端子接线：按住橙色件，线进绿色圆孔</h1><p class="note">先确认断电，再接线或取线；图示为单孔操作，不表示固定极性。</p><div class="steps">{cards.replace('assets/guide/','../site/assets/guide/')}</div><p class="note"><b>取线：</b>断电后，按住对应橙色件，再沿进线孔方向取出；不要硬拽或撬动。</p><p class="note">剥线 11 mm 依据 KEFA KF142V-5.08 图纸。整机最高建议 3 A；超过可能损坏硬件。</p></main></body></html>'''
(ROOT/'artwork/terminal-steps.html').write_text(sheet)

def build_pdf():
    from reportlab.pdfgen import canvas
    from reportlab.lib.colors import HexColor
    from reportlab.lib.pagesizes import A4
    from reportlab.pdfbase import pdfmetrics
    from reportlab.pdfbase.ttfonts import TTFont
    from reportlab.platypus import Paragraph
    from reportlab.lib.styles import ParagraphStyle
    from svglib.svglib import svg2rlg
    from reportlab.graphics import renderPDF

    font=os.environ.get('VOLOO_DOC_FONT','/System/Library/Fonts/Supplemental/Arial Unicode.ttf')
    pdfmetrics.registerFont(TTFont('GuideCJK',font))
    target=ROOT.parent/'output/pdf/voloo-quick-start-zh.pdf';target.parent.mkdir(parents=True,exist_ok=True)
    c=canvas.Canvas(str(target),pagesize=A4);W,H=A4;M=38;CW=W-2*M
    c.setTitle('voloo 快速使用说明');c.setAuthor('librezoo');c.setCreator('librezoo / voloo')
    def text(x,y,t,size=11,color=INK,bold=False,font='GuideCJK'):
        c.saveState();c.setFillColor(HexColor(color));obj=c.beginText(x,y);obj.setFont(font,size);obj.setTextRenderMode(0)
        if bold:obj.setTextRenderMode(2);c.setStrokeColor(HexColor(color));c.setLineWidth(.14)
        obj.textOut(t);c.drawText(obj);c.restoreState()
    def para(x,top,w,t,size=10.5,color=INK,leading=None):
        st=ParagraphStyle('body',fontName='GuideCJK',fontSize=size,leading=leading or size*1.5,textColor=HexColor(color),wordWrap='CJK')
        p=Paragraph(escape(t).replace('\n','<br/>'),st);_,h=p.wrap(w,1000);p.drawOn(c,x,top-h);return top-h
    def box(x,y,w,h,fill=PAPER,stroke='#e0d6dd',r=10):
        c.setFillColor(HexColor(fill));c.setStrokeColor(HexColor(stroke));c.setLineWidth(.6);c.roundRect(x,y,w,h,r,fill=1,stroke=1)
    def img(path,x,y,w,h):c.drawImage(str(path),x,y,width=w,height=h,preserveAspectRatio=True,anchor='c',mask='auto')
    def svg(path,x,y,w,h):
        d=svg2rlg(str(path));k=min(w/d.width,h/d.height);c.saveState();c.translate(x+(w-d.width*k)/2,y+(h-d.height*k)/2);c.scale(k,k);renderPDF.draw(d,c,0,0);c.restoreState()
    def header(page,label):
        img(ROOT/'branding/png/librezoo-voloo-compact.png',M,H-74,190,39)
        text(W-M-116,H-45,'快速使用说明',12,bold=True);text(W-M-116,H-64,f'{page:02d}  {label}',9,'#65545f')
        c.setStrokeColor(HexColor('#ded5dc'));c.line(M,H-87,W-M,H-87)
    def footer(page):
        text(M,30,'USB-C 连接版 voloo  ·  2026-11-15',8,'#65545f');text(W-M-25,30,f'{page}/2',8,'#65545f')

    header(1,'连接与首次使用')
    text(M, H-123,'接好三处，再打开仪表盘。',23,bold=True)
    text(M,H-151,'3.3-28 V 直流   |   最高建议电流 3 A',15,bold=True)
    text(W-M-123,H-151,'禁止接交流市电',9,'#65545f')
    para(M,H-165,CW,'voloo 不调压。电源电压须匹配设备，并设置合适限流；超过 3 A 可能损坏硬件。',10,'#65545f')
    img(ROOT/'site/assets/hero.png',M-7,335,270,222)
    x=M+285
    for y,title,body in [(540,'1  电源 A → IN / SOURCE','设置与设备匹配的电压和限流。\n用万用表核对极性，关闭输出后接线。'),(475,'2  OUT / DUT → 设备','设备只从 OUT 取电，断开它的其他供电。'),(410,'3  电脑 → USB-C','使用 USB 数据线；电脑由另一独立电源 B 供电。')]:
        text(x,y,title,12,bold=True);para(x,y-12,CW-285,body,10)
    box(M,292,CW,34,fill='#f5f1e9');text(M+12,304,'红线接 ＋',12,'#b92c32',True);text(M+113,304,'黑线接 −',12,'#252026',True)
    text(M+221,304,'线色辅助，核对实物标记',10,'#65545f')
    box(M,578,CW,72,fill='#fff0be',stroke='#ead792')
    text(M+13,629,'接反会损坏硬件',12,bold=True)
    para(M+13,619,225,'在未连接 voloo 时测清电源极性，关闭输出后再按实物＋／−标记接线。',10)
    text(M+274,629,'设备与电脑分别供电',12,bold=True)
    para(M+274,619,CW-287,'不要共用同一适配器；同一多口适配器的不同输出也不行。',10)
    text(M,270,'安装并打开',15,bold=True)
    text(M+119,270,'Node.js 18+  ·  nodejs.org',9,'#65545f')
    box(M,215,CW,42,fill='#241622',stroke='#241622')
    text(M+14,231,'npm install -g @librezoo/voloo',11,'#fffdf8',font='Courier')
    box(M,173,CW,33,fill='#f5f1e9');text(M+14,185,'voloo ui',11,font='Courier')
    text(M,150,'完成第一次控制',14,bold=True)
    para(M,138,CW-75,'1. 首次只接目标这台 voloo，确认它出现在仪表盘且输出为 OFF。',10.5)
    para(M,114,CW-75,'2. 核对接线、标称电压和限流后，再打开独立直流电源输出。',10.5)
    para(M,90,CW-75,'3. 确认设备允许断电后，在仪表盘长按开关；泄放需确认 VOUT ≤ 1 V。',10.5)
    svg(ASSETS/'user-guide-qr.svg',W-M-58,59,58,58)
    c.linkURL(WEBSITE,(W-M-58,59,W-M,117),relative=0,thickness=0)
    text(W-M-58,48,'扫码访问官网',7.5,'#65545f')
    text(M,47,'产品外观为 AI 辅助效果图，操作图为结构示意。',7.5,'#65545f')
    footer(1);c.showPage()

    header(2,'端子接线与排错')
    text(M,H-122,'按住橙色件，线进绿色圆孔。',22,bold=True)
    para(M,H-135,CW,'橙色件负责夹线，不是电源开关；通断电在电脑仪表盘里操作。',10.5,'#65545f')
    box(M,625,CW,54,fill='#fff0be',stroke='#ead792')
    para(M+12,668,CW-24,'只在断电后接拔线。如设备有残压，先保持 USB 连接执行泄放并确认 VOUT ≤ 1 V；再关闭并断开外部电源、拔下 USB-C，确认两端无电后操作。',9.8)
    gap=14;w=(CW-gap)/2;card_h=178
    for i,(title,body) in enumerate(STEPS):
        x=M+(i%2)*(w+gap);y=431-(i//2)*(card_h+12)
        box(x,y,w,card_h)
        svg(ASSETS/f'terminal-step-{i}.svg',x+10,y+64,w-20,108)
        text(x+13,y+49,f'{i+1}  {title}',11.5,bold=True)
        para(x+13,y+38,w-26,body,9,leading=12)
    box(M,164,CW,69,fill='#f5f1e9')
    svg(ASSETS/'terminal-step-4.svg',M+5,164,105,69)
    text(M+117,214,'取线：断电后，按住，再取出',11.5,bold=True)
    para(M+117,202,CW-131,'按住对应橙色件，再沿进线孔方向取出。不要直接硬拽，也不要撬动按压件。橙色开口不是插线孔。',9.5)
    text(M,143,'遇到问题',12,bold=True)
    for y,title,body in [(124,'没有设备','换 USB 数据线，检查连接，运行 voloo doctor。'),(97,'OFF 后仍有电压','使用泄放确认 ≤ 1 V，并排查设备的其他供电。'),(70,'异常／过压','先查状态，勿盲重试；过压后回到标称范围，再明确开启。')]:
        text(M,y,title,9.5,bold=True);para(M+113,y+10,CW-113,body,9.5)
    text(M,54,'剥线 11 mm：KEFA KF142V-5.08 厂家图纸（立创 C475134）。示意不按比例，不表示固定极性。',7.5,'#65545f')
    footer(2);c.save()
    shutil.copy2(target,ASSETS/'voloo-quick-start-zh.pdf')
    print('PDF:',target)

if __name__=='__main__':
    build_pdf()
    print('Built quick-start HTML, five terminal illustrations and two-page PDF.')
