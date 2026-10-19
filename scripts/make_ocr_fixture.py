"""Create a synthetic image-only page plus a digital table. Requires Pillow/reportlab."""
from pathlib import Path
import argparse
from PIL import Image, ImageDraw, ImageFont
from reportlab.pdfgen import canvas
from reportlab.lib.utils import ImageReader
p=argparse.ArgumentParser();p.add_argument('output',type=Path);a=p.parse_args()
a.output.parent.mkdir(parents=True,exist_ok=True)
image=Image.new('RGB',(1600,1000),'white');draw=ImageDraw.Draw(image)
font=ImageFont.load_default(size=70)
lines=['SYNTHETIC ARCHIVE RECORD','Reading and arithmetic','were proposed in 1799.','No outcome is stated.']
for i,line in enumerate(lines):draw.text((80,100+i*170),line,font=font,fill='black')
c=canvas.Canvas(str(a.output),pagesize=(600,800));c.drawImage(ImageReader(image),20,200,width=560,height=350);c.showPage()
c.setFont('Helvetica',18)
for i,(left,right) in enumerate([('Place','Year'),('Bern','1799'),('Zurich','1800'),('Basel','1801')]):
 c.drawString(50,720-i*60,left);c.drawString(350,720-i*60,right)
c.save()
print('Created synthetic two-page OCR/table fixture.')
