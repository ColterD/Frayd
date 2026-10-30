"""Private two-case startup diagnostic; run from the frozen QA checkout."""

import argparse
import hashlib
import importlib.util
import json
import os
import shutil
import sys
import time
from pathlib import Path

EXPECTED_HASHES = {
    'windows_job.py': '47c87910e33faef9414d46953663b46004d8a21d60a261caff1a38b7ca949e4e',
    'test_bootstrap_windows.py': 'd36d076c986759636e87878f3b969b66413dabc7ebf955ee4f34107006cd8fb0',
}
EXPECTED_BLOBS = {
    'windows_job.py': '224ccf0d5760802ef95589646a1673159fa91c20',
    'test_bootstrap_windows.py': 'cd3ba0521853f14a353a4730d8b924b66d45f59b',
}
STAGES = {'parent-enter', 'parent-before-child', 'child-enter', 'before-join-path',
          'after-join-path', 'before-sleep'}
PHASE_WRITER = r"""
function Write-ProbePhase {
  param([string]$Stage)
  $probeExecutable = [IO.Path]::GetFileName([Diagnostics.Process]::GetCurrentProcess().MainModule.FileName)
  $probeLine = '{0}|{1}|{2}|{3}|{4}|{5}|{6}|{7}' -f $Stage, [DateTime]::UtcNow.Ticks, $PID, $probeExecutable, [int][bool]$env:USERPROFILE, [int][bool]$env:APPDATA, [int][bool]$env:LOCALAPPDATA, [int][bool]$env:PSModulePath
  [IO.File]::AppendAllText([IO.Path]::Combine($env:TEMP, 'startup-phases.log'), $probeLine + [Environment]::NewLine)
}
"""


class ProbeSetupError(RuntimeError):
    def __init__(self, stage, reason):
        super().__init__(reason)
        self.stage = stage
        self.reason = reason


def setup_stage(name):
    print('setup_stage=' + name, flush=True)


def verify_sources(tests):
    verified = {}
    for name, expected in EXPECTED_HASHES.items():
        setup_stage('verify-' + name)
        raw = (tests / name).read_bytes()
        canonical = raw.replace(b'\r\n', b'\n')
        digest = hashlib.sha256(canonical).hexdigest()
        blob = hashlib.sha1(b'blob ' + str(len(canonical)).encode('ascii') + b'\0' + canonical).hexdigest()
        print('source_identity=' + json.dumps({'file': name, 'raw_sha256': hashlib.sha256(raw).hexdigest(),
              'canonical_lf_sha256': digest, 'canonical_lf_git_blob': blob,
              'crlf_pairs': raw.count(b'\r\n')}), flush=True)
        if digest != expected or blob != EXPECTED_BLOBS[name]:
            raise ProbeSetupError('verify-' + name, 'source_digest_mismatch')
        verified[name] = raw
    return verified


def load_verified_modules(tests, verified):
    modules = []
    for name in EXPECTED_HASHES:
        setup_stage('import-' + name)
        module_name = Path(name).stem
        spec = importlib.util.spec_from_file_location(module_name, tests / name)
        module = importlib.util.module_from_spec(spec)
        sys.modules[module_name] = module
        # Compile exactly the bytes verified above: no reread, pyc, or path lookup.
        exec(compile(verified[name], str(tests / name), 'exec'), module.__dict__)  # noqa: S102 - only hash-pinned bytes; bypass pyc/rereads
        modules.append(module)
    return tuple(modules)


def instrument_child(fixtures):
    source = fixtures.harmless_entry_script(sleep_seconds=60)
    first = "[IO.File]::WriteAllText((Join-Path $env:TEMP 'entry-script-started'), '1')"
    if source.count(first) != 1 or source.count('Start-Sleep -Seconds 60') != 1:
        raise ProbeSetupError('construct-child', 'child_fixture_contract_changed')
    source = source.replace(first, PHASE_WRITER + "\nWrite-ProbePhase 'child-enter'\n"
                            "Write-ProbePhase 'before-join-path'\n"
                            "$probeMarker = Join-Path $env:TEMP 'entry-script-started'\n"
                            "Write-ProbePhase 'after-join-path'\n"
                            "[IO.File]::WriteAllText($probeMarker, '1')")
    return source.replace('Start-Sleep -Seconds 60', "Write-ProbePhase 'before-sleep'\nStart-Sleep -Seconds 60")


