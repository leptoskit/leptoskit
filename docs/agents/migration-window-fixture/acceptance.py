"""Real separately compiled old/new executables against an owned PostgreSQL cluster."""
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import selectors
import socket
import subprocess
import sys

source = Path(__file__).resolve().parent / "fixtures"
evidence = Path(sys.argv[1]).resolve()
evidence.mkdir(parents=True, exist_ok=True)
pg = Path(os.environ["LEPTOSKIT_POSTGRES_BIN"])
old, new = evidence / "old-backend", evidence / "new-backend"
checks = []
processes = []


def run(*args, **kwargs):
    result = subprocess.run(args, capture_output=True, text=True, timeout=30, **kwargs)

    if result.returncode:
        raise RuntimeError((args, result.stdout, result.stderr))

    return result.stdout


def line(process):
    with selectors.DefaultSelector() as selector:
        selector.register(process.stdout, selectors.EVENT_READ)
        assert selector.select(15), "executable response timed out"
    answer = process.stdout.readline().strip()
    assert answer, "executable exited before response"

    return answer


def start(binary, bootstrap=False):
    err = open(evidence / (binary.name + "-process-" + str(len(processes)) + ".stderr"), "w")
    process = subprocess.Popen([str(binary)] + (["--bootstrap"] if bootstrap else []), env=environment,
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=err, text=True)
    processes.append((process, err))
    assert line(process).startswith("ready:")

    return process


def command(process, sql):
    process.stdin.write(sql + "\n")
    process.stdin.flush()

    return line(process)


def stop(process):
    process.stdin.write("quit\n")
    process.stdin.flush()
    assert process.wait(timeout=10) == 0


with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    port = sock.getsockname()[1]
data = evidence / "postgres-data"
run(str(pg / "initdb"), "-D", str(data), "-A", "trust", "--no-locale", "-E", "UTF8")
run(str(pg / "pg_ctl"), "-D", str(data), "-l", str(evidence / "postgres.log"), "-o",
    f"-h 127.0.0.1 -p {port} -k /tmp", "-w", "start")
environment = dict(os.environ, DATABASE_URL=f"postgres://127.0.0.1:{port}/a05?sslmode=disable",
                   LEPTOSKIT_MIGRATION_WINDOW=str(source / "window.json"),
                   LEPTOSKIT_MIGRATION_WINDOW_SHA256=(source / "window.sha256").read_text())
try:

    run(str(pg / "createdb"), "-h", "127.0.0.1", "-p", str(port), "a05")

    for binary in [old, new]:
        target = evidence / (binary.name + "-contract.json")
        bad_env = dict(environment, LEPTOSKIT_DB_CONTRACT_OUT=str(target), DATABASE_URL="invalid", LEPTOS_SITE_ADDR="invalid")
        run(str(binary), env=bad_env)
        before = target.read_bytes()
        assert subprocess.run([str(binary)], env=bad_env, capture_output=True, timeout=10).returncode != 0
        assert target.read_bytes() == before
    checks.append("both executable contracts export before invalid startup and refuse replacement")
    initial = start(old, True)
    assert command(initial, "INSERT INTO notes (id, body) VALUES (1, 'baseline')") == "1"
    stop(initial)
    previous = start(old)
    expanded = start(new)
    assert command(expanded, "UPDATE notes SET extra = 'new value' WHERE id = 1") == "1"
    assert command(previous, "UPDATE notes SET body = 'old writer' WHERE id = 1") == "1"
    def writes(pair):
        process, base = pair

        for index in range(10):
            assert command(process, f"INSERT INTO notes (id, body) VALUES ({base + index}, 'concurrent')") == "1"
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        list(pool.map(writes, [(previous, 10), (expanded, 30)]))
    checks.append("old and new executable processes coexist and perform concurrent named-column writes")
    stop(previous)
    restarted = start(old)

    observed = command(restarted, "query SELECT body, extra FROM notes WHERE id = 1")
    assert "old writer" in observed and "new value" in observed
    stop(expanded)
    assert command(restarted, "INSERT INTO notes (id, body) VALUES (100, 'old rollback')") == "1"
    stop(restarted)
    checks.append("fresh old executable restart and application rollback preserve expanded schema and rows")

    for patch in [{"LEPTOSKIT_MIGRATION_WINDOW_SHA256": "0" * 64}, {"LEPTOSKIT_MIGRATION_WINDOW": str(source / "missing.json")}]:
        assert subprocess.run([str(old)], env=dict(environment, **patch), capture_output=True, timeout=10).returncode != 0
    checks.append("substituted pin and missing window fail before ready")
    dump = run(str(pg / "pg_dump"), "-h", "127.0.0.1", "-p", str(port), "a05")
    (evidence / "database.sql").write_text(dump)

    report = {"checks": checks, "executables": {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in [old, new]},
              "limits": ["local processes, not managed container admission or runtime attestation"]}
    (evidence / "acceptance.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
finally:
    for process, err in processes:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=10)
        err.close()
    run(str(pg / "pg_ctl"), "-D", str(data), "-m", "immediate", "-w", "stop")
