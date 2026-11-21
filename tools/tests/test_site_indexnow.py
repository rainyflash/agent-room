from __future__ import annotations

import io
import json
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
import tempfile
from typing import Sequence
import unittest
from unittest import mock

from tools import site_indexnow
from tools.site_indexnow import (
    IndexNowFailure,
    changed_urls,
    live_version,
    parse_sitemap,
    payload,
    read_key,
    release_window,
    wait_for_version,
)

SITEMAP = """<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:xhtml="http://www.w3.org/1999/xhtml">
  <url>
    <loc>https://agentroom.chat/</loc>
    <xhtml:link rel="alternate" hreflang="zh-CN" href="https://agentroom.chat/zh/"/>
    <lastmod>2026-11-20</lastmod>
  </url>
  <url>
    <loc>https://agentroom.chat/guides/project-room/</loc>
    <lastmod>2026-11-16</lastmod>
  </url>
  <url>
    <loc>https://agentroom.chat/guides/about-the-name/</loc>
    <lastmod>2026-10-20</lastmod>
  </url>
  <url>
    <loc>https://agentroom.chat/llms.txt</loc>
  </url>
</urlset>
"""



class _Clock:
    """测试用的时钟：sleep 只把时间往前拨。"""

    def __init__(self) -> None:
        self.now = 0.0

    def __call__(self) -> float:
        return self.now

    def sleep(self, seconds: float) -> None:
        self.now += seconds


