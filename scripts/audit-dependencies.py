"""Audit locked crates with OSV; verify one disputed range against its publisher."""
import json
import sys
import tomllib
import urllib.request
from pathlib import Path

OSV_URL = 'https://api.osv.dev/v1/querybatch'
PRIMARY_URL = 'https://api.github.com/repos/RustCrypto/utils/security-advisories/GHSA-qwgh-2vcv-g2f7'
DISPUTED_FINDING = ('block-buffer', '0.10.4', 'GHSA-qwgh-2vcv-g2f7')


def fetch_json(request):
    with urllib.request.urlopen(request, timeout=45) as response:
        return json.load(response)


def classify(packages, results, primary_fetch=fetch_json):
    if len(results) != len(packages):
        raise ValueError('OSV result count differs from lockfile package count')
    affected = []
    resolved = []
    for package, result in zip(packages, results):
        for advisory in result.get('vulns', []):
            finding = {'name': package['name'], 'version': package['version'], 'advisory': advisory['id']}
            if (package['name'], package['version'], advisory['id']) != DISPUTED_FINDING:
                affected.append(finding)
                continue
            try:
                primary = primary_fetch(PRIMARY_URL)
                # Exact publisher schema/range is deliberate: changes require policy review.
                expected = [{'package': {'ecosystem': 'rust', 'name': 'block_buffer'},
                             'vulnerable_version_range': '>=0.11.0,<0.12.1',
                             'patched_versions': '0.12.1', 'vulnerable_functions': []}]
                if primary['ghsa_id'] != advisory['id'] or primary['vulnerabilities'] != expected:
                    raise ValueError('Publisher identity or affected range changed')
            except Exception as error:
                affected.append({**finding, 'classification_error': str(error)})
                continue
            resolved.append({**finding, 'status': 'not_affected', 'source': PRIMARY_URL,
                             'publisher_affected_range': '>=0.11.0,<0.12.1',
                             'reason': 'Locked 0.10.4 predates affected 0.11.0; raw OSV disagreement retained.'})
    return {'checked_packages': len(packages), 'source': OSV_URL,
            'affected': affected, 'resolved': resolved}


def main():
    root = Path(__file__).resolve().parents[1]
    packages = tomllib.loads((root / 'server/Cargo.lock').read_text())['package']
    packages = [package for package in packages if package.get('source', '').startswith('registry+')]
    request = urllib.request.Request(
        OSV_URL, data=json.dumps({'queries': [
            {'package': {'name': package['name'], 'ecosystem': 'crates.io'}, 'version': package['version']}
            for package in packages]}).encode(), headers={'Content-Type': 'application/json'})
    report = classify(packages, fetch_json(request)['results'])
    print(json.dumps(report, indent=2))
    return bool(report['affected'])


if __name__ == '__main__':
    sys.exit(main())
