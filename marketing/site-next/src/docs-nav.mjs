/**
 * Which docs exist, and how they group. Single source of truth.
 *
 * The old site had this grouping twice: once in scripts/build-docs.py (which
 * generated the /docs hub's card grid) and once in the hand-written nav on every
 * page. They drifted — `docs/agents` reached the hub before it reached the nav.
 * Here the sidebar in astro.config.mjs and the /docs landing page both read this
 * array, so a page added in one place cannot go missing from the other.
 *
 * Slugs are Starlight collection slugs, English form. The Chinese pages live at
 * zh/<slug> and are derived, never listed separately.
 */
export const SECTIONS = [
  {
    label: 'Start here',
    zh: '从这里开始',
    slugs: ['docs/intro', 'docs/setup', 'docs/first-session'],
  },
  {
    label: 'Agents',
    zh: 'Agent',
    slugs: ['docs/agents', 'docs/worktrees', 'docs/herdr'],
  },
  {
    label: 'Terminal',
    zh: '终端',
    slugs: [
      'docs/multiplexers',
      'docs/tmux',
      'docs/mosh',
      'docs/keyboard',
      'docs/ime',
      'docs/scrolling',
      'docs/clipboard',
    ],
  },
  {
    label: 'Reference',
    zh: '参考',
    slugs: ['docs/keys', 'docs/appearance', 'docs/troubleshooting'],
  },
];

/** The sidebar shape Starlight wants, built from the array above. */
export const sidebar = SECTIONS.map(({ label, zh, slugs }) => ({
  label,
  translations: { zh },
  items: slugs,
}));
