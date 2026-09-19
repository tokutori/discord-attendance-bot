"""Check resolved dependency direction, including transitive dependencies."""
import json
import subprocess

metadata = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--locked", "--format-version", "1"], encoding="utf-8"
))
packages = {p["id"]: p["name"] for p in metadata["packages"]}
nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}


def dependencies(name):
    pending = [key for key, value in packages.items() if value == name]
    seen = set()
    while pending:
        key = pending.pop()
        if key in seen:
            continue
        seen.add(key)
        pending.extend(nodes[key]["dependencies"])
    return {packages[key] for key in seen}


assert "discord-attendance-bot" not in dependencies("attendance-view")
assert "discord-attendance-bot" not in dependencies("attendance-query")
assert not {"attendance-view", "printpdf"} & dependencies("discord-attendance-bot")
assert not {"attendance-query", "attendance-view", "discord-attendance-bot", "poise", "printpdf"} & dependencies("attendance-shared")
print("Dependency boundaries passed: view cannot import writer; core has no PDF/view dependency.")
