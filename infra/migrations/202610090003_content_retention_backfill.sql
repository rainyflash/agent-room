-- 已经存着的消息正文和附件，补设按房间保留期算的到期时间（隐私说明页，接 202610080001）。
-- 算法和新上传的一样：上传的时间加房间的保留期，再多留一天等聊天服务器每天那次清理；
-- 目录上没设保留期、或者查不到房间的，按聊天服务器默认的 30 天。
-- 补完以后由正文回收按过期删掉（存储对象和记录一起）：已经过了的下一轮就删，没过的到时候删。
-- 只补还在用的（active）、绑在房间上的；被头像引用的不补。客户端看到的 expires_at 不变。
UPDATE agent_room.content_object AS content
   SET retention_expires_at = content.created_at + make_interval(days => 1 + coalesce((
           SELECT max(catalog.retention_days)
             FROM agent_room.content_access_policy AS policy
             JOIN agent_room.room_instance AS instance
               ON instance.matrix_room_id = policy.matrix_room_id
             JOIN agent_room.room_catalog_entry AS catalog
               ON catalog.id = instance.catalog_entry_id
            WHERE policy.content_id = content.id
       ), 30))
 WHERE content.lifecycle_state = 'active'
   AND content.retention_expires_at IS NULL
   AND EXISTS (
       SELECT 1 FROM agent_room.content_access_policy AS policy
        WHERE policy.content_id = content.id
   )
   AND NOT EXISTS (
       SELECT 1 FROM agent_room.principal WHERE avatar_content_id = content.id
   )
   AND NOT EXISTS (
       SELECT 1 FROM agent_room.agent WHERE avatar_content_id = content.id
   );
