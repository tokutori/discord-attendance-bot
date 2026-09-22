"""Validate role-specific Compose environments using dummy configuration only.

No Docker daemon, bot, host .env, host credentials, or network is used.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
SHARED = {
    "DISCORD_GUILD_ID", "DATABASE_URL", "ATTENDANCE_TIMEZONE",
    "ATTENDANCE_AUTO_END_TIME", "RUST_LOG", "DISCORD_TEST_GUILD_ID",
    "DISCORD_RELEASE_GUILD_ID", "DATABASE_URL_TEST", "DATABASE_URL_RELEASE",
}
CORE = {"DISCORD_TOKEN", "ATTENDANCE_SQLITE_SYNCHRONOUS"}
VIEW = {
    "DISCORD_VIEW_TOKEN", "DISCORD_CORE_APPLICATION_ID", "ATTENDANCE_STATUS_MODE",
    "ATTENDANCE_STATUS_CHANNEL_ID", "ATTENDANCE_STATUS_REFRESH_SECONDS",
    "ATTENDANCE_PDF_FONT_PATH", "ATTENDANCE_STATUS_CHANNEL_ID_TEST",
    "ATTENDANCE_STATUS_CHANNEL_ID_RELEASE",
}


def main():
    docker = shutil.which("docker")
    standalone = shutil.which("docker-compose")
    if not docker and not standalone:
        raise RuntimeError("Docker CLI with Compose is required (daemon is not needed)")
    # Docker Desktop exposes the standalone CLI; prefer it so plugin discovery
    # never depends on a user's Docker config.json (which may contain credentials).
    command = [standalone] if standalone else [docker, "compose"]
    with tempfile.TemporaryDirectory(prefix="attendance-compose-") as directory:
        temporary = Path(directory)
        compose = temporary / "compose.yaml"
        compose.write_text((ROOT / "compose.yaml").read_text(encoding="utf-8"), encoding="utf-8")
        dummy = temporary / "dummy.env"
        # Deliberately do not inherit the calling shell's configuration variables.
        environment = {
            "PATH": os.defpath,
            "HOME": directory,
            "USERPROFILE": directory,
            "DOCKER_CONFIG": str(temporary / "docker-config"),
        }
        if os.name == "nt":
            environment["SystemRoot"] = os.environ.get("SystemRoot", r"C:\Windows")
        samples = [
            # Read only the tracked example, never any actual configuration file.
            (ROOT / ".env.example").read_text(encoding="utf-8") + "\nUNRELATED_VALUE=dummy\n",
            # Core-only installs must not require view credentials to render Compose.
            "DISCORD_TOKEN=dummy-core\nDISCORD_GUILD_ID=1\n",
            "DISCORD_TOKEN=dummy-core\nDISCORD_VIEW_TOKEN=dummy-view\n"
            "DISCORD_CORE_APPLICATION_ID=2\nDISCORD_GUILD_ID=3\n"
            "ATTENDANCE_TIMEZONE=UTC\nATTENDANCE_AUTO_END_TIME=disabled\n"
            "ATTENDANCE_STATUS_MODE=names\nATTENDANCE_STATUS_CHANNEL_ID=4\n"
            "ATTENDANCE_STATUS_REFRESH_SECONDS=120\nUNRELATED_VALUE=dummy\n",
        ]
        for sample in samples:
            dummy.write_text(sample, encoding="utf-8")
            result = subprocess.run(
                command + ["--project-directory", directory,
                 "--env-file", str(dummy), "-f", str(compose), "--profile", "view",
                 "--profile", "maintenance", "config", "--format", "json"],
                cwd=directory, env=environment, text=True, encoding="utf-8",
                capture_output=True, timeout=30,
            )
            if result.returncode:
                raise RuntimeError("Dummy Compose validation failed: " + result.stderr)
            services = json.loads(result.stdout)["services"]
            core = services["bot"]["environment"]
            view = services["view"]["environment"]
            assert set(core) == SHARED | CORE, "unexpected core environment keys"
            assert set(view) == SHARED | VIEW, "unexpected view environment keys"
            assert not services["maintenance"].get("environment"), "maintenance inherited credentials"
            for name in SHARED:
                assert core[name] == view[name], f"shared policy diverged: {name}"
            assert core["DATABASE_URL"] == "sqlite:///data/attendance.db"
            assert core["ATTENDANCE_SQLITE_SYNCHRONOUS"] == "full"
            custom = "ATTENDANCE_TIMEZONE=UTC" in sample
            assert core["ATTENDANCE_TIMEZONE"] == ("UTC" if custom else "Asia/Tokyo")
            assert core["ATTENDANCE_AUTO_END_TIME"] == ("disabled" if custom else "21:00")
            if custom:
                assert core["DISCORD_TOKEN"] == "dummy-core"
                assert view["DISCORD_VIEW_TOKEN"] == "dummy-view"
                assert view["DISCORD_CORE_APPLICATION_ID"] == "2"
                assert view["DISCORD_GUILD_ID"] == "3"
                assert view["ATTENDANCE_STATUS_MODE"] == "names"
                assert view["ATTENDANCE_STATUS_CHANNEL_ID"] == "4"
                assert view["ATTENDANCE_STATUS_REFRESH_SECONDS"] == "120"
            volumes = services["view"]["volumes"]
            assert len(volumes) == 1 and volumes[0]["target"] == "/data"
            assert volumes[0]["type"] == "volume" and volumes[0]["read_only"]
            assert services["view"]["read_only"]
            # No host configuration file is mounted into any service.
            assert all(volume["type"] == "volume" for service in services.values()
                       for volume in service.get("volumes", []))
        print("Compose configuration passed: 3 dummy cases; role allowlists and readonly volume verified.")


if __name__ == "__main__":
    main()
