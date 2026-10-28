// Fixture-only protocol. Kept in <head> so it contributes no body text offsets.
window.addEventListener('message', event => {
  if (event.source !== parent || event.data?.type !== 'regression-probe') return;
  const { id, action, start, end, nextNode } = event.data;
  try {
    if (action === 'ready') parent.postMessage({ type: 'reader-ready', generation: event.data.generation }, '*');
    if (action === 'select') {
      const target = document.getElementById('target');
      const walker = document.createTreeWalker(target, NodeFilter.SHOW_TEXT);
      const nodes = [];
      while (walker.nextNode()) nodes.push(walker.currentNode);
      function endpoint(offset) {
        for (const node of nodes) {
          if (offset <= node.length) return [node, offset];
          offset -= node.length;
        }
        throw new Error('Selection offset exceeds fixture text');
      }
      const range = document.createRange();
      range.setStart(...endpoint(start));
      if (nextNode) range.setEnd(document.getElementById('next').firstChild, 0);
      else range.setEnd(...endpoint(end));
      const selection = getSelection();
      selection.removeAllRanges();
      selection.addRange(range);
      document.dispatchEvent(new MouseEvent('mouseup', { bubbles: true }));
    }
    setTimeout(() => parent.postMessage({ type: 'regression-reply', id,
      marks: [...document.querySelectorAll('mark.user-highlight')].map(mark => mark.textContent),
      searchMarks: [...document.querySelectorAll('mark.search-hit')].map(mark => mark.textContent),
      opaqueOrigin: self.origin,
    }, '*'), 0);
  } catch (error) {
    parent.postMessage({ type: 'regression-reply', id, error: error.stack }, '*');
  }
});
