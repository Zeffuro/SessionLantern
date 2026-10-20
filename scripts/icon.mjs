import {deflateSync} from 'node:zlib';
import {mkdir,writeFile} from 'node:fs/promises';

const size=64;
const pixels=Buffer.alloc((size*4+1)*size);
for(let y=0;y<size;y++)for(let x=0;x<size;x++){
  const offset=y*(size*4+1)+1+x*4;
  const glow=(x-32)**2+(y-33)**2<18**2;
  const frame=(x>=17&&x<=46&&y>=17&&y<=50)&&(x<21||x>42||y<21||y>46);
  const handle=y>=9&&y<=17&&(x>=25&&x<=38)&&(y<12||x<28||x>35);
  const flame=y>=25&&y<=41&&Math.abs(x-32)<(y-23)*.38;
  const base=y>=51&&y<=54&&x>=17&&x<=46;
  const color=frame||handle||base?[121,220,212]:flame?[255,210,129]:glow?[32,72,79]:[14,31,42];
  pixels.set([...color,255],offset);
}
const crc=buffer=>{let value=0xffffffff;for(const b of buffer){value^=b;for(let n=0;n<8;n++)value=(value>>>1)^((value&1)?0xedb88320:0);}return(value^0xffffffff)>>>0;};
const chunk=(name,data)=>{const type=Buffer.from(name),length=Buffer.alloc(4),checksum=Buffer.alloc(4);length.writeUInt32BE(data.length);checksum.writeUInt32BE(crc(Buffer.concat([type,data])));return Buffer.concat([length,type,data,checksum]);};
const header=Buffer.alloc(13);header.writeUInt32BE(size);header.writeUInt32BE(size,4);header[8]=8;header[9]=6;
const png=Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),chunk('IHDR',header),chunk('IDAT',deflateSync(pixels)),chunk('IEND',Buffer.alloc(0))]);
const ico=Buffer.alloc(22);ico.writeUInt16LE(1,2);ico.writeUInt16LE(1,4);ico[6]=size;ico[7]=size;ico.writeUInt16LE(1,10);ico.writeUInt16LE(32,12);ico.writeUInt32LE(png.length,14);ico.writeUInt32LE(22,18);
await mkdir(new URL('../src-tauri/icons/',import.meta.url),{recursive:true});
await writeFile(new URL('../src-tauri/icons/icon.png',import.meta.url),png);
await writeFile(new URL('../src-tauri/icons/icon.ico',import.meta.url),Buffer.concat([ico,png]));