def instrument_parent(fixtures, source):
    command = fixtures.generated_windows_command(" -Lib 'deadline fixture'")
    invocation = '; powershell.exe '
    if command.count(invocation) != 1:
        raise ProbeSetupError('construct-parent', 'parent_fixture_contract_changed')
    command = command.replace(invocation, "; Write-ProbePhase 'parent-before-child'; powershell.exe ")
    return PHASE_WRITER + "\nWrite-ProbePhase 'parent-enter'; " + fixtures.copy_download_fixture(source) + command


def report_phases(directory):
    path = Path(directory) / 'startup-phases.log'
    try:
        if not path.exists():
            print('phase_events=[]', flush=True)
            return []
        with path.open('rb') as stream:
            data = stream.read(8193)
        if len(data) > 8192:
            raise ValueError('phase log exceeded bound')
        events = []
        for line in data.decode('utf-8-sig').splitlines():
            fields = line.split('|')
            if (len(fields) != 8 or fields[0] not in STAGES
                    or not fields[1].isdigit() or not fields[2].isdigit()
                    or fields[3].lower() not in {'pwsh.exe', 'powershell.exe'}
                    or any(value not in {'0', '1'} for value in fields[4:])):
                raise ValueError('invalid phase fields')
            events.append(dict(zip(('stage', 'utc_ticks', 'pid', 'exe_basename', 'has_userprofile',
                                    'has_appdata', 'has_localappdata', 'has_psmodulepath'), fields, strict=True)))
        print('phase_events=' + json.dumps(events), flush=True)
        return events
    except (OSError, ValueError, UnicodeError) as error:
        # Reporting must never replace a primary JobCleanupError.
        print('phase_report_error=' + type(error).__name__, flush=True)
        return None


def profile_environment(base, directory, condition):
    if condition not in {'A', 'B'}:
        raise ProbeSetupError('profile-environment', 'invalid_profile_condition')
    profile = Path(directory) / 'profile'
    roaming = profile / 'AppData' / 'Roaming'
    local = profile / 'AppData' / 'Local'
    for folder in (roaming, local):
        folder.mkdir(parents=True, exist_ok=True)
    env = dict(base)
    if condition == 'B':
        env.update(USERPROFILE=str(profile), APPDATA=str(roaming), LOCALAPPDATA=str(local))
    return env


def phase_before_deadline(case, stage):
    start = case['launch_utc_ticks']
    return any(event['stage'] == stage and start <= int(event['utc_ticks']) < start + 300_000_000
               for event in case['events'] or [])


def profile_bundle_oracle(cases):
    comparisons = []
    for engine in ('pwsh', 'powershell'):
        original = cases[(engine, 'A')]
        treatment = cases[(engine, 'B')]
        flags_valid = all(
            case['events'] and all(all(event[key] == expected for key in (
                'has_userprofile', 'has_appdata', 'has_localappdata')) for event in case['events'])
            for case, expected in ((original, '0'), (treatment, '1')))
        original_reached_boundary = phase_before_deadline(original, 'before-join-path')
        original_progress = all(phase_before_deadline(original, stage) for stage in ('after-join-path', 'before-sleep'))
        treatment_progress = all(phase_before_deadline(treatment, stage) for stage in ('after-join-path', 'before-sleep'))
        comparisons.append({'engine': engine, 'profile_flags_valid': bool(flags_valid),
                            'original_reached_boundary': original_reached_boundary,
                            'original_progress_before_30': original_progress,
                            'treatment_progress_before_30': treatment_progress,
                            'bundle_supported': bool(flags_valid and original_reached_boundary
                                                     and treatment_progress and not original_progress)})
    supported = all(item['bundle_supported'] for item in comparisons)
    print('profile_bundle_oracle=' + json.dumps({'comparisons': comparisons, 'supported': supported,
          'individual_variable_causality': 'unverified', 'production_acceptance': 'unverified'}), flush=True)
    return supported


