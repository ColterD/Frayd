"""Exercise Docker package-step deadlines without host APT or network access."""
import os
from pathlib import Path
import re
import shutil
import shlex
import signal
import subprocess
import tempfile
import time
import unittest


DOCKERFILE = Path(os.environ.get("FRAYD_TEST_DOCKERFILE", Path(__file__).resolve().parents[1] / "Dockerfile"))


def package_commands():
    source = DOCKERFILE.read_text().replace("\\\n", " ")
    return re.findall(r"^RUN (.*apt-get.*)$", source, re.MULTILINE)


@unittest.skipUnless(
    Path("/proc").exists() and shutil.which("timeout")
    and "GNU coreutils" in subprocess.run(["timeout", "--version"], capture_output=True, text=True, check=False).stdout,
    "requires GNU timeout and Linux process inspection",
)
class BuildPackageDeadlines(unittest.TestCase):
    def test_stalled_acquisition_stops_children_and_later_steps(self):
        commands = package_commands()
        self.assertEqual(len(commands), 2)
        for stage, command in enumerate(commands):
            # Accelerate only the deadlines; execute the real Dockerfile shell chain.
            command = command.replace("180s", "1s").replace("360s", "1s").replace("10s", "1s")
            for stalled_phase in ("update", "install"):
                with self.subTest(stage=stage, phase=stalled_phase), tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    apt = root / "apt-get"
                    apt.write_text("""#!/usr/bin/env python3
import os
from pathlib import Path
import subprocess
import sys
import time
root = Path(os.environ['FRAYD_APT_FIXTURE'])
phase = 'update' if 'update' in sys.argv else 'install'
with (root / 'calls').open('a') as output:
    output.write(phase + '\\n')
if phase == os.environ['FRAYD_APT_STALL']:
    child = subprocess.Popen(['sleep', '60'])
    (root / 'pids').write_text(str(os.getpid()) + ' ' + str(child.pid))
    time.sleep(60)
""")
                    apt.chmod(0o755)
                    environment = dict(os.environ, PATH=str(root) + os.pathsep + os.environ["PATH"],
                                       FRAYD_APT_FIXTURE=str(root), FRAYD_APT_STALL=stalled_phase)
                    # Replace everything after package installation with an observable marker.
                    acquisition = command.split("&& rm -rf", 1)[0] + " && touch " + shlex.quote(str(root / "later-step"))
                    start = time.monotonic()
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
                        self.fail("Package command exceeded the test watchdog (old unbounded behavior)")
                    finally:
                        # The watchdog owns only this fixture's process group.
                        try:
                            os.killpg(process.pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                    self.assertEqual(process.returncode, 124, output.decode())
                    self.assertLess(time.monotonic() - start, 4)
                    self.assertFalse((root / "later-step").exists())
                    expected = ["update"] if stalled_phase == "update" else ["update", "install"]
                    self.assertEqual((root / "calls").read_text().splitlines(), expected)
                    for pid, state in child_states.items():
                        self.assertEqual(state, "Z", f"live APT descendant {pid}")
                    print(f"stage={stage} phase={stalled_phase} exit=124 elapsed={time.monotonic() - start:.3f}s "
                          "live_descendants=0 later_step_absent=1")


if __name__ == "__main__":
    unittest.main()
