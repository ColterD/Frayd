"""Installer contract.

bootstrap.sh runs for real, but only against recording PATH shims, so nothing is downloaded or installed and every
write stays inside a temporary HOME. bootstrap.ps1 is checked statically: its behavior is not executed here
because PowerShell may be unavailable.
"""
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
BOOTSTRAP_SH = ROOT / 'client/install/bootstrap.sh'
BOOTSTRAP_PS1 = ROOT / 'client/install/bootstrap.ps1'
BASH = shutil.which('bash')
# Real utilities the script needs for local file work. Everything else is a recording shim or absent.
REAL_TOOLS = ('mkdir', 'cat', 'tee', 'mktemp', 'rm', 'sh')
RECORDERS = ('apt-get', 'dnf', 'pacman', 'brew', 'systemctl', 'loginctl', 'launchctl', 'cargo', 'tailscale', 'cloudflared')
PACKAGE_MANAGERS = ('apt-get', 'dnf', 'pacman', 'brew')
DEFAULT_TOOLS = ('cargo', 'systemctl', 'loginctl', 'tailscale')

LOG = ('printf "%s" "${0##*/}" >> "$SHIM_LOG"; printf " %s" "$@" >> "$SHIM_LOG"; '
       'printf "\\n" >> "$SHIM_LOG"\n')
CURL_SHIM = LOG + r'''
out=""; prev=""; url=""
for argument in "$@"; do
  [ "$prev" = "-o" ] && out="$argument"
  prev="$argument"; url="$argument"
done
[ -n "${SHIM_CURL_FAIL:-}" ] && exit 22
payload() {
  case "$url" in
    *sh.rustup.rs*)
      echo 'echo "rustup-init $*" >> "$SHIM_LOG"'
      echo 'mkdir -p "$HOME/.cargo"'
      echo 'printf "export PATH=\"%s:\$PATH\"\n" "$SHIM_STASH" > "$HOME/.cargo/env"' ;;
    *tailscale.com/install.sh*) echo 'echo "tailscale-install" >> "$SHIM_LOG"' ;;
    *) echo "downloaded-data" ;;
  esac
}
if [ -n "$out" ]; then payload > "$out"; else payload; fi
# A transfer that fails after producing output: whatever it produced must never run.
[ -n "${SHIM_CURL_PARTIAL:-}" ] && exit 18
exit 0
'''
SHIMS = {
    'curl': CURL_SHIM,
    'sudo': LOG + 'if [ "${1:-}" = tee ]; then cat > /dev/null; fi\n',
    'git': LOG + 'if [ "${1:-}" = clone ]; then mkdir -p Frayd/server; fi\n',
}


class Run:
    """One bootstrap.sh run: exit status, output and the commands the shims recorded."""

    def __init__(self, result, home, calls):
        self.returncode = result.returncode
        self.stdout = result.stdout
        self.stderr = result.stderr
        self.home = home
        self.calls = calls

    def called(self, prefix):
        return any(call.startswith(prefix) for call in self.calls)


