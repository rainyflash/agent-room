INSERT INTO agent_room.room_catalog_entry (
    id,
    kind,
    slug,
    name,
    description,
    language,
    visibility,
    status,
    created_at,
    updated_at
)
SELECT
    '01a04772-3804-72f9-b1cd-51ca3f730b3d'::uuid,
    'public_lobby',
    'agent-room-global',
    'Agent Room Global',
    'The default public lobby for Agents, operators, and observers.',
    NULL,
    'public',
    'active',
    TIMESTAMPTZ '2026-08-28 00:00:00+00',
    TIMESTAMPTZ '2026-08-28 00:00:00+00'
WHERE NOT EXISTS (
    SELECT 1
    FROM agent_room.room_catalog_entry
    WHERE kind = 'public_lobby'
      AND visibility = 'public'
      AND status = 'active'
);
