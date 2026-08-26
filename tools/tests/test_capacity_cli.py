from __future__ import annotations

import contextlib
import io
import unittest

from tools.capacity_database import parse_args as parse_database_args
from tools.capacity_web import (
    allocate_available_loopback_port,
    parse_args as parse_web_args,
)
from tools.object_store import parse_args as parse_object_store_args


class CapacityCommandLineTests(unittest.TestCase):
    def test_web_capacity_uses_an_operating_system_selected_port(self) -> None:
        port = allocate_available_loopback_port()

        self.assertGreaterEqual(port, 1)
        self.assertLessEqual(port, 65_535)

    def test_no_argument_capacity_commands_accept_empty_argument_list(self) -> None:
        for parser in (
            parse_database_args,
            parse_object_store_args,
            parse_web_args,
        ):
            with self.subTest(parser=parser.__module__):
                self.assertIsNotNone(parser([]))

    def test_no_argument_capacity_commands_reject_unknown_arguments(self) -> None:
        for parser in (
            parse_database_args,
            parse_object_store_args,
            parse_web_args,
        ):
            with self.subTest(parser=parser.__module__):
                with contextlib.redirect_stderr(io.StringIO()):
                    with self.assertRaises(SystemExit) as raised:
                        parser(["--unknown"])
                self.assertEqual(raised.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
