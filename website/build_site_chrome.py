"""Apply the shared site brand links and footer to both languages."""
from pathlib import Path
from copy import deepcopy
import xml.etree.ElementTree as ET
from bs4 import BeautifulSoup
ROOT=Path(__file__).resolve().parent
SITE=ROOT/'site'
ASSETS=SITE/'assets/brand'
ET.register_namespace('', 'http://www.w3.org/2000/svg')
# Split the approved lockup without changing its outlines, colors or placement.
for dark in [False,True]:
 suffix='-on-dark' if dark else ''
 source=ET.parse(ROOT.parent/f'branding/librezoo-voloo-compact{suffix}.svg').getroot()
 children=list(source)
 for name,view,child in [('librezoo-logo','0 0 120 50',children[1]),('voloo-wordmark','150 0 94 50',children[3])]:
  out=ET.Element('{http://www.w3.org/2000/svg}svg',{'viewBox':view,'role':'img','aria-label':'LibreZoo' if name=='librezoo-logo' else 'voloo'})
  out.append(deepcopy(child));ET.ElementTree(out).write(ASSETS/f'{name}{suffix}.svg',encoding='unicode')

def brand(english):
 top='voloo · Back to top' if english else 'voloo · 回到顶部'
 return f'''<div class="brand">
 <a class="brand-home" href="https://librezoo.com/" aria-label="LibreZoo">
  <picture class="brand-on-light"><img src="assets/brand/librezoo-logo.svg" alt="LibreZoo" width="120" height="50"></picture>
  <picture class="brand-on-dark"><img src="assets/brand/librezoo-logo-on-dark.svg" alt="LibreZoo" width="120" height="50"></picture>
 </a><span class="brand-divider" aria-hidden="true"></span>
 <a class="brand-top" href="#top" aria-label="{top}">
  <picture class="brand-on-light"><img src="assets/brand/voloo-wordmark.svg" alt="voloo" width="94" height="50"></picture>
  <picture class="brand-on-dark"><img src="assets/brand/voloo-wordmark-on-dark.svg" alt="voloo" width="94" height="50"></picture>
 </a></div>'''

wordmark=(ROOT.parent/'branding/voloo-wordmark.svg').read_text().replace('#2D1A2B','currentColor')
svg=BeautifulSoup(wordmark,'html.parser').svg
svg['class']='footer-wordmark';svg['aria-hidden']='true';svg.attrs.pop('aria-label',None);svg.attrs.pop('role',None)

def footer(english):
 suffix='.en' if english else ''
 labels=['Quick start','User guide','Downloads','GitHub'] if english else ['快速上手','使用说明','下载','GitHub']
 urls=[f'quick-start{suffix}.html',f'guide{suffix}.html',f'downloads{suffix}.html','https://github.com/librezoo/voloo']
 links=''.join(f'<a href="{url}">{label}</a>' for url,label in zip(urls,labels))
 nav='Footer navigation' if english else '页脚导航';top='voloo · Back to top' if english else 'voloo · 回到顶部'
 return f'''<footer class="site-footer"><div class="footer-inner">
 <div class="footer-top"><div class="footer-brandline"><a class="footer-voloo" href="#top" aria-label="{top}">{svg}</a><span class="footer-dot" aria-hidden="true">·</span><span class="footer-credit">Powered by <a href="https://librezoo.com/">LibreZoo</a></span></div>
 <nav class="footer-nav" aria-label="{nav}">{links}</nav></div>
 <div class="footer-bottom"><span>© 2026 LibreZoo</span></div>
 </div></footer>'''

for path in SITE.glob('*.html'):
 if path.name=='page.html':continue
 soup=BeautifulSoup(path.read_text(),'html.parser');english=soup.html.get('lang')=='en'
 if soup.select_one('header.bar .brand'):
  soup.select_one('header.bar .brand').replace_with(BeautifulSoup(brand(english),'html.parser'))
 elif soup.select_one('header'):
  header=soup.select_one('header');image=header.select_one('img[alt="librezoo · voloo"]')
  header['class']=list(dict.fromkeys(header.get('class',[])+['doc-header']))
  if image:
   target=image.parent if image.parent.name=='a' else image
   target.replace_with(BeautifulSoup(brand(english),'html.parser'))
  # A repeated build already has split links.
  elif header.select_one('.brand'):header.select_one('.brand').replace_with(BeautifulSoup(brand(english),'html.parser'))
  language=header.select_one('nav a[lang]')
  if language:
   language['class']=['locale-link'];header.append(language.extract())
 # Give every product wordmark a real top-of-page anchor, also on guide/download pages.
 if not soup.select_one('#top'):
  main=soup.select_one('main');(main or soup.body)['id']='top'
 if path.name.startswith('quick-start') and not soup.select_one('.illustration-note'):
  hero=soup.select_one('.hero')
  if hero:
   note=soup.new_tag('p',attrs={'class':'illustration-note','style':'font-size:13px;color:#65545f;margin-top:10px'})
   note.string='Product appearance is an AI-assisted rendering; operation diagrams are schematic. Follow the physical markings when wiring.' if english else '产品外观为 AI 辅助效果渲染；操作图为结构示意。实际接线以实物标识为准。'
   hero.insert_after(note)
 old=soup.select_one('footer')
 if old:
  # Keep instructional attribution next to the terminal instructions instead of in the site footer.
  if path.name.startswith('quick-start'):
   small=soup.select_one('small')
   if small and not small.select_one('a'):
    a=soup.new_tag('a',href='https://www.lcsc.com/datasheet/C475134.pdf');a.string=' Manufacturer drawing' if english else ' 厂家图纸';small.append(a)
  old.replace_with(BeautifulSoup(footer(english),'html.parser'))
 else:soup.body.append(BeautifulSoup(footer(english),'html.parser'))
 if not path.name.startswith('index'):
  soup.select_one('.site-footer')['class'].append('document-footer')
 if not soup.select_one('link[href="assets/site-chrome.css"]'):
  soup.head.append(soup.new_tag('link',rel='stylesheet',href='assets/site-chrome.css'))
 for link in soup.select('link[rel]'):
  if any('icon' in rel for rel in link.get('rel',[])):link.decompose()
 for attributes in [dict(rel='icon',type='image/svg+xml',href='favicon.svg'),dict(rel='icon',type='image/png',sizes='32x32',href='favicon-32.png'),dict(rel='apple-touch-icon',href='apple-touch-icon.png')]:
  soup.head.append(soup.new_tag('link',**attributes))
 path.write_text(str(soup).rstrip()+'\n')
print('Applied separate LibreZoo / voloo links and shared bilingual footers.')
