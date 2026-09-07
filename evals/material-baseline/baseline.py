"""Reproduce the metadata baseline without models or a persistent workspace."""

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import urllib.request


HERE = Path(__file__).resolve().parent


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def run(args):
    inventory_bytes = args.inventory.read_bytes()
    inventory = json.loads(inventory_bytes)
    report = {
        "schema_version": 1,
        "started_at": datetime.now(timezone.utc).isoformat(),
        "inventory_sha256": sha256(inventory_bytes),
        "binary_sha256": sha256(args.binary.read_bytes()),
        "scope": "Source metadata lexical baseline; no scientific quality judgment",
        "commands": [], "materials": [], "status": "passed",
    }
    # A minimal environment prevents inherited model keys or user configuration.
    with tempfile.TemporaryDirectory(prefix="knowmesh-p01-") as folder:
        root = Path(folder)
        workspace = root / "workspace"
        environment = {"PATH": os.environ.get("PATH", ""), "HOME": str(root),
                       "LANG": "C.UTF-8"}

        def command(arguments, raw=False):
            argv = [str(args.binary), "--workspace", str(workspace), *arguments]
            result = subprocess.run(argv, capture_output=True, env=environment, timeout=60)
            recorded = [part.replace(str(workspace), "$WORKSPACE")
                        .replace(str(args.inputs), "$INPUTS") for part in argv]
            recorded[0] = "$BINARY"
            entry = {"argv": recorded, "exit_code": result.returncode}
            report["commands"].append(entry)
            if result.returncode:
                entry["stderr"] = result.stderr.decode("utf-8", errors="replace")
                raise RuntimeError(f"CLI failed: {arguments[0]}: {entry['stderr']}")
            if raw:
                entry.update({"stdout_sha256": sha256(result.stdout),
                              "stdout_bytes": len(result.stdout)})
                return result.stdout
            response = json.loads(result.stdout)
            entry["response"] = response
            if response.get("ok") is not True:
                raise RuntimeError(f"CLI returned unsuccessful envelope: {arguments[0]}")
            return response["data"]

        try:
            report["version"] = command(["version"])
            command(["init", str(workspace)])
            for material in inventory["materials"]:
                item = {"id": material["id"], "expected_sha256": material["sha256"],
                        "acquisition_url": material["acquisition_url"], "status": "failed"}
                if "parent_material_id" in material:
                    item.update({"parent_material_id": material["parent_material_id"],
                                 "original_sha256": material["original_sha256"]})
                report["materials"].append(item)
                stage = "acquisition"
                try:
                    path = args.inputs / material["filename"]
                    if not path.is_file() and args.acquire:
                        request = urllib.request.Request(material["acquisition_url"],
                                                         headers={"User-Agent": "KnowMesh-P01/1"})
                        with urllib.request.urlopen(request, timeout=30) as response:
                            content = response.read(100 * 1024 * 1024 + 1)
                            item["resolved_url"] = response.url
                        if len(content) > 100 * 1024 * 1024:
                            raise ValueError("Acquisition exceeds 100 MiB")
                        item["sha256"] = sha256(content)
                        stage = "hash"
                        if item["sha256"] != material["sha256"]:
                            raise ValueError("Downloaded content hash changed; inventory requires review")
                        path.parent.mkdir(parents=True, exist_ok=True)
                        path.write_bytes(content)
                        item["acquisition"] = "downloaded"
                    else:
                        content = path.read_bytes()
                        item["acquisition"] = "local"
                    stage = "hash"
                    item["sha256"] = sha256(content)
                    item["bytes"] = len(content)
                    if item["sha256"] != material["sha256"]:
                        raise ValueError("Content hash changed; inventory requires review")
                    stage = "import"
                    add_args = ["source", "add", str(path), "--title", material["title"]]
                    for tag in material.get("tags", []):
                        add_args.extend(["--tag", tag])
                    added = command(add_args)
                    source_id = added["source"]["id"]
                    revision_id = added["revision"]["id"]
                    item.update({"source_id": source_id, "revision_id": revision_id})
                    if added["revision"]["sha256"] != item["sha256"]:
                        raise ValueError("Imported revision hash differs from acquired bytes")
                    stage = "read"
                    command(["sync"])
                    source = command(["source", "get", source_id])["source"]
                    if revision_id not in [r["id"] for r in source["revisions"]]:
                        raise ValueError("Source get did not return imported revision")
                    if command(["source", "content", revision_id, "--raw"], raw=True) != content:
                        raise ValueError("Source content differs from acquired bytes")
                    stage = "repeat-import"
                    repeat = command([*add_args, "--source-id", source_id])
                    item["repeat_deduplicated"] = repeat["deduplicated"]
                    item["repeat_revision_id"] = repeat["revision"]["id"]
                    if not repeat["deduplicated"] or repeat["revision"]["id"] != revision_id:
                        raise ValueError("Repeat import did not preserve revision identity")
                    stage = "search"
                    item["searches"] = []
                    for query in material["queries"]:
                        present = query["query"].casefold() in content.decode("utf-8").casefold()
                        if query["kind"] == "body-gap" and not present:
                            raise ValueError(f"Body probe is absent from acquired UTF-8 bytes: {query['query']}")
                        data = command(["search", query["query"], "--record-type", "source",
                                        "--limit", "100", "--explain"])
                        hits = data["groups"]["sources"]
                        matched = any(hit["record_id"] == source_id for hit in hits)
                        item["searches"].append({**query, "matched_source": matched,
                                                 "query_in_raw_utf8": present,
                                                 "source_ids": [h["record_id"] for h in hits]})
                        if query["kind"] == "metadata" and not matched:
                            raise ValueError(f"Metadata query missed imported Source: {query['query']}")
                    item["status"] = "passed"
                except (OSError, ValueError, RuntimeError, KeyError, subprocess.TimeoutExpired) as error:
                    item.update({"failure_stage": stage, "error": str(error)})
                    report["status"] = "failed"
        except (OSError, ValueError, RuntimeError, KeyError, subprocess.TimeoutExpired) as error:
            report.update({"status": "failed", "error": str(error)})
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--inventory", type=Path, default=HERE / "inventory.json")
    parser.add_argument("--inputs", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--acquire", action="store_true", help="Download missing originals; enforce pinned hashes")
    options = parser.parse_args()
    for field in ("binary", "inventory", "inputs", "report"):
        setattr(options, field, getattr(options, field).resolve())
    raise SystemExit(run(options))
