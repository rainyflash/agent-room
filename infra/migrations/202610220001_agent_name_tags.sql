-- Agent 名字后面的四位编号（specs/agent-name-tag/design.md）：给已有的 Agent 补上。
-- 编号取 ID 末尾 32 位除以 10000 的余数，和 crates/domain/src/agent_names.rs 一样；
-- 名字先去掉末尾已有的 ` #` 加四位数字，原名截到 74 个字符，整个名字最多 80 个字符。
-- 名片不用另外改：本机 Bridge 和网关连上时发现名字变了会自己重写一次。

ALTER TABLE agent_room.network_agent
    DROP CONSTRAINT network_agent_display_name_length;
ALTER TABLE agent_room.network_agent
    ADD CONSTRAINT network_agent_display_name_length
    CHECK (length(display_name) BETWEEN 1 AND 80);

DO $$
DECLARE
    agent record;
    tag integer;
    candidate text;
BEGIN
    -- 网络 Agent 的名字记在三处，写同一个；没停用的全站不重名，撞上了往后挪一个号。
    FOR agent IN
        SELECT id, principal_id, agent_id, display_name
          FROM agent_room.network_agent
         WHERE status <> 'disabled'
         ORDER BY created_at, id
    LOOP
        tag := (('x' || lpad(right(replace(agent.id::text, '-', ''), 8), 16, '0'))::bit(64)::bigint
                % 10000)::integer;
        LOOP
            candidate := rtrim(left(rtrim(regexp_replace(btrim(agent.display_name), ' +#[0-9]{4}$', '')), 74))
                || ' #' || lpad(tag::text, 4, '0');
            EXIT WHEN NOT EXISTS (
                SELECT 1
                  FROM agent_room.network_agent other
                 WHERE other.status <> 'disabled'
                   AND other.id <> agent.id
                   AND lower(other.display_name) = lower(candidate)
            );
            tag := (tag + 1) % 10000;
        END LOOP;
        UPDATE agent_room.network_agent SET display_name = candidate WHERE id = agent.id;
        UPDATE agent_room.principal SET display_name = candidate WHERE id = agent.principal_id;
        IF agent.agent_id IS NOT NULL THEN
            UPDATE agent_room.agent
               SET display_name = candidate, updated_at = greatest(updated_at, now())
             WHERE id = agent.agent_id;
        END IF;
    END LOOP;
END
$$;

-- 其他 Agent（本机的、默认 Agent）按 Agent ID 算编号。
UPDATE agent_room.agent
   SET display_name = rtrim(left(rtrim(regexp_replace(btrim(display_name), ' +#[0-9]{4}$', '')), 74))
           || ' #'
           || lpad(((('x' || lpad(right(replace(id::text, '-', ''), 8), 16, '0'))::bit(64)::bigint
                   % 10000)::integer)::text, 4, '0'),
       updated_at = greatest(updated_at, now())
 WHERE lifecycle_state <> 'retired'
   AND NOT EXISTS (
       SELECT 1 FROM agent_room.network_agent network WHERE network.agent_id = agent.id
   );
