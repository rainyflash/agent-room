-- 去掉没人读的 Outbox（specs/outbox/design.md 第 3 步）：Alpha 87（#637）起控制面不再写这张表，
-- 也从来没有发布器读过它。表上的约束和索引跟着一起删。
-- 回滚只换镜像、不退数据库：Alpha 86 及更早的控制面建 Agent、登记连接、撤销电脑时还在同一个事务里写它，
-- 这个迁移上线以后不能再回到那些版本。

DROP TABLE agent_room.outbox_event;
