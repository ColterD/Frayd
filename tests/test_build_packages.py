"""Exercise Docker package-step deadlines without host APT or network access."""
import os
from pathlib import Path
import re
import shutil
import shlex
import signal
import subprocess
import tempfile
import unittest


DOCKERFILE = Path(__file__).resolve().parents[1] / "Dockerfile"

# Stands in for apt-get: records the phase, then stalls in the requested one with a child process.
FAKE_APT = """#!/usr/bin/env python3
import os
from pathlib import Path
import subprocess
import sys
import time
root = Path(os.environ['FRAYD_APT_FIXTURE'])
phase = 'update' if 'update' in sys.argv else 'install'
with (root / 'calls').open('a') as output:
    output.write(' '.join([phase, *sys.argv[1:]]) + '\\n')
if phase == os.environ['FRAYD_APT_STALL']:
    child = subprocess.Popen(['sleep', '60'])
    (root / 'pids').write_text(str(os.getpid()) + ' ' + str(child.pid))
    time.sleep(60)
"""


def package_commands():
    """Return the APT_TRANSPORT default and the apt-get RUN chains, as Docker would run them."""
    source = DOCKERFILE.read_text().replace("\\\n", " ")
    transport = re.search(r'^ARG APT_TRANSPORT="([^"]*)"$', source, re.MULTILINE).group(1)
    return transport, re.findall(r"^RUN (.*apt-get.*)$", source, re.MULTILINE)


@unittest.skipUnless(
    Path("/proc").exists() and shutil.which("timeout")
    and "GNU coreutils" in subprocess.run(["timeout", "--version"], capture_output=True, text=True, check=False).stdout,
    "requires GNU timeout and Linux process inspection",
)
class BuildPackageDeadlines(unittest.TestCase):
    def test_stalled_acquisition_stops_children_and_later_steps(self):
        transport, commands = package_commands()
        self.assertEqual(len(commands), 2)
        for stage, command in enumerate(commands):
            # Accelerate only the deadlines; execute the real Dockerfile shell chain.
            command = command.replace("180s", "1s").replace("360s", "1s").replace("10s", "1s")
            for stalled_phase in ("update", "install"):
                with self.subTest(stage=stage, phase=stalled_phase), tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    apt = root / "apt-get"
                    apt.write_text(FAKE_APT)
                    apt.chmod(0o755)
                    environment = dict(os.environ, PATH=str(root) + os.pathsep + os.environ["PATH"],
                                       FRAYD_APT_FIXTURE=str(root), FRAYD_APT_STALL=stalled_phase,
                                       APT_TRANSPORT=transport)
                    # Replace everything after package installation with an observable marker.
                    acquisition = command.split("&& rm -rf", 1)[0] + " && touch " + shlex.quote(str(root / "later-step"))
                    process = subprocess.Popen(["sh", "-c", acquisition], env=environment,
                                               stdout=subprocess.PIPE, stderr=subprocess.STDOUT, start_new_session=True)
                    try:
                        output, _ = process.communicate(timeout=4)
                        child_states = {}
                        for pid in (root / "pids").read_text().split():
                            stat = Path("/proc") / pid / "stat"
                            if stat.exists():
                                child_states[pid] = stat.read_text().split(")", 1)[1].split()[0]
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.communicate()
                        self.fail("Package command exceeded the test watchdog")
                    finally:
                        # The watchdog owns only this fixture's process group.
                        try:
                            os.killpg(process.pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                    self.assertEqual(process.returncode, 124, output.decode())
                    self.assertFalse((root / "later-step").exists())
                    expected = ["update"] if stalled_phase == "update" else ["update", "install"]
                    calls = [line.split() for line in (root / "calls").read_text().splitlines()]
                    self.assertEqual([call[0] for call in calls], expected)
                    for call in calls:
                        for option in ("Acquire::http::Timeout=30", "Acquire::https::Timeout=30", "Acquire::Retries=2"):
                            self.assertIn(option, call)
                    for pid, state in child_states.items():
                        self.assertEqual(state, "Z", f"live APT descendant {pid}")


if __name__ == "__main__":
    unittest.main()
