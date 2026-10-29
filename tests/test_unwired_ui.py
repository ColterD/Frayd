"""Guard inert design fragments against accidental activation or form submission."""
from html.parser import HTMLParser
from pathlib import Path
import unittest


class PageInventory(HTMLParser):
    def __init__(self, source):
        super().__init__()
        self.templates = {}
        self.template = None
        self.active = []
        self.ids = []
        self.hero_open = False
        self.template_inside_hero = False
        self.feed(source)

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if 'id' in attrs:
            self.ids.append(attrs['id'])
        if attrs.get('id') == 'hero':
            self.hero_open = True
        if tag == 'template':
            self.template_inside_hero |= self.hero_open
            self.template = attrs.get('id')
            self.templates[self.template] = []
        elif self.template is not None:
            self.templates[self.template].append((tag, attrs))
        else:
            self.active.append((tag, attrs))

    def handle_endtag(self, tag):
        if tag == 'template':
            self.template = None
        # The production hero is an empty div, replaced by renderHero().
        if tag == 'div' and self.hero_open:
            self.hero_open = False


class UnwiredUiTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.page = PageInventory(
            (Path(__file__).resolve().parents[1] / 'client/index.html').read_text()
        )

    def test_preserved_controls_cannot_enter_active_dom_or_dynamic_hero(self):
        page = self.page
        self.assertEqual(len(page.templates), 5, 'preserve the five design fragments')
        self.assertFalse(page.template_inside_hero, 'renderHero would delete the fragment')
        self.assertEqual(len(page.ids), len(set(page.ids)), 'avoid duplicate selector targets')
        active_controls = [attrs for tag, attrs in page.active if tag in {'button', 'input'}]
        self.assertFalse(any(attrs.get('type') == 'checkbox' for attrs in active_controls))
        self.assertFalse(any(attrs.get('aria-label') in {'Previous', 'Next', 'Notifications'}
                             for attrs in active_controls))
        self.assertFalse(any(attrs.get('id') in {'remember', 'bell', 'hero-info'}
                             for attrs in active_controls))

    def test_prototype_controls_require_disabled_non_submission_semantics(self):
        controls = []
        for fragment in self.page.templates.values():
            for tag, attrs in fragment:
                self.assertNotIn(tag, {'script', 'style', 'a', 'form', 'iframe'})
                self.assertFalse(any(name.startswith('on') for name in attrs))
                self.assertNotIn('id', attrs, 'cloning must not duplicate live selector targets')
                if tag in {'button', 'input'}:
                    controls.append((tag, attrs))
                    self.assertIn('disabled', attrs)
                    self.assertEqual(attrs.get('type'), 'button' if tag == 'button' else 'checkbox')
        self.assertEqual(sum(tag == 'button' for tag, _ in controls), 6)
        self.assertEqual(sum(tag == 'input' for tag, _ in controls), 1)


if __name__ == '__main__':
    unittest.main()
