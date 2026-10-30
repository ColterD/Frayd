"""Windows qualification for client/install/bootstrap.ps1.

The unmodified production script runs inside real powershell.exe (Windows PowerShell 5.1) and pwsh through
tests/fixtures/bootstrap-windows.ps1. The fixture shadows Start-Process, Invoke-WebRequest and Out-File with recording
or rejecting functions; every other command is real. Native commands are .cmd stubs that return real exit codes. The
generated Startup batch file is then executed with cmd.exe against a small compiled stand-in for the server.

These are bounded interceptions for that one inspected script, not a security sandbox.

Without the interpreters, ordinary discovery skips these tests. FRAYD_WINDOWS_REQUIRE_QUALIFICATION=1 turns every
missing prerequisite into a failure, so a skip can never pass as qualification.
"""
import atexit
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import types
import unittest
from pathlib import Path

TESTS = Path(__file__).resolve().parent
ROOT = TESTS.parent
BOOTSTRAP_PS1 = ROOT / 'client/install/bootstrap.ps1'
FIXTURE = TESTS / 'fixtures/bootstrap-windows.ps1'
REQUIRE = os.environ.get('FRAYD_WINDOWS_REQUIRE_QUALIFICATION') == '1'
REQUIRED_PREFIX = 'Windows qualification is required but unavailable'
POLICY_CHILD = 'FRAYD_WINDOWS_POLICY_CHILD'
# The only parent variables a child process inherits: Windows system locations, none of them secret.
RUNTIME_ENV = ('SYSTEMROOT', 'WINDIR', 'SYSTEMDRIVE', 'COMSPEC', 'PATHEXT', 'OS', 'PROCESSOR_ARCHITECTURE',
               'NUMBER_OF_PROCESSORS', 'PROGRAMFILES', 'PROGRAMFILES(X86)', 'PROGRAMW6432', 'COMMONPROGRAMFILES',
               'COMMONPROGRAMFILES(X86)', 'COMMONPROGRAMW6432', 'PROGRAMDATA', 'ALLUSERSPROFILE')


def runtime_environment(**extra):
    """A child environment built from RUNTIME_ENV plus explicit values, never a copy of this process's environment."""
    env = {key: os.environ[key] for key in RUNTIME_ENV if key in os.environ}
    env.update(extra)
    return env

UNICODE_PROFILE = 'Jos\u00e9 Pr\u00f6file \u65e5\u672c'
UNICODE_LIBRARY = 'M\u00e9dia Libr\u00e4ry \u00e8\u00e9 \u65e5\u672c\u8a9e'
ENV_COMMENTS = ['# Public internet exposure? Uncomment for shorter sessions (days):', '# FRAYD_SESSION_DAYS=7']
LEGACY_CODE_PAGE = '437'
ALREADY_INSTALLED = -1978335135
NO_APPLICABLE_UPDATE = -1978335189

STUB_SOURCE = r'''
using System;
using System.IO;
using System.Text;
public static class Stub {
  // Only these oracle keys are recorded; never enumerate the environment.
  static readonly string[] Recorded = { "FRAYD_LIBRARY", "FRAYD_SESSION_DAYS" };
  public static int Main(string[] args) {
    string record = Environment.GetEnvironmentVariable("FRAYD_STUB_RECORD");
    if (!string.IsNullOrEmpty(record)) {
      StringBuilder text = new StringBuilder();
      text.Append("exe=").Append(Environment.GetCommandLineArgs()[0]).Append("\n");
      text.Append("cwd=").Append(Directory.GetCurrentDirectory()).Append("\n");
      text.Append("args=").Append(string.Join(" ", args)).Append("\n");
      foreach (string key in Recorded) {
        string value = Environment.GetEnvironmentVariable(key);
        if (value != null) {
          text.Append("env:").Append(key).Append("=").Append(value).Append("\n");
        }
      }
      File.AppendAllText(record, text.ToString(), new UTF8Encoding(false));
    }
    int code;
    return int.TryParse(Environment.GetEnvironmentVariable("FRAYD_STUB_EXIT"), out code) ? code : 0;
  }
}
'''

