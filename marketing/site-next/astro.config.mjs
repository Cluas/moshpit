// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import { sidebar } from './src/docs-nav.mjs';

// The site this replaces was 48 hand-written HTML files: 26 pages, each with a
// second copy for Chinese, every one carrying its own <head>, nav and footer.
// Changing one sentence meant editing two files; the rename touched 51.
//
// Two rules for this migration, both load-bearing:
//   1. Output stays a plain static directory, so the Dockerfile, nginx config
//      and scripts/deploy-site.sh keep working untouched.
//   2. The design does not get rewritten. marketing/site/assets/moshpit.css is
//      the design system and moves across verbatim; Astro's job is to stop us
//      maintaining the chrome around it twice.
export default defineConfig({
  site: 'https://moshpit.cluas.eu.org',

  // The old site served /docs/mosh — no extension, no trailing slash — and that
  // is the shape to keep: it is in the App Store support URL and in every link
  // anyone has already written.
  //
  // `format: 'file'` looks like the way to get it, and it is a trap. It writes
  // docs/mosh.html, and then every link Astro generates from a slug — the whole
  // sidebar, prev/next, the table of contents, and the canonical tag — points at
  // /docs/mosh.html. That shipped 732 extension-bearing links and, worse, a
  // canonical tag telling search engines the .html form was the real one.
  //
  // `format: 'directory'` writes docs/mosh/index.html, and with trailingSlash
  // 'never' Astro generates /docs/mosh. nginx's `try_files $uri $uri/index.html`
  // serves it. Same URLs as the old site, and nothing has to know they are
  // directories.
  trailingSlash: 'never',
  build: { format: 'directory' },

  i18n: {
    locales: ['en', 'zh'],
    defaultLocale: 'en',
    // The old site served English at / and Chinese at /zh — keep that shape so
    // nothing that already points at moshpit.cluas.eu.org has to change.
    routing: { prefixDefaultLocale: false },
  },

  integrations: [
    starlight({
      title: 'Moshpit',
      // Starlight ships its own theme; ours is already written. `customCss`
      // loads last, so moshpit.css wins wherever the two disagree.
      customCss: ['./src/styles/tokens.css', './src/styles/starlight-bridge.css'],
      // No `locales` here on purpose: Starlight adopts the Astro `i18n` config
      // above, and declaring both is a hard error. One source of truth for
      // which languages exist, shared with the marketing pages.
      // Docs live under /docs/*, matching the old docs-*.html URLs after the
      // redirect map in nginx.
      social: [
        { icon: 'github', label: 'GitHub', href: 'https://github.com/Cluas/moshpit' },
      ],
      // Shared with the /docs landing page — see src/docs-nav.mjs for why.
      sidebar,
      // Nests each page's own headings under its own sidebar entry instead of
      // a separate right-hand "On this page" rail (that rail is hidden in
      // starlight-bridge.css) — the single-list pattern design/docs.html
      // specifies. See src/components/Sidebar.astro for the override itself;
      // it's a copy-and-extend of Starlight's stock Sidebar/SidebarSublist,
      // per Starlight's own override mechanism, not a fork of its internals.
      components: {
        Sidebar: './src/components/Sidebar.astro',
      },
    }),
  ],
});
