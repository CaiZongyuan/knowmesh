"""Exercise the baseline runner through its CLI and the real KnowMesh binary."""

import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import unittest


HERE = Path(__file__).resolve().parent
BINARY = Path(os.environ.get("KNOWMESH_BIN", HERE.parents[1] / "target/debug/knowmesh")).resolve()


class BaselineTest(unittest.TestCase):
    def test_acquisition_and_import_failures_are_locatable_and_do_not_stop_other_materials(self):
        content = b"# Research\n\nAcquired material.\n"

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                if self.path == "/missing":
                    self.send_error(404)
                    return
                self.send_response(200)
                self.end_headers()
                self.wfile.write(content)

            def log_message(self, *args):
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            with tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                url = f"http://127.0.0.1:{server.server_port}"
                materials = []
                for name, extension, endpoint, checksum in [
                    ("missing", "md", "/missing", hashlib.sha256(content).hexdigest()),
                    ("changed", "md", "/paper", "0" * 64),
                    ("invalid", "pdf", "/paper", hashlib.sha256(content).hexdigest()),
                    ("valid", "md", "/paper", hashlib.sha256(content).hexdigest()),
                ]:
                    materials.append({"id": name, "filename": f"{name}.{extension}",
                                      "title": "Research", "sha256": checksum,
                                      "acquisition_url": url + endpoint,
                                      "queries": [{"query": "Research", "kind": "metadata"}]})
                manifest = root / "inventory.json"
                manifest.write_text(json.dumps({"materials": materials}))
                report = root / "report.json"
                argv = [sys.executable, str(HERE / "baseline.py"), "--binary", str(BINARY),
                        "--inventory", str(manifest), "--inputs", str(root), "--report", str(report)]
                result = subprocess.run([*argv, "--acquire"], capture_output=True, text=True)
                self.assertEqual(result.returncode, 1, result.stderr)
                data = json.loads(report.read_text())
                self.assertEqual(data["status"], "failed")
                self.assertEqual([m.get("failure_stage") for m in data["materials"]],
                                 ["acquisition", "hash", "import", None])
                self.assertIn("404", data["materials"][0]["error"])
                self.assertEqual(data["materials"][1]["sha256"], hashlib.sha256(content).hexdigest())
                self.assertIn("SOURCE_MIME_MISMATCH", data["materials"][2]["error"])
                self.assertEqual(data["materials"][3]["status"], "passed")
                self.assertFalse((root / "changed.md").exists())
                (root / "valid.md").write_bytes(b"Changed after acquisition")
                result = subprocess.run(argv, capture_output=True, text=True)
                self.assertEqual(result.returncode, 1, result.stderr)
                data = json.loads(report.read_text())
                self.assertEqual(data["materials"][0]["failure_stage"], "acquisition")
                self.assertEqual(data["materials"][3]["failure_stage"], "hash")
        finally:
            server.shutdown()
            server.server_close()
            thread.join()

    def test_delivered_candidates_have_stable_locators_and_no_gold(self):
        inventory = json.loads((HERE / "inventory.json").read_text())
        materials = {m["id"]: m for m in inventory["materials"]}
        excerpts = json.loads((HERE / "excerpts.json").read_text())
        self.assertGreaterEqual(len(excerpts["candidates"]), 20)
        self.assertEqual(len({c["id"] for c in excerpts["candidates"]}), len(excerpts["candidates"]))
        for fixture in excerpts["fixtures"]:
            self.assertEqual(hashlib.sha256((HERE / fixture["path"]).read_bytes()).hexdigest(),
                             fixture["sha256"])
            self.assertEqual(materials[fixture["material_id"]]["license"]["status"], "CC-BY-4.0")
        for candidate in excerpts["candidates"]:
            self.assertEqual(candidate["status"], "unreviewed")
            self.assertIsNone(candidate["gold"])
            self.assertIsNone(candidate["review_record"])
            self.assertEqual(candidate["original_sha256"], materials[candidate["material_id"]]["sha256"])
            quote = (HERE / candidate["fixture"]).read_text()[candidate["char_start"]:candidate["char_end"]]
            self.assertEqual(hashlib.sha256(quote.encode()).hexdigest(), candidate["text_sha256"])
        questions = json.loads((HERE / "questions.json").read_text())["candidates"]
        self.assertGreaterEqual(len(questions), 10)
        self.assertEqual(len({q["id"] for q in questions}), len(questions))
        for question in questions:
            self.assertEqual(question["status"], "unreviewed")
            for field in ("gold_answer", "relevance_judgments", "support_judgments", "review_record"):
                self.assertIsNone(question[field])
            self.assertTrue(set(question["materials"]) <= materials.keys())
            self.assertTrue(question["locators"])
        self.assertEqual(json.loads((HERE / "reviews.json").read_text())["records"], [])

    def test_clean_runs_preserve_content_identity_and_repeat_revision(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            content = b"# Research\n\nBodyonlyprobe describes an experiment.\n"
            (root / "paper.md").write_bytes(content)
            manifest = root / "inventory.json"
            manifest.write_text(json.dumps({"materials": [{
                "id": "paper", "filename": "paper.md", "title": "Research",
                "sha256": hashlib.sha256(content).hexdigest(),
                "acquisition_url": "http://127.0.0.1:1/unavailable",
                "queries": [{"query": "Research", "kind": "metadata"},
                            {"query": "Bodyonlyprobe", "kind": "body-gap"}],
            }]}))
            identities = []
            for run in range(2):
                report = root / f"report-{run}.json"
                result = subprocess.run([
                    sys.executable, str(HERE / "baseline.py"), "--binary", str(BINARY),
                    "--inventory", str(manifest), "--inputs", str(root),
                    "--report", str(report),
                ], capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                data = json.loads(report.read_text())
                item = data["materials"][0]
                self.assertEqual(item["status"], "passed")
                self.assertTrue(item["repeat_deduplicated"])
                self.assertEqual(item["sha256"], hashlib.sha256(content).hexdigest())
                self.assertEqual(item["searches"][0]["matched_source"], True)
                self.assertEqual(item["searches"][1]["matched_source"], False)
                self.assertEqual(item["repeat_revision_id"], item["revision_id"])
                identities.append(item["sha256"])
                self.assertNotIn("describes an experiment", report.read_text())
            self.assertEqual(*identities)


if __name__ == "__main__":
    unittest.main()
