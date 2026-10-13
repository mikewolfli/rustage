// SPDX-License-Identifier: MIT
//
// Area-average downscaling in premultiplied alpha.
//
// Derivation always rasterises a master once at the largest size needed and then scales down, so
// the colour and the edge treatment of every smaller frame come from one fixed rule instead of
// from whatever a desktop tool happened to export. Upscaling is refused: a smaller source cannot
// honestly claim a larger frame.

export function downscale(image, targetWidth, targetHeight) {  const { width, height, rgba } = image;
  if (targetWidth === width && targetHeight === height) return { width, height, rgba: Buffer.from(rgba) };
  if (targetWidth > width || targetHeight > height) {
    throw new Error(
      `downscale: refusing to scale ${width}x${height} up to ${targetWidth}x${targetHeight}; `
      + 'rasterise the master at the largest size first',
    );
  }

  const output = Buffer.alloc(targetWidth * targetHeight * 4);
  const scaleX = width / targetWidth;
  const scaleY = height / targetHeight;

  for (let y = 0; y < targetHeight; y += 1) {
    const startY = Math.floor(y * scaleY);
    const endY = Math.max(startY + 1, Math.ceil((y + 1) * scaleY));
    for (let x = 0; x < targetWidth; x += 1) {
      const startX = Math.floor(x * scaleX);
      const endX = Math.max(startX + 1, Math.ceil((x + 1) * scaleX));
      let red = 0;
      let green = 0;
      let blue = 0;
      let alpha = 0;
      let samples = 0;
      for (let sourceY = startY; sourceY < endY && sourceY < height; sourceY += 1) {
        for (let sourceX = startX; sourceX < endX && sourceX < width; sourceX += 1) {
          const index = (sourceY * width + sourceX) * 4;
          const weight = rgba[index + 3] / 255;
          red += rgba[index] * weight;
          green += rgba[index + 1] * weight;
          blue += rgba[index + 2] * weight;
          alpha += rgba[index + 3];
          samples += 1;
        }
      }
      const target = (y * targetWidth + x) * 4;
      const averageAlpha = alpha / samples;
      const weight = averageAlpha / 255;
      output[target] = weight === 0 ? 0 : Math.round(red / samples / weight);
      output[target + 1] = weight === 0 ? 0 : Math.round(green / samples / weight);
      output[target + 2] = weight === 0 ? 0 : Math.round(blue / samples / weight);
      output[target + 3] = Math.round(averageAlpha);
    }
  }
  return { width: targetWidth, height: targetHeight, rgba: output };
}
