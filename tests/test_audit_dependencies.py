"""No-network policy tests; real queries run separately in CI."""
import copy
import importlib.util
import subprocess
import sys
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('audit', Path(__file__).resolve().parents[1] / 'scripts/audit-dependencies.py')
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class AuditPolicyTests(unittest.TestCase):
    """Keep incomplete scans and unverified exceptions from reporting a clean audit."""

    def setUp(self):
        self.packages = [{'name': 'block-buffer', 'version': '0.10.4'}]
        self.results = [{'vulns': [{'id': 'GHSA-qwgh-2vcv-g2f7'}]}]
        self.primary = {'ghsa_id': 'GHSA-qwgh-2vcv-g2f7', 'vulnerabilities': [
            {'package': {'ecosystem': 'rust', 'name': 'block_buffer'},
             'vulnerable_version_range': '>=0.11.0,<0.12.1', 'patched_versions': '0.12.1',
             'vulnerable_functions': []}]}

    def classify(self, primary=None):
        return audit.classify(self.packages, self.results, lambda url: self.primary if primary is None else primary)

    def test_verified_publisher_range_resolves_only_exact_finding(self):
        report = self.classify()
        self.assertEqual(report['affected'], [])
        self.assertEqual(report['resolved'][0]['source'], audit.PRIMARY_URL)
        self.assertEqual(report['resolved'][0]['status'], 'not_affected')

    def test_unexpected_versions_packages_and_ids_stay_fatal(self):
        for name, version, advisory_id in [('block-buffer', '0.11.0', audit.DISPUTED_FINDING[2]),
                                          ('other', '0.10.4', audit.DISPUTED_FINDING[2]),
                                          ('block-buffer', '0.10.4', 'OTHER')]:
            with self.subTest(name=name, version=version, advisory_id=advisory_id):
                report = audit.classify([{'name': name, 'version': version}],
                                        [{'vulns': [{'id': advisory_id}]}],
                                        lambda url: self.fail('Unrelated finding must not invoke exception policy'))
                self.assertEqual(len(report['affected']), 1)
                self.assertEqual(report['resolved'], [])

    def test_unmodified_other_osv_finding_remains_fatal(self):
        self.results[0]['vulns'].append({'id': 'NEW-ADVISORY'})
        report = self.classify()
        self.assertEqual(report['affected'][0]['advisory'], 'NEW-ADVISORY')
        self.assertEqual(len(report['resolved']), 1)

    def test_changed_or_malformed_publisher_evidence_fails_closed(self):
        changed = copy.deepcopy(self.primary)
        changed['vulnerabilities'][0]['vulnerable_version_range'] = '>=0,<0.12.1'
        wrong_id = {**self.primary, 'ghsa_id': 'OTHER'}
        wrong_package = copy.deepcopy(self.primary)
        wrong_package['vulnerabilities'][0]['package']['name'] = 'other'
        for primary in [{}, [], changed, wrong_id, wrong_package]:
            with self.subTest(primary=primary):
                report = self.classify(primary)
                self.assertEqual(len(report['affected']), 1)
                self.assertEqual(report['resolved'], [])

    def test_unavailable_publisher_fails_closed(self):
        def unavailable(url):
            raise OSError('offline')
        report = audit.classify(self.packages, self.results, unavailable)
        self.assertEqual(report['affected'][0]['classification_error'], 'offline')
        self.assertEqual(report['resolved'], [])

    def test_paginated_results_fail_before_classifying_any_finding(self):
        for advisory in [[], [{'id': 'GHSA-qwgh-2vcv-g2f7'}]]:
            with self.subTest(advisory=advisory):
                with self.assertRaisesRegex(ValueError, 'incomplete'):
                    audit.classify(
                        self.packages * 2,
                        [self.results[0], {'vulns': advisory, 'next_page_token': 'more-results'}],
                        lambda url: self.fail('Incomplete batches must not reach publisher policy'))

    def test_complete_clean_result_stays_clean(self):
        for result in [{}, {'vulns': []}, {'vulns': [], 'next_page_token': ''}]:
            with self.subTest(result=result):
                report = audit.classify(self.packages, [result])
                self.assertEqual(report['affected'], [])
                self.assertEqual(report['resolved'], [])
                self.assertEqual(report['checked_packages'], 1)

    def test_cli_exits_nonzero_without_report_on_truncated_response(self):
        """Exercise the CLI entry point with an incomplete network response."""
        script = Path(__file__).resolve().parents[1] / 'scripts/audit-dependencies.py'
        program = """
import io
import json
import runpy
import sys
from unittest.mock import patch

def response(request, timeout):
    '''Return one truncated OSV result for every real lockfile query.'''
    count = len(json.loads(request.data)['queries'])
    return io.StringIO(json.dumps({'results': [{'next_page_token': 'more-results'}] * count}))

with patch('urllib.request.urlopen', response):
    runpy.run_path(sys.argv[1], run_name='__main__')
"""
        result = subprocess.run([sys.executable, '-c', program, str(script)],
                                capture_output=True, text=True, timeout=10)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, '')
        self.assertIn('incomplete', result.stderr)

    def test_result_count_mismatch_fails_before_any_publisher_fetch(self):
        """A short or long batch is rejected before a row is classified, so the disputed row never reaches the network."""
        fetched = []

        def record(url):
            fetched.append(url)
            return self.primary
        for results in ([], self.results * 2, self.results + [{}]):
            with self.subTest(results=len(results)), self.assertRaisesRegex(ValueError, 'count'):
                audit.classify(self.packages, results, record)
        self.assertEqual(fetched, [])


if __name__ == '__main__':
    unittest.main()
