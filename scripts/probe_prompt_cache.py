"""Report prompt-cache hit rates from the token usage log.

The app's SQLite file is copied to a temp path first so a running instance
never sees a reader lock.

Usage:
    python scripts/probe_prompt_cache.py                # last 40 API calls
    python scripts/probe_prompt_cache.py <session_id>   # one session
    python scripts/probe_prompt_cache.py --repeats      # requests whose prompt
                                                        # size repeats across
                                                        # generations; these
                                                        # should hit ~100%
"""
import os
import shutil
import sqlite3
import sys
from collections import defaultdict

SOURCE = os.path.join(
    os.environ["APPDATA"], "dev.lumen.app", "atelier", "atelier.db"
)


def connect():
    copy = os.path.join(os.environ["TEMP"], "atelier_cache_probe.db")
    shutil.copyfile(SOURCE, copy)
    conn = sqlite3.connect(copy)
    conn.row_factory = sqlite3.Row
    return conn


def fmt(row):
    p = row["prompt_tokens"] or 0
    c = row["cache_read_tokens"] or 0
    pct = 100.0 * c / p if p else 0.0
    return (
        f"{row['created_at']} corr={row['correlation_id']} "
        f"round={row['turn_index']} {row['model']:<22} "
        f"prompt={p:>7} cache={c:>7} ({pct:5.1f}%) miss={p - c:>7}"
    )


def recent(conn, session):
    where = "AND session_id = ?" if session else ""
    rows = conn.execute(
        f"""
        SELECT created_at, correlation_id, turn_index, model,
               prompt_tokens, cache_read_tokens
        FROM token_usage_events
        WHERE event_kind = 'api_call' AND prompt_tokens IS NOT NULL {where}
        ORDER BY created_at DESC LIMIT 40
        """,
        (session,) if session else (),
    ).fetchall()
    for r in rows:
        print(fmt(r))

    total_p = sum(r["prompt_tokens"] or 0 for r in rows)
    total_c = sum(r["cache_read_tokens"] or 0 for r in rows)
    if total_p:
        print(f"\naggregate hit rate: {100.0 * total_c / total_p:.1f}% "
              f"({total_c} / {total_p})")


def repeats(conn):
    """Same session + model + prompt size, seen in more than one generation.

    Identical token counts across separate generations mean the same content
    was sent. Anything well below 100% here is a prefix that got reordered
    rather than a genuinely cold cache.
    """
    rows = conn.execute(
        """
        SELECT created_at, session_id, correlation_id, turn_index, model,
               prompt_tokens, cache_read_tokens
        FROM token_usage_events
        WHERE event_kind = 'api_call' AND prompt_tokens IS NOT NULL
        ORDER BY created_at
        """
    ).fetchall()
    groups = defaultdict(list)
    for r in rows:
        groups[(r["session_id"], r["model"], r["prompt_tokens"])].append(r)

    for (sess, model, ptok), rs in sorted(groups.items(), key=lambda kv: -len(kv[1])):
        if len(rs) < 3 or len({r["correlation_id"] for r in rs}) < 2:
            continue
        print(f"\n=== session={sess} model={model} prompt_tokens={ptok} x{len(rs)}")
        for r in rs:
            print("  " + fmt(r))


if __name__ == "__main__":
    arg = sys.argv[1] if len(sys.argv) > 1 else None
    with connect() as conn:
        if arg == "--repeats":
            repeats(conn)
        else:
            recent(conn, arg)
