#!/usr/bin/env python3
"""Verify the published collection via authenticated HTTP, without model calls."""
import argparse
import hashlib
import http.cookiejar
import json
import pathlib
import time
import urllib.error
import urllib.request

p = argparse.ArgumentParser()
p.add_argument("--base", required=True)
p.add_argument("--origin", required=True)
p.add_argument("--output", type=pathlib.Path, required=True)
a = p.parse_args()
checks = []
opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))


def call(path, data=None, authenticated=True):
    request = urllib.request.Request(a.base + path, data=None if data is None else json.dumps(data).encode(),
                                     headers={"Origin": a.origin, "Content-Type": "application/json"})
    try:
        with (opener if authenticated else urllib.request.build_opener()).open(request, timeout=60) as response:
            return response.status, response.read()
    except urllib.error.HTTPError as error:
        return error.code, error.read()


def check(name, passed):
    checks.append({"check": name, "passed": bool(passed)})
    if not passed:
        raise AssertionError(name)


started = time.monotonic()
check("anonymous_export_denied", call("/api/datasets/stapfer-1799-answers/export?format=json", authenticated=False)[0] == 401)
check("demo_auth", call("/api/auth/demo", {})[0] == 200)
status, content = call("/api/datasets")
check("datasets_readable", status == 200)
datasets = {r["id"]: r for r in json.loads(content)["datasets"]}
check("both_collections_visible", {"stapfer-1799", "stapfer-1799-answers"} <= datasets.keys())
dataset = datasets["stapfer-1799-answers"]
check("declared_counts_exact", dataset["record_count"] == 2245 and dataset["accepted_answer_rows"] == 14743 and dataset["missing_answer_rows"] == 58)
check("rejection_registry_retained", len(dataset["rejected_rows"]) == 58 and all(r["reason"] == "empty_answer" for r in dataset["rejected_rows"]))
status, content = call("/api/datasets/stapfer-1799-answers/export?format=json")
check("json_export", status == 200)
rows = json.loads(content)
check("export_count_matches_manifest", len(rows) == dataset["record_count"])
check("unique_source_ids", len({r["id"] for r in rows}) == len(rows))
check("all_exact_text_hashes", all(hashlib.sha256(r["text"].encode()).hexdigest() == r["text_sha256"] for r in rows))
check("all_original_workbook_hashes", all(r["source_sha256"] == dataset["source_sha256"] for r in rows))
check("missing_rows_not_silently_dropped", sum(r["provenance"]["missing_row_count"] for r in rows) == 58)
check("complete_row_accounting", sum(r["provenance"]["expected_row_count"] for r in rows) == 14801)
accepted = [item for row in rows for item in row["provenance"]["accepted_rows"]]
check("numeric_cell_provenance", sum(item["answer_cell_type"] != "str" for item in accepted) == 136)
check("answer_span_hashes", all(hashlib.sha256(row["text"][item["answer_start_char"]:item["answer_end_char"]].encode()).hexdigest() == item["answer_sha256"] for row in rows for item in row["provenance"]["accepted_rows"]))
for format in ("jsonl", "tsv"):
    status, content = call(f"/api/datasets/stapfer-1799-answers/export?format={format}")
    check(format + "_export_count", status == 200 and len(content.splitlines()) == 2245 + (format == "tsv"))
legacy = [json.loads(line) for line in (pathlib.Path(__file__).resolve().parents[1] / "data/stapfer-sources.jsonl").read_text().splitlines()]
status, content = call("/api/datasets/stapfer-1799/export?format=json")
exported = {r["id"]: r for r in json.loads(content)}
check("legacy_105_records_unchanged", status == 200 and len(exported) == 105 and all(all(exported[r["id"]][key] == value for key, value in r.items()) for r in legacy))
check("restored_sources_have_one_revision", all(r["version"] == 1 for r in rows))
receipt = {"validation_class": "actual local HTTP collection export and provenance checks", "passed": sum(c["passed"] for c in checks),
           "failed": sum(not c["passed"] for c in checks), "checks": checks, "elapsed_seconds": round(time.monotonic() - started, 3),
           "records": len(rows), "accepted_answer_rows": len(accepted), "missing_answer_rows": 58, "provider_calls": 0,
           "historical_accuracy": "NOT_RUN", "original_workbook_files_committed": False}
a.output.parent.mkdir(parents=True, exist_ok=True)
a.output.write_text(json.dumps(receipt, indent=2) + "\n")
print(json.dumps({k: receipt[k] for k in ("passed", "failed", "records", "elapsed_seconds")}))
