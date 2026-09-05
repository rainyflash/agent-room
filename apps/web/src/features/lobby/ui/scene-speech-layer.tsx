import { placeSpeech, type SpeechBounds } from '../scene/speech-placement';
import { useImperativeHandle, useLayoutEffect, useRef, type Ref } from 'react';
import { useTranslation } from 'react-i18next';
import type { RoomSpeech } from '../domain/room-speech';
import type { SceneFrame } from '../scene/scene-character';

export type SceneSpeechLayerHandle = { position(frame: SceneFrame): void };
export function SceneSpeechLayer({
  speech,
  onOpen,
  ref,
}: {
  readonly speech: readonly RoomSpeech[];
  readonly onOpen: (id: string) => void;
  readonly ref: Ref<SceneSpeechLayerHandle>;
}) {
  const { t } = useTranslation();
  const elements = useRef(new Map<string, HTMLButtonElement>());
  const frameRef = useRef<SceneFrame | null>(null);
  const position = (frame: SceneFrame): void => {
    frameRef.current = frame;
    const placed: SpeechBounds[] = [];
    for (const [id, element] of elements.current) {
      const character = frame.characters.find((candidate) => candidate.characterId === id);
      if (character === undefined) {
        element.hidden = true;
        continue;
      }
      element.hidden = false;
      const placement = placeSpeech(
        character,
        { width: element.offsetWidth, height: element.offsetHeight },
        frame,
        placed,
      );
      if (placement === null) {
        element.hidden = true;
        continue;
      }
      element.style.transform = `translate(${String(Math.round(placement.x))}px, ${String(Math.round(placement.y))}px)`;
      placed.push(placement);
    }
  };
  useImperativeHandle(ref, () => ({ position }));
  useLayoutEffect(() => {
    if (frameRef.current !== null) position(frameRef.current);
  }, [speech]);
  return (
    <div className="scene-speech-layer" role="group" aria-label={t('roomGame.recentSpeech')}>
      {speech.map((bubble) => (
        <button
          key={bubble.messageId}
          className="scene-speech"
          type="button"
          data-speaker={bubble.characterId}
          ref={(element) => {
            if (element === null) elements.current.delete(bubble.characterId);
            else elements.current.set(bubble.characterId, element);
          }}
          aria-label={t('roomGame.openSpeech', { name: bubble.name, text: bubble.text })}
          onClick={() => {
            onOpen(bubble.messageId);
          }}
        >
          <strong>{bubble.name}</strong>
          <span>{bubble.text}</span>
        </button>
      ))}
    </div>
  );
}
