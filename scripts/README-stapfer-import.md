# Reproduce the supplied Stapfer workbook import

Run from the backend directory using Python 3 with `openpyxl==3.1.5`. The importer reads the two supplied files `export_antworten.xlsx` and `export_antworten(1).xlsx`; it does not download files or execute workbook formulas, macros, external links, PDF content, or the SQL archive.

```sh
python3 scripts/import_stapfer.py --input-dir /path/to/supplied/Stapfer/data
python3 scripts/test_import_stapfer.py --receipt evidence/stapfer-rejected-fixture-validation.json
python3 scripts/check_stapfer_collection.py --base http://127.0.0.1:8080 --origin http://localhost:3000 --output /tmp/stapfer-http-check.json
```

The original 105 full-text records in `data/stapfer-sources.jsonl` are checked against workbook1 and remain unchanged. The second workbook produces `data/stapfer-answer-sources.jsonl`: 2,245 transcription groups from 14,801 rows, including 14,743 accepted answer rows and 58 explicitly missing answers. The 136 numeric cells retain their typed value and use canonical JSON scalar rendering for searchable text. Original string answers retain their characters and line endings.

`data/stapfer-import-manifest.json` records both file hashes, the first workbook's row hashes, collection overlap and output hashes. `data/stapfer-rejected-rows.jsonl` records each empty answer's file hash, row hash, locator, question and reason. Each grouped source contains its accepted answer spans, expected row count, missing question rows and raw-row manifest. The raw-row hash covers the canonical JSON array of the twelve cell values; the source record hash covers its ordered row-number/hash manifest; `text_sha256` covers the exact assembled UTF-8 text. These hashes have different scopes.

The backend resumes an interrupted import by matching existing immutable IDs and hashes. It writes transactions of at most 200 records and 3 MB per request, then publishes the collection marker. A changed existing record stops the import. An already complete collection takes the existing marker path. Initial import timing depends on the deployed database and must be measured there; the local recovery receipt is not an Azure timing guarantee.

Authenticated `/api/datasets` includes both collections. `/api/datasets/stapfer-1799-answers/export?format=json`, `jsonl`, and `tsv` export the actual stored records. JSON and JSONL preserve complete provenance; TSV is a spreadsheet-safe flat view whose text cell normalizes tabs and line breaks. The dataset metadata contains the import manifest and rejected rows. The original workbooks and schema PDF remain external inputs and are not copied into the repository.

The Bern model mapping is in `evidence/stapfer-schema-mapping.json`. Its important unresolved distinction is exported `Kanton 1750` versus the model's `ort_kanton_1780`; these are retained as different concepts. Page positions, numeric question IDs, authorship, historical completeness and handwriting quality are not inferred from these exports. The two collections overlap and must not be counted as independent observations.
