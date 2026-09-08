import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  characterFoot,
  characterSize,
  characterVariant,
  loadStudioCharacterUrl,
  spriteCell,
} from '../studio-assets';

let sourceUrl: string | undefined;
function useCharacters() {
  const [source, setSource] = useState(sourceUrl);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let active = true;
    void loadStudioCharacterUrl()
      .then((url) => {
        sourceUrl = url;
        if (active) setSource(url);
      })
      .catch(() => {
        if (active) setFailed(true);
      });
    return () => {
      active = false;
    };
  }, []);
  return { source, failed };
}

export function StudioSprite({
  id,
  portrait = false,
}: {
  readonly id: string;
  readonly portrait?: boolean;
}) {
  const { t } = useTranslation();
  const { source, failed } = useCharacters();
  const column = characterVariant(id);
  const foot = characterFoot(id);
  return source === undefined ? (
    <g data-asset-state={failed ? 'failed' : 'loading'}>
      {failed ? <title>{t('studio.characterAssetUnavailable')}</title> : null}
      <circle cy="-38" r="18" fill="#737b68" />
      {failed ? (
        <text y="-32" textAnchor="middle" fill="white">
          !
        </text>
      ) : null}
    </g>
  ) : (
    <svg
      x={portrait ? -48 : (-foot.x * characterSize.width) / spriteCell.width}
      y={portrait ? -88 : (-foot.y * characterSize.height) / spriteCell.height}
      width={portrait ? 96 : characterSize.width}
      height={portrait ? 96 : characterSize.height}
      viewBox={
        portrait
          ? `${String(foot.x - 110)} 214 220 220`
          : `0 0 ${String(spriteCell.width)} ${String(spriteCell.height)}`
      }
      overflow="hidden"
    >
      <image
        data-character-sprite="true"
        href={source}
        x={-column * spriteCell.width}
        y={0}
        width="1536"
        height="1024"
      />
    </svg>
  );
}
