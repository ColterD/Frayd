"""docs/design keeps each unwired UI fragment inert, unserved and paired with an activation contract.

Whether the default UI renders only implemented controls is the real DOM suite's job (tests/browser-regression.cjs).
"""
from html.parser import HTMLParser
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
CONTROLS = {'button', 'input', 'select', 'textarea'}


class Inventory(HTMLParser):
    """Start tags as (enclosing template id or None, tag, attributes)."""

    def __init__(self, source):
        super().__init__()
        self.tags, self.template = [], None
        self.feed(source)

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == 'template':
            self.template = attrs.get('id')
        self.tags.append((self.template, tag, attrs))

    def handle_endtag(self, tag):
        if tag == 'template':
            self.template = None


def read(path):
    return (ROOT / path).read_text()


class UnwiredFragmentTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.page = Inventory(read('client/index.html'))
        cls.fragments = Inventory(read('docs/design/unwired-ui-fragments.html'))
        cls.fragment_ids = {attrs['id'] for _, tag, attrs in cls.fragments.tags if tag == 'template'}

    def test_ids_are_unique_selector_targets(self):
        ids = [attrs['id'] for _, _, attrs in self.page.tags if 'id' in attrs]
        self.assertEqual(len(ids), len(set(ids)))

    def test_every_fragment_has_an_activation_contract(self):
        rows = set(re.findall(r'^\| `(unwired-[a-z-]+)`', read('docs/development.md'), re.MULTILINE))
        self.assertEqual(rows, self.fragment_ids)

    def test_fragments_stay_inert_design_notes(self):
        self.assertEqual(len(self.fragment_ids), 5)
        for template, tag, attrs in self.fragments.tags:
            if template is None:
                continue
            self.assertFalse([name for name in attrs if name.startswith('on')], (template, tag))
            if tag in CONTROLS:
                self.assertIn('disabled', attrs, (template, tag))
                self.assertNotIn('id', attrs, (template, tag))
        self.assertNotIn('docs/design', read('client/index.html'))


if __name__ == '__main__':
    unittest.main()
