#!/usr/bin/env python3
"""Exercise the built Linux image through a verified TLS reverse proxy.

This checks packaging and MCP transport, not a live Matrix conversation. The
receiver's message/receipt contract is covered separately by its runtime tests.
"""
from __future__ import annotations

import argparse
from http.client import HTTPConnection, HTTPSConnection
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import secrets
import ssl
import subprocess
import tempfile
import threading
import time
from typing import Sequence


def checked(command: Sequence[str]) -> str:
    result = subprocess.run(command, check=True, capture_output=True, text=True, timeout=60)
    return result.stdout.strip()


class Proxy(ThreadingHTTPServer):
    backend_port: int = 0


class Handler(BaseHTTPRequestHandler):
    server: Proxy

    def log_message(self, _format: str, *args: object) -> None:
        # Test traffic may carry credentials; access logs are deliberately disabled.
        return

    def do_POST(self) -> None:
        length = int(self.headers.get("Content-Length", "0"))
        if length > 65_536:
            self.send_error(413)
            return
        connection = HTTPConnection("127.0.0.1", self.server.backend_port, timeout=10)
        try:
            connection.request("POST", self.path, self.rfile.read(length), dict(self.headers))
            response = connection.getresponse()
            body = response.read(131_073)
            if len(body) > 131_072:
                self.send_error(502)
                return
            self.send_response(response.status)
            self.send_header("Content-Type", response.getheader("Content-Type", "application/json"))
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        finally:
            connection.close()


def probe(port: int, context: ssl.SSLContext, token: str | None,
          method: str, params: dict[str, object], origin: str | None = None) -> tuple[int, object]:
    connection = HTTPSConnection("127.0.0.1", port, context=context, timeout=10)
    headers = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream"}
    if token is not None:
        headers["Authorization"] = f"Bearer {token}"
    if origin is not None:
        headers["Origin"] = origin
    try:
        connection.request("POST", "/mcp", json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}), headers)
        response = connection.getresponse()
        payload = response.read(131_072)
        if response.status != 200:
            return response.status, None
        return response.status, json.loads(payload)
    finally:
        connection.close()


def accept(image: str, report: Path) -> None:
    if os.name != "posix":
        raise RuntimeError("The image acceptance runs on a Linux Docker host.")
    with tempfile.TemporaryDirectory(prefix="agent-runtime-acceptance-") as temporary:
        directory = Path(temporary)
        token = secrets.token_urlsafe(48)
        token_file = directory / "mcp.token"
        token_file.write_text(token, encoding="utf-8")
        token_file.chmod(0o600)
        os.chown(token_file, 10001, 10001)
        certificate, key = directory / "cert.pem", directory / "key.pem"
        checked(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
                 "-subj", "/CN=127.0.0.1", "-addext", "subjectAltName=IP:127.0.0.1",
                 "-keyout", str(key), "-out", str(certificate)])
        tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        tls.load_cert_chain(certificate, key)
        client_tls = ssl.create_default_context(cafile=str(certificate))
        proxy = Proxy(("127.0.0.1", 0), Handler)
        proxy.socket = tls.wrap_socket(proxy.socket, server_side=True)
        port = proxy.server_port
        container = checked(["docker", "run", "--detach", "--read-only", "--cap-drop=ALL",
            "--security-opt=no-new-privileges", "--publish", "127.0.0.1::8181",
            "--mount", f"type=bind,source={token_file},target=/run/mcp.token,readonly",
            "--stop-signal=SIGINT", image, "agent-room-mcp", "--http", "0.0.0.0:8181",
            "--public-url", f"https://127.0.0.1:{port}/mcp", "--token-file", "/run/mcp.token"])
        thread = None
        try:
            proxy.backend_port = int(checked(["docker", "port", container, "8181/tcp"]).rsplit(":", 1)[1])
            thread = threading.Thread(target=proxy.serve_forever, daemon=True)
            thread.start()
            deadline = time.monotonic() + 20
            while True:
                try:
                    status, _ = probe(port, client_tls, None, "initialize", {})
                    if status == 401:
                        break
                except OSError:
                    if time.monotonic() >= deadline:
                        raise
                if time.monotonic() >= deadline:
                    raise RuntimeError("MCP did not become ready behind TLS.")
                time.sleep(0.25)
            status, response = probe(port, client_tls, token, "initialize", {
                "protocolVersion": "2025-03-26", "capabilities": {},
                "clientInfo": {"name": "image-acceptance", "version": "1"}})
            if status != 200 or not isinstance(response, dict) or "result" not in response:
                raise RuntimeError("MCP initialization failed.")
            status, response = probe(port, client_tls, token, "tools/list", {})
            if status != 200 or not isinstance(response, dict):
                raise RuntimeError("MCP tool listing failed.")
            names = {tool["name"] for tool in response["result"]["tools"]}
            required = {"agent_room_wait_for_messages", "agent_room_send_message", "agent_room_register_reception"}
            if not required <= names:
                raise RuntimeError("Image contains an outdated MCP server.")
            if probe(port, client_tls, "invalid", "tools/list", {})[0] != 401:
                raise RuntimeError("Invalid token accepted.")
            if probe(port, client_tls, token, "tools/list", {}, "https://untrusted.example")[0] != 403:
                raise RuntimeError("Cross-origin request accepted.")
            version = checked(["docker", "run", "--rm", image, "agent-room", "--version"])
            if not version.startswith("agent-room "):
                raise RuntimeError("Image CLI version is unavailable.")
            report.parent.mkdir(parents=True, exist_ok=True)
            report.write_text(json.dumps({"image": image, "cliVersion": version, "tlsVerified": True,
                "mcpInitialized": True, "tools": sorted(names), "tokenRejected": True,
                "originRejected": True, "matrixDeliveryTested": False}, indent=2) + "\n", encoding="utf-8")
        finally:
            checked(["docker", "rm", "--force", container])
            if thread is not None:
                proxy.shutdown()
                thread.join(timeout=5)
            proxy.server_close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    accept(args.image, args.report)