@unittest.skipUnless(BASH, 'bash is required')
class BootstrapShell(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name).resolve()
        self.library = self.root / 'media'
        for name in ('bin', 'home', 'work', 'tmp', 'stash'):
            (self.root / name).mkdir()

    def shim(self, directory, name, body):
        path = directory / name
        path.write_text(f'#!{BASH}\n{body}')
        path.chmod(0o755)

    def run_bootstrap(self, *arguments, stdin='', managers=('apt-get',), tools=DEFAULT_TOOLS, environment=None):
        """Run the real script with HOME, TMPDIR and PATH confined to the temporary root.

        PATH holds only the shim directory, so a command without a shim cannot run at all, and the sudo
        shim records its arguments instead of executing them.
        """
        shims = self.root / 'bin'
        for name in REAL_TOOLS:
            (shims / name).symlink_to(shutil.which(name))
        for name in RECORDERS:
            body = LOG + ('[ "$*" = "ip -4" ] && echo 100.64.0.1\nexit 0\n' if name == 'tailscale' else '')
            if name == 'cargo':
                self.shim(self.root / 'stash', name, body)
            if name in managers or name in tools:
                self.shim(shims, name, body)
        for name, body in SHIMS.items():
            self.shim(shims, name, body)
        env = {'PATH': str(shims), 'HOME': str(self.root / 'home'), 'TMPDIR': str(self.root / 'tmp'), 'USER': 'tester',
               'SHIM_LOG': str(self.root / 'calls.log'), 'SHIM_STASH': str(self.root / 'stash'), **(environment or {})}
        result = subprocess.run([BASH, str(BOOTSTRAP_SH), *arguments], input=stdin, env=env, cwd=self.root / 'work',
                                capture_output=True, text=True, timeout=30, check=False)
        log = self.root / 'calls.log'
        return Run(result, self.root / 'home', log.read_text().splitlines() if log.exists() else [])

    def assert_no_leftover_downloads(self, run):
        self.assertEqual(list((self.root / 'tmp').iterdir()), [], 'a failed download must not leave its temp file behind')

    def env_file(self, run):
        return (run.home / 'Frayd/server/.env').read_text()

    def assert_installed(self, run, expected_library):
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertIn(f'Media folder: {expected_library}', run.stdout)
        self.assertIn(f'FRAYD_LIBRARY={self.library}\n', self.env_file(run))
        self.assertTrue(self.library.is_dir())

    def test_library_then_tunnel(self):
        run = self.run_bootstrap(str(self.library), '--tunnel', 'tailscale')
        self.assert_installed(run, self.library)
        self.assertIn('Tailscale remote access', run.stdout)
        self.assertTrue(run.called('sudo tailscale up'))

    def test_tunnel_then_library(self):
        """Guards against '--tunnel' being taken as the library path when it precedes the path."""
        run = self.run_bootstrap('--tunnel', 'tailscale', str(self.library))
        self.assert_installed(run, self.library)
        self.assertIn('Tailscale remote access', run.stdout)
        self.assertTrue(run.called('sudo tailscale up'))

    def test_tunnel_without_library_prompts_for_one(self):
        """Guards against '--tunnel' becoming the library path so that no tunnel is selected."""
        run = self.run_bootstrap('--tunnel', 'cloudflared', stdin=f'{self.library}\n')
        self.assert_installed(run, '<unset>')
        self.assertIn('Cloudflare Tunnel remote access', run.stdout)
        self.assertTrue(run.called('sudo apt-get install -y -qq cloudflared'))

    def test_no_arguments_prompt_and_default_to_home_media(self):
        run = self.run_bootstrap(stdin='\n')
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertIn('Media folder: <unset>', run.stdout)
        self.assertIn(f'FRAYD_LIBRARY={run.home}/Media\n', self.env_file(run))
        self.assertIn('Local network only', run.stdout)

    def test_library_alone_is_local_only(self):
        run = self.run_bootstrap(str(self.library))
        self.assert_installed(run, self.library)
        self.assertIn('Local network only', run.stdout)
        self.assertFalse(run.called('sudo tailscale'))

    def test_tunnel_flag_without_value_or_with_unknown_value_is_local_only(self):
        for tunnel in (['--tunnel'], ['--tunnel', 'bogus']):
            with self.subTest(tunnel=tunnel):
                self.setUp()
                run = self.run_bootstrap(str(self.library), *tunnel)
                self.assert_installed(run, self.library)
                self.assertIn('Local network only', run.stdout)
                self.assertFalse(run.called('sudo tailscale'))

    def test_unknown_flag_stops_before_any_install(self):
        """Guards against an unknown flag becoming the library path and failing only after installs and clone."""
        run = self.run_bootstrap('--bogus')
        self.assertEqual(run.returncode, 2, run.stdout + run.stderr)
        self.assertIn('Unknown option: --bogus', run.stderr)
        self.assertEqual(run.calls, [])

    def test_failed_pipeline_download_stops_the_script(self):
        """Guards against a failed curl feeding 'sudo tee' going unnoticed when pipefail is off."""
        run = self.run_bootstrap(str(self.library), '--tunnel', 'cloudflared', environment={'SHIM_CURL_FAIL': '1'})
        self.assertNotEqual(run.returncode, 0)
        self.assertFalse(run.called('sudo apt-get install -y -qq cloudflared'))
        self.assertNotIn('Next steps', run.stdout)
        self.assertNotIn('Frayd running', run.stdout)

    def test_rustup_runs_from_a_completely_downloaded_file(self):
        run = self.run_bootstrap(str(self.library), tools=('systemctl', 'loginctl'))
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        download = next(call for call in run.calls if call.startswith('curl') and 'sh.rustup.rs' in call)
        self.assertIn(' -o ', download, 'rustup must be saved to a file before it runs')
        self.assertTrue(run.called('rustup-init -y --default-toolchain stable'))
        self.assertTrue(run.called('cargo build --release'))
        self.assertEqual(list((self.root / 'tmp').iterdir()), [], 'the downloaded installer must be removed')

    def test_truncated_rustup_download_never_runs(self):
        """Guards against the partial output of a failed curl being piped straight into sh, and against leftovers."""
        run = self.run_bootstrap(str(self.library), tools=('systemctl', 'loginctl'), environment={'SHIM_CURL_PARTIAL': '1'})
        self.assertNotEqual(run.returncode, 0)
        self.assertFalse(run.called('rustup-init'))
        self.assertFalse(run.called('cargo build'))
        self.assert_no_leftover_downloads(run)

    def test_failed_rustup_download_leaves_no_temp_file(self):
        run = self.run_bootstrap(str(self.library), tools=('systemctl', 'loginctl'), environment={'SHIM_CURL_FAIL': '1'})
        self.assertNotEqual(run.returncode, 0)
        self.assertFalse(run.called('rustup-init'))
        self.assert_no_leftover_downloads(run)

    def test_tailscale_installer_runs_from_a_completely_downloaded_file(self):
        run = self.run_bootstrap(str(self.library), '--tunnel', 'tailscale', tools=('cargo', 'systemctl', 'loginctl'))
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertTrue(run.called('tailscale-install'))
        self.assertIn(' -o ', next(call for call in run.calls if call.startswith('curl') and 'tailscale.com' in call))

    def test_truncated_tailscale_installer_never_runs(self):
        """Guards against the partial output of a failed curl being piped straight into sh, and against leftovers."""
        run = self.run_bootstrap(str(self.library), '--tunnel', 'tailscale', tools=('cargo', 'systemctl', 'loginctl'),
                                 environment={'SHIM_CURL_PARTIAL': '1'})
        self.assertNotEqual(run.returncode, 0)
        self.assertFalse(run.called('tailscale-install'))
        self.assertFalse(run.called('sudo tailscale up'))
        self.assert_no_leftover_downloads(run)

    def test_failed_tailscale_download_leaves_no_temp_file(self):
        run = self.run_bootstrap(str(self.library), '--tunnel', 'tailscale', tools=('cargo', 'systemctl', 'loginctl'),
                                 environment={'SHIM_CURL_FAIL': '1'})
        self.assertNotEqual(run.returncode, 0)
        self.assertFalse(run.called('tailscale-install'))
        self.assert_no_leftover_downloads(run)

    def test_final_message_claims_running_only_after_a_service_started(self):
        """Guards against the no-service case still printing 'Frayd running'."""
        started = self.run_bootstrap(str(self.library))
        self.assertEqual(started.returncode, 0, started.stdout + started.stderr)
        self.assertTrue(started.called('systemctl --user enable --now frayd.service'))
        self.assertIn('[OK] Frayd running at http://localhost:8484', started.stdout)
        self.setUp()
        launchd = self.run_bootstrap(str(self.library), tools=('cargo', 'launchctl'))
        self.assertEqual(launchd.returncode, 0, launchd.stdout + launchd.stderr)
        self.assertTrue(launchd.called('launchctl load'))
        self.assertIn('[OK] Frayd running at http://localhost:8484', launchd.stdout)
        self.setUp()
        manual = self.run_bootstrap(str(self.library), tools=('cargo',))
        self.assertEqual(manual.returncode, 0, manual.stdout + manual.stderr)
        self.assertIn('start manually', manual.stdout)
        self.assertNotIn('Frayd running', manual.stdout)
        self.assertIn('once started it serves http://localhost:8484', manual.stdout)
        self.assertFalse(manual.called('systemctl'))
        self.assertFalse(manual.called('launchctl'))


