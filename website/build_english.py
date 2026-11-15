"""Build English public pages from reviewed Chinese sources; no archives are produced.
Run with: uv run --with beautifulsoup4 python website/build_english.py
"""
from pathlib import Path
import json,re
from bs4 import BeautifulSoup,Comment
ROOT=Path(__file__).resolve().parent
TRANSLATIONS=json.loads((ROOT/'i18n/en.json').read_text())
HAN=re.compile(r'[\u4e00-\u9fff]')
JS={
'已复制：':'Copied: ', '现在':'now', '1 V 泄放线':'1 V discharge threshold',
'# 交互模拟，输出为示例，不连接真实设备。输入 help 查看命令。':'# Interactive simulation. Example output only; no real hardware. Type help for commands.',
'bench-a 或':'bench-a or','（试试 voloo ls）':'(try voloo ls)','（输入 help）':'(type help)',
'模拟操作完成，结果见终端。':'Done. Check the result in the terminal.',
'要按住 0.6 秒，防止误触。':'Press and hold for 0.6 seconds. A quick tap will not switch power.',
'正负极接反会损坏硬件。先在未连接 voloo 时量清 SOURCE 的 + / −，关闭电源后再接线。':'Reversed polarity damages hardware. Measure SOURCE + / − before connecting voloo, then turn the supply OFF before wiring.',
'不要用同一适配器同时给设备和 USB 操作电脑供电，不同输出口也不行。请分别使用独立电源。':'Use independent supplies for the device and USB host computer. Different ports on one adapter do not qualify.',
'拔掉设备的其他供电，只让它从 OUT 取电。调试器只连 GND 和信号线。':'Disconnect other device power sources; use OUT only. Connect debuggers only to GND and required signal lines.',
'先别通电':'Do not switch on yet','三项自查已填写':'Checklist complete',
'这里只根据回答提示，不检测实物接线。还需确认标称电压范围、最高建议电流 3 A，以及功率和温升条件。插上 USB-C，运行 ':'This checklist uses your answers; it cannot inspect physical wiring. Also check the nominal voltage range, maximum recommended 3 A current, power and temperature. Connect USB-C and run ',
'还差 ':'Remaining: ',' 项':' checks','确认接线及电气边界后，再打开电源。':'Check wiring and electrical limits before enabling power.',
'已复制':'Copied','请手动复制':'Copy manually','复制':'Copy',
}
BASE_CSS='''
.locale-link{display:inline-flex;align-items:center;justify-content:center;flex:none;min-width:44px;min-height:44px;padding:7px 10px;border:1px solid currentColor;border-radius:999px;font:600 13px/1.2 system-ui,sans-serif;text-decoration:none;white-space:nowrap}
@media(max-width:640px){.bar .wrap{gap:10px}.bar .brand img{width:min(48vw,244px);height:auto}.bar .product-category{display:none}.bar .theme-toggle{flex:none}.bar .locale-link{margin-left:auto}.bar .brand{margin-right:auto}}
'''
EN_CSS='''
html[lang=en] body{font-family:Arial,Helvetica,system-ui,sans-serif}
html[lang=en] .hero h1{font-family:Arial,Helvetica,sans-serif;font-size:clamp(30px,3.8vw,54px);line-height:1.13;letter-spacing:-.045em;gap:10px}
html[lang=en] .hero h1 .title-line{height:auto;min-height:1.15em;column-gap:.18em}
html[lang=en] .hero .lede{max-width:51ch}
html[lang=en] .h2{font-size:clamp(27px,4.2vw,50px)}
html[lang=en] .sticker small{font-size:11px}
html[lang=en] .chip{white-space:normal;max-width:100%;line-height:1.5}
@media(max-width:860px){html[lang=en] .hero h1{font-size:clamp(30px,7.5vw,56px)}}
@media(max-width:480px){html[lang=en] .hero-cta{gap:12px}html[lang=en] .hero-cta .btn{padding:12px 14px;font-size:14px}html[lang=en] .hero-note{font-size:13px}html[lang=en] .terminal-steps article{grid-template-columns:80px minmax(0,1fr)}}
'''
POSTER_CSS='''
h1{font-family:Arial,Helvetica,sans-serif;font-size:92px;line-height:1.12;letter-spacing:-.04em}.sub{font-family:Arial,Helvetica,sans-serif;font-size:24px;line-height:1.5;width:540px}.pt{padding:18px 16px}.pt .b{width:44px;height:44px;margin-bottom:10px}.pt h3{font-size:21px;line-height:1.25}.pt p{font-size:16px;line-height:1.35;margin-top:6px}.warn .t{font-size:25px}.warn .t small{font-size:14px}.warn .r{padding:8px 16px}.warn .r b{font-size:17px;line-height:1.2}.warn .r span{font-size:13px;line-height:1.35}.get .url{font-size:12px}.scope text{font-size:13px}
'''
LONG_CSS='''
h1{font-family:Arial,Helvetica,sans-serif;font-size:88px;line-height:1.16;letter-spacing:-.04em}.intro{font-size:37px}.eyebrow{letter-spacing:.03em;font-size:27px}.flow{font-size:32px}.row h3{font-size:39px}.row p{font-size:33px}.terminal-panels h3{font-size:25px}.terminal-panels p{font-size:24px}.sticker small{font-size:24px}.footer{grid-template-columns:minmax(0,1fr) 360px}.footer .address{font-size:29px}.footer .fine{font-size:27px}
'''
QUICK_EN_CSS='''
@media(max-width:650px){
 .steps .step{display:grid;grid-template-columns:92px minmax(0,1fr);gap:8px 12px}
 .steps .step>div,.remove>div{display:contents}
 .steps .step img{grid-column:1;grid-row:1;height:105px}
 .steps .step h3{grid-column:2;grid-row:1;align-self:center;display:grid;grid-template-columns:28px minmax(0,1fr);column-gap:9px;font-size:18px}
 .steps .step h3 span{margin-right:0}
 .steps .step p{grid-column:1/-1;margin:0;font-size:15px;line-height:1.6}
 .remove{grid-template-columns:92px minmax(0,1fr);gap:8px 12px}
 .remove img{grid-column:1;grid-row:1;width:92px}
 .remove h3{grid-column:2;grid-row:1;align-self:center}
 .remove p{grid-column:1/-1;margin:0;font-size:15px;line-height:1.6}
}
'''
def language_ui(soup,filename,english=False):
    soup.html['lang']='en' if english else 'zh-CN'
    for n in soup.select('#locale-style,.locale-link,link[hreflang]'):n.decompose()
    other=filename.replace('.en.html','.html') if english else filename.replace('.html','.en.html')
    a=soup.new_tag('a',href=other,attrs={'class':'locale-link','lang':'zh-CN' if english else 'en','hreflang':'zh-CN' if english else 'en'})
    a.string='中文' if english else 'EN'
    bar=soup.select_one('.bar .wrap') or soup.select_one('header') or soup.select_one('.page-nav')
    if bar:
        theme=bar.select_one('.theme-toggle')
        if theme:theme.insert_before(a)
        else:bar.append(a)
    extra=EN_CSS if english and filename.startswith('index') else QUICK_EN_CSS if english and filename.startswith('quick-start') else ''
    style=soup.new_tag('style',id='locale-style');style.string=BASE_CSS+extra;soup.head.append(style)
    for lang,url in [('zh-CN',filename.replace('.en.html','.html')),('en',filename.replace('.en.html','.html').replace('.html','.en.html'))]:
        soup.head.append(soup.new_tag('link',rel='alternate',hreflang=lang,href=url))
    canonical=soup.select_one('link[rel=canonical]')
    if canonical:canonical.decompose()
    soup.head.append(soup.new_tag('link',rel='canonical',href='https://voloo.librezoo.com/'+('' if filename=='index.html' else filename)))

