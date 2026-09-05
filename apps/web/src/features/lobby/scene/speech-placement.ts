export type SpeechBounds = {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
};

/** 优先贴近说话人；空间不足时隐藏气泡，完整消息仍保留在大厅对话。 */
export function placeSpeech(
  anchor: { readonly x: number; readonly y: number },
  size: { readonly width: number; readonly height: number },
  viewport: { readonly width: number; readonly height: number },
  occupied: readonly SpeechBounds[],
): SpeechBounds | null {
  if (
    anchor.x < 0 ||
    anchor.x > viewport.width ||
    anchor.y < 110 ||
    anchor.y > viewport.height - 90
  )
    return null;
  const desired = {
    ...size,
    x: Math.max(12, Math.min(viewport.width - size.width - 12, anchor.x - size.width / 2)),
    y: Math.max(120, anchor.y - size.height - 10),
  };
  const candidates = [
    desired,
    ...occupied.flatMap((prior) => [
      { ...desired, y: prior.y - size.height - 10 },
      { ...desired, y: prior.y + prior.height + 10 },
      { ...desired, x: prior.x - size.width - 10 },
      { ...desired, x: prior.x + prior.width + 10 },
    ]),
  ].toSorted(
    (a, b) =>
      Math.hypot(a.x - desired.x, a.y - desired.y) - Math.hypot(b.x - desired.x, b.y - desired.y),
  );
  return (
    candidates.find(
      (candidate) =>
        candidate.x >= 12 &&
        candidate.x + candidate.width <= viewport.width - 12 &&
        candidate.y >= 120 &&
        candidate.y + candidate.height <= viewport.height - 100 &&
        Math.hypot(candidate.x - desired.x, candidate.y - desired.y) <= 180 &&
        occupied.every(
          (prior) =>
            candidate.x >= prior.x + prior.width + 8 ||
            candidate.x + candidate.width + 8 <= prior.x ||
            candidate.y >= prior.y + prior.height + 8 ||
            candidate.y + candidate.height + 8 <= prior.y,
        ),
    ) ?? null
  );
}
