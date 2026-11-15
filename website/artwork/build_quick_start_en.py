"""Render the English two-page A4 quick start. No archive output.
uv run --with reportlab --with svglib python website/artwork/build_quick_start_en.py
"""
from pathlib import Path
from html import escape
import shutil
from reportlab.pdfgen import canvas
from reportlab.lib.colors import HexColor
from reportlab.lib.pagesizes import A4
from reportlab.platypus import Paragraph
from reportlab.lib.styles import ParagraphStyle
from svglib.svglib import svg2rlg
from reportlab.graphics import renderPDF
ROOT=Path(__file__).resolve().parent.parent
ASSETS=ROOT/'site/assets/guide'
TARGET=ROOT.parent/'output/pdf/voloo-quick-start-en.pdf';TARGET.parent.mkdir(parents=True,exist_ok=True)
W,H=A4;M=38;CW=W-2*M;INK='#2d1a2b'
c=canvas.Canvas(str(TARGET),pagesize=A4);c.setTitle('voloo Quick Start');c.setAuthor('librezoo');c.setCreator('librezoo / voloo')
def text(x,y,t,size=11,color=INK,bold=False,font=None):
 c.saveState();c.setFillColor(HexColor(color));c.setFont(font or ('Helvetica-Bold' if bold else 'Helvetica'),size);c.drawString(x,y,t);c.restoreState()
def para(x,top,w,t,size=10,color=INK,leading=None):
 p=Paragraph(escape(t).replace('\n','<br/>'),ParagraphStyle('p',fontName='Helvetica',fontSize=size,leading=leading or size*1.4,textColor=HexColor(color)))
 _,h=p.wrap(w,1000);p.drawOn(c,x,top-h);return top-h

def box(x,y,w,h,fill='#fffdf8',stroke='#e0d6dd'):
 c.setFillColor(HexColor(fill));c.setStrokeColor(HexColor(stroke));c.setLineWidth(.6);c.roundRect(x,y,w,h,10,fill=1,stroke=1)
def svg(name,x,y,w,h):
 d=svg2rlg(str(ASSETS/name));k=min(w/d.width,h/d.height);c.saveState();c.translate(x+(w-d.width*k)/2,y+(h-d.height*k)/2);c.scale(k,k);renderPDF.draw(d,c,0,0);c.restoreState()
def header(page,label):
 c.drawImage(str(ROOT/'branding/png/librezoo-voloo-compact.png'),M,H-74,190,39,preserveAspectRatio=True,mask='auto')
 text(W-M-141,H-45,'Quick start',12,bold=True);text(W-M-141,H-64,f'{page:02d}  {label}',9,'#65545f')
 c.setStrokeColor(HexColor('#ded5dc'));c.line(M,H-87,W-M,H-87)
def footer(page):
 text(M,30,'voloo USB-C edition  |  2026-11-15',8,'#65545f');text(W-M-25,30,f'{page}/2',8,'#65545f')
header(1,'Connect and get started')
text(M,H-123,'Three connections. Then take control.',23,bold=True)
text(M,H-151,'3.3-28 V DC  |  3 A maximum recommended',13,bold=True)
text(W-M-128,H-151,'DC only. Never mains AC.',9,'#65545f')
para(M,H-165,CW,'voloo does not regulate voltage. Match the device voltage and set an external current limit. Exceeding 3 A can damage the hardware.',9.5,'#65545f')
box(M,562,CW,88,fill='#fff0be',stroke='#ead792')
text(M+13,632,'Reversed polarity damages hardware',10.5,bold=True)
para(M+13,620,225,'Measure supply polarity before connecting voloo. Turn the supply OFF, then wire to the physical + / - markings.',9.5)
text(M+274,632,'Use independent supplies',10.5,bold=True)
para(M+274,620,CW-287,'Separate supplies for the device and USB host computer. Different ports on one multiport adapter do not qualify.',9.5)
c.drawImage(str(ROOT/'site/assets/hero.png'),M-7,335,270,222,preserveAspectRatio=True,anchor='c',mask='auto')
x=M+285
for y,title,body in [(540,'1  Supply A to IN / SOURCE','Set voltage and current limits for the device. Verify polarity, then switch the supply OFF before wiring.'),(473,'2  OUT / DUT to the device','Power it only from OUT. Disconnect its other USB, battery and debugger power.'),(405,'3  Computer to USB-C','Use a USB data cable. Power the computer from independent supply B.')]:
 text(x,y,title,11,bold=True);para(x,y-12,CW-285,body,9.5)
