"""The shipped page offers implemented controls only; design fragments live in docs/design."""
from html.parser import HTMLParser
from pathlib import Path
import unittest


class PageInventory(HTMLParser):
    def __init__(self, source):
        super().__init__()
        self.tags = []
        self.comments = []
        self.feed(source)

    def handle_starttag(self, tag, attrs):
        self.tags.append((tag, dict(attrs)))

    def handle_comment(self, data):
        self.comments.append(data)


class ShippedPageTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.page = PageInventory(
            (Path(__file__).resolve().parents[1] / 'client/index.html').read_text()
        )

    def test_index_ships_no_unwired_controls(self):
        self.assertNotIn('template', [tag for tag, _ in self.page.tags])
        self.assertEqual([(tag, attrs) for tag, attrs in self.page.tags if 'disabled' in attrs], [])
        self.assertEqual([comment for comment in self.page.comments if 'UNWIRED' in comment], [])

    def test_ids_are_unique_selector_targets(self):
        ids = [attrs['id'] for _, attrs in self.page.tags if 'id' in attrs]
        self.assertEqual(len(ids), len(set(ids)))


if __name__ == '__main__':
    unittest.main()
