// Every screenshot is decoded and encoded again before it goes to GitHub, so
// what gets uploaded is only the picture: nothing riding inside or after the
// image data (a zip or a program behind a PNG header), and no metadata such
// as where a photo was taken.

import sharp from 'sharp';

// libvips keeps recent results to reuse; this server never repeats work, so
// the cache would only hold memory.
sharp.cache(false);

// A 6K display is about 20 million pixels. The cap stops a small file that
// decodes to something enormous from taking the server's memory.
const MAX_PIXELS = 40_000_000;

const FORMATS = {
  png: { mime: 'image/png', encode: (image) => image.png() },
  jpg: { mime: 'image/jpeg', encode: (image) => image.jpeg({ quality: 90 }) },
  gif: { mime: 'image/gif', encode: (image) => image.gif() },
  webp: { mime: 'image/webp', encode: (image) => image.webp({ quality: 90 }) },
};

// Its message finishes the sentence "Screenshot 2 …".
export class ImageError extends Error {
  constructor(status, message) {
    super(message);
    this.name = 'ImageError';
    this.status = status;
  }
}

// Re-encodes the image at `input` (a path or bytes) into the file `output`, in
// the format `ext` names ("png", "jpg", "gif" or "webp"), keeping animation and
// turning photos the way their camera said. If that comes out over
// `maxBytes`, the first frame becomes a JPEG instead. Returns {ext, mime}.
export async function reencode(input, output, ext, maxBytes) {
  const open = (animated) =>
    sharp(input, { animated, autoOrient: true, limitInputPixels: MAX_PIXELS, failOn: 'error' });

  try {
    const same = await FORMATS[ext].encode(open(true)).toFile(output);
    if (same.size <= maxBytes) return { ext, mime: FORMATS[ext].mime };

    const smaller = await open(false).flatten({ background: '#ffffff' }).jpeg({ quality: 85 }).toFile(output);
    if (smaller.size <= maxBytes) return { ext: 'jpg', mime: 'image/jpeg' };
  } catch (err) {
    if (/pixel limit/i.test(err.message)) {
      throw new ImageError(413, 'is too large: 40 million pixels at most.');
    }
    throw new ImageError(400, "couldn't be read as an image.");
  }
  throw new ImageError(413, `must be ${Math.round(maxBytes / (1024 * 1024))} MB or smaller.`);
}
