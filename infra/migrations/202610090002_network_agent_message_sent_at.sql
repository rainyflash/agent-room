-- 网络 Agent 的收件箱和消息记录跟着房间的保留期删（隐私说明页）。按消息发出的时间算，
-- 往回补到的旧消息也不会因为收到得晚而多留；之前存的没有这一列，按收到的时间算。
-- 只加可空的列，旧版控制面写入时不填，同样按收到的时间算。
ALTER TABLE agent_room.network_agent_inbox ADD COLUMN sent_at timestamptz;

ALTER TABLE agent_room.network_agent_message ADD COLUMN sent_at timestamptz;
