from __future__ import annotations

from pathlib import Path
import tempfile
import unittest

from tools.federation import (
    ALPHA,
    BETA,
    FederationFailure,
    encoded,
    joined_room_events,
    read_environment,
    room_membership,
    synapse_configuration,
)


class FederationConfigurationTests(unittest.TestCase):
    def test_environment_parser_rejects_empty_and_malformed_values(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "runtime.env"
            path.write_text("VALID=value\nBROKEN=\n", encoding="utf-8")
            with self.assertRaisesRegex(FederationFailure, "空值或无效行"):
                read_environment(path)

    def test_homeservers_keep_database_identity_and_trust_roots_isolated(self) -> None:
        values = {
            "ALPHA_DATABASE_PASSWORD": "alpha-database-secret",
            "BETA_DATABASE_PASSWORD": "beta-database-secret",
            "ALPHA_REGISTRATION_SECRET": "alpha-registration-secret",
            "BETA_REGISTRATION_SECRET": "beta-registration-secret",
            "ALPHA_MACAROON_SECRET": "alpha-macaroon-secret",
            "BETA_MACAROON_SECRET": "beta-macaroon-secret",
            "ALPHA_FORM_SECRET": "alpha-form-secret",
            "BETA_FORM_SECRET": "beta-form-secret",
        }

        alpha = synapse_configuration(ALPHA, BETA, values)
        beta = synapse_configuration(BETA, ALPHA, values)

        self.assertIn('server_name: "alpha.agent-room.test"', alpha)
        self.assertIn("host: postgres-alpha", alpha)
        self.assertIn('  - "beta.agent-room.test"', alpha)
        self.assertNotIn("beta-database-secret", alpha)
        self.assertIn('server_name: "beta.agent-room.test"', beta)
        self.assertIn("host: postgres-beta", beta)
        self.assertIn('  - "alpha.agent-room.test"', beta)
        self.assertNotIn("alpha-database-secret", beta)
        self.assertIn("federation_custom_ca_list:", alpha)
        self.assertIn("federation_custom_ca_list:", beta)


class FederationSyncParsingTests(unittest.TestCase):
    def test_timeline_and_membership_only_accept_expected_object_shapes(self) -> None:
        room_id = "!room:alpha.agent-room.test"
        response: dict[str, object] = {
            "rooms": {
                "join": {
                    room_id: {
                        "timeline": {
                            "events": [
                                {"event_id": "$valid"},
                                "拒绝非对象事件",
                            ]
                        }
                    }
                }
            }
        }

        self.assertTrue(room_membership(response, room_id, "join"))
        self.assertFalse(room_membership(response, room_id, "invite"))
        self.assertEqual(joined_room_events(response, room_id), [{"event_id": "$valid"}])
        self.assertEqual(encoded("!room:alpha/test"), "%21room%3Aalpha%2Ftest")


if __name__ == "__main__":
    unittest.main()
