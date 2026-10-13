// SPDX-License-Identifier: MIT
//
// Windows ICO and macOS ICNS containers for the derived platform icons.
//
// The frames themselves come from the branding masters through the fixed rasteriser; these
// writers only package them. Frames up to `bmpMaxSize` are written as 32-bit DIBs with an AND mask
// because that is what the Windows shell reads everywhere, and larger frames as PNG, which is the
// Windows Vista and later encoding. `docs/plan/resources.md` section 13.1 asks for both the small
// frames and the 256 px frame to be readable by the target system; that verification happens on
// Windows, not here.

import { encodePng } from './png.mjs';

function dibFrame(image) {
  const { width, height, rgba } = image;
  const xorSize = width * height * 4;
  const maskStride = Math.ceil(width / 32) * 4;
  const maskSize = maskStride * height;
  const header = Buffer.alloc(40);
  header.writeUInt32LE(40, 0);
  header.writeInt32LE(width, 4);
  header.writeInt32LE(height * 2, 8);
  header.writeUInt16LE(1, 12);
  header.writeUInt16LE(32, 14);
  header.writeUInt32LE(0, 16);
  header.writeUInt32LE(xorSize + maskSize, 20);

  const pixels = Buffer.alloc(xorSize);
  const mask = Buffer.alloc(maskSize, 0);
  for (let y = 0; y < height; y += 1) {
    const sourceRow = height - 1 - y;
    for (let x = 0; x < width; x += 1) {
      const source = (sourceRow * width + x) * 4;
      const target = (y * width + x) * 4;
      pixels[target] = rgba[source + 2];
      pixels[target + 1] = rgba[source + 1];
      pixels[target + 2] = rgba[source];
      pixels[target + 3] = rgba[source + 3];
      if (rgba[source + 3] === 0) {
        mask[y * maskStride + (x >> 3)] |= 0x80 >> (x & 7);
      }
    }
  }
  return Buffer.concat([header, pixels, mask]);
}

export function encodeIco(frames, { bmpMaxSize = 64 } = {}) {
  if (frames.length === 0) throw new Error('encodeIco: no frames');
  const entries = [];
  for (const frame of frames.slice().sort((left, right) => left.size - right.size)) {
    const { size } = frame;
    if (size > 256) throw new Error(`encodeIco: ${size} px exceeds the ICO limit of 256`);
    entries.push({
      size,
      data: size > bmpMaxSize
        ? encodePng({ width: frame.width, height: frame.height, rgba: frame.rgba })
        : dibFrame(frame),
    });
  }

  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(entries.length, 4);
  let offset = 6 + entries.length * 16;
  const directory = [];
  for (const entry of entries) {
    const record = Buffer.alloc(16);
    record[0] = entry.size === 256 ? 0 : entry.size;
    record[1] = entry.size === 256 ? 0 : entry.size;
    record[2] = 0;
    record[3] = 0;
    record.writeUInt16LE(1, 4);
    record.writeUInt16LE(32, 6);
    record.writeUInt32LE(entry.data.length, 8);
    record.writeUInt32LE(offset, 12);
    directory.push(record);
    offset += entry.data.length;
  }
  return Buffer.concat([header, ...directory, ...entries.map(entry => entry.data)]);
}

// The element types macOS reads for a modern retina icon: 16/32/64 as the older `icp*` codes so
// pre-retina shells still resolve an icon, then 128 through 1024 as PNG payloads.
const ICNS_TYPES = [
  ['icp4', 16],
  ['icp5', 32],
  ['icp6', 64],
  ['ic07', 128],
  ['ic08', 256],
  ['ic09', 512],
  ['ic10', 1024],
  ['ic11', 32],
  ['ic12', 64],
  ['ic13', 256],
  ['ic14', 512],
];

export function encodeIcns(frames) {
  const bySize = new Map(frames.map(frame => [frame.size, frame]));
  const entries = [];
  for (const [type, size] of ICNS_TYPES) {
    const frame = bySize.get(size);
    if (frame === undefined) continue;
    const payload = encodePng({ width: frame.width, height: frame.height, rgba: frame.rgba });
    const header = Buffer.alloc(8);
    header.write(type, 0, 'ascii');
    header.writeUInt32BE(payload.length + 8, 4);
    entries.push(Buffer.concat([header, payload]));
  }
  if (entries.length === 0) throw new Error('encodeIcns: no frame matched an ICNS element type');
  const body = Buffer.concat(entries);
  const header = Buffer.alloc(8);
  header.write('icns', 0, 'ascii');
  header.writeUInt32BE(body.length + 8, 4);
  return Buffer.concat([header, body]);
}
