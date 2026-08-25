import unittest

from tools.mcp_client import McpClientFailure, parse_json_object


class McpResponseValidationTests(unittest.TestCase):
    def test_只接受_json_对象(self) -> None:
        self.assertEqual(parse_json_object('{"ok":true}', "响应"), {"ok": True})
        with self.assertRaises(McpClientFailure):
            parse_json_object("[]", "响应")

    def test_拒绝损坏的_json(self) -> None:
        with self.assertRaises(McpClientFailure):
            parse_json_object("{", "响应")


if __name__ == "__main__":
    unittest.main()