box(M,292,CW,34,fill='#f5f1e9');text(M+12,304,'Red to +',11,'#b92c32',True);text(M+113,304,'Black to -',11,'#252026',True);text(M+221,304,'Color is an aid. Verify physical labels.',9,'#65545f')
text(M,270,'Install and open',15,bold=True);text(M+145,270,'Node.js 18+  |  nodejs.org',9,'#65545f')
box(M,215,CW,42,fill='#241622',stroke='#241622');text(M+14,231,'npm install -g @librezoo/voloo',11,'#fffdf8',font='Courier')
box(M,173,CW,33,fill='#f5f1e9');text(M+14,185,'voloo ui',11,font='Courier')
text(M,150,'Your first control',14,bold=True)
para(M,138,CW-84,'1. Connect only the target voloo. Confirm it appears in the dashboard and its output is OFF.',9.5)
para(M,110,CW-84,'2. Check wiring, voltage and current limits, then enable the independent DC supply.',9.5)
para(M,83,CW-84,'3. Confirm the device can safely lose power, then hold the dashboard switch. Discharge requires confirmed VOUT <= 1 V.',9.5)
svg('user-guide-qr.svg',W-M-58,59,58,58);c.linkURL('https://voloo.librezoo.com/',(W-M-58,59,W-M,117),relative=0,thickness=0)
text(W-M-58,48,'Scan for website',7.5,'#65545f');text(M,47,'AI-assisted product rendering. Operation figures are schematic.',7.5,'#65545f');footer(1);c.showPage()
header(2,'Terminals and troubleshooting')
text(M,H-122,'Press orange. Wire into green.',23,bold=True)
para(M,H-135,CW,'The orange actuator opens the wire clamp. It is not the power switch; use the dashboard to control power.',10,'#65545f')
box(M,615,CW,64,fill='#fff0be',stroke='#ead792')
para(M+12,668,CW-24,'Wire only when unpowered. If residual voltage remains, keep USB connected and discharge until VOUT <= 1 V is confirmed. Then switch off and disconnect the external supply, unplug USB-C, and verify both ends are unpowered before handling wires.',9.5)
steps=[('Strip 11 mm','Strip untreated wire 11 mm. Check prepared ends fit; keep strands together.'),('Press orange','Hold the matching orange actuator toward the circuit board to open the clamp.'),('Insert into green','Keep pressing. Fully insert the conductor into the adjacent green round hole.'),('Release and check','Release, then gently tug to check the grip. No bare copper outside the hole.')]
gap=14;w=(CW-gap)/2;card_h=178
for i,(title,body) in enumerate(steps):
 x=M+(i%2)*(w+gap);y=426-(i//2)*(card_h+12);box(x,y,w,card_h);svg(f'terminal-step-{i}.svg',x+10,y+64,w-20,108);text(x+13,y+49,f'{i+1}  {title}',11.5,bold=True);para(x+13,y+38,w-26,body,9,leading=12)
box(M,159,CW,69,fill='#f5f1e9');svg('terminal-step-4.svg',M+5,159,105,69)
text(M+117,209,'Remove: unpower, hold, withdraw',11,bold=True)
para(M+117,197,CW-131,'Hold the matching orange actuator and withdraw along the entry direction. Do not yank or pry. Orange openings are not wire entries.',9.5)
text(M,140,'Troubleshooting',12,bold=True)
for y,title,body in [(120,'Device missing','Try a USB data cable, check the connection, run voloo doctor.'),(93,'Voltage after OFF','OFF is not 0 V. Discharge to <= 1 V; check other power sources.'),(66,'Unknown / overvoltage','Read state first. Return to the nominal range, then explicitly turn ON.')]:
 text(M,y,title,9,bold=True);para(M+110,y+10,CW-110,body,9)
text(M,47,'11 mm strip length: KEFA KF142V-5.08 drawing, LCSC C475134. Diagrams are not to scale or fixed polarity.',7,'#65545f')
footer(2);c.save();shutil.copy2(TARGET,ASSETS/TARGET.name)
print(TARGET)
