"""Build locally hosted OFL webfont subsets for the current public copy.
uv run --with fonttools --with brotli python website/build_fonts.py
Upstream font files stay in ignored output/font-sources; only web subsets and licenses are published.
"""
from pathlib import Path
from urllib.request import urlretrieve
from urllib.parse import quote
import re
from fontTools import subset
from fontTools.ttLib import TTFont
ROOT=Path(__file__).resolve().parent
SOURCES=ROOT.parent/'output/font-sources';SOURCES.mkdir(parents=True,exist_ok=True)
TARGET=ROOT/'site/assets/fonts';TARGET.mkdir(parents=True,exist_ok=True)
text=''.join(p.read_text() for folder,ext in [(ROOT/'site','*.html'),(ROOT/'poster','*.html'),(ROOT/'i18n','*.json')] for p in folder.glob(ext))
text+=''.join(chr(i) for i in range(32,127))
for family,name,out,primary in [('notosanssc','NotoSansSC[wght].ttf','cjk.woff2','Voloo CJK Subset'),('unbounded','Unbounded[wght].ttf','display.woff2','Voloo Display Subset'),('dmmono','DMMono-Regular.ttf','mono-400.woff2','Voloo Mono Subset'),('dmmono','DMMono-Medium.ttf','mono-500.woff2','Voloo Mono Subset')]:
 source=SOURCES/name
 if not source.exists():urlretrieve('https://raw.githubusercontent.com/google/fonts/main/ofl/'+family+'/'+quote(name),source)
 license=TARGET/(family+'-OFL.txt')
 if not license.exists():urlretrieve('https://raw.githubusercontent.com/google/fonts/main/ofl/'+family+'/OFL.txt',license)
 license.write_text('\n'.join(line.rstrip() for line in license.read_text().splitlines())+'\n')
 font=TTFont(source);options=subset.Options();options.flavor='woff2';options.layout_features=['*'];options.name_IDs=['*'];options.name_legacy=True;options.name_languages=['*']
 sub=subset.Subsetter(options=options);sub.populate(text=text);sub.subset(font)
 for n in font['name'].names:
  if n.nameID in [1,4,6]:font['name'].setName(primary.replace(' ','') if n.nameID==6 else primary,n.nameID,n.platformID,n.platEncID,n.langID)
 font.flavor='woff2';font.save(TARGET/out);print(out,(TARGET/out).stat().st_size)
css='''/* Webfont subsets from Google Fonts. Copyrights and SIL OFL 1.1 are in the adjacent *-OFL.txt files. */
@font-face{font-family:"Noto Sans SC";font-style:normal;font-weight:100 900;font-display:swap;src:url("cjk.woff2") format("woff2")}
@font-face{font-family:"Unbounded";font-style:normal;font-weight:200 900;font-display:swap;src:url("display.woff2") format("woff2")}
@font-face{font-family:"DM Mono";font-style:normal;font-weight:400;font-display:swap;src:url("mono-400.woff2") format("woff2")}
@font-face{font-family:"DM Mono";font-style:normal;font-weight:500;font-display:swap;src:url("mono-500.woff2") format("woff2")}
'''
(TARGET/'fonts.css').write_text(css)
for folder in ['site','poster']:
 for p in (ROOT/folder).glob('*.html'):
  s=p.read_text();s=re.sub(r'<link\b[^>]*(?:fonts\.googleapis\.com|fonts\.gstatic\.com|assets/fonts/fonts\.css)[^>]*>\s*','',s)
  href='assets/fonts/fonts.css' if folder=='site' else '../site/assets/fonts/fonts.css'
  link=f'<link rel="stylesheet" href="{href}">\n'
  if '</head>' in s:s=s.replace('</head>',link+'</head>',1)
  else:s=s.replace('<style>',link+'<style>',1)
  p.write_text(s.rstrip()+'\n')
print('All public HTML uses local webfont CSS.')
