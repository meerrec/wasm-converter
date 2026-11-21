import { inflateSync, deflateSync } from 'node:zlib';
import { createHash } from 'node:crypto';

const signature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
const crcTable = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
function crc32(buffer) {
  let c = 0xffffffff;
  for (const byte of buffer) c = crcTable[(c ^ byte) & 255] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function paeth(a, b, c) {
  const p = a + b - c;
  const pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
  return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
}

/** Decode the deliberately narrow, deterministic certification PNG contract. */
export function decodePng(bytes) {
  const buffer = Buffer.from(bytes);
  if (!buffer.subarray(0, 8).equals(signature)) throw new Error('PNG signature is missing');
  let offset = 8, width, height, channels, colorType;
  const idat = [];
  while (offset < buffer.length) {
    const length = buffer.readUInt32BE(offset);
    const type = buffer.toString('ascii', offset + 4, offset + 8);
    const data = buffer.subarray(offset + 8, offset + 8 + length);
    const expected = buffer.readUInt32BE(offset + 8 + length);
    const actual = crc32(buffer.subarray(offset + 4, offset + 8 + length));
    if (expected !== actual) throw new Error(`PNG ${type} CRC mismatch`);
    if (type === 'IHDR') {
      width = data.readUInt32BE(0); height = data.readUInt32BE(4);
      if (data[8] !== 8 || data[12] !== 0) throw new Error('Only 8-bit non-interlaced PNG is supported');
      colorType = data[9];
      if (colorType === 2) channels = 3;
      else if (colorType === 6) channels = 4;
      else throw new Error(`PNG color type ${colorType} is outside RGB8/RGBA8 contract`);
    } else if (type === 'IDAT') idat.push(data);
    else if (type === 'IEND') break;
    offset += length + 12;
  }
  if (!width || !height || !channels || idat.length === 0) throw new Error('Incomplete PNG');
  const raw = inflateSync(Buffer.concat(idat));
  const stride = width * channels;
  if (raw.length !== height * (stride + 1)) throw new Error('Unexpected PNG scanline length');
  const pixels = Buffer.alloc(width * height * 4);
  const previous = Buffer.alloc(stride);
  for (let y = 0; y < height; y += 1) {
    const filter = raw[y * (stride + 1)];
    const row = Buffer.from(raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1)));
    for (let x = 0; x < stride; x += 1) {
      const left = x >= channels ? row[x - channels] : 0;
      const up = previous[x], upperLeft = x >= channels ? previous[x - channels] : 0;
      if (filter === 1) row[x] = (row[x] + left) & 255;
      else if (filter === 2) row[x] = (row[x] + up) & 255;
      else if (filter === 3) row[x] = (row[x] + Math.floor((left + up) / 2)) & 255;
      else if (filter === 4) row[x] = (row[x] + paeth(left, up, upperLeft)) & 255;
      else if (filter !== 0) throw new Error(`Unknown PNG filter ${filter}`);
    }
    for (let x = 0; x < width; x += 1) {
      const source = x * channels, target = (y * width + x) * 4;
      pixels[target] = row[source]; pixels[target + 1] = row[source + 1]; pixels[target + 2] = row[source + 2];
      pixels[target + 3] = channels === 4 ? row[source + 3] : 255;
    }
    row.copy(previous);
  }
  return { width, height, colorType: colorType === 2 ? 'RGB8' : 'RGBA8', pixels };
}

function chunk(type, data) {
  const name = Buffer.from(type), body = Buffer.from(data);
  const result = Buffer.alloc(body.length + 12);
  result.writeUInt32BE(body.length, 0); name.copy(result, 4); body.copy(result, 8);
  result.writeUInt32BE(crc32(Buffer.concat([name, body])), body.length + 8);
  return result;
}
export function encodePng({ width, height, pixels }) {
  if (pixels.length !== width * height * 4) throw new Error('RGBA buffer size mismatch');
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0); ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; ihdr[9] = 6;
  const raw = Buffer.alloc(height * (width * 4 + 1));
  for (let y = 0; y < height; y += 1) {
    const start = y * (width * 4 + 1);
    pixels.copy(raw, start + 1, y * width * 4, (y + 1) * width * 4);
  }
  return Buffer.concat([signature, chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw, { level: 9 })), chunk('IEND', Buffer.alloc(0))]);
}
export function sha256(bytes) { return createHash('sha256').update(bytes).digest('hex'); }
