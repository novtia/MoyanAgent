# -*- coding: utf-8 -*-
"""Batch sentence scoring with jevnovel's infer.py.

Usage: python jev_score.py <jevnovel_dir> <device> <input.jsonl> <output.jsonl>

Input lines:  {"id": 0, "sentence": "..."}
Output lines: {"id": 0, "p_bad": 0.0412}   or   {"id": 0, "error": "..."}

The model is loaded once for the whole batch. A sentence that cannot be scored
(too long, nothing scorable) gets an `error` line and the batch goes on.
"""
import contextlib
import json
import sys


def main():
    model_dir, device, src, dst = sys.argv[1:5]
    sys.path.insert(0, model_dir)
    # infer.py prints progress to stdout while loading; keep stdout clean.
    with contextlib.redirect_stdout(sys.stderr):
        import infer

        predictor = infer.Predictor(model_dir, device)

    with open(src, encoding="utf-8") as fin, open(dst, "w", encoding="utf-8") as fout:
        for line in fin:
            line = line.strip()
            if not line:
                continue
            row = json.loads(line)
            try:
                result = predictor.scan(row["sentence"])
                out = {"id": row["id"], "p_bad": result["p_bad"]}
            except Exception as err:  # noqa: BLE001 - one bad sentence must not stop the batch
                out = {"id": row["id"], "error": str(err)}
            fout.write(json.dumps(out, ensure_ascii=False) + "\n")
            fout.flush()


if __name__ == "__main__":
    main()
