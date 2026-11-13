from __future__ import annotations

from pathlib import Path
import tempfile
import tomllib
import unittest

from tools.vendored_advisories import (
    ROOT,
    VendoredAdvisoryError,
    database_advisories,
    load_vendored,
    unreviewed,
)


WORKFLOWS = ROOT / ".github" / "workflows"


def advisory_file(advisory_id: str, *, withdrawn: bool = False) -> str:
    withdrawn_line = 'withdrawn = "2026-11-12"\n' if withdrawn else ""
    return (
        "```toml\n[advisory]\n"
        f'id = "{advisory_id}"\npackage = "demo"\nurl = "https://example.invalid/{advisory_id}"\n'
        f"{withdrawn_line}\n[versions]\npatched = [\">= 2.0.0\"]\n```\n\n# 标题 {advisory_id}\n\n正文。\n"
    )


class FakeRepository:
    def __init__(self, base: Path) -> None:
        self.root = base / "repo"
        self.database = base / "advisory-db-test"
        (self.root / "third_party" / "demo").mkdir(parents=True)
        (self.database / "crates" / "demo").mkdir(parents=True)
        self.write_manifest("1.0.0")

    def write_manifest(self, version: str) -> None:
        (self.root / "third_party" / "demo" / "Cargo.toml").write_text(
            f'[package]\nname = "demo"\nversion = "{version}"\n', encoding="utf-8"
        )

    def write_vendored(self, *advisories: tuple[str, str]) -> Path:
        lines = ['[[crate]]\nname = "demo"\nversion = "1.0.0"\npath = "third_party/demo"\n']
        for advisory_id, status in advisories:
            lines.append(f'[[crate.advisory]]\nid = "{advisory_id}"\nstatus = "{status}"\nnote = "看过了。"\n')
        path = self.root / "third_party" / "vendored.toml"
        path.write_text("\n".join(lines), encoding="utf-8")
        return path

    def publish(self, advisory_id: str, *, withdrawn: bool = False) -> None:
        (self.database / "crates" / "demo" / f"{advisory_id}.md").write_text(
            advisory_file(advisory_id, withdrawn=withdrawn), encoding="utf-8"
        )


class VendoredAdvisoryTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.repo = FakeRepository(Path(directory.name))

    def check(self, vendored: Path) -> list[str]:
        crates = load_vendored(vendored, self.repo.root)
        return [advisory.id for _, advisory in unreviewed(crates, (self.repo.database,))]

    def test_reviewed_advisories_pass(self) -> None:
        self.repo.publish("RUSTSEC-2026-0001")
        self.repo.publish("RUSTSEC-2026-0002")
        vendored = self.repo.write_vendored(("RUSTSEC-2026-0001", "patched"), ("RUSTSEC-2026-0002", "unaffected"))

        self.assertEqual(self.check(vendored), [])

    def test_new_advisory_is_reported_with_its_title(self) -> None:
        self.repo.publish("RUSTSEC-2026-0001")
        self.repo.publish("RUSTSEC-2026-0003")
        vendored = self.repo.write_vendored(("RUSTSEC-2026-0001", "patched"))

        self.assertEqual(self.check(vendored), ["RUSTSEC-2026-0003"])
        advisories = database_advisories(self.repo.database, "demo")
        self.assertEqual(advisories[1].title, "标题 RUSTSEC-2026-0003")
        self.assertEqual(advisories[1].url, "https://example.invalid/RUSTSEC-2026-0003")

    def test_withdrawn_advisory_is_ignored(self) -> None:
        self.repo.publish("RUSTSEC-2026-0004", withdrawn=True)

        self.assertEqual(self.check(self.repo.write_vendored()), [])

    def test_bumped_version_must_be_reviewed_again(self) -> None:
        self.repo.write_manifest("1.0.1")

        with self.assertRaisesRegex(VendoredAdvisoryError, "对不上"):
            load_vendored(self.repo.write_vendored(), self.repo.root)

    def test_status_must_be_known(self) -> None:
        with self.assertRaisesRegex(VendoredAdvisoryError, "patched 或 unaffected"):
            load_vendored(self.repo.write_vendored(("RUSTSEC-2026-0001", "ignored")), self.repo.root)

    def test_missing_database_fails(self) -> None:
        crates = load_vendored(self.repo.write_vendored(), self.repo.root)

        with self.assertRaisesRegex(VendoredAdvisoryError, "cargo deny check"):
            unreviewed(crates, ())
        with self.assertRaisesRegex(VendoredAdvisoryError, "不像 RustSec 公告库"):
            unreviewed(crates, (self.repo.root,))


class RepositoryWiringTests(unittest.TestCase):
    def test_every_patched_crate_is_listed_and_excluded(self) -> None:
        manifest = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
        patches = manifest.get("patch", {}).get("crates-io", {})
        vendored = tomllib.loads((ROOT / "third_party" / "vendored.toml").read_text(encoding="utf-8"))
        listed = {entry["name"]: entry["path"] for entry in vendored["crate"]}

        self.assertEqual({name: entry["path"] for name, entry in patches.items()}, listed)
        self.assertEqual(sorted(manifest["workspace"].get("exclude", [])), sorted(listed.values()))
        self.assertEqual([crate.name for crate in load_vendored()], sorted(listed))

    def test_check_runs_after_cargo_deny(self) -> None:
        for workflow in ("advisories.yml", "ci.yml"):
            text = (WORKFLOWS / workflow).read_text(encoding="utf-8")
            deny = text.index("run: cargo deny check")
            self.assertIn("tools/vendored_advisories.py", text[deny : deny + 400], workflow)

    def test_pull_requests_touching_vendored_code_run_the_check(self) -> None:
        text = (WORKFLOWS / "advisories.yml").read_text(encoding="utf-8")
        trigger = text[text.index("  pull_request:") : text.index("  workflow_dispatch:")]

        for path in ("third_party/**", "tools/vendored_advisories.py"):
            self.assertIn(f"- {path}", trigger)


if __name__ == "__main__":
    unittest.main()
