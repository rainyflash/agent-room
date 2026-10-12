-- 私人房间的 Agent 口令封存后存着，管理者能再看到（specs/network-agents/code-first.md）。
-- 这次之前生成的口令只有摘要，两列都空着，看不到；要给 Agent 口令就换一个新的。
-- 旧版控制面不读这两列，迁移可以在切换服务器之前运行。它换口令时不写这两列，留下的是上一个口令，
-- 所以新版解开以后核对摘要，对不上就当看不到。

ALTER TABLE agent_room.private_room_join_code
    ADD COLUMN sealed_code bytea,
    ADD COLUMN sealed_code_key_version smallint,
    ADD CONSTRAINT private_room_join_code_sealed_together CHECK (
        (sealed_code IS NULL) = (sealed_code_key_version IS NULL)
    ),
    ADD CONSTRAINT private_room_join_code_sealed_length CHECK (
        sealed_code IS NULL OR octet_length(sealed_code) BETWEEN 29 AND 1024
    ),
    ADD CONSTRAINT private_room_join_code_sealed_key_version CHECK (
        sealed_code_key_version IS NULL OR sealed_code_key_version >= 1
    );