CMD_STUBS = {
    'git': '''>>"%FAKE_LOG%" echo git %*
if /i "%~1"=="clone" goto clone
if /i "%~1"=="pull" goto pull
exit /b 99
:clone
if defined FAKE_GIT_CLONE_EXIT exit /b %FAKE_GIT_CLONE_EXIT%
mkdir Frayd\\server
exit /b 0
:pull
if defined FAKE_GIT_PULL_EXIT exit /b %FAKE_GIT_PULL_EXIT%
exit /b 0
''',
    'cargo': '''>>"%FAKE_LOG%" echo cargo %*
if defined FAKE_CARGO_EXIT exit /b %FAKE_CARGO_EXIT%
exit /b 0
''',
    'winget': '''>>"%FAKE_LOG%" echo winget %*
if defined FAKE_WINGET_EXIT exit /b %FAKE_WINGET_EXIT%
exit /b 0
''',
    'tailscale': '''>>"%FAKE_LOG%" echo tailscale %*
if defined FAKE_TAILSCALE_STDERR echo fixture diagnostic 1>&2
if defined FAKE_TAILSCALE_EXIT exit /b %FAKE_TAILSCALE_EXIT%
if /i "%~1"=="ip" echo 100.64.0.7
exit /b 0
''',
    'cloudflared': '''>>"%FAKE_LOG%" echo cloudflared %*
exit /b 0
''',
    'ffmpeg': '''>>"%FAKE_LOG%" echo ffmpeg %*
exit /b 0
''',
}

_STUB = {}


def unavailable(reason):
    """Skip in ordinary discovery; fail closed when qualification is required."""
    if REQUIRE:
        raise AssertionError(f'{REQUIRED_PREFIX}: {reason}')
    raise unittest.SkipTest(reason)


def stub_executable():
    """Compile the recording stand-in once with the in-box .NET Framework C# compiler."""
    if 'path' in _STUB:
        return _STUB['path']
    windows = os.environ.get('WINDIR') or os.environ.get('SystemRoot') or ''
    compilers = [Path(windows, 'Microsoft.NET', framework, 'v4.0.30319', 'csc.exe') for framework in ('Framework64', 'Framework')]
    compiler = next((candidate for candidate in compilers if candidate.is_file()), None)
    if compiler is None:
        unavailable('the .NET Framework C# compiler (csc.exe) is missing')
    directory = Path(tempfile.mkdtemp(prefix='frayd-stub-'))
    atexit.register(shutil.rmtree, directory, True)
    source = directory / 'stub.cs'
    source.write_text(STUB_SOURCE, encoding='ascii')
    output = directory / 'stub.exe'
    result = subprocess.run([str(compiler), '/nologo', '/target:exe', f'/out:{output}', str(source)],
                            env=runtime_environment(TEMP=str(directory), TMP=str(directory)),
                            capture_output=True, text=True, timeout=120, check=False)
    if result.returncode != 0 or not output.is_file():
        unavailable(f'the stub server did not compile: {result.stdout}{result.stderr}')
    _STUB['path'] = output
    return output


class Run:
    """One fixture run: process status, console output, fixture events and the native stub calls."""

    def __init__(self, result, case):
        self.returncode = result.returncode
        self.stdout = result.stdout.decode('utf-8', 'replace')
        self.stderr = result.stderr.decode('utf-8', 'replace')
        text = case.events.read_text(encoding='utf-8') if case.events.exists() else ''
        self.events = [line.split('|') for line in text.splitlines()]
        self.calls = [line.strip() for line in case.log.read_text(errors='replace').splitlines()] if case.log.exists() else []

    def of(self, kind):
        return [event for event in self.events if event[0] == kind]

    @property
    def outcome(self):
        results = self.of('result')
        return (results[0][1], '|'.join(results[0][2:])) if results else None

    def called(self, prefix):
        return any(call.startswith(prefix) for call in self.calls)

    def describe(self):
        return (f'exit={self.returncode}\nevents={self.events}\ncalls={self.calls}\n'
                f'stdout={self.stdout}\nstderr={self.stderr}')