class BootstrapPowerShellStatic(unittest.TestCase):
    """Static checks only. ps1 behavior is not executed here: pwsh is not required to run the suite."""

    @classmethod
    def setUpClass(cls):
        cls.text = BOOTSTRAP_PS1.read_text()
        cls.code = '\n'.join(line for line in cls.text.splitlines() if not line.lstrip().startswith('#'))

    def test_env_file_is_written_without_a_byte_order_mark(self):
        self.assertRegex(self.code, r'\[IO\.File\]::WriteAllText\(\(Join-Path \$PWD "\.env"\)')
        self.assertNotRegex(self.code, r'(?m)^.*(Out-File|Set-Content|Add-Content).*\.env')
        self.assertNotRegex(self.code, r'Out-File\s+-Encoding\s+utf8')

    def test_every_native_call_is_checked(self):
        self.assertRegex(self.code, r'function Assert-NativeSuccess\b[^\n]*\$LASTEXITCODE -ne 0[^\n]*throw')
        lines = self.code.splitlines()
        checked = []
        for index, line in enumerate(lines):
            for match in re.finditer(r'(?:^|[;{]\s*)(git|cargo)\s+[a-z]', line):
                following = line[match.end():] + '\n' + (lines[index + 1] if index + 1 < len(lines) else '')
                self.assertRegex(following.strip(), r'Assert-NativeSuccess\b', f'unchecked native call: {line.strip()}')
                checked.append(match.group(0).strip(' ;{'))
        self.assertEqual(sorted(checked), ['cargo b', 'git c', 'git p'], 'git pull, git clone and cargo build must each be checked')

    def test_startup_script_loads_every_uncommented_env_line(self):
        self.assertIn('for /f "usebackq eol=# tokens=*" %%a in (".env") do set "%%a"', self.code)
        self.assertNotIn('set /p', self.code)

    def test_library_is_exported_before_the_server_is_started(self):
        launches = [match.start() for match in re.finditer(r'Start-Process[^\n]*frayd-server\.exe', self.code)]
        self.assertEqual(len(launches), 1)
        assignment = self.code.find('$env:FRAYD_LIBRARY = $Lib')
        self.assertGreaterEqual(assignment, 0)
        self.assertLess(assignment, launches[0])


if __name__ == '__main__':
    unittest.main()
