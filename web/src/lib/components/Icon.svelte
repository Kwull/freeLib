<script lang="ts">
  // A small curated set of inline stroke icons matching the prototype's style
  // (stroke-width 1.8–2, round caps). Keeping them as one component avoids
  // repeating raw <svg> markup across every route.
  const paths: Record<string, string> = {
    search: 'M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14Zm9 2-3.5-3.5',
    close: 'M6 6l12 12M18 6 6 18',
    chevronDown: 'm6 9 6 6 6-6',
    chevronUp: 'm6 15 6-6 6 6',
    grip: 'M9 5h.01M15 5h.01M9 12h.01M15 12h.01M9 19h.01M15 19h.01',
    chevronRight: 'm9 6 6 6-6 6',
    chevronLeft: 'm15 6-6 6 6 6',
    send: 'M22 2 11 13M22 2 15 22l-4-9-9-4Z',
    download: 'M12 3v12m0 0-5-5m5 5 5-5M4 21h16',
    read: 'M2 5h7a3 3 0 0 1 3 3v13a2 2 0 0 0-2-2H2ZM22 5h-7a3 3 0 0 0-3 3v13a2 2 0 0 1 2-2h8Z',
    library: 'M4 21h16M6 21V9l6-4 6 4v12M10 21v-6h4v6',
    opds: 'M4 5h14l2 8v6H3v-6ZM3 13h5l2 3h4l2-3h5',
    newArrivals: 'M3 13h5l2 3h4l2-3h5M5 5h14l2 8v6H3v-6Z',
    authors: 'M12 8a4 4 0 1 0 0-8 4 4 0 0 0 0 8ZM4 21c0-4 4-6 8-6s8 2 8 6',
    series: 'm12 3 9 5-9 5-9-5ZM3 13l9 5 9-5',
    genres: 'M3 3h7v7H3zM14 3h7v7h-7zM3 14h7v7H3zM14 14h7v7h-7z',
    shelves: 'M3 12V4h8l10 10-8 8ZM7.5 7.5h.01',
    settings: 'M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6ZM12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M4.9 19.1 7 17M17 7l2.1-2.1',
    activity: 'M3 12h4l3 8 4-16 3 8h4',
    plus: 'M12 5v14M5 12h14',
    table: 'M8 6h13M8 12h13M8 18h13M3 6h1M3 12h1M3 18h1',
    grid: 'M3 3h7v7H3zM14 3h7v7h-7zM3 14h7v7H3zM14 14h7v7h-7z',
    check: 'm5 12 5 5 9-10',
    star: 'm12 3 2.8 5.8 6.2.9-4.5 4.4 1 6.3L12 17.5 6.5 20.4l1-6.3L3 9.7l6.2-.9Z',
    logout: 'M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4M16 17l5-5-5-5M21 12H9',
    folder: 'M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z',
    file: 'M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8ZM14 2v6h6',
    trash: 'M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m3 0-1 14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2L4 6',
    refresh: 'M3 12a9 9 0 0 1 15-6.7L21 8M21 3v5h-5M21 12a9 9 0 0 1-15 6.7L3 16m0 5v-5h5',
    filter: 'M4 5h16l-6 8v6l-4 2v-8Z',
    external: 'M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6M15 3h6v6M10 14 21 3',
    menu: 'M3 6h18M3 12h18M3 18h18',
    sun: 'M12 17a5 5 0 1 0 0-10 5 5 0 0 0 0 10ZM12 1v2M12 21v2M4.2 4.2l1.4 1.4M18.4 18.4l1.4 1.4M1 12h2M21 12h2M4.2 19.8l1.4-1.4M18.4 5.6l1.4-1.4',
    moon: 'M21 12.8A9 9 0 1 1 11.2 3 7 7 0 0 0 21 12.8Z',
    alert: 'M12 9v4M12 17h.01M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z',
    key: 'M15 7a4 4 0 1 1-3.5 6L4 20.5V17h3v-3h3l1.5-1.5A4 4 0 0 1 15 7ZM16 9h.01',
    link: 'M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7',
    user: 'M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8ZM4 21c0-4 4-6 8-6s8 2 8 6',
    home: 'M3 11 12 4l9 7M5 10v10h5v-6h4v6h5V10',
    bell: 'M6 16v-5a6 6 0 0 1 12 0v5l2 2H4ZM10 21a2 2 0 0 0 4 0',
    layers: 'm12 3 9 5-9 5-9-5ZM3 12l9 5 9-5M3 16l9 5 9-5',
  };

  let { name, size = 18, strokeWidth = 1.8, ...rest }: { name: string; size?: number; strokeWidth?: number; class?: string } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width={strokeWidth}
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  {...rest}
>
  <path d={paths[name] ?? ''} />
</svg>