class QualificationMixin:
    """The qualification cases; each concrete class binds them to one PowerShell interpreter."""

    interpreter_name = ''

    @classmethod
    def setUpClass(cls):
        super().setUpClass()
        if os.name != 'nt':
            unavailable('these tests run only on Windows')
        if not Path(os.environ.get('SystemRoot', ''), 'System32', 'cmd.exe').is_file():
            unavailable('cmd.exe is missing')
        found = shutil.which(cls.interpreter_name)
        if not found:
            unavailable(f'{cls.interpreter_name} is not on PATH')
        cls.interpreter = found
        cls.stub_exe = stub_executable()
        # Fail once per engine before installer scenarios if the synthetic profile is not a known-folder profile.
        probe = cls()
        try:
            case = probe.make_case()
            run = probe.install(case, setup_only=True)
            probe.assertEqual(run.returncode, 0, run.describe())
            probe.assertEqual(run.outcome, ('setup-only', '0'), run.describe())
            probe.assertEqual(run.calls, [], run.describe())
            probe.assertFalse((case.startup / 'frayd.bat').exists(), run.describe())
            probe.assertFalse((case.profile / 'Frayd').exists(), run.describe())
        finally:
            probe.doCleanups()

    def check_engine(self, version, edition):
        raise NotImplementedError

    def make_case(self, *, profile='user', library='media', git=True, cargo='path', ffmpeg=True, winget=True,
                  tailscale=False, cloudflared=False, existing=False):
        directory = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
        self.addCleanup(directory.cleanup)
        root = Path(directory.name).resolve()
        case = types.SimpleNamespace(root=root, profile=root / profile, library=root / library, bin=root / 'bin',
                                     startup=root / 'startup', events=root / 'events.log', log=root / 'calls.log')
        case.appdata = case.profile / 'AppData/Roaming'
        case.known_startup = case.appdata / 'Microsoft/Windows/Start Menu/Programs/Startup'
        for folder in (case.known_startup, case.bin, case.startup, root / 'work', root / 'tmp',
                       root / 'localappdata', root / 'elsewhere'):
            folder.mkdir(parents=True)
        if existing:
            (case.profile / 'Frayd/server').mkdir(parents=True)
        wanted = {'git': git, 'winget': winget, 'ffmpeg': ffmpeg, 'tailscale': tailscale, 'cloudflared': cloudflared}
        if cargo == 'path':
            wanted['cargo'] = True
        for name, present in wanted.items():
            if present:
                self.write_stub(case.bin, name)
        if cargo == 'rustup':
            (case.profile / '.cargo/bin').mkdir(parents=True)
            self.write_stub(case.profile / '.cargo/bin', 'cargo')
        return case

    def write_stub(self, directory, name):
        body = '@echo off\n' + CMD_STUBS[name]
        (directory / f'{name}.cmd').write_bytes(body.replace('\n', '\r\n').encode('ascii'))

    def environment(self, case, **extra):
        """Build each case's environment from scratch: no parent variable is copied unless it is named here."""
        system = Path(os.environ['SystemRoot'])
        env = runtime_environment(
            USERPROFILE=str(case.profile), APPDATA=str(case.appdata),
            LOCALAPPDATA=str(case.root / 'localappdata'), TEMP=str(case.root / 'tmp'), TMP=str(case.root / 'tmp'),
            PATH=os.pathsep.join([str(case.bin), str(system / 'System32'), str(system), str(system / 'System32/Wbem')]),
            FAKE_LOG=str(case.log), POWERSHELL_TELEMETRY_OPTOUT='1', POWERSHELL_UPDATECHECK='Off')
        env.update(extra)
        return env

    def install(self, case, *, library=None, tunnel='', fake=None, rustup=False, setup_only=False):
        arguments = [self.interpreter, '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', str(FIXTURE),
                     '-InstallerPath', str(BOOTSTRAP_PS1), '-Events', str(case.events),
                     '-StartupRedirect', str(case.startup / 'frayd.bat'), '-Lib', str(library or case.library)]
        if setup_only:
            arguments += ['-SetupOnly']
        if tunnel:
            arguments += ['-Tunnel', tunnel]
        if rustup:
            arguments += ['-RustupStub', str(self.stub_exe)]
        result = subprocess.run(arguments, env=self.environment(case, **(fake or {})), cwd=case.root / 'work',
                                stdin=subprocess.DEVNULL, capture_output=True, timeout=240, check=False)
        run = Run(result, case)
        engine = run.of('engine')
        self.assertEqual(len(engine), 1, run.describe())
        self.check_engine(engine[0][1], engine[0][2])
        self.assertEqual(run.of('startup'), [['startup', str(case.known_startup)]], run.describe())
        self.assertIsNotNone(run.outcome, run.describe())
        return run

    def assert_installed(self, run, case):
        self.assertEqual(run.outcome[0], 'returned', run.describe())
        self.assertIn('[OK] Frayd installed. Starting now...', run.stdout, run.describe())
        self.assertIn('[OK] Runs at startup. Open http://localhost:8484', run.stdout, run.describe())
        servers = [event for event in run.of('process') if Path(event[1]).name.lower() == 'frayd-server.exe']
        self.assertEqual(len(servers), 1, run.describe())
        expected = case.profile / 'Frayd/server/target/release/frayd-server.exe'
        self.assertEqual(os.path.normcase(servers[0][1]), os.path.normcase(str(expected)), run.describe())
        self.assertEqual(servers[0][3], str(case.library), 'the library must be exported before the server starts')
        self.assertEqual(servers[0][4], 'Hidden')
        self.assertTrue((case.startup / 'frayd.bat').is_file(), run.describe())
        self.assertTrue(case.library.is_dir(), run.describe())

    def test_new_checkout_installs(self):
        case = self.make_case()
        run = self.install(case)
        self.assert_installed(run, case)
        self.assertEqual(run.calls[0], 'git clone https://github.com/kattiest/Frayd.git', run.describe())
        self.assertEqual(run.calls[-1], 'cargo build --release', run.describe())
        self.assertFalse(run.called('git pull'))
        self.assertFalse(run.called('winget'))
        self.assertEqual(run.of('web'), [])

    def test_existing_checkout_is_pulled_not_cloned(self):
        case = self.make_case(existing=True)
        run = self.install(case)
        self.assert_installed(run, case)
        self.assertTrue(run.called('git pull --ff-only'), run.describe())
        self.assertFalse(run.called('git clone'), run.describe())
        self.assertEqual(run.calls[-1], 'cargo build --release')

    def test_unicode_and_space_paths_install(self):
        case = self.make_case(profile=UNICODE_PROFILE, library=UNICODE_LIBRARY)
        run = self.install(case)
        self.assert_installed(run, case)
        self.assertTrue((case.profile / 'Frayd/server/.env').is_file())

    def test_env_file_is_exact_utf8_without_a_byte_order_mark(self):
        case = self.make_case(profile=UNICODE_PROFILE, library=UNICODE_LIBRARY)
        run = self.install(case)
        self.assertEqual(run.outcome[0], 'returned', run.describe())
        raw = (case.profile / 'Frayd/server/.env').read_bytes()
        self.assertFalse(raw.startswith(b'\xef\xbb\xbf'), 'a byte order mark would corrupt the first variable name')
        self.assertEqual(raw.decode('utf-8').splitlines(), [f'FRAYD_LIBRARY={case.library}', *ENV_COMMENTS])
        self.assertIn(str(case.library).encode('utf-8'), raw)

    def test_relative_library_is_absolute_for_initial_and_startup_launch(self):
        case = self.make_case(profile=UNICODE_PROFILE)
        relative = Path(UNICODE_LIBRARY) / 'nested'
        case.library = case.profile / relative
        run = self.install(case, library=relative)
        self.assert_installed(run, case)
        raw = (case.profile / 'Frayd/server/.env').read_bytes()
        self.assertFalse(raw.startswith(b'\xef\xbb\xbf'))
        self.assertEqual(raw.decode('utf-8').splitlines(), [f'FRAYD_LIBRARY={case.library}', *ENV_COMMENTS])
        result, directory, variables, pages = self.run_startup_batch(case, case.startup / 'frayd.bat')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(pages, (LEGACY_CODE_PAGE, '65001'))
        self.assertEqual(os.path.normcase(directory), os.path.normcase(str(case.profile / 'Frayd/server')))
        self.assertEqual(variables.get('FRAYD_LIBRARY'), str(case.library))

    def test_native_failures_stop_every_later_action(self):
        rows = (
            ('clone', {}, {'FAKE_GIT_CLONE_EXIT': '128'}, 'git clone failed (exit 128)', ('cargo',), False),
            ('pull', {'existing': True}, {'FAKE_GIT_PULL_EXIT': '1'}, 'git pull failed (exit 1)', ('cargo',), False),
            ('build', {}, {'FAKE_CARGO_EXIT': '101'}, 'cargo build failed (exit 101)', (), False),
            ('ffmpeg install', {'ffmpeg': False}, {'FAKE_WINGET_EXIT': '1'},
             'winget install Gyan.FFmpeg failed (exit 1)', ('git', 'cargo'), False),
            ('rustup', {'cargo': None}, {'FRAYD_STUB_EXIT': '9'}, 'rustup-init failed (exit 9)', ('git', 'cargo'), False),
            ('tailscale install', {'tailscale': False}, {'FAKE_WINGET_EXIT': '1'},
             'winget install tailscale.tailscale failed (exit 1)', (), True),
            ('cloudflared install', {'cloudflared': False}, {'FAKE_WINGET_EXIT': '1'},
             'winget install Cloudflare.cloudflared failed (exit 1)', (), True),
        )
        for label, options, fake, message, forbidden, env_written in rows:
            with self.subTest(label):
                case = self.make_case(**options)
                tunnel = {'tailscale install': 'tailscale', 'cloudflared install': 'cloudflared'}.get(label, '')
                run = self.install(case, tunnel=tunnel, fake=fake, rustup=label == 'rustup')
                self.assertEqual(run.outcome[0], 'threw', run.describe())
                self.assertIn(message, run.outcome[1], run.describe())
                for prefix in forbidden:
                    self.assertFalse(run.called(prefix), run.describe())
                self.assertEqual((case.profile / 'Frayd/server/.env').exists(), env_written, run.describe())
                self.assertEqual(run.of('process'), [], run.describe())
                self.assertEqual(run.of('outfile'), [], run.describe())
                self.assertFalse((case.startup / 'frayd.bat').exists(), run.describe())
                self.assertNotIn('Frayd installed', run.stdout)
                self.assertNotIn('Runs at startup', run.stdout)

    def test_missing_git_stops_before_anything_else(self):
        case = self.make_case(git=False)
        run = self.install(case)
        self.assertIn('[!] git is required', run.stdout, run.describe())
        self.assertEqual(run.outcome, ('returned', '1'), run.describe())
        self.assertEqual(run.calls, [])
        self.assertEqual(run.of('process') + run.of('outfile') + run.of('web'), [])
        self.assertFalse((case.profile / 'Frayd').exists())

    def test_rustup_installer_runs_when_cargo_is_missing(self):
        case = self.make_case(cargo='rustup')
        record = case.root / 'rustup-record.txt'
        run = self.install(case, rustup=True, fake={'FRAYD_STUB_RECORD': str(record)})
        self.assert_installed(run, case)
        self.assertEqual([event[1] for event in run.of('web')], ['https://win.rustup.rs/x86_64'], run.describe())
        self.assertIn('args=-y --default-toolchain stable', record.read_text(encoding='utf-8'))
        self.assertTrue(run.called('cargo build --release'), run.describe())

    def test_winget_already_installed_codes_do_not_stop_the_install(self):
        for code, succeeds in ((0, True), (ALREADY_INSTALLED, True), (NO_APPLICABLE_UPDATE, True), (-1978335104, False)):
            with self.subTest(code=code):
                case = self.make_case(ffmpeg=False)
                run = self.install(case, fake={'FAKE_WINGET_EXIT': str(code)})
                self.assertTrue(run.called('winget install -e --id Gyan.FFmpeg'), run.describe())
                if succeeds:
                    self.assert_installed(run, case)
                else:
                    self.assertEqual(run.outcome[0], 'threw', run.describe())
                    self.assertIn(f'winget install Gyan.FFmpeg failed (exit {code})', run.outcome[1], run.describe())

    def test_tailscale_address_follows_the_lookup_exit_code(self):
        rows = (
            ('address', {'tailscale': True}, {}, "[OK] Your server's Tailscale address: 100.64.0.7"),
            ('lookup fails', {'tailscale': True}, {'FAKE_TAILSCALE_EXIT': '1'}, '[!] No Tailscale address yet'),
            ('stderr with address', {'tailscale': True}, {'FAKE_TAILSCALE_STDERR': '1'}, "[OK] Your server's Tailscale address: 100.64.0.7"),
            ('stderr without address', {'tailscale': True}, {'FAKE_TAILSCALE_STDERR': '1', 'FAKE_TAILSCALE_EXIT': '1'}, '[!] No Tailscale address yet'),
            ('not on PATH after install', {'tailscale': False}, {}, '[!] No Tailscale address yet'),
        )
        for label, options, fake, expected in rows:
            with self.subTest(label):
                case = self.make_case(**options)
                run = self.install(case, tunnel='tailscale', fake=fake)
                self.assert_installed(run, case)
                self.assertIn(expected, run.stdout, run.describe())
                self.assertEqual(next((event[1] for event in run.of('process')), None), 'tailscale', run.describe())

    def test_no_network_or_service_effects(self):
        real_startup = Path(os.environ.get('APPDATA', ''), 'Microsoft/Windows/Start Menu/Programs/Startup/frayd.bat')

        def snapshot():
            return (real_startup.stat().st_mtime_ns, real_startup.stat().st_size) if real_startup.exists() else None

        before = snapshot()
        for tunnel in ('', 'tailscale', 'cloudflared'):
            with self.subTest(tunnel=tunnel):
                case = self.make_case(tailscale=True, cloudflared=True)
                run = self.install(case, tunnel=tunnel)
                self.assert_installed(run, case)
                self.assertEqual(run.of('web'), [], run.describe())
                self.assertFalse(run.called('winget'), run.describe())
                allowed = {'frayd-server.exe', 'tailscale'}
                self.assertTrue(all(Path(event[1]).name in allowed for event in run.of('process')), run.describe())
                self.assertEqual([event[2] for event in run.of('outfile')], ['ascii'], run.describe())
        self.assertEqual(snapshot(), before, 'the real Startup folder must not be touched')

    def run_startup_batch(self, case, batch):
        """Run a launcher from a cmd child that starts on legacy code page 437 and records the code page around it.

        CREATE_NEW_CONSOLE gives the child its own console, hidden by SW_HIDE, so chcp changes only that console.
        """
        record = case.root / 'server-record.txt'
        wrapper = batch.with_name(f'legacy-{batch.stem}.bat')
        wrapper.write_bytes(('@echo off\r\n'
                             f'chcp {LEGACY_CODE_PAGE} >nul || exit /b 90\r\n'
                             'chcp >"%~dp0codepage-before.txt"\r\n'
                             f'call "%~dp0{batch.name}"\r\n'
                             'chcp >"%~dp0codepage-after.txt"\r\n').encode('ascii'))
        release = case.profile / 'Frayd/server/target/release'
        release.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(self.stub_exe, release / 'frayd-server.exe')
        if record.exists():
            record.unlink()
        result = subprocess.run(['cmd.exe', '/d', '/c', str(wrapper)], env=self.environment(case, FRAYD_STUB_RECORD=str(record)),
                                cwd=case.root / 'elsewhere', stdin=subprocess.DEVNULL, capture_output=True, timeout=60,
                                check=False, creationflags=subprocess.CREATE_NEW_CONSOLE,
                                startupinfo=subprocess.STARTUPINFO(dwFlags=subprocess.STARTF_USESHOWWINDOW,
                                                                   wShowWindow=subprocess.SW_HIDE))
        deadline = time.monotonic() + 30
        while not record.exists() and time.monotonic() < deadline:
            time.sleep(0.1)
        self.assertTrue(record.exists(), f'the server stand-in never ran: {result.stdout!r} {result.stderr!r}')
        lines = record.read_text(encoding='utf-8').splitlines()
        self.assertTrue(all(line.startswith(('exe=', 'cwd=', 'args=', 'env:')) for line in lines), 'unexpected record line')
        variables = {line[4:].split('=', 1)[0].upper(): line[4:].split('=', 1)[1] for line in lines if line.startswith('env:')}
        directory = next(line[4:] for line in lines if line.startswith('cwd='))
        pages = tuple(''.join(re.findall(r'\d+', batch.with_name(name).read_text(encoding='ascii')))
                      for name in ('codepage-before.txt', 'codepage-after.txt'))
        return result, directory, variables, pages

    def test_generated_startup_batch_starts_the_server_with_the_library(self):
        case = self.make_case(profile=UNICODE_PROFILE, library=UNICODE_LIBRARY)
        run = self.install(case)
        self.assert_installed(run, case)
        batch = case.startup / 'frayd.bat'
        self.assertIn('%USERPROFILE%', batch.read_bytes().decode('ascii'), 'the launcher must be pure ASCII')
        result, directory, variables, pages = self.run_startup_batch(case, batch)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(pages, (LEGACY_CODE_PAGE, '65001'), 'the launcher must switch a legacy console to UTF-8')
        self.assertEqual(os.path.normcase(directory), os.path.normcase(str(case.profile / 'Frayd/server')))
        self.assertEqual(variables.get('FRAYD_LIBRARY'), str(case.library))
        self.assertNotIn('FRAYD_SESSION_DAYS', variables, 'commented .env lines must not be loaded')
        self.assertEqual(sorted(variables), ['FRAYD_LIBRARY'], 'the stand-in records only its oracle keys')

    def test_startup_batch_needs_the_utf8_code_page(self):
        """Control: on code page 437 without chcp 65001 the non-ASCII library must not survive, so the case above has teeth."""
        case = self.make_case(profile=UNICODE_PROFILE, library=UNICODE_LIBRARY)
        run = self.install(case)
        self.assert_installed(run, case)
        text = (case.startup / 'frayd.bat').read_bytes().decode('ascii')
        stripped = case.root / 'no-chcp.bat'
        stripped.write_bytes(re.sub(r'(?m)^chcp 65001 >nul\r?\n', '', text).encode('ascii'))
        self.assertNotIn('chcp', stripped.read_text())
        result, _, variables, pages = self.run_startup_batch(case, stripped)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(pages, (LEGACY_CODE_PAGE, LEGACY_CODE_PAGE), 'the control must run on the legacy code page')
        self.assertNotEqual(variables.get('FRAYD_LIBRARY'), str(case.library))