def translate(source,target,kind='site'):
    soup=BeautifulSoup(source.read_text(),'html.parser');missing=set()
    for node in list(soup.find_all(string=True)):
        if isinstance(node,Comment):continue
        raw=str(node);key=raw.strip()
        if node.parent.name=='style':continue
        if node.parent.name=='script':
            for old,new in sorted(JS.items(),key=lambda a:-len(a[0])):raw=raw.replace(old,new)
            node.replace_with(raw);continue
        if HAN.search(key):
            if key in TRANSLATIONS:
                value=TRANSLATIONS[key]
                # Chinese source often joins inline tags without spaces. English
                # needs word boundaries around code, emphasis and links.
                if value and node.parent.name not in ['pre','code','text','title']:
                    value=' '+value+' '
                node.replace_with(raw.replace(key,value))
            else:missing.add(key)
        elif any(c in raw for c in '。，；：'):
            node.replace_with(raw.replace('。','.').replace('，',', ').replace('；','; ').replace('：',': '))
    for tag in soup.find_all(True):
        for name in ['alt','title','aria-label','placeholder','content']:
            value=tag.get(name)
            if isinstance(value,str) and HAN.search(value):
                if value in TRANSLATIONS:tag[name]=TRANSLATIONS[value]
                else:missing.add(value)
        if tag.get('href'):
            href=tag['href']
            for old,new in [('index.html','index.en.html'),('quick-start.html','quick-start.en.html'),('guide.html','guide.en.html'),('downloads.html','downloads.en.html'),('voloo-quick-start-zh.pdf','voloo-quick-start-en.pdf')]:href=href.replace(old,new)
            tag['href']=href
    if missing:raise ValueError('Missing English translations: '+repr(sorted(missing)))
    # Inline emphasis/code in the Chinese source can leave a space before
    # punctuation after translation. Keep word spacing without "OFF .".
    for node in list(soup.find_all(string=True)):
        if isinstance(node,Comment) or node.parent.name in ['script','style','pre','code']:continue
        raw=str(node)
        if re.match(r'^\s+[.,;:!?)]',raw):
            prev=node.previous_sibling
            if prev and getattr(prev,'find_all',None):
                texts=prev.find_all(string=True)
                if texts:texts[-1].replace_with(str(texts[-1]).rstrip())
            node.replace_with(raw.lstrip())
    if kind=='site':
        language_ui(soup,target.name,True)
        if target.name=='index.en.html':
            soup.select_one('.hero h1').clear()
            soup.select_one('.hero h1').append(BeautifulSoup('<span class="title-line">A power switch</span><span class="title-line">you can <span class="hl">program.</span></span>','html.parser'))
            for paragraph in soup.select('.agent p'):
                if 'read_only = true' in paragraph.get_text():
                    paragraph.clear()
                    paragraph.append(BeautifulSoup('Agents can list devices, check status, control power and read operation logs. For read-only access, set <span class="mono">read_only = true</span> under <span class="mono">[mcp]</span> in <span class="mono">~/.voloo/config.toml</span>.','html.parser'))
    else:
        soup.html['lang']='en'
        style=soup.new_tag('style');style.string=POSTER_CSS if kind=='poster' else LONG_CSS;soup.head.append(style)
        headline='A power switch<br>you can<br><span class="hl">program.</span>' if kind=='poster' else 'A power switch<br>you can <span class="highlight">program.</span>'
        h=soup.select_one('h1');h.clear();h.append(BeautifulSoup(headline,'html.parser'))
    target.write_text(str(soup).rstrip()+'\n')

for name in ['index.html','quick-start.html']:
    path=ROOT/'site'/name
    soup=BeautifulSoup(path.read_text(),'html.parser');language_ui(soup,name);path.write_text(str(soup).rstrip()+'\n')
    translate(path,path.with_name(name.replace('.html','.en.html')))
for name,kind in [('poster.html','poster'),('friends-long.html','long')]:translate(ROOT/'poster'/name,ROOT/'poster'/name.replace('.html','.en.html'),kind)
# Full manuals already have independently authored English text.
for name,english in [('guide.html',False),('guide.en.html',True)]:
    path=ROOT/'site'/name;soup=BeautifulSoup(path.read_text(),'html.parser');language_ui(soup,name,english)
    for a in soup.select('a[href]'):
        if english and a['href']=='index.html':a['href']='index.en.html'
        if english and a['href']=='quick-start.html':a['href']='quick-start.en.html'
    path.write_text(str(soup).rstrip()+'\n')
print('Built Chinese / English navigation, landing pages, quick starts and poster sources.')