def run_case(engine, interpreter, fixtures, controller, condition=None):
    with controller.job_fixture(prefix='Frayd startup probe ') as directory:
        source = Path(directory) / 'harmless-source.ps1'
        setup_stage('construct-child')
        source.write_text(instrument_child(fixtures), encoding='ascii')
        setup_stage('construct-parent')
        command = instrument_parent(fixtures, source)
        env = fixtures.entry_environment(interpreter, directory)
        if condition is not None:
            env = profile_environment(env, directory, condition)
        print('case=' + engine + (':' + condition if condition is not None else ''), flush=True)
        # .NET ticks since year1; the slightly earlier timestamp is conservative.
        launch_utc_ticks = time.time_ns() // 100 + 621_355_968_000_000_000
        print('case_launch_utc_ticks=' + str(launch_utc_ticks), flush=True)
        cleanup_ok = False
        try:
            result = controller.run_windows_job(
                [interpreter, '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-Command', command],
                env=env, directory=directory)
            print('unexpected_completion=' + str(result.returncode), flush=True)
        except controller.JobTimeout as error:
            print('cleanup=' + json.dumps({'status': 'JobTimeout', 'active': error.job_active_processes,
                  'total': error.job_total_processes, 'host_exited': error.host_exited,
                  'elapsed': error.elapsed}), flush=True)
            cleanup_ok = error.job_active_processes == 0 and error.host_exited and 30 <= error.elapsed < 36
        except controller.JobCleanupError:
            print('cleanup_unverified_fixture=' + directory, flush=True)
            raise
        finally:
            events = report_phases(directory)
        return {'cleanup_ok': cleanup_ok, 'events': events, 'launch_utc_ticks': launch_utc_ticks}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tests-dir', type=Path, default=Path.cwd() / 'tests')
    parser.add_argument('--profile-control', action='store_true', help='run B then A per engine; test private profile bundle')
    args = parser.parse_args()
    setup_stage('platform')
    if os.name != 'nt':
        raise ProbeSetupError('platform', 'windows_required')
    setup_stage('tests-directory')
    tests = args.tests_dir.resolve(strict=True)
    verified = verify_sources(tests)
    controller, fixtures = load_verified_modules(tests, verified)
    succeeded = True
    cases = {}
    for engine in ('pwsh', 'powershell'):
        setup_stage('resolve-' + engine)
        interpreter = shutil.which(engine)
        if interpreter is None:
            raise ProbeSetupError('resolve-' + engine, 'required_interpreter_missing')
        for condition in (('B', 'A') if args.profile_control else (None,)):
            case = run_case(engine, interpreter, fixtures, controller, condition)
            cases[(engine, condition)] = case
            succeeded = case['cleanup_ok'] and succeeded
    if args.profile_control:
        supported = profile_bundle_oracle(cases)
        print('diagnostic_cleanup_verified=' + json.dumps(succeeded), flush=True)
        return 0 if succeeded and supported else 1
    # A diagnostic exit0 proves bounded ownership cleanup only, not startup acceptance.
    return 0 if succeeded else 1


if __name__ == '__main__':
    try:
        status = main()
    except (OSError, RuntimeError, AssertionError, ValueError, ImportError) as error:
        # TimeoutExpired repr/tracebacks contain argv; never print them here.
        print('probe_error_type=' + type(error).__name__, flush=True)
        if isinstance(error, ProbeSetupError):
            print('probe_setup_failure=' + json.dumps({'stage': error.stage, 'reason': error.reason}), flush=True)
        else:
            print('probe_error_reason=unclassified_failure_at_last_stage', flush=True)
        status = 2
    sys.exit(status)