class WindowsPowerShell51(QualificationMixin, unittest.TestCase):
    interpreter_name = 'powershell'

    def test_generated_file_entry_overrides_only_process_restricted_policy(self):
        # No persistent policy writes. A managed policy is a held qualification,
        # rather than permission to override the administrator's policy.
        env = runtime_environment(PATH=os.pathsep.join([str(Path(self.interpreter).parent),
                                                      str(Path(os.environ['SystemRoot']) / 'System32')]))
        policy = subprocess.run([self.interpreter, '-NoProfile', '-NonInteractive', '-Command',
            "Get-ExecutionPolicy -Scope MachinePolicy; Get-ExecutionPolicy -Scope UserPolicy"],
            env=env, capture_output=True, text=True, timeout=30, check=False)
        self.assertEqual(policy.returncode, 0, policy.stderr)
        self.assertEqual(policy.stdout.split(), ['Undefined', 'Undefined'],
                         'qualification held: Group Policy governs script execution')
        with tempfile.TemporaryDirectory(prefix="Frayd entry owner's ") as directory:
            script = Path(directory) / 'frayd-install.ps1'
            script.write_text("param([string]$Lib)\nWrite-Output $Lib\n", encoding='ascii')
            env.update(TEMP=directory, TMP=directory)
            base = [self.interpreter, '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Restricted', '-Command']
            blocked = subprocess.run(base + ["& '.\\frayd-install.ps1' -Lib 'harmless fixture'"],
                                     env=env, cwd=directory, capture_output=True, text=True, timeout=30, check=False)
            self.assertNotEqual(blocked.returncode, 0, 'Restricted control must refuse the file')
            self.assertNotIn('harmless fixture', blocked.stdout)
            template = re.search(r"; (powershell\.exe [^\n]+)\$\{argStr\}`;", (ROOT / 'client/install/index.html').read_text())
            self.assertIsNotNone(template, 'generated file entry must be present')
            command = template.group(1).replace('\\\\', '\\') + " -Lib 'Owner''s $Library with spaces'"
            allowed = subprocess.run(base + [command], env=env, cwd=directory, capture_output=True, text=True,
                                     timeout=30, check=False)
            self.assertEqual(allowed.returncode, 0, allowed.stderr)
            self.assertEqual(allowed.stdout.strip(), "Owner's $Library with spaces")

    def check_engine(self, version, edition):
        self.assertTrue(version.startswith('5.1.') and edition == 'Desktop', f'expected Windows PowerShell 5.1, ran {version} {edition}')


