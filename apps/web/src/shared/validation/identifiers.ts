import { z } from 'zod';

export const uuidV7Schema = z
  .uuid()
  .refine((value) => value[14]?.toLowerCase() === '7', 'identifier must be UUIDv7');

export const matrixRoomIdSchema = z
  .string()
  .min(4)
  .max(512)
  .regex(/^![^\s:]+:[^\s]+$/u);
