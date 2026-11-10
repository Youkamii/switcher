// One stroke system for all controls. Icons are drawn here, not font/emoji glyphs.
const paths = {
  key: ['M14.5 8.5a4 4 0 1 1-2.7 6.7L7 20H4v-3l4.8-4.8a4 4 0 0 1 5.7-3.7Z', 'm15 10 1 1'],
  plus: ['M12 5v14M5 12h14'],
  reconnect: ['M20 7v5h-5M4 17v-5h5', 'M6 7a7 7 0 0 1 12-1l2 3M4 15l2 3a7 7 0 0 0 12-1'],
  edit: ['m15 4 5 5M4 20l5-1L20 8a2 2 0 0 0-4-4L5 15l-1 5Z'],
  trash: ['M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13M10 10v7M14 10v7'],
  check: ['m5 12 4 4L19 6'],
  close: ['m6 6 12 12M18 6 6 18'],
  terminal: ['m5 7 5 5-5 5M13 17h6'],
  copy: ['M8 8h12v12H8zM16 8V4H4v12h4'],
  arrow: ['M4 12h16m-6-6 6 6-6 6'],
  shield: ['M12 3 4 6v6c0 5 8 9 8 9s8-4 8-9V6l-8-3Z', 'm8 12 3 3 5-6'],
} as const;
export type IconName = keyof typeof paths;
export function icon(name: IconName): SVGSVGElement {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox', '0 0 24 24'); svg.setAttribute('aria-hidden', 'true'); svg.classList.add('ui-icon');
  for (const d of paths[name]) { const path=document.createElementNS(svg.namespaceURI,'path'); path.setAttribute('d',d); svg.append(path); }
  return svg;
}
export function iconButton(name: IconName, label: string, className = ''): HTMLButtonElement {
  const button=document.createElement('button'); button.type='button'; button.className=`icon-button ${className}`.trim();
  button.title=label; button.setAttribute('aria-label',label); button.append(icon(name)); return button;
}
export function setButtonIcon(button: HTMLButtonElement, name: IconName, label: string) {
  button.replaceChildren(icon(name)); button.title=label; button.setAttribute('aria-label',label);
}
