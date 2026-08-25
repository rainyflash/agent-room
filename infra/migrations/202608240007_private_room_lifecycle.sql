CREATE TABLE agent_room.private_room_state (
    catalog_entry_id uuid PRIMARY KEY
        REFERENCES agent_room.room_catalog_entry(id) ON DELETE CASCADE,
    room_instance_id uuid NOT NULL UNIQUE
        REFERENCES agent_room.room_instance(id) ON DELETE CASCADE,
    version bigint NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    CONSTRAINT private_room_state_version_nonnegative CHECK (version >= 0),
    CONSTRAINT private_room_state_timestamp_order CHECK (updated_at >= created_at)
);

ALTER TABLE agent_room.room_catalog_entry
    ADD CONSTRAINT room_catalog_entry_private_visibility CHECK (
        kind <> 'private_room' OR visibility IN ('unlisted', 'private')
    );

CREATE TABLE agent_room.private_room_membership (
    catalog_entry_id uuid NOT NULL
        REFERENCES agent_room.private_room_state(catalog_entry_id) ON DELETE CASCADE,
    principal_id uuid NOT NULL REFERENCES agent_room.principal(id),
    membership_status text NOT NULL,
    permission_bits smallint NOT NULL,
    created_at timestamptz NOT NULL,
    status_changed_at timestamptz NOT NULL,
    PRIMARY KEY (catalog_entry_id, principal_id),
    CONSTRAINT private_room_membership_status CHECK (
        membership_status IN ('invited', 'joined', 'declined', 'removed', 'banned')
    ),
    CONSTRAINT private_room_membership_permission_bits CHECK (permission_bits BETWEEN 0 AND 31),
    CONSTRAINT private_room_membership_permission_dependencies CHECK (
        (permission_bits = 0 OR (permission_bits & 1) = 1)
        AND ((permission_bits & 16) = 0 OR (permission_bits & 2) = 2)
    ),
    CONSTRAINT private_room_membership_state_permissions CHECK (
        (
            membership_status IN ('invited', 'joined')
            AND (permission_bits & 1) = 1
        )
        OR (
            membership_status IN ('declined', 'removed', 'banned')
            AND permission_bits = 0
        )
    ),
    CONSTRAINT private_room_membership_timestamp_order CHECK (
        status_changed_at >= created_at
    )
);

CREATE INDEX private_room_membership_principal_idx
    ON agent_room.private_room_membership (principal_id, membership_status);

CREATE FUNCTION agent_room.enforce_private_room_integrity()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    target_catalog_id uuid;
    catalog_kind text;
    catalog_owner_id uuid;
    catalog_status text;
    state_instance_id uuid;
    instance_catalog_id uuid;
    instance_status text;
    owner_status text;
    owner_permissions smallint;
BEGIN
    IF TG_TABLE_NAME = 'room_catalog_entry' THEN
        target_catalog_id := CASE WHEN TG_OP = 'DELETE' THEN OLD.id ELSE NEW.id END;
    ELSIF TG_TABLE_NAME = 'room_instance' THEN
        target_catalog_id := CASE
            WHEN TG_OP = 'DELETE' THEN OLD.catalog_entry_id
            ELSE NEW.catalog_entry_id
        END;
    ELSE
        target_catalog_id := CASE
            WHEN TG_OP = 'DELETE' THEN OLD.catalog_entry_id
            ELSE NEW.catalog_entry_id
        END;
    END IF;

    SELECT catalog.kind, catalog.owner_principal_id, catalog.status
    INTO catalog_kind, catalog_owner_id, catalog_status
    FROM agent_room.room_catalog_entry AS catalog
    WHERE catalog.id = target_catalog_id;

    IF NOT FOUND THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;

    IF catalog_kind <> 'private_room' THEN
        IF EXISTS (
            SELECT 1
            FROM agent_room.private_room_state AS state
            WHERE state.catalog_entry_id = target_catalog_id
        ) THEN
            RAISE EXCEPTION 'private room facts require a private_room catalog';
        END IF;
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;

    SELECT state.room_instance_id, instance.catalog_entry_id, instance.state
    INTO state_instance_id, instance_catalog_id, instance_status
    FROM agent_room.private_room_state AS state
    JOIN agent_room.room_instance AS instance ON instance.id = state.room_instance_id
    WHERE state.catalog_entry_id = target_catalog_id;

    IF NOT FOUND OR instance_catalog_id <> target_catalog_id THEN
        RAISE EXCEPTION 'private room state and Matrix instance must share one catalog';
    END IF;

    IF NOT (
        (catalog_status = 'active' AND instance_status = 'active')
        OR (catalog_status = 'archived' AND instance_status = 'archived')
    ) THEN
        RAISE EXCEPTION 'private room catalog and Matrix instance lifecycle must match';
    END IF;

    SELECT membership.membership_status, membership.permission_bits
    INTO owner_status, owner_permissions
    FROM agent_room.private_room_membership AS membership
    WHERE membership.catalog_entry_id = target_catalog_id
      AND membership.principal_id = catalog_owner_id;

    IF NOT FOUND OR owner_status <> 'joined' OR owner_permissions <> 31 THEN
        RAISE EXCEPTION 'private room owner must be a joined member with all permissions';
    END IF;

    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$;

CREATE CONSTRAINT TRIGGER room_catalog_private_integrity
AFTER INSERT OR UPDATE ON agent_room.room_catalog_entry
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW
EXECUTE FUNCTION agent_room.enforce_private_room_integrity();

CREATE CONSTRAINT TRIGGER room_instance_private_integrity
AFTER INSERT OR UPDATE OR DELETE ON agent_room.room_instance
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW
EXECUTE FUNCTION agent_room.enforce_private_room_integrity();

CREATE CONSTRAINT TRIGGER private_room_state_integrity
AFTER INSERT OR UPDATE OR DELETE ON agent_room.private_room_state
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW
EXECUTE FUNCTION agent_room.enforce_private_room_integrity();

CREATE CONSTRAINT TRIGGER private_room_membership_integrity
AFTER INSERT OR UPDATE OR DELETE ON agent_room.private_room_membership
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW
EXECUTE FUNCTION agent_room.enforce_private_room_integrity();
