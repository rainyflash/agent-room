-- 已经有的电脑补上“上次使用”（设置里“已登录的电脑”，specs/workspace-tidy/design.md 第 3a 步）。
-- 这一列以前从来没写过，从这一版起换令牌时记。Bridge 每换一次令牌，令牌组的到期时间就续到
-- 那一刻加 30 天（生产的空闲时限），所以往回推 30 天就是最后一次换令牌的时候。碰到一年上限的，
-- 推出来的比真正最后一次早，只会显得更久没用。不早于这台电脑注册的时候，不晚于现在。
UPDATE agent_room.device AS device
   SET last_seen_at = greatest(device.created_at, least(now(), family.latest - interval '30 days'))
  FROM (
      SELECT device_id, max(expires_at) AS latest
        FROM agent_room.device_token_family
       GROUP BY device_id
  ) AS family
 WHERE family.device_id = device.id
   AND device.last_seen_at IS NULL;
