"""Protect the release probe against JSON/SSE transport changes and invalid replies."""
import json
import unittest

from tools.agent_runtime_acceptance import MAX_RESPONSE_BYTES, decode_rpc_response


class McpResponseTests(unittest.TestCase):
    def test_json_response(self):
        response = {"jsonrpc": "2.0", "id": 1, "result": {"tools": []}}
        self.assertEqual(decode_rpc_response(json.dumps(response).encode(), "application/json; charset=utf-8", 1), response)

    def test_sse_ignores_keepalive_and_progress_before_result(self):
        response = {"jsonrpc": "2.0", "id": 1, "result": {"tools": [{"name": "agent_room_wait_for_messages"}]}}
        payload = (
            ': keep-alive\r\n\r\n'
            'event: message\r\ndata: {"jsonrpc":"2.0","method":"notifications/progress","params":{"progress":0}}\r\n\r\n'
            f'event: message\r\nid: event-1\r\ndata: {json.dumps(response)}\r\n\r\n'
        ).encode()
        self.assertEqual(decode_rpc_response(payload, "text/event-stream; charset=utf-8", 1), response)

    def test_sse_multiline_data_and_error_response(self):
        payload = b'event: message\ndata: {"jsonrpc":"2.0","id":1,\ndata: "error":{"code":-32602,"message":"Invalid params"}}\n\n'
        response = decode_rpc_response(payload, "text/event-stream", 1)
        self.assertEqual(response["error"]["code"], -32602)

    def test_rejects_empty_truncated_duplicate_or_mismatched_results(self):
        result = b'data: {"jsonrpc":"2.0","id":1,"result":{}}\n\n'
        invalid = [b': keep-alive\n\n', result.rstrip(b'\n'), result + result,
                   b'data: {"jsonrpc":"2.0","id":2,"result":{}}\n\n',
                   b'data: {"jsonrpc":"2.0","id":true,"result":{}}\n\n',
                   b'data: []\n\n', b'data: not-json\n\n',
                   b'data: {"jsonrpc":"2.0","id":1,"result":{},"error":{}}\n\n']
        for payload in invalid:
            with self.subTest(payload=payload), self.assertRaises(ValueError):
                decode_rpc_response(payload, "text/event-stream", 1)

    def test_rejects_unsupported_or_oversized_body(self):
        with self.assertRaises(ValueError):
            decode_rpc_response(b'{}', "text/html", 1)
        with self.assertRaises(ValueError):
            decode_rpc_response(b' ' * (MAX_RESPONSE_BYTES + 1), "application/json", 1)


if __name__ == "__main__":
    unittest.main()
