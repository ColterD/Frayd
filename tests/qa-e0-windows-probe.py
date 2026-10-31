"""Owned Windows-only probe; python probe.py REPOSITORY BASELINE_BOOTSTRAP."""
import os
from pathlib import Path
import sys
import unittest

repo = Path(sys.argv[1]).resolve()
baseline = Path(sys.argv[2]).resolve()
if os.name != 'nt':
    raise SystemExit('Windows required; no qualification performed')
os.environ['FRAYD_WINDOWS_REQUIRE_QUALIFICATION'] = '1'
sys.path.insert(0, str(repo / 'tests'))
import test_bootstrap_windows as w

method = 'test_tailscale_address_follows_the_lookup_exit_code'
for engine in (w.WindowsPowerShell51, w.PowerShellCore):
    for label, installer in (('RED baseline', baseline), ('GREEN current', w.BOOTSTRAP_PS1)):
        def install(self, case, _installer=installer, **kwargs):
            return w.QualificationMixin.install(self, case, installer_path=_installer, **kwargs)
        scoped = type('Probe' + engine.__name__, (engine,), {'install': install})
        print(label, engine.__name__, flush=True)
        result = unittest.TextTestRunner(verbosity=2).run(unittest.TestSuite([scoped(method)]))
        if label.startswith('RED'):
            if result.errors or result.skipped or len(result.failures) != 2:
                raise SystemExit('baseline must fail exactly the two nonzero lookup rows')
        elif not result.wasSuccessful() or result.skipped:
            raise SystemExit('patched lookup qualification failed')

# Use the same owned Win32 Job runner as generated-entry cases, with a fresh fixture per control.
for engine in ('powershell', 'pwsh'):
    interpreter = w.shutil.which(engine)
    if not interpreter:
        raise SystemExit(f'missing {engine}')
    with w.job_fixture(prefix='Frayd exit semantic ') as directory:
        root = Path(directory)
        optional = root / 'optional.ps1'
        optional.write_text('cmd.exe /d /c exit 7\nWrite-Host "completed"\n', encoding='ascii')
        throwing = root / 'throw.ps1'
        throwing.write_text("throw 'mandatory failure'\n", encoding='ascii')
        quoted = "'" + str(optional).replace("'", "''") + "'"
        controls = (
            ('file optional failure', ['-File', str(optional)], 0),
            ('command native failure', ['-Command', 'cmd.exe /d /c exit 7'], 1),
            ('command explicit native status', ['-Command', 'cmd.exe /d /c exit 7; exit $LASTEXITCODE'], 7),
            ('in-process residual', ['-Command', '& ' + quoted + '; Write-Host "residual=$LASTEXITCODE"'], 0),
            ('file terminating error', ['-File', str(throwing)], 1),
            ('production generated wrapper', ['-Command', w.copy_download_fixture(optional) + w.generated_windows_command()], 0),
        )
        for label, args, expected in controls:
            run = w.run_generated_entry([interpreter, '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', *args],
                                        env=w.entry_environment(interpreter, directory), directory=directory)
            print(engine, label, 'status=', run.returncode, 'stdout=', repr(run.stdout), 'stderr=', repr(run.stderr), flush=True)
            if run.returncode != expected:
                raise SystemExit(f'{engine} {label}: expected {expected}, observed {run.returncode}')
            if label == 'in-process residual' and b'residual=7' not in run.stdout:
                raise SystemExit('native status was not preserved in-process')
            if list(root.glob('frayd-install-*.ps1')):
                raise SystemExit('generated wrapper left an owned download')
print('RED/GREEN and semantic controls passed', flush=True)
