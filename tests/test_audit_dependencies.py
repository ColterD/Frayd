"""No-network policy tests; real queries run separately in CI."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('audit', Path(__file__).resolve().parents[1] / 'scripts/audit-dependencies.py')
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class AuditPolicyTests(unittest.TestCase):
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

    def test_result_count_mismatch_fails(self):
        with self.assertRaises(ValueError):
            audit.classify(self.packages, [])


if __name__ == '__main__':
    unittest.main()
