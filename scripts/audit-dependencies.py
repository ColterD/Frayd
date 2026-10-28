"""Query existing crates.io lockfile dependencies; never modify dependencies."""
import json
import sys
import tomllib
import urllib.request
from pathlib import Path

root = Path(__file__).resolve().parents[1]
packages = tomllib.loads((root / 'server/Cargo.lock').read_text())['package']
packages = [package for package in packages if package.get('source', '').startswith('registry+')]
request = urllib.request.Request(
    'https://api.osv.dev/v1/querybatch',
    data=json.dumps({'queries': [
        {'package': {'name': package['name'], 'ecosystem': 'crates.io'}, 'version': package['version']}
        for package in packages
    ]}).encode(),
    headers={'Content-Type': 'application/json'},
)
with urllib.request.urlopen(request, timeout=45) as response:
    results = json.load(response)['results']
assert len(results) == len(packages)
affected = [
    {'name': package['name'], 'version': package['version'],
     'advisories': [advisory['id'] for advisory in result.get('vulns', [])]}
    for package, result in zip(packages, results) if result.get('vulns')
]
report = {'checked_packages': len(packages), 'source': request.full_url, 'affected': affected}
print(json.dumps(report, indent=2))
sys.exit(bool(affected))
