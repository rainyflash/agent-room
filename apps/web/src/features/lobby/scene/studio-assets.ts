import { characterSeed } from '../domain/room-floor';

export const studioCharacters = '/studio/flat-robots.png';
export const spriteCell = Object.freeze({ width: 384, height: 512 });
export const characterSize = Object.freeze({ width: 108, height: 144 });
export const characterVariant = (id: string): number => characterSeed(id) % 4;
const standingFeet = [
  { x: 218, y: 441 },
  { x: 205, y: 450 },
  { x: 178, y: 450 },
  { x: 153, y: 450 },
] as const;
const walkingFeet = [
  { x: 220, y: 338 },
  { x: 208, y: 341 },
  { x: 180, y: 342 },
  { x: 158, y: 340 },
] as const;
export function characterFoot(id: string, walking = false) {
  const feet = walking ? walkingFeet : standingFeet;
  return feet[characterVariant(id)] ?? feet[0];
}

let characters: Promise<HTMLCanvasElement> | undefined;
let characterUrl: Promise<string> | undefined;

/** One document-lifetime URL; repeating the full data URL in large rosters exhausts DOM memory. */
export function loadStudioCharacterUrl(): Promise<string> {
  characterUrl ??= loadStudioCharacters()
    .then(
      (canvas) =>
        new Promise<string>((resolve, reject) => {
          canvas.toBlob((blob) => {
            if (blob === null) reject(new Error('Studio character atlas could not be encoded.'));
            else resolve(URL.createObjectURL(blob));
          }, 'image/png');
        }),
    )
    .catch((error: unknown) => {
      characterUrl = undefined;
      throw error;
    });
  return characterUrl;
}

/** Decode the keyed production sprite atlas once; both renderers share identical pixels. */
export function loadStudioCharacters(): Promise<HTMLCanvasElement> {
  characters ??= new Promise<HTMLCanvasElement>((resolve, reject) => {
    const source = new Image();
    source.onload = () => {
      try {
        if (source.naturalWidth !== 1536 || source.naturalHeight !== 1024)
          throw new Error('Studio character atlas has unexpected dimensions.');
        const canvas = document.createElement('canvas');
        canvas.width = source.naturalWidth;
        canvas.height = source.naturalHeight;
        const context = canvas.getContext('2d', { willReadFrequently: true });
        if (context === null) throw new Error('Studio character atlas requires a 2D context.');
        context.drawImage(source, 0, 0);
        const pixels = context.getImageData(0, 0, canvas.width, canvas.height);
        const data = pixels.data;
        for (let index = 0; index < data.length; index += 4) {
          const red = data[index] ?? 0;
          const green = data[index + 1] ?? 0;
          const blue = data[index + 2] ?? 0;
          const spill = Math.min(red, blue) - green;
          if (spill <= 35) continue;
          const opacity = Math.max(0, 1 - (spill - 35) / 85);
          data[index + 3] = Math.round(255 * opacity);
          if (opacity > 0) {
            data[index] = Math.min(red, green + 35);
            data[index + 2] = Math.min(blue, green + 35);
          }
        }
        context.putImageData(pixels, 0, 0);
        resolve(canvas);
      } catch (error) {
        reject(new Error('Studio character atlas could not be decoded.', { cause: error }));
      }
    };
    source.onerror = () => {
      reject(new Error('Studio character atlas could not be loaded.'));
    };
    source.src = studioCharacters;
  }).catch((error: unknown) => {
    characters = undefined;
    throw error instanceof Error
      ? error
      : new Error('Studio character atlas failed.', { cause: error });
  });
  return characters;
}
