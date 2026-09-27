// http(s) URLs in plain text become links. Only text nodes are touched and the anchors are built
// with DOM calls (never by pasting strings into HTML), so nothing in the text can inject markup.
import { LINK_REL, splitUrls } from './linkify';

/** Links the URLs in the text nodes of `root` (anchors are skipped). */
export function linkifyNode(root: Node): void {
  const doc = root.ownerDocument ?? document;
  const walker = doc.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const texts: Text[] = [];
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if ((n.parentElement?.closest('a'))) continue;
    if (/https?:\/\//i.test(n.nodeValue ?? '')) texts.push(n as Text);
  }
  for (const node of texts) {
    const parts = splitUrls(node.nodeValue ?? '');
    if (!parts.some((p) => p.url)) continue;
    const frag = doc.createDocumentFragment();
    for (const p of parts) {
      if (!p.url) { frag.append(doc.createTextNode(p.text)); continue; }
      const a = doc.createElement('a');
      a.setAttribute('href', p.url);
      a.setAttribute('target', '_blank');
      a.setAttribute('rel', LINK_REL);
      a.textContent = p.text;
      frag.append(a);
    }
    node.replaceWith(frag);
  }
}

/** The (already sanitized) annotation HTML with its URLs linked. Parsed in an inert <template>. */
export function linkifyHtml(html: string): string {
  if (!/https?:\/\//i.test(html)) return html;
  const tpl = document.createElement('template');
  tpl.innerHTML = html;
  linkifyNode(tpl.content);
  return tpl.innerHTML;
}
