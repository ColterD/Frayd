"""Keep rollback cleanup confined to the empty, exact task-owned bridge."""
import copy
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1] / 'scripts'
spec = importlib.util.spec_from_file_location('download_qualification', SCRIPTS / 'qualify-downloads.py')
qualification = importlib.util.module_from_spec(spec)
# The production runner imports its sibling integration module; preserve any prior test module.
with patch.object(sys, 'path', [str(SCRIPTS), *sys.path]):
    spec.loader.exec_module(qualification)


class EgressCleanupTests(unittest.TestCase):
    def test_only_exact_empty_owned_bridge_is_removed(self):
        project = 'frayd-owned-test'
        network = {'Name': project + '_downloads-egress', 'Labels': {'com.docker.compose.project': project}, 'Containers': {}}
        for invalid in ({'Name': 'other'}, {'Labels': {'com.docker.compose.project': 'other'}},
                        {'Containers': {'running': {}}}):
            with self.subTest(invalid=invalid), patch.object(qualification.qa, 'docker', side_effect=[
                subprocess.CompletedProcess([], 0, stdout=b'owned-id\n'),
                subprocess.CompletedProcess([], 0, stdout=json.dumps([{**network, **invalid}]).encode()),
            ]) as docker, self.assertRaises(AssertionError):
                qualification.retire_egress(project)
            self.assertFalse(any(call.args[:2] == ('network', 'rm') for call in docker.call_args_list))
        with patch.object(qualification.qa, 'docker', side_effect=[
            subprocess.CompletedProcess([], 0, stdout=b'owned-id\n'),
            subprocess.CompletedProcess([], 0, stdout=json.dumps([network]).encode()),
            subprocess.CompletedProcess([], 0, stdout=b''),
        ]) as docker:
            qualification.retire_egress(project)
        self.assertEqual(docker.call_args_list[-1].args, ('network', 'rm', 'owned-id'))


class ProtectedCleanupTests(unittest.TestCase):
    def test_scenario_failure_before_creation_survives_all_cleanup_failures(self):
        original = AssertionError('startup before app or volume creation')
        with patch.object(qualification, 'controlled_config', return_value={'services': {}}), \
             patch.object(qualification, 'fixture_service', return_value={}), \
             patch.object(qualification.qa, 'project_compose') as factory, \
             patch.object(qualification.qa, 'start_project', side_effect=original), \
             patch.object(qualification, 'retire_egress', side_effect=RuntimeError('bridge cleanup')) as retire, \
             patch.object(qualification.qa, 'assert_project_removed', side_effect=RuntimeError('inventory')) as inventory, \
             patch.object(sys, 'stderr', io.StringIO()) as diagnostic:
            factory.return_value.side_effect = RuntimeError('compose cleanup before volume exists')
            with self.assertRaises(AssertionError) as raised:
                qualification.optional_scenario(object())
        self.assertIs(raised.exception, original)
        factory.return_value.assert_called_once_with('down', '--volumes', '--remove-orphans')
        retire.assert_called_once()
        inventory.assert_called_once_with(retire.call_args.args[0])
        self.assertEqual(len(original.__notes__), 3)
        self.assertIn('bridge cleanup', diagnostic.getvalue())
        self.assertIn('inventory', diagnostic.getvalue())

    def test_successful_body_reports_cleanup_failure_and_still_checks_resources(self):
        with patch.object(qualification, 'retire_egress', side_effect=RuntimeError('bridge cleanup')) as retire, \
             patch.object(qualification.qa, 'assert_project_removed') as inventory:
            with self.assertRaises(ExceptionGroup) as raised:
                with qualification.cleanup_download_project(lambda *args: None, 'frayd-owned-test'):
                    pass
        self.assertEqual(str(raised.exception.exceptions[0]), 'bridge cleanup')
        retire.assert_called_once_with('frayd-owned-test')
        inventory.assert_called_once_with('frayd-owned-test')

    def test_positive_cleanup_uses_compose_owned_volumes_then_exact_bridge_and_inventory(self):
        calls = []
        with patch.object(qualification, 'retire_egress', side_effect=lambda project: calls.append(('bridge', project))), \
             patch.object(qualification.qa, 'assert_project_removed', side_effect=lambda project: calls.append(('inventory', project))):
            with qualification.cleanup_download_project(lambda *args: calls.append(args), 'frayd-owned-test'):
                calls.append(('body',))
        self.assertEqual(calls, [('body',), ('down', '--volumes', '--remove-orphans'),
                                 ('bridge', 'frayd-owned-test'), ('inventory', 'frayd-owned-test')])


class SameImageOptInTests(unittest.TestCase):
    def test_profile_can_change_only_enablement_backend_and_egress(self):
        default = {'name': 'frayd', 'services': {'app': {'image': 'frayd:local', 'build': {'context': '.'},
                   'environment': {'FRAYD_TRUSTED_PROXY': '1'}, 'networks': {'isolated': None},
                   'read_only': True, 'volumes': ['data:/data']},
                   'proxy': {'ports': ['127.0.0.1:3100:8080']}},
                   'networks': {'isolated': {'internal': True}, 'edge': {}}, 'volumes': {'data': {}}}
        optional = copy.deepcopy(default)
        optional['services']['app']['environment'].update(FRAYD_DOWNLOADS_ENABLED='1', FRAYD_DOWNLOAD_BACKEND='direct')
        optional['services']['app']['networks']['downloads-egress'] = None
        optional['networks']['downloads-egress'] = {'name': 'frayd_downloads-egress'}
        qualification.assert_profile_contract(default, optional)
        for change in ('image', 'target', 'writable', 'volume', 'port'):
            altered = copy.deepcopy(optional)
            if change == 'image':
                altered['services']['app']['image'] = 'unapproved-image'
            elif change == 'target':
                altered['services']['app']['build']['target'] = 'python'
            elif change == 'writable':
                altered['services']['app']['read_only'] = False
            elif change == 'volume':
                altered['services']['app']['volumes'] = ['other:/data']
            else:
                altered['services']['proxy']['ports'] = ['0.0.0.0:3100:8080']
            with self.subTest(change=change), self.assertRaises(AssertionError):
                qualification.assert_profile_contract(default, altered)


class ExternalFixtureTests(unittest.TestCase):
    def test_only_explicit_public_revision_fixture_is_allowed(self):
        url = 'https://raw.githubusercontent.com/example/project/' + 'a' * 40 + '/tests/fixtures/native-resume.mp4'
        self.assertEqual(qualification.validate_external_url(url), url)
        for invalid in (url.replace('https:', 'http:'), url.replace('raw.githubusercontent.com', '127.0.0.1'),
                        url.replace('raw.githubusercontent.com', 'user:password@raw.githubusercontent.com'),
                        url + '?token=synthetic', url + '#fragment', url.replace('a' * 40, 'main'),
                        url.replace('native-resume.mp4', 'arbitrary-user-file.mp4')):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                qualification.validate_external_url(invalid)


if __name__ == '__main__':
    unittest.main()
