"""Read logical SQLite state without creating or modifying the fixture database."""
import json
from pathlib import Path
import sqlite3
import sys


def snapshot(path):
    # mode=ro deliberately refuses a missing database; a broken journey must
    # never pass by comparing two newly created empty databases.
    with sqlite3.connect(Path(path).resolve().as_uri() + "?mode=ro", uri=True) as database:
        schema = database.execute(
            "SELECT type, name, tbl_name, sql FROM sqlite_master "
            "WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name"
        ).fetchall()
        rows = {}
        for kind, name, _, _ in schema:
            if kind != "table":
                continue
            quoted = '"' + name.replace('"', '""') + '"'
            values = [
                [{"blob": value.hex()} if isinstance(value, bytes) else value for value in row]
                for row in database.execute("SELECT * FROM " + quoted)
            ]
            rows[name] = sorted(values, key=lambda row: json.dumps(row, sort_keys=True))
        return {"schema": schema, "rows": rows}


if __name__ == "__main__":
    print(json.dumps(snapshot(sys.argv[1]), sort_keys=True))
