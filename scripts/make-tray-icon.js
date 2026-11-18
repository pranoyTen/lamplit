// Draws the Mac menu bar icon: the bookmark ribbon from the app icon in solid black on transparent. macOS tints a "template" image to suit the menu bar,
// so only the shape (the alpha) matters. Run: node scripts/make-tray-icon.js   ->  src-tauri/icons/tray.png
const fs = require('node:fs');
const zlib = require('node:zlib');
const path = require('node:path');

const N = 44, SS = 4;
// point-in-polygon for the ribbon, on a 44 x 44 grid
const inside = (x, y, poly) => { let c = false; for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) { const [xi, yi] = poly[i], [xj, yj] = poly[j]; if ((yi > y) !== (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi) c = !c; } return c; };
const RIBBON = [[14, 7], [30, 7], [30, 36], [22, 28], [14, 36]]; // same proportions as the ribbon in icon.png: about 1:1.8 with a notch
const glyph = (x, y) => inside(x, y, RIBBON);

const raw = Buffer.alloc((N * 4 + 1) * N);
for (let y = 0; y < N; y++) {
  raw[y * (N * 4 + 1)] = 0;
  for (let x = 0; x < N; x++) {
    let a = 0;
    for (let sy = 0; sy < SS; sy++) for (let sx = 0; sx < SS; sx++) if (glyph(x + (sx + .5) / SS, y + (sy + .5) / SS)) a++;
    raw[y * (N * 4 + 1) + 1 + x * 4 + 3] = Math.round(a / (SS * SS) * 255); // rgb stay 0 (black)
  }
}

const crcTable = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
const crc = b => { let c = 0xffffffff; for (const v of b) c = crcTable[(c ^ v) & 0xff] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
const chunk = (type, data) => { const t = Buffer.from(type), len = Buffer.alloc(4), sum = Buffer.alloc(4); len.writeUInt32BE(data.length); sum.writeUInt32BE(crc(Buffer.concat([t, data]))); return Buffer.concat([len, t, data, sum]); };
const head = Buffer.alloc(13); head.writeUInt32BE(N, 0); head.writeUInt32BE(N, 4); head.set([8, 6, 0, 0, 0], 8); // 8 bit RGBA
const png = Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', head), chunk('IDAT', zlib.deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]);
fs.writeFileSync(path.join(__dirname, '..', 'src-tauri', 'icons', 'tray.png'), png);
console.log('wrote tray.png', png.length, 'bytes');
