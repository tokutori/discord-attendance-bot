"""Offline SQLx/SQLite regression on real readonly mounts; disposable Docker resources only.

Requires wal-probe:ci and pdf-probe:ci targets. Never reads .env or starts a bot.
"""
import subprocess
import re
import time
import uuid

prefix = "attendance-probe-" + uuid.uuid4().hex[:12]
containers = []
volumes = []


def docker(*args, check=True):
    result = subprocess.run(["docker", *args], text=True, encoding="utf-8",
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=150)
    if check and result.returncode:
        raise RuntimeError(f"docker {args}: {result.stdout}")
    return result


def wait_until(predicate):
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.2)
    raise RuntimeError("runtime probe readiness timed out")


def stage(writer, number):
    wait_until(lambda: docker("exec", writer, "test", "-f", f"/data/ready-{number}",
                              check=False).returncode == 0)


def advance(writer, number):
    docker("exec", writer, "wal-runtime-probe", "signal", "/data", f"advance-{number}")


def reader(volume, count, hold=False):
    name = f"{prefix}-reader-{len(containers)}"
    containers.append(name)
    args = ["run", "--name", name, "--network", "none", "--read-only",
            "--cap-drop=ALL", "--security-opt=no-new-privileges:true",
            "--mount", f"type=volume,src={volume},dst=/data,readonly"]
    if hold:
        args.append("-d")
    return name, docker(*args, "wal-probe:ci", "reader-hold" if hold else "reader",
                        "/data", str(count), check=False)


try:
    for anchored in [False, True]:
        volume = f"{prefix}-data-{int(anchored)}"
        writer = f"{prefix}-writer-{int(anchored)}"
        volumes.append(volume)
        containers.append(writer)
        docker("volume", "create", volume)
        docker("run", "-d", "--name", writer, "--network", "none",
               "--mount", f"type=volume,src={volume},dst=/data",
               "wal-probe:ci", "writer" if anchored else "unanchored", "/data")
        pid = docker("inspect", "--format", "{{.State.Pid}}", writer).stdout.strip()
        stage(writer, 0)
        _, result = reader(volume, 1)
        if not anchored:
            # SQLite 3.51.3/SQLx on a readonly Docker mount reports CANTOPEN (14)
            # at the first read; other SQLite builds report READONLY_DIRECTORY.
            # The writer already proved sidecars disappeared after pool size=0.
            codes = {int(code) for code in re.findall(r"\(code: (\d+)\)", result.stdout)}
            assert result.returncode != 0 and codes & {8, 14, 1544}, result.stdout
            print(result.stdout.strip(), flush=True)
            print("Negative control: pool reaped to zero without anchor; readonly startup rejected.", flush=True)
            advance(writer, 0)
        else:
            assert result.returncode == 0, result.stdout
            print(result.stdout.strip(), flush=True)
            held_reader, result = reader(volume, 1, hold=True)
            assert result.returncode == 0, result.stdout
            wait_until(lambda: "READER_READY" in docker("logs", held_reader).stdout)
            advance(writer, 0)
            stage(writer, 1)  # writer commits and pool reaps while view stays alive
            docker("kill", held_reader)
            advance(writer, 1)  # recording continues with the view killed
            stage(writer, 2)
            _, result = reader(volume, 3)  # replacement view, no writer access since pool size=0
            assert result.returncode == 0, result.stdout
            assert docker("inspect", "--format", "{{.State.Pid}}", writer).stdout.strip() == pid
            print(result.stdout.strip(), flush=True)
            print("Anchor regression passed: view stop/kill/recreate, 3 records, unchanged writer PID.", flush=True)
            advance(writer, 2)
        assert docker("wait", writer).stdout.strip() == "0", docker("logs", writer).stdout
        print(docker("logs", writer).stdout.strip(), flush=True)

    pdf = docker("run", "--rm", "--network", "none", "--read-only",
                 "--memory=512m", "--cpus=1.0", "--pids-limit=128",
                 "--cap-drop=ALL", "--security-opt=no-new-privileges:true",
                 "--tmpfs", "/tmp:size=64m", "pdf-probe:ci")
    print(pdf.stdout.strip(), flush=True)
except Exception:
    for name in containers:
        print(docker("logs", name, check=False).stdout, flush=True)
    raise
finally:
    # Only resources created by this invocation are ever removed.
    for name in containers:
        docker("rm", "-f", name, check=False)
    for volume in volumes:
        docker("volume", "rm", volume, check=False)
