"""Regenerate attributed CC BY text fixtures and unreviewed excerpt locators."""

import argparse
import hashlib
from html.parser import HTMLParser
import json
from pathlib import Path
import xml.etree.ElementTree as ET


HERE = Path(__file__).resolve().parent
SELECTIONS = {
    "state-v1": [
        ("p-4", "model overview"), ("p-6", "perturbation generalization"),
        ("p-7", "existing model comparison"), ("p-9", "destructive measurement and heterogeneity"),
        ("p-10", "technical noise"), ("p-17", "architecture"),
        ("p-18", "training data scales"), ("p-19", "performance claims"),
        ("p-20", "optimal transport connection"), ("p-21", "state transition and embedding"),
    ],
    "perturb-seq-2022": [
        ("P6", "genotype phenotype mapping"), ("P7", "forward and reverse genetics"),
        ("P8", "screen limitations"), ("P9", "single-cell CRISPR readout"),
        ("P10", "genome-scale motivation"), ("P11", "study scope"),
        ("P12", "Perturb-seq mechanism"), ("P13", "CRISPRi selection"),
        ("P14", "guide library design"), ("P15", "screen design and cell contexts"),
    ],
}


class Paragraphs(HTMLParser):
    def __init__(self):
        super().__init__()
        self.active = None
        self.parts = []
        self.paragraphs = {}

    def handle_starttag(self, tag, attrs):
        if tag == "p":
            self.active = dict(attrs).get("id")
            self.parts = []

    def handle_data(self, text):
        if self.active is not None:
            self.parts.append(text)

    def handle_endtag(self, tag):
        if tag == "p" and self.active is not None:
            if self.active in self.paragraphs:
                raise ValueError(f"Duplicate paragraph ID: {self.active}")
            self.paragraphs[self.active] = " ".join("".join(self.parts).split())
            self.active = None


def digest(data):
    return hashlib.sha256(data).hexdigest()


def prepare(inputs, output):
    inventory = json.loads((HERE / "inventory.json").read_text())
    candidates = []
    fixtures = []
    fixture_materials = []
    for material in inventory["materials"]:
        if material["id"] not in SELECTIONS:
            continue
        raw = (inputs / material["filename"]).read_bytes()
        if digest(raw) != material["sha256"]:
            raise ValueError(f"Hash mismatch: {material['id']}")
        if material["license"]["status"] != "CC-BY-4.0":
            raise ValueError(f"Excerpt redistribution not approved: {material['id']}")
        if material["id"] == "state-v1":
            parser = Paragraphs()
            parser.feed(raw.decode("utf-8"))
            paragraphs = parser.paragraphs
        else:
            paragraphs = {p.attrib["id"]: " ".join("".join(p.itertext()).split())
                          for p in ET.fromstring(raw).findall(".//body//p") if "id" in p.attrib}
        text = (f"# {material['subject']}: unreviewed paper excerpts\n\n"
                f"{material['authors_or_publisher']}\n\n"
                f"Original: {material['acquisition_url']}\n\n"
                "License: https://creativecommons.org/licenses/by/4.0/\n\n"
                "Changes: selected paragraphs; markup removed and whitespace collapsed. "
                "Inline reference labels retained. No scientific annotations approved.\n\n")
        filename = f"fixtures/{material['id']}.md"
        for number, (paragraph, topic) in enumerate(SELECTIONS[material["id"]], 1):
            quote = paragraphs[paragraph]
            if len(quote) < 100:
                raise ValueError(f"Missing or implausibly short paragraph: {paragraph}")
            candidate_id = f"ex-{material['id']}-{number:02}"
            text += f"## {candidate_id} ({paragraph})\n\n"
            start = len(text)
            text += quote + "\n\n"
            candidates.append({
                "id": candidate_id, "status": "unreviewed", "material_id": material["id"],
                "original_sha256": material["sha256"], "topic": topic,
                "original_locator": {"format": "HTML id" if material["id"] == "state-v1" else "JATS p/@id",
                                     "value": paragraph},
                "fixture": filename, "char_start": start, "char_end": start + len(quote),
                "text_sha256": digest(quote.encode("utf-8")),
                "normalization": "DOM itertext, entity decoding, whitespace collapsed with single spaces",
                "gold": None, "review_record": None,
            })
        text = text.rstrip() + "\n"
        path = output / filename
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
        fixtures.append({"path": filename, "sha256": digest(text.encode("utf-8")),
                         "material_id": material["id"], "original_sha256": material["sha256"],
                         "license": "CC-BY-4.0", "kind": "derived text excerpts, not original bytes"})
        fixture_materials.append({
            "id": f"{material['id']}-excerpts", "parent_material_id": material["id"],
            "original_sha256": material["sha256"], "filename": Path(filename).name,
            "sha256": digest(text.encode("utf-8")), "title": material["title"],
            "acquisition_url": material["acquisition_url"], "tags": material["tags"],
            "queries": [*material["queries"][:2],
                        {"query": "Therapeutic discovery" if material["id"] == "state-v1" else "CRISPRi",
                         "kind": "body-gap"}],
        })
    output.mkdir(parents=True, exist_ok=True)
    (output / "excerpts.json").write_text(json.dumps({"schema_version": 1, "fixtures": fixtures,
                                                     "candidates": candidates}, indent=2) + "\n")
    (output / "fixture-inventory.json").write_text(json.dumps({
        "schema_version": 1, "kind": "Generated offline subset; not the six-original baseline",
        "materials": fixture_materials,
    }, ensure_ascii=False, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inputs", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=HERE)
    options = parser.parse_args()
    prepare(options.inputs, options.output)