class PowerShellCore(QualificationMixin, unittest.TestCase):
    interpreter_name = 'pwsh'

    def check_engine(self, version, edition):
        self.assertTrue(edition == 'Core' and int(version.split('.')[0]) >= 7, f'expected PowerShell 7 or later, ran {version} {edition}')


@unittest.skipIf(os.environ.get(POLICY_CHILD), 'a policy child run must not start another one')
class QualificationPolicy(unittest.TestCase):
    """Missing prerequisites skip in ordinary discovery and fail closed under FRAYD_WINDOWS_REQUIRE_QUALIFICATION=1."""

    def discover_without_interpreters(self, required):
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as empty:
            env = runtime_environment(PATH=empty, TEMP=empty, TMP=empty, **{POLICY_CHILD: '1'})
            if required:
                env['FRAYD_WINDOWS_REQUIRE_QUALIFICATION'] = '1'
            return subprocess.run([sys.executable, '-m', 'unittest', 'discover', '-s', str(TESTS), '-p', Path(__file__).name],
                                  env=env, cwd=empty, capture_output=True, text=True, timeout=120, check=False)

    def test_missing_interpreters_skip_in_ordinary_discovery(self):
        result = self.discover_without_interpreters(required=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('skipped', result.stderr)
        self.assertNotIn(REQUIRED_PREFIX, result.stderr)

    def test_missing_interpreters_fail_closed_when_qualification_is_required(self):
        result = self.discover_without_interpreters(required=True)
        self.assertNotEqual(result.returncode, 0, result.stderr)
        self.assertIn(REQUIRED_PREFIX, result.stderr)

    def test_powershell_sources_are_ascii_for_windows_powershell_5_1(self):
        """Windows PowerShell 5.1 reads a script without a byte order mark as the ANSI code page."""
        for script in (BOOTSTRAP_PS1, FIXTURE):
            self.assertTrue(all(byte < 128 for byte in script.read_bytes()), script.name)


if __name__ == '__main__':
    unittest.main()