class SiteIndexNowTests(unittest.TestCase):
    def test_the_repository_key_has_the_indexnow_shape(self) -> None:
        key = read_key()

        self.assertRegex(key, r"^[A-Za-z0-9-]{8,128}$")

    def test_a_key_file_with_anything_else_in_it_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "indexnow-key.txt"
            path.write_text("not a key!\n", encoding="utf-8")

            with self.assertRaises(IndexNowFailure):
                read_key(path)

    def test_the_sitemap_gives_every_page_with_its_date(self) -> None:
        self.assertEqual(
            parse_sitemap(SITEMAP),
            [
                ("https://agentroom.chat/", "2026-11-20"),
                ("https://agentroom.chat/guides/project-room/", "2026-11-16"),
                ("https://agentroom.chat/guides/about-the-name/", "2026-10-20"),
                ("https://agentroom.chat/llms.txt", None),
            ],
        )

    def test_pages_changed_on_or_after_the_previous_release_day_are_announced(self) -> None:
        entries = parse_sitemap(SITEMAP)

        self.assertEqual(
            changed_urls(entries, "2026-11-16T08:24:51Z"),
            [
                "https://agentroom.chat/",
                "https://agentroom.chat/guides/project-room/",
                "https://agentroom.chat/llms.txt",
            ],
        )

    def test_without_a_start_every_page_is_announced(self) -> None:
        self.assertEqual(len(changed_urls(parse_sitemap(SITEMAP), None)), 4)

    def test_pages_on_other_hosts_are_left_out(self) -> None:
        entries = [("https://app.agentroom.chat/rooms", None), ("https://agentroom.chat/zh/", None)]

        self.assertEqual(changed_urls(entries, None), ["https://agentroom.chat/zh/"])

    def test_the_payload_points_at_the_key_file_on_the_site(self) -> None:
        body = payload("0123456789abcdef", ["https://agentroom.chat/"])

        self.assertEqual(
            body,
            {
                "host": "agentroom.chat",
                "key": "0123456789abcdef",
                "keyLocation": "https://agentroom.chat/0123456789abcdef.txt",
                "urlList": ["https://agentroom.chat/"],
            },
        )

    def test_the_release_window_is_the_newest_version_and_the_time_of_the_one_before(self) -> None:
        calls: list[Sequence[str]] = []

        def gh(arguments: Sequence[str]) -> str:
            calls.append(arguments)
            return json.dumps(
                [
                    {"tagName": "v0.1.0-alpha.101", "publishedAt": "2026-11-21T16:14:34Z"},
                    {"tagName": "v0.1.0-alpha.100", "publishedAt": "2026-11-20T08:24:51Z"},
                ]
            )

        self.assertEqual(
            release_window("rainyflash/agent-room", gh),
            ("0.1.0-alpha.101", "2026-11-20T08:24:51Z"),
        )
        self.assertIn("--exclude-pre-releases", calls[0])
        self.assertIn("--exclude-drafts", calls[0])

    def test_with_only_one_public_release_there_is_no_start(self) -> None:
        def gh(arguments: Sequence[str]) -> str:
            return json.dumps([{"tagName": "v0.1.0", "publishedAt": "2026-11-20T08:24:51Z"}])

        self.assertEqual(release_window("rainyflash/agent-room", gh), ("0.1.0", None))

    def test_the_live_version_comes_from_the_home_page_structured_data(self) -> None:
        page = '<script type="application/ld+json">[{"softwareVersion":"0.1.0-alpha.101"}]</script>'

        self.assertEqual(live_version(page), "0.1.0-alpha.101")
        self.assertIsNone(live_version("<html></html>"))

    def test_waiting_stops_as_soon_as_the_site_shows_the_release(self) -> None:
        pages = iter(
            [
                '{"softwareVersion":"0.1.0-alpha.100"}',
                '{"softwareVersion":"0.1.0-alpha.100"}',
                '{"softwareVersion":"0.1.0-alpha.101"}',
            ]
        )
        clock = _Clock()
        with redirect_stdout(io.StringIO()):
            live = wait_for_version(
                "0.1.0-alpha.101",
                45 * 60,
                fetch=lambda url: next(pages),
                sleep=clock.sleep,
                clock=clock,
            )

        self.assertTrue(live)
        self.assertEqual(clock.now, 120)

    def test_waiting_gives_up_at_the_deadline_and_an_unreadable_page_is_not_live(self) -> None:
        def unreadable(url: str) -> str:
            raise IndexNowFailure(f"{url} 打不开")

        clock = _Clock()
        with redirect_stdout(io.StringIO()):
            live = wait_for_version(
                "0.1.0-alpha.101", 5 * 60, fetch=unreadable, sleep=clock.sleep, clock=clock
            )

        self.assertFalse(live)
        self.assertLessEqual(clock.now, 5 * 60)

    def test_a_release_run_waits_for_the_new_site_before_reading_the_sitemap(self) -> None:
        key = read_key()
        pages = {
            "https://agentroom.chat/sitemap.xml": SITEMAP,
            f"https://agentroom.chat/{key}.txt": key,
        }
        order: list[str] = []

        def wait(version: str, timeout_seconds: float) -> bool:
            order.append(f"wait {version} {timeout_seconds:.0f}")
            return True

        def fetch(url: str) -> str:
            order.append(url)
            return pages[url]

        with (
            mock.patch.object(
                site_indexnow,
                "release_window",
                return_value=("0.1.0-alpha.101", "2026-11-20T08:24:51Z"),
            ),
            mock.patch.object(site_indexnow, "wait_for_version", side_effect=wait),
            mock.patch.object(site_indexnow, "fetch_text", side_effect=fetch),
            mock.patch.object(site_indexnow, "submit", return_value=200) as submit,
            redirect_stdout(io.StringIO()),
        ):
            status = site_indexnow.main(["--repository", "rainyflash/agent-room"])

        self.assertEqual(status, 0)
        self.assertEqual(
            order[:2], ["wait 0.1.0-alpha.101 2700", "https://agentroom.chat/sitemap.xml"]
        )
        submit.assert_called_once_with(
            payload(key, ["https://agentroom.chat/", "https://agentroom.chat/llms.txt"])
        )

    def test_a_release_run_reports_nothing_while_the_old_site_is_still_up(self) -> None:
        stderr = io.StringIO()
        with (
            mock.patch.object(
                site_indexnow,
                "release_window",
                return_value=("0.1.0-alpha.101", "2026-11-20T08:24:51Z"),
            ),
            mock.patch.object(site_indexnow, "wait_for_version", return_value=False),
            mock.patch.object(site_indexnow, "fetch_text") as fetch,
            mock.patch.object(site_indexnow, "submit") as submit,
            redirect_stderr(stderr),
        ):
            status = site_indexnow.main(["--repository", "rainyflash/agent-room"])

        self.assertEqual(status, 1)
        fetch.assert_not_called()
        submit.assert_not_called()
        self.assertIn("0.1.0-alpha.101", stderr.getvalue())

    def test_nothing_is_sent_while_the_live_key_file_is_missing(self) -> None:
        key = read_key()
        pages = {
            "https://agentroom.chat/sitemap.xml": SITEMAP,
            f"https://agentroom.chat/{key}.txt": "an older key",
        }
        stderr = io.StringIO()
        with (
            mock.patch.object(site_indexnow, "fetch_text", side_effect=pages.__getitem__),
            mock.patch.object(site_indexnow, "submit") as submit,
            redirect_stdout(io.StringIO()),
            redirect_stderr(stderr),
        ):
            status = site_indexnow.main(["--all"])

        self.assertEqual(status, 1)
        submit.assert_not_called()
        self.assertIn(f"{key}.txt", stderr.getvalue())

    def test_changed_pages_are_sent_once_and_an_accepted_answer_is_success(self) -> None:
        key = read_key()
        pages = {
            "https://agentroom.chat/sitemap.xml": SITEMAP,
            f"https://agentroom.chat/{key}.txt": key,
        }
        stdout = io.StringIO()
        with (
            mock.patch.object(site_indexnow, "fetch_text", side_effect=pages.__getitem__),
            mock.patch.object(site_indexnow, "submit", return_value=202) as submit,
            redirect_stdout(stdout),
        ):
            status = site_indexnow.main(["--since", "2026-11-17T00:00:00Z"])

        self.assertEqual(status, 0)
        submit.assert_called_once_with(
            payload(key, ["https://agentroom.chat/", "https://agentroom.chat/llms.txt"])
        )
        self.assertIn("IndexNow 回 202", stdout.getvalue())

    def test_a_refused_announcement_fails_the_run(self) -> None:
        key = read_key()
        pages = {
            "https://agentroom.chat/sitemap.xml": SITEMAP,
            f"https://agentroom.chat/{key}.txt": key,
        }
        with (
            mock.patch.object(site_indexnow, "fetch_text", side_effect=pages.__getitem__),
            mock.patch.object(site_indexnow, "submit", return_value=403),
            redirect_stdout(io.StringIO()),
        ):
            self.assertEqual(site_indexnow.main(["--all"]), 1)

    def test_a_dry_run_lists_the_pages_and_sends_nothing(self) -> None:
        stdout = io.StringIO()
        with (
            mock.patch.object(site_indexnow, "fetch_text", return_value=SITEMAP),
            mock.patch.object(site_indexnow, "submit") as submit,
            redirect_stdout(stdout),
        ):
            status = site_indexnow.main(["--all", "--dry-run"])

        self.assertEqual(status, 0)
        submit.assert_not_called()
        self.assertIn("https://agentroom.chat/guides/project-room/", stdout.getvalue())


if __name__ == "__main__":
    unittest.main()
