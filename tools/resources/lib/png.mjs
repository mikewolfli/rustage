// SPDX-License-Identifier: MIT
//
// Minimal PNG reader/writer for the icon pipeline.
//
// The pipeline needs PNG for two reasons and no more: the rasteriser hands its result back as
// PNG, and the Linux hicolor set, the Windows ICO frames above the BMP threshold and the macOS
// ICNS entries are PNG files. Writing it here keeps the derived output deterministic (fixed
// filter, no encoder-specific metadata, no timestamps) and lets an independent decoder verify
// the result instead of the same library checking itself.
//
// Only what the pipeline produces is supported: 8-bit non-interlaced grey, RGB or RGBA. Anything
// else is refused with a message rather than decoded approximately.

import { deflateSync, inflateSync } from 'node:zlib';

const SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

const CRC_TABLE = (() => {
  const table = new Int32Array(256);
  for (let index = 0; index < 256; index += 1) {
    let value = index;
    for (let bit = 0; bit < 8; bit += 1) {
      value = value & 1 ? 0xedb88320 ^ (value >>> 1) : value >>> 1;
    }
    table[index] = value;
  }
  return table;
})();

function crc32(buffer) {
  let crc = -1;
  for (const byte of buffer) crc = CRC_TABLE[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  return (crc ^ -1) >>> 0;
}

function chunk(type, data) {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length, 0);
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body), 0);
  return Buffer.concat([length, body, crc]);
}

export function encodePng({ width, height, rgba }) {
  if (rgba.length !== width * height * 4) {
    throw new Error(`encodePng: expected ${width * height * 4} RGBA bytes, received ${rgba.length}`);
  }
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header[8] = 8;
  header[9] = 6;
  header[10] = 0;
  header[11] = 0;
  header[12] = 0;

  const stride = width * 4;
  const raw = Buffer.alloc((stride + 1) * height);
  for (let y = 0; y < height; y += 1) {
    raw[y * (stride + 1)] = 0;
    rgba.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride);
  }

  return Buffer.concat([
    SIGNATURE,
    chunk('IHDR', header),
    chunk('IDAT', deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

function paeth(left, above, upperLeft) {
  const estimate = left + above - upperLeft;
  const leftDistance = Math.abs(estimate - left);
  const aboveDistance = Math.abs(estimate - above);
  const upperLeftDistance = Math.abs(estimate - upperLeft);
  if (leftDistance <= aboveDistance && leftDistance <= upperLeftDistance) return left;
  return aboveDistance <= upperLeftDistance ? above : upperLeft;
}

export function decodePng(buffer) {
  if (!buffer.subarray(0, 8).equals(SIGNATURE)) throw new Error('decodePng: not a PNG file');
  let offset = 8;
  let header = null;
  const data = [];
  while (offset + 8 <= buffer.length) {
    const length = buffer.readUInt32BE(offset);
    const type = buffer.toString('ascii', offset + 4, offset + 8);
    const body = buffer.subarray(offset + 8, offset + 8 + length);
    if (type === 'IHDR') {
      header = {
        width: body.readUInt32BE(0),
        height: body.readUInt32BE(4),
        depth: body[8],
        colorType: body[9],
        interlace: body[12],
      };
    } else if (type === 'IDAT') {
      data.push(body);
    } else if (type === 'IEND') {
      break;
    }
    offset += 12 + length;
  }
  if (header === null) throw new Error('decodePng: missing IHDR chunk');
  if (header.depth !== 8) throw new Error(`decodePng: only 8-bit channels are supported, found ${header.depth}`);
  if (header.interlace !== 0) throw new Error('decodePng: interlaced images are not supported');
  const channels = { 0: 1, 2: 3, 4: 2, 6: 4 }[header.colorType];
  if (channels === undefined) {
    throw new Error(`decodePng: colour type ${header.colorType} is not supported (grey, RGB and RGBA only)`);
  }

  const { width, height } = header;
  const stride = width * channels;
  const raw = inflateSync(Buffer.concat(data));
  if (raw.length < (stride + 1) * height) throw new Error('decodePng: truncated image data');
  const pixels = Buffer.alloc(stride * height);

  for (let y = 0; y < height; y += 1) {
    const filter = raw[y * (stride + 1)];
    const source = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    const target = pixels.subarray(y * stride, (y + 1) * stride);
    const previous = y === 0 ? null : pixels.subarray((y - 1) * stride, y * stride);
    for (let index = 0; index < stride; index += 1) {
      const left = index >= channels ? target[index - channels] : 0;
      const above = previous === null ? 0 : previous[index];
      const upperLeft = previous === null || index < channels ? 0 : previous[index - channels];
      const value = source[index];
      switch (filter) {
        case 0: target[index] = value; break;
        case 1: target[index] = (value + left) & 0xff; break;
        case 2: target[index] = (value + above) & 0xff; break;
        case 3: target[index] = (value + ((left + above) >> 1)) & 0xff; break;
        case 4: target[index] = (value + paeth(left, above, upperLeft)) & 0xff; break;
        default: throw new Error(`decodePng: unknown row filter ${filter}`);
      }
    }
  }

  const rgba = Buffer.alloc(width * height * 4);
  for (let index = 0; index < width * height; index += 1) {
    const source = index * channels;
    const target = index * 4;
    if (channels === 4) {
      pixels.copy(rgba, target, source, source + 4);
    } else if (channels === 3) {
      pixels.copy(rgba, target, source, source + 3);
      rgba[target + 3] = 255;
    } else if (channels === 2) {
      rgba[target] = pixels[source];
      rgba[target + 1] = pixels[source];
      rgba[target + 2] = pixels[source];
      rgba[target + 3] = pixels[source + 1];
    } else {
      rgba[target] = pixels[source];
      rgba[target + 1] = pixels[source];
      rgba[target + 2] = pixels[source];
      rgba[target + 3] = 255;
    }
  }

  return { width, height, rgba };
}
