// Platform checks for delivery shortcuts.

/**
 * iPhone, iPad or iPod (Safari or any iOS browser — they all use WebKit and hand EPUB
 * downloads to Books). iPadOS reports itself as a Mac, so a Mac with a touch screen counts.
 */
export function isIOS(nav: Pick<Navigator, 'userAgent' | 'maxTouchPoints'> = navigator): boolean {
  const ua = nav.userAgent || '';
  if (/iPhone|iPad|iPod/.test(ua)) return true;
  return /Macintosh/.test(ua) && (nav.maxTouchPoints ?? 0) > 1;
}

/** Whether the page was opened as localhost (a phone could not follow a link to it). */
export function isLocalHost(host = location.hostname): boolean {
  return host === 'localhost' || host === '127.0.0.1' || host === '[::1]' || host === '::1' || host.endsWith('.localhost');
}
